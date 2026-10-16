use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AuditRecord {
    pub timestamp: u64,
    pub command: String,
    pub policy_version: String,
    pub operator: String,
    pub approver: Option<String>,
    pub device: String,
    pub result: String,
    pub error_code: Option<String>,
}

pub struct AuditLog {
    path: PathBuf,
}
impl AuditLog {
    pub fn open(path: impl Into<PathBuf>) -> io::Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        Ok(Self { path })
    }
    pub fn append(&self, record: &AuditRecord) -> io::Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        serde_json::to_writer(&mut file, record).map_err(io::Error::other)?;
        file.write_all(b"\n")
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
}
