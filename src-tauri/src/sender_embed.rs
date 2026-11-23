//! Embeds the managed Chrome sender window inside the Tauri app window on
//! Windows. The official Cast Web Sender SDK only runs in Chrome, so the Chrome
//! process is still required — but its window is re-parented into a panel of
//! the app instead of floating as a separate window.
//!
//! The DOM rect supplied by the UI is in CSS pixels relative to the WebView2
//! content area, so the Chrome window is parented to WebView2's own HWND and
//! the rect is scaled with the window DPI. That keeps the sender exactly inside
//! its panel regardless of decorations, title bar height or display scaling.

use std::sync::{Mutex, OnceLock};

#[cfg(windows)]
use windows::Win32::Foundation::{HWND, LPARAM};
#[cfg(windows)]
use windows::Win32::UI::HiDpi::GetDpiForWindow;
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, EnumWindows, GetClassNameW, GetWindowLongPtrW, GetWindowTextW,
    GetWindowThreadProcessId, IsWindow, SetParent, SetWindowLongPtrW, SetWindowPos,
    ShowWindow, GWL_EXSTYLE, GWL_STYLE, HWND_TOP, SW_HIDE, SW_RESTORE, SWP_FRAMECHANGED,
    SWP_NOACTIVATE, SWP_NOZORDER, SWP_SHOWWINDOW, SW_SHOWNOACTIVATE, WS_CHILD,
    WS_CLIPCHILDREN, WS_CLIPSIBLINGS, WS_EX_APPWINDOW, WS_EX_TOOLWINDOW,
    WS_OVERLAPPEDWINDOW, WS_POPUP,
};

static EMBEDDED_SENDER: OnceLock<Mutex<Option<isize>>> = OnceLock::new();

fn stored() -> &'static Mutex<Option<isize>> {
    EMBEDDED_SENDER.get_or_init(|| Mutex::new(None))
}

/// CSS-pixel rectangle of the host element, relative to the WebView2 viewport.
#[derive(Clone, Copy, Debug, serde::Deserialize)]
pub struct HostRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[cfg(windows)]
struct FindContext {
    pid: Option<u32>,
    found: Option<HWND>,
}

#[cfg(windows)]
unsafe extern "system" fn enum_top_window(hwnd: HWND, lparam: LPARAM) -> windows::core::BOOL {
    let context = &mut *(lparam.0 as *mut FindContext);
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if context.pid.is_some_and(|wanted| wanted != pid) {
        return windows::Win32::Foundation::TRUE;
    }
    let mut class = [0u16; 128];
    let class_len = GetClassNameW(hwnd, &mut class);
    if class_len <= 0 || String::from_utf16_lossy(&class[..class_len as usize]) != "Chrome_WidgetWin_1" {
        return windows::Win32::Foundation::TRUE;
    }
    let mut title = [0u16; 256];
    let title_len = GetWindowTextW(hwnd, &mut title).max(0) as usize;
    let title = String::from_utf16_lossy(&title[..title_len]);
    if title.contains("Cast Sender") {
        context.found = Some(hwnd);
        return windows::Win32::Foundation::FALSE;
    }
    windows::Win32::Foundation::TRUE
}

#[cfg(windows)]
unsafe extern "system" fn enum_webview_child(hwnd: HWND, lparam: LPARAM) -> windows::core::BOOL {
    let context = &mut *(lparam.0 as *mut FindContext);
    let mut class = [0u16; 128];
    let class_len = GetClassNameW(hwnd, &mut class);
    if class_len > 0 && String::from_utf16_lossy(&class[..class_len as usize]) == "Chrome_WidgetWin_1" {
        context.found = Some(hwnd);
        return windows::Win32::Foundation::FALSE;
    }
    windows::Win32::Foundation::TRUE
}

#[cfg(windows)]
fn find_sender_window(pid: Option<u32>) -> Option<HWND> {
    unsafe {
        let mut context = FindContext { pid, found: None };
        let _ = EnumWindows(Some(enum_top_window), LPARAM(&mut context as *mut _ as isize));
        context.found
    }
}

#[cfg(windows)]
fn webview_hwnd(top_level: HWND) -> HWND {
    unsafe {
        let mut context = FindContext { pid: None, found: None };
        let _ = EnumChildWindows(Some(top_level), Some(enum_webview_child), LPARAM(&mut context as *mut _ as isize));
        context.found.unwrap_or(top_level)
    }
}

#[cfg(windows)]
fn scaled_rect(host: HWND, rect: HostRect) -> (i32, i32, i32, i32) {
    let dpi = unsafe { GetDpiForWindow(host) }.max(96);
    let scale = dpi as f32 / 96.0;
    (
        (rect.x as f32 * scale).round() as i32,
        (rect.y as f32 * scale).round() as i32,
        (rect.width as f32 * scale).max(1.0) as i32,
        (rect.height as f32 * scale).max(1.0) as i32,
    )
}

#[cfg(windows)]
fn place(hwnd: HWND, host: HWND, rect: HostRect, bring_to_front: bool) -> Result<(), String> {
    let (x, y, width, height) = scaled_rect(host, rect);
    let insert_after = if bring_to_front { Some(HWND_TOP) } else { None };
    let mut flags = SWP_FRAMECHANGED | SWP_NOACTIVATE | SWP_SHOWWINDOW;
    if !bring_to_front {
        flags |= SWP_NOZORDER;
    }
    unsafe {
        SetWindowPos(hwnd, insert_after, x, y, width, height, flags)
            .map_err(|error| format!("Could not position the Cast sender window ({error})."))?;
    }
    Ok(())
}

