use std::{
    fs::{self, OpenOptions},
    os::windows::fs::OpenOptionsExt,
    sync::mpsc,
};

use domain::{LockState, ScanEvent, ScanIndex, ScanStatus, ScanWarningKind, normalize_path};
use platform_windows::{
    CancellationToken, FileScanner, PlatformErrorKind, ScanIssueKind, ScanOptions,
    WindowsFileScanner, paths_equal_ordinal, query_volume_stats, read_metadata,
    validate_physical_target,
};
use tempfile::tempdir;

fn normalized(path: &std::path::Path) -> domain::NormalizedPath {
    normalize_path(path.to_str().expect("temporary path must be Unicode"))
        .expect("temporary path must be a local absolute path")
}

#[test]
fn scans_long_ntfs_paths_with_native_metadata_and_volume_stats() {
    let fixture = tempdir().unwrap();
    let mut directory = fixture.path().to_path_buf();
    while directory.as_os_str().len() < 280 {
        directory.push("long-directory-component");
    }
    fs::create_dir_all(&directory).unwrap();
    let file_path = directory.join("payload.bin");
    fs::write(&file_path, b"hello").unwrap();

    let root = normalized(fixture.path());
    let metadata = read_metadata(&normalized(&file_path)).unwrap();
    let volume = query_volume_stats(&root).unwrap();
    let (sender, receiver) = mpsc::channel();
    let summary = WindowsFileScanner
        .scan(
            root.clone(),
            ScanOptions::default(),
            sender,
            CancellationToken::new(),
        )
        .unwrap();
    let files: Vec<_> = receiver
        .into_iter()
        .filter_map(|event| match event {
            ScanEvent::File(file) => Some(file),
            _ => None,
        })
        .collect();

    assert_eq!(summary.status(), ScanStatus::Complete);
    assert_eq!(summary.file_count(), 1);
    assert_eq!(summary.logical_size(), 5);
    assert!(summary.allocated_size() >= 5);
    assert_eq!(files.len(), 1);
    assert_eq!(files[0].identity(), metadata.identity());
    assert_eq!(files[0].facts().last_write_time, metadata.last_write_time());
    assert_eq!(files[0].facts().attributes, metadata.attributes());
    assert_eq!(files[0].facts().link_count, metadata.link_count());
    assert_eq!(files[0].facts().lock_state, LockState::Available);
    assert_eq!(
        files[0].facts().physical_path.as_deref(),
        Some(metadata.physical_path())
    );
    assert_eq!(metadata.logical_size(), 5);
    assert!(metadata.allocated_size() >= 5);
    assert!(metadata.last_write_time() > 0);
    assert!(volume.total_bytes() > 0);
    assert!(volume.total_free_bytes() <= volume.total_bytes());
    validate_physical_target(&root, &normalized(&file_path)).unwrap();
    assert!(paths_equal_ordinal(r"C:\Temp\File", r"c:\temp\file"));
}

#[test]
fn cancellation_and_entry_limit_return_non_final_summaries() {
    let fixture = tempdir().unwrap();
    fs::write(fixture.path().join("one.bin"), b"1").unwrap();
    fs::write(fixture.path().join("two.bin"), b"2").unwrap();
    let root = normalized(fixture.path());

    let cancel = CancellationToken::new();
    cancel.cancel();
    let (sender, receiver) = mpsc::channel();
    let canceled = WindowsFileScanner
        .scan(root.clone(), ScanOptions::default(), sender, cancel)
        .unwrap();
    assert_eq!(canceled.status(), ScanStatus::Canceled);
    assert!(matches!(
        receiver.into_iter().last(),
        Some(ScanEvent::Canceled)
    ));

    let (sender, _receiver) = mpsc::channel();
    let limited = WindowsFileScanner
        .scan(
            root,
            ScanOptions {
                max_entries: 1,
                ..ScanOptions::default()
            },
            sender,
            CancellationToken::new(),
        )
        .unwrap();
    assert_eq!(limited.status(), ScanStatus::Partial);
    assert!(
        limited
            .issues()
            .iter()
            .any(|issue| issue.kind() == ScanIssueKind::TraversalLimit)
    );
}

#[test]
fn cancellation_during_scan_emits_canceled_terminal() {
    let fixture = tempdir().unwrap();
    let nested = fixture.path().join("nested");
    fs::create_dir(&nested).unwrap();
    for index in 0..4096 {
        fs::write(nested.join(format!("{index}.bin")), b"x").unwrap();
    }

    let cancel = CancellationToken::new();
    let worker_cancel = cancel.clone();
    let (sender, receiver) = mpsc::channel();
    let worker = std::thread::spawn(move || {
        WindowsFileScanner.scan(
            normalized(fixture.path()),
            ScanOptions::default(),
            sender,
            worker_cancel,
        )
    });
    assert!(matches!(
        receiver.recv_timeout(std::time::Duration::from_secs(5)),
        Ok(ScanEvent::Directory(_))
    ));
    cancel.cancel();
    let summary = worker.join().unwrap().unwrap();
    let events: Vec<_> = receiver.into_iter().collect();
    assert_eq!(summary.status(), ScanStatus::Canceled);
    assert!(matches!(events.last(), Some(ScanEvent::Canceled)));
}

