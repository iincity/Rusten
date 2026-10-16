use std::{collections::HashSet, ffi::OsStr, os::windows::ffi::OsStrExt};

use domain::{
    DirectoryId, DirectoryNode, FileFacts, FileIdentity, FileRecord, LockState, NormalizedPath,
    ReparseKind, ScanEvent, ScanIndex, ScanStatus, ScanWarning, ScanWarningKind, normalize_path,
};
use windows_sys::Win32::{
    Foundation::{
        ERROR_FILE_NOT_FOUND, ERROR_NO_MORE_FILES, GetLastError, HANDLE, INVALID_HANDLE_VALUE,
    },
    Storage::FileSystem::{
        FILE_ATTRIBUTE_DIRECTORY, FIND_FIRST_EX_LARGE_FETCH, FindClose, FindExInfoBasic,
        FindExSearchNameMatch, FindFirstFileExW, FindNextFileW, WIN32_FIND_DATAW,
    },
};

use crate::query_volume_stats;

use super::{
    CancellationToken, FileScanner, PlatformError, PlatformErrorKind, ReparsePointKind,
    ScanEventSink, ScanIssue, ScanIssueKind, ScanOptions, ScanSummary,
    identity::{paths_equal_ordinal, physical_path_is_within},
    lock::LockStatus,
    metadata::{read_metadata, read_reparse_metadata, read_statistics_metadata},
    receiver_closed, reparse,
};

#[derive(Clone, Copy, Debug, Default)]
pub struct WindowsFileScanner;

impl FileScanner for WindowsFileScanner {
    fn scan<S: ScanEventSink>(
        &self,
        root: NormalizedPath,
        options: ScanOptions,
        sender: S,
        cancel: CancellationToken,
    ) -> Result<ScanSummary, PlatformError> {
        if options.max_depth == 0 || options.max_entries == 0 {
            return Err(PlatformError::new(
                PlatformErrorKind::InvalidRoot,
                Some(root),
                "scan limits must be greater than zero",
            ));
        }
        let mut ancestor = root.display()[..3].to_owned();
        for component in root.display()[3..]
            .split('\\')
            .filter(|part| !part.is_empty())
        {
            ancestor.push_str(component);
            let path = normalize_path(&ancestor).expect("scan root components are normalized");
            if reparse::reparse_point_kind(&path)?.is_some() {
                return Err(PlatformError::new(
                    PlatformErrorKind::InvalidRoot,
                    Some(root),
                    "scan root passes through a reparse point",
                ));
            }
            ancestor.push('\\');
        }
        let volume = query_volume_stats(&root)?;
        let root_metadata = read_metadata(&root)?;
        if !root_metadata.is_directory() {
            return Err(PlatformError::new(
                PlatformErrorKind::InvalidRoot,
                Some(root),
                "scan root is not a directory",
            ));
        }
        let mut state = ScanState {
            directory_index: ScanIndex::new_with_path_matcher(root.clone(), paths_equal_ordinal),
            root_physical_path: root_metadata.physical_path().to_owned(),
            root_volume: root_metadata.identity().volume_serial,
            visited_directories: HashSet::from([root_metadata.identity()]),
            allocated_files: HashSet::new(),
            sender,
            cancel,
            options,
            entries_seen: 0,
            directory_count: 1,
            file_count: 0,
            logical_size: 0,
            allocated_size: 0,
            issues: Vec::new(),
            omitted_issues: 0,
            stopped_by_limit: false,
        };

        state.cancel.wait_if_paused();
        if !state.cancel.is_cancelled() {
            state.walk(&root, state.directory_index.root_directory_id(), 0)?;
        }
        let status = if state.cancel.is_cancelled() {
            ScanStatus::Canceled
        } else if !state.issues.is_empty() || state.stopped_by_limit {
            ScanStatus::Partial
        } else {
            ScanStatus::Complete
        };
        state.send(match status {
            ScanStatus::Complete => ScanEvent::Completed,
            ScanStatus::Canceled => ScanEvent::Canceled,
            ScanStatus::Partial => ScanEvent::Partial,
            ScanStatus::InProgress => unreachable!(),
        })?;

        Ok(ScanSummary {
            status,
            directory_count: state.directory_count,
            file_count: state.file_count,
            logical_size: state.logical_size,
            allocated_size: state.allocated_size,
            issues: state.issues,
            omitted_issues: state.omitted_issues,
            volume,
        })
    }
}

