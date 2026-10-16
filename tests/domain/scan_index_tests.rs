use domain::{
    CleanupCandidate, CleanupMode, Confidence, DirectoryNode, FileFacts, FileFilter, FileIdentity,
    FileRecord, RecoveryMode, RiskLevel, ScanIndex, ScanStatus, ScanWarning, normalize_path,
};

fn path(value: &str) -> domain::NormalizedPath {
    normalize_path(value).unwrap()
}

#[test]
fn index_filters_size_time_and_extension_without_rescanning() {
    let mut index = ScanIndex::new(path(r"C:\Data"));
    let root = index.root_directory_id();
    index.add_file(
        file(r"C:\Data\large.TMP", root, 1, 100, 100).with_facts(FileFacts {
            last_write_time: 10,
            ..Default::default()
        }),
    );
    index.add_file(
        file(r"C:\Data\small.log", root, 2, 10, 10).with_facts(FileFacts {
            last_write_time: 1,
            ..Default::default()
        }),
    );
    let filtered = index.filter_files(&FileFilter {
        minimum_size: Some(50),
        modified_before: Some(20),
        extensions: vec!["tmp".into()],
        owner: None,
    });
    assert_eq!(filtered.len(), 1);
    assert_eq!(filtered[0].path().display(), r"C:\Data\large.TMP");
}

fn file(
    path_value: &str,
    parent: domain::DirectoryId,
    file_index: u128,
    logical_size: u64,
    allocated_size: u64,
) -> FileRecord {
    FileRecord::new(
        path(path_value),
        parent,
        FileIdentity {
            volume_serial: 7,
            file_index,
        },
        logical_size,
        allocated_size,
    )
}

fn candidate(path_value: &str) -> CleanupCandidate {
    CleanupCandidate {
        path: path(path_value),
        file_identity: None,
        rule_id: None,
        category: None,
        reason: None,
        estimated_size: None,
        quick_clean_eligible: false,
        quick_clean_block_reason: None,
        mode: Some(CleanupMode::Quick),
        risk: Some(RiskLevel::P3),
        confidence: Some(Confidence::High),
        recovery: Some(RecoveryMode::Recycle),
        locked: Some(false),
        path_allowed: Some(true),
        file_identity_unchanged: Some(true),
        last_write_time: None,
    }
}

