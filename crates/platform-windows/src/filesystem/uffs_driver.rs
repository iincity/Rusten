#[cfg(feature = "uffs")]
use std::collections::{HashMap, HashSet};

#[cfg(feature = "uffs")]
use crate::VolumeStats;
use domain::NormalizedPath;
#[cfg(feature = "uffs")]
use domain::{
    DirectoryId, DirectoryNode, FileFacts, FileIdentity, FileRecord, LockState, ReparseKind,
    ScanEngine, ScanEvent, ScanIndex, ScanPhase, ScanStatus, ScanWarning, ScanWarningKind,
    normalize_path,
};

#[cfg(feature = "uffs")]
use super::{
    CancellationToken, FileScanner, PlatformError, PlatformErrorKind, ScanEventSink, ScanIssue,
    ScanIssueKind, ScanOptions, ScanSummary, receiver_closed,
};
#[cfg(not(feature = "uffs"))]
use super::{
    CancellationToken, FileScanner, PlatformError, ScanEventSink, ScanOptions, ScanSummary,
    receiver_closed,
};

#[derive(Clone, Copy, Debug, Default)]
pub struct FastFileScanner;

#[cfg(feature = "uffs")]
const WIN32_QUICK_SCAN_LIMIT: u64 = 10_000;

impl FileScanner for FastFileScanner {
    fn scan<S: ScanEventSink>(
        &self,
        root: NormalizedPath,
        options: ScanOptions,
        sender: S,
        cancel: CancellationToken,
    ) -> Result<ScanSummary, PlatformError> {
        #[cfg(feature = "uffs")]
        if options.max_entries <= WIN32_QUICK_SCAN_LIMIT {
            sender
                .send_event(ScanEvent::ScannerStatus {
                    engine: ScanEngine::Win32,
                    phase: ScanPhase::Enumerating,
                    total_entries: None,
                    processed_entries: None,
                })
                .map_err(|_| receiver_closed())?;
            return super::WindowsFileScanner.scan(root, options, sender, cancel);
        }

        #[cfg(feature = "uffs")]
        if is_c_drive_root(&root) {
            sender
                .send_event(ScanEvent::ScannerStatus {
                    engine: ScanEngine::Uffs,
                    phase: ScanPhase::Indexing,
                    total_entries: None,
                    processed_entries: None,
                })
                .map_err(|_| receiver_closed())?;
            match UffsFileScanner::try_new(&root) {
                Ok(scanner) => return scanner.scan(root, options, sender, cancel),
                Err(_) => {
                    sender
                        .send_event(ScanEvent::ScannerStatus {
                            engine: ScanEngine::Win32,
                            phase: ScanPhase::Fallback,
                            total_entries: None,
                            processed_entries: None,
                        })
                        .map_err(|_| receiver_closed())?;
                }
            }
        }

        sender
            .send_event(ScanEvent::ScannerStatus {
                engine: ScanEngine::Win32,
                phase: ScanPhase::Enumerating,
                total_entries: None,
                processed_entries: None,
            })
            .map_err(|_| receiver_closed())?;
        super::WindowsFileScanner.scan(root, options, sender, cancel)
    }
}

#[cfg(feature = "uffs")]
pub struct UffsFileScanner {
    index: uffs_mft::MftIndex,
}

#[cfg(feature = "uffs")]
impl UffsFileScanner {
    fn try_new(root: &NormalizedPath) -> Result<Self, PlatformError> {
        if !is_c_drive_root(root) {
            return Err(PlatformError::new(
                PlatformErrorKind::InvalidRoot,
                Some(root.clone()),
                "UFFS driver supports only the C drive root",
            ));
        }

        let drive = uffs_mft::platform::DriveLetter::C;
        let reader = uffs_mft::MftReader::open(drive)
            .map_err(|error| uffs_error(root, error))?
            .with_use_bitmap(true)
            .with_merge_extensions(false)
            .with_expand_links(true);
        let index = reader
            .read_index_cached_sync(uffs_mft::INDEX_TTL_SECONDS)
            .map_err(|error| uffs_error(root, error))?;
        Ok(Self { index })
    }
}

