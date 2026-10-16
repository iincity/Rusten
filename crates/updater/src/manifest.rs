use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum PackageKind {
    Product,
    Rules,
    Config,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct Manifest {
    pub name: String,
    pub kind: PackageKind,
    pub version: String,
    pub minimum_os: String,
    pub sha256: String,
    pub key_id: String,
    pub signature: String,
    pub rollback_version: Option<String>,
    pub not_before: Option<u64>,
    pub expires_at: Option<u64>,
}

impl Manifest {
    pub fn allowed_name(&self) -> bool {
        matches!(
            self.name.as_str(),
            "rusten-product" | "rusten-rules" | "rusten-config"
        )
    }

    pub fn allowed_kind(&self) -> bool {
        matches!(
            (self.name.as_str(), self.kind),
            ("rusten-product", PackageKind::Product)
                | ("rusten-rules", PackageKind::Rules)
                | ("rusten-config", PackageKind::Config)
        )
    }

    pub fn compatible_with(&self, os_version: &str) -> bool {
        parse_version(os_version)
            .zip(parse_version(&self.minimum_os))
            .is_some_and(|(actual, minimum)| actual >= minimum)
    }

    pub fn valid_at(&self, os_version: &str, now: u64) -> bool {
        !self.version.trim().is_empty()
            && self.allowed_name()
            && self.allowed_kind()
            && self.compatible_with(os_version)
            && self.not_before.is_none_or(|value| now >= value)
            && self.expires_at.is_none_or(|value| now < value)
    }
}

fn parse_version(value: &str) -> Option<Vec<u64>> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }
    value
        .split('.')
        .map(str::parse)
        .collect::<Result<Vec<_>, _>>()
        .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> Manifest {
        Manifest {
            name: "rusten-rules".into(),
            kind: PackageKind::Rules,
            version: "1".into(),
            minimum_os: "10.0".into(),
            sha256: "hash".into(),
            key_id: "release".into(),
            signature: "signature".into(),
            rollback_version: Some("0".into()),
            not_before: Some(10),
            expires_at: Some(20),
        }
    }

    #[test]
    fn validity_checks_system_and_time_window() {
        let manifest = manifest();
        assert!(manifest.valid_at("10.1", 10));
        assert!(!manifest.valid_at("9.9", 10));
        assert!(!manifest.valid_at("10.1", 20));
    }
}
