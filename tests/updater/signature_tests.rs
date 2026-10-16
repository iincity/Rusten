use ed25519_dalek::{Signer, SigningKey};
use sha2::{Digest, Sha256};
use updater::{
    manifest::{Manifest, PackageKind},
    rollback::rollback,
    rollout_policy::{ReleaseState, RolloutPolicy},
    signature::{Ed25519TrustRoot, manifest_payload, verify_package, verify_package_for},
    staged_install::{
        install_verified_at_root, install_verified_with_fallback_at_root, recover_pending_at_root,
        target_path,
    },
};

#[test]
fn tampered_or_unsigned_packages_are_rejected() {
    let bytes = b"rules";
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    let hash = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    let signing_key = SigningKey::from_bytes(&[8; 32]);
    let public_key = signing_key
        .verifying_key()
        .to_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let mut manifest = Manifest {
        name: "rusten-rules".into(),
        kind: PackageKind::Rules,
        version: "1".into(),
        minimum_os: "10".into(),
        sha256: hash,
        key_id: "test".into(),
        signature: String::new(),
        rollback_version: Some("0".into()),
        not_before: None,
        expires_at: None,
    };
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    manifest.signature = signing_key
        .sign(&manifest_payload(&manifest, &digest))
        .to_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let root = Ed25519TrustRoot::from_hex("test", &public_key).unwrap();
    let verified = verify_package(&manifest, bytes, &root).unwrap();
    assert_eq!(verified.manifest(), &manifest);
    assert!(verify_package(&manifest, b"tampered", &root).is_err());
}

#[test]
fn real_trust_root_rejects_manifest_tampering() {
    let bytes = b"rules";
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    let signing_key = SigningKey::from_bytes(&[9; 32]);
    let public_key = signing_key
        .verifying_key()
        .to_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let mut manifest = Manifest {
        name: "rusten-rules".into(),
        kind: PackageKind::Rules,
        version: "1".into(),
        minimum_os: "10".into(),
        sha256: digest.iter().map(|byte| format!("{byte:02x}")).collect(),
        key_id: "release".into(),
        signature: String::new(),
        rollback_version: Some("0".into()),
        not_before: None,
        expires_at: None,
    };
    manifest.signature = signing_key
        .sign(&manifest_payload(&manifest, &digest))
        .to_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let root = Ed25519TrustRoot::from_hex("release", &public_key).unwrap();
    assert!(verify_package(&manifest, bytes, &root).is_ok());
    manifest.version = "2".into();
    assert!(verify_package(&manifest, bytes, &root).is_err());
}

#[test]
fn manifest_validity_window_is_covered_by_signature() {
    let bytes = b"rules";
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    let signing_key = SigningKey::from_bytes(&[12; 32]);
    let public_key = signing_key
        .verifying_key()
        .to_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let mut manifest = Manifest {
        name: "rusten-rules".into(),
        kind: PackageKind::Rules,
        version: "1".into(),
        minimum_os: "10".into(),
        sha256: digest.iter().map(|byte| format!("{byte:02x}")).collect(),
        key_id: "test".into(),
        signature: String::new(),
        rollback_version: Some("0".into()),
        not_before: None,
        expires_at: None,
    };
    manifest.signature = signing_key
        .sign(&manifest_payload(&manifest, &digest))
        .to_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    manifest.expires_at = Some(1);

    let root = Ed25519TrustRoot::from_hex("test", &public_key).unwrap();
    assert!(verify_package(&manifest, bytes, &root).is_err());
}

#[test]
fn install_uses_fixed_targets_and_rollout_requires_verified_state() {
    let root = tempfile::tempdir().unwrap();
    assert_eq!(
        target_path(root.path(), PackageKind::Rules),
        root.path().join("rules/builtin.toml")
    );
    let policy = RolloutPolicy {
        target_version: "2".into(),
        rollout_percentage: 50,
        state: ReleaseState::Active,
        maintenance_window: true,
        verified_rollback_version: Some("1".into()),
    };
    assert!(policy.allows(10, "1"));
    assert!(!policy.allows(60, "1"));
    assert!(policy.can_rollback_to("1"));
}

