//! Files beside the database: each log's slice (kept, so a log can be
//! re-derived by a later parser) and derived record, and dev-log uploads.

use std::path::{Path, PathBuf};

#[derive(Clone)]
pub struct Store {
    root: PathBuf,
}

/// Only ids this site made reach the filesystem, but check anyway.
fn safe(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

impl Store {
    pub fn new(root: &Path) -> std::io::Result<Self> {
        for dir in ["slices", "records", "devlogs"] {
            std::fs::create_dir_all(root.join(dir))?;
        }
        Ok(Self { root: root.to_path_buf() })
    }

    pub fn slice_path(&self, id: &str) -> Option<PathBuf> {
        safe(id).then(|| self.root.join("slices").join(format!("{id}.bin")))
    }

    pub fn record_path(&self, id: &str) -> Option<PathBuf> {
        safe(id).then(|| self.root.join("records").join(format!("{id}.json")))
    }

    pub fn devlog_dir(&self, code: &str) -> Option<PathBuf> {
        safe(code).then(|| self.root.join("devlogs").join(code))
    }

    pub fn write_log(&self, id: &str, slice: &[u8], record_json: &[u8]) -> std::io::Result<()> {
        let (Some(slice_path), Some(record_path)) = (self.slice_path(id), self.record_path(id)) else {
            return Err(std::io::Error::other("bad id"));
        };
        write_atomic(&slice_path, slice)?;
        write_atomic(&record_path, record_json)
    }

    pub fn write_record(&self, id: &str, record_json: &[u8]) -> std::io::Result<()> {
        let path = self.record_path(id).ok_or_else(|| std::io::Error::other("bad id"))?;
        write_atomic(&path, record_json)
    }

    pub fn read_record(&self, id: &str) -> Option<Vec<u8>> {
        std::fs::read(self.record_path(id)?).ok()
    }

    pub fn read_slice(&self, id: &str) -> Option<Vec<u8>> {
        std::fs::read(self.slice_path(id)?).ok()
    }

    pub fn remove_log(&self, id: &str) {
        if let Some(p) = self.slice_path(id) {
            let _ = std::fs::remove_file(p);
        }
        if let Some(p) = self.record_path(id) {
            let _ = std::fs::remove_file(p);
        }
    }

    pub fn remove_devlog(&self, code: &str) {
        if let Some(dir) = self.devlog_dir(code) {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, data)?;
    std::fs::rename(tmp, path)
}
