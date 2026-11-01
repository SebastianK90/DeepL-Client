//! In-process Win32 selector. Pixels never leave RAM or enter the clipboard.
use super::selection::{Point, Region, buffer_len, crop};
use anyhow::{Result, ensure};
use std::{
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::{ERROR_CLASS_ALREADY_EXISTS, GetLastError, HWND, LPARAM, LRESULT, RECT, WPARAM},
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{
        HiDpi::*,
        Input::KeyboardAndMouse::{ReleaseCapture, SetCapture, SetFocus, VK_ESCAPE},
        WindowsAndMessaging::*,
    },
};

struct Desktop {
    width: i32,
    height: i32,
    x: i32,
    y: i32,
    pixels: Vec<u8>,
    dimmed: Vec<u8>,
}
struct Selection {
    desktop: Desktop,
    start: Option<Point>,
    end: Point,
    result: Option<Region>,
    hint: Vec<u16>,
    armed: bool,
    destroyed: bool,
    click_select: bool,
}
struct DpiContext(DPI_AWARENESS_CONTEXT);
impl Drop for DpiContext {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                SetThreadDpiAwarenessContext(self.0);
            }
        }
    }
}
struct Window(HWND);
impl Drop for Window {
    fn drop(&mut self) {
        unsafe {
            if IsWindow(self.0) != 0 {
                DestroyWindow(self.0);
            }
        }
    }
}

pub fn select(hint: &str) -> Result<Option<arboard::ImageData<'static>>> {
    unsafe {
        let _dpi = DpiContext(SetThreadDpiAwarenessContext(
            DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        ));
        let desktop = snapshot()?;
        let (x, y, width, height) = (desktop.x, desktop.y, desktop.width, desktop.height);
        let mut state = Box::new(Selection {
            desktop,
            start: None,
            end: Point { x: 0, y: 0 },
            result: None,
            hint: hint.encode_utf16().collect(),
            armed: false,
            destroyed: false,
            click_select: false,
        });
        let instance = GetModuleHandleW(null());
        let class = windows_sys::w!("DeepLClientLocalCapture");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(procedure),
            hInstance: instance,
            lpszClassName: class,
            hCursor: LoadCursorW(null_mut(), IDC_CROSS),
            ..zeroed()
        };
        ensure!(
            RegisterClassW(&wc) != 0 || GetLastError() == ERROR_CLASS_ALREADY_EXISTS,
            "Screenshot-Erfassung fehlgeschlagen."
        );
        let style = WS_EX_TOPMOST | WS_EX_TOOLWINDOW;
        let handle = CreateWindowExW(
            style,
            class,
            windows_sys::w!("DeepL Client – Screenshot"),
            WS_POPUP,
            x,
            y,
            width,
            height,
            null_mut(),
            null_mut(),
            instance,
            (&mut *state as *mut Selection).cast(),
        );
        ensure!(!handle.is_null(), "Screenshot-Erfassung fehlgeschlagen.");
        let _window = Window(handle);
        ShowWindow(handle, SW_SHOW);
        SetForegroundWindow(handle);
        SetFocus(handle);
        UpdateWindow(handle);
        SetTimer(handle, 1, 120000, None);
        if state.destroyed {
            return Ok(None);
        }
        state.armed = true;
        let mut message = zeroed();
        loop {
            let status = GetMessageW(&mut message, null_mut(), 0, 0);
            if status == 0 {
                break;
            }
            ensure!(status != -1, "Screenshot-Erfassung fehlgeschlagen.");
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        match state.result {
            Some(r) => crop(&state.desktop.pixels, width, height, r).map(Some),
            None => Ok(None),
        }
    }
}

unsafe fn snapshot() -> Result<Desktop> {
    unsafe {
        let (width, height, x, y) = (
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
        );
        let length = buffer_len(width, height)?;
        let screen = GetDC(null_mut());
        ensure!(!screen.is_null(), "Screenshot-Erfassung fehlgeschlagen.");
        let memory = CreateCompatibleDC(screen);
        let bitmap = CreateCompatibleBitmap(screen, width, height);
        // All handles are released on every path, including allocation/read errors.
        let result = (|| -> Result<Vec<u8>> {
            ensure!(
                !memory.is_null() && !bitmap.is_null(),
                "Screenshot-Erfassung fehlgeschlagen."
            );
            let previous = SelectObject(memory, bitmap);
            let copied = BitBlt(
                memory,
                0,
                0,
                width,
                height,
                screen,
                x,
                y,
                SRCCOPY | CAPTUREBLT,
            );
            SelectObject(memory, previous);
            ensure!(copied != 0, "Screenshot-Erfassung fehlgeschlagen.");
            let mut pixels = vec![0; length];
            let mut info = bitmap_info(width, height);
            ensure!(
                GetDIBits(
                    memory,
                    bitmap,
                    0,
                    height as u32,
                    pixels.as_mut_ptr().cast(),
                    &mut info,
                    DIB_RGB_COLORS
                ) == height,
                "Screenshot-Erfassung fehlgeschlagen."
            );
            Ok(pixels)
        })();
        if !bitmap.is_null() {
            DeleteObject(bitmap);
        }
        if !memory.is_null() {
            DeleteDC(memory);
        }
        ReleaseDC(null_mut(), screen);
        let pixels = result?;
        let mut dimmed = pixels.clone();
        for pixel in dimmed.as_chunks_mut::<4>().0 {
            for channel in &mut pixel[..3] {
                *channel = (*channel as u16 * 45 / 100) as u8;
            }
        }
        Ok(Desktop {
            width,
            height,
            x,
            y,
            pixels,
            dimmed,
        })
    }
}
fn bitmap_info(width: i32, height: i32) -> BITMAPINFO {
    BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: width,
            biHeight: -height,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..unsafe { zeroed() }
        },
        ..unsafe { zeroed() }
    }
}
fn pointer(lparam: LPARAM) -> Point {
    Point {
        x: (lparam as u16 as i16) as i32,
        y: ((lparam >> 16) as u16 as i16) as i32,
    }
}

