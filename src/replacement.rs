//! Native selection replacement. No target handles or arbitrary text come from IPC.
#[derive(Clone, Copy)]
pub struct Target {
    #[cfg(target_os = "windows")]
    hwnd: isize,
    #[cfg(target_os = "windows")]
    focus: isize,
    #[cfg(target_os = "windows")]
    process_id: u32,
}

#[cfg(not(target_os = "windows"))]
pub fn capture() -> Option<Target> {
    None
}
#[cfg(not(target_os = "windows"))]
pub fn replace(
    _target: Target,
    _original: &str,
    _text: String,
    _current: impl Fn() -> bool,
) -> Result<(), String> {
    Err("Ersetzen ist auf diesem Desktop nicht verfügbar. Bitte Kopieren verwenden.".into())
}

#[cfg(target_os = "windows")]
mod windows {
    use super::Target;
    use std::time::{Duration, Instant};
    use windows_sys::Win32::{
        Foundation::HWND,
        System::DataExchange::GetClipboardSequenceNumber,
        UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
    };

    fn focus(hwnd: HWND) -> Option<HWND> {
        let thread = unsafe { GetWindowThreadProcessId(hwnd, std::ptr::null_mut()) };
        let mut info: GUITHREADINFO = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<GUITHREADINFO>() as u32;
        if thread != 0
            && unsafe { GetGUIThreadInfo(thread, &mut info) } != 0
            && !info.hwndFocus.is_null()
        {
            Some(info.hwndFocus)
        } else {
            None
        }
    }
    pub fn capture() -> Option<Target> {
        let hwnd = unsafe { GetForegroundWindow() };
        if hwnd.is_null() {
            return None;
        }
        let mut process_id = 0;
        unsafe { GetWindowThreadProcessId(hwnd, &mut process_id) };
        if process_id == 0 || process_id == std::process::id() {
            return None;
        }
        Some(Target {
            hwnd: hwnd as isize,
            focus: focus(hwnd)? as isize,
            process_id,
        })
    }
    fn active(target: Target) -> bool {
        let hwnd = target.hwnd as HWND;
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        (unsafe { IsWindow(hwnd) != 0 && GetForegroundWindow() == hwnd })
            && pid == target.process_id
            && focus(hwnd).is_some_and(|f| f as isize == target.focus)
    }
    fn key(vk: u16, up: bool) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: 0,
                    dwFlags: if up { KEYEVENTF_KEYUP } else { 0 },
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }
    fn chord(vk: u16) -> Result<(), String> {
        let inputs = [
            key(VK_CONTROL, false),
            key(vk, false),
            key(vk, true),
            key(VK_CONTROL, true),
        ];
        let count = unsafe {
            SendInput(
                inputs.len() as u32,
                inputs.as_ptr(),
                std::mem::size_of::<INPUT>() as i32,
            )
        };
        if count != inputs.len() as u32 {
            let release = key(VK_CONTROL, true);
            unsafe { SendInput(1, &release, std::mem::size_of::<INPUT>() as i32) };
            return Err(
                "Windows hat die Tastatureingabe blockiert. Bitte Kopieren verwenden.".into(),
            );
        }
        Ok(())
    }
    fn modifiers_down() -> bool {
        [VK_CONTROL, VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN]
            .into_iter()
            .any(|k| unsafe { GetAsyncKeyState(k as i32) } < 0)
    }
    pub fn replace(
        target: Target,
        original: &str,
        text: String,
        current: impl Fn() -> bool,
    ) -> Result<(), String> {
        let hwnd = target.hwnd as HWND;
        let mut pid = 0;
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        if unsafe { IsWindow(hwnd) } == 0 || pid != target.process_id {
            return Err("Das ursprüngliche Fenster ist nicht mehr verfügbar.".into());
        }
        let start = Instant::now();
        while modifiers_down() && start.elapsed() < Duration::from_millis(800) {
            std::thread::sleep(Duration::from_millis(20));
        }
        if modifiers_down() {
            return Err("Bitte alle Zusatztasten loslassen und erneut Ersetzen wählen.".into());
        }
        unsafe {
            if IsIconic(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            }
            SetForegroundWindow(hwnd);
        }
        let start = Instant::now();
        while !active(target) && start.elapsed() < Duration::from_millis(500) {
            std::thread::sleep(Duration::from_millis(15));
        }
        if !active(target) || !current() {
            return Err(
                "Der Fokus des ursprünglichen Fensters konnte nicht wiederhergestellt werden."
                    .into(),
            );
        }
        let before = unsafe { GetClipboardSequenceNumber() };
        chord(0x43)?;
        let start = Instant::now();
        let mut selected = None;
        while start.elapsed() < Duration::from_millis(1000) {
            if !active(target) || !current() {
                return Err(
                    "Die ursprüngliche Textauswahl wurde geändert. Bitte erneut Strg+C+C drücken."
                        .into(),
                );
            }
            let sequence = unsafe { GetClipboardSequenceNumber() };
            if sequence != before {
                // Never accept the old clipboard while the copied selection is still pending.
                // Delayed rendering may itself advance the sequence, so retry until stable.
                if let Ok(observed) = crate::clipboard::read_text()
                    && unsafe { GetClipboardSequenceNumber() } == sequence
                {
                    selected = Some(observed);
                    break;
                }
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let selected=selected.ok_or("Die ursprüngliche Textauswahl konnte nicht bestätigt werden. Bitte Kopieren verwenden.")?;
        crate::core::quick_session::verify_selection(original, &selected, current())
            .map_err(str::to_string)?;
        if !active(target) || modifiers_down() {
            return Err(
                "Der Fokus des ursprünglichen Fensters konnte nicht wiederhergestellt werden."
                    .into(),
            );
        }
        crate::clipboard::write_text(text).map_err(|e| e.to_string())?;
        // Commit only with the same foreground, focused control, and quick session.
        if !active(target) || !current() || modifiers_down() {
            return Err(
                "Die ursprüngliche Textauswahl wurde geändert. Bitte erneut Strg+C+C drücken."
                    .into(),
            );
        }
        chord(0x56)
    }
}
#[cfg(target_os = "windows")]
pub use windows::{capture, replace};
