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
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, POINT, RECT};
#[cfg(windows)]
use windows::Win32::Graphics::Gdi::{ClientToScreen, ScreenToClient};
#[cfg(windows)]
use windows::Win32::UI::HiDpi::GetDpiForWindow;
#[cfg(windows)]
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEINPUT,
};
#[cfg(windows)]
use windows::Win32::UI::WindowsAndMessaging::{
    EnumChildWindows, EnumWindows, GetAncestor, GetClassNameW, GetCursorPos, GetParent,
    GetWindowLongPtrW, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, IsWindow,
    SetCursorPos, SetForegroundWindow, SetLayeredWindowAttributes, SetParent, SetWindowLongPtrW,
    SetWindowPos, ShowWindow, GA_ROOT, GWL_EXSTYLE, GWL_STYLE, HWND_TOP, LWA_ALPHA, SW_HIDE,
    SW_RESTORE, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOZORDER, SWP_SHOWWINDOW, SW_SHOWNOACTIVATE,
    WS_CHILD, WS_CLIPCHILDREN,
    WS_CLIPSIBLINGS, WS_EX_APPWINDOW, WS_EX_LAYERED, WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT,
    WS_OVERLAPPEDWINDOW, WS_POPUP,
};

static EMBEDDED_SENDER: OnceLock<Mutex<Option<isize>>> = OnceLock::new();
/// Original 1×1 geometry saved while the sender is temporarily enlarged for a
/// trusted Cast picker click. Restored when the panel is hidden again.
static SENDER_CLICK_RECT: OnceLock<Mutex<Option<(i32, i32, i32, i32)>>> = OnceLock::new();

fn stored() -> &'static Mutex<Option<isize>> {
    EMBEDDED_SENDER.get_or_init(|| Mutex::new(None))
}

fn click_rect() -> &'static Mutex<Option<(i32, i32, i32, i32)>> {
    SENDER_CLICK_RECT.get_or_init(|| Mutex::new(None))
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
        // Chrome often hands the app window to an already-running browser
        // process, so the window PID can differ from the launcher PID. Prefer
        // the launcher PID when it matches, but always fall back to the window
        // title so a real sender window is never left floating over the app.
        sender = find_sender_window(pid).or_else(|| find_sender_window(None));
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

