mod enumerator;
mod hardlink;
mod identity;
mod lock;
mod metadata;
mod reparse;
mod uffs_driver;

use std::{
    collections::HashSet,
    error::Error,
    fmt, fs,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{SendError, Sender, SyncSender},
    },
};

use domain::{
    FileFacts, FileIdentity, FileRecord, LockState, NormalizedPath, ReparseKind, ScanEvent,
    ScanIndex, ScanStatus, normalize_path,
};
use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_CANT_RESOLVE_FILENAME, ERROR_FILENAME_EXCED_RANGE, ERROR_NOT_READY,
    ERROR_SHARING_VIOLATION,
};

pub use enumerator::WindowsFileScanner;
pub use hardlink::is_hardlink_pair;
pub use identity::{paths_equal_ordinal, validate_physical_target};
pub use lock::{LockStatus, lock_status};
pub use metadata::{FileMetadata, delete_verified_file, read_metadata};
pub use reparse::{ReparsePointKind, reparse_point_kind};
pub use uffs_driver::FastFileScanner;
#[cfg(feature = "uffs")]
pub use uffs_driver::UffsFileScanner;

pub trait ScanEventSink: Send + 'static {
    fn send_event(&self, event: ScanEvent) -> Result<(), Box<SendError<ScanEvent>>>;
}

pub fn enumerate_statistics_files(
    roots: &[NormalizedPath],
    max_entries: u64,
) -> Result<Vec<FileRecord>, PlatformError> {
    let mut files = Vec::new();
    for root in roots {
        if fs::metadata(root.display()).is_ok_and(|metadata| metadata.is_file()) {
            let metadata = read_metadata(root)?;
            let index = ScanIndex::new(root.clone());
            files.push(FileRecord::new(
                root.clone(),
                index.root_directory_id(),
                metadata.identity(),
                metadata.logical_size(),
                metadata.allocated_size(),
            ));
            continue;
        }
        let (sender, receiver) = std::sync::mpsc::sync_channel(1024);
        let worker = std::thread::spawn({
            let root = root.clone();
            move || {
                WindowsFileScanner.scan(
                    root,
                    ScanOptions {
                        max_depth: 512,
                        max_entries,
                        collect_lock_state: false,
                    },
                    sender,
                    CancellationToken::default(),
                )
            }
        });
        let mut index = ScanIndex::new_with_path_matcher(root.clone(), paths_equal_ordinal);
        for event in receiver {
            match event {
                ScanEvent::Directory(directory) => {
                    index.add_directory(directory);
                }
                ScanEvent::File(file) => {
                    index.add_file(file);
                }
                ScanEvent::ReparsePoint(file) => {
                    index.add_reparse_point(file);
                }
                ScanEvent::Warning(warning) => {
                    index.add_warning(warning);
                }
                ScanEvent::Completed => {
                    index.mark_complete();
                }
                ScanEvent::Partial => {
                    index.mark_partial();
                }
                ScanEvent::Canceled => {
                    index.mark_canceled();
                }
                ScanEvent::CleanupCandidate(_)
                | ScanEvent::Progress(_)
                | ScanEvent::ScannerStatus { .. } => {}
            }
        }
        worker.join().map_err(|_| {
            PlatformError::new(PlatformErrorKind::Io, None, "statistics scan panicked")
        })??;
        files.extend(index.files());
        files.extend(index.reparse_points().iter().cloned());
    }
    Ok(files)
}

