use std::collections::HashMap;

use crate::{FileIdentity, FileRecord};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DuplicateGroup {
    pub logical_size: u64,
    pub files: Vec<FileRecord>,
    pub reclaimable_bytes: u64,
}

pub fn find_duplicate_groups(
    files: &[FileRecord],
    fast_hash: impl FnMut(&FileRecord) -> Option<u64>,
    strong_hash: impl FnMut(&FileRecord) -> Option<Vec<u8>>,
) -> Vec<DuplicateGroup> {
    find_duplicate_groups_with_progress(files, fast_hash, strong_hash, |_| {})
}

pub fn find_duplicate_groups_with_progress(
    files: &[FileRecord],
    mut fast_hash: impl FnMut(&FileRecord) -> Option<u64>,
    mut strong_hash: impl FnMut(&FileRecord) -> Option<Vec<u8>>,
    mut progress: impl FnMut(u8),
) -> Vec<DuplicateGroup> {
    let mut by_size: HashMap<u64, Vec<&FileRecord>> = HashMap::new();
    for file in files {
        by_size.entry(file.logical_size()).or_default().push(file);
    }
    progress(10);
    let mut by_fast: HashMap<(u64, u64), Vec<&FileRecord>> = HashMap::new();
    let fast_candidates = by_size
        .values()
        .filter(|files| files.len() > 1)
        .map(Vec::len)
        .sum::<usize>();
    let mut fast_processed = 0;
    for (size, candidates) in by_size.into_iter().filter(|(_, files)| files.len() > 1) {
        for file in candidates {
            if let Some(hash) = fast_hash(file) {
                by_fast.entry((size, hash)).or_default().push(file);
            }
            fast_processed += 1;
            if fast_candidates > 0 {
                progress(10 + (fast_processed * 40 / fast_candidates).min(40) as u8);
            }
        }
    }
    let mut by_strong: HashMap<(u64, Vec<u8>), Vec<&FileRecord>> = HashMap::new();
    let strong_candidates = by_fast
        .values()
        .filter(|files| files.len() > 1)
        .map(Vec::len)
        .sum::<usize>();
    let mut strong_processed = 0;
    for ((size, _), candidates) in by_fast.into_iter().filter(|(_, files)| files.len() > 1) {
        for file in candidates {
            if let Some(hash) = strong_hash(file) {
                by_strong.entry((size, hash)).or_default().push(file);
            }
            strong_processed += 1;
            if strong_candidates > 0 {
                progress(50 + (strong_processed * 45 / strong_candidates).min(45) as u8);
            }
        }
    }
    let groups = by_strong
        .into_iter()
        .filter_map(|((size, _), candidates)| {
            let mut unique: HashMap<FileIdentity, &FileRecord> = HashMap::new();
            for file in candidates {
                unique.entry(file.identity()).or_insert(file);
            }
            if unique.len() < 2 {
                return None;
            }
            let files: Vec<_> = unique.into_values().cloned().collect();
            let reclaimable_bytes = files.iter().skip(1).map(FileRecord::allocated_size).sum();
            Some(DuplicateGroup {
                logical_size: size,
                files,
                reclaimable_bytes,
            })
        })
        .collect();
    progress(100);
    groups
}

#[cfg(test)]
mod tests {
    use super::find_duplicate_groups;
    use crate::{FileIdentity, FileRecord, ScanIndex, normalize_path};

    #[test]
    fn duplicate_groups_use_three_filters_and_exclude_hardlinks() {
        let index = ScanIndex::new(normalize_path(r"C:\Data").unwrap());
        let root = index.root_directory_id();
        let files = [
            FileRecord::new(
                normalize_path(r"C:\Data\a.bin").unwrap(),
                root,
                FileIdentity {
                    volume_serial: 1,
                    file_index: 1,
                },
                4,
                4,
            ),
            FileRecord::new(
                normalize_path(r"C:\Data\a-link.bin").unwrap(),
                root,
                FileIdentity {
                    volume_serial: 1,
                    file_index: 1,
                },
                4,
                4,
            ),
            FileRecord::new(
                normalize_path(r"C:\Data\b.bin").unwrap(),
                root,
                FileIdentity {
                    volume_serial: 1,
                    file_index: 2,
                },
                4,
                4,
            ),
            FileRecord::new(
                normalize_path(r"C:\Data\c.bin").unwrap(),
                root,
                FileIdentity {
                    volume_serial: 1,
                    file_index: 3,
                },
                5,
                5,
            ),
        ];
        let groups = find_duplicate_groups(
            &files,
            |_| Some(1),
            |file| Some(vec![file.logical_size() as u8]),
        );
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].files.len(), 2);
        assert_eq!(groups[0].reclaimable_bytes, 4);
    }
}