/// Makes the embedded sender visually invisible while keeping its window alive.
///
/// `ShowWindow(SW_HIDE)` also hides Chrome's own receiver picker, which is
/// anchored to this window. A fully transparent layered child keeps the Cast
/// SDK running and lets Chrome show its native picker without ever exposing the
/// sender page inside the app.
#[cfg(windows)]
pub fn hide_embedded() -> Result<(), String> {
    let Some(handle) = stored_window() else {
        // Never hide a top-level sender window here. During a cold launch the
        // window exists before it has been embedded; hiding it would drop the
        // bridge connection and make `launch()` time out.
        return Ok(());
    };
    let hwnd = HWND(handle as *mut std::ffi::c_void);
    if !unsafe { IsWindow(Some(hwnd)).as_bool() } {
        if let Ok(mut guard) = stored().lock() {
            *guard = None;
        }
        return Ok(());
    }
    unsafe {
        let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let ex_style = (ex_style | WS_EX_LAYERED.0 as isize | WS_EX_TRANSPARENT.0 as isize)
            & !(WS_EX_APPWINDOW.0 as isize);
        let _ = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex_style);
        SetLayeredWindowAttributes(hwnd, COLORREF(0), 0, LWA_ALPHA)
            .map_err(|error| format!("Could not conceal the Cast sender window ({error})."))?;
        // Restore the 1×1 anchor after a picker click temporarily enlarged it.
        if let Ok(mut saved) = click_rect().lock() {
            if let Some((x, y, width, height)) = *saved {
                let _ = SetWindowPos(
                    hwnd,
                    None,
                    x,
                    y,
                    width,
                    height,
                    SWP_NOACTIVATE | SWP_NOZORDER,
                );
                *saved = None;
            }
        }
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn hide_embedded() -> Result<(), String> {
    Ok(())
}

/// Supplies a real user gesture to the hidden sender so the official Cast SDK
/// will open Chrome's receiver picker.
///
/// The Web Sender SDK refuses `requestSession()` unless it is called from a
/// user gesture in the Chrome page. A bridge command from the app is not a
/// gesture, so the page arms the selected receiver first and this function
/// sends an OS-level click to an invisible button in the embedded page. The
/// window is briefly made hit-testable (alpha 1, no `WS_EX_TRANSPARENT`) and
/// widened to the left of its 1×1 anchor so the click lands on screen, then it
/// is click-through again. `hide_embedded` restores the 1×1 anchor and full
/// transparency when the device menu closes.
#[cfg(windows)]
pub fn click_cast_trigger() -> Result<(), String> {
    let Some(handle) = stored_window() else {
        return Err("The Cast sender is not embedded in this window yet.".to_string());
    };
    let hwnd = HWND(handle as *mut std::ffi::c_void);
    if !unsafe { IsWindow(Some(hwnd)).as_bool() } {
        if let Ok(mut guard) = stored().lock() {
            *guard = None;
        }
        return Err("The Cast sender window is no longer available.".to_string());
    }

    unsafe {
        let mut original_rect = RECT::default();
        GetWindowRect(hwnd, &mut original_rect)
            .map_err(|error| format!("Could not read the Cast sender position ({error})."))?;
        let original_width = original_rect.right - original_rect.left;
        let original_height = original_rect.bottom - original_rect.top;
        let parent = GetParent(hwnd).unwrap_or_default();
        // `GetWindowRect` is screen-relative but `SetWindowPos` on a child is
        // parent-relative. Keep both forms so the trigger and the later 1×1
        // restore stay correct on any monitor or DPI.
        let mut original_parent_point = POINT { x: original_rect.left, y: original_rect.top };
        if parent.0 as isize != 0 {
            let _ = ScreenToClient(parent, &mut original_parent_point);
        }
        let mut original_cursor = POINT::default();
        let _ = GetCursorPos(&mut original_cursor);
        let _ = SetForegroundWindow(GetAncestor(hwnd, GA_ROOT));

        let original_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let clickable_style =
            (original_style & !(WS_EX_TRANSPARENT.0 as isize)) | WS_EX_LAYERED.0 as isize;
        let _ = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, clickable_style);
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 1, LWA_ALPHA);

        // Grow leftwards from the 1×1 anchor so the whole trigger stays on
        // screen when the app is docked against the right edge. SWP_FRAMECHANGED
        // applies the extended-style change; without it Windows keeps treating
        // the layered child as click-through and Chrome never sees the click.
        let mut click_parent = POINT {
            x: original_rect.right.saturating_sub(240),
            y: original_rect.top,
        };
        if parent.0 as isize != 0 {
            let _ = ScreenToClient(parent, &mut click_parent);
        }
        let _ = SetWindowPos(
            hwnd,
            Some(HWND_TOP),
            click_parent.x,
            click_parent.y,
            240,
            160,
            SWP_FRAMECHANGED | SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );

        // Chrome still renders a ~30 px caption inside this child HWND, so the
        // web page starts below it. Aim inside the hidden button, not the bar.
        let mut target = POINT { x: 40, y: 72 };
        let _ = ClientToScreen(hwnd, &mut target);
        let _ = SetCursorPos(target.x, target.y);

        let input_size = std::mem::size_of::<INPUT>() as i32;
        let down = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dwFlags: MOUSEEVENTF_LEFTDOWN,
                    ..Default::default()
                },
            },
        };
        let up = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dwFlags: MOUSEEVENTF_LEFTUP,
                    ..Default::default()
                },
            },
        };
        let down_sent = SendInput(&[down], input_size);
        std::thread::sleep(std::time::Duration::from_millis(60));
        let up_sent = SendInput(&[up], input_size);
        // Let Chrome dispatch the trusted click before making the sender
        // click-through again. Keep the enlarged, nearly transparent window
        // while the native picker is open; `hide_embedded` restores its 1×1
        // geometry and full transparency when the device menu closes.
        std::thread::sleep(std::time::Duration::from_millis(250));

        if let Ok(mut saved) = click_rect().lock() {
            *saved = Some((original_parent_point.x, original_parent_point.y, original_width, original_height));
        }
        let _ = SetWindowLongPtrW(hwnd, GWL_EXSTYLE, original_style);
        let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 1, LWA_ALPHA);
        let _ = SetWindowPos(
            hwnd,
            None,
            click_parent.x,
            click_parent.y,
            240,
            160,
            SWP_FRAMECHANGED | SWP_NOACTIVATE | SWP_NOZORDER,
        );
        let _ = SetCursorPos(original_cursor.x, original_cursor.y);

        if down_sent != 1 || up_sent != 1 {
            return Err("Could not send the Cast picker click to the embedded sender.".to_string());
        }
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn click_cast_trigger() -> Result<(), String> {
    Err("Sending the Cast picker click is only available on Windows.".to_string())
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