pub fn enumerate_statistics_files_for_patterns(
    patterns: &[String],
    max_entries: u64,
) -> Result<Vec<FileRecord>, PlatformError> {
    let mut roots = Vec::new();
    for pattern in patterns {
        for root in statistics_roots_for_pattern(pattern) {
            let key = root.display().to_ascii_lowercase();
            if !roots
                .iter()
                .any(|item: &NormalizedPath| item.display().to_ascii_lowercase() == key)
            {
                roots.push(root);
            }
        }
    }
    if roots.is_empty() {
        return Ok(Vec::new());
    }

    #[cfg(feature = "uffs")]
    if roots
        .iter()
        .all(|root| root.display().to_ascii_lowercase().starts_with(r"c:\"))
        && let Ok(files) = enumerate_statistics_files_for_patterns_uffs(&roots, max_entries)
    {
        return Ok(files);
    }

    // Each statistics root is independent. A small worker pool avoids adding
    // the long tail of large Windows cache trees serially while keeping the
    // number of concurrent metadata walks bounded.
    let worker_count = roots.len().min(8);
    let mut batches = (0..worker_count)
        .map(|_| Vec::new())
        .collect::<Vec<Vec<NormalizedPath>>>();
    for (index, root) in roots.into_iter().enumerate() {
        batches[index % worker_count].push(root);
    }
    let mut files = Vec::new();
    std::thread::scope(|scope| {
        let workers = batches
            .into_iter()
            .map(|batch| {
                scope.spawn(move || {
                    batch
                        .into_iter()
                        .filter_map(|root| {
                            enumerate_statistics_files(std::slice::from_ref(&root), max_entries)
                                .ok()
                        })
                        .flatten()
                        .collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>();
        for worker in workers {
            if let Ok(mut found) = worker.join() {
                files.append(&mut found);
            }
        }
    });
    Ok(files)
}

fn statistics_roots_for_pattern(pattern: &str) -> Vec<NormalizedPath> {
    let normalized = pattern.replace('/', "\\");
    let components = normalized
        .split('\\')
        .filter(|component| !component.is_empty())
        .collect::<Vec<_>>();
    if components.is_empty() {
        return Vec::new();
    }
    let wildcard_indices = components
        .iter()
        .enumerate()
        .filter_map(|(index, component)| component.contains(['*', '?']).then_some(index))
        .collect::<Vec<_>>();
    let last_wildcard = wildcard_indices.last().copied();
    let end = last_wildcard.unwrap_or(components.len());
    let mut candidates = vec![components[0].to_owned()];
    for component in components.iter().take(end).skip(1) {
        if component.contains(['*', '?']) {
            candidates = candidates
                .into_iter()
                .flat_map(|base| {
                    fs::read_dir(&base)
                        .into_iter()
                        .flatten()
                        .filter_map(Result::ok)
                        .filter_map(|entry| {
                            entry
                                .file_type()
                                .ok()
                                .filter(|kind| kind.is_dir())
                                .and_then(|_| {
                                    component_matches(
                                        component,
                                        &entry.file_name().to_string_lossy(),
                                    )
                                    .then(|| entry.path().to_string_lossy().into_owned())
                                })
                        })
                        .collect::<Vec<_>>()
                })
                .collect();
        } else {
            candidates = candidates
                .into_iter()
                .map(|base| join_windows_path(&base, component))
                .collect();
        }
    }
    candidates
        .into_iter()
        .filter_map(|candidate| normalize_path(&candidate).ok())
        .filter(|candidate| {
            fs::metadata(candidate.display())
                .is_ok_and(|metadata| metadata.is_dir() || metadata.is_file())
        })
        .collect()
}

#[cfg(feature = "uffs")]
fn enumerate_statistics_files_for_patterns_uffs(
    roots: &[NormalizedPath],
    max_entries: u64,
) -> Result<Vec<FileRecord>, PlatformError> {
    let root = normalize_path(r"C:\").expect("static C drive root is normalized");
    let volume_serial = read_metadata(&root)?.identity().volume_serial;
    let reader = uffs_mft::MftReader::open(uffs_mft::platform::DriveLetter::C)
        .map_err(|error| PlatformError::new(PlatformErrorKind::Io, None, error.to_string()))?
        .with_use_bitmap(true)
        .with_merge_extensions(false)
        .with_expand_links(true);
    let index = reader
        .read_index_cached_sync(uffs_mft::INDEX_TTL_SECONDS)
        .map_err(|error| PlatformError::new(PlatformErrorKind::Io, None, error.to_string()))?;
    let resolver = uffs_mft::index::PathResolver::build(&index, false);
    let root_id = ScanIndex::new(root).root_directory_id();
    let root_matches = roots
        .iter()
        .filter_map(|root| {
            let metadata = fs::metadata(root.display()).ok()?;
            let file_metadata = read_metadata(root).ok()?;
            let record_number =
                (file_metadata.identity().file_index & 0x0000_ffff_ffff_ffff) as u64;
            let record_index = index.frs_to_idx_opt(uffs_mft::Frs::new(record_number))?;
            Some((record_index, metadata.is_dir()))
        })
        .collect::<Vec<_>>();
    let mut files = Vec::new();
    let mut seen = HashSet::new();

    for (record_index, is_dir) in root_matches {
        collect_uffs_statistics_files(
            &index,
            &resolver,
            record_index,
            is_dir,
            volume_serial,
            root_id,
            max_entries,
            &mut seen,
            &mut files,
        );
        if files.len() as u64 >= max_entries {
            break;
        }
    }
    Ok(files)
}

#[cfg(feature = "uffs")]
#[expect(
    clippy::too_many_arguments,
    reason = "keeps UFFS statistics traversal allocation-free"
)]
fn collect_uffs_statistics_files(
    index: &uffs_mft::MftIndex,
    resolver: &uffs_mft::index::PathResolver,
    record_index: usize,
    include_descendants: bool,
    volume_serial: u64,
    root_id: domain::DirectoryId,
    max_entries: u64,
    seen: &mut HashSet<FileIdentity>,
    files: &mut Vec<FileRecord>,
) {
    if files.len() as u64 >= max_entries || !resolver.is_valid_idx(record_index) {
        return;
    }
    let Some(source) = index.records.get(record_index) else {
        return;
    };
    if source.is_deleted() || source.is_corrupt() || source.is_extension() {
        return;
    }
    if source.is_directory() {
        if !include_descendants || source.reparse_tag != 0 {
            return;
        }
        let mut child_entry = source.first_child;
        while child_entry != uffs_mft::index::NO_ENTRY && (files.len() as u64) < max_entries {
            let Some(child) = index.children.get(child_entry as usize) else {
                break;
            };
            if let Some(child_index) = index.frs_to_idx_opt(child.child_frs) {
                collect_uffs_statistics_files(
                    index,
                    resolver,
                    child_index,
                    true,
                    volume_serial,
                    root_id,
                    max_entries,
                    seen,
                    files,
                );
            }
            child_entry = child.next_entry;
        }
        return;
    }

    let identity = FileIdentity {
        volume_serial,
        file_index: ntfs_file_index(source.frs.raw(), source.sequence_number),
    };
    if !seen.insert(identity) {
        return;
    }
    let path = resolver.materialize_path_cached(index, record_index, &[]);
    let Ok(path) = normalize_path(&path) else {
        return;
    };
    files.push(
        FileRecord::new(
            path,
            root_id,
            identity,
            source.first_stream.size.length,
            source.first_stream.size.allocated,
        )
        .with_facts(FileFacts {
            creation_time: source.stdinfo.created.max(0) as u64,
            last_access_time: source.stdinfo.accessed.max(0) as u64,
            last_write_time: source.stdinfo.modified.max(0) as u64,
            attributes: source.stdinfo.to_attributes(),
            link_count: u32::from(source.name_count),
            lock_state: LockState::Unknown,
            reparse_tag: (source.reparse_tag != 0).then_some(source.reparse_tag),
            reparse_kind: (source.reparse_tag != 0)
                .then_some(ReparseKind::Other(source.reparse_tag)),
            physical_path: None,
            target_path: None,
            target_volume_serial: None,
            owner: None,
        }),
    );
}

#[cfg(feature = "uffs")]
fn ntfs_file_index(record_number: u64, sequence_number: u16) -> u128 {
    ((sequence_number as u128) << 48) | (record_number as u128 & 0x0000_ffff_ffff_ffff)
}

fn join_windows_path(base: &str, component: &str) -> String {
    if base.ends_with(':') {
        format!("{base}\\{component}")
    } else {
        format!(r"{base}\{component}")
    }
}

fn component_matches(pattern: &str, value: &str) -> bool {
    let pattern = pattern.to_ascii_lowercase();
    let value = value.to_ascii_lowercase();
    let (mut pattern_index, mut value_index) = (0, 0);
    let (mut star_index, mut star_value_index) = (None, 0);
    let pattern = pattern.as_bytes();
    let value = value.as_bytes();
    while value_index < value.len() {
        if pattern_index < pattern.len()
            && (pattern[pattern_index] == b'?' || pattern[pattern_index] == value[value_index])
        {
            pattern_index += 1;
            value_index += 1;
        } else if pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
            star_index = Some(pattern_index);
            star_value_index = value_index;
            pattern_index += 1;
        } else if let Some(star) = star_index {
            star_value_index += 1;
            value_index = star_value_index;
            pattern_index = star + 1;
        } else {
            return false;
        }
    }
    while pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
        pattern_index += 1;
    }
    pattern_index == pattern.len()
}

impl ScanEventSink for Sender<ScanEvent> {
    fn send_event(&self, event: ScanEvent) -> Result<(), Box<SendError<ScanEvent>>> {
        self.send(event).map_err(Box::new)
    }
}

impl ScanEventSink for SyncSender<ScanEvent> {
    fn send_event(&self, event: ScanEvent) -> Result<(), Box<SendError<ScanEvent>>> {
        self.send(event).map_err(Box::new)
    }
}

use crate::VolumeStats;

#[derive(Clone, Debug)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            paused: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }

    pub fn pause(&self) {
        if !self.is_cancelled() {
            self.paused.store(true, Ordering::Release);
        }
    }

    pub fn resume(&self) {
        self.paused.store(false, Ordering::Release);
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Acquire)
    }

    pub fn wait_if_paused(&self) {
        while self.is_paused() && !self.is_cancelled() {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ScanOptions {
    pub max_depth: usize,
    pub max_entries: u64,
    pub collect_lock_state: bool,
}

impl Default for ScanOptions {
    fn default() -> Self {
        Self {
            max_depth: 512,
            max_entries: 10_000_000,
            collect_lock_state: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlatformErrorKind {
    AccessDenied,
    EventReceiverClosed,
    InvalidRoot,
    Io,
    Locked,
    PathTooLong,
    PhysicalTargetOutsideRoot,
    VolumeOffline,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlatformError {
    kind: PlatformErrorKind,
    path: Option<NormalizedPath>,
    code: Option<u32>,
    message: String,
}

impl PlatformError {
    pub fn kind(&self) -> PlatformErrorKind {
        self.kind
    }

    pub fn path(&self) -> Option<&NormalizedPath> {
        self.path.as_ref()
    }

    pub fn code(&self) -> Option<u32> {
        self.code
    }

    pub(crate) fn from_last_os_error(path: Option<NormalizedPath>) -> Self {
        Self::from_io(path, std::io::Error::last_os_error())
    }

    pub(crate) fn from_io(path: Option<NormalizedPath>, error: std::io::Error) -> Self {
        let code = error.raw_os_error().map(|value| value as u32);
        Self {
            kind: code.map(classify_error).unwrap_or(PlatformErrorKind::Io),
            path,
            code,
            message: error.to_string(),
        }
    }

    pub(crate) fn new(
        kind: PlatformErrorKind,
        path: Option<NormalizedPath>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            path,
            code: None,
            message: message.into(),
        }
    }
}

impl fmt::Display for PlatformError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.message)
    }
}

impl Error for PlatformError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScanIssueKind {
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
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScanIssue {
    kind: ScanIssueKind,
    path: Option<NormalizedPath>,
    code: Option<u32>,
    message: String,
}

impl ScanIssue {
    pub fn kind(&self) -> ScanIssueKind {
        self.kind
    }

    pub fn path(&self) -> Option<&NormalizedPath> {
        self.path.as_ref()
    }

    pub fn code(&self) -> Option<u32> {
        self.code
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub(crate) fn new(
        kind: ScanIssueKind,
        path: Option<NormalizedPath>,
        code: Option<u32>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            path,
            code,
            message: message.into(),
        }
    }

    pub(crate) fn from_io(path: NormalizedPath, error: std::io::Error) -> Self {
        let code = error.raw_os_error().map(|value| value as u32);
        Self::new(
            code.map(classify_issue)
                .unwrap_or(ScanIssueKind::MetadataUnavailable),
            Some(path),
            code,
            error.to_string(),
        )
    }
}

#[derive(Clone, Debug)]
pub struct ScanSummary {
    status: ScanStatus,
    directory_count: u64,
    file_count: u64,
    logical_size: u64,
    allocated_size: u64,
    issues: Vec<ScanIssue>,
    omitted_issues: u64,
    volume: VolumeStats,
}

impl ScanSummary {
    pub fn status(&self) -> ScanStatus {
        self.status
    }

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
        self.allocated_size
    }

    pub fn issues(&self) -> &[ScanIssue] {
        &self.issues
    }

    pub fn omitted_issues(&self) -> u64 {
        self.omitted_issues
    }

    pub fn volume(&self) -> VolumeStats {
        self.volume
    }
}

pub trait FileScanner {
    fn scan<S: ScanEventSink>(
        &self,
        root: NormalizedPath,
        options: ScanOptions,
        sender: S,
        cancel: CancellationToken,
    ) -> Result<ScanSummary, PlatformError>;
}

fn classify_error(code: u32) -> PlatformErrorKind {
    match code {
        ERROR_ACCESS_DENIED => PlatformErrorKind::AccessDenied,
        ERROR_FILENAME_EXCED_RANGE => PlatformErrorKind::PathTooLong,
        ERROR_NOT_READY => PlatformErrorKind::VolumeOffline,
        ERROR_SHARING_VIOLATION => PlatformErrorKind::Locked,
        _ => PlatformErrorKind::Io,
    }
}

fn classify_issue(code: u32) -> ScanIssueKind {
    match code {
        ERROR_ACCESS_DENIED => ScanIssueKind::AccessDenied,
        ERROR_FILENAME_EXCED_RANGE => ScanIssueKind::PathTooLong,
        ERROR_NOT_READY => ScanIssueKind::VolumeOffline,
        ERROR_SHARING_VIOLATION => ScanIssueKind::Locked,
        ERROR_CANT_RESOLVE_FILENAME => ScanIssueKind::Cycle,
        _ => ScanIssueKind::MetadataUnavailable,
    }
}

pub(crate) fn receiver_closed() -> PlatformError {
    PlatformError::new(
        PlatformErrorKind::EventReceiverClosed,
        None,
        "scan event receiver closed",
    )
}

#[cfg(test)]
mod tests {
    use windows_sys::Win32::Foundation::{
        ERROR_ACCESS_DENIED, ERROR_CANT_RESOLVE_FILENAME, ERROR_FILENAME_EXCED_RANGE,
        ERROR_NOT_READY, ERROR_PATH_NOT_FOUND,
    };

    use super::{ScanIssueKind, classify_issue};

    #[test]
    fn statistics_roots_include_literal_files() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let pattern = file.path().display().to_string();

        let roots = super::statistics_roots_for_pattern(&pattern);

        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].display(), pattern);
    }

    #[test]
    fn native_errors_map_to_structured_scan_issues() {
        assert_eq!(
            classify_issue(ERROR_ACCESS_DENIED),
            ScanIssueKind::AccessDenied
        );
        assert_eq!(
            classify_issue(ERROR_FILENAME_EXCED_RANGE),
            ScanIssueKind::PathTooLong
        );
        assert_eq!(
            classify_issue(ERROR_NOT_READY),
            ScanIssueKind::VolumeOffline
        );
        assert_eq!(
            classify_issue(ERROR_CANT_RESOLVE_FILENAME),
            ScanIssueKind::Cycle
        );
        assert_eq!(
            classify_issue(ERROR_PATH_NOT_FOUND),
            ScanIssueKind::MetadataUnavailable
        );
    }
}
