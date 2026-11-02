use std::collections::BTreeMap;
use std::{
    collections::HashSet,
    fs,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Sender},
    },
    thread,
    time::{Duration, Instant},
};

use application::{
    AppError, CancellationToken, CleanupActions, CleanupItemResult, CleanupPlan, CleanupProgress,
    CleanupResult, RuleService, ScanPort, ScanRequest, ScanResult, ScanService,
    preview_cleanup_with_explicit,
};
use domain::{
    AdminOperation, AdminRequest, CleanupCandidate, CleanupMode, Confidence, PROTOCOL_VERSION,
    RecoveryMode, RequestId, RiskLevel, RuleMatchContext, ScanEngine, ScanEvent, ScanIndex,
    ScanPhase, ScanStatus, normalize_path,
};
use persistence::OperationDb;
use platform_windows::{FastFileScanner, FileScanner, ScanOptions, WindowsFileScanner};

const STATISTICS_SCAN_MAX_ENTRIES: u64 = 10_000_000;

#[derive(Debug)]
pub struct ScanReport {
    pub index: ScanIndex,
    pub engine: ScanEngine,
    pub status: ScanStatus,
    pub files: u64,
    pub logical_bytes: u64,
    pub candidate_count: usize,
    pub estimated_bytes: u64,
    pub warnings: u64,
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub categories: Vec<ScanCategory>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScanProgress {
    pub engine: ScanEngine,
    pub phase: ScanPhase,
    pub files: u64,
    pub logical_bytes: u64,
    pub warnings: u64,
    pub progress_percent: u8,
    pub current_path: Option<String>,
    pub categories: Vec<ScanCategory>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScanCategory {
    pub key: String,
    pub group: String,
    pub group_order: u32,
    pub name: String,
    pub display_order: u32,
    pub count: u64,
    pub estimated_bytes: u64,
}

struct LiveCategoryStats<'a> {
    rules: &'a [domain::Rule],
    categories: BTreeMap<String, ScanCategory>,
}

impl<'a> LiveCategoryStats<'a> {
    fn new(rules: &'a [domain::Rule]) -> Self {
        Self {
            rules,
            categories: declared_scan_categories(rules),
        }
    }

    fn add_file(&mut self, file: &domain::FileRecord) {
        let bytes = statistic_file_size(file);
        if let Some(key) = self.category_key_for_file(file) {
            self.add_stats(&key, 1, bytes);
        }
    }

    fn category_key_for_file(&self, file: &domain::FileRecord) -> Option<String> {
        if let Some((_, rule)) = self.matching_rule(file) {
            return Some(
                rule.display_item_for_path(file.path())
                    .map(|item| item.key.clone())
                    .unwrap_or_else(|| rule.category().to_owned()),
            );
        }
        self.display_category_for_path(file)
    }

    fn display_category_for_path(&self, file: &domain::FileRecord) -> Option<String> {
        self.rules
            .iter()
            .enumerate()
            .filter_map(|(index, rule)| {
                rule.display_item_for_path(file.path())
                    .map(|item| (index, rule.priority(), item.key.clone()))
            })
            .max_by(
                |(left_index, left_priority, _), (right_index, right_priority, _)| {
                    left_priority
                        .cmp(right_priority)
                        .then_with(|| right_index.cmp(left_index))
                },
            )
            .map(|(_, _, key)| key)
    }

    fn matching_rule(&self, file: &domain::FileRecord) -> Option<(usize, &domain::Rule)> {
        self.rules
            .iter()
            .enumerate()
            .filter(|(_, rule)| rule.matches_path(file.path()))
            .fold(None, |selected, (index, rule)| match selected {
                None => Some((index, rule)),
                Some((old_index, old_rule))
                    if rule.priority() > old_rule.priority()
                        || (rule.priority() == old_rule.priority() && index < old_index) =>
                {
                    Some((index, rule))
                }
                Some(current) => Some(current),
            })
    }

    fn add_stats(&mut self, key: &str, count: u64, bytes: u64) {
        if let Some(category) = self.categories.get_mut(key) {
            category.count = category.count.saturating_add(count);
            category.estimated_bytes = category.estimated_bytes.saturating_add(bytes);
        }
    }

