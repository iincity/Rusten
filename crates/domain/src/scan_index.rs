use std::{
    cmp::{Ordering, Reverse},
    collections::{BinaryHeap, HashMap},
};

use crate::{
    CleanupCandidate, DirectoryId, DirectoryNode, FileFacts, FileIdentity, FileRecord,
    NormalizedPath, ScanStatistics, ScanStatus, ScanWarning,
};

#[derive(Debug)]
struct TopFileEntry {
    sequence: u64,
    file: FileRecord,
}

impl PartialEq for TopFileEntry {
    fn eq(&self, other: &Self) -> bool {
        self.file.logical_size().max(self.file.allocated_size())
            == other.file.logical_size().max(other.file.allocated_size())
            && self.sequence == other.sequence
    }
}

impl Eq for TopFileEntry {}

impl PartialOrd for TopFileEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for TopFileEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        self.file
            .logical_size()
            .max(self.file.allocated_size())
            .cmp(&other.file.logical_size().max(other.file.allocated_size()))
            .then_with(|| self.sequence.cmp(&other.sequence))
    }
}

#[derive(Debug)]
struct PhysicalOwner {
    // Allocated bytes belong to the smallest (lexical key, original path).
    path: String,
    parent: DirectoryId,
    allocated_size: u64,
}

#[derive(Debug)]
struct FileFingerprint {
    path: String,
    parent: DirectoryId,
    logical_size: u64,
    allocated_size: u64,
    facts: FileFacts,
}

#[derive(Debug)]
pub struct ScanIndex {
    root: NormalizedPath,
    directory_arena: Vec<DirectoryNode>,
    directory_lookup: HashMap<String, DirectoryId>,
    path_matcher: Option<fn(&str, &str) -> bool>,
    top_files_heap: BinaryHeap<Reverse<TopFileEntry>>,
    cleanup_candidates: Vec<CleanupCandidate>,
    physical_owners: HashMap<FileIdentity, PhysicalOwner>,
    file_fingerprints: HashMap<FileIdentity, Vec<FileFingerprint>>,
    reparse_points: Vec<FileRecord>,
    statistics: ScanStatistics,
    warnings: Vec<ScanWarning>,
    omitted_warnings: u64,
    status: ScanStatus,
    next_sequence: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FileFilter {
    pub minimum_size: Option<u64>,
    pub modified_before: Option<u64>,
    pub extensions: Vec<String>,
    pub owner: Option<String>,
}

impl ScanIndex {
    pub const TOP_FILES_CAPACITY: usize = 1000;
    pub const WARNINGS_CAPACITY: usize = 1024;

    pub fn new(root: NormalizedPath) -> Self {
        let root_id = DirectoryId(0);
        let mut directory_lookup = HashMap::new();
        directory_lookup.insert(root.display().to_owned(), root_id);

        let mut statistics = ScanStatistics::default();
        statistics.add_directory();

        Self {
            root: root.clone(),
            directory_arena: vec![DirectoryNode::root(root)],
            directory_lookup,
            path_matcher: None,
            top_files_heap: BinaryHeap::new(),
            cleanup_candidates: Vec::new(),
            physical_owners: HashMap::new(),
            file_fingerprints: HashMap::new(),
            reparse_points: Vec::new(),
            statistics,
            warnings: Vec::new(),
            omitted_warnings: 0,
            status: ScanStatus::InProgress,
            next_sequence: 0,
        }
    }

    pub fn new_with_path_matcher(root: NormalizedPath, matcher: fn(&str, &str) -> bool) -> Self {
        let mut index = Self::new(root);
        index.path_matcher = Some(matcher);
        index
    }

    pub fn root(&self) -> &NormalizedPath {
        &self.root
    }

    pub fn root_directory_id(&self) -> DirectoryId {
        DirectoryId(0)
    }

    pub fn add_directory(&mut self, directory: DirectoryNode) -> Option<DirectoryId> {
        if self.status != ScanStatus::InProgress {
            return None;
        }
        let path_key = directory.path().display().to_owned();
        if let Some(id) = self.directory_lookup.get(&path_key) {
            return Some(*id);
        }

        let parent = directory
            .parent()
            .expect("only the ScanIndex root may omit a parent");
        assert!(
            parent.0 < self.directory_arena.len(),
            "parent directory must be added before its child"
        );

        let id = DirectoryId(self.directory_arena.len());
        self.directory_lookup.insert(path_key, id);
        self.directory_arena[parent.0].add_child(id);
        self.directory_arena.push(directory);
        self.statistics.add_directory();
        Some(id)
    }

