//! Discord's local IPC pipes, for the Discord activity (`presence.rs`).

use std::path::{Path, PathBuf};

pub type Stream = std::fs::File;

/// The ten pipe names. Asking a pipe for its metadata opens it, so each is
/// simply tried, as the `discord-rich-presence` crate does; a missing one
/// fails to open.
pub fn socket_paths() -> Vec<PathBuf> {
    (0..10).map(|i| PathBuf::from(format!(r"\\.\pipe\discord-ipc-{i}"))).collect()
}

pub fn connect(path: &Path) -> std::io::Result<Stream> {
    std::fs::OpenOptions::new().read(true).write(true).open(path)
}