fn signed_package(
    bytes: &[u8],
    version: &str,
    rollback_version: Option<&str>,
) -> (Manifest, Ed25519TrustRoot) {
    let signing_key = SigningKey::from_bytes(&[11; 32]);
    let public_key = signing_key
        .verifying_key()
        .to_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    let mut manifest = Manifest {
        name: "rusten-rules".into(),
        kind: PackageKind::Rules,
        version: version.into(),
        minimum_os: "10".into(),
        sha256: digest.iter().map(|byte| format!("{byte:02x}")).collect(),
        key_id: "test".into(),
        signature: String::new(),
        rollback_version: rollback_version.map(str::to_owned),
        not_before: None,
        expires_at: None,
    };
    manifest.signature = signing_key
        .sign(&manifest_payload(&manifest, &digest))
        .to_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    (
        manifest,
        Ed25519TrustRoot::from_hex("test", &public_key).unwrap(),
    )
}

#[test]
fn real_files_are_installed_atomically_and_corrupt_pending_is_discarded() {
    let root = tempfile::tempdir().unwrap();
    let target = target_path(root.path(), PackageKind::Rules);
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&target, b"current").unwrap();
    let pending = target.with_extension("pending");
    std::fs::write(&pending, b"").unwrap();
    assert!(recover_pending_at_root(root.path(), PackageKind::Rules).unwrap());
    assert!(!pending.exists());
    assert_eq!(std::fs::read(&target).unwrap(), b"current");

    let bytes = b"new rules";
    let staged = root.path().join("rusten-rules-2.pkg");
    std::fs::write(&staged, bytes).unwrap();
    let (manifest, trust_root) = signed_package(bytes, "2", Some("1"));
    let verified = verify_package_for(&manifest, bytes, &trust_root, "10.0", 1).unwrap();
    let backup = install_verified_at_root(&verified, &staged, root.path(), "10.0", 1).unwrap();

    assert_eq!(std::fs::read(&target).unwrap(), bytes);
    assert_eq!(std::fs::read(backup).unwrap(), b"current");
    assert!(!target.with_extension("pending").exists());
}

#[test]
fn failed_update_installs_verified_fallback_and_rollback_replaces_real_file() {
    let root = tempfile::tempdir().unwrap();
    let target = target_path(root.path(), PackageKind::Rules);
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&target, b"current").unwrap();

    let fallback_bytes = b"verified fallback";
    let fallback_staged = root.path().join("rusten-rules-1.pkg");
    std::fs::write(&fallback_staged, fallback_bytes).unwrap();
    let (fallback_manifest, trust_root) = signed_package(fallback_bytes, "1", None);
    let fallback =
        verify_package_for(&fallback_manifest, fallback_bytes, &trust_root, "10.0", 1).unwrap();

    let (manifest, _) = signed_package(b"new rules", "2", Some("1"));
    let new_staged = root.path().join("missing-rules.pkg");
    let new_verified = verify_package_for(&manifest, b"new rules", &trust_root, "10.0", 1).unwrap();
    assert!(
        install_verified_with_fallback_at_root(
            &new_verified,
            &new_staged,
            root.path(),
            &fallback,
            &fallback_staged,
            "10.0",
            1,
        )
        .is_ok()
    );
    assert_eq!(std::fs::read(&target).unwrap(), fallback_bytes);

    let previous = root.path().join("previous.rules");
    std::fs::write(&previous, b"rolled back").unwrap();
    rollback(&target, &previous).unwrap();
    assert_eq!(std::fs::read(&target).unwrap(), b"rolled back");
    assert!(!target.with_extension("rollback.pending").exists());
}
