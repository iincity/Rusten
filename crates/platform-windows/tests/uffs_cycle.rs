#![cfg(feature = "uffs")]

use uffs_mft::{
    frs::Frs,
    index::{ChildInfo, FileRecord, MftIndex, NO_ENTRY},
    platform::DriveLetter,
};

#[test]
fn mft_directory_cycle_terminates_without_losing_ordinary_file_sizes() {
    let mut index = MftIndex::new(DriveLetter::C);
    for frs in [5, 6, 7] {
        let mut record = FileRecord::new(Frs::new(frs));
        record.stdinfo.set_directory(frs != 7);
        index.records.push(record);
    }
    index.frs_to_idx = vec![NO_ENTRY; 8];
    for (idx, frs) in [5, 6, 7].into_iter().enumerate() {
        index.frs_to_idx[frs] = idx as u32;
    }
    index.records[0].first_child = 0;
    index.records[1].first_child = 1;
    index.records[2].first_stream.size.length = 1234;
    index.records[2].first_stream.size.allocated = 4096;
    index.children = vec![
        ChildInfo {
            child_frs: Frs::new(6),
            next_entry: 2,
            ..Default::default()
        },
        ChildInfo {
            child_frs: Frs::new(5),
            next_entry: NO_ENTRY,
            ..Default::default()
        },
        ChildInfo {
            child_frs: Frs::new(7),
            next_entry: NO_ENTRY,
            ..Default::default()
        },
    ];
    index.compute_tree_metrics();
    assert_eq!(index.records[0].treesize, 1234);
    assert_eq!(index.records[0].tree_allocated, 4096);
    assert!(
        index
            .records
            .iter()
            .filter(|record| record.is_directory())
            .all(|record| record.descendants > 0)
    );

    // A completed visit through another hardlink must still contribute its share.
    index.records[2].name_count = 2;
    index.children[2].next_entry = 3;
    index.children.push(ChildInfo {
        child_frs: Frs::new(7),
        name_index: 1,
        next_entry: NO_ENTRY,
        ..Default::default()
    });
    index.compute_tree_metrics();
    assert_eq!(index.records[0].treesize, 1234);
    assert_eq!(index.records[0].tree_allocated, 4096);
}
