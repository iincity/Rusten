use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AuditUpload {
    pub event_id: String,
    pub timestamp: u64,
    pub device_id: String,
    pub actor: String,
    pub session_id: String,
    pub request_id: u64,
    pub command: String,
    pub policy_version: String,
    pub approval: String,
    pub result_category: String,
    pub error_code: Option<String>,
}

#[derive(Default)]
pub struct OfflineQueue {
    entries: Vec<AuditUpload>,
    path: Option<PathBuf>,
}
impl OfflineQueue {
    pub fn open(path: impl Into<PathBuf>) -> io::Result<Self> {
        let path = path.into();
        let mut entries = Vec::new();
        if path.is_file() {
            for line in fs::read_to_string(&path)?
                .lines()
                .filter(|line| !line.trim().is_empty())
            {
                entries.push(serde_json::from_str(line).map_err(io::Error::other)?);
            }
        }
        Ok(Self {
            entries,
            path: Some(path),
        })
    }

    pub fn push(&mut self, entry: AuditUpload) {
        let _ = self.append(entry);
    }
    pub fn append(&mut self, entry: AuditUpload) -> io::Result<()> {
        if let Some(path) = &self.path {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut file = OpenOptions::new().create(true).append(true).open(path)?;
            serde_json::to_writer(&mut file, &entry).map_err(io::Error::other)?;
            file.write_all(b"\n")?;
        }
        self.entries.push(entry);
        Ok(())
    }
    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn drain(&mut self) -> Vec<AuditUpload> {
        let entries = std::mem::take(&mut self.entries);
        if let Some(path) = &self.path {
            let _ = fs::remove_file(path);
        }
        entries
    }
}

impl OfflineQueue {
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
}