#[cfg(feature = "uffs")]
impl FileScanner for UffsFileScanner {
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

        let volume = super::super::query_volume_stats(&root)?;
        let root_metadata = super::read_metadata(&root)?;
        sender
            .send_event(ScanEvent::ScannerStatus {
                engine: ScanEngine::Uffs,
                phase: ScanPhase::Preparing,
                total_entries: Some((self.index.records.len() as u64).saturating_mul(2)),
                processed_entries: Some(0),
            })
            .map_err(|_| receiver_closed())?;
        let path_resolver = uffs_mft::index::PathResolver::build(&self.index, false);

        // Limited scans are used by the UI and CLI previews. They must not
        // pay the full-drive directory path cache cost before yielding rows.
        if options.max_entries < self.index.records.len() as u64 {
            return self.scan_limited(
                root,
                options,
                sender,
                cancel,
                volume,
                root_metadata.identity().volume_serial,
                &path_resolver,
                &[],
            );
        }

        let mut progress_send_failed = false;
        let dir_cache = path_resolver.pre_cache_directory_paths_with_progress(
            &self.index,
            |processed, total| {
                if processed == total || processed % 8192 == 0 {
                    progress_send_failed |= sender
                        .send_event(ScanEvent::ScannerStatus {
                            engine: ScanEngine::Uffs,
                            phase: ScanPhase::Preparing,
                            total_entries: Some((total as u64).saturating_mul(2)),
                            processed_entries: Some(processed as u64),
                        })
                        .is_err();
                }
            },
        );
        if progress_send_failed {
            return Err(receiver_closed());
        }

        let mut entries = Vec::new();
        let mut issues = Vec::new();
        let mut omitted_issues = 0;

        let record_count = self.index.records.len();
        for (record_index, record) in self.index.records.iter().enumerate() {
            if cancel.is_cancelled() {
                break;
            }
            cancel.wait_if_paused();
            if record_index + 1 == record_count || (record_index + 1) % 8192 == 0 {
                sender
                    .send_event(ScanEvent::ScannerStatus {
                        engine: ScanEngine::Uffs,
                        phase: ScanPhase::Preparing,
                        total_entries: Some((record_count as u64).saturating_mul(2)),
                        processed_entries: Some(record_count as u64 + record_index as u64 + 1),
                    })
                    .map_err(|_| receiver_closed())?;
            }
            if record.is_deleted() || record.is_corrupt() || record.is_extension() {
                continue;
            }
            if !path_resolver.is_valid_idx(record_index) {
                continue;
            }
            let path = path_resolver.materialize_path_cached(&self.index, record_index, &dir_cache);
            let path = match normalize_path(&path) {
                Ok(path) if path_is_within(&root, &path) => path,
                Ok(path) => {
                    add_issue(
                        &mut issues,
                        &mut omitted_issues,
                        ScanIssue::new(
                            ScanIssueKind::PhysicalTargetOutsideRoot,
                            Some(path),
                            None,
                            "UFFS path is outside the scan root",
                        ),
                    );
                    continue;
                }
                Err(_) => {
                    add_issue(
                        &mut issues,
                        &mut omitted_issues,
                        ScanIssue::new(
                            ScanIssueKind::PathTooLong,
                            None,
                            None,
                            "UFFS returned a path Rusten could not normalize",
                        ),
                    );
                    continue;
                }
            };
            if path == root {
                continue;
            }
            entries.push(UffsEntry {
                record_index,
                path,
                directory: record.is_directory(),
                reparse_tag: (record.reparse_tag != 0).then_some(record.reparse_tag),
            });
        }

