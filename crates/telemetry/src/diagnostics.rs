use crate::{
    consent::Consent,
    event_model::TelemetryEvent,
    redaction::{DiagnosticRecord, redact_diagnostic_record, redact_event},
    transport::{Transport, encrypt_bytes},
};
use std::{fs, io, path::Path};

pub fn write_local_bundle(path: &Path, events: &[TelemetryEvent]) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let redacted = events.iter().cloned().map(redact_event).collect::<Vec<_>>();
    fs::write(
        path,
        serde_json::to_vec_pretty(&redacted).map_err(io::Error::other)?,
    )
}

pub fn generate_local_diagnostic_package(
    path: &Path,
    records: &[DiagnosticRecord],
) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let redacted = records
        .iter()
        .cloned()
        .map(redact_diagnostic_record)
        .collect::<Vec<_>>();
    fs::write(
        path,
        serde_json::to_vec_pretty(&redacted).map_err(io::Error::other)?,
    )
}

pub fn upload_with_consent<T: Transport>(
    transport: &T,
    consent: Consent,
    path: &Path,
    key: &[u8; 32],
    nonce: [u8; 12],
) -> Result<bool, String> {
    if !consent.is_enabled() {
        return Ok(false);
    }
    let payload = fs::read(path).map_err(|error| error.to_string())?;
    transport.send(&encrypt_bytes(&payload, key, nonce)?)?;
    Ok(true)
}

pub fn delete_local_bundle(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}
