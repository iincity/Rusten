use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PolicyVersion {
    pub version: String,
    pub payload: String,
    pub key_id: String,
    pub signature: String,
}

pub struct Ed25519TrustRoot {
    keys: BTreeMap<String, VerifyingKey>,
}

impl Ed25519TrustRoot {
    pub fn from_hex(key_id: impl Into<String>, public_key: &str) -> Result<Self, String> {
        let bytes = decode_hex(public_key)?;
        let bytes: [u8; 32] = bytes
            .try_into()
            .map_err(|_| "invalid public key length".to_owned())?;
        let key = VerifyingKey::from_bytes(&bytes).map_err(|error| error.to_string())?;
        Ok(Self {
            keys: [(key_id.into(), key)].into_iter().collect(),
        })
    }
}

pub fn signed_payload(version: &PolicyVersion) -> Vec<u8> {
    serde_json::to_vec(&(&version.key_id, &version.version, &version.payload)).unwrap_or_default()
}

pub fn accept(
    version: PolicyVersion,
    trust_root: &Ed25519TrustRoot,
) -> Result<PolicyVersion, String> {
    let key = trust_root
        .keys
        .get(&version.key_id)
        .ok_or("unknown policy key")?;
    let signature = decode_hex(&version.signature)?;
    let signature: [u8; 64] = signature
        .try_into()
        .map_err(|_| "invalid policy signature length")?;
    let signature = Signature::from_bytes(&signature);
    key.verify(&signed_payload(&version), &signature)
        .map_err(|_| "remote policy signature invalid".to_owned())?;
    if parse_version(&version.version).is_none() || version.payload.trim().is_empty() {
        Err("remote policy is empty".into())
    } else {
        Ok(version)
    }
}

pub fn accept_after(
    version: PolicyVersion,
    current_version: &str,
    trust_root: &Ed25519TrustRoot,
) -> Result<PolicyVersion, String> {
    let accepted = accept(version, trust_root)?;
    let accepted_version = parse_version(&accepted.version)
        .ok_or_else(|| "remote policy version is invalid".to_owned())?;
    let current_version = parse_version(current_version)
        .ok_or_else(|| "current policy version is invalid".to_owned())?;
    if accepted_version < current_version {
        return Err("remote policy rollback rejected".into());
    }
    Ok(accepted)
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

fn decode_hex(value: &str) -> Result<Vec<u8>, String> {
    if !value.len().is_multiple_of(2) {
        return Err("invalid hex".into());
    }
    (0..value.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&value[index..index + 2], 16).map_err(|_| "invalid hex".into())
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    #[test]
    fn only_signed_policy_version_is_accepted() {
        let signing_key = SigningKey::from_bytes(&[5; 32]);
        let public_key = signing_key
            .verifying_key()
            .to_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let mut version = PolicyVersion {
            version: "4".into(),
            payload: "rules-hash".into(),
            key_id: "release".into(),
            signature: String::new(),
        };
        version.signature = signing_key
            .sign(&signed_payload(&version))
            .to_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let root = Ed25519TrustRoot::from_hex("release", &public_key).unwrap();
        assert!(accept(version.clone(), &root).is_ok());
        assert!(accept_after(version.clone(), "5", &root).is_err());
        version.payload = "tampered".into();
        assert!(accept(version, &root).is_err());
    }
}
