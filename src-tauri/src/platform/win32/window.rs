//! Window behaviour Tauri does not expose the way the overlay needs it.

use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
use windows::Win32::UI::WindowsAndMessaging::*;

fn hwnd_of(window: &tauri::WebviewWindow) -> Option<HWND> {
    window.hwnd().ok().map(|h| HWND(h.0))
}

/// Where the mouse pointer is, in physical screen pixels. The click-through
/// lock uses it to keep its own button clickable.
pub fn cursor_position() -> Option<(i32, i32)> {
    let mut point = POINT::default();
    unsafe { GetCursorPos(&mut point) }.ok()?;
    Some((point.x, point.y))
}

/// Begin dragging the window, as if its title bar had been grabbed.
pub fn start_drag(window: &tauri::WebviewWindow) {
    use windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
    let Some(hwnd) = hwnd_of(window) else { return };
    unsafe {
        let _ = ReleaseCapture();
        const HTCAPTION: usize = 2;
        let _ = PostMessageW(Some(hwnd), WM_NCLBUTTONDOWN, WPARAM(HTCAPTION), LPARAM(0));
    }
}

/// Bring the overlay back on top without taking focus from the game.
pub fn show_on_top_without_focus(window: &tauri::WebviewWindow) {
    let Some(hwnd) = hwnd_of(window) else { return };
    unsafe {
        let _ = ShowWindow(hwnd, SW_SHOWNOACTIVATE);
        let _ = SetWindowPos(
            hwnd, Some(HWND_TOPMOST),
            0, 0, 0, 0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
    }
}

/// Hide a window that `show_on_top_without_focus` shows, the same way: Tauri's
/// own show and hide go through tao, which keeps a visible flag of its own.
/// A raw show leaves that flag "hidden", so a tao hide after it changed
/// nothing (the window stayed up), and a tao show takes focus. Shown and
/// hidden raw both ways, tao's flag stays "visible" from the window's
/// creation (`HIDE_AFTER_BUILD`).
pub fn hide_without_focus(window: &tauri::WebviewWindow) {
    let Some(hwnd) = hwnd_of(window) else { return };
    unsafe {
        let _ = ShowWindow(hwnd, SW_HIDE);
    }
}

/// The window in front now (the game, while playing), for `never_activate`
/// to give the front back to.
pub fn front_window() -> Option<isize> {
    let hwnd = unsafe { GetForegroundWindow() };
    (!hwnd.0.is_null()).then_some(hwnd.0 as isize)
}

/// Whether the window in front is the meter's own (any of its windows) or
/// the game's: what a tool window stays on top for (`keep_on_top_while_ours`).
pub fn front_is_ours_or_game() -> bool {
    use windows::Win32::System::Threading::GetCurrentProcessId;
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.0.is_null() {
        return true;
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
    pid == unsafe { GetCurrentProcessId() } || crate::platform::window_detector::is_aion2_foreground()
}

/// If `window` has the front, give it back to `before` (the window in front
/// when it was made): a widget's page has just loaded, which is when WebView2
/// can take it.
pub fn give_front_back(window: &tauri::WebviewWindow, before: Option<isize>) {
    let (Some(hwnd), Some(before)) = (hwnd_of(window), before) else { return };
    unsafe {
        if GetForegroundWindow() == hwnd && before != hwnd.0 as isize {
            let back = SetForegroundWindow(HWND(before as *mut _)).as_bool();
            tracing::info!("A widget's page took the front as it loaded; given back: {back}");
        }
    }
}

/// A small window that must never take the front from the game (a plugin's
/// alert icon): it still gets clicks and drags, but is never activated
/// (`WS_EX_NOACTIVATE`). Built with focus off, it still took the front as
/// its page was created (WebView2), and an alert icon appearing tabbed a
/// player out of the game (2026-10-18). For ten seconds after it is built,
/// if it has the front anyway, the front goes back to `before`: three were
/// too few on a busy PC, where WebView2 takes longer to start (Hirai, still
/// tabbed out on 2.0.71, borderless). Its page loading checks once more
/// (`give_front_back`).
pub fn never_activate(window: &tauri::WebviewWindow, before: Option<isize>) {
    let Some(hwnd) = hwnd_of(window) else { return };
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | WS_EX_NOACTIVATE.0 as isize);
    }
    let Some(before) = before else { return };
    let own = hwnd.0 as isize;
    if before == own {
        return;
    }
    std::thread::spawn(move || {
        for _ in 0..100 {
            std::thread::sleep(std::time::Duration::from_millis(100));
            unsafe {
                if GetForegroundWindow().0 as isize == own {
                    let back = SetForegroundWindow(HWND(before as *mut _)).as_bool();
                    tracing::info!("A widget took the front; given back to the window before it: {back}");
                }
            }
        }
    });
}

/// A window to be shown and hidden with the functions above is built visible
/// (without focus), so tao applies its frameless style and marks it visible,
/// then hidden with `hide_without_focus` if it should start hidden. Built
/// hidden, tao never styled it, and a raw show brought it up with a title bar.
pub const HIDE_AFTER_BUILD: bool = true;

/// Drop the overlay from the top and minimise it, without activating anything.
pub fn minimize_off_top(window: &tauri::WebviewWindow) {
    let Some(hwnd) = hwnd_of(window) else { return };
    unsafe {
        let _ = SetWindowPos(
            hwnd, Some(HWND_NOTOPMOST),
            0, 0, 0, 0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        );
        // SW_MINIMIZE activates the next window in the Z order: the game, even
        // with an overlay (NVIDIA's) in front, and the game then locks the
        // mouse to the middle of the screen.
        let _ = ShowWindow(hwnd, SW_SHOWMINNOACTIVE);
    }
}

pub fn set_size(window: &tauri::WebviewWindow, size: tauri::Size) {
    let _ = window.set_size(size);
}

pub fn release_size(_window: &tauri::WebviewWindow, _min: tauri::LogicalSize<f64>) {}

pub fn primary_button_down() -> Option<bool> {
    None
}

pub fn compositor_resize_supported(_window: &tauri::WebviewWindow) -> bool {
    false
}

pub fn resize_pointer_down(_window: &tauri::WebviewWindow) -> Option<bool> {
    None
}

pub async fn prepare_resize(_window: &tauri::WebviewWindow, _min: tauri::LogicalSize<f64>) -> Result<(), String> {
    Err("Compositor resize is unavailable".into())
}

/// The click-through lock as an input region, on Linux only (see
/// `linux/window.rs`). Here the pointer watch in `app.rs` does it instead.
pub fn click_through_except(_window: &tauri::WebviewWindow, _ignore: bool, _keep: Option<(f64, f64, f64, f64, f64)>) -> bool {
    false
}

/// Hide a plugin window so it comes back where it was. Here a hidden window
/// keeps its place, so it is simply hidden (Linux keeps it mapped instead).
pub fn hide_in_place(window: &tauri::WebviewWindow) {
    hide_without_focus(window);
}

/// Windows are placed by the meter itself here (Linux on Wayland asks KWin).
pub fn keep_places(
    _save: impl Fn(&str, (i32, i32, i32, i32)) + Send + Sync + 'static,
    _load: impl Fn(&str) -> Option<(i32, i32, i32, i32)> + Send + Sync + 'static,
) {
}

/// Nothing is hidden in place here.
pub fn is_hidden_in_place(_window: &tauri::WebviewWindow) -> bool {
    false
}

/// Nothing is hidden in place here: windows are shown as usual.
pub fn reveal_in_place(_window: &tauri::WebviewWindow) -> bool {
    false
}
