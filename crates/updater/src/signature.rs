use crate::manifest::Manifest;
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

trait SignatureVerifier {
    fn verify(&self, manifest: &Manifest, digest: &[u8; 32]) -> bool;
}

pub struct Ed25519TrustRoot {
    keys: BTreeMap<String, VerifyingKey>,
}

impl Ed25519TrustRoot {
    pub fn from_hex(key_id: impl Into<String>, public_key: &str) -> Result<Self, String> {
        let bytes = decode_hex(public_key)?;
        let bytes: [u8; 32] = bytes
            .try_into()
            .map_err(|_| "package public key must contain 32 bytes".to_owned())?;
        let key = VerifyingKey::from_bytes(&bytes)
            .map_err(|error| format!("invalid package public key: {error}"))?;
        Ok(Self {
            keys: [(key_id.into(), key)].into_iter().collect(),
        })
    }
}

impl SignatureVerifier for Ed25519TrustRoot {
    fn verify(&self, manifest: &Manifest, digest: &[u8; 32]) -> bool {
        let Some(key) = self.keys.get(&manifest.key_id) else {
            return false;
        };
        let Ok(bytes) = decode_hex(&manifest.signature) else {
            return false;
        };
        let Ok(bytes): Result<[u8; 64], _> = bytes.try_into() else {
            return false;
        };
        let signature = Signature::from_bytes(&bytes);
        key.verify(&manifest_payload(manifest, digest), &signature)
            .is_ok()
    }
}

pub fn manifest_payload(manifest: &Manifest, digest: &[u8; 32]) -> Vec<u8> {
    serde_json::to_vec(&(
        &manifest.name,
        manifest.kind,
        &manifest.version,
        &manifest.minimum_os,
        &manifest.sha256,
        &manifest.key_id,
        &manifest.rollback_version,
        &manifest.not_before,
        &manifest.expires_at,
        digest,
    ))
    .unwrap_or_default()
}

fn decode_hex(value: &str) -> Result<Vec<u8>, String> {
    let value = value.trim();
    if !value.len().is_multiple_of(2) {
        return Err("hex value must have an even length".into());
    }
    (0..value.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(&value[index..index + 2], 16).map_err(|_| "invalid hex".into())
        })
        .collect()
}

#[derive(Clone, Debug)]
pub struct VerifiedPackage {
    manifest: Manifest,
    digest: [u8; 32],
}

impl VerifiedPackage {
    pub fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    pub fn digest(&self) -> &[u8; 32] {
        &self.digest
    }
}

pub fn verify_package(
    manifest: &Manifest,
    bytes: &[u8],
    trust_root: &Ed25519TrustRoot,
) -> Result<VerifiedPackage, String> {
    if !manifest.allowed_name()
        || !manifest.allowed_kind()
        || manifest.signature.trim().is_empty()
        || manifest.minimum_os.trim().is_empty()
    {
        return Err("package manifest is not trusted".into());
    }
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    let actual = digest
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if !actual.eq_ignore_ascii_case(&manifest.sha256) {
        return Err("package hash mismatch".into());
    }
    if !trust_root.verify(manifest, &digest) {
        return Err("package signature invalid".into());
    }
    Ok(VerifiedPackage {
        manifest: manifest.clone(),
        digest,
    })
}

pub fn verify_package_for(
    manifest: &Manifest,
    bytes: &[u8],
    trust_root: &Ed25519TrustRoot,
    os_version: &str,
    now: u64,
) -> Result<VerifiedPackage, String> {
    if !manifest.valid_at(os_version, now) {
        return Err("package is expired or incompatible with this system".into());
    }
    verify_package(manifest, bytes, trust_root)
}
