use std::{fs, process::Command, sync::mpsc};

use domain::{ReparseKind, ScanEvent, ScanIndex, ScanStatus, normalize_path};
use platform_windows::{
    CancellationToken, FileScanner, PlatformErrorKind, ReparsePointKind, ScanIssueKind,
    ScanOptions, WindowsFileScanner, reparse_point_kind,
};
use tempfile::tempdir;

fn normalized(path: &std::path::Path) -> domain::NormalizedPath {
    normalize_path(path.to_str().unwrap()).unwrap()
}

#[test]
fn junction_is_identified_and_not_followed() {
    let fixture = tempdir().unwrap();
    let scan_root = fixture.path().join("scan");
    let target = fixture.path().join("target");
    let junction = scan_root.join("junction");
    fs::create_dir(&scan_root).unwrap();
    fs::create_dir(&target).unwrap();
    fs::write(target.join("must-not-be-scanned.bin"), b"outside").unwrap();
    let status = Command::new("cmd")
        .args(["/c", "mklink", "/J"])
        .arg(&junction)
        .arg(&target)
        .status()
        .unwrap();
    assert!(status.success(), "failed to create NTFS junction fixture");

    assert_eq!(
        reparse_point_kind(&normalized(&junction)).unwrap(),
        Some(ReparsePointKind::MountPoint)
    );

    let (sender, _receiver) = mpsc::channel();
    let root_error = WindowsFileScanner
        .scan(
            normalized(&junction),
            ScanOptions::default(),
            sender,
            CancellationToken::new(),
        )
        .unwrap_err();
    assert_eq!(root_error.kind(), PlatformErrorKind::InvalidRoot);

    let nested_root = junction.join("nested");
    fs::create_dir(target.join("nested")).unwrap();
    let (sender, _receiver) = mpsc::channel();
    let nested_error = WindowsFileScanner
        .scan(
            normalized(&nested_root),
            ScanOptions::default(),
            sender,
            CancellationToken::new(),
        )
        .unwrap_err();
    assert_eq!(nested_error.kind(), PlatformErrorKind::InvalidRoot);

    let (sender, receiver) = mpsc::channel();
    let summary = WindowsFileScanner
        .scan(
            normalized(&scan_root),
            ScanOptions::default(),
            sender,
            CancellationToken::new(),
        )
        .unwrap();
    let events: Vec<_> = receiver.into_iter().collect();
    let escaped_file_seen = events.iter().any(|event| {
        matches!(event, ScanEvent::File(file) if file.path().display().ends_with("must-not-be-scanned.bin"))
    });
    assert!(events.iter().any(|event| matches!(event, ScanEvent::ReparsePoint(file) if file.path().display().ends_with("junction") && file.facts().reparse_kind == Some(ReparseKind::Junction) && file.facts().target_path.is_some())));
    assert!(!events.iter().any(|event| matches!(event, ScanEvent::File(file) if file.path().display().ends_with("junction"))));
    assert_eq!(summary.file_count(), 0);
    assert_eq!(summary.logical_size(), 0);
    assert_eq!(summary.allocated_size(), 0);

    let mut index = ScanIndex::new(normalized(&scan_root));
    for event in &events {
        match event {
            ScanEvent::Directory(directory) => {
                index.add_directory(directory.clone());
            }
            ScanEvent::File(file) => {
                index.add_file(file.clone());
            }
            _ => {}
        }
    }
    assert_eq!(index.statistics().file_count(), 0);
    assert!(index.top_files(1).is_empty());

    assert_eq!(summary.status(), ScanStatus::Partial);
    assert!(!escaped_file_seen);
    assert!(
        summary
            .issues()
            .iter()
            .any(|issue| issue.kind() == ScanIssueKind::ReparsePointSkipped)
    );
}

#[test]
fn file_reparse_points_are_not_counted_or_indexed() {
    let fixture = tempdir().unwrap();
    let scan_root = fixture.path().join("scan");
    let target = scan_root.join("target.bin");
    let link = scan_root.join("link.bin");
    fs::create_dir(&scan_root).unwrap();
    fs::write(&target, vec![7_u8; 4096]).unwrap();
    let status = Command::new("cmd")
        .args(["/c", "mklink"])
        .arg(&link)
        .arg(&target)
        .status()
        .unwrap();
    assert!(
        status.success(),
        "failed to create NTFS file symlink fixture"
    );

    let (sender, receiver) = mpsc::channel();
    let summary = WindowsFileScanner
        .scan(
            normalized(&scan_root),
            ScanOptions::default(),
            sender,
            CancellationToken::new(),
        )
        .unwrap();
    let events: Vec<_> = receiver.into_iter().collect();
    assert!(events.iter().any(|event| {
        matches!(
            event,
            ScanEvent::ReparsePoint(file)
                if file.path().display().ends_with("link.bin")
                    && file.facts().reparse_kind == Some(ReparseKind::SymbolicLink)
        )
    }));
    assert_eq!(summary.file_count(), 1);
    assert_eq!(summary.logical_size(), 4096);

    let mut index = ScanIndex::new(normalized(&scan_root));
    for event in &events {
        match event {
            ScanEvent::Directory(directory) => {
                index.add_directory(directory.clone());
            }
            ScanEvent::File(file) => {
                index.add_file(file.clone());
            }
            _ => {}
        }
    }
    assert_eq!(index.statistics().file_count(), 1);
    assert!(
        index.top_files(1)[0]
            .path()
            .display()
            .ends_with("target.bin")
    );
    assert!(
        !index
            .top_files(10)
            .iter()
            .any(|file| file.path().display().ends_with("link.bin"))
    );
}
