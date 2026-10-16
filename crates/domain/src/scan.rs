use crate::{CleanupCandidate, DirectoryNode, FileRecord, NormalizedPath};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScanStatus {
    InProgress,
    Complete,
    Partial,
    Canceled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScanEngine {
    Uffs,
    Win32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScanPhase {
    Starting,
    LoadingCache,
    Indexing,
    Preparing,
    Enumerating,
    Finalizing,
    AnalyzingDuplicates,
    Fallback,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ScanStatistics {
    directory_count: u64,
    file_count: u64,
    logical_size: u64,
    allocated_size: u128,
}

impl ScanStatistics {
    pub fn directory_count(&self) -> u64 {
        self.directory_count
    }

    pub fn file_count(&self) -> u64 {
        self.file_count
    }

    pub fn logical_size(&self) -> u64 {
        self.logical_size
    }

    pub fn allocated_size(&self) -> u64 {
        self.allocated_size.min(u64::MAX as u128) as u64
    }

    pub(crate) fn add_directory(&mut self) {
        self.directory_count = self.directory_count.saturating_add(1);
    }

    pub(crate) fn add_file(&mut self, logical_size: u64, allocated_size: u64) {
        self.file_count = self.file_count.saturating_add(1);
        self.logical_size = self.logical_size.saturating_add(logical_size);
        self.allocated_size += allocated_size as u128;
    }

    pub(crate) fn adjust_allocated_size(&mut self, old: u64, new: u64) {
        self.allocated_size = self.allocated_size - old as u128 + new as u128;
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScanWarningKind {
    AccessDenied,
    CrossVolumeSkipped,
    Cycle,
    Locked,
    MetadataUnavailable,
    PathTooLong,
    PhysicalTargetOutsideRoot,
    ReparsePointSkipped,
    TraversalLimit,
    VolumeOffline,
    Other,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScanWarning {
    path: Option<NormalizedPath>,
    message: String,
    kind: ScanWarningKind,
    code: Option<u32>,
}

impl ScanWarning {
    pub fn new(path: Option<NormalizedPath>, message: impl Into<String>) -> Self {
        Self {
            path,
            message: message.into(),
            kind: ScanWarningKind::Other,
            code: None,
        }
    }

    pub fn structured(
        path: Option<NormalizedPath>,
        kind: ScanWarningKind,
        code: Option<u32>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            path,
            kind,
            code,
            message: message.into(),
        }
    }

    pub fn kind(&self) -> ScanWarningKind {
        self.kind
    }

    pub fn code(&self) -> Option<u32> {
        self.code
    }

    pub fn path(&self) -> Option<&NormalizedPath> {
        self.path.as_ref()
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScanEvent {
    ScannerStatus {
        engine: ScanEngine,
        phase: ScanPhase,
        total_entries: Option<u64>,
        processed_entries: Option<u64>,
    },
    Directory(DirectoryNode),
    File(FileRecord),
    ReparsePoint(FileRecord),
    CleanupCandidate(CleanupCandidate),
    Progress(ScanStatistics),
    Warning(ScanWarning),
    Completed,
    Partial,
    Canceled,
}
