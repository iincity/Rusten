use domain::{FileIdentity, FileRecord, ScanIndex, find_duplicate_groups, normalize_path};

#[test]
fn identical_content_groups_do_not_make_hardlinks_look_reclaimable_twice() {
    let index = ScanIndex::new(normalize_path(r"C:\Data").unwrap());
    let root = index.root_directory_id();
    let files = vec![
        FileRecord::new(
            normalize_path(r"C:\Data\a.bin").unwrap(),
            root,
            FileIdentity {
                volume_serial: 1,
                file_index: 1,
            },
            8,
            8,
        ),
        FileRecord::new(
            normalize_path(r"C:\Data\b.bin").unwrap(),
            root,
            FileIdentity {
                volume_serial: 1,
                file_index: 2,
            },
            8,
            8,
        ),
    ];
    let groups = find_duplicate_groups(&files, |_| Some(1), |_| Some(vec![9]));
    assert_eq!(groups[0].reclaimable_bytes, 8);
}