/// Finds the Chrome sender window and embeds it into the app's WebView2 panel.
/// `pid` is the Chrome browser process started by the app and is used as a
/// fallback if the window title changes.
#[cfg(windows)]
pub fn embed(top_level_hwnd: isize, pid: Option<u32>, rect: HostRect) -> Result<(), String> {
    if top_level_hwnd == 0 {
        return Err("The app window handle is unavailable.".to_string());
    }
    let top_level = HWND(top_level_hwnd as *mut std::ffi::c_void);
    let mut sender = None;
    for _ in 0..25 {
        sender = find_sender_window(pid);
        if sender.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    let hwnd = sender.ok_or_else(|| "The Chrome sender window was not found yet. Try opening it again.".to_string())?;
    let host = webview_hwnd(top_level);

    unsafe {
        let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
        let style = (style & !(WS_POPUP.0 as isize | WS_OVERLAPPEDWINDOW.0 as isize))
            | (WS_CHILD.0 | WS_CLIPSIBLINGS.0 | WS_CLIPCHILDREN.0) as isize;
        let _ = SetWindowLongPtrW(hwnd, GWL_STYLE, style);

        let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let ex_style = (ex_style & !(WS_EX_APPWINDOW.0 as isize)) | WS_EX_TOOLWINDOW.0 as isize;
        let _ = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex_style);

        let _ = ShowWindow(hwnd, SW_RESTORE);
        SetParent(hwnd, Some(host))
            .map_err(|error| format!("Could not embed the Cast sender window ({error})."))?;
        place(hwnd, host, rect, true)?;
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }

    if let Ok(mut guard) = stored().lock() {
        *guard = Some(hwnd.0 as isize);
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn embed(_top_level_hwnd: isize, _pid: Option<u32>, _rect: HostRect) -> Result<(), String> {
    Err("Embedding the Cast sender window is only available on Windows.".to_string())
}

fn stored_window() -> Option<isize> {
    stored().lock().ok().and_then(|guard| *guard)
}

/// Shows an already embedded sender and repositions it in its panel.
#[cfg(windows)]
pub fn show_embedded(top_level_hwnd: isize, rect: HostRect) -> Result<(), String> {
    if top_level_hwnd == 0 {
        return Err("The app window handle is unavailable.".to_string());
    }
    let Some(handle) = stored_window() else {
        return embed(top_level_hwnd, None, rect);
    };
    let hwnd = HWND(handle as *mut std::ffi::c_void);
    let host = webview_hwnd(HWND(top_level_hwnd as *mut std::ffi::c_void));
    if !unsafe { IsWindow(Some(hwnd)).as_bool() } {
        if let Ok(mut guard) = stored().lock() {
            *guard = None;
        }
        return embed(top_level_hwnd, None, rect);
    }
    let _ = unsafe { ShowWindow(hwnd, SW_RESTORE) };
    place(hwnd, host, rect, true)?;
    let _ = unsafe { ShowWindow(hwnd, SW_SHOWNOACTIVATE) };
    Ok(())
}

#[cfg(not(windows))]
pub fn show_embedded(top_level_hwnd: isize, rect: HostRect) -> Result<(), String> {
    embed(top_level_hwnd, None, rect)
}

/// Repositions an embedded sender when the host panel moves or resizes.
#[cfg(windows)]
pub fn move_embedded(top_level_hwnd: isize, rect: HostRect) -> Result<(), String> {
    let Some(handle) = stored_window() else {
        return Ok(());
    };
    if top_level_hwnd == 0 {
        return Ok(());
    }
    let hwnd = HWND(handle as *mut std::ffi::c_void);
    if !unsafe { IsWindow(Some(hwnd)).as_bool() } {
        if let Ok(mut guard) = stored().lock() {
            *guard = None;
        }
        return Ok(());
    }
    let host = webview_hwnd(HWND(top_level_hwnd as *mut std::ffi::c_void));
    place(hwnd, host, rect, false)
}

#[cfg(not(windows))]
pub fn move_embedded(_top_level_hwnd: isize, _rect: HostRect) -> Result<(), String> {
    Ok(())
}

/// Hides the embedded sender. The Chrome process and Cast session stay alive,
/// so playback continues and reopening the panel is instant.
#[cfg(windows)]
pub fn hide_embedded() -> Result<(), String> {
    let Some(handle) = stored_window() else {
        return Ok(());
    };
    let hwnd = HWND(handle as *mut std::ffi::c_void);
    if unsafe { IsWindow(Some(hwnd)).as_bool() } {
        let _ = unsafe { ShowWindow(hwnd, SW_HIDE) };
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn hide_embedded() -> Result<(), String> {
    Ok(())
}

/// Unparents and hides the sender before the Chrome process is killed.
#[cfg(windows)]
pub fn detach_embedded() {
    let Some(handle) = stored_window() else {
        return;
    };
    let hwnd = HWND(handle as *mut std::ffi::c_void);
    if unsafe { IsWindow(Some(hwnd)).as_bool() } {
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
            let _ = SetParent(hwnd, None);
        }
    }
    if let Ok(mut guard) = stored().lock() {
        *guard = None;
    }
}

#[cfg(not(windows))]
pub fn detach_embedded() {}

/// True when the sender is currently a live child window.
#[cfg(windows)]
pub fn embedded_window_is_valid() -> bool {
    stored_window().is_some_and(|handle| unsafe {
        IsWindow(Some(HWND(handle as *mut std::ffi::c_void))).as_bool()
    })
}

#[cfg(not(windows))]
pub fn embedded_window_is_valid() -> bool {
    false
}