    pub fn add_file(&mut self, file: FileRecord) -> bool {
        if self.status != ScanStatus::InProgress {
            return false;
        }
        let parent = file.parent();
        assert!(
            parent.0 < self.directory_arena.len(),
            "file parent directory must be added before the file"
        );

        self.claim_physical_allocation(&file);
        self.remember_file(&file);
        self.aggregate_file(parent, file.logical_size(), 0);
        self.statistics.add_file(file.logical_size(), 0);
        self.retain_top_file(file);
        true
    }

    pub fn add_reparse_point(&mut self, file: FileRecord) -> bool {
        if self.status == ScanStatus::Canceled {
            return false;
        }
        self.reparse_points.push(file);
        true
    }

    pub fn reparse_points(&self) -> &[FileRecord] {
        &self.reparse_points
    }

    pub fn add_cleanup_candidate(&mut self, candidate: CleanupCandidate) -> bool {
        if self.status != ScanStatus::InProgress {
            return false;
        }
        self.cleanup_candidates.push(candidate);
        true
    }

    pub fn add_cleanup_candidates_after_scan(
        &mut self,
        candidates: impl IntoIterator<Item = CleanupCandidate>,
    ) {
        self.cleanup_candidates.extend(candidates);
    }

    pub fn add_warning(&mut self, warning: ScanWarning) -> bool {
        if self.status != ScanStatus::InProgress {
            return false;
        }
        if self.warnings.len() < Self::WARNINGS_CAPACITY {
            self.warnings.push(warning);
        } else {
            self.omitted_warnings = self.omitted_warnings.saturating_add(1);
        }
        true
    }

    pub fn top_files(&self, limit: usize) -> Vec<&FileRecord> {
        let mut entries: Vec<_> = self.top_files_heap.iter().map(|entry| &entry.0).collect();
        entries.sort_unstable_by(|left, right| right.cmp(left));
        entries
            .into_iter()
            .take(limit)
            .map(|entry| &entry.file)
            .collect()
    }

    pub fn files(&self) -> Vec<FileRecord> {
        self.file_fingerprints
            .iter()
            .flat_map(|(identity, records)| {
                records.iter().map(move |record| {
                    FileRecord::new(
                        NormalizedPath::from_display_unchecked(record.path.clone()),
                        record.parent,
                        *identity,
                        record.logical_size,
                        record.allocated_size,
                    )
                    .with_facts(record.facts.clone())
                })
            })
            .collect()
    }

    pub fn filter_files(&self, filter: &FileFilter) -> Vec<FileRecord> {
        self.files()
            .into_iter()
            .filter(|file| {
                filter
                    .minimum_size
                    .is_none_or(|minimum| file.logical_size() >= minimum)
                    && filter
                        .modified_before
                        .is_none_or(|before| file.facts().last_write_time < before)
                    && (filter.extensions.is_empty()
                        || file
                            .path()
                            .display()
                            .rsplit_once('.')
                            .is_some_and(|(_, extension)| {
                                filter
                                    .extensions
                                    .iter()
                                    .any(|wanted| wanted.eq_ignore_ascii_case(extension))
                            }))
                    && filter
                        .owner
                        .as_ref()
                        .is_none_or(|owner| file.facts().owner.as_ref() == Some(owner))
            })
            .collect()
    }

    pub fn directory(&self, id: DirectoryId) -> Option<&DirectoryNode> {
        self.directory_arena.get(id.0)
    }

    pub fn directories(&self) -> &[DirectoryNode] {
        &self.directory_arena
    }

    pub fn directory_id(&self, path: &NormalizedPath) -> Option<DirectoryId> {
        if let Some(id) = self.directory_lookup.get(path.display()) {
            return Some(*id);
        }
        let matcher = self.path_matcher?;

        let mut matches = self
            .directory_arena
            .iter()
            .enumerate()
            .filter(|(_, directory)| matcher(directory.path().display(), path.display()));
        let (index, _) = matches.next()?;
        if matches.next().is_some() {
            return None;
        }
        Some(DirectoryId(index))
    }

