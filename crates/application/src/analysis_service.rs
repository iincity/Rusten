use domain::{
    DuplicateGroup, FileFilter, FileRecord, ScanIndex, find_duplicate_groups_with_progress,
};

pub trait FileHashPort {
    fn fast_hash(&self, file: &FileRecord) -> Option<u64>;
    fn strong_hash(&self, file: &FileRecord) -> Option<Vec<u8>>;
}

pub struct AnalysisService<H> {
    hasher: H,
}

#[cfg(test)]
mod tests {
    use super::{AnalysisService, FileHashPort};
    use domain::{FileIdentity, FileRecord, normalize_path};

    struct TestHasher;

    impl FileHashPort for TestHasher {
        fn fast_hash(&self, file: &FileRecord) -> Option<u64> {
            Some(file.logical_size())
        }

        fn strong_hash(&self, file: &FileRecord) -> Option<Vec<u8>> {
            Some(vec![file.logical_size() as u8])
        }
    }

    #[test]
    fn duplicate_analysis_accepts_a_file_batch_without_scan_index() {
        let files = vec![
            FileRecord::new(
                normalize_path(r"C:\Data\a.bin").unwrap(),
                domain::ScanIndex::new(normalize_path(r"C:\Data").unwrap()).root_directory_id(),
                FileIdentity {
                    volume_serial: 1,
                    file_index: 1,
                },
                4,
                4,
            ),
            FileRecord::new(
                normalize_path(r"C:\Data\b.bin").unwrap(),
                domain::ScanIndex::new(normalize_path(r"C:\Data").unwrap()).root_directory_id(),
                FileIdentity {
                    volume_serial: 1,
                    file_index: 2,
                },
                4,
                4,
            ),
        ];

        let groups = AnalysisService::new(TestHasher).duplicate_groups_from_files(&files);

        assert_eq!(groups.len(), 1);
    }
}

impl<H: FileHashPort> AnalysisService<H> {
    pub fn new(hasher: H) -> Self {
        Self { hasher }
    }

    pub fn duplicate_groups(&self, index: &ScanIndex) -> Vec<DuplicateGroup> {
        self.duplicate_groups_from_files(&index.files())
    }

    pub fn duplicate_groups_from_files(&self, files: &[FileRecord]) -> Vec<DuplicateGroup> {
        self.duplicate_groups_from_files_with_progress(files, |_| {})
    }

    pub fn duplicate_groups_from_files_with_progress(
        &self,
        files: &[FileRecord],
        progress: impl FnMut(u8),
    ) -> Vec<DuplicateGroup> {
        find_duplicate_groups_with_progress(
            files,
            |file| self.hasher.fast_hash(file),
            |file| self.hasher.strong_hash(file),
            progress,
        )
    }

    pub fn large_files<'a>(&self, index: &'a ScanIndex, minimum_size: u64) -> Vec<&'a FileRecord> {
        let mut files: Vec<_> = index
            .top_files(ScanIndex::TOP_FILES_CAPACITY)
            .into_iter()
            .filter(|file| file.logical_size() >= minimum_size)
            .collect();
        files.sort_unstable_by_key(|file| std::cmp::Reverse(file.logical_size()));
        files
    }

    pub fn filtered_files(&self, index: &ScanIndex, filter: &FileFilter) -> Vec<FileRecord> {
        index.filter_files(filter)
    }
}