struct ScanState<S: ScanEventSink> {
    directory_index: ScanIndex,
    root_physical_path: String,
    root_volume: u64,
    visited_directories: HashSet<FileIdentity>,
    allocated_files: HashSet<FileIdentity>,
    sender: S,
    cancel: CancellationToken,
    options: ScanOptions,
    entries_seen: u64,
    directory_count: u64,
    file_count: u64,
    logical_size: u64,
    allocated_size: u64,
    issues: Vec<ScanIssue>,
    omitted_issues: u64,
    stopped_by_limit: bool,
}

impl<S: ScanEventSink> ScanState<S> {
    fn walk(
        &mut self,
        directory: &NormalizedPath,
        directory_id: DirectoryId,
        depth: usize,
    ) -> Result<(), PlatformError> {
        let mut entries = match DirectoryEnumerator::open(directory) {
            Ok(entries) => entries,
            Err(error) => {
                self.warn(ScanIssue::from_io(directory.clone(), error))?;
                return Ok(());
            }
        };
        while !self.cancel.is_cancelled() && !self.stopped_by_limit {
            self.cancel.wait_if_paused();
            if self.cancel.is_cancelled() {
                break;
            }
            let entry = match entries.next() {
                Some(Ok(entry)) => entry,
                Some(Err(error)) => {
                    self.warn(ScanIssue::from_io(directory.clone(), error))?;
                    break;
                }
                None => break,
            };
            self.entries_seen = self.entries_seen.saturating_add(1);
            if self.entries_seen > self.options.max_entries {
                self.stopped_by_limit = true;
                self.warn(ScanIssue::new(
                    ScanIssueKind::TraversalLimit,
                    Some(directory.clone()),
                    None,
                    "scan entry limit reached",
                ))?;
                break;
            }
            let child = match child_path(directory, &entry.name) {
                Ok(path) => path,
                Err(message) => {
                    self.warn(ScanIssue::new(
                        ScanIssueKind::PathTooLong,
                        Some(directory.clone()),
                        None,
                        message,
                    ))?;
                    continue;
                }
            };
            let reparse_kind = reparse::classify(entry.attributes, entry.reparse_tag);
            let metadata = match if reparse_kind.is_some() {
                read_reparse_metadata(&child)
            } else if !self.options.collect_lock_state {
                read_statistics_metadata(&child)
            } else {
                read_metadata(&child)
            } {
                Ok(metadata) => metadata,
                Err(error) => {
                    let issue = ScanIssue::new(
                        match error.kind() {
                            PlatformErrorKind::AccessDenied => ScanIssueKind::AccessDenied,
                            PlatformErrorKind::Locked => ScanIssueKind::Locked,
                            PlatformErrorKind::PathTooLong => ScanIssueKind::PathTooLong,
                            PlatformErrorKind::VolumeOffline => ScanIssueKind::VolumeOffline,
                            _ => ScanIssueKind::MetadataUnavailable,
                        },
                        Some(child),
                        error.code(),
                        error.to_string(),
                    );
                    self.warn(issue)?;
                    continue;
                }
            };
            if let Some(kind) = reparse_kind {
                let target_path = if matches!(
                    kind,
                    ReparsePointKind::SymbolicLink | ReparsePointKind::MountPoint
                ) {
                    std::fs::read_link(child.extended())
                        .ok()
                        .map(|path| path.to_string_lossy().into_owned())
                } else {
                    None
                };
                let domain_kind = match kind {
                    ReparsePointKind::SymbolicLink => ReparseKind::SymbolicLink,
                    ReparsePointKind::MountPoint => {
                        reparse::mount_target_kind(target_path.as_deref())
                    }
                    ReparsePointKind::Cloud => ReparseKind::Cloud,
                    ReparsePointKind::Other(tag) => ReparseKind::Other(tag),
                };
                let facts = self.facts(
                    &metadata,
                    LockState::Unknown,
                    Some(entry.reparse_tag),
                    Some(domain_kind),
                    target_path,
                );
                self.emit_reparse(child.clone(), directory_id, &metadata, facts)?;
                if kind != ReparsePointKind::Cloud
                    || entry.attributes & FILE_ATTRIBUTE_DIRECTORY != 0
                {
                    self.warn(ScanIssue::new(
                        ScanIssueKind::ReparsePointSkipped,
                        Some(child),
                        None,
                        format!("reparse target not traversed: {kind:?}"),
                    ))?;
                }
                continue;
            }
            if metadata.identity().volume_serial != self.root_volume {
                self.warn(ScanIssue::new(
                    ScanIssueKind::CrossVolumeSkipped,
                    Some(child),
                    None,
                    "cross-volume target skipped",
                ))?;
                continue;
            }
            if !physical_path_is_within(&self.root_physical_path, metadata.physical_path()) {
                self.warn(ScanIssue::new(
                    ScanIssueKind::PhysicalTargetOutsideRoot,
                    Some(child),
                    None,
                    "physical target is outside the scan root",
                ))?;
                continue;
            }

            if entry.attributes & FILE_ATTRIBUTE_DIRECTORY != 0 {
                if depth >= self.options.max_depth {
                    self.warn(ScanIssue::new(
                        ScanIssueKind::TraversalLimit,
                        Some(child),
                        None,
                        "scan depth limit reached",
                    ))?;
                    continue;
                }
                if !self.visited_directories.insert(metadata.identity()) {
                    self.warn(ScanIssue::new(
                        ScanIssueKind::Cycle,
                        Some(child),
                        None,
                        "directory identity already visited",
                    ))?;
                    continue;
                }
                let node = DirectoryNode::new(child.clone(), directory_id);
                self.send(ScanEvent::Directory(node.clone()))?;
                let child_id = self
                    .directory_index
                    .add_directory(node)
                    .expect("directory index remains in progress during scanning");
                self.directory_count = self.directory_count.saturating_add(1);
                self.walk(&child, child_id, depth + 1)?;
            } else {
                let lock = if self.options.collect_lock_state {
                    super::lock::lock_status(&child)
                } else {
                    LockStatus::Available
                };
                let state = match lock {
                    LockStatus::Available => LockState::Available,
                    LockStatus::Locked => LockState::Locked,
                    LockStatus::AccessDenied => LockState::AccessDenied,
                    LockStatus::Unknown(_) => LockState::Unknown,
                };
                let facts = self.facts(&metadata, state, None, None, None);
                self.emit_file(child.clone(), directory_id, &metadata, facts)?;
                if lock == LockStatus::Locked {
                    self.warn(ScanIssue::new(
                        ScanIssueKind::Locked,
                        Some(child),
                        Some(windows_sys::Win32::Foundation::ERROR_SHARING_VIOLATION),
                        "file is locked",
                    ))?;
                }
            }
        }
        Ok(())
    }

