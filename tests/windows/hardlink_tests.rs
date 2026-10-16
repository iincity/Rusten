use std::{fs, sync::mpsc};

use domain::{ScanEvent, ScanStatus, normalize_path};
use platform_windows::{
    CancellationToken, FileScanner, ScanOptions, WindowsFileScanner, is_hardlink_pair,
    read_metadata,
};
use tempfile::tempdir;

fn normalized(path: &std::path::Path) -> domain::NormalizedPath {
    normalize_path(path.to_str().unwrap()).unwrap()
}

#[test]
fn hardlinks_share_identity_and_count_physical_bytes_once() {
    let fixture = tempdir().unwrap();
    let original = fixture.path().join("original.bin");
    let linked = fixture.path().join("linked.bin");
    fs::write(&original, vec![7_u8; 8_192]).unwrap();
    fs::hard_link(&original, &linked).unwrap();

    let original_metadata = read_metadata(&normalized(&original)).unwrap();
    let linked_metadata = read_metadata(&normalized(&linked)).unwrap();
    assert_eq!(original_metadata.identity(), linked_metadata.identity());
    assert!(original_metadata.link_count() >= 2);
    assert!(is_hardlink_pair(&normalized(&original), &normalized(&linked)).unwrap());

    let (sender, receiver) = mpsc::channel();
    let summary = WindowsFileScanner
        .scan(
            normalized(fixture.path()),
            ScanOptions::default(),
            sender,
            CancellationToken::new(),
        )
        .unwrap();
    let identities: Vec<_> = receiver
        .into_iter()
        .filter_map(|event| match event {
            ScanEvent::File(file) => Some(file.identity()),
            _ => None,
        })
        .collect();

    assert_eq!(summary.status(), ScanStatus::Complete);
    assert_eq!(summary.file_count(), 2);
    assert_eq!(summary.logical_size(), 16_384);
    assert_eq!(summary.allocated_size(), original_metadata.allocated_size());
    assert_eq!(identities, vec![original_metadata.identity(); 2]);
}