        let max_entry_depth = entries
            .iter()
            .map(|entry| path_depth(&entry.path))
            .max()
            .unwrap_or(0);
        let mut depth_buckets: Vec<(Vec<UffsEntry>, Vec<UffsEntry>)> = (0..=max_entry_depth)
            .map(|_| (Vec::new(), Vec::new()))
            .collect();
        for entry in entries {
            let bucket = &mut depth_buckets[path_depth(&entry.path)];
            if entry.directory {
                bucket.0.push(entry);
            } else {
                bucket.1.push(entry);
            }
        }
        let entry_capacity = depth_buckets
            .iter()
            .map(|(directories, files)| directories.len() + files.len())
            .sum();
        let mut ordered_entries = Vec::with_capacity(entry_capacity);
        for (directories, files) in depth_buckets {
            ordered_entries.extend(directories);
            ordered_entries.extend(files);
        }

        sender
            .send_event(ScanEvent::ScannerStatus {
                engine: ScanEngine::Uffs,
                phase: ScanPhase::Enumerating,
                total_entries: Some(ordered_entries.len() as u64),
                processed_entries: None,
            })
            .map_err(|_| receiver_closed())?;

        let mut directory_index =
            ScanIndex::new_with_path_matcher(root.clone(), super::paths_equal_ordinal);
        let mut directory_ids = HashMap::new();
        directory_ids.insert(
            root.display().to_ascii_lowercase(),
            directory_index.root_directory_id(),
        );
        let mut directory_identities = HashSet::new();
        let mut directory_count = 1_u64;
        let mut file_count = 0_u64;
        let mut logical_size = 0_u64;
        let mut allocated_size = 0_u64;
        let mut entries_seen = 0_u64;
        let mut stopped_by_limit = false;

        for entry in ordered_entries {
            if cancel.is_cancelled() {
                break;
            }
            cancel.wait_if_paused();
            if cancel.is_cancelled() {
                break;
            }
            if path_depth(&entry.path) > options.max_depth {
                stopped_by_limit = true;
                add_issue(
                    &mut issues,
                    &mut omitted_issues,
                    ScanIssue::new(
                        ScanIssueKind::TraversalLimit,
                        Some(entry.path),
                        None,
                        "scan depth limit reached",
                    ),
                );
                continue;
            }
            if entries_seen >= options.max_entries {
                stopped_by_limit = true;
                add_issue(
                    &mut issues,
                    &mut omitted_issues,
                    ScanIssue::new(
                        ScanIssueKind::TraversalLimit,
                        Some(entry.path),
                        None,
                        "scan entry limit reached",
                    ),
                );
                break;
            }
            entries_seen = entries_seen.saturating_add(1);

            let Some(parent_path) = parent_path(&entry.path, &root) else {
                continue;
            };
            let Some(parent) = directory_ids
                .get(&parent_path.display().to_ascii_lowercase())
                .copied()
                .or_else(|| directory_index.directory_id(&parent_path))
            else {
                continue;
            };
            let source = &self.index.records[entry.record_index];
            let identity = uffs_identity(source, root_metadata.identity().volume_serial);
            let file_logical_size = source.first_stream.size.length;
            let file_allocated_size = source.first_stream.size.allocated;
            let record = uffs_file_record(entry.path.clone(), parent, identity, source);

            if entry.reparse_tag.is_some() {
                if entry.directory && directory_identities.insert(identity) {
                    let directory_key = entry.path.display().to_ascii_lowercase();
                    if !directory_ids.contains_key(&directory_key) {
                        let node = DirectoryNode::new(entry.path.clone(), parent);
                        sender
                            .send_event(ScanEvent::Directory(node.clone()))
                            .map_err(|_| receiver_closed())?;
                        if let Some(id) = directory_index.add_directory(node) {
                            directory_ids.insert(directory_key, id);
                            directory_count = directory_count.saturating_add(1);
                        }
                    }
                }
                sender
                    .send_event(ScanEvent::ReparsePoint(record))
                    .map_err(|_| receiver_closed())?;
                continue;
            }
            if entry.directory {
                if !directory_identities.insert(identity) {
                    continue;
                }
                let directory_key = entry.path.display().to_ascii_lowercase();
                if directory_ids.contains_key(&directory_key) {
                    continue;
                }
                let node = DirectoryNode::new(entry.path, parent);
                sender
                    .send_event(ScanEvent::Directory(node.clone()))
                    .map_err(|_| receiver_closed())?;
                if let Some(id) = directory_index.add_directory(node) {
                    directory_ids.insert(directory_key, id);
                }
                directory_count = directory_count.saturating_add(1);
            } else {
                sender
                    .send_event(ScanEvent::File(record))
                    .map_err(|_| receiver_closed())?;
                file_count = file_count.saturating_add(1);
                logical_size = logical_size.saturating_add(file_logical_size);
                allocated_size = allocated_size.saturating_add(file_allocated_size);
            }
        }

