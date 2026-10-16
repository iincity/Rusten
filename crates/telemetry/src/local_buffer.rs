use crate::{consent::Consent, event_model::TelemetryEvent, redaction::redact_event};
use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::PathBuf,
};

pub struct LocalBuffer {
    path: PathBuf,
}
impl LocalBuffer {
    pub fn open(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
    pub fn append(&self, consent: Consent, event: TelemetryEvent) -> io::Result<bool> {
        if !consent.is_enabled() {
            return Ok(false);
        }
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        serde_json::to_writer(&mut file, &redact_event(event)).map_err(io::Error::other)?;
        file.write_all(b"\n")?;
        Ok(true)
    }
}
