use application::rule_service::RuleService;
use domain::{FileIdentity, FileRecord, RuleMatchContext, ScanIndex, normalize_path};

#[test]
fn loads_all_builtin_rules_without_filesystem_or_environment_access() {
    let service = RuleService::load_builtin().unwrap();
    assert_eq!(service.rules().len(), 8);
    assert!(service.shell_recycle_bin_rule().is_some());
    assert!(service.rules().iter().any(|rule| rule.id() == "edge_cache"));
    assert!(
        service
            .rules()
            .iter()
            .any(|rule| rule.id() == "system_logs")
    );
}

#[test]
fn recycle_bin_rule_cannot_generate_internal_path_candidates() {
    let service = RuleService::load_builtin().unwrap();
    let index = ScanIndex::new(normalize_path(r"C:\").unwrap());
    let file = FileRecord::new(
        normalize_path(r"C:\$Recycle.Bin\S-1-5-21\$I123.tmp").unwrap(),
        index.root_directory_id(),
        FileIdentity {
            volume_serial: 1,
            file_index: 1,
        },
        1,
        1,
    );
    assert!(
        service
            .candidates(&[file], &RuleMatchContext::default())
            .is_empty()
    );
}

#[test]
fn classifies_windows_error_reports_as_log_cleanup() {
    let service = RuleService::load_builtin().unwrap();
    let index = ScanIndex::new(normalize_path(r"C:\").unwrap());
    let file = FileRecord::new(
        normalize_path(r"C:\ProgramData\Microsoft\Windows\WER\ReportArchive\Report123\report.wer")
            .unwrap(),
        index.root_directory_id(),
        FileIdentity {
            volume_serial: 1,
            file_index: 2,
        },
        4096,
        4096,
    );
    let candidates = service.candidates(&[file], &RuleMatchContext::default());

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].category.as_deref(), Some("system.logs"));
    assert_eq!(candidates[0].estimated_size, Some(4096));
}

#[test]
fn classifies_wechat_files_from_documents_as_cleanup_candidates() {
    let service = RuleService::load_builtin().unwrap();
    let index = ScanIndex::new(normalize_path(r"C:\").unwrap());
    let file = FileRecord::new(
        normalize_path(
            r"C:\Users\Administrator\Documents\WeChat Files\wxid_demo\FileStorage\Image\2026\image.jpg",
        )
        .unwrap(),
        index.root_directory_id(),
        FileIdentity {
            volume_serial: 1,
            file_index: 3,
        },
        8192,
        8192,
    );

    let candidates = service.candidates(&[file], &RuleMatchContext::default());

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].rule_id.as_deref(), Some("wechat_cache"));
    assert_eq!(
        candidates[0].category.as_deref(),
        Some("wechat.chat_images")
    );
}