    fn snapshot(&self) -> Vec<ScanCategory> {
        self.categories.values().cloned().collect()
    }
}

struct WindowsScanPort {
    progress: Sender<ScanProgress>,
    engine_uffs: Arc<AtomicBool>,
    rules: Vec<domain::Rule>,
    initial_category_stats: Vec<(String, u64, u64)>,
    final_categories: Arc<Mutex<Option<Vec<ScanCategory>>>>,
}

impl ScanPort for WindowsScanPort {
    fn scan(
        &self,
        request: &ScanRequest,
        cancel: &CancellationToken,
    ) -> Result<ScanResult, AppError> {
        let (sender, receiver) = mpsc::sync_channel(1024);
        let platform_cancel = platform_windows::CancellationToken::default();
        let bridge_cancel = Arc::new(AtomicBool::new(false));
        let bridge_stop = Arc::clone(&bridge_cancel);
        let bridge_source = cancel.clone();
        let bridge_target = platform_cancel.clone();
        let bridge = thread::spawn(move || {
            while !bridge_stop.load(Ordering::Acquire) {
                if bridge_source.is_cancelled() {
                    bridge_target.cancel();
                } else if bridge_source.is_paused() {
                    bridge_target.pause();
                } else {
                    bridge_target.resume();
                }
                thread::sleep(Duration::from_millis(10));
            }
        });
        let scan_root = request.root.clone();
        let scan_options = ScanOptions {
            max_depth: request.max_depth,
            max_entries: request.max_entries,
            // Lock state is checked again immediately before cleanup. Avoid opening every
            // scanned file a second time during the read-only discovery pass.
            collect_lock_state: false,
        };
        let scan_thread = thread::spawn(move || {
            FastFileScanner.scan(scan_root, scan_options, sender, platform_cancel)
        });
        let mut index = ScanIndex::new_with_path_matcher(
            request.root.clone(),
            platform_windows::paths_equal_ordinal,
        );
        let mut engine = ScanEngine::Win32;
        let mut phase = ScanPhase::Starting;
        let mut total_entries = None;
        let mut processed_entries = None;
        let mut last_progress = Instant::now() - Duration::from_millis(100);
        let mut category_stats = LiveCategoryStats::new(&self.rules);
        for (key, count, bytes) in &self.initial_category_stats {
            category_stats.add_stats(key, *count, *bytes);
        }
        let mut current_path = None;
        let timing = std::env::var_os("RUSTEN_SCAN_TIMING").is_some();
        let started = Instant::now();
        let mut classification_time = Duration::ZERO;
        let mut index_time = Duration::ZERO;
        for event in receiver {
            let force_progress = matches!(
                &event,
                ScanEvent::ScannerStatus { .. }
                    | ScanEvent::Completed
                    | ScanEvent::Partial
                    | ScanEvent::Canceled
            );
            match event {
                ScanEvent::ScannerStatus {
                    engine: next_engine,
                    phase: next_phase,
                    total_entries: next_total_entries,
                    processed_entries: next_processed_entries,
                } => {
                    if timing && phase != next_phase {
                        eprintln!(
                            "scan phase {next_phase:?}: {:.3}s",
                            started.elapsed().as_secs_f64()
                        );
                    }
                    engine = next_engine;
                    phase = next_phase;
                    total_entries = next_total_entries;
                    processed_entries = next_processed_entries;
                    self.engine_uffs
                        .store(matches!(engine, ScanEngine::Uffs), Ordering::Release);
                }
                ScanEvent::Directory(directory) => {
                    index.add_directory(directory);
                }
                ScanEvent::File(file) => {
                    current_path = Some(file.path().display().to_owned());
                    let classify_started = timing.then(Instant::now);
                    category_stats.add_file(&file);
                    if let Some(start) = classify_started {
                        classification_time += start.elapsed();
                    }
                    let index_started = timing.then(Instant::now);
                    index.add_file(file);
                    if let Some(start) = index_started {
                        index_time += start.elapsed();
                    }
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
                ScanEvent::ReparsePoint(record) => {
                    index.add_reparse_point(record);
                }
                ScanEvent::Progress(_) | ScanEvent::CleanupCandidate(_) => {}
            }
            if force_progress || last_progress.elapsed() >= Duration::from_millis(100) {
                let statistics = index.statistics();
                let _ = self.progress.send(ScanProgress {
                    engine,
                    phase,
                    files: statistics.file_count(),
                    logical_bytes: statistics.logical_size(),
                    warnings: index.warnings().len() as u64 + index.omitted_warnings(),
                    progress_percent: progress_percent(
                        engine,
                        phase,
                        statistics,
                        total_entries,
                        processed_entries,
                        request.max_entries,
                    ),
                    current_path: current_path.clone(),
                    categories: category_stats.snapshot(),
                });
                last_progress = Instant::now();
            }
        }
        bridge_cancel.store(true, Ordering::Release);
        if timing {
            eprintln!(
                "scan work: classification={:.3}s index={:.3}s",
                classification_time.as_secs_f64(),
                index_time.as_secs_f64()
            );
        }
        let _ = bridge.join();
        let summary = scan_thread
            .join()
            .map_err(|_| AppError::Scanner("Windows scan worker panicked".into()))?
            .map_err(|error| AppError::Scanner(error.to_string()))?;
        let status = summary.status();
        *self.final_categories.lock().unwrap() = Some(category_stats.snapshot());
        ScanResult::from_index(index, status)
    }
}

fn progress_percent(
    engine: ScanEngine,
    phase: ScanPhase,
    statistics: &domain::ScanStatistics,
    total_entries: Option<u64>,
    processed_entries: Option<u64>,
    max_entries: u64,
) -> u8 {
    let discovered = statistics
        .file_count()
        .saturating_add(statistics.directory_count().saturating_sub(1));
    let total = if matches!(engine, ScanEngine::Uffs) && total_entries.is_some() {
        total_entries.unwrap_or(max_entries)
    } else {
        max_entries
    };
    let fraction = processed_entries
        .unwrap_or(discovered)
        .saturating_mul(100)
        .checked_div(total.max(1))
        .unwrap_or(0)
        .min(100);
    if matches!(engine, ScanEngine::Uffs) {
        match phase {
            ScanPhase::Preparing => (fraction * 40 / 100) as u8,
            ScanPhase::Enumerating => (40 + fraction * 59 / 100) as u8,
            ScanPhase::Finalizing => 99,
            _ => 0,
        }
    } else {
        fraction.min(99) as u8
    }
}

#[cfg(test)]
pub fn scan_system_drive(
    cancel: CancellationToken,
    progress: Sender<ScanProgress>,
) -> Result<ScanReport, String> {
    let root = normalize_path(r"C:\").map_err(|error| error.to_string())?;
    scan_system_drive_at(root, cancel, progress)
}

pub fn scan_system_drive_at(
    root: domain::NormalizedPath,
    cancel: CancellationToken,
    progress: Sender<ScanProgress>,
) -> Result<ScanReport, String> {
    let timing = std::env::var_os("RUSTEN_SCAN_TIMING").is_some();
    let started = Instant::now();
    let checkpoint = |stage: &str| {
        if timing {
            eprintln!(
                "scan timing {stage}: {:.3}s",
                started.elapsed().as_secs_f64()
            );
        }
    };
    let system_scope = root.display().eq_ignore_ascii_case(r"C:\");
    let volume = platform_windows::query_volume_stats(&root).map_err(|error| error.to_string())?;
    let engine_uffs = Arc::new(AtomicBool::new(false));
    let final_categories = Arc::new(Mutex::new(None));
    let rule_service = RuleService::load_builtin().map_err(|error| error.to_string())?;
    let rules = rule_service.rules().to_vec();
    let recycle_bin_query =
        system_scope.then(platform_windows::shell::recycle_bin::query_current_user_stats);
    let recycle_bin_stats = recycle_bin_query
        .as_ref()
        .and_then(|result| result.as_ref().ok())
        .copied();
    let mut initial_category_stats = Vec::new();
    if let Some(stats) = recycle_bin_stats {
        initial_category_stats.push(("recycle_bin".to_owned(), stats.item_count, stats.bytes));
    }
    let restore_points_bytes = system_scope
        .then(|| platform_windows::recovery::query_c_drive_shadow_storage_bytes().ok())
        .flatten();
    checkpoint("external statistics");
    if let Some(bytes) = restore_points_bytes {
        initial_category_stats.push(("system.restore_points".to_owned(), 1, bytes));
    }
    let service = ScanService::new(
        WindowsScanPort {
            progress: progress.clone(),
            engine_uffs: Arc::clone(&engine_uffs),
            rules: rules.clone(),
            initial_category_stats,
            final_categories: Arc::clone(&final_categories),
        },
        rule_service,
    );
    let handle = service.start_with_cancellation(
        ScanRequest {
            root: root.clone(),
            max_depth: ScanOptions::default().max_depth,
            max_entries: u64::MAX,
        },
        cancel.clone(),
    );
    let rules = service.rules().map_err(|error| error.to_string())?;
    let result = handle.finish().map_err(|error| error.to_string())?;
    checkpoint("enumeration and live categories");
    let engine = if engine_uffs.load(Ordering::Acquire) {
        ScanEngine::Uffs
    } else {
        ScanEngine::Win32
    };
    let (mut index, status) = result.into_parts();
    if cancel.is_cancelled() || status == ScanStatus::Canceled {
        return Ok(canceled_scan_report(index, engine, &volume));
    }
    // Refresh after scanning so items recycled during the scan are also actionable.
    let recycle_bin_query = if system_scope {
        Some(platform_windows::shell::recycle_bin::query_current_user_stats())
    } else {
        recycle_bin_query
    };
    let recycle_bin_stats = match recycle_bin_query {
        Some(Ok(stats)) => Some(stats),
        Some(Err(error)) => {
            index.add_warning(domain::ScanWarning::new(
                None,
                format!("回收站读取失败：{error}"),
            ));
            None
        }
        None => None,
    };
    let mut supplemental_files = if system_scope && needs_supplemental_statistics(status) {
        let platform_cancel = platform_windows::CancellationToken::default();
        let worker = spawn_supplemental_statistics_scan(
            supplemental_statistics_patterns(&rules),
            platform_cancel.clone(),
        );
        while !worker.is_finished() {
            if cancel.is_cancelled() {
                platform_cancel.cancel();
            }
            thread::sleep(Duration::from_millis(10));
        }
        worker.join().unwrap_or_default()
    } else {
        Vec::new()
    };
    if system_scope {
        scan_redirected_profile_files(&mut index, &cancel, &mut supplemental_files);
    }
    checkpoint("refresh and supplemental statistics");
    if cancel.is_cancelled() {
        return Ok(canceled_scan_report(index, engine, &volume));
    }
    let files = index.files();
    let mut candidate_files = files.clone();
    candidate_files.extend(supplemental_files.iter().cloned());
    let context = platform_windows::running_process_names()
        .map(|names| RuleMatchContext::default().with_running_processes(names))
        .unwrap_or_default();
    let mut candidates = domain::generate_candidates(&rules, &candidate_files, &context);
    checkpoint("candidate matching");
    mark_locked_cleanup_candidates_with_cancel(&mut candidates, &cancel);
    if cancel.is_cancelled() {
        return Ok(canceled_scan_report(index, engine, &volume));
    }
    checkpoint("candidate lock checks");
    if let Some(candidate) =
        recycle_bin_stats.and_then(|stats| recycle_bin_candidate_if_present(&root, stats))
    {
        candidates.push(candidate);
    }
    let mut categories = final_categories.lock().unwrap().take().unwrap_or_default();
    let reparse_categories = final_categories_for_files(&rules, &[], index.reparse_points());
    for category in reparse_categories {
        add_category_stats(
            &mut categories,
            &category.key,
            category.count,
            category.estimated_bytes,
        );
    }
    add_supplemental_category_stats(
        &rules,
        &mut categories,
        &files,
        index.reparse_points(),
        &supplemental_files,
    );
    if let Some(stats) = recycle_bin_stats {
        add_external_category_stats(
            &mut categories,
            "recycle_bin",
            stats.item_count,
            stats.bytes,
        );
    }
    if let Some(bytes) = restore_points_bytes {
        add_external_category_stats(&mut categories, "system.restore_points", 1, bytes);
    }
    if system_scope {
        candidates.extend(system_storage_candidates(&categories));
    }
    let executable_plan = prepare_cleanup_plan_with_explicit(&candidates);
    let executable_candidates = executable_plan
        .items
        .iter()
        .map(|item| item.candidate.clone())
        .collect::<Vec<_>>();
    reconcile_actionable_category_stats(&mut categories, &executable_candidates);
    index.add_cleanup_candidates_after_scan(candidates.clone());
    let statistics = index.statistics();
    let files_count = statistics.file_count();
    let logical_bytes = statistics.logical_size();
    let warnings = index.warnings().len() as u64 + index.omitted_warnings();
    let _ = progress.send(ScanProgress {
        engine,
        phase: ScanPhase::Finalizing,
        files: statistics.file_count(),
        logical_bytes: statistics.logical_size(),
        warnings,
        progress_percent: 99,
        current_path: None,
        categories: categories.clone(),
    });
    checkpoint("complete");
    Ok(ScanReport {
        index,
        engine,
        status,
        files: files_count,
        logical_bytes,
        candidate_count: executable_plan.items.len(),
        estimated_bytes: executable_plan.estimated_bytes,
        warnings,
        total_bytes: volume.total_bytes(),
        available_bytes: volume.available_bytes(),
        categories,
    })
}

fn scan_redirected_profile_files(
    index: &mut ScanIndex,
    cancel: &CancellationToken,
    files: &mut Vec<domain::FileRecord>,
) {
    let Some(local) = std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from) else {
        return;
    };
    let Some(profile) = local
        .parent()
        .and_then(std::path::Path::parent)
        .map(std::path::Path::to_path_buf)
    else {
        return;
    };
    let mut roots = vec![local];
    if let Some(roaming) = std::env::var_os("APPDATA").map(std::path::PathBuf::from) {
        roots.push(roaming.join("Tencent/WeChat"));
        roots.push(roaming.join("Microsoft/Windows/Recent"));
    }
    roots.push(profile.join("Documents/WeChat Files"));

    for logical_root in roots {
        if cancel.is_cancelled() {
            return;
        }
        let Ok(logical) = normalize_path(&logical_root.to_string_lossy()) else {
            continue;
        };
        let Ok(metadata) = platform_windows::read_metadata(&logical) else {
            continue;
        };
        if !metadata.is_directory() {
            continue;
        }
        let Ok(physical) = normalize_path(
            metadata
                .physical_path()
                .strip_prefix(r"\\?\")
                .unwrap_or(metadata.physical_path()),
        ) else {
            continue;
        };
        if platform_windows::paths_equal_ordinal(logical.display(), physical.display()) {
            continue;
        }
        let (sender, receiver) = mpsc::channel();
        let scan_cancel = platform_windows::CancellationToken::default();
        let worker_cancel = scan_cancel.clone();
        let scan_root = physical.clone();
        let worker = thread::spawn(move || {
            WindowsFileScanner.scan(
                scan_root,
                ScanOptions {
                    collect_lock_state: false,
                    ..ScanOptions::default()
                },
                sender,
                worker_cancel,
            )
        });
        for event in receiver {
            if cancel.is_cancelled() {
                scan_cancel.cancel();
            }
            let ScanEvent::File(file) = event else {
                continue;
            };
            let Some(suffix) = file.path().display().get(physical.display().len()..) else {
                continue;
            };
            let Ok(mapped) = normalize_path(&format!("{}{suffix}", logical.display())) else {
                continue;
            };
            files.push(
                domain::FileRecord::new(
                    mapped,
                    index.root_directory_id(),
                    file.identity(),
                    file.logical_size(),
                    file.allocated_size(),
                )
                .with_facts(file.facts().clone()),
            );
        }
        match worker.join() {
            Ok(Ok(_)) => {}
            Ok(Err(error)) => {
                index.add_warning(domain::ScanWarning::new(
                    Some(logical),
                    format!("重定向目录扫描失败：{error}"),
                ));
            }
            Err(_) => {
                index.add_warning(domain::ScanWarning::new(
                    Some(logical),
                    "重定向目录扫描线程异常退出",
                ));
            }
        }
    }
}

#[cfg(test)]
fn final_categories_for_candidates(
    rules: &[domain::Rule],
    candidates: &[CleanupCandidate],
) -> Vec<ScanCategory> {
    aggregate_scan_categories(rules, candidates)
}

#[cfg(test)]
fn statistics_patterns(rules: &[domain::Rule]) -> Vec<String> {
    rules
        .iter()
        .flat_map(|rule| rule.display_items())
        .flat_map(|item| item.paths.iter().cloned())
        .collect()
}

fn supplemental_statistics_patterns(rules: &[domain::Rule]) -> Vec<String> {
    rules
        .iter()
        .flat_map(|rule| rule.display_items())
        .flat_map(|item| item.paths.iter().cloned())
        .collect()
}

fn spawn_supplemental_statistics_scan(
    patterns: Vec<String>,
    cancel: platform_windows::CancellationToken,
) -> thread::JoinHandle<Vec<domain::FileRecord>> {
    thread::spawn(move || {
        platform_windows::filesystem::enumerate_statistics_files_for_patterns_with_cancel(
            &patterns,
            STATISTICS_SCAN_MAX_ENTRIES,
            &cancel,
        )
        .unwrap_or_default()
    })
}

fn needs_supplemental_statistics(status: ScanStatus) -> bool {
    status == ScanStatus::Partial
}

fn final_categories_for_files(
    rules: &[domain::Rule],
    files: &[domain::FileRecord],
    reparse_points: &[domain::FileRecord],
) -> Vec<ScanCategory> {
    let mut stats = LiveCategoryStats::new(rules);
    for file in files.iter().chain(reparse_points) {
        stats.add_file(file);
    }
    stats.snapshot()
}

#[cfg(test)]
fn final_categories_with_external_stats(
    rules: &[domain::Rule],
    candidates: &[CleanupCandidate],
    restore_points_bytes: Option<u64>,
) -> Vec<ScanCategory> {
    let mut categories = final_categories_for_candidates(rules, candidates);
    if let Some(bytes) = restore_points_bytes {
        add_external_category_stats(&mut categories, "system.restore_points", 1, bytes);
    }
    categories
}

fn add_category_stats(categories: &mut [ScanCategory], key: &str, count: u64, bytes: u64) {
    if let Some(category) = categories.iter_mut().find(|category| category.key == key) {
        category.count = category.count.saturating_add(count);
        category.estimated_bytes = category.estimated_bytes.saturating_add(bytes);
    }
}

fn add_external_category_stats(categories: &mut [ScanCategory], key: &str, count: u64, bytes: u64) {
    let already_present = categories
        .iter()
        .find(|category| category.key == key)
        .is_some_and(|category| category.count != 0);
    if !already_present {
        add_category_stats(categories, key, count, bytes);
    }
}

fn add_supplemental_category_stats(
    rules: &[domain::Rule],
    categories: &mut [ScanCategory],
    existing: &[domain::FileRecord],
    existing_reparse_points: &[domain::FileRecord],
    supplemental: &[domain::FileRecord],
) {
    if supplemental.is_empty() {
        return;
    }
    let stats = LiveCategoryStats::new(rules);
    let mut seen = HashSet::new();
    for file in existing.iter().chain(existing_reparse_points) {
        if let Some(key) = stats.category_key_for_file(file) {
            seen.insert((key, file.path().display().to_ascii_lowercase()));
        }
    }
    for file in supplemental {
        let Some(key) = stats.category_key_for_file(file) else {
            continue;
        };
        if seen.insert((key.clone(), file.path().display().to_ascii_lowercase())) {
            add_category_stats(categories, &key, 1, statistic_file_size(file));
        }
    }
}

fn reconcile_actionable_category_stats(
    categories: &mut [ScanCategory],
    candidates: &[CleanupCandidate],
) {
    let mut actionable = std::collections::BTreeMap::<&str, (u64, u64)>::new();
    for candidate in candidates {
        if candidate.quick_clean_block_reason.is_some() {
            continue;
        }
        let Some(key) = candidate.category.as_deref() else {
            continue;
        };
        if !matches!(
            key,
            "system.temp" | "system.cache" | "system.logs" | "system.debug"
        ) {
            continue;
        }
        let entry = actionable.entry(key).or_default();
        entry.0 = entry.0.saturating_add(1);
        entry.1 = entry
            .1
            .saturating_add(candidate.estimated_size.unwrap_or(0));
    }
    for category in categories {
        if matches!(
            category.key.as_str(),
            "system.temp" | "system.cache" | "system.logs" | "system.debug"
        ) {
            let (count, bytes) = actionable
                .get(category.key.as_str())
                .copied()
                .unwrap_or_default();
            category.count = count;
            category.estimated_bytes = bytes;
        }
    }
}

#[cfg(test)]
fn mark_locked_cleanup_candidates(candidates: &mut Vec<CleanupCandidate>) {
    mark_locked_cleanup_candidates_with_cancel(candidates, &CancellationToken::default());
}

fn mark_locked_cleanup_candidates_with_cancel(
    candidates: &mut Vec<CleanupCandidate>,
    cancel: &CancellationToken,
) {
    candidates.retain_mut(|candidate| {
        if cancel.is_cancelled() {
            return true;
        }
        if candidate.file_identity.is_none() {
            return true;
        }
        let status = platform_windows::filesystem::lock_status(&candidate.path);
        if matches!(status, platform_windows::filesystem::LockStatus::Unknown(code)
            if std::io::Error::from_raw_os_error(code as i32).kind()
                == std::io::ErrorKind::NotFound)
        {
            return false;
        }
        if candidate.mode == Some(CleanupMode::Quick)
            && quick_clean_probe_blocks(&candidate.path, status)
        {
            candidate.quick_clean_eligible = false;
            candidate.quick_clean_block_reason = Some(
                match status {
                    platform_windows::filesystem::LockStatus::Locked => {
                        "文件正在使用，无法立即清理"
                    }
                    platform_windows::filesystem::LockStatus::AccessDenied => {
                        "当前权限不足，无法删除"
                    }
                    platform_windows::filesystem::LockStatus::Unknown(_) => {
                        "无法验证删除权限，暂不清理"
                    }
                    platform_windows::filesystem::LockStatus::Available => unreachable!(),
                }
                .into(),
            );
        }
        true
    });
}

fn quick_clean_probe_blocks(
    path: &domain::NormalizedPath,
    status: platform_windows::filesystem::LockStatus,
) -> bool {
    status != platform_windows::filesystem::LockStatus::Available
        && !(status == platform_windows::filesystem::LockStatus::AccessDenied
            && requires_admin_cleanup(path))
}

fn canceled_scan_report(
    mut index: ScanIndex,
    engine: ScanEngine,
    volume: &platform_windows::VolumeStats,
) -> ScanReport {
    index.mark_canceled();
    let statistics = index.statistics();
    ScanReport {
        files: statistics.file_count(),
        logical_bytes: statistics.logical_size(),
        warnings: index.warnings().len() as u64 + index.omitted_warnings(),
        index,
        engine,
        status: ScanStatus::Canceled,
        candidate_count: 0,
        estimated_bytes: 0,
        total_bytes: volume.total_bytes(),
        available_bytes: volume.available_bytes(),
        categories: builtin_scan_categories(),
    }
}

fn statistic_file_size(file: &domain::FileRecord) -> u64 {
    if file.allocated_size() == 0 {
        file.logical_size()
    } else {
        file.allocated_size()
    }
}

fn recycle_bin_candidate(root: &domain::NormalizedPath, bytes: u64) -> CleanupCandidate {
    CleanupCandidate {
        path: root.clone(),
        file_identity: None,
        rule_id: Some("recycle_bin".into()),
        category: Some("recycle_bin".into()),
        reason: Some("清空当前用户在所有本地磁盘上的回收站，需要明确确认".into()),
        estimated_size: Some(bytes),
        quick_clean_eligible: false,
        quick_clean_block_reason: Some("回收站清空需要明确确认".into()),
        mode: Some(CleanupMode::Manual),
        risk: Some(RiskLevel::P2),
        confidence: Some(Confidence::High),
        recovery: Some(RecoveryMode::Recycle),
        locked: Some(false),
        path_allowed: Some(true),
        file_identity_unchanged: Some(true),
        last_write_time: None,
    }
}

fn recycle_bin_candidate_if_present(
    root: &domain::NormalizedPath,
    stats: platform_windows::shell::recycle_bin::RecycleBinStats,
) -> Option<CleanupCandidate> {
    (stats.item_count != 0).then(|| recycle_bin_candidate(root, stats.bytes))
}

fn system_storage_candidates(categories: &[ScanCategory]) -> Vec<CleanupCandidate> {
    [
        (
            "system.virtual_memory",
            r"C:\pagefile.sys",
            "禁用分页文件配置，重启后释放虚拟内存文件",
            "虚拟内存清理需要管理员权限并在重启后生效",
        ),
        (
            "system.hibernate",
            r"C:\hiberfil.sys",
            "关闭系统休眠并释放休眠文件",
            "关闭休眠需要管理员权限，且会同时关闭快速启动",
        ),
    ]
    .into_iter()
    .filter_map(|(key, path, reason, block_reason)| {
        let category = categories
            .iter()
            .find(|category| category.key == key && category.count != 0)?;
        Some(CleanupCandidate {
            path: normalize_path(path).expect("fixed system storage path must be valid"),
            file_identity: None,
            rule_id: Some(key.into()),
            category: Some(key.into()),
            reason: Some(reason.into()),
            estimated_size: Some(category.estimated_bytes),
            quick_clean_eligible: false,
            quick_clean_block_reason: Some(block_reason.into()),
            mode: Some(CleanupMode::Manual),
            risk: Some(RiskLevel::P2),
            confidence: Some(Confidence::High),
            recovery: Some(RecoveryMode::DeleteOnly),
            locked: Some(false),
            path_allowed: Some(true),
            file_identity_unchanged: Some(true),
            last_write_time: None,
        })
    })
    .collect()
}

pub fn builtin_scan_categories() -> Vec<ScanCategory> {
    RuleService::load_builtin()
        .map(|service| {
            declared_scan_categories(service.rules())
                .into_values()
                .collect()
        })
        .unwrap_or_default()
}

fn declared_scan_categories(
    rules: &[domain::Rule],
) -> std::collections::BTreeMap<String, ScanCategory> {
    let mut categories = std::collections::BTreeMap::<String, ScanCategory>::new();
    for rule in rules {
        if rule.display_items().is_empty() {
            categories.insert(
                rule.category().to_owned(),
                ScanCategory {
                    key: rule.category().to_owned(),
                    group: rule.group().to_owned(),
                    group_order: rule.group_order(),
                    name: rule.display_name().to_owned(),
                    display_order: rule.display_order(),
                    count: 0,
                    estimated_bytes: 0,
                },
            );
        } else {
            for item in rule.display_items() {
                categories.insert(
                    item.key.clone(),
                    ScanCategory {
                        key: item.key.clone(),
                        group: rule.group().to_owned(),
                        group_order: rule.group_order(),
                        name: item.name.clone(),
                        display_order: item.order,
                        count: 0,
                        estimated_bytes: 0,
                    },
                );
            }
        }
    }
    categories
}

#[cfg(test)]
fn aggregate_scan_categories(
    rules: &[domain::Rule],
    candidates: &[CleanupCandidate],
) -> Vec<ScanCategory> {
    let mut categories = declared_scan_categories(rules);
    for candidate in candidates {
        let key = candidate
            .category
            .clone()
            .unwrap_or_else(|| "未分类".into());
        let rule = candidate
            .rule_id
            .as_deref()
            .and_then(|id| rules.iter().find(|rule| rule.id() == id));
        let display_item =
            rule.and_then(|rule| rule.display_items().iter().find(|item| item.key == key));
        let entry = categories
            .entry(key.clone())
            .or_insert_with(|| ScanCategory {
                key: key.clone(),
                group: rule.map_or_else(|| "其他清理".into(), |rule| rule.group().into()),
                group_order: rule.map_or(100, |rule| rule.group_order()),
                name: display_item.map_or_else(
                    || {
                        rule.map_or_else(
                            || key.clone(),
                            |rule| rule.display_name_for_path(&candidate.path),
                        )
                    },
                    |item| item.name.clone(),
                ),
                display_order: display_item.map_or(999, |item| item.order),
                count: 0,
                estimated_bytes: 0,
            });
        if let Some(item) = display_item {
            entry.name = item.name.clone();
        } else if let Some(rule) = rule {
            entry.name = rule.display_name_for_path(&candidate.path);
        }
        entry.count = entry.count.saturating_add(1);
        entry.estimated_bytes = entry
            .estimated_bytes
            .saturating_add(candidate.estimated_size.unwrap_or(0));
    }
    categories.into_values().collect()
}

#[cfg(test)]
mod tests {
    use platform_windows::FileScanner;
    use std::{
        fs,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::{
        LiveCategoryStats, add_category_stats, add_external_category_stats,
        aggregate_scan_categories, builtin_scan_categories, declared_scan_categories,
        mark_locked_cleanup_candidates, needs_supplemental_statistics,
        prepare_cleanup_plan_with_explicit, reconcile_actionable_category_stats,
        spawn_supplemental_statistics_scan, statistics_patterns,
    };

    #[test]
    fn supplemental_statistics_scan_is_safe_for_empty_patterns() {
        let files = spawn_supplemental_statistics_scan(
            Vec::new(),
            platform_windows::CancellationToken::default(),
        )
        .join()
        .expect("supplemental scan worker must exit");
        assert!(files.is_empty());
    }

    #[test]
    fn canceled_supplemental_scan_does_not_walk_roots() {
        let cancel = platform_windows::CancellationToken::default();
        cancel.cancel();
        let files = spawn_supplemental_statistics_scan(vec![r"C:\Windows\*".into()], cancel)
            .join()
            .unwrap();
        assert!(files.is_empty());
    }

    #[test]
    fn canceled_scan_retains_index_without_building_cleanup_work() {
        let root = normalize_path(&std::env::temp_dir().to_string_lossy()).unwrap();
        let volume = platform_windows::query_volume_stats(&root).unwrap();
        let mut index = ScanIndex::new(root.clone());
        index.add_file(FileRecord::new(
            normalize_path(&format!(r"{}\retained.tmp", root.display())).unwrap(),
            index.root_directory_id(),
            FileIdentity {
                volume_serial: 1,
                file_index: 1,
            },
            42,
            42,
        ));
        let report = super::canceled_scan_report(index, ScanEngine::Win32, &volume);
        assert_eq!(report.status, domain::ScanStatus::Canceled);
        assert_eq!(report.files, 1);
        assert_eq!(report.logical_bytes, 42);
        assert_eq!(report.candidate_count, 0);
        assert_eq!(report.index.files().len(), 1);
    }

    #[test]
    fn complete_scan_does_not_repeat_supplemental_directory_walk() {
        assert!(!needs_supplemental_statistics(domain::ScanStatus::Complete));
        assert!(needs_supplemental_statistics(domain::ScanStatus::Partial));
        assert!(!needs_supplemental_statistics(domain::ScanStatus::Canceled));
    }
    use application::RuleService;
    use domain::{
        CleanupCandidate, CleanupMode, Confidence, FileFacts, FileIdentity, FileRecord, LockState,
        RecoveryMode, RiskLevel, RuleMatchContext, ScanEngine, ScanIndex, ScanPhase,
        ScanStatistics, normalize_path,
    };

    #[test]
    fn final_categories_are_derived_from_cleanup_candidates() {
        let service = RuleService::load_builtin().unwrap();
        let index = ScanIndex::new(normalize_path(r"C:\").unwrap());
        let file = FileRecord::new(
            normalize_path(r"C:\Windows\Temp\final.tmp").unwrap(),
            index.root_directory_id(),
            FileIdentity {
                volume_serial: 1,
                file_index: 21,
            },
            2048,
            4096,
        );
        let candidates = service.candidates(&[file], &RuleMatchContext::default());
        let categories = super::final_categories_for_candidates(service.rules(), &candidates);

        assert_eq!(
            categories
                .iter()
                .map(|category| category.estimated_bytes)
                .sum::<u64>(),
            candidates
                .iter()
                .map(|candidate| candidate.estimated_size.unwrap_or(0))
                .sum::<u64>()
        );
        assert_eq!(
            categories
                .iter()
                .find(|category| category.key == "system.temp")
                .unwrap()
                .estimated_bytes,
            4096
        );
    }

    #[test]
    fn final_categories_include_external_restore_point_stats_once() {
        let service = RuleService::load_builtin().unwrap();
        let categories =
            super::final_categories_with_external_stats(service.rules(), &[], Some(12_345));
        let restore_points = categories
            .iter()
            .find(|category| category.key == "system.restore_points")
            .unwrap();

        assert_eq!(restore_points.count, 1);
        assert_eq!(restore_points.estimated_bytes, 12_345);
    }

    #[test]
    fn seeded_external_restore_point_stats_are_not_added_twice() {
        let service = RuleService::load_builtin().unwrap();
        let mut categories = declared_scan_categories(service.rules())
            .into_values()
            .collect::<Vec<_>>();
        add_category_stats(&mut categories, "system.restore_points", 1, 12_345);

        add_external_category_stats(&mut categories, "system.restore_points", 1, 12_345);

        let restore_points = categories
            .iter()
            .find(|category| category.key == "system.restore_points")
            .unwrap();
        assert_eq!(restore_points.count, 1);
        assert_eq!(restore_points.estimated_bytes, 12_345);
    }

    #[test]
    fn uffs_progress_does_not_reset_between_preparing_and_enumerating() {
        let statistics = ScanStatistics::default();
        assert_eq!(
            super::progress_percent(
                ScanEngine::Uffs,
                ScanPhase::Preparing,
                &statistics,
                Some(200),
                Some(100),
                200
            ),
            20
        );
        assert_eq!(
            super::progress_percent(
                ScanEngine::Uffs,
                ScanPhase::Preparing,
                &statistics,
                Some(200),
                Some(200),
                200
            ),
            40
        );
        assert_eq!(
            super::progress_percent(
                ScanEngine::Uffs,
                ScanPhase::Enumerating,
                &statistics,
                Some(100),
                None,
                200
            ),
            40
        );
        assert_eq!(
            super::progress_percent(
                ScanEngine::Uffs,
                ScanPhase::Finalizing,
                &statistics,
                Some(100),
                Some(100),
                200
            ),
            99
        );
    }

    #[test]
    fn builtin_categories_are_available_before_scan() {
        let categories = builtin_scan_categories();

        assert!(
            categories
                .iter()
                .any(|category| category.key == "system.temp")
        );
        assert!(
            categories
                .iter()
                .any(|category| category.key == "recycle_bin")
        );
    }

    #[test]
    fn statistics_patterns_include_cache_display_paths() {
        let service = RuleService::load_builtin().unwrap();
        let patterns = statistics_patterns(service.rules());

        assert!(
            patterns
                .iter()
                .any(|pattern| { pattern.contains(r"Windows\SoftwareDistribution\DataStore") })
        );
        assert!(
            patterns
                .iter()
                .any(|pattern| { pattern.contains(r"AppData\Local\Microsoft\Windows\INetCache") })
        );
        assert!(
            patterns
                .iter()
                .any(|pattern| { pattern.contains(r"AppData\Local\Microsoft\Windows\Explorer") })
        );
        assert!(
            patterns
                .iter()
                .any(|pattern| pattern.contains(r"Windows\Servicing\LCU"))
        );
    }

    #[test]
    fn keeps_declared_categories_and_aggregates_matched_size() {
        let service = RuleService::load_builtin().unwrap();
        let index = ScanIndex::new(normalize_path(r"C:\").unwrap());
        let file = FileRecord::new(
            normalize_path(r"C:\Windows\Temp\test.tmp").unwrap(),
            index.root_directory_id(),
            FileIdentity {
                volume_serial: 1,
                file_index: 1,
            },
            2048,
            2048,
        );
        let candidates = service.candidates(&[file], &RuleMatchContext::default());
        let categories = aggregate_scan_categories(service.rules(), &candidates);

        assert!(
            categories.iter().any(|category| {
                category.key == "system.cache" && category.estimated_bytes == 0
            })
        );
        assert!(
            categories.iter().any(|category| {
                category.key == "system.temp" && category.estimated_bytes == 2048
            })
        );
    }

    #[test]
    fn live_category_stats_count_rule_matched_files_before_scan_finishes() {
        let service = RuleService::load_builtin().unwrap();
        let mut index = ScanIndex::new(normalize_path(r"C:\").unwrap());
        let file = FileRecord::new(
            normalize_path(r"C:\Windows\Temp\live.tmp").unwrap(),
            index.root_directory_id(),
            FileIdentity {
                volume_serial: 1,
                file_index: 7,
            },
            3072,
            4096,
        );
        let mut stats = LiveCategoryStats::new(service.rules());

        stats.add_file(&file);

        let temp = stats
            .snapshot()
            .into_iter()
            .find(|category| category.key == "system.temp")
            .unwrap();
        assert_eq!(temp.count, 1);
        assert_eq!(temp.estimated_bytes, 4096);
        index.add_file(file.clone());
        let mut plan =
            super::prepare_cleanup_plan_with_explicit(&[super::explicit_file_candidate(&file)]);
        super::classify_explicit_cleanup_items(&mut plan, &index);
        assert_eq!(
            plan.items[0].candidate.category.as_deref(),
            Some("system.temp")
        );
        assert!(plan.items[0].candidate.rule_id.is_none());
        assert_eq!(plan.by_category.get("system.temp"), Some(&4096));
    }

    #[test]
    fn adds_external_stats_to_the_declared_category() {
        let service = RuleService::load_builtin().unwrap();
        let mut stats = LiveCategoryStats::new(service.rules());
        stats.add_stats("recycle_bin", 3, 17_000);

        let recycle_bin = stats
            .snapshot()
            .into_iter()
            .find(|category| category.key == "recycle_bin")
            .unwrap();
        assert_eq!(recycle_bin.count, 3);
        assert_eq!(recycle_bin.estimated_bytes, 17_000);
    }

    #[test]
    fn display_paths_fill_system_categories_without_creating_candidates() {
        let service = RuleService::load_builtin().unwrap();
        let index = ScanIndex::new(normalize_path(r"C:\").unwrap());
        let files = vec![
            FileRecord::new(
                normalize_path(r"C:\Windows\Servicing\LCU\package.cab").unwrap(),
                index.root_directory_id(),
                FileIdentity {
                    volume_serial: 1,
                    file_index: 3,
                },
                4096,
                8192,
            ),
            FileRecord::new(
                normalize_path(r"C:\swapfile.sys").unwrap(),
                index.root_directory_id(),
                FileIdentity {
                    volume_serial: 1,
                    file_index: 4,
                },
                1024,
                1024,
            ),
        ];
        let mut stats = LiveCategoryStats::new(service.rules());
        for file in &files {
            stats.add_file(file);
        }

        let categories = stats.snapshot();
        let lcu = categories
            .iter()
            .find(|category| category.key == "system.lcu")
            .unwrap();
        assert_eq!(lcu.count, 1);
        assert_eq!(lcu.estimated_bytes, 8192);
        let virtual_memory = categories
            .iter()
            .find(|category| category.key == "system.virtual_memory")
            .unwrap();
        assert_eq!(virtual_memory.count, 1);
        assert_eq!(virtual_memory.estimated_bytes, 1024);

        assert!(
            service
                .candidates(&files, &RuleMatchContext::default())
                .is_empty()
        );
    }

    #[test]
    fn system_storage_statistics_create_manual_cleanup_candidates() {
        let service = RuleService::load_builtin().unwrap();
        let mut categories = declared_scan_categories(service.rules())
            .into_values()
            .collect::<Vec<_>>();
        add_category_stats(&mut categories, "system.virtual_memory", 2, 10_000);
        add_category_stats(&mut categories, "system.hibernate", 1, 20_000);

        let candidates = super::system_storage_candidates(&categories);

        assert_eq!(candidates.len(), 2);
        for candidate in &candidates {
            assert_eq!(candidate.mode, Some(CleanupMode::Manual));
            assert_eq!(candidate.risk, Some(RiskLevel::P2));
            assert_eq!(candidate.confidence, Some(Confidence::High));
            assert_eq!(candidate.locked, Some(false));
            assert_eq!(candidate.path_allowed, Some(true));
            assert_eq!(candidate.file_identity_unchanged, Some(true));
        }
        let paging = candidates
            .iter()
            .find(|candidate| candidate.rule_id.as_deref() == Some("system.virtual_memory"))
            .unwrap();
        assert_eq!(paging.path.display(), r"C:\pagefile.sys");
        assert_eq!(paging.estimated_size, Some(10_000));
        let hibernation = candidates
            .iter()
            .find(|candidate| candidate.rule_id.as_deref() == Some("system.hibernate"))
            .unwrap();
        assert_eq!(hibernation.path.display(), r"C:\hiberfil.sys");
        assert_eq!(hibernation.estimated_size, Some(20_000));
    }

    #[test]
    fn cache_statistics_have_an_executable_cleanup_plan() {
        let service = RuleService::load_builtin().unwrap();
        let index = ScanIndex::new(normalize_path(r"C:\").unwrap());
        let file = FileRecord::new(
            normalize_path(
                r"C:\Users\Administrator\AppData\Local\Microsoft\Windows\INetCache\cache.bin",
            )
            .unwrap(),
            index.root_directory_id(),
            FileIdentity {
                volume_serial: 1,
                file_index: 6,
            },
            4096,
            8192,
        );

        let candidates =
            service.candidates(std::slice::from_ref(&file), &RuleMatchContext::default());
        assert_eq!(candidates.len(), 1);
        let plan = super::prepare_cleanup_plan_with_explicit(&candidates);
        assert_eq!(plan.items.len(), 1);
        assert_eq!(plan.estimated_bytes, 8192);
        assert_eq!(plan.by_category.get("system.cache"), Some(&8192));
        let categories = super::final_categories_for_files(service.rules(), &[file], &[]);
        let cache = categories
            .iter()
            .find(|category| category.key == "system.cache")
            .unwrap();

        assert_eq!(cache.count, 1);
        assert_eq!(cache.estimated_bytes, 8192);
    }

    #[test]
    fn displayed_system_cleanup_files_are_actionable_candidates() {
        let service = RuleService::load_builtin().unwrap();
        let index = ScanIndex::new(normalize_path(r"C:\").unwrap());
        let cases = [
            (
                r"C:\Windows\SoftwareDistribution\Download\update.cab",
                "system.cache",
            ),
            (
                r"C:\Users\Administrator\AppData\Local\CrashDumps\app.wer",
                "system.debug",
            ),
            (r"C:\Windows\Logs\CBS\CBS.log", "system.logs"),
            (r"C:\Windows\Temp\cleanup.tmp", "system.temp"),
        ];

        for (path, category) in cases {
            let file = FileRecord::new(
                normalize_path(path).unwrap(),
                index.root_directory_id(),
                FileIdentity {
                    volume_serial: 1,
                    file_index: 100,
                },
                4096,
                4096,
            );
            let candidates = service.candidates(&[file], &RuleMatchContext::default());
            assert_eq!(candidates.len(), 1, "{path} must be actionable");
            assert_eq!(candidates[0].category.as_deref(), Some(category));
        }
    }

    #[test]
    fn final_cleanup_stats_ignore_display_only_files() {
        let service = RuleService::load_builtin().unwrap();
        let index = ScanIndex::new(normalize_path(r"C:\").unwrap());
        let files = [
            (r"C:\Windows\Temp\cleanup.tmp", "system.temp"),
            (
                r"C:\Windows\SoftwareDistribution\Download\update.cab",
                "system.cache",
            ),
            (r"C:\Windows\Logs\CBS\CBS.log", "system.logs"),
            (r"C:\Windows\Minidump\mini.dmp", "system.debug"),
        ];
        let mut candidates = Vec::new();
        for (number, (path, _)) in files.iter().enumerate() {
            let file = FileRecord::new(
                normalize_path(path).unwrap(),
                index.root_directory_id(),
                FileIdentity {
                    volume_serial: 1,
                    file_index: number as u128 + 1,
                },
                4096,
                4096,
            );
            candidates.extend(service.candidates(&[file], &RuleMatchContext::default()));
        }
        let mut categories = declared_scan_categories(service.rules())
            .into_values()
            .collect::<Vec<_>>();
        for (_, key) in files {
            add_category_stats(&mut categories, key, 9, 9999);
        }

        reconcile_actionable_category_stats(&mut categories, &candidates);

        for key in ["system.temp", "system.cache", "system.logs", "system.debug"] {
            let category = categories
                .iter()
                .find(|category| category.key == key)
                .unwrap();
            assert_eq!(category.count, 1, "{key} must reflect candidates only");
            assert_eq!(category.estimated_bytes, 4096);
        }

        let mut display_only = declared_scan_categories(service.rules())
            .into_values()
            .collect::<Vec<_>>();
        add_category_stats(&mut display_only, "system.cache", 9, 9999);
        reconcile_actionable_category_stats(&mut display_only, &[]);
        let cache = display_only
            .iter()
            .find(|category| category.key == "system.cache")
            .unwrap();
        assert_eq!(cache.count, 0);
        assert_eq!(cache.estimated_bytes, 0);
    }

    #[test]
    fn builtin_input_method_display_matches_nested_files() {
        let service = RuleService::load_builtin().unwrap();
        let file = FileRecord::new(
            normalize_path(r"C:\Windows\System32\IME\IMEJP\nested.dll").unwrap(),
            ScanIndex::new(normalize_path(r"C:\").unwrap()).root_directory_id(),
            FileIdentity {
                volume_serial: 1,
                file_index: 7,
            },
            43_000_000,
            43_000_000,
        );
        let categories = super::final_categories_for_files(service.rules(), &[file], &[]);
        let input = categories
            .iter()
            .find(|category| category.key == "system.input_methods")
            .unwrap();
        assert_eq!(input.estimated_bytes, 43_000_000);
    }

    #[test]
    fn builtin_input_method_display_matches_reparse_files() {
        let service = RuleService::load_builtin().unwrap();
        let file = FileRecord::new(
            normalize_path(r"C:\Windows\System32\IME\IMEJP\IMJPCD.DLL").unwrap(),
            ScanIndex::new(normalize_path(r"C:\").unwrap()).root_directory_id(),
            FileIdentity {
                volume_serial: 1,
                file_index: 8,
            },
            43_000_000,
            43_000_000,
        )
        .with_facts(domain::FileFacts {
            reparse_tag: Some(1),
            reparse_kind: Some(domain::ReparseKind::Other(1)),
            ..domain::FileFacts::default()
        });
        let categories = super::final_categories_for_files(service.rules(), &[], &[file]);
        let input = categories
            .iter()
            .find(|category| category.key == "system.input_methods")
            .unwrap();
        assert_eq!(input.estimated_bytes, 43_000_000);
    }

    #[test]
    fn supplemental_statistics_deduplicate_hardlink_bytes() {
        let service = RuleService::load_builtin().unwrap();
        let index = ScanIndex::new(normalize_path(r"C:\").unwrap());
        let first = FileRecord::new(
            normalize_path(r"C:\Windows\System32\IME\first.dll").unwrap(),
            index.root_directory_id(),
            FileIdentity {
                volume_serial: 1,
                file_index: 9,
            },
            10,
            10,
        );
        let hardlink = FileRecord::new(
            normalize_path(r"C:\Windows\System32\IME\first.dll").unwrap(),
            index.root_directory_id(),
            first.identity(),
            10,
            10,
        );
        let new_file = FileRecord::new(
            normalize_path(r"C:\Windows\IME\new.dll").unwrap(),
            index.root_directory_id(),
            FileIdentity {
                volume_serial: 1,
                file_index: 10,
            },
            20,
            20,
        );
        let mut categories = declared_scan_categories(service.rules())
            .into_values()
            .collect::<Vec<_>>();
        super::add_supplemental_category_stats(
            service.rules(),
            &mut categories,
            std::slice::from_ref(&first),
            &[],
            &[hardlink, new_file],
        );
        let input = categories
            .iter()
            .find(|category| category.key == "system.input_methods")
            .unwrap();
        assert_eq!(input.estimated_bytes, 20);
        assert_eq!(input.count, 1);
    }

    #[test]
    fn display_paths_handle_unicode_paths_without_panicking() {
        let index = ScanIndex::new(normalize_path(r"C:\").unwrap());
        let files = [FileRecord::new(
            normalize_path(r"C:\用户\缓存文件.tmp").unwrap(),
            index.root_directory_id(),
            FileIdentity {
                volume_serial: 1,
                file_index: 5,
            },
            512,
            1024,
        )];

        let mut stats = LiveCategoryStats::new(&[]);
        stats.add_file(&files[0]);
        assert!(stats.snapshot().is_empty());
    }

    #[test]
    fn explicit_preview_does_not_require_filesystem_revalidation() {
        let candidate = CleanupCandidate {
            path: normalize_path(r"C:\missing-large-file.bin").unwrap(),
            file_identity: Some(FileIdentity {
                volume_serial: 1,
                file_index: 2,
            }),
            rule_id: None,
            category: Some("大文件".into()),
            reason: Some("用户明确选择的大文件".into()),
            estimated_size: Some(4096),
            quick_clean_eligible: false,
            quick_clean_block_reason: Some("需要用户明确确认".into()),
            mode: Some(CleanupMode::Manual),
            risk: Some(RiskLevel::P2),
            confidence: Some(Confidence::High),
            recovery: Some(RecoveryMode::Recycle),
            locked: Some(false),
            path_allowed: Some(true),
            file_identity_unchanged: Some(true),
            last_write_time: Some(1),
        };

        let plan = prepare_cleanup_plan_with_explicit(&[candidate]);

        assert_eq!(plan.items.len(), 1);
        assert_eq!(plan.estimated_bytes, 4096);
    }

    #[test]
    fn explicit_preview_excludes_candidates_marked_unavailable() {
        use std::os::windows::fs::OpenOptionsExt;

        let path = std::env::temp_dir().join(format!(
            "rusten-locked-preview-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::write(&path, b"locked").unwrap();
        let held = fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&path)
            .unwrap();
        let candidate = CleanupCandidate {
            path: normalize_path(&path.to_string_lossy()).unwrap(),
            file_identity: Some(FileIdentity {
                volume_serial: 1,
                file_index: 3,
            }),
            rule_id: Some("system_temp".into()),
            category: Some("system.temp".into()),
            reason: Some("locked".into()),
            estimated_size: Some(4096),
            quick_clean_eligible: false,
            quick_clean_block_reason: None,
            mode: Some(CleanupMode::Quick),
            risk: Some(RiskLevel::P3),
            confidence: Some(Confidence::High),
            recovery: Some(RecoveryMode::Recycle),
            locked: Some(true),
            path_allowed: Some(true),
            file_identity_unchanged: Some(true),
            last_write_time: Some(1),
        };

        let mut candidates = vec![candidate];
        mark_locked_cleanup_candidates(&mut candidates);
        let plan = prepare_cleanup_plan_with_explicit(&candidates);

        assert!(plan.items.is_empty());
        assert_eq!(plan.estimated_bytes, 0);
        drop(held);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn missing_cached_files_are_removed_before_cleanup_planning() {
        let missing = std::env::temp_dir().join(format!(
            "rusten-missing-candidate-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let candidate = CleanupCandidate {
            path: normalize_path(&missing.to_string_lossy()).unwrap(),
            file_identity: Some(FileIdentity {
                volume_serial: 1,
                file_index: 3,
            }),
            rule_id: Some("system_temp".into()),
            category: Some("system.temp".into()),
            reason: None,
            estimated_size: Some(4096),
            quick_clean_eligible: true,
            quick_clean_block_reason: None,
            mode: Some(CleanupMode::Quick),
            risk: Some(RiskLevel::P3),
            confidence: Some(Confidence::High),
            recovery: Some(RecoveryMode::Recycle),
            locked: Some(false),
            path_allowed: Some(true),
            file_identity_unchanged: Some(true),
            last_write_time: Some(1),
        };
        let mut manual = candidate.clone();
        manual.mode = Some(CleanupMode::Manual);
        let mut candidates = vec![candidate, manual];

        mark_locked_cleanup_candidates(&mut candidates);

        assert!(candidates.is_empty());
    }

    #[test]
    fn admin_fallback_covers_allowed_system_garbage_only() {
        assert!(!super::quick_clean_probe_blocks(
            &normalize_path(r"C:\Windows\Logs\CBS\CBS.log").unwrap(),
            platform_windows::filesystem::LockStatus::AccessDenied,
        ));
        assert!(super::quick_clean_probe_blocks(
            &normalize_path(r"C:\Users\Alice\AppData\Local\Temp\temp.tmp").unwrap(),
            platform_windows::filesystem::LockStatus::AccessDenied,
        ));
        for path in [
            r"C:\Windows\Logs\CBS\CBS.log",
            r"C:\Windows\LiveKernelReports\WATCHDOG\dump.dmp",
            r"C:\Windows\SoftwareDistribution\Download\update.cab",
            r"C:\ProgramData\Microsoft\Windows\DeliveryOptimization\Cache\chunk.bin",
        ] {
            assert!(
                super::requires_admin_cleanup(&normalize_path(path).unwrap()),
                "{path}"
            );
        }
        for path in [
            r"C:\Windows\System32\kernel32.dll",
            r"C:\Windows\SoftwareDistribution\DataStore\DataStore.edb",
            r"C:\Users\Alice\AppData\Local\Temp\temp.tmp",
        ] {
            assert!(
                !super::requires_admin_cleanup(&normalize_path(path).unwrap()),
                "{path}"
            );
        }
    }

    #[test]
    fn recycle_bin_is_emptied_before_new_files_are_recycled() {
        let root = normalize_path(r"C:\").unwrap();
        let mut file = super::recycle_bin_candidate(&root, 100);
        file.path = normalize_path(r"C:\Users\A\AppData\Local\Temp\test.tmp").unwrap();
        file.rule_id = None;
        file.category = Some("system.temp".into());
        let plan = super::prepare_cleanup_plan_with_explicit(&[
            file,
            super::recycle_bin_candidate(&root, 900),
        ]);
        assert_eq!(
            plan.items[0].candidate.rule_id.as_deref(),
            Some("recycle_bin")
        );
        assert_eq!(
            plan.items[1].candidate.category.as_deref(),
            Some("system.temp")
        );
        assert_eq!(plan.estimated_bytes, 1000);
    }

    #[test]
    fn zero_byte_recycle_bin_items_still_create_a_cleanup_candidate() {
        let root = normalize_path(r"C:\").unwrap();
        let candidate = super::recycle_bin_candidate_if_present(
            &root,
            platform_windows::shell::recycle_bin::RecycleBinStats {
                item_count: 3,
                bytes: 0,
            },
        )
        .unwrap();

        assert_eq!(candidate.estimated_size, Some(0));
    }

    #[test]
    fn cleanup_success_requires_the_target_to_be_gone() {
        let folder = std::env::temp_dir().join(format!(
            "rusten-delete-check-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&folder).unwrap();
        let path = folder.join("still-present.tmp");
        fs::write(&path, b"must remain for this check").unwrap();
        let normalized = normalize_path(&path.to_string_lossy()).unwrap();

        let present = super::verify_deleted(&normalized, 12);
        fs::remove_file(&path).unwrap();
        let absent = super::verify_deleted(&normalized, 12);
        fs::remove_dir(&folder).unwrap();

        assert!(present.is_err());
        assert_eq!(absent.unwrap(), 12);
    }

    #[test]
    #[ignore = "permanently deletes only a disposable file created by this test"]
    fn builtin_cache_cleanup_deletes_and_rescans_only_its_disposable_file() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let folder = std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap())
            .join("Microsoft/Windows/INetCache")
            .join(format!("rusten-rule-test-{suffix}"));
        fs::create_dir_all(&folder).unwrap();
        let path = folder.join("disposable-cache.bin");
        fs::write(&path, vec![0u8; 4096]).unwrap();
        let scan = || {
            let (sender, receiver) = std::sync::mpsc::channel();
            platform_windows::WindowsFileScanner
                .scan(
                    normalize_path(&folder.to_string_lossy()).unwrap(),
                    platform_windows::ScanOptions::default(),
                    sender,
                    platform_windows::CancellationToken::default(),
                )
                .unwrap();
            receiver
                .try_iter()
                .filter_map(|event| match event {
                    domain::ScanEvent::File(file) => Some(file),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let service = RuleService::load_builtin().unwrap();
        let candidates = service.candidates(&scan(), &RuleMatchContext::default());
        let plan = prepare_cleanup_plan_with_explicit(&candidates);
        assert_eq!(plan.items.len(), 1);
        assert_eq!(plan.by_category.get("system.cache"), Some(&4096));
        use std::os::windows::fs::OpenOptionsExt;
        let held = fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&path)
            .unwrap();
        let locked_result = super::execute_cleanup(&plan).unwrap();
        assert_eq!(
            locked_result.items[0].result,
            application::CleanupItemResult::SkippedLocked
        );
        assert!(path.exists());
        drop(held);
        let result = super::execute_cleanup(&plan).unwrap();
        assert_eq!(
            result.items[0].result,
            application::CleanupItemResult::DeletedPermanently
        );
        assert!(!path.exists());
        assert!(scan().is_empty());
        fs::remove_dir(&folder).unwrap();
    }

    #[test]
    #[ignore = "read-only full system scan timing on the local machine"]
    fn system_scan_read_only_timing() {
        let (sender, _receiver) = std::sync::mpsc::channel();
        let started = std::time::Instant::now();
        let report = super::scan_system_drive(Default::default(), sender).unwrap();
        eprintln!(
            "scan result: {:?}, {:?}, {} files, {} candidates, {:.3}s",
            report.status,
            report.engine,
            report.files,
            report.candidate_count,
            started.elapsed().as_secs_f64()
        );
        assert_eq!(report.status, domain::ScanStatus::Complete);
        assert!(report.files > 0);
    }

    #[test]
    #[ignore = "runs the real system scan; deletes only four disposable fixture files"]
    fn system_scan_cleanup_roundtrip_tracks_live_files() {
        struct OwnedFolders(Vec<std::path::PathBuf>);
        impl Drop for OwnedFolders {
            fn drop(&mut self) {
                for folder in &self.0 {
                    let _ = fs::remove_dir_all(folder);
                }
            }
        }

        let scan = || {
            let (sender, _receiver) = std::sync::mpsc::channel();
            super::scan_system_drive(Default::default(), sender).unwrap()
        };
        let initial = scan();
        let mut facts = std::collections::BTreeMap::<String, usize>::new();
        for candidate in initial.index.cleanup_candidates() {
            let outcome = platform_windows::cleanup::revalidate(candidate);
            let key = format!(
                "{}: {}",
                candidate.category.as_deref().unwrap_or("unknown"),
                outcome
                    .map(|_| "Valid".to_owned())
                    .unwrap_or_else(|e| format!("{e:?}"))
            );
            *facts.entry(key).or_default() += 1;
        }
        eprintln!("LIVE_SCAN_VALIDATION={facts:?}");
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let local = std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap());
        let mut paths = Vec::new();
        for (relative, category) in [
            ("Temp", "system.temp"),
            ("Microsoft/Windows/INetCache", "system.cache"),
            ("Microsoft/Windows/WER/ReportArchive", "system.logs"),
            ("CrashDumps", "system.debug"),
        ] {
            let folder = local
                .join(relative)
                .join(format!("rusten-roundtrip-{suffix}"));
            fs::create_dir_all(&folder).unwrap();
            let path = folder.join("disposable.bin");
            fs::write(&path, vec![0x5au8; 65536]).unwrap();
            paths.push((path, category));
        }
        let _owned = OwnedFolders(
            paths
                .iter()
                .map(|(path, _)| path.parent().unwrap().to_path_buf())
                .collect(),
        );
        let before = scan();
        let candidates = before
            .index
            .cleanup_candidates()
            .iter()
            .filter(|c| {
                paths
                    .iter()
                    .any(|(p, _)| c.path == normalize_path(&p.to_string_lossy()).unwrap())
            })
            .cloned()
            .collect::<Vec<_>>();
        eprintln!("FIXTURE_SCAN: expected=4 found={}", candidates.len());
        assert_eq!(
            candidates.len(),
            4,
            "full UI scan must find newly created files, not cached entries"
        );
        for candidate in &candidates {
            eprintln!(
                "FIXTURE_BEFORE: {:?} {:?}",
                candidate.path.display(),
                platform_windows::cleanup::revalidate(candidate)
            );
        }
        let plan = prepare_cleanup_plan_with_explicit(&candidates);
        assert_eq!(plan.items.len(), 4);
        let result = super::execute_cleanup(&plan).unwrap();
        eprintln!("FIXTURE_RESULT: {result:?}");
        assert!(
            result
                .items
                .iter()
                .all(|item| item.result == application::CleanupItemResult::DeletedPermanently)
        );
        assert!(paths.iter().all(|(p, _)| !p.exists()));
        let after = scan();
        let remaining = after
            .index
            .cleanup_candidates()
            .iter()
            .filter(|c| {
                paths
                    .iter()
                    .any(|(p, _)| c.path == normalize_path(&p.to_string_lossy()).unwrap())
            })
            .count();
        eprintln!("FIXTURE_RESCAN: remaining={remaining}");
        assert_eq!(
            remaining, 0,
            "deleted files must not reappear in full UI rescan"
        );
    }

    #[test]
    #[ignore = "runs a real system scan and deletes only disposable category fixtures"]
    fn other_cleanup_categories_delete_owned_files() {
        struct OwnedFolders(Vec<std::path::PathBuf>);
        impl Drop for OwnedFolders {
            fn drop(&mut self) {
                for folder in &self.0 {
                    let _ = fs::remove_dir_all(folder);
                }
            }
        }

        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let local = std::path::PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap());
        let marker = format!("rusten-process-test-{suffix}");
        let firefox_root = local.join("Mozilla/Firefox/Profiles").join(&marker);
        let chrome_root = local.join("Google/Chrome/User Data").join(&marker);
        let edge_root = local.join("Microsoft/Edge/User Data").join(&marker);
        let documents = local.parent().unwrap().parent().unwrap().join("Documents");
        let wechat_root = documents.join("WeChat Files").join(&marker);
        let wechat_roaming_root = local
            .parent()
            .unwrap()
            .join("Roaming/Tencent/WeChat")
            .join(&marker);
        let app_root = local.join("VLC/Cache").join(&marker);
        let history_root = local.join("Microsoft/Windows/History").join(&marker);
        let store_root = local.join("Packages").join(&marker);
        let installers_root = local.join("Downloaded Installations").join(&marker);
        let mut owned = vec![
            firefox_root.clone(),
            chrome_root.clone(),
            edge_root.clone(),
            wechat_root.clone(),
            wechat_roaming_root.clone(),
            app_root.clone(),
            history_root.clone(),
            store_root.clone(),
            installers_root.clone(),
        ];
        let mut paths = vec![
            (
                firefox_root.join("cache2/disposable.bin"),
                "browser.firefox",
            ),
            (chrome_root.join("Cache/disposable.bin"), "browser.chrome"),
            (edge_root.join("Cache/disposable.bin"), "browser.edge"),
            (wechat_root.join("Log/disposable.log"), "wechat.logs"),
            (
                wechat_roaming_root.join("Log/disposable.log"),
                "wechat.logs",
            ),
            (
                wechat_root.join("Cache/disposable.bin"),
                "wechat.cache_backup",
            ),
            (
                wechat_root.join("FileStorage/Image/disposable.bin"),
                "wechat.chat_images",
            ),
            (
                wechat_root.join("FileStorage/Video/disposable.bin"),
                "wechat.chat_videos",
            ),
            (
                wechat_root.join("FileStorage/General/disposable.bin"),
                "wechat.moments",
            ),
            (
                wechat_root.join("FileStorage/File/disposable.bin"),
                "wechat.received_files",
            ),
            (app_root.join("disposable.bin"), "app.vlc"),
            (history_root.join("disposable.txt"), "system.history"),
            (store_root.join("LocalCache/disposable.bin"), "system.store"),
            (installers_root.join("disposable.bin"), "system.installers"),
        ];
        for (relative, category) in [
            ("Tencent/WeCom/Cache", "app.wecom"),
            ("Tencent/QQ/Cache", "app.qq"),
            ("Thunder Network/Thunder/Cache", "app.thunder"),
            ("FileZilla/Cache", "app.filezilla"),
            ("VMware/Cache", "app.vmware"),
            ("DingTalk/Cache", "app.dingtalk"),
            ("Tencent/Meeting/Cache", "app.tencent_meeting"),
            ("Adobe/Acrobat/Cache", "app.adobe_reader"),
            ("Microsoft/Office/Cache", "app.office"),
            ("Sun/Java/Deployment/cache", "app.java"),
            ("BaiduNetdisk/Cache", "app.baidu_netdisk"),
            ("Oray/Sunlogin/Cache", "app.sunlogin"),
            ("HiSuite/Cache", "app.hisuite"),
        ] {
            let root = local.join(relative).join(&marker);
            paths.push((root.join("disposable.bin"), category));
            owned.push(root);
        }
        let _owned = OwnedFolders(owned);
        for (path, _) in &paths {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"owned cleanup verification fixture").unwrap();
        }
        let (sender, _receiver) = std::sync::mpsc::channel();
        let report = super::scan_system_drive(Default::default(), sender).unwrap();
        let mut candidates = report
            .index
            .cleanup_candidates()
            .iter()
            .filter(|candidate| {
                paths.iter().any(|(path, _)| {
                    candidate.path == normalize_path(&path.to_string_lossy()).unwrap()
                })
            })
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(
            candidates.len(),
            paths.len(),
            "full scan missed: {:?}",
            paths
                .iter()
                .filter(|(path, _)| !candidates.iter().any(|candidate| {
                    candidate.path == normalize_path(&path.to_string_lossy()).unwrap()
                }))
                .map(|(path, category)| {
                    let normalized = normalize_path(&path.to_string_lossy()).unwrap();
                    (
                        path,
                        category,
                        report
                            .index
                            .iter_files()
                            .any(|file| file.path() == &normalized),
                    )
                })
                .collect::<Vec<_>>()
        );
        for (path, category) in &paths {
            let normalized = normalize_path(&path.to_string_lossy()).unwrap();
            let candidate = candidates
                .iter()
                .find(|candidate| candidate.path == normalized)
                .unwrap();
            assert_eq!(candidate.category.as_deref(), Some(*category), "{path:?}");
        }
        // These unique fixture profiles are not in use by the running apps.
        for candidate in &mut candidates {
            if candidate
                .quick_clean_block_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("related process is running"))
            {
                candidate.quick_clean_block_reason = None;
                candidate.quick_clean_eligible = true;
            }
        }
        let plan = prepare_cleanup_plan_with_explicit(&candidates);
        for (_, category) in &paths {
            assert!(
                plan.by_category.contains_key(*category),
                "{category} missing from plan"
            );
        }
        let result = super::execute_cleanup(&plan).unwrap();
        assert_eq!(result.items.len(), paths.len());
        assert!(
            result
                .items
                .iter()
                .all(|item| { item.result == application::CleanupItemResult::DeletedPermanently }),
            "{result:?}"
        );
        assert!(paths.iter().all(|(path, _)| !path.exists()));
        let (sender, _receiver) = std::sync::mpsc::channel();
        let after = super::scan_system_drive(Default::default(), sender).unwrap();
        assert!(
            after.index.cleanup_candidates().iter().all(|candidate| {
                !paths.iter().any(|(path, _)| {
                    candidate.path == normalize_path(&path.to_string_lossy()).unwrap()
                })
            }),
            "cleaned fixture files reappeared in the scan"
        );
    }

    #[test]
    #[ignore = "permanently deletes only disposable files created by this test"]
    fn selected_files_are_all_deleted_and_absent_from_rescan() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let folder = std::env::temp_dir().join(format!("rusten-cleanup-rescan-{suffix}"));
        fs::create_dir_all(&folder).unwrap();
        let index = ScanIndex::new(normalize_path(r"C:\").unwrap());
        let paths = (0..3)
            .map(|number| folder.join(format!("disposable-{number}.bin")))
            .collect::<Vec<_>>();
        let candidates = paths
            .iter()
            .map(|path| {
                fs::write(path, b"disposable Rusten cleanup verification").unwrap();
                let normalized = normalize_path(&path.display().to_string()).unwrap();
                let metadata = platform_windows::read_metadata(&normalized).unwrap();
                let file = FileRecord::new(
                    normalized,
                    index.root_directory_id(),
                    metadata.identity(),
                    metadata.logical_size(),
                    metadata.allocated_size(),
                )
                .with_facts(FileFacts {
                    last_write_time: metadata.last_write_time(),
                    lock_state: LockState::Available,
                    ..FileFacts::default()
                });
                super::explicit_file_candidate(&file)
            })
            .collect::<Vec<_>>();
        let plan = prepare_cleanup_plan_with_explicit(&candidates);
        let scan_folder = || {
            let (sender, receiver) = std::sync::mpsc::channel();
            platform_windows::WindowsFileScanner
                .scan(
                    normalize_path(&folder.to_string_lossy()).unwrap(),
                    platform_windows::ScanOptions::default(),
                    sender,
                    platform_windows::CancellationToken::default(),
                )
                .unwrap();
            receiver
                .try_iter()
                .filter(|event| matches!(event, domain::ScanEvent::File(_)))
                .count()
        };
        assert_eq!(plan.items.len(), paths.len());
        assert_eq!(scan_folder(), paths.len());

        let result = super::execute_cleanup(&plan).unwrap();

        assert_eq!(result.items.len(), plan.items.len());
        assert!(result.items.iter().all(|item| matches!(
            item.result,
            application::CleanupItemResult::DeletedPermanently
        )));
        assert!(paths.iter().all(|path| !path.exists()));
        assert_eq!(scan_folder(), 0, "rescan must not retain the cleaned file");
        assert_eq!(
            result.recoverable, 0,
            "permanent cleanup must not create a recovery entry"
        );
        fs::remove_dir(&folder).unwrap();
    }
}

pub fn prepare_cleanup_plan_with_explicit(candidates: &[CleanupCandidate]) -> CleanupPlan {
    let mut plan = prepare_cleanup_plan_with(candidates, preview_cleanup_with_explicit);
    // Handle the recycle bin first so a stop request leaves its state predictable.
    plan.items
        .sort_by_key(|item| item.candidate.rule_id.as_deref() != Some("recycle_bin"));
    plan
}

pub fn explicit_file_candidate(file: &domain::FileRecord) -> CleanupCandidate {
    CleanupCandidate {
        path: file.path().clone(),
        file_identity: Some(file.identity()),
        rule_id: None,
        category: Some("手动选择".into()),
        reason: Some("用户明确选择的文件".into()),
        estimated_size: Some(file.logical_size().max(file.allocated_size())),
        quick_clean_eligible: false,
        quick_clean_block_reason: Some("需要用户明确确认".into()),
        mode: Some(CleanupMode::Manual),
        risk: Some(RiskLevel::P2),
        confidence: Some(Confidence::High),
        recovery: Some(RecoveryMode::Recycle),
        locked: Some(false),
        path_allowed: Some(true),
        file_identity_unchanged: Some(true),
        last_write_time: Some(file.facts().last_write_time),
    }
}

pub(crate) fn classify_explicit_cleanup_items(plan: &mut CleanupPlan, index: &ScanIndex) {
    let selected = plan
        .items
        .iter()
        .filter(|item| item.candidate.rule_id.is_none())
        .map(|item| item.candidate.path.display())
        .collect::<std::collections::HashSet<_>>();
    if selected.is_empty() {
        return;
    }
    let Ok(rules) = RuleService::load_builtin() else {
        return;
    };
    let stats = LiveCategoryStats::new(rules.rules());
    let categories = index
        .files_matching(|path| selected.contains(path))
        .filter_map(|file| {
            stats
                .category_key_for_file(&file)
                .map(|key| (file.path().clone(), key))
        })
        .collect::<std::collections::HashMap<_, _>>();
    for item in &mut plan.items {
        if item.candidate.rule_id.is_none()
            && let Some(category) = categories.get(&item.candidate.path)
        {
            item.candidate.category = Some(category.clone());
        }
    }
    plan.by_category.clear();
    for item in &plan.items {
        *plan
            .by_category
            .entry(item.candidate.category.clone().unwrap_or_default())
            .or_default() += item.estimated_bytes;
    }
}

fn prepare_cleanup_plan_with(
    candidates: &[CleanupCandidate],
    preview: fn(&[CleanupCandidate]) -> CleanupPlan,
) -> CleanupPlan {
    let preview_candidates = candidates
        .iter()
        .cloned()
        .map(|candidate| candidate.with_execution_facts(false, true, true))
        .collect::<Vec<_>>();
    preview(&preview_candidates)
}

#[cfg(test)]
pub fn execute_cleanup(plan: &CleanupPlan) -> Result<CleanupResult, String> {
    execute_cleanup_with_progress(plan, |_| {})
}

#[cfg(test)]
pub fn execute_cleanup_with_progress<F>(
    plan: &CleanupPlan,
    on_progress: F,
) -> Result<CleanupResult, String>
where
    F: FnMut(CleanupProgress),
{
    execute_cleanup_with_cancellation(plan, &CancellationToken::default(), on_progress)
}

#[cfg(test)]
pub fn execute_cleanup_with_cancellation<F>(
    plan: &CleanupPlan,
    cancel: &CancellationToken,
    on_progress: F,
) -> Result<CleanupResult, String>
where
    F: FnMut(CleanupProgress),
{
    let root = std::env::var_os("LOCALAPPDATA")
        .map(std::path::PathBuf::from)
        .ok_or_else(|| "LOCALAPPDATA is unavailable; operation log cannot be opened".to_owned())?;
    let data_root = root.join("Rusten");
    execute_cleanup_with_cancellation_at(&data_root, plan, cancel, on_progress)
}

pub fn execute_cleanup_with_cancellation_at<F>(
    data_root: &std::path::Path,
    plan: &CleanupPlan,
    cancel: &CancellationToken,
    on_progress: F,
) -> Result<CleanupResult, String>
where
    F: FnMut(CleanupProgress),
{
    fs::create_dir_all(data_root).map_err(|error| error.to_string())?;
    let log = OperationDb::open(data_root.join("operations.sqlite"))
        .map_err(|error| error.to_string())?;
    let actions = WindowsCleanupActions {
        admin_client: platform_windows::admin_client::AdminClient::new(Duration::from_secs(10)),
        next_request_id: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|time| time.as_nanos() as u64)
            .unwrap_or(1)
            .max(1),
    };
    let service = application::CleanupService::new(actions, log);
    Ok(service.execute_permanently_with_cancellation(plan, cancel, on_progress))
}

#[derive(Clone, Default)]
pub struct CleanupProgressMailbox(Arc<Mutex<Option<CleanupProgress>>>);

impl CleanupProgressMailbox {
    pub fn publish(&self, progress: CleanupProgress) {
        if let Ok(mut latest) = self.0.lock() {
            *latest = Some(progress);
        }
    }

    pub fn take(&self) -> Option<CleanupProgress> {
        self.0.try_lock().ok()?.take()
    }
}

#[cfg(test)]
mod cleanup_progress_tests {
    #[test]
    fn a_burst_keeps_only_the_latest_progress_without_blocking_the_producer() {
        let mailbox = super::CleanupProgressMailbox::default();
        let producer = mailbox.clone();
        std::thread::spawn(move || {
            for processed in 0..100_000 {
                producer.publish(application::CleanupProgress {
                    processed,
                    total: 100_000,
                    ..Default::default()
                });
            }
        })
        .join()
        .unwrap();
        assert_eq!(mailbox.take().unwrap().processed, 99_999);
        assert!(mailbox.take().is_none());
    }
}

struct WindowsCleanupActions {
    admin_client: platform_windows::admin_client::AdminClient,
    next_request_id: u64,
}

fn verify_deleted(path: &domain::NormalizedPath, released_bytes: u64) -> Result<u64, String> {
    match fs::symlink_metadata(path.extended()) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(released_bytes),
        Ok(_) => Err("删除后目标仍然存在".into()),
        Err(error) => Err(format!("无法确认删除结果：{error}")),
    }
}

impl CleanupActions for WindowsCleanupActions {
    fn delete_permanently(&mut self, candidate: &CleanupCandidate) -> Result<u64, String> {
        if candidate.rule_id.as_deref() == Some("recycle_bin") {
            return platform_windows::cleanup::empty_recycle_bin();
        }
        if matches!(
            candidate.rule_id.as_deref(),
            Some("system.hibernate" | "system.virtual_memory")
        ) {
            let request_id = RequestId(self.next_request_id);
            self.next_request_id = self.next_request_id.saturating_add(1).max(1);
            let operation = if candidate.rule_id.as_deref() == Some("system.hibernate") {
                AdminOperation::DisableHibernation
            } else {
                AdminOperation::DisablePagingFiles
            };
            let response = self
                .admin_client
                .request(&AdminRequest {
                    protocol_version: PROTOCOL_VERSION,
                    request_id,
                    operation,
                })
                .map_err(|error| format!("管理员助手系统清理失败：{error:?}"))?;
            return match response.result {
                domain::AdminResult::PermanentlyDeleted { released_bytes }
                    if candidate.rule_id.as_deref() == Some("system.hibernate") =>
                {
                    verify_deleted(&candidate.path, released_bytes)
                }
                domain::AdminResult::Completed { .. }
                    if candidate.rule_id.as_deref() == Some("system.virtual_memory") =>
                {
                    Ok(0)
                }
                domain::AdminResult::Rejected { reason } => {
                    Err(format!("管理员助手拒绝系统清理：{reason:?}"))
                }
                _ => Err("管理员助手返回了错误的系统清理结果".into()),
            };
        }
        let identity = candidate
            .file_identity
            .ok_or("file identity is unavailable")?;
        let size = candidate.estimated_size.ok_or("file size is unavailable")?;
        let last_write_time = candidate
            .last_write_time
            .ok_or("file timestamp is unavailable")?;
        let explicit = candidate.mode == Some(CleanupMode::Manual);
        match platform_windows::filesystem::delete_verified_file(
            &candidate.path,
            identity,
            size,
            last_write_time,
            explicit,
        ) {
            Ok(bytes) => return verify_deleted(&candidate.path, bytes),
            Err(error)
                if error.kind() == std::io::ErrorKind::PermissionDenied
                    && requires_admin_cleanup(&candidate.path) => {}
            Err(error) => return Err(format!("直接删除失败：{error}")),
        }
        let request_id = RequestId(self.next_request_id);
        self.next_request_id = self.next_request_id.saturating_add(1).max(1);
        let response = self
            .admin_client
            .request(&AdminRequest {
                protocol_version: PROTOCOL_VERSION,
                request_id,
                operation: AdminOperation::DeleteFile {
                    path: candidate.path.clone(),
                    identity,
                    size,
                    last_write_time,
                    explicit,
                },
            })
            .map_err(|error| format!("管理员助手删除失败：{error:?}"))?;
        match response.result {
            domain::AdminResult::PermanentlyDeleted { released_bytes } => {
                verify_deleted(&candidate.path, released_bytes)
            }
            domain::AdminResult::Completed { .. } => Err("管理员助手返回了错误的操作结果".into()),
            domain::AdminResult::Rejected { reason } => {
                Err(format!("管理员助手拒绝删除：{reason:?}"))
            }
        }
    }

    fn revalidate(
        &self,
        candidate: &CleanupCandidate,
    ) -> Result<CleanupCandidate, CleanupItemResult> {
        if matches!(
            candidate.rule_id.as_deref(),
            Some("system.hibernate" | "system.virtual_memory")
        ) {
            return Ok(candidate.clone().with_execution_facts(false, true, true));
        }
        match platform_windows::cleanup::revalidate(candidate) {
            Err(platform_windows::cleanup::VerificationFailure::Permission)
                if requires_admin_cleanup(&candidate.path)
                    && candidate.file_identity.is_some()
                    && candidate.estimated_size.is_some()
                    && candidate.last_write_time.is_some() =>
            {
                // The helper revalidates identity, size, timestamp and sharing on its deletion handle.
                Ok(candidate.clone().with_execution_facts(false, true, true))
            }
            result => result,
        }
        .map_err(|failure| match failure {
            platform_windows::cleanup::VerificationFailure::Locked => {
                CleanupItemResult::SkippedLocked
            }
            platform_windows::cleanup::VerificationFailure::Permission => {
                CleanupItemResult::SkippedPermission
            }
            platform_windows::cleanup::VerificationFailure::Forbidden => {
                CleanupItemResult::SkippedForbidden
            }
            platform_windows::cleanup::VerificationFailure::Missing
            | platform_windows::cleanup::VerificationFailure::IdentityChanged
            | platform_windows::cleanup::VerificationFailure::SizeChanged
            | platform_windows::cleanup::VerificationFailure::ModifiedChanged => {
                CleanupItemResult::SkippedChanged
            }
        })
    }

    fn move_to_recycle_bin(&mut self, candidate: &CleanupCandidate) -> Result<u64, String> {
        if candidate.rule_id.as_deref() == Some("recycle_bin") {
            return platform_windows::cleanup::empty_recycle_bin();
        }
        if !requires_admin_cleanup(&candidate.path) {
            return platform_windows::cleanup::move_to_recycle_bin(candidate);
        }
        let request_id = RequestId(self.next_request_id);
        self.next_request_id = self.next_request_id.saturating_add(1).max(1);
        let response = self
            .admin_client
            .request(&AdminRequest {
                protocol_version: PROTOCOL_VERSION,
                request_id,
                operation: AdminOperation::MoveToRecycleBin {
                    path: candidate.path.clone(),
                },
            })
            .map_err(|error| format!("管理员助手回收操作失败：{error:?}"))?;
        match response.result {
            domain::AdminResult::Completed { .. } => Ok(candidate.estimated_size.unwrap_or(0)),
            domain::AdminResult::PermanentlyDeleted { .. } => {
                Err("管理员助手返回了错误的操作结果".into())
            }
            domain::AdminResult::Rejected { reason } => {
                Err(format!("管理员助手拒绝回收操作：{reason:?}"))
            }
        }
    }

    fn backup_then_delete(&mut self, candidate: &CleanupCandidate) -> Result<u64, String> {
        let _ = candidate;
        Err("文件恢复功能已移除".into())
    }
}

fn requires_admin_cleanup(path: &domain::NormalizedPath) -> bool {
    let value = path.display().to_ascii_lowercase();
    !domain::is_forbidden_path(path)
        && (value.get(3..).is_some_and(|relative| {
            relative.starts_with(r"windows\") || relative.starts_with(r"programdata\")
        }))
}
