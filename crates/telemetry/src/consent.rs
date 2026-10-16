use std::{fs, io, path::Path};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Consent {
    enabled: bool,
}
impl Consent {
    pub fn disabled() -> Self {
        Self::default()
    }
    pub fn enable_explicit(&mut self) {
        self.enabled = true;
    }
    pub fn disable(&mut self) {
        self.enabled = false;
    }

    pub fn revoke(
        &mut self,
        consent_path: &Path,
        audit_path: &Path,
        created_at: u64,
    ) -> io::Result<()> {
        self.disable();
        self.save(consent_path)?;
        if let Some(parent) = audit_path.parent() {
            fs::create_dir_all(parent)?;
        }
        let record = serde_json::json!({
            "event": "telemetry_consent_revoked",
            "created_at": created_at,
        });
        use std::io::Write;
        let mut audit = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(audit_path)?;
        serde_json::to_writer(&mut audit, &record).map_err(io::Error::other)?;
        audit.write_all(b"\n")
    }
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn load(path: &Path) -> io::Result<Self> {
        if !path.is_file() {
            return Ok(Self::disabled());
        }
        let contents = fs::read_to_string(path)?;
        Ok(serde_json::from_str(&contents).unwrap_or_default())
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(path, serde_json::to_vec(self).map_err(io::Error::other)?)
    }
}

#[cfg(test)]
mod tests {
    use super::Consent;

    #[test]
    fn malformed_consent_fails_closed() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("consent.json");
        std::fs::write(&path, b"not-json").unwrap();
        assert!(!Consent::load(&path).unwrap().is_enabled());
    }
}
