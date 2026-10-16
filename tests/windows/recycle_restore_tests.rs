use application::{RecoveryId, RecoveryItem, RecoveryStore, RestoreDestination};
use domain::{FileIdentity, normalize_path};
use persistence::BackupCatalog;

#[test]
fn backup_catalog_restores_a_file_and_rejects_conflict() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("backup.bin");
    let destination = root.path().join("restored.bin");
    std::fs::write(&source, b"restored").unwrap();
    let catalog = BackupCatalog::open(root.path().join("catalog.jsonl")).unwrap();
    let original = normalize_path(destination.to_str().unwrap()).unwrap();
    let backup = normalize_path(source.to_str().unwrap()).unwrap();
    catalog
        .record(RecoveryItem {
            id: RecoveryId(1),
            original_path: original.clone(),
            current_path: None,
            file_identity: FileIdentity {
                volume_serial: 1,
                file_index: 1,
            },
            size: 8,
            backup_path: Some(backup),
            recoverable: true,
            created_at: 0,
        })
        .unwrap();
    catalog
        .restore(RecoveryId(1), RestoreDestination::Original)
        .unwrap();
    assert_eq!(std::fs::read(destination).unwrap(), b"restored");
    assert!(
        catalog
            .restore(RecoveryId(1), RestoreDestination::Original)
            .is_err()
    );
}

#[test]
fn backup_catalog_restores_long_paths_using_extended_names() {
    let root = tempfile::tempdir().unwrap();
    let display_directory = (0..12).fold(root.path().to_path_buf(), |path, index| {
        path.join(format!("long-directory-{index:02}-abcdefghij"))
    });
    let display_source = display_directory.join("backup.bin");
    let display_destination = display_directory.join("restored.bin");
    let extended_directory =
        std::path::PathBuf::from(format!(r"\\?\{}", display_directory.to_string_lossy()));
    let extended_source = extended_directory.join("backup.bin");
    let extended_destination = extended_directory.join("restored.bin");
    std::fs::create_dir_all(&extended_directory).unwrap();
    std::fs::write(&extended_source, b"restored").unwrap();

    let catalog = BackupCatalog::open(root.path().join("catalog.jsonl")).unwrap();
    catalog
        .record(RecoveryItem {
            id: RecoveryId(2),
            original_path: normalize_path(&display_destination.to_string_lossy()).unwrap(),
            current_path: None,
            file_identity: FileIdentity {
                volume_serial: 1,
                file_index: 2,
            },
            size: 8,
            backup_path: Some(normalize_path(&display_source.to_string_lossy()).unwrap()),
            recoverable: true,
            created_at: 0,
        })
        .unwrap();

    catalog
        .restore(RecoveryId(2), RestoreDestination::Original)
        .unwrap();
    assert_eq!(std::fs::read(extended_destination).unwrap(), b"restored");
}