unsafe extern "system" fn procedure(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    unsafe {
        // These reentrant messages must never borrow Selection while a mouse
        // handler is finishing, or while BeginPaint dispatches background erase.
        match message {
            WM_NCCREATE => {
                let create = &*(lparam as *const CREATESTRUCTW);
                SetWindowLongPtrW(window, GWLP_USERDATA, create.lpCreateParams as isize);
                return 1;
            }
            WM_DESTROY => {
                let data = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut Selection;
                if !data.is_null() {
                    (*data).destroyed = true;
                }
                PostQuitMessage(0);
                return 0;
            }
            WM_NCDESTROY => {
                SetWindowLongPtrW(window, GWLP_USERDATA, 0);
                return DefWindowProcW(window, message, wparam, lparam);
            }
            WM_ERASEBKGND => return 1,
            WM_KILLFOCUS => {
                let data = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut Selection;
                if !data.is_null() && (*data).armed {
                    DestroyWindow(window);
                }
                return 0;
            }
            WM_CLOSE | WM_TIMER => {
                DestroyWindow(window);
                return 0;
            }
            WM_KEYDOWN if wparam == VK_ESCAPE as usize => {
                DestroyWindow(window);
                return 0;
            }
            WM_RBUTTONDOWN => {
                DestroyWindow(window);
                return 0;
            }
            _ => {}
        }
        let data = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut Selection;
        if data.is_null() {
            return DefWindowProcW(window, message, wparam, lparam);
        }
        match message {
            WM_PAINT => {
                let mut ps = zeroed();
                let dc = BeginPaint(window, &mut ps);
                paint(dc, &*data);
                EndPaint(window, &ps);
                0
            }
            WM_LBUTTONDOWN => {
                {
                    let state = &mut *data;
                    if !state.click_select {
                        state.start = Some(pointer(lparam));
                    }
                    state.click_select = false;
                    state.end = pointer(lparam);
                }
                SetCapture(window);
                InvalidateRect(window, null(), 0);
                0
            }
            WM_MOUSEMOVE => {
                let dragging = {
                    let state = &mut *data;
                    state.end = pointer(lparam);
                    state.start.is_some()
                };
                if dragging {
                    InvalidateRect(window, null(), 0);
                }
                0
            }
            WM_LBUTTONUP => {
                let complete = {
                    let state = &mut *data;
                    state.end = pointer(lparam);
                    state.result = state.start.and_then(|p| {
                        Region::between(p, state.end, state.desktop.width, state.desktop.height)
                    });
                    state.click_select = state.result.is_none();
                    if state.result.is_some() {
                        state.start = None;
                    }
                    state.result.is_some()
                };
                ReleaseCapture();
                if complete {
                    DestroyWindow(window);
                } else {
                    InvalidateRect(window, null(), 0);
                }
                0
            }
            WM_CAPTURECHANGED => {
                let dragging = (*data).start.is_some() && !(*data).click_select;
                if dragging {
                    DestroyWindow(window);
                }
                0
            }
            _ => DefWindowProcW(window, message, wparam, lparam),
        }
    }
}
unsafe fn paint(dc: HDC, state: &Selection) {
    unsafe {
        let desktop = &state.desktop;
        let info = bitmap_info(desktop.width, desktop.height);
        StretchDIBits(
            dc,
            0,
            0,
            desktop.width,
            desktop.height,
            0,
            0,
            desktop.width,
            desktop.height,
            desktop.dimmed.as_ptr().cast(),
            &info,
            DIB_RGB_COLORS,
            SRCCOPY,
        );
        if let Some(r) = state
            .start
            .and_then(|p| Region::between(p, state.end, desktop.width, desktop.height))
        {
            let saved = SaveDC(dc);
            IntersectClipRect(dc, r.left, r.top, r.right, r.bottom);
            StretchDIBits(
                dc,
                0,
                0,
                desktop.width,
                desktop.height,
                0,
                0,
                desktop.width,
                desktop.height,
                desktop.pixels.as_ptr().cast(),
                &info,
                DIB_RGB_COLORS,
                SRCCOPY,
            );
            RestoreDC(dc, saved);
            let rect = RECT {
                left: r.left,
                top: r.top,
                right: r.right,
                bottom: r.bottom,
            };
            FrameRect(dc, &rect, GetStockObject(WHITE_BRUSH) as HBRUSH);
        }
        SetBkMode(dc, TRANSPARENT as i32);
        SetTextColor(dc, 0x00ffffff);
        let font = CreateFontW(
            22,
            0,
            0,
            0,
            FW_NORMAL as i32,
            0,
            0,
            0,
            DEFAULT_CHARSET as u32,
            OUT_DEFAULT_PRECIS as u32,
            CLIP_DEFAULT_PRECIS as u32,
            ANTIALIASED_QUALITY as u32,
            DEFAULT_PITCH as u32,
            windows_sys::w!("Segoe UI"),
        );
        let previous = SelectObject(dc, font);
        TextOutW(dc, 28, 24, state.hint.as_ptr(), state.hint.len() as i32);
        SelectObject(dc, previous);
        DeleteObject(font);
    }
}
