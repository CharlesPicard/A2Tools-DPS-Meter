//! Handing things to the OS shell.

/// Open a URL in the default browser.
///
/// ShellExecuteW hands the URL to its registered handler as it is. `cmd /C
/// start` did this before, and cmd reads `&`, `|` and the like in the URL as
/// its own syntax: a link from a plugin could have run a command.
pub fn open_url(url: &str) {
    use windows::core::{w, PCWSTR};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let wide: Vec<u16> = url.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: both strings are NUL-terminated and outlive the call.
    unsafe {
        ShellExecuteW(None, w!("open"), PCWSTR(wide.as_ptr()), PCWSTR::null(), PCWSTR::null(), SW_SHOWNORMAL);
    }
}