fn hardlink_allocation_snapshot(zed_first: bool) -> (u64, u64, u64, u64, u64) {
    let mut index = ScanIndex::new(path(r"C:\"));
    let root = index.root_directory_id();
    let alpha = index
        .add_directory(DirectoryNode::new(path(r"C:\Alpha"), root))
        .unwrap();
    let zed = index
        .add_directory(DirectoryNode::new(path(r"C:\Zed"), root))
        .unwrap();

    if zed_first {
        index.add_file(file(r"C:\Zed\shared.bin", zed, 42, 100, 128));
        index.add_file(file(r"C:\Alpha\shared.bin", alpha, 42, 100, 128));
    } else {
        index.add_file(file(r"C:\Alpha\shared.bin", alpha, 42, 100, 128));
        index.add_file(file(r"C:\Zed\shared.bin", zed, 42, 100, 128));
    }

    (
        index.directory(root).unwrap().logical_size(),
        index.directory(root).unwrap().allocated_size(),
        index.directory(alpha).unwrap().allocated_size(),
        index.directory(zed).unwrap().allocated_size(),
        index.statistics().allocated_size(),
    )
}

#[test]
fn aggregates_directory_sizes_and_counts_hardlink_allocation_once() {
    let mut index = ScanIndex::new(path(r"C:\"));
    let root = index.root_directory_id();
    let first = index
        .add_directory(DirectoryNode::new(path(r"C:\First"), root))
        .unwrap();
    let second = index
        .add_directory(DirectoryNode::new(path(r"C:\Second"), root))
        .unwrap();

    index.add_file(file(r"C:\First\shared.bin", first, 42, 100, 128));
    index.add_file(file(r"C:\Second\shared.bin", second, 42, 100, 128));

    let root_node = index.directory(root).unwrap();
    assert_eq!(root_node.logical_size(), 200);
    assert_eq!(root_node.allocated_size(), 128);
    assert_eq!(root_node.file_count(), 2);

    assert_eq!(index.directory(first).unwrap().allocated_size(), 128);
    assert_eq!(index.directory(second).unwrap().allocated_size(), 0);
    assert_eq!(index.statistics().logical_size(), 200);
    assert_eq!(index.statistics().allocated_size(), 128);
    assert_eq!(index.statistics().file_count(), 2);
}

#[test]
fn hardlink_allocation_owner_is_independent_of_enumeration_order() {
    let alpha_first = hardlink_allocation_snapshot(false);
    let zed_first = hardlink_allocation_snapshot(true);

    assert_eq!(alpha_first, (200, 128, 128, 0, 128));
    assert_eq!(zed_first, alpha_first);
}

#[test]
fn hardlink_reassignment_recomputes_saturated_allocations() {
    let mut index = ScanIndex::new(path(r"C:\"));
    let root = index.root_directory_id();
    let alpha = index
        .add_directory(DirectoryNode::new(path(r"C:\Alpha"), root))
        .unwrap();
    let zed = index
        .add_directory(DirectoryNode::new(path(r"C:\Zed"), root))
        .unwrap();

    index.add_file(file(r"C:\Zed\shared.bin", zed, 42, 1, u64::MAX));
    index.add_file(file(r"C:\Zed\filler.bin", zed, 43, 1, 1));
    index.add_file(file(r"C:\Alpha\shared.bin", alpha, 42, 1, u64::MAX));

    assert_eq!(index.directory(alpha).unwrap().allocated_size(), u64::MAX);
    assert_eq!(index.directory(zed).unwrap().allocated_size(), 1);
    assert_eq!(index.directory(root).unwrap().allocated_size(), u64::MAX);
    assert_eq!(index.statistics().allocated_size(), u64::MAX);
    assert_eq!(index.statistics().logical_size(), 3);
}

#[test]
fn duplicate_identity_and_owner_path_allocate_physical_size_once() {
    let mut index = ScanIndex::new(path(r"C:\"));
    let root = index.root_directory_id();
    let owner = index
        .add_directory(DirectoryNode::new(path(r"C:\Owner"), root))
        .unwrap();

    index.add_file(file(r"C:\Owner\same.bin", owner, 42, 10, 128));
    index.add_file(file(r"C:\Owner\same.bin", owner, 42, 10, 128));

    assert_eq!(index.directory(owner).unwrap().logical_size(), 20);
    assert_eq!(index.directory(owner).unwrap().allocated_size(), 128);
    assert_eq!(index.statistics().logical_size(), 20);
    assert_eq!(index.statistics().allocated_size(), 128);
}

#[test]
fn indexes_parent_child_relationships_and_paths() {
    let mut index = ScanIndex::new(path(r"C:\"));
    let root = index.root_directory_id();
    let users = index
        .add_directory(DirectoryNode::new(path(r"C:\Users"), root))
        .unwrap();
    let alice = index
        .add_directory(DirectoryNode::new(path(r"C:\Users\Alice"), users))
        .unwrap();

    assert_eq!(index.directory(root).unwrap().children(), &[users]);
    assert_eq!(index.directory(users).unwrap().children(), &[alice]);
    assert_eq!(index.directory(alice).unwrap().parent(), Some(users));
    assert_eq!(index.directory_id(&path(r"C:\Users\Alice")), Some(alice));
    assert_eq!(index.statistics().directory_count(), 3);
}

#[test]
fn directory_lookup_preserves_case_sensitive_siblings() {
    let mut index = ScanIndex::new(path(r"C:\"));
    let root = index.root_directory_id();
    let users = index
        .add_directory(DirectoryNode::new(path(r"C:\Users"), root))
        .unwrap();

    let other = index
        .add_directory(DirectoryNode::new(path(r"C:\USERS"), root))
        .unwrap();
    assert_ne!(users, other);
    assert_eq!(index.directory_id(&path(r"C:\Users")), Some(users));
    assert_eq!(index.directory_id(&path(r"C:\USERS")), Some(other));
    assert_eq!(index.directory_id(&path(r"c:\users")), None);
    assert_eq!(index.statistics().directory_count(), 3);
}

#[test]
fn matching_lookup_accepts_one_case_variant_but_rejects_ambiguous_siblings() {
    let mut index =
        ScanIndex::new_with_path_matcher(path(r"C:\"), str_equal_ascii_case_insensitive);
    let root = index.root_directory_id();
    let users = index
        .add_directory(DirectoryNode::new(path(r"C:\Users"), root))
        .unwrap();
    assert_eq!(index.directory_id(&path(r"c:\users")), Some(users));

    index
        .add_directory(DirectoryNode::new(path(r"C:\USERS"), root))
        .unwrap();
    assert_eq!(index.directory_id(&path(r"c:\users")), None);
}

fn str_equal_ascii_case_insensitive(left: &str, right: &str) -> bool {
    left.eq_ignore_ascii_case(right)
}

#[test]
fn hardlink_owner_reassignment_preserves_exact_saturated_totals() {
    let mut index = ScanIndex::new(path(r"C:\"));
    let root = index.root_directory_id();
    let alpha = index
        .add_directory(DirectoryNode::new(path(r"C:\Alpha"), root))
        .unwrap();
    let zed = index
        .add_directory(DirectoryNode::new(path(r"C:\Zed"), root))
        .unwrap();
    index.add_file(file(r"C:\Zed\shared", zed, 7, 1, u64::MAX));
    index.add_file(file(r"C:\Zed\other", zed, 8, 1, u64::MAX));
    index.add_file(file(r"C:\Alpha\shared", alpha, 7, 1, u64::MAX));
    assert_eq!(index.directory(zed).unwrap().allocated_size(), u64::MAX);
    assert_eq!(index.directory(alpha).unwrap().allocated_size(), u64::MAX);
    assert_eq!(index.statistics().allocated_size(), u64::MAX);
}

#[test]
fn serves_bounded_top_files_and_cleanup_candidates_from_one_index() {
    let mut index = ScanIndex::new(path(r"C:\"));
    let root = index.root_directory_id();

    for number in 0..=ScanIndex::TOP_FILES_CAPACITY {
        index.add_file(file(
            &format!(r"C:\file-{number}.bin"),
            root,
            number as u128,
            number as u64,
            number as u64,
        ));
    }
    index.add_cleanup_candidate(candidate(r"C:\file-1000.bin"));

    let top = index.top_files(2);
    assert_eq!(top.len(), 2);
    assert_eq!(top[0].logical_size(), 1000);
    assert_eq!(top[1].logical_size(), 999);
    assert_eq!(
        index.top_files(usize::MAX).len(),
        ScanIndex::TOP_FILES_CAPACITY
    );
    assert_eq!(index.cleanup_candidates().len(), 1);
}

#[test]
fn top_files_include_allocated_only_files_in_size_order() {
    let mut index = ScanIndex::new(path(r"C:\"));
    let root = index.root_directory_id();
    index.add_file(file(r"C:\logical.bin", root, 1, 2048, 0));
    index.add_file(file(r"C:\allocated.bin", root, 2, 0, 4096));

    assert_eq!(index.top_files(1)[0].path().display(), r"C:\allocated.bin");
}

#[test]
fn canceled_scan_keeps_partial_data_and_is_not_final() {
    let mut index = ScanIndex::new(path(r"C:\"));
    let root = index.root_directory_id();
    index.add_file(file(r"C:\partial.bin", root, 1, 10, 16));
    index.add_warning(ScanWarning::new(
        Some(path(r"C:\Restricted")),
        "access denied",
    ));

    index.mark_canceled();

    assert_eq!(index.status(), ScanStatus::Canceled);
    assert!(!index.is_complete());
    assert_eq!(index.directory(root).unwrap().logical_size(), 10);
    assert_eq!(index.top_files(10).len(), 1);
    assert_eq!(index.warnings().len(), 1);
}

#[test]
fn completed_scan_is_marked_final() {
    let mut index = ScanIndex::new(path(r"C:\"));

    index.mark_complete();

    assert_eq!(index.status(), ScanStatus::Complete);
    assert!(index.is_complete());
}

#[test]
fn scan_with_access_warnings_can_finish_partial() {
    let mut index = ScanIndex::new(path(r"C:\"));
    index.add_warning(ScanWarning::new(
        Some(path(r"C:\Restricted")),
        "access denied",
    ));

    index.mark_partial();

    assert_eq!(index.status(), ScanStatus::Partial);
    assert!(!index.is_complete());
    assert_eq!(index.warnings().len(), 1);
}

#[test]
fn terminal_status_cannot_transition() {
    let mut completed = ScanIndex::new(path(r"C:\"));
    assert!(completed.mark_complete());
    assert!(!completed.mark_canceled());
    assert_eq!(completed.status(), ScanStatus::Complete);

    let mut partial = ScanIndex::new(path(r"C:\"));
    assert!(partial.mark_partial());
    assert!(!partial.mark_complete());
    assert_eq!(partial.status(), ScanStatus::Partial);

    let mut canceled = ScanIndex::new(path(r"C:\"));
    assert!(canceled.mark_canceled());
    assert!(!canceled.mark_complete());
    assert_eq!(canceled.status(), ScanStatus::Canceled);
}

#[test]
fn terminal_index_ignores_trailing_files_and_directories() {
    let mut index = ScanIndex::new(path(r"C:\"));
    let root = index.root_directory_id();
    assert!(index.add_file(file(r"C:\kept.bin", root, 1, 10, 16)));
    assert!(index.mark_canceled());

    assert!(!index.add_file(file(r"C:\trailing.bin", root, 2, 20, 32)));
    assert_eq!(
        index.add_directory(DirectoryNode::new(path(r"C:\Trailing"), root)),
        None
    );

    assert_eq!(index.statistics().directory_count(), 1);
    assert_eq!(index.statistics().file_count(), 1);
    assert_eq!(index.statistics().logical_size(), 10);
    assert_eq!(index.statistics().allocated_size(), 16);
    assert_eq!(index.top_files(10).len(), 1);
}

#[test]
fn terminal_index_ignores_trailing_candidates_and_warnings() {
    for terminal in [
        ScanStatus::Complete,
        ScanStatus::Partial,
        ScanStatus::Canceled,
    ] {
        let mut index = ScanIndex::new(path(r"C:\"));
        match terminal {
            ScanStatus::Complete => assert!(index.mark_complete()),
            ScanStatus::Partial => assert!(index.mark_partial()),
            ScanStatus::Canceled => assert!(index.mark_canceled()),
            ScanStatus::InProgress => unreachable!(),
        }

        assert!(!index.add_cleanup_candidate(candidate(r"C:\trailing.tmp")));
        assert!(!index.add_warning(ScanWarning::new(
            Some(path(r"C:\Restricted")),
            "trailing warning",
        )));
        assert!(index.cleanup_candidates().is_empty());
        assert!(index.warnings().is_empty());
    }
}

#[test]
fn completed_index_can_attach_candidates_from_the_same_scan() {
    let mut index = ScanIndex::new(path(r"C:\"));
    assert!(index.mark_complete());

    index.add_cleanup_candidates_after_scan([candidate(r"C:\Temp\cache.tmp")]);

    assert_eq!(index.cleanup_candidates().len(), 1);
}

#[test]
fn warning_details_are_bounded() {
    let mut index = ScanIndex::new(path(r"C:\"));
    for _ in 0..ScanIndex::WARNINGS_CAPACITY + 2 {
        assert!(index.add_warning(ScanWarning::new(None, "unreadable")));
    }
    assert_eq!(index.warnings().len(), ScanIndex::WARNINGS_CAPACITY);
    assert_eq!(index.omitted_warnings(), 2);
}