#[test]
fn locked_files_warn_and_do_not_stop_sibling_enumeration() {
    let fixture = tempdir().unwrap();
    let locked_path = fixture.path().join("locked.bin");
    fs::write(&locked_path, b"locked").unwrap();
    fs::write(fixture.path().join("readable.bin"), b"readable").unwrap();
    let _lock = OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(&locked_path)
        .unwrap();

    let (sender, receiver) = mpsc::channel();
    let summary = WindowsFileScanner
        .scan(
            normalized(fixture.path()),
            ScanOptions::default(),
            sender,
            CancellationToken::new(),
        )
        .unwrap();
    let events: Vec<_> = receiver.into_iter().collect();
    let mut index = ScanIndex::new(normalized(fixture.path()));
    for event in &events {
        match event {
            ScanEvent::Directory(directory) => {
                index.add_directory(directory.clone());
            }
            ScanEvent::File(file) => {
                index.add_file(file.clone());
            }
            ScanEvent::Warning(warning) => {
                index.add_warning(warning.clone());
            }
            _ => {}
        }
    }
    let files: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            ScanEvent::File(file) => Some(file),
            _ => None,
        })
        .collect();

    assert_eq!(summary.status(), ScanStatus::Partial);
    assert_eq!(summary.file_count(), 2);
    assert_eq!(summary.logical_size(), 14);
    assert_eq!(index.statistics().logical_size(), summary.logical_size());
    assert_eq!(
        index.statistics().allocated_size(),
        summary.allocated_size()
    );
    assert!(
        files
            .iter()
            .any(|file| file.path().display().ends_with("readable.bin"))
    );
    assert!(
        files
            .iter()
            .any(|file| file.path().display().ends_with("locked.bin") && file.locked())
    );
    assert!(events.iter().any(|event| matches!(event, ScanEvent::Warning(warning) if warning.kind() == ScanWarningKind::Locked && warning.code() == Some(32))));
    assert!(
        summary
            .issues()
            .iter()
            .any(|issue| issue.kind() == ScanIssueKind::Locked)
    );
}

#[test]
fn physical_target_outside_root_is_rejected() {
    let fixture = tempdir().unwrap();
    let root = fixture.path().join("root");
    fs::create_dir(&root).unwrap();
    let outside = fixture.path().join("outside.bin");
    fs::write(&outside, b"outside").unwrap();

    let error = validate_physical_target(&normalized(&root), &normalized(&outside)).unwrap_err();
    assert_eq!(error.kind(), PlatformErrorKind::PhysicalTargetOutsideRoot);
}

#[test]
fn scan_completes_without_consuming_events_concurrently() {
    let fixture = tempdir().unwrap();
    for index in 0..128 {
        fs::write(fixture.path().join(format!("{index}.bin")), b"x").unwrap();
    }
    let (sender, receiver) = mpsc::channel();
    let summary = WindowsFileScanner
        .scan(
            normalized(fixture.path()),
            ScanOptions::default(),
            sender,
            CancellationToken::new(),
        )
        .unwrap();
    let events: Vec<_> = receiver.into_iter().collect();
    assert_eq!(summary.file_count(), 128);
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, ScanEvent::File(_)))
            .count(),
        128
    );
    assert!(matches!(events.last(), Some(ScanEvent::Completed)));
}

#[test]
fn disconnected_receiver_returns_structured_error() {
    let fixture = tempdir().unwrap();
    let (sender, receiver) = mpsc::channel();
    drop(receiver);

    let error = WindowsFileScanner
        .scan(
            normalized(fixture.path()),
            ScanOptions::default(),
            sender,
            CancellationToken::new(),
        )
        .unwrap_err();
    assert_eq!(error.kind(), PlatformErrorKind::EventReceiverClosed);
}

#[test]
fn ordinal_directory_lookup_does_not_merge_case_sensitive_siblings() {
    let root = normalized(std::path::Path::new(r"C:\"));
    let mut index = ScanIndex::new_with_path_matcher(root, paths_equal_ordinal);
    let users = index
        .add_directory(domain::DirectoryNode::new(
            normalize_path(r"C:\Users").unwrap(),
            index.root_directory_id(),
        ))
        .unwrap();
    assert_eq!(
        index.directory_id(&normalize_path(r"c:\users").unwrap()),
        Some(users)
    );

    index
        .add_directory(domain::DirectoryNode::new(
            normalize_path(r"C:\USERS").unwrap(),
            index.root_directory_id(),
        ))
        .unwrap();
    assert_eq!(
        index.directory_id(&normalize_path(r"c:\users").unwrap()),
        None
    );
}
