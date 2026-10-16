use domain::{
    FileIdentity, FileRecord, RecoveryMode, RiskLevel, Rule, RuleMatchContext, generate_candidates,
    normalize_path,
};

fn file(path: &str) -> FileRecord {
    let index = domain::ScanIndex::new(normalize_path(r"C:\").unwrap());
    FileRecord::new(
        normalize_path(path).unwrap(),
        index.root_directory_id(),
        FileIdentity {
            volume_serial: 1,
            file_index: 1,
        },
        100,
        128,
    )
}

fn rule(risk: &str, mode: &str, confidence: &str, recovery: &str) -> Rule {
    Rule::from_toml(&format!(
        r#"
id = "policy.test"
name = "Policy test"
category = "test"
mode = "{mode}"
risk = "{risk}"
confidence = "{confidence}"
recovery = "{recovery}"
requires_admin = false
warning = "reason"
version = "1"

[match]
paths = ["C:/Users/*/AppData/Local/Temp/*"]
"#
    ))
    .unwrap()
}

#[test]
fn p0_and_p2_candidates_are_blocked_by_quick_policy() {
    let path = r"C:\Users\A\AppData\Local\Temp\old.tmp";
    for risk in ["P0", "P2"] {
        let candidate = generate_candidates(
            &[rule(risk, "quick", "high", "recycle")],
            &[file(path)],
            &RuleMatchContext::default(),
        )
        .pop()
        .unwrap();
        assert!(!candidate.quick_clean_eligible);
        assert_eq!(
            candidate.risk,
            Some(if risk == "P0" {
                RiskLevel::P0
            } else {
                RiskLevel::P2
            })
        );
    }
}

#[test]
fn p3_candidate_becomes_eligible_only_after_execution_facts_are_revalidated() {
    let candidate = generate_candidates(
        &[rule("P3", "quick", "high", "backup_then_delete")],
        &[file(r"C:\Users\A\AppData\Local\Temp\old.tmp")],
        &RuleMatchContext::default(),
    )
    .pop()
    .unwrap();
    assert!(!candidate.quick_clean_eligible);
    let candidate = candidate.with_execution_facts(false, true, true);
    assert!(candidate.quick_clean_eligible);
    assert_eq!(candidate.risk, Some(RiskLevel::P3));
    assert_eq!(candidate.recovery, Some(RecoveryMode::BackupThenDelete));
}

#[test]
fn forbidden_path_cannot_be_reenabled_by_a_rule() {
    let rule = Rule::from_toml(
        r#"
id = "unsafe"
name = "Unsafe"
category = "cache"
mode = "quick"
risk = "P3"
confidence = "high"
recovery = "recycle"
requires_admin = false
warning = "unsafe"
version = "1"

[match]
paths = ["C:/Users/*/Documents/*"]
"#,
    )
    .unwrap();
    assert!(
        generate_candidates(
            &[rule],
            &[file(r"C:\Users\A\Documents\important.txt")],
            &RuleMatchContext::default()
        )
        .is_empty()
    );
}