        let status = if cancel.is_cancelled() {
            ScanStatus::Canceled
        } else if stopped_by_limit || !issues.is_empty() || omitted_issues != 0 {
            ScanStatus::Partial
        } else {
            ScanStatus::Complete
        };
        let terminal = match status {
            ScanStatus::Complete => ScanEvent::Completed,
            ScanStatus::Partial => ScanEvent::Partial,
            ScanStatus::Canceled => ScanEvent::Canceled,
            ScanStatus::InProgress => unreachable!(),
        };
        sender
            .send_event(ScanEvent::ScannerStatus {
                engine: ScanEngine::Uffs,
                phase: ScanPhase::Finalizing,
                total_entries: Some(entries_seen),
                processed_entries: Some(entries_seen),
            })
            .map_err(|_| receiver_closed())?;
        for issue in &issues {
            sender
                .send_event(ScanEvent::Warning(ScanWarning::structured(
                    issue.path().cloned(),
                    warning_kind(issue.kind()),
                    issue.code(),
                    issue.message(),
                )))
                .map_err(|_| receiver_closed())?;
        }
        sender.send_event(terminal).map_err(|_| receiver_closed())?;

        Ok(ScanSummary {
            status,
            directory_count,
            file_count,
            logical_size,
            allocated_size,
            issues,
            omitted_issues,
            volume,
        })
    }
}

