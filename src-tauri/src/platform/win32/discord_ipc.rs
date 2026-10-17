//! Discord's local IPC pipes, for the Discord activity (`presence.rs`).

use std::path::{Path, PathBuf};

pub type Stream = std::fs::File;

/// The ten pipe names. Asking a pipe for its metadata opens it, so each is
/// simply tried, as the `discord-rich-presence` crate does; a missing one
/// fails to open.
pub fn socket_paths() -> Vec<PathBuf> {
    (0..10).map(|i| PathBuf::from(format!(r"\\.\pipe\discord-ipc-{i}"))).collect()
}

/// A pipe opened as a `File` has no read timeout: a Discord that hangs
/// without closing its end blocks the read until it does. So `presence.rs`
/// runs each handshake on its own thread and waits for it only so long.
pub fn connect(path: &Path) -> std::io::Result<Stream> {
    std::fs::OpenOptions::new().read(true).write(true).open(path)
}

/// No identity to tell one pipe instance from the next without opening it.
/// A pipe with no Discord behind it fails to open at once anyway.
pub fn socket_id(_path: &Path) -> Option<u64> {
    None
}
