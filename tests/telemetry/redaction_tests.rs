use telemetry::{
    consent::Consent,
    diagnostics::{generate_local_diagnostic_package, upload_with_consent},
    event_model::TelemetryEvent,
    local_buffer::LocalBuffer,
    redaction::{DiagnosticRecord, redact_diagnostic_record, redact_text},
    retention::prune_older_than,
    transport::{EncryptedTelemetry, Transport, send_with_consent},
};

#[test]
fn disabled_telemetry_does_not_write_and_redacts_paths() {
    let root = tempfile::tempdir().unwrap();
    let buffer = LocalBuffer::open(root.path().join("events.jsonl"));
    let event = TelemetryEvent {
        created_at: 1,
        product_version: "1".into(),
        os_version: "Windows".into(),
        architecture: "x64".into(),
        duration_bucket: "1s".into(),
        error_code: Some(r"C:\Users\Alice\secret".into()),
        result_category: "ok".into(),
    };
    assert!(!buffer.append(Consent::disabled(), event).unwrap());
    assert!(!root.path().join("events.jsonl").exists());
    assert_eq!(redact_text(r"C:\Users\Alice\secret"), "<redacted-path>");
}

struct Collector(std::sync::Mutex<Vec<EncryptedTelemetry>>);
impl Transport for Collector {
    fn send(&self, event: &EncryptedTelemetry) -> Result<(), String> {
        self.0.lock().unwrap().push(event.clone());
        Ok(())
    }
}

#[test]
fn consent_is_persistent_ttl_is_enforced_and_transport_receives_ciphertext() {
    let root = tempfile::tempdir().unwrap();
    let consent_path = root.path().join("consent.json");
    let mut consent = Consent::disabled();
    consent.enable_explicit();
    consent.save(&consent_path).unwrap();
    assert!(Consent::load(&consent_path).unwrap().is_enabled());

    let buffer_path = root.path().join("events.jsonl");
    let buffer = LocalBuffer::open(&buffer_path);
    for created_at in [1, 100] {
        buffer
            .append(
                consent,
                TelemetryEvent {
                    created_at,
                    product_version: "1".into(),
                    os_version: "Windows".into(),
                    architecture: "x64".into(),
                    duration_bucket: "1s".into(),
                    error_code: None,
                    result_category: "ok".into(),
                },
            )
            .unwrap();
    }
    prune_older_than(&buffer_path, 100, 10).unwrap();
    assert_eq!(
        std::fs::read_to_string(&buffer_path)
            .unwrap()
            .lines()
            .count(),
        1
    );

    let collector = Collector(std::sync::Mutex::new(Vec::new()));
    assert!(
        send_with_consent(
            &collector,
            consent,
            TelemetryEvent {
                created_at: 100,
                product_version: "1".into(),
                os_version: "Windows".into(),
                architecture: "x64".into(),
                duration_bucket: "1s".into(),
                error_code: Some(r"C:\Users\Alice\secret".into()),
                result_category: "ok".into(),
            },
            &[7; 32],
            [3; 12],
        )
        .unwrap()
    );
    assert!(!collector.0.lock().unwrap()[0].0.is_empty());
}

#[test]
fn revoking_consent_persists_disabled_state_and_writes_audit_record() {
    let root = tempfile::tempdir().unwrap();
    let consent_path = root.path().join("consent.json");
    let audit_path = root.path().join("telemetry-audit.jsonl");
    let mut consent = Consent::disabled();
    consent.enable_explicit();

    consent.revoke(&consent_path, &audit_path, 42).unwrap();

    assert!(!Consent::load(&consent_path).unwrap().is_enabled());
    let audit = std::fs::read_to_string(audit_path).unwrap();
    assert!(audit.contains("telemetry_consent_revoked"));
    assert!(audit.contains("42"));
}

#[test]
fn local_diagnostic_package_is_redacted_and_disabled_consent_cannot_upload() {
    let root = tempfile::tempdir().unwrap();
    let package_path = root.path().join("diagnostics.json");
    let record = DiagnosticRecord {
        category: "scan".into(),
        message: r"failed at C:\Users\Alice\secret.txt".into(),
        path: Some(r"C:\Users\Alice\secret.txt".into()),
        file_name: Some("secret.txt".into()),
        content: Some("private file contents".into()),
        hash: Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".into()),
    };
    let redacted = redact_diagnostic_record(record.clone());
    assert!(
        !serde_json::to_string(&redacted)
            .unwrap()
            .contains("private file contents")
    );
    assert!(
        !serde_json::to_string(&redacted)
            .unwrap()
            .contains("secret.txt")
    );
    generate_local_diagnostic_package(&package_path, &[record]).unwrap();
    let package = std::fs::read_to_string(&package_path).unwrap();
    assert!(!package.contains("C:\\Users\\Alice"));
    assert!(!package.contains("private file contents"));
    assert!(!package.contains("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"));

    let collector = Collector(std::sync::Mutex::new(Vec::new()));
    assert!(
        !upload_with_consent(
            &collector,
            Consent::disabled(),
            &package_path,
            &[7; 32],
            [3; 12],
        )
        .unwrap()
    );
    assert!(collector.0.lock().unwrap().is_empty());
}

#[test]
fn redaction_removes_standalone_hashes_and_file_names() {
    assert_eq!(redact_text("secret.txt"), "<redacted-path>");
    assert_eq!(
        redact_text("0123456789abcdef0123456789abcdef"),
        "<redacted-hash>"
    );
}