#[cfg(feature = "uffs")]
impl UffsFileScanner {
    #[expect(
        clippy::too_many_arguments,
        reason = "keeps the limited fast path next to the full scan without changing public scanner traits"
    )]
    fn scan_limited<S: ScanEventSink>(
        &self,
        root: NormalizedPath,
        options: ScanOptions,
        sender: S,
        cancel: CancellationToken,
        volume: VolumeStats,
        volume_serial: u64,
        path_resolver: &uffs_mft::index::PathResolver,
        dir_cache: &[String],
    ) -> Result<ScanSummary, PlatformError> {
        let mut directory_index =
            ScanIndex::new_with_path_matcher(root.clone(), super::paths_equal_ordinal);
        let mut directory_ids = HashMap::new();
        directory_ids.insert(
            root.display().to_ascii_lowercase(),
            directory_index.root_directory_id(),
        );
        let mut directory_identities = HashSet::new();
        let mut directory_count = 1_u64;
        let mut file_count = 0_u64;
        let mut logical_size = 0_u64;
        let mut allocated_size = 0_u64;
        let mut entries_seen = 0_u64;
        let mut issues = Vec::new();
        let mut omitted_issues = 0;
        let mut stopped_by_limit = false;

        sender
            .send_event(ScanEvent::ScannerStatus {
                engine: ScanEngine::Uffs,
                phase: ScanPhase::Enumerating,
                total_entries: Some(options.max_entries),
                processed_entries: Some(0),
            })
            .map_err(|_| receiver_closed())?;

        for (record_index, source) in self.index.records.iter().enumerate() {
            if cancel.is_cancelled() {
                break;
            }
            cancel.wait_if_paused();
            if cancel.is_cancelled() {
                break;
            }
            if entries_seen >= options.max_entries {
                stopped_by_limit = true;
                add_issue(
                    &mut issues,
                    &mut omitted_issues,
                    ScanIssue::new(
                        ScanIssueKind::TraversalLimit,
                        None,
                        None,
                        "scan entry limit reached",
                    ),
                );
                break;
            }
            if source.is_deleted() || source.is_corrupt() || source.is_extension() {
                continue;
            }
            if !path_resolver.is_valid_idx(record_index) {
                continue;
            }
            let path = match normalize_uffs_path(
                &root,
                path_resolver.materialize_path_cached(&self.index, record_index, dir_cache),
                &mut issues,
                &mut omitted_issues,
            ) {
                Some(path) => path,
                None => continue,
            };
            if path == root {
                continue;
            }
            if path_depth(&path) > options.max_depth {
                stopped_by_limit = true;
                add_issue(
                    &mut issues,
                    &mut omitted_issues,
                    ScanIssue::new(
                        ScanIssueKind::TraversalLimit,
                        Some(path),
                        None,
                        "scan depth limit reached",
                    ),
                );
                continue;
            }
            let parent = ensure_directory_path(
                &path,
                &root,
                &mut directory_ids,
                &mut directory_index,
                &mut directory_count,
                &sender,
            )?;
            entries_seen = entries_seen.saturating_add(1);

            let identity = uffs_identity(source, volume_serial);
            let record = uffs_file_record(path.clone(), parent, identity, source);
            if source.reparse_tag != 0 {
                if source.is_directory() && directory_identities.insert(identity) {
                    let directory_key = path.display().to_ascii_lowercase();
                    if !directory_ids.contains_key(&directory_key) {
                        let node = DirectoryNode::new(path.clone(), parent);
                        sender
                            .send_event(ScanEvent::Directory(node.clone()))
                            .map_err(|_| receiver_closed())?;
                        if let Some(id) = directory_index.add_directory(node) {
                            directory_ids.insert(directory_key, id);
                            directory_count = directory_count.saturating_add(1);
                        }
                    }
                }
                sender
                    .send_event(ScanEvent::ReparsePoint(record))
                    .map_err(|_| receiver_closed())?;
            } else if source.is_directory() {
                if !directory_identities.insert(identity) {
                    continue;
                }
                let directory_key = path.display().to_ascii_lowercase();
                if !directory_ids.contains_key(&directory_key) {
                    let node = DirectoryNode::new(path, parent);
                    sender
                        .send_event(ScanEvent::Directory(node.clone()))
                        .map_err(|_| receiver_closed())?;
                    if let Some(id) = directory_index.add_directory(node) {
                        directory_ids.insert(directory_key, id);
                    }
                    directory_count = directory_count.saturating_add(1);
                }
            } else {
                sender
                    .send_event(ScanEvent::File(record))
                    .map_err(|_| receiver_closed())?;
                file_count = file_count.saturating_add(1);
                logical_size = logical_size.saturating_add(source.first_stream.size.length);
                allocated_size = allocated_size.saturating_add(source.first_stream.size.allocated);
            }

            if entries_seen % 8192 == 0 || entries_seen == options.max_entries {
                sender
                    .send_event(ScanEvent::ScannerStatus {
                        engine: ScanEngine::Uffs,
                        phase: ScanPhase::Enumerating,
                        total_entries: Some(options.max_entries),
                        processed_entries: Some(entries_seen),
                    })
                    .map_err(|_| receiver_closed())?;
            }
        }

        let status = if cancel.is_cancelled() {
            ScanStatus::Canceled
        } else if stopped_by_limit || !issues.is_empty() || omitted_issues != 0 {
            ScanStatus::Partial
        } else {
            ScanStatus::Complete
        };
        let terminal = match status {
            ScanStatus::Complete => ScanEvent::Completed,
            ScanStatus::Partial => ScanEvent::Partial,
            ScanStatus::Canceled => ScanEvent::Canceled,
            ScanStatus::InProgress => unreachable!(),
        };
        sender
            .send_event(ScanEvent::ScannerStatus {
                engine: ScanEngine::Uffs,
                phase: ScanPhase::Finalizing,
                total_entries: Some(entries_seen),
                processed_entries: Some(entries_seen),
            })
            .map_err(|_| receiver_closed())?;
        for issue in &issues {
            sender
                .send_event(ScanEvent::Warning(ScanWarning::structured(
                    issue.path().cloned(),
                    warning_kind(issue.kind()),
                    issue.code(),
                    issue.message(),
                )))
                .map_err(|_| receiver_closed())?;
        }
        sender.send_event(terminal).map_err(|_| receiver_closed())?;

        Ok(ScanSummary {
            status,
            directory_count,
            file_count,
            logical_size,
            allocated_size,
            issues,
            omitted_issues,
            volume,
        })
    }
}