    fn facts(
        &self,
        metadata: &super::FileMetadata,
        lock_state: LockState,
        reparse_tag: Option<u32>,
        reparse_kind: Option<ReparseKind>,
        target_path: Option<String>,
    ) -> FileFacts {
        FileFacts {
            creation_time: metadata.creation_time(),
            last_access_time: metadata.last_access_time(),
            last_write_time: metadata.last_write_time(),
            attributes: metadata.attributes(),
            link_count: metadata.link_count(),
            owner: None,
            lock_state,
            reparse_tag,
            reparse_kind,
            physical_path: Some(metadata.physical_path().to_owned()),
            target_path,
            target_volume_serial: None,
        }
    }

    fn emit_file(
        &mut self,
        path: NormalizedPath,
        parent: DirectoryId,
        metadata: &super::FileMetadata,
        facts: FileFacts,
    ) -> Result<(), PlatformError> {
        let record = self.record(path, parent, metadata, facts);
        self.send(ScanEvent::File(record))?;
        self.file_count = self.file_count.saturating_add(1);
        self.logical_size = self.logical_size.saturating_add(metadata.logical_size());
        if self.allocated_files.insert(metadata.identity()) {
            self.allocated_size = self
                .allocated_size
                .saturating_add(metadata.allocated_size());
        }
        Ok(())
    }

    fn emit_reparse(
        &mut self,
        path: NormalizedPath,
        parent: DirectoryId,
        metadata: &super::FileMetadata,
        facts: FileFacts,
    ) -> Result<(), PlatformError> {
        self.send(ScanEvent::ReparsePoint(
            self.record(path, parent, metadata, facts),
        ))
    }

    fn record(
        &self,
        path: NormalizedPath,
        parent: DirectoryId,
        metadata: &super::FileMetadata,
        facts: FileFacts,
    ) -> FileRecord {
        FileRecord::new(
            path,
            parent,
            metadata.identity(),
            metadata.logical_size(),
            metadata.allocated_size(),
        )
        .with_facts(facts)
    }

