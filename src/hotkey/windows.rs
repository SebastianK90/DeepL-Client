use super::DoubleCopy;
use std::{
    sync::{Mutex, OnceLock, mpsc},
    time::Instant,
};
use windows_sys::Win32::{
    Foundation::{LPARAM, LRESULT, WPARAM},
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        Input::KeyboardAndMouse::{
            GetAsyncKeyState, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT,
        },
        WindowsAndMessaging::*,
    },
};

static STATE: OnceLock<Mutex<(DoubleCopy, mpsc::SyncSender<()>)>> = OnceLock::new();

unsafe extern "system" fn callback(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let event = unsafe { &*(lparam as *const KBDLLHOOKSTRUCT) };
        if event.flags & LLKHF_INJECTED == 0 {
            let down = wparam as u32 == WM_KEYDOWN || wparam as u32 == WM_SYSKEYDOWN;
            let up = wparam as u32 == WM_KEYUP || wparam as u32 == WM_SYSKEYUP;
            if (down || up)
                && let Some(state) = STATE.get()
                && let Ok(mut state) = state.lock()
            {
                let ctrl = unsafe { GetAsyncKeyState(VK_CONTROL as i32) } < 0;
                let extra = [VK_SHIFT, VK_MENU, VK_LWIN, VK_RWIN]
                    .iter()
                    .any(|key| unsafe { GetAsyncKeyState(*key as i32) } < 0);
                if event.vkCode == VK_CONTROL as u32 || event.vkCode == 0xa2 || event.vkCode == 0xa3
                {
                    if up {
                        state.0.reset();
                    }
                } else if state
                    .0
                    .event(event.vkCode == 0x43, down, ctrl && !extra, Instant::now())
                {
                    let _ = state.1.try_send(());
                }
            }
        }
    }
    unsafe { CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam) }
}

pub fn start(app: tauri::AppHandle, ms: u64) {
    let (tx, rx) = mpsc::sync_channel(1);
    let _ = STATE.set(Mutex::new((DoubleCopy::new(ms), tx)));
    let hook_app = app.clone();
    std::thread::spawn(move || {
        // Callback only updates a small state machine. Clipboard and UI run elsewhere.
        let hook = unsafe {
            SetWindowsHookExW(
                WH_KEYBOARD_LL,
                Some(callback),
                GetModuleHandleW(std::ptr::null()),
                0,
            )
        };
        if hook.is_null() {
            crate::ui::set_hotkey_status(
                &hook_app,
                "Windows-Tastaturhook konnte nicht registriert werden.",
            );
            return;
        }
        crate::ui::set_hotkey_status(&hook_app, &format!("Ctrl+C+C · {ms} ms (Windows)"));
        let mut message = unsafe { std::mem::zeroed() };
        while unsafe { GetMessageW(&mut message, std::ptr::null_mut(), 0, 0) } > 0 {
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        unsafe {
            UnhookWindowsHookEx(hook);
        }
    });
    std::thread::spawn(move || {
        while rx.recv().is_ok() {
            // Leave the original copy shortcut untouched and allow its clipboard write.
            crate::ui::quick_from_clipboard(app.clone());
        }
    });
}