#[cfg(feature = "uffs")]
struct UffsEntry {
    record_index: usize,
    path: NormalizedPath,
    directory: bool,
    reparse_tag: Option<u32>,
}

#[cfg(feature = "uffs")]
fn uffs_error(root: &NormalizedPath, error: impl std::fmt::Display) -> PlatformError {
    let message = error.to_string();
    let lower = message.to_ascii_lowercase();
    let kind = if lower.contains("access")
        || lower.contains("privilege")
        || lower.contains("permission")
    {
        PlatformErrorKind::AccessDenied
    } else {
        PlatformErrorKind::Io
    };
    PlatformError::new(kind, Some(root.clone()), message)
}

#[cfg(feature = "uffs")]
fn add_issue(issues: &mut Vec<ScanIssue>, omitted: &mut u64, issue: ScanIssue) {
    if issues.len() < ScanIndex::WARNINGS_CAPACITY {
        issues.push(issue);
    } else {
        *omitted = omitted.saturating_add(1);
    }
}

#[cfg(feature = "uffs")]
fn normalize_uffs_path(
    root: &NormalizedPath,
    path: String,
    issues: &mut Vec<ScanIssue>,
    omitted: &mut u64,
) -> Option<NormalizedPath> {
    match normalize_path(&path) {
        Ok(path) if path_is_within(root, &path) => Some(path),
        Ok(path) => {
            add_issue(
                issues,
                omitted,
                ScanIssue::new(
                    ScanIssueKind::PhysicalTargetOutsideRoot,
                    Some(path),
                    None,
                    "UFFS path is outside the scan root",
                ),
            );
            None
        }
        Err(_) => {
            add_issue(
                issues,
                omitted,
                ScanIssue::new(
                    ScanIssueKind::PathTooLong,
                    None,
                    None,
                    "UFFS returned a path Rusten could not normalize",
                ),
            );
            None
        }
    }
}

#[cfg(feature = "uffs")]
fn ensure_directory_path<S: ScanEventSink>(
    path: &NormalizedPath,
    root: &NormalizedPath,
    directory_ids: &mut HashMap<String, DirectoryId>,
    directory_index: &mut ScanIndex,
    directory_count: &mut u64,
    sender: &S,
) -> Result<DirectoryId, PlatformError> {
    let Some(parent) = parent_path(path, root) else {
        return Ok(directory_index.root_directory_id());
    };
    ensure_directory_node(
        &parent,
        root,
        directory_ids,
        directory_index,
        directory_count,
        sender,
    )
}

