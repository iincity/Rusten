use application::{RecoveryError, RecoveryId, RecoveryItem};
use domain::{FileIdentity, normalize_path};
use persistence::BackupCatalog;

#[test]
fn backup_quota_rejects_new_item_without_removing_existing_recovery() {
    let root = tempfile::tempdir().unwrap();
    let backup = root.path().join("backup.bin");
    std::fs::write(&backup, b"data").unwrap();
    let catalog = BackupCatalog::open(root.path().join("catalog.jsonl")).unwrap();
    let item = RecoveryItem {
        id: RecoveryId(1),
        original_path: normalize_path(r"C:\Windows\Temp\cache.bin").unwrap(),
        current_path: None,
        file_identity: FileIdentity {
            volume_serial: 1,
            file_index: 1,
        },
        size: 4,
        backup_path: Some(normalize_path(&backup.to_string_lossy()).unwrap()),
        recoverable: true,
        created_at: 0,
    };
    catalog.record_with_quota(item.clone(), 4).unwrap();
    let second = RecoveryItem {
        id: RecoveryId(2),
        ..item
    };
    assert_eq!(
        catalog.record_with_quota(second, 4),
        Err(RecoveryError::QuotaExceeded)
    );
    assert_eq!(catalog.items().len(), 1);
}