    fn warn(&mut self, issue: ScanIssue) -> Result<(), PlatformError> {
        let kind = match issue.kind() {
            ScanIssueKind::AccessDenied => ScanWarningKind::AccessDenied,
            ScanIssueKind::CrossVolumeSkipped => ScanWarningKind::CrossVolumeSkipped,
            ScanIssueKind::Cycle => ScanWarningKind::Cycle,
            ScanIssueKind::Locked => ScanWarningKind::Locked,
            ScanIssueKind::MetadataUnavailable => ScanWarningKind::MetadataUnavailable,
            ScanIssueKind::PathTooLong => ScanWarningKind::PathTooLong,
            ScanIssueKind::PhysicalTargetOutsideRoot => ScanWarningKind::PhysicalTargetOutsideRoot,
            ScanIssueKind::ReparsePointSkipped => ScanWarningKind::ReparsePointSkipped,
            ScanIssueKind::TraversalLimit => ScanWarningKind::TraversalLimit,
            ScanIssueKind::VolumeOffline => ScanWarningKind::VolumeOffline,
        };
        self.send(ScanEvent::Warning(ScanWarning::structured(
            issue.path().cloned(),
            kind,
            issue.code(),
            issue.message(),
        )))?;
        if self.issues.len() < ScanIndex::WARNINGS_CAPACITY {
            self.issues.push(issue);
        } else {
            self.omitted_issues = self.omitted_issues.saturating_add(1);
        }
        Ok(())
    }

    fn send(&self, event: ScanEvent) -> Result<(), PlatformError> {
        self.sender.send_event(event).map_err(|_| receiver_closed())
    }
}

struct FindEntry {
    name: String,
    attributes: u32,
    reparse_tag: u32,
}

struct DirectoryEnumerator {
    handle: Option<HANDLE>,
    data: WIN32_FIND_DATAW,
    first: bool,
}

impl DirectoryEnumerator {
    fn open(path: &NormalizedPath) -> std::io::Result<Self> {
        let pattern = format!(r"{}\*", path.extended().trim_end_matches('\\'));
        let wide = wide_null(&pattern);
        let mut data = WIN32_FIND_DATAW::default();
        let handle = unsafe {
            FindFirstFileExW(
                wide.as_ptr(),
                FindExInfoBasic,
                (&mut data as *mut WIN32_FIND_DATAW).cast(),
                FindExSearchNameMatch,
                std::ptr::null(),
                FIND_FIRST_EX_LARGE_FETCH,
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() == Some(ERROR_FILE_NOT_FOUND as i32) {
                return Ok(Self {
                    handle: None,
                    data,
                    first: false,
                });
            }
            return Err(error);
        }
        Ok(Self {
            handle: Some(handle),
            data,
            first: true,
        })
    }
}

impl Iterator for DirectoryEnumerator {
    type Item = std::io::Result<FindEntry>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let handle = self.handle?;
            if self.first {
                self.first = false;
            } else if unsafe { FindNextFileW(handle, &mut self.data) } == 0 {
                let code = unsafe { GetLastError() };
                if code == ERROR_NO_MORE_FILES {
                    self.handle.take();
                    unsafe {
                        FindClose(handle);
                    }
                    return None;
                }
                return Some(Err(std::io::Error::from_raw_os_error(code as i32)));
            }
            let length = self
                .data
                .cFileName
                .iter()
                .position(|value| *value == 0)
                .unwrap_or(self.data.cFileName.len());
            let name = match String::from_utf16(&self.data.cFileName[..length]) {
                Ok(name) => name,
                Err(error) => {
                    return Some(Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        error,
                    )));
                }
            };
            if name == "." || name == ".." {
                continue;
            }
            return Some(Ok(FindEntry {
                name,
                attributes: self.data.dwFileAttributes,
                reparse_tag: self.data.dwReserved0,
            }));
        }
    }
}

impl Drop for DirectoryEnumerator {
    fn drop(&mut self) {
        if let Some(handle) = self.handle.take() {
            unsafe {
                FindClose(handle);
            }
        }
    }
}

fn child_path(parent: &NormalizedPath, name: &str) -> Result<NormalizedPath, String> {
    let separator = if parent.display().ends_with('\\') {
        ""
    } else {
        r"\"
    };
    normalize_path(&format!("{}{separator}{name}", parent.display()))
        .map_err(|error| error.to_string())
}

fn wide_null(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain(Some(0)).collect()
}
