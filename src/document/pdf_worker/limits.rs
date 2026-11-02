//! Operating-system resource limits for the short-lived PDF subprocess.
//! These limits do not isolate filesystem access granted to the current user.
use std::process::{Child, Command};

pub(super) const MEMORY_BYTES: usize = 2 * 1024 * 1024 * 1024;
#[cfg(target_os = "linux")]
pub(super) struct ProcessLimit;

#[cfg(target_os = "linux")]
pub(super) fn configure(command: &mut Command) -> anyhow::Result<()> {
    use std::os::unix::process::CommandExt;
    // Cache this before fork. Checking getppid after setting PDEATHSIG closes
    // the race where the parent dies before the signal has been installed.
    let parent = unsafe { libc::getpid() };
    command.process_group(0);
    // SAFETY: this closure executes between fork and exec. It only makes
    // async-signal-safe syscalls and constructs errno-only io::Errors; no
    // locks, allocations, logging or runtime facilities are used here.
    unsafe {
        command.pre_exec(move || {
            for (resource, soft, hard) in [
                (
                    libc::RLIMIT_AS,
                    MEMORY_BYTES as libc::rlim_t,
                    MEMORY_BYTES as libc::rlim_t,
                ),
                (libc::RLIMIT_CPU, 60, 61),
                (libc::RLIMIT_CORE, 0, 0),
                (libc::RLIMIT_FSIZE, 128 * 1024 * 1024, 128 * 1024 * 1024),
            ] {
                let limit = libc::rlimit {
                    rlim_cur: soft,
                    rlim_max: hard,
                };
                if libc::setrlimit(resource, &limit) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            if libc::prctl(
                libc::PR_SET_NO_NEW_PRIVS,
                1 as libc::c_ulong,
                0 as libc::c_ulong,
                0 as libc::c_ulong,
                0 as libc::c_ulong,
            ) != 0
            {
                return Err(std::io::Error::last_os_error());
            }
            if libc::prctl(
                libc::PR_SET_PDEATHSIG,
                libc::SIGKILL as libc::c_ulong,
                0 as libc::c_ulong,
                0 as libc::c_ulong,
                0 as libc::c_ulong,
            ) != 0
            {
                return Err(std::io::Error::last_os_error());
            }
            if libc::getppid() != parent {
                return Err(std::io::Error::from_raw_os_error(libc::ECHILD));
            }
            // The direct syscall avoids requiring a new glibc symbol in the
            // AppImage. CLOEXEC needs Linux >=5.11 (Ubuntu22.04 uses >=5.15).
            // Fail closed on older kernels. Do not close immediately: Rust's
            // private exec-error pipe must remain open until exec succeeds.
            if libc::syscall(
                libc::SYS_close_range,
                3 as libc::c_uint,
                libc::c_uint::MAX,
                libc::CLOSE_RANGE_CLOEXEC,
            ) != 0
            {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    Ok(())
}

#[cfg(target_os = "linux")]
pub(super) fn attach(_: &Child) -> anyhow::Result<ProcessLimit> {
    Ok(ProcessLimit)
}

#[cfg(windows)]
pub(super) struct ProcessLimit {
    // OwnedHandle closes the last, non-inheritable handle on drop; the job's
    // KILL_ON_JOB_CLOSE flag then terminates any still-running worker.
    _job: std::os::windows::io::OwnedHandle,
}

#[cfg(windows)]
pub(super) fn configure(command: &mut Command) -> anyhow::Result<()> {
    use std::os::windows::process::CommandExt;
    use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
    command.creation_flags(CREATE_NO_WINDOW);
    Ok(())
}

#[cfg(windows)]
pub(super) fn attach(child: &Child) -> anyhow::Result<ProcessLimit> {
    use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
    use windows_sys::Win32::System::JobObjects::*;
    let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
    anyhow::ensure!(
        !raw.is_null(),
        "Cannot create PDF worker job: {}",
        std::io::Error::last_os_error()
    );
    // SAFETY: CreateJobObjectW returned a new valid owned handle. Null security
    // attributes make it non-inheritable. This handle is never duplicated.
    let job = unsafe { OwnedHandle::from_raw_handle(raw) };
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_PROCESS_MEMORY
        | JOB_OBJECT_LIMIT_PROCESS_TIME
        | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        | JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
    limits.BasicLimitInformation.ActiveProcessLimit = 1;
    // Windows job CPU time is user-mode time in 100-nanosecond units.
    limits.BasicLimitInformation.PerProcessUserTimeLimit = 60 * 10_000_000;
    limits.ProcessMemoryLimit = MEMORY_BYTES;
    // SAFETY: the live job handle and correctly sized limits buffer remain
    // valid throughout the call. The child has not received a PDF yet.
    anyhow::ensure!(
        unsafe {
            SetInformationJobObject(
                job.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&limits) as u32,
            )
        } != 0,
        "Cannot set PDF worker job limits: {}",
        std::io::Error::last_os_error()
    );
    // Verify enforcement was accepted before assigning the worker or letting
    // the parent send document data. Compatibility layers may report success
    // while silently ignoring individual job limits.
    let mut observed = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    anyhow::ensure!(
        unsafe {
            QueryInformationJobObject(
                job.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                (&mut observed as *mut JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of_val(&observed) as u32,
                std::ptr::null_mut(),
            )
        } != 0,
        "Cannot verify PDF worker job limits: {}",
        std::io::Error::last_os_error()
    );
    anyhow::ensure!(
        observed.ProcessMemoryLimit == MEMORY_BYTES
            && observed.BasicLimitInformation.ActiveProcessLimit == 1
            && observed.BasicLimitInformation.PerProcessUserTimeLimit == 60 * 10_000_000
            && observed.BasicLimitInformation.LimitFlags & limits.BasicLimitInformation.LimitFlags
                == limits.BasicLimitInformation.LimitFlags,
        "Windows did not retain the required PDF worker job limits"
    );
    // SAFETY: both handles are live; the caller keeps the child waiting for
    // its stdin handshake until this assignment succeeds. Failure is fatal.
    anyhow::ensure!(
        unsafe { AssignProcessToJobObject(job.as_raw_handle(), child.as_raw_handle()) } != 0,
        "Cannot assign PDF worker to restricted job: {}",
        std::io::Error::last_os_error()
    );
    Ok(ProcessLimit { _job: job })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_child_observes_resource_limits_and_no_inherited_descriptor() {
        use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
        let file = std::fs::File::open("/dev/null").unwrap();
        // Deliberately create a descriptor without CLOEXEC, as a hostile native
        // dependency could do in the GUI parent.
        let descriptor = unsafe { libc::dup(file.as_raw_fd()) };
        assert!(descriptor >= 3);
        let descriptor = unsafe { OwnedFd::from_raw_fd(descriptor) };
        let name = concat!(module_path!(), "::linux_probe_child");
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", name.split_once("::").unwrap().1, "--nocapture"])
            .env("DEEPL_LIMITS_TEST_CHILD", "1")
            .env("DEEPL_LIMITS_TEST_FD", descriptor.as_raw_fd().to_string());
        configure(&mut command).unwrap();
        // libtest creates a new test thread after exec. PDEATHSIG belongs to
        // the original child thread, so inspect it here instead of querying
        // the newly created libtest thread (which correctly starts at zero).
        use std::os::unix::process::CommandExt;
        unsafe {
            command.pre_exec(|| {
                let mut signal: libc::c_int = 0;
                if libc::prctl(libc::PR_GET_PDEATHSIG, &mut signal) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if signal != libc::SIGKILL {
                    return Err(std::io::Error::from_raw_os_error(libc::EINVAL));
                }
                Ok(())
            });
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_probe_child() {
        if std::env::var_os("DEEPL_LIMITS_TEST_CHILD").is_none() {
            return;
        }
        for (resource, soft, hard) in [
            (libc::RLIMIT_AS, 2_147_483_648, 2_147_483_648),
            (libc::RLIMIT_CPU, 60, 61),
            (libc::RLIMIT_CORE, 0, 0),
            (libc::RLIMIT_FSIZE, 134_217_728, 134_217_728),
        ] {
            let mut limit: libc::rlimit = unsafe { std::mem::zeroed() };
            assert_eq!(unsafe { libc::getrlimit(resource, &mut limit) }, 0);
            assert_eq!(
                (limit.rlim_cur, limit.rlim_max),
                (soft, hard),
                "resource {resource}"
            );
        }
        assert_eq!(
            unsafe { libc::prctl(libc::PR_GET_NO_NEW_PRIVS, 0, 0, 0, 0) },
            1
        );
        assert_eq!(unsafe { libc::getpgrp() }, unsafe { libc::getpid() });
        let descriptor: i32 = std::env::var("DEEPL_LIMITS_TEST_FD")
            .unwrap()
            .parse()
            .unwrap();
        assert_eq!(unsafe { libc::fcntl(descriptor, libc::F_GETFD) }, -1);
        assert_eq!(
            std::io::Error::last_os_error().raw_os_error(),
            Some(libc::EBADF)
        );
    }
    #[cfg(windows)]
    #[test]
    fn windows_job_limits_are_assigned_and_drop_terminates_worker() {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::{
            Foundation::WAIT_OBJECT_0,
            System::{JobObjects::*, Threading::WaitForSingleObject},
        };
        let name = concat!(module_path!(), "::windows_waiting_child");
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", name.split_once("::").unwrap().1])
            .env("DEEPL_LIMITS_TEST_CHILD", "1")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        configure(&mut command).unwrap();
        let mut child = command.spawn().unwrap();
        let limit = attach(&child).unwrap();
        let mut observed = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        assert_ne!(
            unsafe {
                QueryInformationJobObject(
                    limit._job.as_raw_handle(),
                    JobObjectExtendedLimitInformation,
                    (&mut observed as *mut JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                    std::mem::size_of_val(&observed) as u32,
                    std::ptr::null_mut(),
                )
            },
            0
        );
        assert_eq!(observed.ProcessMemoryLimit, 2_147_483_648);
        assert_eq!(observed.BasicLimitInformation.ActiveProcessLimit, 1);
        assert_eq!(
            observed.BasicLimitInformation.PerProcessUserTimeLimit,
            600_000_000
        );
        assert_eq!(
            observed.BasicLimitInformation.LimitFlags,
            JOB_OBJECT_LIMIT_PROCESS_MEMORY
                | JOB_OBJECT_LIMIT_PROCESS_TIME
                | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
                | JOB_OBJECT_LIMIT_ACTIVE_PROCESS
        );
        let mut in_job = 0;
        assert_ne!(
            unsafe {
                IsProcessInJob(
                    child.as_raw_handle(),
                    limit._job.as_raw_handle(),
                    &mut in_job,
                )
            },
            0
        );
        assert_ne!(in_job, 0);
        drop(limit);
        assert_eq!(
            unsafe { WaitForSingleObject(child.as_raw_handle(), 5_000) },
            WAIT_OBJECT_0
        );
        assert!(!child.wait().unwrap().success());
    }

    #[cfg(windows)]
    #[test]
    fn windows_waiting_child() {
        if std::env::var_os("DEEPL_LIMITS_TEST_CHILD").is_none() {
            return;
        }
        use std::io::Read;
        let mut byte = [0];
        std::io::stdin().read_exact(&mut byte).unwrap();
    }
}