#[cfg(feature = "uffs")]
fn ensure_directory_node<S: ScanEventSink>(
    path: &NormalizedPath,
    root: &NormalizedPath,
    directory_ids: &mut HashMap<String, DirectoryId>,
    directory_index: &mut ScanIndex,
    directory_count: &mut u64,
    sender: &S,
) -> Result<DirectoryId, PlatformError> {
    let key = path.display().to_ascii_lowercase();
    if let Some(id) = directory_ids.get(&key) {
        return Ok(*id);
    }
    if path == root {
        return Ok(directory_index.root_directory_id());
    }
    let Some(parent_path) = parent_path(path, root) else {
        return Ok(directory_index.root_directory_id());
    };
    let parent = ensure_directory_node(
        &parent_path,
        root,
        directory_ids,
        directory_index,
        directory_count,
        sender,
    )?;
    let node = DirectoryNode::new(path.clone(), parent);
    sender
        .send_event(ScanEvent::Directory(node.clone()))
        .map_err(|_| receiver_closed())?;
    let id = directory_index.add_directory(node).unwrap_or(parent);
    directory_ids.insert(key, id);
    *directory_count = directory_count.saturating_add(1);
    Ok(id)
}

#[cfg(feature = "uffs")]
fn uffs_identity(source: &uffs_mft::index::FileRecord, volume_serial: u64) -> FileIdentity {
    FileIdentity {
        volume_serial,
        file_index: ntfs_file_index(source.frs.raw(), source.sequence_number),
    }
}

#[cfg(feature = "uffs")]
fn uffs_file_record(
    path: NormalizedPath,
    parent: DirectoryId,
    identity: FileIdentity,
    source: &uffs_mft::index::FileRecord,
) -> FileRecord {
    FileRecord::new(
        path,
        parent,
        identity,
        source.first_stream.size.length,
        source.first_stream.size.allocated,
    )
    .with_facts(FileFacts {
        creation_time: non_negative_time(source.stdinfo.created),
        last_access_time: non_negative_time(source.stdinfo.accessed),
        last_write_time: non_negative_time(source.stdinfo.modified),
        attributes: source.stdinfo.to_attributes(),
        link_count: u32::from(source.name_count),
        lock_state: LockState::Unknown,
        reparse_tag: (source.reparse_tag != 0).then_some(source.reparse_tag),
        reparse_kind: (source.reparse_tag != 0).then_some(ReparseKind::Other(source.reparse_tag)),
        physical_path: None,
        target_path: None,
        target_volume_serial: None,
        owner: None,
    })
}

#[cfg(feature = "uffs")]
fn non_negative_time(value: i64) -> u64 {
    value.max(0) as u64
}

#[cfg(feature = "uffs")]
fn ntfs_file_index(record_number: u64, sequence_number: u16) -> u128 {
    ((sequence_number as u128) << 48) | (record_number as u128 & 0x0000_ffff_ffff_ffff)
}

#[cfg(all(test, feature = "uffs"))]
mod tests {
    #[test]
    fn cached_index_reader_exposes_a_sync_entrypoint() {
        let _ = uffs_mft::MftReader::read_index_cached_sync;
    }

    #[test]
    fn file_identity_keeps_the_ntfs_sequence_number() {
        assert_eq!(
            super::ntfs_file_index(0x0000_1234_5678_9abc, 0xcdef),
            0xcdef_1234_5678_9abc
        );
    }
}

#[cfg(feature = "uffs")]
fn is_c_drive_root(path: &NormalizedPath) -> bool {
    path.display().eq_ignore_ascii_case(r"C:\")
}

#[cfg(feature = "uffs")]
fn path_is_within(root: &NormalizedPath, path: &NormalizedPath) -> bool {
    path.display().len() >= root.display().len()
        && path.display()[..root.display().len()].eq_ignore_ascii_case(root.display())
}

#[cfg(feature = "uffs")]
fn path_depth(path: &NormalizedPath) -> usize {
    path.display().bytes().filter(|byte| *byte == b'\\').count() - 1
}

#[cfg(feature = "uffs")]
fn parent_path(path: &NormalizedPath, root: &NormalizedPath) -> Option<NormalizedPath> {
    let (parent, _) = path.display().rsplit_once('\\')?;
    if parent.len() == 2 {
        Some(root.clone())
    } else {
        normalize_path(parent).ok()
    }
}

#[cfg(feature = "uffs")]
fn warning_kind(kind: ScanIssueKind) -> ScanWarningKind {
    match kind {
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
    }
}
