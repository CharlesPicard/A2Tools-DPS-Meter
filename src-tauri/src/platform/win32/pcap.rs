//! Which packet-capture library to load. The API is libpcap's on every OS;
//! on Windows it comes from Npcap.

use std::path::Path;

/// The library `capture::pcap_capturer` loads at runtime (the first of these
/// that loads).
pub const LIBRARIES: &[&str] = &["wpcap.dll"];

/// What to tell the player when it will not load.
pub const MISSING_HELP: &str = "Is Npcap installed? Download from https://npcap.com";

/// The meter can fetch and run Npcap's installer for the player.
pub const OFFERS_INSTALL: bool = true;

/// Whether a read returns at its timeout on a device with no traffic: Npcap's
/// does.
pub const READ_TIMES_OUT_WHEN_IDLE: bool = true;

/// Whether a filter can change while capturing without losing what the
/// kernel has queued. Not here: Npcap empties its buffer on a new filter
/// (BIOCSETF), so a lock that narrowed the filter would cut the game's
/// stream right where it was found. The filter is set once, when the device
/// opens, and kept (`pcap_capturer`).
pub const FILTER_CHANGE_KEEPS_QUEUE: bool = false;

/// Npcap puts `wpcap.dll` in System32 only when it is installed in its
/// WinPcap-compatible mode; otherwise it lives in `System32\Npcap`, where a
/// plain load does not look. Npcap's guidance for programs is to add that
/// folder to the DLL search path, so either kind of install works.
pub fn prepare() {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::PCWSTR;
    use windows::Win32::System::LibraryLoader::SetDllDirectoryW;
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        let folder = Path::new(&root).join("System32").join("Npcap");
        let wide: Vec<u16> = folder.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        // SAFETY: a NUL-terminated path that outlives the call.
        if let Err(e) = unsafe { SetDllDirectoryW(PCWSTR(wide.as_ptr())) } {
            tracing::debug!("SetDllDirectoryW({}) failed: {e}", folder.display());
        }
    });
}

/// Whether the capture library can be loaded at all.
pub fn library_available() -> bool {
    prepare();
    // SAFETY: loading Npcap runs no initialisation we depend on not running.
    LIBRARIES.iter().any(|name| unsafe { libloading::Library::new(name).is_ok() })
}

/// Run Npcap's installer and wait until it closes. It needs administrator
/// rights, which the meter may not have, so Windows is asked to elevate it
/// (the UAC prompt names Npcap's publisher); that prompt being refused is an
/// error.
pub fn run_installer(installer: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{WaitForSingleObject, INFINITE};
    use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let file: Vec<u16> = installer.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
    let mut info = SHELLEXECUTEINFOW {
        cbSize: std::mem::size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: w!("runas"),
        lpFile: PCWSTR(file.as_ptr()),
        nShow: SW_SHOWNORMAL.0,
        ..Default::default()
    };
    // SAFETY: `info` and the path it points to live across the call.
    unsafe { ShellExecuteExW(&mut info) }.map_err(|e| e.to_string())?;
    if !info.hProcess.is_invalid() {
        // SAFETY: a process handle ShellExecuteExW opened for us; closed once.
        unsafe {
            WaitForSingleObject(info.hProcess, INFINITE);
            let _ = CloseHandle(info.hProcess);
        }
    }
    Ok(())
}

/// Devices not worth opening on this OS. None here.
pub fn skip_device(_name: &str) -> bool {
    false
}