    pub fn cleanup_candidates(&self) -> &[CleanupCandidate] {
        &self.cleanup_candidates
    }

    pub fn statistics(&self) -> &ScanStatistics {
        &self.statistics
    }

    pub fn contains_file_identity(&self, identity: FileIdentity) -> bool {
        self.physical_owners.contains_key(&identity)
    }

    pub fn contains_file_record(&self, file: &FileRecord) -> bool {
        self.file_fingerprints
            .get(&file.identity())
            .is_some_and(|records| {
                records.iter().any(|record| {
                    record.path == file.path().display()
                        && record.parent == file.parent()
                        && record.logical_size == file.logical_size()
                        && record.allocated_size == file.allocated_size()
                        && record.facts == *file.facts()
                })
            })
    }

    pub fn warnings(&self) -> &[ScanWarning] {
        &self.warnings
    }

    pub fn omitted_warnings(&self) -> u64 {
        self.omitted_warnings
    }

    pub fn status(&self) -> ScanStatus {
        self.status
    }

    pub fn is_complete(&self) -> bool {
        self.status == ScanStatus::Complete
    }

    pub fn mark_complete(&mut self) -> bool {
        self.mark_terminal(ScanStatus::Complete)
    }

    pub fn mark_partial(&mut self) -> bool {
        self.mark_terminal(ScanStatus::Partial)
    }

    pub fn mark_canceled(&mut self) -> bool {
        self.mark_terminal(ScanStatus::Canceled)
    }

    pub fn cancel_after_request(&mut self) {
        self.status = ScanStatus::Canceled;
    }

    fn mark_terminal(&mut self, status: ScanStatus) -> bool {
        if self.status != ScanStatus::InProgress {
            return false;
        }
        self.status = status;
        true
    }

    fn aggregate_file(
        &mut self,
        mut directory_id: DirectoryId,
        logical_size: u64,
        allocated_size: u64,
    ) {
        loop {
            let directory = &mut self.directory_arena[directory_id.0];
            directory.add_file(logical_size, allocated_size);
            match directory.parent() {
                Some(parent) => directory_id = parent,
                None => break,
            }
        }
    }

    fn claim_physical_allocation(&mut self, file: &FileRecord) {
        let identity = file.identity();
        let path = file.path().display();
        if let Some(owner) = self.physical_owners.get(&identity) {
            if path >= owner.path.as_str() {
                return;
            }
            let old_parent = owner.parent;
            let old_size = owner.allocated_size;
            self.adjust_allocated(old_parent, old_size, 0);
        }
        self.adjust_allocated(file.parent(), 0, file.allocated_size());
        self.physical_owners.insert(
            identity,
            PhysicalOwner {
                path: path.to_owned(),
                parent: file.parent(),
                allocated_size: file.allocated_size(),
            },
        );
    }

    fn remember_file(&mut self, file: &FileRecord) {
        let records = self.file_fingerprints.entry(file.identity()).or_default();
        if records.iter().any(|record| {
            record.path == file.path().display()
                && record.parent == file.parent()
                && record.logical_size == file.logical_size()
                && record.allocated_size == file.allocated_size()
                && record.facts == *file.facts()
        }) {
            return;
        }
        records.push(FileFingerprint {
            path: file.path().display().to_owned(),
            parent: file.parent(),
            logical_size: file.logical_size(),
            allocated_size: file.allocated_size(),
            facts: file.facts().clone(),
        });
    }

    fn adjust_allocated(&mut self, mut directory_id: DirectoryId, old: u64, new: u64) {
        self.statistics.adjust_allocated_size(old, new);
        loop {
            let directory = &mut self.directory_arena[directory_id.0];
            directory.adjust_allocated_size(old, new);
            match directory.parent() {
                Some(parent) => directory_id = parent,
                None => break,
            }
        }
    }

    fn retain_top_file(&mut self, file: FileRecord) {
        let entry = TopFileEntry {
            sequence: self.next_sequence,
            file,
        };
        self.next_sequence = self.next_sequence.saturating_add(1);

        if self.top_files_heap.len() < Self::TOP_FILES_CAPACITY {
            self.top_files_heap.push(Reverse(entry));
        } else if self
            .top_files_heap
            .peek()
            .is_some_and(|smallest| entry > smallest.0)
        {
            self.top_files_heap.pop();
            self.top_files_heap.push(Reverse(entry));
        }
    }
}
