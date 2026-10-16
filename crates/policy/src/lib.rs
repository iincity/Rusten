use domain::{NormalizedPath, normalize_path};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct EnterprisePolicy {
    pub version: String,
    pub scan_roots: Vec<NormalizedPath>,
    pub exclusions: Vec<String>,
    pub maintenance_window: Option<String>,
    pub forced_forbidden: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PolicyEnvelope {
    pub policy: EnterprisePolicy,
    pub key_id: String,
    pub signature: String,
}

pub trait PolicyVerifier {
    fn verify(&self, envelope: &PolicyEnvelope) -> bool;
}

pub struct Ed25519TrustRoot {
    keys: BTreeMap<String, VerifyingKey>,
}

impl Ed25519TrustRoot {
    pub fn from_hex(key_id: impl Into<String>, public_key: &str) -> Result<Self, String> {
        let bytes = decode_hex(public_key)?;
        let bytes: [u8; 32] = bytes
            .try_into()
            .map_err(|_| "policy public key must contain 32 bytes".to_owned())?;
        let key = VerifyingKey::from_bytes(&bytes)
            .map_err(|error| format!("invalid policy public key: {error}"))?;
        Ok(Self {
            keys: [(key_id.into(), key)].into_iter().collect(),
        })
    }
}

impl PolicyVerifier for Ed25519TrustRoot {
    fn verify(&self, envelope: &PolicyEnvelope) -> bool {
        let Some(key) = self.keys.get(&envelope.key_id) else {
            return false;
        };
        let Ok(bytes) = decode_hex(&envelope.signature) else {
            return false;
        };
        let Ok(bytes): Result<[u8; 64], _> = bytes.try_into() else {
            return false;
        };
        let signature = Signature::from_bytes(&bytes);
        key.verify(&policy_payload(envelope), &signature).is_ok()
    }
}

pub fn load_toml(input: &str) -> Result<EnterprisePolicy, String> {
    toml::from_str(input).map_err(|error| error.to_string())
}

pub fn verify_and_apply<V: PolicyVerifier>(
    envelope: PolicyEnvelope,
    verifier: &V,
) -> Result<EnterprisePolicy, String> {
    if envelope.key_id.trim().is_empty() || envelope.signature.trim().is_empty() {
        return Err("policy signature metadata missing".into());
    }
    if !verifier.verify(&envelope) {
        return Err("policy signature invalid".into());
    }
    validate_policy(&envelope.policy)?;
    Ok(envelope.policy)
}

pub fn verify_embedded_signature(
    envelope: PolicyEnvelope,
    trust_root: &impl PolicyVerifier,
) -> Result<EnterprisePolicy, String> {
    verify_and_apply(envelope, trust_root)
}

fn policy_payload(envelope: &PolicyEnvelope) -> Vec<u8> {
    serde_json::to_vec(&(&envelope.key_id, &envelope.policy)).unwrap_or_default()
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

pub fn enterprise_wins(local: &EnterprisePolicy, managed: &EnterprisePolicy) -> EnterprisePolicy {
    EnterprisePolicy {
        version: managed.version.clone(),
        scan_roots: if managed.scan_roots.is_empty() {
            local.scan_roots.clone()
        } else {
            managed.scan_roots.clone()
        },
        exclusions: managed.exclusions.clone(),
        maintenance_window: managed
            .maintenance_window
            .clone()
            .or_else(|| local.maintenance_window.clone()),
        forced_forbidden: local
            .forced_forbidden
            .iter()
            .chain(managed.forced_forbidden.iter())
            .cloned()
            .collect(),
    }
}

pub fn is_forced_forbidden(policy: &EnterprisePolicy, path: &NormalizedPath) -> bool {
    policy.forced_forbidden.iter().any(|prefix| {
        normalize_path(prefix)
            .ok()
            .is_some_and(|prefix| path_prefix_equal(path.display(), prefix.display()))
    })
}

fn validate_policy(policy: &EnterprisePolicy) -> Result<(), String> {
    if policy.version.trim().is_empty() {
        return Err("policy version is required".into());
    }
    if parse_version(&policy.version).is_none() {
        return Err("policy version is invalid".into());
    }
    for root in &policy.scan_roots {
        normalize_path(root.display()).map_err(|_| "policy scan root is invalid".to_owned())?;
    }
    for path in policy
        .exclusions
        .iter()
        .chain(policy.forced_forbidden.iter())
    {
        normalize_path(path).map_err(|_| "policy path is invalid".to_owned())?;
    }
    Ok(())
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

fn path_prefix_equal(path: &str, prefix: &str) -> bool {
    let path = path.trim_end_matches(['\\', '/']);
    let prefix = prefix.trim_end_matches(['\\', '/']);
    let mut path_chars = path.chars();
    for prefix_char in prefix.chars() {
        let Some(path_char) = path_chars.next() else {
            return false;
        };
        if !path_char.eq_ignore_ascii_case(&prefix_char) {
            return false;
        }
    }
    matches!(path_chars.next(), None | Some('\\') | Some('/'))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    struct Accept(bool);
    impl PolicyVerifier for Accept {
        fn verify(&self, _: &PolicyEnvelope) -> bool {
            self.0
        }
    }
    #[test]
    fn invalid_signature_fails_closed_and_managed_policy_wins() {
        let policy = EnterprisePolicy {
            version: "2".into(),
            exclusions: vec![r"C:\managed".into()],
            ..Default::default()
        };
        let envelope = PolicyEnvelope {
            policy: policy.clone(),
            key_id: "k".into(),
            signature: "s".into(),
        };
        assert!(verify_and_apply(envelope.clone(), &Accept(false)).is_err());
        let local = EnterprisePolicy {
            version: "1".into(),
            exclusions: vec![r"C:\local".into()],
            ..Default::default()
        };
        assert_eq!(
            enterprise_wins(&local, &verify_and_apply(envelope, &Accept(true)).unwrap()).exclusions,
            vec![r"C:\managed"]
        );
    }

    #[test]
    fn ed25519_trust_root_verifies_only_signed_policy() {
        let signing_key = SigningKey::from_bytes(&[7; 32]);
        let public_key = signing_key
            .verifying_key()
            .to_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let mut envelope = PolicyEnvelope {
            policy: EnterprisePolicy {
                version: "2".into(),
                ..Default::default()
            },
            key_id: "test".into(),
            signature: String::new(),
        };
        envelope.signature = signing_key
            .sign(&policy_payload(&envelope))
            .to_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        let root = Ed25519TrustRoot::from_hex("test", &public_key).unwrap();
        assert!(verify_embedded_signature(envelope.clone(), &root).is_ok());
        envelope.policy.version = "3".into();
        assert!(verify_embedded_signature(envelope, &root).is_err());
    }

    #[test]
    fn invalid_policy_versions_are_rejected() {
        let policy = EnterprisePolicy {
            version: "release-candidate".into(),
            ..Default::default()
        };
        let envelope = PolicyEnvelope {
            policy,
            key_id: "k".into(),
            signature: "s".into(),
        };
        assert!(verify_and_apply(envelope, &Accept(true)).is_err());
    }
}
