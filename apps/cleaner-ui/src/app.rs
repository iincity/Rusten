use std::{
    collections::{HashMap, HashSet},
    rc::Rc,
    sync::mpsc,
    time::{Duration, Instant},
};

use application::{CleanupPlan, CleanupProgress, CleanupResult, RecoveryId, RecoveryItem};
use domain::{FileIdentity, FileRecord, ScanIndex};
use gpui::{
    AnyElement, App, Application, Axis, Bounds, Context, DragMoveEvent, MouseButton,
    MouseDownEvent, Pixels, Render, Task, Window, WindowBounds, WindowOptions, div, prelude::*, px,
    rgb, rgba, size,
};
use gpui_component::{
    StyledExt, Theme, VirtualListScrollHandle, init,
    scroll::{Scrollbar, ScrollbarShow, ScrollbarState},
    v_virtual_list,
};

use crate::{
    commands::UiCommand,
    components::density::{LARGE_FILE_ROW_HEIGHT, SIDEBAR_WIDTH},
    components::format_bytes,
    routes::ViewId,
    runtime,
    screens::settings::SettingsModel,
    state::{AppState, ScanViewStatus},
};

const LARGE_FILE_MIN_BYTES: u64 = 100 * 1024 * 1024;
const ANALYSIS_CLEANUP_ROW_HEIGHT: f32 = 48.0;
const ANALYSIS_CLEANUP_ITEM_ROW_HEIGHT: f32 = 38.0;
const ANALYSIS_DIRECTORY_IMPACT_ROW_HEIGHT: f32 = 44.0;
const ANALYSIS_IMPACT_DEFAULT_HEIGHT: f32 = 188.0;

#[cfg(test)]
static CLEANUP_UI_SMOKE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[derive(Clone)]
struct AnalysisSplitDrag;

impl Render for AnalysisSplitDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

#[derive(Clone, Copy, Debug)]
struct AnalysisSplitDragState {
    start_y: Pixels,
    start_impact_height: f32,
}

#[derive(Clone)]
struct AnalysisHorizontalSplitDrag;

impl Render for AnalysisHorizontalSplitDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        gpui::Empty
    }
}

#[derive(Clone, Copy, Debug)]
struct AnalysisHorizontalSplitDragState {
    start_x: Pixels,
    start_sources_width: f32,
}

#[derive(Clone, Debug)]
struct CleanupRunState {
    item_count: usize,
    estimated_bytes: u64,
    progress: CleanupProgress,
    started_at: Instant,
    finished_at: Option<Instant>,
    cancel: application::CancellationToken,
    finished: bool,
    error: Option<String>,
    issues: Vec<application::CleanupResultItem>,
    disk_available_change: Option<i128>,
}

pub struct CleanerApp {
    pub state: AppState,
    scan_task: Option<Task<()>>,
    cleanup_plan_task: Option<Task<()>>,
    cleanup_task: Option<Task<()>>,
    scan_cancel: Option<application::CancellationToken>,
    scan_generation: u64,
    cleanup_plan_generation: u64,
    scan_index: Option<ScanIndex>,
    large_file_records_cache: Option<Vec<FileRecord>>,
    large_file_rows_cache: Option<Rc<Vec<crate::components::large_file_list::LargeFileRow>>>,
    large_file_row_sizes_cache: Option<Rc<Vec<gpui::Size<gpui::Pixels>>>>,
    large_file_scroll_handle: VirtualListScrollHandle,
    large_file_scroll_state: ScrollbarState,
    analysis_cleanup_scroll_handle: VirtualListScrollHandle,
    analysis_cleanup_scroll_state: ScrollbarState,
    directory_scroll_handle: VirtualListScrollHandle,
    directory_scroll_state: ScrollbarState,
    directory_row_sizes_cache: Option<Rc<Vec<gpui::Size<gpui::Pixels>>>>,
    analysis_impact_height: f32,
    analysis_split_drag: Option<AnalysisSplitDragState>,
    analysis_sources_width: f32,
    analysis_horizontal_split_drag: Option<AnalysisHorizontalSplitDragState>,
    cleanup_plan: Option<CleanupPlan>,
    cleanup_result: Option<CleanupResult>,
    cleanup_run: Option<CleanupRunState>,
    cleanup_confirmed: bool,
    cleaned_paths: HashSet<String>,
    selected_directories: HashSet<String>,
    selected_large_files: HashSet<FileIdentity>,
    removed_large_files: HashSet<FileIdentity>,
    recovery_task: Option<Task<()>>,
    recovery_items: Vec<RecoveryItem>,
    recovery_quota: application::BackupQuota,
    recovery_corrupt_records: usize,
    settings: SettingsModel,
    expanded_directories: HashSet<String>,
    space_analysis_path: String,
    active_analysis_category: Option<String>,
    collapsed_cleanup_groups: HashSet<String>,
    dispatch_latency: crate::metrics::UiLatency,
    render_latency: crate::metrics::UiLatency,
}

impl CleanerApp {
    pub fn new() -> Self {
        let recovery = runtime::load_recovery_snapshot();
        let categories = runtime::builtin_scan_categories()
            .into_iter()
            .map(|category| crate::state::CleanupCategoryState {
                key: category.key,
                group: category.group,
                group_order: category.group_order,
                name: category.name,
                display_order: category.display_order,
                count: category.count,
                estimated_bytes: category.estimated_bytes,
                executable_bytes: None,
            })
            .collect::<Vec<_>>();
        let selected_categories = categories
            .iter()
            .map(|category| category.key.clone())
            .collect();
        let mut state = AppState::default();
        state.cleanup.categories = categories;
        state.cleanup.selected_categories = selected_categories;
        Self {
            state,
            scan_task: None,
            cleanup_plan_task: None,
            cleanup_task: None,
            scan_cancel: None,
            scan_generation: 0,
            cleanup_plan_generation: 0,
            scan_index: None,
            large_file_records_cache: None,
            large_file_rows_cache: None,
            large_file_row_sizes_cache: None,
            large_file_scroll_handle: VirtualListScrollHandle::new(),
            large_file_scroll_state: ScrollbarState::default(),
            analysis_cleanup_scroll_handle: VirtualListScrollHandle::new(),
            analysis_cleanup_scroll_state: ScrollbarState::default(),
            directory_scroll_handle: VirtualListScrollHandle::new(),
            directory_scroll_state: ScrollbarState::default(),
            directory_row_sizes_cache: None,
            analysis_impact_height: ANALYSIS_IMPACT_DEFAULT_HEIGHT,
            analysis_split_drag: None,
            analysis_sources_width: 340.0,
            analysis_horizontal_split_drag: None,
            cleanup_plan: None,
            cleanup_result: None,
            cleanup_run: None,
            cleanup_confirmed: false,
            cleaned_paths: HashSet::new(),
            selected_directories: HashSet::new(),
            selected_large_files: HashSet::new(),
            removed_large_files: HashSet::new(),
            recovery_task: None,
            recovery_quota: recovery.quota,
            recovery_corrupt_records: recovery.corrupt_records,
            recovery_items: recovery.items,
            settings: SettingsModel::default_path()
                .map(|path| SettingsModel::load_from(&path))
                .unwrap_or_default(),
            expanded_directories: HashSet::new(),
            space_analysis_path: r"C:\".to_owned(),
            active_analysis_category: None,
            collapsed_cleanup_groups: HashSet::new(),
            dispatch_latency: crate::metrics::UiLatency::default(),
            render_latency: crate::metrics::UiLatency::default(),
        }
    }

    pub fn dispatch(&mut self, command: UiCommand, cx: &mut Context<Self>) {
        let started = Instant::now();
        self.dispatch_inner(command, cx);
        self.dispatch_latency.record(started.elapsed());
    }

    fn cleanup_is_running(&self) -> bool {
        self.cleanup_run.as_ref().is_some_and(|run| !run.finished)
    }

    fn dispatch_inner(&mut self, command: UiCommand, cx: &mut Context<Self>) {
        if (self.cleanup_is_running() || self.cleanup_confirmed || self.cleanup_plan_task.is_some())
            && matches!(
                &command,
                UiCommand::ToggleCleanupCategory(_)
                    | UiCommand::ToggleCleanupGroupSelection(_)
                    | UiCommand::ToggleDirectorySelection(_)
                    | UiCommand::ToggleLargeFileSelection(_)
                    | UiCommand::CreateQuickCleanPreview
                    | UiCommand::ToggleBackup
                    | UiCommand::RestoreOperation(_)
                    | UiCommand::PurgeExpiredRecovery
            )
        {
            return;
        }
        if matches!(command, UiCommand::StartScan) {
            self.start_scan(cx);
            return;
        }
        if matches!(command, UiCommand::CancelScan)
            && let Some(cancel) = &self.scan_cancel
        {
            cancel.cancel();
        }
        if matches!(command, UiCommand::PauseScan)
            && let Some(cancel) = &self.scan_cancel
        {
            cancel.pause();
        }
        if matches!(command, UiCommand::ResumeScan) {
            let Some(cancel) = &self.scan_cancel else {
                return;
            };
            cancel.resume();
        }
        if let UiCommand::ToggleCleanupCategory(key) = &command {
            if !self
                .state
                .cleanup
                .categories
                .iter()
                .any(|category| category.key == *key && category.executable_bytes.is_some())
            {
                return;
            }
            self.state.apply(command);
            self.refresh_cleanup_plan();
            cx.notify();
            return;
        }
        if let UiCommand::InspectCleanupCategory(key) = &command {
            self.active_analysis_category = Some(key.clone());
            self.directory_row_sizes_cache = None;
            cx.notify();
            return;
        }
        if let UiCommand::ToggleCleanupGroup(group) = &command {
            if !self.collapsed_cleanup_groups.remove(group) {
                self.collapsed_cleanup_groups.insert(group.clone());
            }
            cx.notify();
            return;
        }
        if let UiCommand::ToggleCleanupGroupSelection(keys) = &command {
            let keys = keys
                .iter()
                .filter(|key| {
                    self.state.cleanup.categories.iter().any(|category| {
                        category.key == **key && category.executable_bytes.is_some()
                    })
                })
                .cloned()
                .collect::<Vec<_>>();
            if keys.is_empty() {
                return;
            }
            toggle_group_selection(&mut self.state.cleanup.selected_categories, &keys);
            self.refresh_cleanup_plan();
            cx.notify();
            return;
        }
        if let UiCommand::ToggleDirectory(path) = &command {
            if !self.expanded_directories.remove(path) {
                self.expanded_directories.insert(path.clone());
            }
            self.directory_row_sizes_cache = None;
            cx.notify();
            return;
        }
        if let UiCommand::ToggleDirectorySelection(path) = &command {
            if !self.selected_directories.remove(path) {
                self.selected_directories.insert(path.clone());
            }
            self.directory_row_sizes_cache = None;
            self.refresh_cleanup_plan();
            cx.notify();
            return;
        }
        if let UiCommand::OpenAnalysisDirectory(path) = &command {
            self.space_analysis_path = path.clone();
            self.expanded_directories.insert(path.clone());
            self.directory_row_sizes_cache = None;
            cx.notify();
            return;
        }
        if matches!(command, UiCommand::OpenAnalysisParent) {
            if let Some(parent) = analysis_parent_path(&self.space_analysis_path) {
                self.space_analysis_path = parent;
                self.directory_row_sizes_cache = None;
                cx.notify();
            }
            return;
        }
        if let UiCommand::ToggleLargeFileSelection(identity) = &command {
            if !self.selected_large_files.remove(identity) {
                self.selected_large_files.insert(*identity);
            }
            self.large_file_rows_cache = None;
            self.refresh_cleanup_plan();
            cx.notify();
            return;
        }
        if matches!(command, UiCommand::ToggleAutoClean) {
            self.settings.auto_clean = !self.settings.auto_clean;
            self.persist_settings();
            cx.notify();
            return;
        }
        if matches!(command, UiCommand::ToggleBackup) {
            self.settings.backup_enabled = !self.settings.backup_enabled;
            self.persist_settings();
            cx.notify();
            return;
        }
        if matches!(command, UiCommand::CreateQuickCleanPreview) {
            self.state.apply(command);
            self.refresh_cleanup_plan();
            self.cleanup_result = None;
            self.cleanup_confirmed = false;
            cx.notify();
            return;
        }
        if let UiCommand::ExecuteCleanup(plan_id) = command {
            if self.cleanup_is_running() {
                return;
            }
            let plan = if plan_id.0 == 2 {
                self.prepare_selected_large_file_cleanup_plan()
            } else {
                self.cleanup_plan.take().unwrap_or_default()
            };
            if plan.items.is_empty() {
                return;
            }
            self.state.cleanup.selected_count = plan.items.len();
            self.state.cleanup.estimated_bytes = plan.estimated_bytes;
            self.cleanup_plan = Some(plan);
            if !self.cleanup_confirmed {
                self.cleanup_confirmed = true;
                cx.notify();
                return;
            }
            return;
        }
        if matches!(command, UiCommand::CancelCleanup) {
            self.cleanup_confirmed = false;
            self.refresh_cleanup_plan();
            cx.notify();
            return;
        }
        if matches!(command, UiCommand::StopCleanup) {
            if let Some(run) = &self.cleanup_run {
                run.cancel.cancel();
            }
            cx.notify();
            return;
        }
        if matches!(command, UiCommand::ConfirmCleanup) {
            if self.cleanup_is_running() {
                return;
            }
            let Some(mut plan) = self.cleanup_plan.take() else {
                self.cleanup_confirmed = false;
                return;
            };
            let item_count = plan.items.len();
            if item_count == 0 {
                self.cleanup_confirmed = false;
                cx.notify();
                return;
            }
            let estimated_bytes = plan.estimated_bytes;
            self.cleanup_plan_generation = self.cleanup_plan_generation.wrapping_add(1);
            self.cleanup_plan_task = None;
            let cancel = application::CancellationToken::default();
            let worker_cancel = cancel.clone();
            self.cleanup_confirmed = false;
            self.cleanup_result = None;
            self.state.scan_error = None;
            self.state.selected_view = ViewId::Cleanup;
            self.cleanup_run = Some(CleanupRunState {
                item_count,
                estimated_bytes,
                progress: CleanupProgress {
                    total: item_count,
                    ..Default::default()
                },
                started_at: Instant::now(),
                finished_at: None,
                cancel,
                finished: false,
                error: None,
                issues: Vec::new(),
                disk_available_change: None,
            });
            let mailbox = runtime::CleanupProgressMailbox::default();
            let worker_mailbox = mailbox.clone();
            let mut cleaned_paths = std::mem::take(&mut self.cleaned_paths);
            let (result_sender, result_receiver) = mpsc::channel();
            std::thread::spawn(move || {
                let root = domain::normalize_path(r"C:\").unwrap();
                let volume_before = platform_windows::query_volume_stats(&root).ok();
                let result =
                    runtime::execute_cleanup_with_cancellation(&plan, &worker_cancel, |progress| {
                        worker_mailbox.publish(progress);
                        #[cfg(test)]
                        if CLEANUP_UI_SMOKE.load(std::sync::atomic::Ordering::Relaxed) {
                            std::thread::sleep(Duration::from_millis(30));
                        }
                    });
                let volume_after = platform_windows::query_volume_stats(&root).ok();
                let recycle_stats =
                    platform_windows::shell::recycle_bin::query_current_user_stats().ok();
                let mut removed_identities = HashSet::new();
                let mut removed_categories = HashMap::<String, (u64, u64)>::new();
                if let Ok(result) = &result {
                    let completed_paths = result
                        .items
                        .iter()
                        .filter(|item| cleanup_item_completed(&item.result))
                        .map(|item| item.path.as_str())
                        .collect::<HashSet<_>>();
                    let removed_paths = result
                        .items
                        .iter()
                        .filter(|item| cleanup_item_removes_scanned_bytes(&item.result))
                        .map(|item| item.path.as_str())
                        .collect::<HashSet<_>>();
                    plan.items.retain(|item| {
                        if !completed_paths.contains(item.candidate.path.display()) {
                            return true;
                        }
                        if removed_paths.contains(item.candidate.path.display()) {
                            cleaned_paths.insert(item.candidate.path.display().to_owned());
                            if let Some(identity) = item.candidate.file_identity {
                                removed_identities.insert(identity);
                            }
                            if let Some(category) = &item.candidate.category {
                                let removed =
                                    removed_categories.entry(category.clone()).or_default();
                                removed.0 += 1;
                                removed.1 = removed.1.saturating_add(item.estimated_bytes);
                            }
                        }
                        false
                    });
                    plan.estimated_bytes = plan.items.iter().map(|item| item.estimated_bytes).sum();
                    plan.by_category.clear();
                    for item in &plan.items {
                        *plan
                            .by_category
                            .entry(
                                item.candidate
                                    .category
                                    .clone()
                                    .unwrap_or_else(|| "未分类".into()),
                            )
                            .or_default() += item.estimated_bytes;
                    }
                }
                let issues = result
                    .as_ref()
                    .ok()
                    .map(|result| {
                        result
                            .items
                            .iter()
                            .filter(|item| !cleanup_item_completed(&item.result))
                            .take(100)
                            .cloned()
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let recovery = runtime::load_recovery_snapshot();
                let _ = result_sender.send((
                    result,
                    plan,
                    recovery,
                    cleaned_paths,
                    removed_identities,
                    removed_categories,
                    issues,
                    volume_before,
                    volume_after,
                    recycle_stats,
                ));
            });
            let task = cx.spawn(
                async move |this: gpui::WeakEntity<CleanerApp>, cx: &mut gpui::AsyncApp| {
                    loop {
                        // One snapshot per tick keeps file-event bursts off the UI executor.
                        cx.background_executor()
                            .timer(Duration::from_millis(100))
                            .await;
                        let progress = mailbox.take();
                        let result = result_receiver.try_recv();
                        let done = !matches!(&result, Err(mpsc::TryRecvError::Empty));
                        if this
                            .update(cx, |app, cx| {
                                if let Some(progress) = progress
                                    && let Some(run) = &mut app.cleanup_run
                                {
                                    run.progress = progress;
                                }
                                match result {
                                    Ok((
                                        result,
                                        plan,
                                        recovery,
                                        cleaned_paths,
                                        identities,
                                        categories,
                                        issues,
                                        volume_before,
                                        volume_after,
                                        recycle_stats,
                                    )) => {
                                        app.cleaned_paths = cleaned_paths;
                                        app.removed_large_files.extend(identities);
                                        app.selected_large_files
                                            .retain(|id| !app.removed_large_files.contains(id));
                                        for category in &mut app.state.cleanup.categories {
                                            if let Some((count, bytes)) =
                                                categories.get(&category.key)
                                            {
                                                category.count =
                                                    category.count.saturating_sub(*count);
                                                category.estimated_bytes =
                                                    category.estimated_bytes.saturating_sub(*bytes);
                                            }
                                        }
                                        app.state.cleanup.selected_count = plan.items.len();
                                        app.state.cleanup.estimated_bytes = plan.estimated_bytes;
                                        if let Some(stats) = recycle_stats {
                                            for category in &mut app.state.cleanup.categories {
                                                if category.key == "recycle_bin" {
                                                    category.count = stats.item_count;
                                                    category.estimated_bytes = stats.bytes;
                                                }
                                            }
                                        }
                                        app.state
                                            .apply_remaining_selected_cleanup(&plan.by_category);
                                        if let Some(volume) = volume_after {
                                            app.state.scan.available_bytes =
                                                volume.available_bytes();
                                            app.state.scan.total_bytes = volume.total_bytes();
                                        }
                                        app.cleanup_plan = Some(plan);
                                        app.recovery_items = recovery.items;
                                        app.recovery_quota = recovery.quota;
                                        app.recovery_corrupt_records = recovery.corrupt_records;
                                        app.large_file_rows_cache = None;
                                        app.large_file_row_sizes_cache = None;
                                        app.directory_row_sizes_cache = None;
                                        if let Some(run) = &mut app.cleanup_run {
                                            run.disk_available_change = volume_before
                                                .zip(volume_after)
                                                .map(|(before, after)| {
                                                    i128::from(after.available_bytes())
                                                        - i128::from(before.available_bytes())
                                                });
                                            run.issues = issues;
                                            run.finished = true;
                                            run.finished_at = Some(Instant::now());
                                            run.progress.current_path = None;
                                            match result {
                                                Ok(result) => {
                                                    run.progress.processed = result.items.len();
                                                    run.progress.released_bytes =
                                                        result.released_bytes;
                                                    run.progress.skipped = result.skipped;
                                                    run.progress.failed = result.failed;
                                                    app.cleanup_result = Some(result);
                                                }
                                                Err(error) => run.error = Some(error),
                                            }
                                        }
                                    }
                                    Err(mpsc::TryRecvError::Disconnected) => {
                                        if let Some(run) = &mut app.cleanup_run {
                                            run.finished = true;
                                            run.finished_at = Some(Instant::now());
                                            run.error = Some(
                                                "清理工作线程异常退出，请重新扫描核实结果".into(),
                                            );
                                        }
                                    }
                                    Err(mpsc::TryRecvError::Empty) => {}
                                }
                                cx.notify();
                            })
                            .is_err()
                            || done
                        {
                            break;
                        }
                    }
                },
            );
            self.cleanup_task = Some(task);
            cx.notify();
            return;
        }
        if let UiCommand::RestoreOperation(operation_id) = command {
            self.state.selected_view = ViewId::Recovery;
            let task = cx.spawn(
                async move |this: gpui::WeakEntity<CleanerApp>, cx: &mut gpui::AsyncApp| {
                    let result = std::thread::spawn(move || {
                        runtime::restore_recovery(RecoveryId(operation_id.0))
                    })
                    .join()
                    .map_err(|_| "恢复线程异常退出".to_owned())
                    .and_then(|result| result);
                    this.update(cx, |app, cx| {
                        match result {
                            Ok(()) => app.refresh_recovery(),
                            Err(error) => app.state.scan_error = Some(error),
                        }
                        cx.notify();
                    })
                    .ok();
                },
            );
            self.recovery_task = Some(task);
            cx.notify();
            return;
        }
        if matches!(command, UiCommand::PurgeExpiredRecovery) {
            let task = cx.spawn(
                async move |this: gpui::WeakEntity<CleanerApp>, cx: &mut gpui::AsyncApp| {
                    let result = std::thread::spawn(runtime::purge_expired_recovery)
                        .join()
                        .map_err(|_| "恢复备份清理线程异常退出".to_owned())
                        .and_then(|result| result);
                    this.update(cx, |app, cx| {
                        if let Err(error) = result {
                            app.state.scan_error = Some(error);
                        }
                        app.refresh_recovery();
                        cx.notify();
                    })
                    .ok();
                },
            );
            self.recovery_task = Some(task);
            cx.notify();
            return;
        }
        self.state.apply(command);
        cx.notify();
    }

    fn selected_candidates(
        &self,
        candidates: &[domain::CleanupCandidate],
    ) -> Vec<domain::CleanupCandidate> {
        candidates
            .iter()
            .filter(|candidate| {
                !self.cleaned_paths.contains(candidate.path.display())
                    && candidate
                        .file_identity
                        .is_none_or(|identity| !self.selected_large_files.contains(&identity))
                    && candidate.category.as_ref().is_none_or(|category| {
                        self.state.cleanup.selected_categories.contains(category)
                    })
                    && (self.selected_directories.is_empty()
                        || self
                            .selected_directories
                            .iter()
                            .any(|directory| path_is_within(directory, candidate.path.display())))
            })
            .cloned()
            .collect()
    }

    fn refresh_cleanup_plan(&mut self) {
        self.cleanup_plan_generation = self.cleanup_plan_generation.wrapping_add(1);
        let plan = self.prepare_selected_cleanup_plan();
        self.state.cleanup.selected_count = plan.items.len();
        self.state.cleanup.estimated_bytes = plan.estimated_bytes;
        self.cleanup_plan = Some(plan);
    }

    fn prepare_selected_cleanup_plan(&mut self) -> CleanupPlan {
        let Some(index) = &self.scan_index else {
            return CleanupPlan::default();
        };
        let mut candidates = self.selected_candidates(index.cleanup_candidates());
        if !self.selected_large_files.is_empty() {
            self.ensure_large_file_records_cache();
            let selected = &self.selected_large_files;
            let removed = &self.removed_large_files;
            let directories = &self.selected_directories;
            if let Some(files) = &self.large_file_records_cache {
                candidates.extend(
                    files
                        .iter()
                        .filter(|file| {
                            selected.contains(&file.identity())
                                && !removed.contains(&file.identity())
                                && (directories.is_empty()
                                    || directories.iter().any(|directory| {
                                        path_is_within(directory, file.path().display())
                                    }))
                        })
                        .map(runtime::explicit_file_candidate),
                );
            }
        }
        runtime::prepare_cleanup_plan_with_explicit(&candidates)
    }

    fn prepare_selected_large_file_cleanup_plan(&mut self) -> CleanupPlan {
        if self.scan_index.is_none() {
            return CleanupPlan::default();
        }
        self.ensure_large_file_records_cache();
        let selected = &self.selected_large_files;
        let removed = &self.removed_large_files;
        let directories = &self.selected_directories;
        let candidates = self
            .large_file_records_cache
            .as_ref()
            .into_iter()
            .flat_map(|files| files.iter())
            .filter(|file| {
                selected.contains(&file.identity())
                    && !removed.contains(&file.identity())
                    && (directories.is_empty()
                        || directories
                            .iter()
                            .any(|directory| path_is_within(directory, file.path().display())))
            })
            .map(runtime::explicit_file_candidate)
            .collect::<Vec<_>>();
        runtime::prepare_cleanup_plan_with_explicit(&candidates)
    }

    fn ensure_large_file_records_cache(&mut self) {
        if self.large_file_records_cache.is_none() {
            self.large_file_records_cache = self
                .scan_index
                .as_ref()
                .map(|index| cleanup_large_file_records(index, &HashSet::new()))
                .or_else(|| Some(Vec::new()));
        }
    }

    fn persist_settings(&mut self) {
        if let Some(path) = SettingsModel::default_path()
            && let Err(error) = self.settings.save_to(&path)
        {
            self.state.scan_error = Some(format!("设置保存失败：{error}"));
        }
    }

    fn refresh_recovery(&mut self) {
        let snapshot = runtime::load_recovery_snapshot();
        self.recovery_items = snapshot.items;
        self.recovery_quota = snapshot.quota;
        self.recovery_corrupt_records = snapshot.corrupt_records;
    }

    fn start_scan(&mut self, cx: &mut Context<Self>) {
        if self.state.scan.status == ScanViewStatus::Scanning {
            return;
        }
        if self.cleanup_is_running() {
            return;
        }
        self.state.apply(UiCommand::StartScan);
        self.scan_generation = self.scan_generation.wrapping_add(1);
        self.cleanup_plan_generation = self.cleanup_plan_generation.wrapping_add(1);
        let scan_generation = self.scan_generation;
        self.state.scan_error = None;
        self.scan_index = None;
        self.large_file_records_cache = None;
        self.large_file_rows_cache = None;
        self.large_file_row_sizes_cache = None;
        self.directory_row_sizes_cache = None;
        self.active_analysis_category = None;
        self.removed_large_files.clear();
        self.cleaned_paths.clear();
        self.cleanup_plan = None;
        self.cleanup_plan_task = None;
        self.cleanup_task = None;
        self.cleanup_result = None;
        self.cleanup_run = None;
        self.collapsed_cleanup_groups.clear();
        let cancel = application::CancellationToken::default();
        let worker_cancel = cancel.clone();
        self.scan_cancel = Some(cancel);
        let (progress_sender, progress_receiver) = mpsc::channel();
        let (result_sender, result_receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let result = runtime::scan_system_drive(worker_cancel, progress_sender);
            let _ = result_sender.send(result);
        });
        let task = cx.spawn(
            async move |this: gpui::WeakEntity<CleanerApp>, cx: &mut gpui::AsyncApp| {
                loop {
                    while let Ok(progress) = progress_receiver.try_recv() {
                        this.update(cx, |app, cx| {
                            app.state.apply_progress(&progress);
                            cx.notify();
                        })
                        .ok();
                    }
                    match result_receiver.try_recv() {
                        Ok(result) => {
                            this.update(cx, |app, cx| {
                                if app.scan_generation != scan_generation {
                                    return;
                                }
                                app.scan_cancel = None;
                                match result {
                                    Ok(report) => {
                                        app.cleanup_plan = None;
                                        app.cleanup_result = None;
                                        app.expanded_directories.clear();
                                        app.space_analysis_path =
                                            report.index.root().display().to_owned();
                                        app.selected_directories.clear();
                                        app.selected_large_files.clear();
                                        app.removed_large_files.clear();
                                        app.state.apply_report(&report);
                                        app.active_analysis_category = app
                                            .state
                                            .cleanup
                                            .categories
                                            .iter()
                                            .filter(|category| category.estimated_bytes > 0 || category.executable_bytes.is_some())
                                            .max_by_key(|category| category.estimated_bytes)
                                            .map(|category| category.key.clone());
                                        app.scan_index = Some(report.index);
                                        app.large_file_records_cache = None;
                                        app.large_file_rows_cache = None;
                                        app.large_file_row_sizes_cache = None;
                                        app.directory_row_sizes_cache = None;
                                        app.state.cleanup.estimated_bytes = 0;
                                        app.state.cleanup.selected_count = 0;
                                        let candidates = app
                                            .scan_index
                                            .as_ref()
                                            .map(|index| index.cleanup_candidates().to_vec())
                                            .unwrap_or_default();
                                        let plan_generation = app.scan_generation;
                                        let cleanup_plan_generation = app.cleanup_plan_generation;
                                        let task = cx.spawn(
                                            async move |
                                                this: gpui::WeakEntity<CleanerApp>,
                                                cx: &mut gpui::AsyncApp,
                                            | {
                                                let (plan, availability) = cx.background_executor().spawn(async move {
                                                    let plan = runtime::prepare_cleanup_plan_with_explicit(&candidates);
                                                    let availability = plan.by_category.clone();
                                                    (plan, availability)
                                                }).await;
                                                this.update(cx, |app, cx| {
                                                    if app.scan_generation == plan_generation
                                                        && app.cleanup_plan_generation
                                                            == cleanup_plan_generation
                                                    {
                                                        app.state.cleanup.selected_count =
                                                            plan.items.len();
                                                        app.state.apply_cleanup_availability(&availability);
                                                        app.state.cleanup.estimated_bytes = plan.estimated_bytes;
                                                        app.active_analysis_category = app.state.cleanup.categories.iter()
                                                            .filter(|category| category.executable_bytes.is_some())
                                                            .max_by_key(|category| category.executable_bytes)
                                                            .map(|category| category.key.clone());
                                                        app.cleanup_plan = Some(plan);
                                                        app.cleanup_plan_task = None;
                                                        cx.notify();
                                                    }
                                                })
                                                .ok();
                                            },
                                        );
                                        app.cleanup_plan_task = Some(task);
                                    }
                                    Err(error) => {
                                        app.state.scan.status = ScanViewStatus::Failed;
                                        app.state.scan_error = Some(error);
                                    }
                                }
                                cx.notify();
                            })
                            .ok();
                            break;
                        }
                        Err(mpsc::TryRecvError::Disconnected) => {
                            this.update(cx, |app, cx| {
                                if app.scan_generation != scan_generation {
                                    return;
                                }
                                app.scan_cancel = None;
                                app.state.scan.status = ScanViewStatus::Failed;
                                app.state.scan_error = Some("扫描结果通道已断开".to_owned());
                                cx.notify();
                            })
                            .ok();
                            break;
                        }
                        Err(mpsc::TryRecvError::Empty) => {
                            cx.background_executor()
                                .timer(Duration::from_millis(100))
                                .await;
                        }
                    }
                }
            },
        );
        self.scan_task = Some(task);
        cx.notify();
    }
}

pub fn launch() {
    Application::new().run(|cx: &mut App| {
        init(cx);
        Theme::global_mut(cx).scrollbar_show = ScrollbarShow::Always;
        cx.open_window(
            WindowOptions {
                focus: true,
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(922.0), px(521.0)),
                    cx,
                ))),
                ..Default::default()
            },
            |_, cx| cx.new(|_| CleanerApp::new()),
        )
        .expect("failed to open Rusten window");
        cx.activate(true);
    });
}

impl Render for CleanerApp {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let started = Instant::now();
        let element = self.render_frame(window, cx);
        self.render_latency.record(started.elapsed());
        element
    }
}

impl CleanerApp {
    fn render_frame(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let _dispatch_latency = self.dispatch_latency.percentiles();
        let _render_latency = self.render_latency.percentiles();
        let selected_cleanup_bytes = self.state.cleanup.estimated_bytes;
        let scan_summary = match self.state.scan.status {
            ScanViewStatus::Scanning => "正在努力为您扫描垃圾…".to_owned(),
            ScanViewStatus::Paused => "扫描已暂停".to_owned(),
            ScanViewStatus::Complete | ScanViewStatus::Partial => {
                scan_cleanup_summary(&self.state.cleanup)
            }
            ScanViewStatus::Canceled => "扫描已取消".to_owned(),
            ScanViewStatus::Failed => "扫描失败".to_owned(),
            ScanViewStatus::NotScanned => "准备扫描 C 盘".to_owned(),
        };
        let cleanup_progress_ratio = self.cleanup_run.as_ref().map(|run| {
            let total = run.progress.total.max(run.item_count).max(1);
            (run.progress.processed as f32 / total as f32).clamp(0.0, 1.0)
        });
        let footer_message = if let Some(run) = &self.cleanup_run {
            cleanup_run_message(run)
        } else if let Some(error) = &self.state.scan_error {
            format!("操作失败：{error}")
        } else if let Some(result) = &self.cleanup_result {
            let summary = format!(
                "本次清理 {}，成功 {} 项，跳过 {} 项，失败 {} 项",
                format_bytes(result.released_bytes),
                result.items.len() as u64 - result.skipped - result.failed,
                result.skipped,
                result.failed
            );
            cleanup_result_detail(result)
                .map(|detail| format!("{summary}。{detail}"))
                .unwrap_or(summary)
        } else {
            "扫描结果来自共享 ScanIndex，执行前仍需复核".to_owned()
        };
        let content = if self.state.selected_view == ViewId::Cleanup && self.cleanup_run.is_some() {
            self.render_cleanup_process(cx).into_any_element()
        } else {
            self.render_content(cx)
        };
        let content_scroll = if matches!(
            self.state.selected_view,
            ViewId::LargeFiles | ViewId::SpaceAnalysis | ViewId::Cleanup
        ) {
            div()
                .flex_1()
                .h_full()
                .min_h(px(0.0))
                .min_w(px(0.0))
                .id("content-scroll")
                .child(content)
                .into_any_element()
        } else {
            div()
                .flex_1()
                .h_full()
                .min_h(px(0.0))
                .min_w(px(0.0))
                .id("content-scroll")
                .child(content)
                .scrollable(Axis::Vertical)
                .into_any_element()
        };
        div()
            .size_full()
            .flex()
            .bg(rgb(0xffffff))
            .text_xs()
            .text_color(rgb(0x18212b))
            .child(
                div()
                    .w(px(SIDEBAR_WIDTH))
                    .h_full()
                    .flex()
                    .flex_col()
                    .bg(rgb(0xf0f9e9))
                    .border_r_1()
                    .border_color(rgb(0xdcebd3))
                    .p_2()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::BOLD)
                            .text_color(rgb(0x1e5135))
                            .child("Rusten"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(0x66727e))
                            .child("C 盘空间治理"),
                    )
                    .child(
                        div()
                            .mt_1()
                            .text_xs()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("工作区"),
                    )
                    .child(nav_item("首页", ViewId::Home, self.state.selected_view, cx))
                    .child(nav_item(
                        "C盘专清",
                        ViewId::Dashboard,
                        self.state.selected_view,
                        cx,
                    ))
                    .child(nav_item(
                        "空间分析",
                        ViewId::SpaceAnalysis,
                        self.state.selected_view,
                        cx,
                    ))
                    .child(nav_item(
                        "可清理项",
                        ViewId::Cleanup,
                        self.state.selected_view,
                        cx,
                    ))
                    .child(nav_item(
                        "大文件专清",
                        ViewId::LargeFiles,
                        self.state.selected_view,
                        cx,
                    ))
                    .child(nav_item(
                        "恢复记录",
                        ViewId::Recovery,
                        self.state.selected_view,
                        cx,
                    ))
                    .child(nav_item(
                        "设置",
                        ViewId::Settings,
                        self.state.selected_view,
                        cx,
                    ))
                    .child(
                        div()
                            .mt_auto()
                            .text_xs()
                            .text_color(rgb(0x66727e))
                            .child("默认全选 · 直接清理"),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(0x66727e))
                            .child(format!("版本 v{}", env!("CARGO_PKG_VERSION"))),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .min_w(px(0.0))
                    .min_h(px(0.0))
                    .flex()
                    .flex_col()
                    .p_0()
                    .gap_0()
                    .child(
                        div()
                            .h(px(66.0))
                            .min_h(px(66.0))
                            .p_2()
                            .bg(rgb(0xe7fbd7))
                            .flex()
                            .justify_between()
                            .items_center()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .flex()
                                            .items_center()
                                            .gap_1()
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                                    .child(
                                                        if matches!(
                                                            self.state.scan.status,
                                                            ScanViewStatus::Scanning
                                                                | ScanViewStatus::Paused
                                                        ) {
                                                            scan_summary.clone()
                                                        } else {
                                                            "预计可释放".to_owned()
                                                        },
                                                    ),
                                            )
                                            .when(
                                                !matches!(
                                                    self.state.scan.status,
                                                    ScanViewStatus::Scanning
                                                        | ScanViewStatus::Paused
                                                ),
                                                |element| {
                                                    element.child(
                                                        div()
                                                            .text_sm()
                                                            .font_weight(gpui::FontWeight::BOLD)
                                                            .text_color(rgb(0xd92d20))
                                                            .child(format_bytes(
                                                                selected_cleanup_bytes,
                                                            )),
                                                    )
                                                },
                                            ),
                                    )
                                    .when(
                                        !matches!(
                                            self.state.scan.status,
                                            ScanViewStatus::Scanning | ScanViewStatus::Paused
                                        ),
                                        |element| {
                                            element.child(
                                                div()
                                                    .text_xs()
                                                    .text_color(rgb(0x526b45))
                                                    .child(scan_summary),
                                            )
                                        },
                                    ),
                            )
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .child(self.render_scan_controls(cx)),
                            ),
                    )
                    .child(
                        div()
                            .px_2()
                            .bg(rgb(0xe7fbd7))
                            .child(crate::components::scan_progress::render(&self.state.scan)),
                    )
                    .child(content_scroll)
                    .when(
                        self.cleanup_is_running()
                            || self.state.scan_error.is_some()
                            || self.cleanup_result.is_some(),
                        |element| {
                            element.child(
                                div()
                                    .border_t_1()
                                    .border_color(rgb(0xe5e7eb))
                                    .px_2()
                                    .py_1()
                                    .child(
                                        div()
                                            .flex()
                                            .flex_col()
                                            .gap_1()
                                            .child(div().text_xs().truncate().child(footer_message))
                                            .when_some(cleanup_progress_ratio, |element, ratio| {
                                                element.child(
                                                    div()
                                                        .w(px(220.0))
                                                        .h(px(3.0))
                                                        .bg(rgb(0xe5e7eb))
                                                        .overflow_hidden()
                                                        .child(
                                                            div()
                                                                .h(px(3.0))
                                                                .w(px(220.0 * ratio))
                                                                .bg(rgb(0x16835b)),
                                                        ),
                                                )
                                            }),
                                    ),
                            )
                        },
                    ),
            )
            .when(self.cleanup_confirmed, |element| {
                element.child(self.render_cleanup_confirmation(cx))
            })
    }

    fn render_scan_controls(&self, cx: &Context<Self>) -> impl IntoElement {
        let scanning = self.state.scan.status == ScanViewStatus::Scanning;
        let cleaning = self.cleanup_is_running();
        div()
            .flex()
            .items_center()
            .justify_end()
            .gap_1()
            .text_xs()
            .child(
                div()
                    .id("toggle-auto-clean-header")
                    .tab_index(0)
                    .px_1()
                    .py(gpui::px(2.0))
                    .on_click(
                        cx.listener(|app, _, _, cx| app.dispatch(UiCommand::ToggleAutoClean, cx)),
                    )
                    .child(format!(
                        "自动清理加速  {}",
                        if self.settings.auto_clean {
                            "● 已开启"
                        } else {
                            "○ 未开启"
                        }
                    )),
            )
            .when(!scanning, |element| {
                element.child(
                    div()
                        .id("start-scan")
                        .tab_index(0)
                        .px_1()
                        .py(gpui::px(2.0))
                        .rounded_sm()
                        .bg(rgb(0x2563eb))
                        .text_color(rgb(0xffffff))
                        .when(!cleaning, |element| {
                            element.on_click(
                                cx.listener(|app, _, _, cx| app.dispatch(UiCommand::StartScan, cx)),
                            )
                        })
                        .child(
                            if matches!(
                                self.state.scan.status,
                                ScanViewStatus::Complete | ScanViewStatus::Partial
                            ) {
                                "重新扫描"
                            } else {
                                "开始扫描"
                            },
                        ),
                )
            })
            .when(scanning, |element| {
                element.child(
                    div()
                        .id("cancel-scan")
                        .tab_index(0)
                        .px_1()
                        .py(gpui::px(2.0))
                        .rounded_sm()
                        .border_1()
                        .border_color(rgb(0xe5e7eb))
                        .on_click(
                            cx.listener(|app, _, _, cx| app.dispatch(UiCommand::CancelScan, cx)),
                        )
                        .child("停止扫描"),
                )
            })
            .when(cleaning, |element| {
                element.child(
                    div()
                        .id("cleanup-running")
                        .tab_index(0)
                        .px_1()
                        .py(gpui::px(2.0))
                        .rounded_sm()
                        .bg(rgb(0x667085))
                        .text_color(rgb(0xffffff))
                        .cursor_pointer()
                        .on_click(cx.listener(|app, _, _, cx| {
                            app.dispatch(UiCommand::Navigate(ViewId::Cleanup), cx)
                        }))
                        .child("清理中…"),
                )
            })
            .when(
                !cleaning
                    && self
                        .cleanup_plan
                        .as_ref()
                        .is_some_and(|plan| !plan.items.is_empty()),
                |element| {
                    element.child(
                        div()
                            .id("execute-cleanup")
                            .tab_index(0)
                            .px_1()
                            .py(gpui::px(2.0))
                            .rounded_sm()
                            .bg(rgb(0x16835b))
                            .text_color(rgb(0xffffff))
                            .on_click(cx.listener(|app, _, _, cx| {
                                app.dispatch(UiCommand::ExecuteCleanup(domain::PlanId(1)), cx)
                            }))
                            .child("立即清理"),
                    )
                },
            )
    }
}

fn cleanup_phase_text(run: &CleanupRunState) -> &'static str {
    if run.finished {
        if run.error.is_some() {
            "清理未完成"
        } else if run.progress.processed < run.item_count {
            "清理已停止"
        } else if run.progress.failed > 0 {
            "清理完成，部分项目失败"
        } else {
            "清理完成"
        }
    } else if run.cancel.is_cancelled() {
        "正在停止清理…"
    } else if run.progress.processed == run.item_count {
        "正在整理清理结果…"
    } else if run.progress.current_path.is_none() && run.progress.processed == 0 {
        "正在准备清理…"
    } else {
        "正在清理…"
    }
}

fn cleanup_run_message(run: &CleanupRunState) -> String {
    format!(
        "{} 已处理 {}/{} 项，已清理空间 {}，跳过 {}，失败 {}。详情见可清理项页面。",
        cleanup_phase_text(run),
        run.progress.processed,
        run.item_count,
        format_bytes(run.progress.released_bytes),
        run.progress.skipped,
        run.progress.failed
    )
}

fn scan_cleanup_summary(cleanup: &crate::state::CleanupViewState) -> String {
    format!(
        "已选 {} 项 · 预计可释放 {}",
        cleanup.selected_count,
        format_bytes(cleanup.estimated_bytes)
    )
}

impl CleanerApp {
    fn render_content(&mut self, cx: &Context<Self>) -> AnyElement {
        match self.state.selected_view {
            ViewId::Home => self.render_dashboard(cx).into_any_element(),
            ViewId::Dashboard => self.render_dashboard(cx).into_any_element(),
            ViewId::SpaceAnalysis => self.render_space_analysis(cx).into_any_element(),
            ViewId::LargeFiles => self.render_large_files_view(cx).into_any_element(),
            ViewId::Cleanup => self.render_cleanup(cx).into_any_element(),
            ViewId::Recovery => self.render_recovery(cx).into_any_element(),
            ViewId::Settings => self.render_settings(cx).into_any_element(),
        }
    }

    fn render_dashboard(&self, cx: &Context<Self>) -> impl IntoElement {
        div()
            .flex_1()
            .w_full()
            .min_w(px(0.0))
            .flex()
            .flex_col()
            .bg(rgb(0xffffff))
            .p_1()
            .gap_1()
            .child(self.render_category_sections("垃圾清理", cx))
    }

    fn render_space_analysis(&mut self, cx: &Context<Self>) -> impl IntoElement + use<> {
        let analysis_path = self.space_analysis_path.clone();
        let cleanup_rows = analysis_cleanup_rows(
            &self.state.cleanup.categories,
            &self.state.cleanup.selected_categories,
            self.active_analysis_category.as_deref(),
        );
        let active_category = self
            .active_analysis_category
            .clone()
            .filter(|key| cleanup_rows.iter().any(|row| row.key == *key));
        let active_row = active_category
            .as_deref()
            .and_then(|key| cleanup_rows.iter().find(|row| row.key == key));
        let active_category_name = active_row
            .map(|row| row.name.clone())
            .unwrap_or_else(|| "全部已选".to_owned());
        let active_category_selected = active_row.is_some_and(|row| row.selected);
        let selected_items =
            self.analysis_selected_cleanup_items(&analysis_path, active_category.as_deref());
        let directory_impacts =
            self.analysis_selected_directory_impacts(&analysis_path, active_category.as_deref());
        let can_go_parent = analysis_parent_path(&analysis_path).is_some();
        let cleanup_total = cleanup_rows
            .iter()
            .map(|category| category.estimated_bytes)
            .sum::<u64>();
        let selected_cleanup = self.state.cleanup.estimated_bytes;
        let directory_total = directory_impacts
            .iter()
            .map(|directory| directory.bytes)
            .sum::<u64>();
        let selected_in_path = self
            .selected_directories
            .iter()
            .filter(|directory| path_is_within(&analysis_path, directory))
            .count();
        let selected_scope_label = if selected_in_path == 0 {
            "全部已选来源".to_owned()
        } else {
            format!("{selected_in_path} 个目录")
        };
        let impact_panel_height = self.analysis_impact_height.max(0.0);
        let directory_sizes = if self
            .directory_row_sizes_cache
            .as_ref()
            .is_some_and(|sizes| sizes.len() == directory_impacts.len())
        {
            self.directory_row_sizes_cache.as_ref().unwrap().clone()
        } else {
            let sizes = Rc::new(vec![
                size(px(0.0), px(ANALYSIS_DIRECTORY_IMPACT_ROW_HEIGHT));
                directory_impacts.len()
            ]);
            self.directory_row_sizes_cache = Some(Rc::clone(&sizes));
            sizes
        };
        let impact_rows = Rc::new(directory_impacts);
        let impact_rows_for_view = Rc::clone(&impact_rows);
        let directory_list = v_virtual_list(
            cx.entity().clone(),
            "analysis-directory-impact-list",
            directory_sizes,
            move |_app, range, _, cx| {
                range
                    .map(|index| {
                        let row = impact_rows_for_view[index].clone();
                        let path = row.path.clone();
                        let selected = row.selected;
                        div()
                            .id(("directory-impact-row", index))
                            .w_full()
                            .min_w(px(0.0))
                            .h(px(ANALYSIS_DIRECTORY_IMPACT_ROW_HEIGHT))
                            .min_h(px(ANALYSIS_DIRECTORY_IMPACT_ROW_HEIGHT))
                            .flex()
                            .items_center()
                            .gap_1()
                            .child(
                                div()
                                    .id(("directory-impact-select", index))
                                    .tab_index(0)
                                    .w(px(62.0))
                                    .h(px(28.0))
                                    .flex_shrink_0()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .rounded_sm()
                                    .border_1()
                                    .border_color(if selected {
                                        rgb(0x16835b)
                                    } else {
                                        rgb(0xd6dde5)
                                    })
                                    .bg(if selected {
                                        rgb(0xe7f7ef)
                                    } else {
                                        rgb(0xffffff)
                                    })
                                    .text_xs()
                                    .text_color(if selected {
                                        rgb(0x16835b)
                                    } else {
                                        rgb(0x66727e)
                                    })
                                    .on_click(cx.listener({
                                        let path = path.clone();
                                        move |app, _, _, cx| {
                                            cx.stop_propagation();
                                            app.dispatch(
                                                UiCommand::ToggleDirectorySelection(path.clone()),
                                                cx,
                                            )
                                        }
                                    }))
                                    .child(if selected { "已限定" } else { "限定" }),
                            )
                            .child(
                                div()
                                    .id(("directory-impact-label", index))
                                    .flex_1()
                                    .min_w(px(0.0))
                                    .tab_index(0)
                                    .on_click(cx.listener({
                                        let path = path.clone();
                                        move |app, _, _, cx| {
                                            app.dispatch(
                                                UiCommand::OpenAnalysisDirectory(path.clone()),
                                                cx,
                                            )
                                        }
                                    }))
                                    .child(analysis_directory_impact_row(row, directory_total)),
                            )
                    })
                    .collect::<Vec<_>>()
            },
        )
        .track_scroll(&self.directory_scroll_handle)
        .flex_1()
        .min_h(px(0.0));
        let impact_count = impact_rows.len();
        let selected_item_total_count = selected_items.total_count;
        let selected_item_total_bytes = selected_items.total_bytes;
        let selected_item_rows = selected_items.rows;
        let selected_item_count = selected_item_rows.len();
        let selected_item_label = if selected_item_count < selected_item_total_count {
            format!(
                "显示前 {} / {} 项 · {}",
                selected_item_count,
                selected_item_total_count,
                format_bytes(selected_item_total_bytes)
            )
        } else {
            format!(
                "{} 项 · {}",
                selected_item_total_count,
                format_bytes(selected_item_total_bytes)
            )
        };
        let selected_item_sizes = Rc::new(vec![
            size(px(0.0), px(ANALYSIS_CLEANUP_ITEM_ROW_HEIGHT));
            selected_item_rows.len()
        ]);
        let selected_item_rows = Rc::new(selected_item_rows);
        let selected_item_rows_for_view = Rc::clone(&selected_item_rows);
        let cleanup_detail_scroll_handle = self.large_file_scroll_handle.clone();
        let cleanup_detail_list = v_virtual_list(
            cx.entity().clone(),
            "analysis-selected-cleanup-list",
            selected_item_sizes,
            move |_app, range, _, _cx| {
                range
                    .map(|index| {
                        analysis_cleanup_item_row(selected_item_rows_for_view[index].clone())
                    })
                    .collect::<Vec<_>>()
            },
        )
        .track_scroll(&cleanup_detail_scroll_handle)
        .flex_1()
        .min_h(px(0.0));
        div()
            .flex_1()
            .h_full()
            .w_full()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .w_full()
                    .flex_shrink_0()
                    .flex()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .truncate()
                            .text_xs()
                            .child(format!("当前目录：{analysis_path}")),
                    ),
            )
            .child(div().w_full().flex_shrink_0().flex().gap_1().children([
                metric_tile("来源占用", format_bytes(cleanup_total)),
                metric_tile("计划处理", format_bytes(selected_cleanup)),
                metric_tile("当前查看", active_category_name.clone()),
                metric_tile("清理范围", selected_scope_label),
            ]))
            .child(
                div()
                    .flex_1()
                    .min_h(px(0.0))
                    .min_w(px(0.0))
                    .flex()
                    .gap_2()
                    .child(
                        div()
                            .w(px(self.analysis_sources_width.max(0.0)))
                            .flex_shrink_0()
                            .min_h(px(0.0))
                            .overflow_hidden()
                            .flex()
                            .child(self.render_analysis_cleanup_panel(cleanup_rows, cx)),
                    )
                    .child(
                        div()
                            .id("analysis-horizontal-resizer")
                            .w(px(10.0))
                            .flex_shrink_0()
                            .cursor_ew_resize()
                            .flex()
                            .items_center()
                            .justify_center()
                            .on_mouse_down(
                                MouseButton::Left,
                                cx.listener(
                                    move |app, event: &MouseDownEvent, _, cx| {
                                        app.analysis_horizontal_split_drag =
                                            Some(AnalysisHorizontalSplitDragState {
                                                start_x: event.position.x,
                                                start_sources_width: app.analysis_sources_width,
                                            });
                                        cx.stop_propagation();
                                    },
                                ),
                            )
                            .on_drag(AnalysisHorizontalSplitDrag, |drag, _, _, cx| {
                                cx.stop_propagation();
                                cx.new(|_| drag.clone())
                            })
                            .on_drag_move(cx.listener(
                                move |app,
                                      event: &DragMoveEvent<AnalysisHorizontalSplitDrag>,
                                      _,
                                      cx| {
                                    let Some(drag) = app.analysis_horizontal_split_drag else {
                                        return;
                                    };
                                    let delta_x =
                                        f32::from(event.event.position.x - drag.start_x);
                                    app.analysis_sources_width =
                                        (drag.start_sources_width + delta_x).max(0.0);
                                    cx.notify();
                                },
                            ))
                            .child(
                                div()
                                    .w(px(2.0))
                                    .h(px(44.0))
                                    .rounded_sm()
                                    .bg(rgb(0xcbd5e1)),
                            ),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .min_h(px(0.0))
                            .overflow_hidden()
                            .flex()
                            .flex_col()
                            .gap_1()
                            .child(
                                div().flex_1().min_h(px(0.0)).min_w(px(0.0)).flex().child(
                                    div()
                                        .w_full()
                                        .h_full()
                                        .min_h(px(0.0))
                                        .min_w(px(0.0))
                                        .flex()
                                        .flex_col()
                                        .bg(rgb(0xffffff))
                                        .rounded_md()
                                        .border_1()
                                        .border_color(rgb(0xe5e7eb))
                                        .p_1()
                                        .gap_1()
                                        .child(
                                            div()
                                                .flex()
                                                .justify_between()
                                                .items_center()
                                                .child(
                                                    div()
                                                        .text_xs()
                                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                                        .child(format!(
                                                            "来源明细 · {active_category_name}"
                                                        )),
                                                )
                                                .child(
                                                    div()
                                                        .flex()
                                                        .items_center()
                                                        .gap_1()
                                                        .child(
                                                            div()
                                                                .px_1()
                                                                .py(gpui::px(1.0))
                                                                .rounded_sm()
                                                                .bg(
                                                                    if active_category.is_none()
                                                                        || active_category_selected
                                                                    {
                                                                        rgb(0xe7f7ef)
                                                                    } else {
                                                                        rgb(0xf3f4f6)
                                                                    },
                                                                )
                                                                .text_xs()
                                                                .text_color(
                                                                    if active_category.is_none()
                                                                        || active_category_selected
                                                                    {
                                                                        rgb(0x16835b)
                                                                    } else {
                                                                        rgb(0x66727e)
                                                                    },
                                                                )
                                                                .child(
                                                                    if active_category.is_none()
                                                                        || active_category_selected
                                                                    {
                                                                        "已纳入清理"
                                                                    } else {
                                                                        "仅查看"
                                                                    },
                                                                ),
                                                        )
                                                        .child(
                                                            div()
                                                                .text_xs()
                                                                .text_color(rgb(0x66727e))
                                                                .child(selected_item_label),
                                                        ),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .flex()
                                                .flex_1()
                                                .min_h(px(0.0))
                                                .min_w(px(0.0))
                                                .when(selected_item_total_count == 0, |element| {
                                                    element.child(
                                                        div()
                                                            .flex_1()
                                                            .text_xs()
                                                            .text_color(rgb(0x66727e))
                                                            .child("当前选择没有命中的清理路径"),
                                                    )
                                                })
                                                .when(selected_item_total_count > 0, |element| {
                                                    element
                                                        .child(
                                                            div()
                                                                .flex_1()
                                                                .min_h(px(0.0))
                                                                .min_w(px(0.0))
                                                                .child(cleanup_detail_list),
                                                        )
                                                        .child(Scrollbar::vertical(
                                                            &self.large_file_scroll_state,
                                                            &cleanup_detail_scroll_handle,
                                                        ))
                                                }),
                                        ),
                                ),
                            )
                            .child(
                                div()
                                    .id("analysis-split-resizer")
                                    .h(px(10.0))
                                    .min_h(px(10.0))
                                    .w_full()
                                    .flex_shrink_0()
                                    .cursor_ns_resize()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(
                                            move |app, event: &MouseDownEvent, _, cx| {
                                                app.analysis_split_drag =
                                                    Some(AnalysisSplitDragState {
                                                        start_y: event.position.y,
                                                        start_impact_height: app
                                                            .analysis_impact_height,
                                                    });
                                                cx.stop_propagation();
                                            },
                                        ),
                                    )
                                    .on_drag(AnalysisSplitDrag, |drag, _, _, cx| {
                                        cx.stop_propagation();
                                        cx.new(|_| drag.clone())
                                    })
                                    .on_drag_move(cx.listener(
                                        move |app,
                                              event: &DragMoveEvent<AnalysisSplitDrag>,
                                              _,
                                              cx| {
                                            let Some(drag) = app.analysis_split_drag else {
                                                return;
                                            };
                                            let delta_y =
                                                f32::from(event.event.position.y - drag.start_y);
                                            app.analysis_impact_height =
                                                (drag.start_impact_height - delta_y).max(0.0);
                                            cx.notify();
                                        },
                                    ))
                                    .child(
                                        div()
                                            .h(px(2.0))
                                            .w(px(44.0))
                                            .rounded_sm()
                                            .bg(rgb(0xcbd5e1)),
                                    ),
                            )
                            .child(
                                div()
                                    .h(px(impact_panel_height))
                                    .min_w(px(0.0))
                                    .flex()
                                    .flex_col()
                                    .bg(rgb(0xffffff))
                                    .rounded_md()
                                    .border_1()
                                    .border_color(rgb(0xe5e7eb))
                                    .p_1()
                                    .gap_1()
                                    .child(
                                        div()
                                            .flex_shrink_0()
                                            .flex()
                                            .justify_between()
                                            .items_center()
                                            .child(
                                                div()
                                                    .flex()
                                                    .flex_col()
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                                            .child("目录影响与清理范围"),
                                                    )
                                                    .child(
                                                        div()
                                                            .text_xs()
                                                            .text_color(rgb(0x66727e))
                                                            .child("点目录名称下钻；用返回按钮逐级回退"),
                                                    ),
                                            )
                                            .child(
                                                div().flex().items_center().gap_1().child(
                                                    div()
                                                        .id("analysis-parent")
                                                        .tab_index(0)
                                                        .px_1()
                                                        .py(gpui::px(2.0))
                                                        .rounded_sm()
                                                        .border_1()
                                                        .border_color(if can_go_parent {
                                                            rgb(0xd6dde5)
                                                        } else {
                                                            rgb(0xe5e7eb)
                                                        })
                                                        .bg(if can_go_parent {
                                                            rgb(0xffffff)
                                                        } else {
                                                            rgb(0xf3f4f6)
                                                        })
                                                        .text_color(if can_go_parent {
                                                            rgb(0x2563eb)
                                                        } else {
                                                            rgb(0x9aa3ad)
                                                        })
                                                        .when(!can_go_parent, |element| {
                                                            element.opacity(0.45)
                                                        })
                                                        .on_click(cx.listener(|app, _, _, cx| {
                                                            app.dispatch(
                                                                UiCommand::OpenAnalysisParent,
                                                                cx,
                                                            )
                                                        }))
                                                        .child("← 返回上级"),
                                                )
                                                .child(
                                                    div()
                                                        .text_xs()
                                                        .text_color(rgb(0x66727e))
                                                        .child(format!(
                                                            "{} 个目录 · {}",
                                                            impact_count,
                                                            format_bytes(directory_total)
                                                        )),
                                                ),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .flex_shrink_0()
                                            .text_xs()
                                            .text_color(rgb(0x66727e))
                                            .truncate()
                                            .child(if selected_in_path == 0 {
                                                "未限定目录，清理会覆盖当前已选来源"
                                            } else {
                                                "已限定目录，清理只处理范围内命中的项目"
                                            }),
                                    )
                                    .child(
                                        div()
                                            .flex()
                                            .flex_1()
                                            .min_h(px(0.0))
                                            .min_w(px(0.0))
                                            .when(impact_count == 0, |element| {
                                                element.child(
                                                    div()
                                                        .flex_1()
                                                        .text_xs()
                                                        .text_color(rgb(0x66727e))
                                                        .child("当前来源没有可定位到目录的清理项"),
                                                )
                                            })
                                            .when(impact_count > 0, |element| {
                                                element
                                                    .child(
                                                        div()
                                                            .flex_1()
                                                            .min_h(px(0.0))
                                                            .min_w(px(0.0))
                                                            .child(directory_list),
                                                    )
                                                    .child(Scrollbar::vertical(
                                                        &self.directory_scroll_state,
                                                        &self.directory_scroll_handle,
                                                    ))
                                            }),
                                    ),
                            ),
                    ),
            )
    }

    fn render_analysis_cleanup_panel(
        &mut self,
        cleanup_rows: Vec<AnalysisCleanupRow>,
        cx: &Context<Self>,
    ) -> impl IntoElement + use<> {
        let rows = Rc::new(cleanup_rows);
        let row_sizes = Rc::new(vec![
            size(px(0.0), px(ANALYSIS_CLEANUP_ROW_HEIGHT));
            rows.len()
        ]);
        let selected_count = rows.iter().filter(|category| category.selected).count();
        let rows_for_view = Rc::clone(&rows);
        let scroll_handle = self.analysis_cleanup_scroll_handle.clone();
        let cleanup_list = v_virtual_list(
            cx.entity().clone(),
            "analysis-cleanup-list",
            row_sizes,
            move |_app, range, _, cx| {
                range
                    .map(|index| {
                        let row = rows_for_view[index].clone();
                        let inspect_key = row.key.clone();
                        let toggle_key = row.key.clone();
                        let selected = row.selected;
                        let active = row.active;
                        let selectable = row.executable_bytes.is_some();
                        div()
                            .id(("analysis-cleanup-row", index))
                            .w_full()
                            .h(px(ANALYSIS_CLEANUP_ROW_HEIGHT))
                            .min_h(px(ANALYSIS_CLEANUP_ROW_HEIGHT))
                            .min_w(px(0.0))
                            .flex()
                            .items_center()
                            .tab_index(0)
                            .on_click(cx.listener(move |app, _, _, cx| {
                                app.dispatch(
                                    UiCommand::InspectCleanupCategory(inspect_key.clone()),
                                    cx,
                                )
                            }))
                            .when(active, |element| element.bg(rgb(0xeaf6ff)))
                            .when(selected && !active, |element| element.bg(rgb(0xf7fff3)))
                            .child(
                                div()
                                    .id(("analysis-cleanup-select", index))
                                    .w(px(28.0))
                                    .h_full()
                                    .flex_shrink_0()
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .tab_index(0)
                                    .on_click(cx.listener(move |app, _, _, cx| {
                                        cx.stop_propagation();
                                        app.dispatch(
                                            UiCommand::ToggleCleanupCategory(toggle_key.clone()),
                                            cx,
                                        )
                                    }))
                                    .child(if selectable {
                                        if selected { "☑" } else { "☐" }
                                    } else {
                                        "—"
                                    }),
                            )
                            .child(analysis_cleanup_row(row))
                    })
                    .collect::<Vec<_>>()
            },
        )
        .track_scroll(&scroll_handle)
        .flex_1()
        .min_h(px(0.0));

        div()
            .w_full()
            .h_full()
            .min_h(px(0.0))
            .min_w(px(0.0))
            .flex()
            .flex_col()
            .bg(rgb(0xffffff))
            .rounded_md()
            .border_1()
            .border_color(rgb(0xe5e7eb))
            .p_1()
            .gap_1()
            .child(
                div()
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_xs()
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .child("可清理来源"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(0x66727e))
                                    .child("点名称查看明细，点限定选择清理范围"),
                            ),
                    )
                    .child(div().text_xs().text_color(rgb(0x66727e)).child(format!(
                        "已选 {} / {}",
                        selected_count,
                        rows.len()
                    ))),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h(px(0.0))
                    .min_w(px(0.0))
                    .when(rows.is_empty(), |element| {
                        element.child(div().flex_1().text_xs().text_color(rgb(0x66727e)).child(
                            match self.state.scan.status {
                                ScanViewStatus::Scanning | ScanViewStatus::Paused => {
                                    "扫描中，清理来源统计中…"
                                }
                                ScanViewStatus::Complete | ScanViewStatus::Partial => {
                                    "未发现占用空间的清理项"
                                }
                                _ => "扫描完成后显示清理来源",
                            },
                        ))
                    })
                    .when(!rows.is_empty(), |element| {
                        element
                            .child(
                                div()
                                    .flex_1()
                                    .min_h(px(0.0))
                                    .min_w(px(0.0))
                                    .child(cleanup_list),
                            )
                            .child(Scrollbar::vertical(
                                &self.analysis_cleanup_scroll_state,
                                &scroll_handle,
                            ))
                    }),
            )
    }

    fn render_large_files_view(&mut self, cx: &Context<Self>) -> impl IntoElement + use<> {
        div()
            .flex_1()
            .h_full()
            .min_h(px(0.0))
            .w_full()
            .min_w(px(0.0))
            .flex()
            .flex_col()
            .gap_2()
            .child(self.render_large_file_panel(cx))
    }

    fn render_large_file_panel(&mut self, cx: &Context<Self>) -> impl IntoElement + use<> {
        let full_height = self.state.selected_view == ViewId::LargeFiles;
        let large_files = self.large_file_rows();
        let selected_large_file_count = large_files.iter().filter(|file| file.selected).count();
        let selected_large_file_bytes = large_files
            .iter()
            .filter(|file| file.selected)
            .map(|file| file.size_bytes)
            .sum::<u64>();
        let large_file_total_bytes = large_files.iter().map(|file| file.size_bytes).sum::<u64>();
        let row_sizes = if self
            .large_file_row_sizes_cache
            .as_ref()
            .is_some_and(|sizes| sizes.len() == large_files.len())
        {
            self.large_file_row_sizes_cache.as_ref().unwrap().clone()
        } else {
            let sizes = Rc::new(vec![
                size(px(0.0), px(LARGE_FILE_ROW_HEIGHT));
                large_files.len()
            ]);
            self.large_file_row_sizes_cache = Some(Rc::clone(&sizes));
            sizes
        };
        let large_file_count = large_files.len();
        let rows_for_view = Rc::clone(&large_files);
        let scroll_handle = self.large_file_scroll_handle.clone();
        let file_list = v_virtual_list(
            cx.entity().clone(),
            "large-file-list",
            row_sizes,
            move |_app, range, _, cx| {
                range
                    .map(|index| {
                        let row = rows_for_view[index].clone();
                        let identity = row.identity;
                        div()
                            .id(("large-file-row", index))
                            .w_full()
                            .h(px(LARGE_FILE_ROW_HEIGHT))
                            .min_h(px(LARGE_FILE_ROW_HEIGHT))
                            .tab_index(0)
                            .on_click(cx.listener(move |app, _, _, cx| {
                                app.dispatch(UiCommand::ToggleLargeFileSelection(identity), cx)
                            }))
                            .child(crate::components::large_file_list::row(row))
                    })
                    .collect::<Vec<_>>()
            },
        )
        .track_scroll(&scroll_handle)
        .h_full();
        div()
            .w_full()
            .min_w(px(0.0))
            .flex()
            .flex_col()
            .when(full_height, |element| {
                element.flex_1().h(px(450.0)).min_h(px(450.0))
            })
            .bg(rgb(0xffffff))
            .rounded_md()
            .border_1()
            .border_color(rgb(0xe5e7eb))
            .p_1()
            .gap_1()
            .child(
                div()
                    .w_full()
                    .min_w(px(0.0))
                    .flex()
                    .justify_between()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .truncate()
                            .child("大文件专清（仅显示可清理候选）"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .flex_shrink_0()
                            .child(div().w(px(220.0)).min_w(px(0.0)).truncate().child(format!(
                                "{} 项 · {} | 已选 {} 项 · {}",
                                large_file_count,
                                format_bytes(large_file_total_bytes),
                                selected_large_file_count,
                                format_bytes(selected_large_file_bytes)
                            )))
                            .when(selected_large_file_count > 0, |element| {
                                element.child(
                                    div()
                                        .id("delete-selected-large-files")
                                        .tab_index(0)
                                        .px_1()
                                        .py(gpui::px(2.0))
                                        .bg(rgb(0xb42318))
                                        .text_color(rgb(0xffffff))
                                        .on_click(cx.listener(|app, _, _, cx| {
                                            app.dispatch(
                                                UiCommand::ExecuteCleanup(domain::PlanId(2)),
                                                cx,
                                            )
                                        }))
                                        .child("删除选中"),
                                )
                            }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .h_full()
                    .min_h(px(0.0))
                    .min_w(px(0.0))
                    .when(full_height, |element| element)
                    .when(!full_height, |element| {
                        element.h(px(160.0)).min_h(px(160.0))
                    })
                    .child(
                        div()
                            .flex_1()
                            .h_full()
                            .min_h(px(0.0))
                            .min_w(px(0.0))
                            .child(file_list),
                    )
                    .child(Scrollbar::vertical(
                        &self.large_file_scroll_state,
                        &scroll_handle,
                    )),
            )
    }

    fn render_cleanup_process(&self, cx: &Context<Self>) -> impl IntoElement {
        let run = self.cleanup_run.as_ref().expect("cleanup process state");
        let progress = &run.progress;
        let ratio = (progress.processed as f32 / run.item_count.max(1) as f32).clamp(0.0, 1.0);
        let elapsed = run
            .finished_at
            .unwrap_or_else(Instant::now)
            .duration_since(run.started_at);
        let current = run.error.clone().unwrap_or_else(|| {
            progress.current_path.clone().unwrap_or_else(|| {
                if run.finished {
                    "本次结果已保留。可重新扫描核实剩余可清理空间。".into()
                } else if progress.processed == run.item_count {
                    "正在保存清理结果和恢复记录…".into()
                } else if progress.processed > 0 {
                    "正在继续处理下一项…".into()
                } else {
                    "正在准备清理，复核文件和恢复记录…".into()
                }
            })
        });
        let details = run
            .issues
            .iter()
            .map(|item| {
                let reason = match &item.result {
                    application::CleanupItemResult::SkippedLocked => "文件正在使用，已跳过".into(),
                    application::CleanupItemResult::SkippedPermission => "权限不足，已跳过".into(),
                    application::CleanupItemResult::SkippedChanged => {
                        "文件已变化或不满足清理条件，已跳过".into()
                    }
                    application::CleanupItemResult::SkippedForbidden => {
                        "受保护的路径，已跳过".into()
                    }
                    application::CleanupItemResult::Failed(error) => {
                        format!("处理失败：{error}")
                    }
                    _ => String::new(),
                };
                div()
                    .w_full()
                    .min_w(px(0.0))
                    .py_2()
                    .border_b_1()
                    .border_color(rgb(0xe5e7eb))
                    .child(div().truncate().child(item.path.clone()))
                    .child(div().truncate().text_color(rgb(0x9a3412)).child(reason))
            })
            .collect::<Vec<_>>();
        div()
            .id("cleanup-process")
            .flex()
            .flex_col()
            .size_full()
            .min_h(px(0.0))
            .min_w(px(0.0))
            .p_4()
            .gap_3()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(gpui::FontWeight::BOLD)
                            .child(cleanup_phase_text(run)),
                    )
                    .child(
                        div()
                            .text_color(rgb(0x667085))
                            .child(format!("耗时 {} 秒", elapsed.as_secs())),
                    ),
            )
            .child(div().text_color(rgb(0x667085)).child(format!(
                "本次计划 {} 项 · 预计释放 {} · 进度按项目数量计算",
                run.item_count,
                format_bytes(run.estimated_bytes)
            )))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_sm().child(format!(
                        "已处理 {} / {} 项",
                        progress.processed, run.item_count
                    )))
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(format!("{:.0}%", ratio * 100.0)),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .h(px(8.0))
                    .flex_shrink_0()
                    .rounded_md()
                    .bg(rgb(0xe5e7eb))
                    .overflow_hidden()
                    .child(div().h_full().w(gpui::relative(ratio)).bg(rgb(0x16835b))),
            )
            .child(
                div()
                    .flex()
                    .justify_between()
                    .gap_2()
                    .p_3()
                    .rounded_md()
                    .bg(rgb(0xf0f9e9))
                    .child(div().child(format!(
                            "成功 {} 项",
                            (progress.processed as u64)
                                .saturating_sub(progress.skipped + progress.failed)
                        )))
                    .child(div().child(format!(
                        "已清理空间 {}",
                        format_bytes(progress.released_bytes)
                    )))
                    .child(div().child(format!("跳过 {} 项", progress.skipped)))
                    .child(
                        div()
                            .text_color(rgb(0x9a3412))
                            .child(format!("失败 {} 项", progress.failed)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w(px(0.0))
                    .w_full()
                    .p_2()
                    .border_1()
                    .rounded_md()
                    .border_color(rgb(0xe5e7eb))
                    .child(div().text_color(rgb(0x667085)).child(if run.finished {
                        "结果"
                    } else {
                        "当前处理"
                    }))
                    .child(div().w_full().line_clamp(2).child(current)),
            )
            .when(run.finished, |element| {
                let disk_change = run
                    .disk_available_change
                    .map(|change| {
                        format!(
                            "{}{}",
                            if change < 0 { "−" } else { "+" },
                            format_bytes(change.unsigned_abs() as u64)
                        )
                    })
                    .unwrap_or_else(|| "读取失败".into());
                element.child(div().text_xs().text_color(rgb(0x667085)).child(format!(
                    "C 盘可用空间变化 {disk_change}（包含其他程序的影响）"
                )))
            })
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(div().text_color(rgb(0x667085)).child(if run.finished {
                        "已直接删除成功清理的垃圾；跳过或失败的项目可查看原因后重新扫描。"
                    } else if run.cancel.is_cancelled() {
                        "停止请求已收到，等待当前文件安全处理完毕。"
                    } else {
                        "清理在后台执行，可切换页面查看；停止将在当前文件处理完后生效。"
                    }))
                    .when(!run.finished && !run.cancel.is_cancelled(), |el| {
                        el.child(
                            div()
                                .id("stop-cleanup")
                                .tab_index(0)
                                .px_3()
                                .py_2()
                                .flex_shrink_0()
                                .border_1()
                                .rounded_md()
                                .border_color(rgb(0xd1d5db))
                                .cursor_pointer()
                                .on_click(cx.listener(|app, _, _, cx| {
                                    app.dispatch(UiCommand::StopCleanup, cx)
                                }))
                                .child("停止清理"),
                        )
                    })
                    .when(run.finished, |el| {
                        el.child(
                            div()
                                .id("rescan-after-cleanup")
                                .tab_index(0)
                                .px_3()
                                .py_2()
                                .flex_shrink_0()
                                .rounded_md()
                                .bg(rgb(0x16835b))
                                .text_color(rgb(0xffffff))
                                .cursor_pointer()
                                .on_click(cx.listener(|app, _, _, cx| {
                                    app.dispatch(UiCommand::StartScan, cx)
                                }))
                                .child("重新扫描"),
                        )
                    }),
            )
            .child(
                div()
                    .id("cleanup-result-details")
                    .flex_1()
                    .min_h(px(0.0))
                    .min_w(px(0.0))
                    .scrollable(Axis::Vertical)
                    .when(!details.is_empty(), |el| {
                        el.child(
                            div()
                                .font_weight(gpui::FontWeight::SEMIBOLD)
                                .child("跳过与失败明细（最多显示 100 项）"),
                        )
                        .children(details)
                    }),
            )
    }

    fn render_cleanup(&self, cx: &Context<Self>) -> impl IntoElement {
        let summary = self.cleanup_result.as_ref().map(|result| {
            crate::components::result_summary::ResultSummary {
                released_bytes: result.released_bytes,
                skipped: result.skipped,
                failed: result.failed,
                recoverable: result.recoverable,
            }
        });
        div()
            .flex_1()
            .flex()
            .flex_col()
            .bg(rgb(0xffffff))
            .rounded_md()
            .border_1()
            .border_color(rgb(0xe5e7eb))
            .p_2()
            .gap_1()
            .child(
                div()
                    .text_sm()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("清理预览与二次确认"),
            )
            .child(div().text_xs().text_color(rgb(0x66727e)).child(format!(
                "已选 {} 项，预计释放 {}，确认后直接删除",
                self.state.cleanup.selected_count,
                format_bytes(self.state.cleanup.estimated_bytes)
            )))
            .child(self.render_category_sections("垃圾清理", cx))
            .child(div().h(px(12.0)).min_h(px(12.0)))
            .when_some(summary, |element, summary| {
                element.child(crate::components::result_summary::render(&summary))
            })
    }

    fn render_category(
        &self,
        category: &crate::state::CleanupCategoryState,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let key = category.key.clone();
        let selected = self.state.cleanup.selected_categories.contains(&key);
        div()
            .id(gpui::SharedString::new(format!("cleanup-category:{key}")))
            .tab_index(0)
            .on_click(cx.listener(move |app, _, _, cx| {
                app.dispatch(UiCommand::ToggleCleanupCategory(key.clone()), cx)
            }))
            .child(crate::components::cleanup_list::render(
                crate::components::cleanup_list::CleanupCategory {
                    name: category.name.clone(),
                    estimated_bytes: category.estimated_bytes,
                    executable_bytes: category.executable_bytes,
                    selected,
                },
            ))
    }

    fn render_category_sections(&self, risk_label: &'static str, cx: &Context<Self>) -> AnyElement {
        if self.state.cleanup.categories.is_empty() {
            let message = match self.state.scan.status {
                ScanViewStatus::Scanning | ScanViewStatus::Paused => "扫描中，分类大小统计中…",
                ScanViewStatus::Complete | ScanViewStatus::Partial => "未发现符合规则的清理项",
                _ => "扫描完成后显示分类大小",
            };
            return div()
                .text_xs()
                .text_color(rgb(0x66727e))
                .child(message)
                .into_any_element();
        }
        let mut sections = Vec::new();
        let mut group = None::<String>;
        let mut categories = Vec::new();
        let mut sorted = self.state.cleanup.categories.iter().collect::<Vec<_>>();
        sorted.sort_by_key(|category| {
            (
                category.group_order,
                category.display_order,
                category.name.clone(),
            )
        });
        for category in sorted {
            if group.as_deref() != Some(category.group.as_str()) {
                if !categories.is_empty() {
                    sections.push(self.render_category_group(
                        group.as_deref().unwrap_or("其他清理"),
                        &categories,
                        risk_label,
                        cx,
                    ));
                    categories.clear();
                }
                group = Some(category.group.clone());
            }
            categories.push(category);
        }
        if !categories.is_empty() {
            sections.push(self.render_category_group(
                group.as_deref().unwrap_or("其他清理"),
                &categories,
                risk_label,
                cx,
            ));
        }
        div()
            .flex()
            .flex_col()
            .w_full()
            .min_w(px(0.0))
            .gap_1()
            .children(sections)
            .into_any_element()
    }

    fn render_category_group(
        &self,
        group: &str,
        categories: &[&crate::state::CleanupCategoryState],
        risk_label: &'static str,
        cx: &Context<Self>,
    ) -> AnyElement {
        let total = categories
            .iter()
            .map(|category| category.executable_bytes.unwrap_or(0))
            .sum::<u64>();
        let selected = categories
            .iter()
            .filter(|category| {
                self.state
                    .cleanup
                    .selected_categories
                    .contains(&category.key)
            })
            .map(|category| category.executable_bytes.unwrap_or(0))
            .sum::<u64>();
        let selectable = categories
            .iter()
            .filter(|category| category.executable_bytes.is_some())
            .collect::<Vec<_>>();
        let all_selected = !selectable.is_empty()
            && selectable.iter().all(|category| {
                self.state
                    .cleanup
                    .selected_categories
                    .contains(&category.key)
            });
        let collapsed = self.collapsed_cleanup_groups.contains(group);
        let keys = categories
            .iter()
            .filter(|category| category.executable_bytes.is_some())
            .map(|category| category.key.clone())
            .collect::<Vec<_>>();
        let selection_group = group.to_owned();
        let title_group = group.to_owned();
        let arrow_group = group.to_owned();
        div()
            .flex()
            .flex_col()
            .w_full()
            .min_w(px(0.0))
            .gap_1()
            .child(
                div()
                    .w_full()
                    .flex()
                    .items_center()
                    .gap_1()
                    .py_1()
                    .border_b_1()
                    .border_color(rgb(0xe5e7eb))
                    .child(
                        div()
                            .id(gpui::SharedString::new(format!(
                                "cleanup-group-select:{selection_group}"
                            )))
                            .tab_index(0)
                            .on_click(cx.listener(move |app, _, _, cx| {
                                app.dispatch(
                                    UiCommand::ToggleCleanupGroupSelection(keys.clone()),
                                    cx,
                                )
                            }))
                            .child(if all_selected { "☑" } else { "☐" }),
                    )
                    .child(
                        div()
                            .id(gpui::SharedString::new(format!(
                                "cleanup-group-title:{title_group}"
                            )))
                            .tab_index(0)
                            .flex_1()
                            .min_w(px(0.0))
                            .flex()
                            .items_center()
                            .justify_between()
                            .on_click(cx.listener(move |app, _, _, cx| {
                                app.dispatch(UiCommand::ToggleCleanupGroup(title_group.clone()), cx)
                            }))
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(gpui::FontWeight::SEMIBOLD)
                                            .child(group.to_owned()),
                                    )
                                    .child(div().text_xs().text_color(rgb(0x66727e)).child(
                                        format!(
                                            "可处理 {}，已选 {}",
                                            format_bytes(total),
                                            format_bytes(selected)
                                        ),
                                    )),
                            )
                            .child(div().text_xs().text_color(rgb(0x66727e)).child(risk_label)),
                    )
                    .child(
                        div()
                            .id(gpui::SharedString::new(format!(
                                "cleanup-group-arrow:{arrow_group}"
                            )))
                            .tab_index(0)
                            .px_1()
                            .on_click(cx.listener(move |app, _, _, cx| {
                                app.dispatch(UiCommand::ToggleCleanupGroup(arrow_group.clone()), cx)
                            }))
                            .child(if collapsed { "⌄" } else { "⌃" }),
                    ),
            )
            .when(!collapsed, |element| {
                element.child(
                    div()
                        .w_full()
                        .min_w(px(0.0))
                        .flex()
                        .flex_wrap()
                        .gap_1()
                        .children(
                            categories
                                .iter()
                                .map(|category| self.render_category(category, cx)),
                        ),
                )
            })
            .into_any_element()
    }

    fn render_cleanup_confirmation(&self, cx: &Context<Self>) -> impl IntoElement {
        let (item_count, estimated_bytes) = self
            .cleanup_plan
            .as_ref()
            .map(|plan| (plan.items.len(), plan.estimated_bytes))
            .unwrap_or_default();
        let description = format!(
            "将清理 {item_count} 项，预计释放 {}。",
            format_bytes(estimated_bytes)
        );
        div()
            .absolute()
            .inset_0()
            .bg(cleanup_confirmation_backdrop())
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    .w(px(320.0))
                    .bg(rgb(0xffffff))
                    .rounded_md()
                    .border_1()
                    .border_color(rgb(0xd1d5db))
                    .p_3()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::BOLD)
                            .child("确认执行清理"),
                    )
                    .child(div().text_xs().child(description))
                    .child(div().text_xs().text_color(rgb(0x9a3412)).child(
                        cleanup_confirmation_warning(
                            self.cleanup_plan.as_ref().expect("cleanup plan"),
                        ),
                    ))
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_1()
                            .child(
                                div()
                                    .id("cancel-cleanup-confirmation")
                                    .tab_index(0)
                                    .px_2()
                                    .py_1()
                                    .border_1()
                                    .border_color(rgb(0xd1d5db))
                                    .on_click(cx.listener(|app, _, _, cx| {
                                        app.dispatch(UiCommand::CancelCleanup, cx)
                                    }))
                                    .child("取消"),
                            )
                            .child(
                                div()
                                    .id("confirm-cleanup-confirmation")
                                    .tab_index(0)
                                    .px_2()
                                    .py_1()
                                    .bg(rgb(0xb42318))
                                    .text_color(rgb(0xffffff))
                                    .on_click(cx.listener(|app, _, _, cx| {
                                        app.dispatch(UiCommand::ConfirmCleanup, cx)
                                    }))
                                    .child("确认清理"),
                            ),
                    ),
            )
    }

    fn render_recovery(&self, cx: &Context<Self>) -> impl IntoElement {
        div()
            .flex_1()
            .flex()
            .flex_col()
            .bg(rgb(0xffffff))
            .rounded_md()
            .border_1()
            .border_color(rgb(0xe5e7eb))
            .p_2()
            .gap_1()
            .child(
                div()
                    .text_sm()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("恢复记录"),
            )
            .child(div().text_xs().text_color(rgb(0x66727e)).child(format!(
                    "{} 项仍可恢复",
                    self.recovery_items
                        .iter()
                        .filter(|item| item.recoverable)
                        .count()
                )))
            .child(div().text_xs().text_color(rgb(0x66727e)).child(format!(
                "备份占用 {} / {}，可恢复 {} 项",
                format_bytes(self.recovery_quota.used_bytes),
                format_bytes(self.recovery_quota.max_bytes),
                self.recovery_quota.recoverable_items
            )))
            .child(div().text_xs().text_color(rgb(0xb42318)).child(format!(
                "损坏或无法校验的记录：{}",
                self.recovery_corrupt_records
            )))
            .child(
                div()
                    .id("purge-expired-recovery")
                    .tab_index(0)
                    .border_1()
                    .border_color(rgb(0xd1d5db))
                    .px_2()
                    .py_1()
                    .on_click(cx.listener(|app, _, _, cx| {
                        app.dispatch(UiCommand::PurgeExpiredRecovery, cx)
                    }))
                    .child("清理已恢复且过期备份"),
            )
            .children(self.recovery_items.iter().map(|item| {
                let id = item.id.0;
                div()
                    .id(("recovery", id))
                    .tab_index(0)
                    .px_2()
                    .py_1()
                    .border_b_1()
                    .border_color(rgb(0xf0f2f4))
                    .on_click(cx.listener(move |app, _, _, cx| {
                        app.dispatch(UiCommand::RestoreOperation(domain::OperationId(id)), cx)
                    }))
                    .child(format!(
                        "#{} · {} · {}",
                        id,
                        format_bytes(item.size),
                        if item.recoverable {
                            "点击恢复"
                        } else {
                            "已恢复"
                        }
                    ))
            }))
    }

    fn render_settings(&self, cx: &Context<Self>) -> impl IntoElement {
        div()
            .flex_1()
            .flex()
            .flex_col()
            .bg(rgb(0xffffff))
            .rounded_md()
            .border_1()
            .border_color(rgb(0xe5e7eb))
            .p_2()
            .gap_1()
            .child(
                div()
                    .text_sm()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("设置"),
            )
            .child("默认离线扫描：已启用")
            .child("垃圾清理：直接删除，选中的回收站一并清空")
            .child(
                div()
                    .id("toggle-auto-clean")
                    .tab_index(0)
                    .on_click(
                        cx.listener(|app, _, _, cx| app.dispatch(UiCommand::ToggleAutoClean, cx)),
                    )
                    .child(format!(
                        "自动清理：{}。所有清理均需预览和二次确认。",
                        if self.settings.auto_clean {
                            "已启用"
                        } else {
                            "已关闭"
                        }
                    )),
            )
    }

    fn analysis_selected_cleanup_items(
        &self,
        path: &str,
        category_filter: Option<&str>,
    ) -> AnalysisCleanupItems {
        let category_names = self
            .state
            .cleanup
            .categories
            .iter()
            .map(|category| (category.key.as_str(), category.name.as_str()))
            .collect::<HashMap<_, _>>();
        let mut rows = self
            .cleanup_plan
            .as_ref()
            .into_iter()
            .flat_map(|plan| plan.items.iter())
            .map(|item| &item.candidate)
            .filter(|candidate| {
                analysis_candidate_category_matches(
                    candidate.category.as_deref(),
                    &self.state.cleanup.selected_categories,
                    category_filter,
                ) && path_is_within(path, candidate.path.display())
            })
            .filter_map(|candidate| {
                let size_bytes = candidate.estimated_size?;
                if size_bytes == 0 {
                    return None;
                }
                let category = candidate.category.as_deref().unwrap_or("其他清理");
                Some(AnalysisCleanupItemRow {
                    path: candidate.path.display().to_owned(),
                    category: category_names
                        .get(category)
                        .copied()
                        .unwrap_or(category)
                        .to_owned(),
                    size_bytes,
                })
            })
            .collect::<Vec<_>>();
        rows.sort_unstable_by(|left, right| {
            right
                .size_bytes
                .cmp(&left.size_bytes)
                .then_with(|| left.path.cmp(&right.path))
        });
        let total_count = rows.len();
        let total_bytes = rows.iter().map(|row| row.size_bytes).sum::<u64>();
        rows.truncate(500);
        AnalysisCleanupItems {
            rows,
            total_count,
            total_bytes,
        }
    }

    fn analysis_selected_directory_impacts(
        &self,
        path: &str,
        category_filter: Option<&str>,
    ) -> Vec<AnalysisDirectoryImpactRow> {
        let mut grouped = HashMap::<String, (u64, u64)>::new();
        for candidate in self
            .cleanup_plan
            .as_ref()
            .into_iter()
            .flat_map(|plan| plan.items.iter())
            .map(|item| &item.candidate)
            .filter(|candidate| {
                analysis_candidate_category_matches(
                    candidate.category.as_deref(),
                    &self.state.cleanup.selected_categories,
                    category_filter,
                ) && path_is_within(path, candidate.path.display())
            })
        {
            let Some(size_bytes) = candidate.estimated_size else {
                continue;
            };
            if size_bytes == 0 {
                continue;
            }
            let bucket = analysis_bucket_path(path, candidate.path.display());
            let entry = grouped.entry(bucket).or_default();
            entry.0 = entry.0.saturating_add(1);
            entry.1 = entry.1.saturating_add(size_bytes);
        }
        let mut rows = grouped
            .into_iter()
            .map(|(path, (count, bytes))| AnalysisDirectoryImpactRow {
                selected: self.selected_directories.contains(&path),
                path,
                count,
                bytes,
            })
            .collect::<Vec<_>>();
        rows.sort_unstable_by(|left, right| {
            right
                .bytes
                .cmp(&left.bytes)
                .then_with(|| left.path.cmp(&right.path))
        });
        rows
    }

    fn large_file_rows(&mut self) -> Rc<Vec<crate::components::large_file_list::LargeFileRow>> {
        if let Some(rows) = &self.large_file_rows_cache {
            return Rc::clone(rows);
        }
        // Keep the expensive index walk and sort out of every UI repaint.
        self.ensure_large_file_records_cache();
        let rows = self
            .large_file_records_cache
            .as_ref()
            .into_iter()
            .flat_map(|files| files.iter())
            .filter(|file| !self.removed_large_files.contains(&file.identity()))
            .map(|file| crate::components::large_file_list::LargeFileRow {
                path: file.path().display().to_owned(),
                identity: file.identity(),
                size_bytes: display_file_size(file),
                selected: self.selected_large_files.contains(&file.identity()),
            })
            .collect();
        let rows = Rc::new(rows);
        self.large_file_rows_cache = Some(Rc::clone(&rows));
        rows
    }
}

fn cleanup_large_file_records(
    index: &ScanIndex,
    removed: &HashSet<FileIdentity>,
) -> Vec<domain::FileRecord> {
    let candidate_identities = index
        .cleanup_candidates()
        .iter()
        .filter_map(|candidate| candidate.file_identity)
        .collect::<HashSet<_>>();
    let candidate_paths = index
        .cleanup_candidates()
        .iter()
        .map(|candidate| candidate.path.display().to_owned())
        .collect::<HashSet<_>>();
    let mut files = index
        .files()
        .into_iter()
        .filter(|file| {
            !removed.contains(&file.identity())
                && display_file_size(file) >= LARGE_FILE_MIN_BYTES
                && (candidate_identities.contains(&file.identity())
                    || candidate_paths.contains(file.path().display()))
        })
        .collect::<Vec<_>>();
    files.sort_unstable_by(|left, right| {
        display_file_size(right)
            .cmp(&display_file_size(left))
            .then_with(|| left.path().display().cmp(right.path().display()))
    });
    files
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AnalysisCleanupRow {
    key: String,
    group: String,
    name: String,
    count: u64,
    estimated_bytes: u64,
    executable_bytes: Option<u64>,
    selected: bool,
    active: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct AnalysisCleanupItems {
    rows: Vec<AnalysisCleanupItemRow>,
    total_count: usize,
    total_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AnalysisCleanupItemRow {
    path: String,
    category: String,
    size_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct AnalysisDirectoryImpactRow {
    path: String,
    count: u64,
    bytes: u64,
    selected: bool,
}

fn analysis_cleanup_rows(
    categories: &[crate::state::CleanupCategoryState],
    selected: &std::collections::BTreeSet<String>,
    active: Option<&str>,
) -> Vec<AnalysisCleanupRow> {
    let mut rows = categories
        .iter()
        .map(|category| AnalysisCleanupRow {
            key: category.key.clone(),
            group: category.group.clone(),
            name: category.name.clone(),
            count: category.count,
            estimated_bytes: category.estimated_bytes,
            executable_bytes: category.executable_bytes,
            selected: selected.contains(&category.key),
            active: active == Some(category.key.as_str()),
        })
        .collect::<Vec<_>>();
    rows.sort_unstable_by(|left, right| {
        right
            .estimated_bytes
            .cmp(&left.estimated_bytes)
            .then_with(|| left.group.cmp(&right.group))
            .then_with(|| left.name.cmp(&right.name))
    });
    rows
}

fn analysis_cleanup_row(row: AnalysisCleanupRow) -> AnyElement {
    div()
        .w_full()
        .h(px(ANALYSIS_CLEANUP_ROW_HEIGHT))
        .min_h(px(ANALYSIS_CLEANUP_ROW_HEIGHT))
        .min_w(px(0.0))
        .flex()
        .items_center()
        .gap_1()
        .pr_1()
        .text_xs()
        .border_b_1()
        .border_color(rgb(0xf0f2f4))
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .flex()
                .flex_col()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_1()
                        .child(div().flex_1().min_w(px(0.0)).truncate().child(row.name))
                        .child(
                            div()
                                .w(px(72.0))
                                .flex_shrink_0()
                                .text_color(rgb(0x2563eb))
                                .child(format_bytes(row.estimated_bytes)),
                        ),
                )
                .child(div().text_color(rgb(0x66727e)).truncate().child(
                    match row.executable_bytes {
                        Some(0) if row.estimated_bytes > 0 => format!("{} · 仅统计占用", row.group),
                        Some(bytes) => format!("{} · 可清理 {}", row.group, format_bytes(bytes)),
                        None => format!("{} · {} 项 · 统计中", row.group, row.count),
                    },
                )),
        )
        .into_any_element()
}

fn analysis_cleanup_item_row(row: AnalysisCleanupItemRow) -> AnyElement {
    div()
        .w_full()
        .h(px(ANALYSIS_CLEANUP_ITEM_ROW_HEIGHT))
        .min_h(px(ANALYSIS_CLEANUP_ITEM_ROW_HEIGHT))
        .min_w(px(0.0))
        .flex()
        .items_center()
        .gap_1()
        .px_1()
        .text_xs()
        .border_b_1()
        .border_color(rgb(0xf0f2f4))
        .child(
            div()
                .flex_1()
                .min_w(px(0.0))
                .flex()
                .flex_col()
                .child(div().truncate().child(row.path))
                .child(
                    div()
                        .text_color(rgb(0x66727e))
                        .truncate()
                        .child(row.category),
                ),
        )
        .child(
            div()
                .w(px(72.0))
                .flex_shrink_0()
                .text_color(rgb(0x2563eb))
                .child(format_bytes(row.size_bytes)),
        )
        .into_any_element()
}

fn analysis_directory_impact_row(row: AnalysisDirectoryImpactRow, total_bytes: u64) -> AnyElement {
    let share = if total_bytes == 0 {
        0
    } else {
        row.bytes.saturating_mul(100) / total_bytes
    };
    div()
        .w_full()
        .h(px(ANALYSIS_DIRECTORY_IMPACT_ROW_HEIGHT))
        .min_h(px(ANALYSIS_DIRECTORY_IMPACT_ROW_HEIGHT))
        .min_w(px(0.0))
        .flex()
        .flex_col()
        .justify_center()
        .px_1()
        .text_xs()
        .border_b_1()
        .border_color(rgb(0xf0f2f4))
        .when(row.selected, |element| element.bg(rgb(0xf7fff3)))
        .child(
            div()
                .w_full()
                .flex()
                .items_center()
                .gap_1()
                .child(div().flex_1().min_w(px(0.0)).truncate().child(row.path))
                .child(
                    div()
                        .w(px(66.0))
                        .flex_shrink_0()
                        .text_color(rgb(0x2563eb))
                        .child(format_bytes(row.bytes)),
                ),
        )
        .child(
            div()
                .w_full()
                .flex()
                .items_center()
                .justify_between()
                .text_color(rgb(0x66727e))
                .child(format!("命中 {} 项 · 占当前 {}%", row.count, share))
                .child(if row.selected {
                    "清理限定已生效"
                } else {
                    "点击左侧按钮限定范围"
                }),
        )
        .into_any_element()
}

fn display_file_size(file: &domain::FileRecord) -> u64 {
    file.logical_size().max(file.allocated_size())
}

fn toggle_group_selection(selected: &mut std::collections::BTreeSet<String>, keys: &[String]) {
    if keys.iter().all(|key| selected.contains(key)) {
        selected.retain(|key| !keys.contains(key));
    } else {
        selected.extend(keys.iter().cloned());
    }
}

fn analysis_candidate_category_matches(
    category: Option<&str>,
    selected: &std::collections::BTreeSet<String>,
    active: Option<&str>,
) -> bool {
    let Some(category) = category else {
        return false;
    };
    analysis_category_matches(category, selected, active)
}

fn analysis_category_matches(
    category: &str,
    selected: &std::collections::BTreeSet<String>,
    active: Option<&str>,
) -> bool {
    if let Some(active) = active {
        category == active
    } else {
        selected.contains(category)
    }
}

fn path_is_within(parent: &str, child: &str) -> bool {
    let parent = parent
        .replace('/', "\\")
        .trim_end_matches(['\\', '/'])
        .to_ascii_lowercase();
    let child = child
        .replace('/', "\\")
        .trim_end_matches(['\\', '/'])
        .to_ascii_lowercase();
    child == parent
        || child
            .strip_prefix(&parent)
            .is_some_and(|rest| rest.starts_with('\\') || rest.starts_with('/'))
}

fn analysis_bucket_path(parent: &str, child: &str) -> String {
    let parent = parent
        .replace('/', "\\")
        .trim_end_matches(['\\', '/'])
        .to_owned();
    let child = child.replace('/', "\\");
    let prefix = if parent.ends_with(':') {
        format!("{parent}\\")
    } else {
        format!("{parent}\\")
    };
    let Some(rest) = child.strip_prefix(&prefix) else {
        return child;
    };
    let Some((first, _)) = rest.split_once('\\') else {
        return child;
    };
    format!("{prefix}{first}")
}

fn analysis_parent_path(path: &str) -> Option<String> {
    let normalized = path.replace('/', "\\");
    let trimmed = normalized.trim_end_matches('\\');
    let (prefix, _) = trimmed.rsplit_once('\\')?;
    if prefix.ends_with(':') {
        Some(format!("{prefix}\\"))
    } else {
        Some(prefix.to_owned())
    }
}

fn metric_tile(label: &'static str, value: String) -> AnyElement {
    div()
        .flex_1()
        .min_w(px(0.0))
        .bg(rgb(0xffffff))
        .rounded_md()
        .border_1()
        .border_color(rgb(0xe5e7eb))
        .p_1()
        .child(div().text_xs().text_color(rgb(0x66727e)).child(label))
        .child(
            div()
                .text_sm()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .truncate()
                .child(value),
        )
        .into_any_element()
}

fn cleanup_confirmation_backdrop() -> gpui::Rgba {
    rgba(0x00000066)
}

fn cleanup_item_completed(result: &application::CleanupItemResult) -> bool {
    matches!(
        result,
        application::CleanupItemResult::DeletedPermanently
            | application::CleanupItemResult::MovedToRecycleBin
            | application::CleanupItemResult::BackedUp
            | application::CleanupItemResult::RecycleBinEmptied
            | application::CleanupItemResult::RestartRequired
    )
}

fn cleanup_item_removes_scanned_bytes(result: &application::CleanupItemResult) -> bool {
    cleanup_item_completed(result)
        && !matches!(result, application::CleanupItemResult::RestartRequired)
}

fn cleanup_confirmation_warning(plan: &CleanupPlan) -> &'static str {
    let has_hibernation = plan
        .items
        .iter()
        .any(|item| item.candidate.rule_id.as_deref() == Some("system.hibernate"));
    let has_paging_files = plan
        .items
        .iter()
        .any(|item| item.candidate.rule_id.as_deref() == Some("system.virtual_memory"));
    if has_hibernation && has_paging_files {
        "将关闭系统休眠和快速启动，并禁用分页文件配置；虚拟内存文件需重启后才会释放。"
    } else if has_hibernation {
        "将关闭系统休眠和快速启动，并删除休眠文件。此操作需要管理员权限。"
    } else if has_paging_files {
        "将禁用分页文件配置；pagefile.sys 和 swapfile.sys 需重启后才会释放。"
    } else if plan
        .items
        .iter()
        .any(|item| item.candidate.rule_id.as_deref() == Some("recycle_bin"))
    {
        "直接删除选中的垃圾，并清空当前用户在所有本地磁盘上的回收站。此操作无法恢复。"
    } else {
        "直接删除选中的文件，不移入回收站。此操作无法恢复。"
    }
}

fn cleanup_result_detail(result: &CleanupResult) -> Option<String> {
    if let Some(item) = result
        .items
        .iter()
        .find(|item| item.result == application::CleanupItemResult::RestartRequired)
    {
        return Some(format!("{}：已更新虚拟内存配置，重启后释放空间", item.path));
    }
    let item = result
        .items
        .iter()
        .find(|item| !cleanup_item_completed(&item.result))?;
    let reason = match &item.result {
        application::CleanupItemResult::SkippedLocked => "文件正在使用中",
        application::CleanupItemResult::SkippedPermission => "没有访问权限",
        application::CleanupItemResult::SkippedChanged => "扫描后文件状态已变化，未执行删除",
        application::CleanupItemResult::SkippedForbidden => "该路径受保护，未执行删除",
        application::CleanupItemResult::Failed(error) => error,
        application::CleanupItemResult::DeletedPermanently
        | application::CleanupItemResult::MovedToRecycleBin
        | application::CleanupItemResult::BackedUp
        | application::CleanupItemResult::RecycleBinEmptied
        | application::CleanupItemResult::RestartRequired => return None,
    };
    Some(format!("{}：{}", item.path, reason))
}

fn nav_item(
    label: &'static str,
    view: ViewId,
    selected: ViewId,
    cx: &Context<CleanerApp>,
) -> impl IntoElement {
    let mut item = div().w_full().px_1().py_1().rounded_sm().child(label);
    if selected == view {
        item = item.bg(rgb(0xdff4d5));
    }
    item.id(label)
        .tab_index(0)
        .on_click(cx.listener(move |app, _, _, cx| app.dispatch(UiCommand::Navigate(view), cx)))
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{
        LARGE_FILE_MIN_BYTES, analysis_bucket_path, analysis_cleanup_rows, analysis_parent_path,
        cleanup_confirmation_backdrop, cleanup_confirmation_warning, cleanup_item_completed,
        cleanup_item_removes_scanned_bytes, cleanup_large_file_records, cleanup_result_detail,
        display_file_size, path_is_within, toggle_group_selection,
    };
    use crate::state::CleanupCategoryState;
    use domain::{
        CleanupCandidate, CleanupMode, Confidence, FileIdentity, FileRecord, RecoveryMode,
        RiskLevel, ScanIndex, normalize_path,
    };

    #[test]
    fn cleanup_summary_uses_the_selected_plan_estimate() {
        let cleanup = crate::state::CleanupViewState {
            selected_count: 3,
            estimated_bytes: 28 * 1024 * 1024 * 1024,
            ..Default::default()
        };
        assert_eq!(
            super::scan_cleanup_summary(&cleanup),
            "已选 3 项 · 预计可释放 28 GB"
        );
    }

    #[test]
    fn cleanup_status_distinguishes_stopping_stopped_and_failures() {
        let mut run = super::CleanupRunState {
            item_count: 10,
            estimated_bytes: 100,
            progress: application::CleanupProgress {
                total: 10,
                ..Default::default()
            },
            started_at: std::time::Instant::now(),
            finished_at: None,
            cancel: application::CancellationToken::default(),
            finished: false,
            error: None,
            issues: Vec::new(),
            disk_available_change: None,
        };
        assert_eq!(super::cleanup_phase_text(&run), "正在准备清理…");
        run.cancel.cancel();
        assert_eq!(super::cleanup_phase_text(&run), "正在停止清理…");
        run.finished = true;
        run.progress.processed = 1;
        assert_eq!(super::cleanup_phase_text(&run), "清理已停止");
        run.error = Some("worker failed".into());
        assert_eq!(super::cleanup_phase_text(&run), "清理未完成");
    }

    #[test]
    #[ignore = "interactive UI smoke test; only deletes disposable files created by this test"]
    fn cleanup_ui_with_disposable_files() {
        use gpui::{App, AppContext, Application, Bounds, WindowBounds, WindowOptions, px, size};
        super::CLEANUP_UI_SMOKE.store(true, std::sync::atomic::Ordering::Relaxed);
        let folder = std::env::temp_dir().join(format!("rusten-ui-smoke-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let mut candidates = Vec::new();
        for i in 0..400 {
            let path = folder.join(format!("disposable-{i:03}.tmp"));
            std::fs::write(&path, vec![0u8; 4096]).unwrap();
            let normalized = normalize_path(&path.to_string_lossy()).unwrap();
            let metadata = platform_windows::read_metadata(&normalized).unwrap();
            let index = ScanIndex::new(normalize_path(r"C:\").unwrap());
            let file = FileRecord::new(
                normalized,
                index.root_directory_id(),
                metadata.identity(),
                metadata.logical_size(),
                metadata.allocated_size(),
            )
            .with_facts(domain::FileFacts {
                last_write_time: metadata.last_write_time(),
                lock_state: domain::LockState::Available,
                ..Default::default()
            });
            candidates.push(crate::runtime::explicit_file_candidate(&file));
        }
        let plan = crate::runtime::prepare_cleanup_plan_with_explicit(&candidates);
        Application::new().run(move |cx: &mut App| {
            gpui_component::init(cx);
            cx.open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                        None,
                        size(px(922.0), px(521.0)),
                        cx,
                    ))),
                    ..Default::default()
                },
                |_, cx| {
                    cx.new(|_| {
                        let mut app = super::CleanerApp::new();
                        app.state.scan.status = crate::state::ScanViewStatus::Complete;
                        app.state.cleanup.categories = vec![
                            CleanupCategoryState {
                                key: "大文件".into(),
                                group: "测试清理".into(),
                                group_order: 1,
                                name: "测试临时文件".into(),
                                display_order: 1,
                                count: plan.items.len() as u64,
                                estimated_bytes: plan.estimated_bytes * 3,
                                executable_bytes: None,
                            },
                            CleanupCategoryState {
                                key: "system.restore_points".into(),
                                group: "测试清理".into(),
                                group_order: 1,
                                name: "系统备份还原".into(),
                                display_order: 2,
                                count: 1,
                                estimated_bytes: 28 * 1024 * 1024 * 1024,
                                executable_bytes: None,
                            },
                        ];
                        app.state.cleanup.categories.push(CleanupCategoryState {
                            key: "system.logs".into(),
                            group: "测试清理".into(),
                            group_order: 1,
                            name: "空日志项".into(),
                            display_order: 3,
                            count: 0,
                            estimated_bytes: 0,
                            executable_bytes: None,
                        });
                        app.state.cleanup.selected_categories = app
                            .state
                            .cleanup
                            .categories
                            .iter()
                            .map(|category| category.key.clone())
                            .collect();
                        app.state.apply_cleanup_availability(&plan.by_category);
                        app.state.cleanup.selected_count = plan.items.len();
                        app.state.cleanup.estimated_bytes = plan.estimated_bytes;
                        let mut index = ScanIndex::new(normalize_path(r"C:\").unwrap());
                        for item in &plan.items {
                            index.add_cleanup_candidate(item.candidate.clone());
                        }
                        app.scan_index = Some(index);
                        app.active_analysis_category = Some("大文件".into());
                        app.cleanup_plan = Some(plan);
                        app
                    })
                },
            )
            .unwrap();
            cx.activate(true);
        });
        let remaining = std::fs::read_dir(&folder).unwrap().count();
        assert!(
            remaining < 400,
            "Confirm cleanup in the test window before closing it"
        );
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn cleanup_group_selection_selects_all_then_clears_all() {
        let mut selected = std::collections::BTreeSet::new();
        let keys = vec!["system.temp".to_owned(), "system.logs".to_owned()];

        toggle_group_selection(&mut selected, &keys);
        assert_eq!(selected, keys.iter().cloned().collect());

        toggle_group_selection(&mut selected, &keys);
        assert!(selected.is_empty());
    }

    #[test]
    fn analysis_cleanup_rows_keep_zero_byte_categories_and_sort_by_space() {
        let categories = vec![
            CleanupCategoryState {
                key: "small".into(),
                group: "应用专清".into(),
                group_order: 2,
                name: "小缓存".into(),
                display_order: 1,
                count: 2,
                estimated_bytes: 100,
                executable_bytes: None,
            },
            CleanupCategoryState {
                key: "empty".into(),
                group: "应用专清".into(),
                group_order: 2,
                name: "空项".into(),
                display_order: 2,
                count: 9,
                estimated_bytes: 0,
                executable_bytes: None,
            },
            CleanupCategoryState {
                key: "large".into(),
                group: "系统专清".into(),
                group_order: 1,
                name: "大缓存".into(),
                display_order: 1,
                count: 1,
                estimated_bytes: 900,
                executable_bytes: None,
            },
        ];
        let selected = ["large".to_owned()]
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>();

        let rows = analysis_cleanup_rows(&categories, &selected, Some("small"));

        assert_eq!(
            rows.iter().map(|row| row.key.as_str()).collect::<Vec<_>>(),
            vec!["large", "small", "empty"]
        );
        assert!(rows[0].selected);
        assert!(rows[1].active);
    }

    #[test]
    fn selected_cleanup_items_exclude_statistical_sources() {
        let mut app = super::CleanerApp::new();
        app.state.cleanup.categories = vec![CleanupCategoryState {
            key: "system.restore_points".into(),
            group: "系统专清".into(),
            group_order: 1,
            name: "系统备份还原".into(),
            display_order: 1,
            count: 1,
            estimated_bytes: 1024,
            executable_bytes: None,
        }];
        app.state
            .cleanup
            .selected_categories
            .insert("system.restore_points".into());
        app.scan_index = Some(ScanIndex::new(normalize_path(r"C:\").unwrap()));

        let items = app.analysis_selected_cleanup_items(r"C:\", Some("system.restore_points"));
        let impacts =
            app.analysis_selected_directory_impacts(r"C:\", Some("system.restore_points"));

        assert_eq!(items.total_count, 0);
        assert_eq!(items.total_bytes, 0);
        assert!(impacts.is_empty());
    }

    #[test]
    fn selected_cleanup_details_and_directory_impacts_follow_category_selection() {
        let mut app = super::CleanerApp::new();
        app.state.cleanup.categories = vec![
            CleanupCategoryState {
                key: "system.temp".into(),
                group: "系统专清".into(),
                group_order: 1,
                name: "临时文件".into(),
                display_order: 1,
                count: 1,
                estimated_bytes: 100,
                executable_bytes: None,
            },
            CleanupCategoryState {
                key: "browser.cache".into(),
                group: "浏览器专清".into(),
                group_order: 2,
                name: "浏览器缓存".into(),
                display_order: 1,
                count: 1,
                estimated_bytes: 200,
                executable_bytes: None,
            },
        ];
        app.state.cleanup.selected_categories =
            ["system.temp".to_owned(), "browser.cache".to_owned()]
                .into_iter()
                .collect();
        let mut index = ScanIndex::new(normalize_path(r"C:\").unwrap());
        index.add_cleanup_candidate(test_quick_candidate(
            r"C:\Windows\Temp\a.tmp",
            "system.temp",
            100,
        ));
        index.add_cleanup_candidate(test_quick_candidate(
            r"C:\Users\A\AppData\cache.bin",
            "browser.cache",
            200,
        ));
        app.scan_index = Some(index);
        app.refresh_cleanup_plan();

        assert_eq!(
            app.analysis_selected_cleanup_items(r"C:\", None)
                .total_bytes,
            300
        );
        assert_eq!(
            app.analysis_selected_directory_impacts(r"C:\", None).len(),
            2
        );
        assert_eq!(
            app.analysis_selected_cleanup_items(r"C:\", Some("browser.cache"))
                .total_bytes,
            200
        );

        app.state
            .cleanup
            .selected_categories
            .remove("browser.cache");
        app.refresh_cleanup_plan();

        let items = app.analysis_selected_cleanup_items(r"C:\", None);
        let impacts = app.analysis_selected_directory_impacts(r"C:\", None);
        assert_eq!(items.total_bytes, 100);
        assert_eq!(items.rows[0].path, r"C:\Windows\Temp\a.tmp");
        assert_eq!(impacts.len(), 1);
        assert_eq!(impacts[0].path, r"C:\Windows");
    }

    #[test]
    fn refresh_cleanup_plan_estimate_matches_executable_items() {
        let mut app = super::CleanerApp::new();
        app.state.cleanup.categories = vec![
            CleanupCategoryState {
                key: "system.temp".into(),
                group: "系统专清".into(),
                group_order: 1,
                name: "临时文件".into(),
                display_order: 1,
                count: 2,
                estimated_bytes: 500,
                executable_bytes: None,
            },
            CleanupCategoryState {
                key: "system.restore_points".into(),
                group: "系统专清".into(),
                group_order: 1,
                name: "系统备份还原".into(),
                display_order: 2,
                count: 1,
                estimated_bytes: 900,
                executable_bytes: None,
            },
        ];
        app.state.cleanup.selected_categories =
            ["system.temp".into(), "system.restore_points".into()]
                .into_iter()
                .collect();
        let mut index = ScanIndex::new(normalize_path(r"C:\").unwrap());
        index.add_cleanup_candidate(test_quick_candidate(
            r"C:\Windows\Temp\a.tmp",
            "system.temp",
            100,
        ));
        index.add_cleanup_candidate(test_candidate(r"C:\", "system.restore_points", 900));
        app.scan_index = Some(index);

        let full_plan = crate::runtime::prepare_cleanup_plan_with_explicit(
            app.scan_index.as_ref().unwrap().cleanup_candidates(),
        );
        app.state.apply_cleanup_availability(&full_plan.by_category);
        app.refresh_cleanup_plan();

        assert_eq!(app.state.cleanup.selected_count, 1);
        assert_eq!(app.state.cleanup.estimated_bytes, 100);
        assert_eq!(app.cleanup_plan.as_ref().unwrap().estimated_bytes, 100);
        assert_eq!(app.state.cleanup.categories[0].estimated_bytes, 500);
        assert_eq!(app.state.cleanup.categories[0].executable_bytes, Some(100));
        assert_eq!(app.state.cleanup.categories.len(), 2);
        assert_eq!(app.state.cleanup.categories[1].executable_bytes, None);
        assert!(
            !app.state
                .cleanup
                .selected_categories
                .contains("system.restore_points")
        );
        assert!(
            app.state
                .cleanup
                .selected_categories
                .contains("system.temp")
        );
        assert_eq!(
            app.analysis_selected_cleanup_items(r"C:\", None)
                .total_bytes,
            100
        );
        assert_eq!(
            app.analysis_selected_directory_impacts(r"C:\", None)[0].bytes,
            100
        );

        app.state.cleanup.selected_categories.remove("system.temp");
        app.refresh_cleanup_plan();
        assert_eq!(app.state.cleanup.estimated_bytes, 0);
        assert_eq!(app.state.cleanup.categories[0].executable_bytes, Some(100));
        app.state
            .cleanup
            .selected_categories
            .insert("system.temp".into());
        app.selected_directories.insert(r"C:\Users".into());
        app.refresh_cleanup_plan();
        assert_eq!(app.state.cleanup.estimated_bytes, 0);
        app.selected_directories.clear();
        app.refresh_cleanup_plan();
        assert_eq!(app.state.cleanup.estimated_bytes, 100);
    }

    #[test]
    fn analysis_bucket_path_groups_by_current_scope_child() {
        assert_eq!(
            analysis_bucket_path(r"C:\", r"C:\Users\A\AppData\cache.bin"),
            r"C:\Users"
        );
        assert_eq!(
            analysis_bucket_path(r"C:\Users", r"C:\Users\A\AppData\cache.bin"),
            r"C:\Users\A"
        );
    }

    #[test]
    fn analysis_parent_path_returns_one_level_and_stops_at_drive_root() {
        assert_eq!(
            analysis_parent_path(r"C:\Users\A\AppData"),
            Some(r"C:\Users\A".to_owned())
        );
        assert_eq!(analysis_parent_path(r"C:\"), None);
    }

    #[test]
    fn analysis_impact_height_only_prevents_negative_layout_size() {
        assert_eq!(40.0_f32.max(0.0), 40.0);
        assert_eq!((-40.0_f32).max(0.0), 0.0);
    }

    #[test]
    fn directory_scope_matches_children_without_matching_sibling_names() {
        assert!(path_is_within(r"C:\Users\A", r"C:\Users\A\Temp\x.tmp"));
        assert!(path_is_within(r"C:\Users\A", r"c:/users/a"));
        assert!(!path_is_within(r"C:\Users\A", r"C:\Users\AB\Temp\x.tmp"));
    }

    #[test]
    fn cleanup_confirmation_backdrop_must_be_translucent() {
        assert_eq!(u32::from(cleanup_confirmation_backdrop()), 0x00000066);
    }

    #[test]
    fn cleanup_result_detail_explains_a_skipped_file() {
        let detail = cleanup_result_detail(&application::CleanupResult {
            items: vec![application::CleanupResultItem {
                path: r"C:\Temp\locked.tmp".into(),
                result: application::CleanupItemResult::SkippedLocked,
                recoverable: false,
                released_bytes: 0,
            }],
            ..application::CleanupResult::default()
        });

        assert_eq!(
            detail.as_deref(),
            Some(r"C:\Temp\locked.tmp：文件正在使用中")
        );
    }

    #[test]
    fn cleanup_result_detail_explains_restart_required() {
        let detail = cleanup_result_detail(&application::CleanupResult {
            items: vec![application::CleanupResultItem {
                path: r"C:\pagefile.sys".into(),
                result: application::CleanupItemResult::RestartRequired,
                recoverable: false,
                released_bytes: 0,
            }],
            ..application::CleanupResult::default()
        });

        assert_eq!(
            detail.as_deref(),
            Some(r"C:\pagefile.sys：已更新虚拟内存配置，重启后释放空间")
        );
    }

    #[test]
    fn restart_required_completes_the_action_without_removing_scanned_bytes() {
        assert!(cleanup_item_completed(
            &application::CleanupItemResult::RestartRequired
        ));
        assert!(!cleanup_item_removes_scanned_bytes(
            &application::CleanupItemResult::RestartRequired
        ));
        assert!(cleanup_item_removes_scanned_bytes(
            &application::CleanupItemResult::DeletedPermanently
        ));
    }

    #[test]
    fn cleanup_confirmation_warns_about_system_storage_side_effects() {
        let mut paging = test_candidate(r"C:\pagefile.sys", "system.virtual_memory", 10_000);
        paging.rule_id = Some("system.virtual_memory".into());
        paging.mode = Some(CleanupMode::Manual);
        let mut hibernation = test_candidate(r"C:\hiberfil.sys", "system.hibernate", 20_000);
        hibernation.rule_id = Some("system.hibernate".into());
        hibernation.mode = Some(CleanupMode::Manual);
        let plan = application::preview_cleanup_with_explicit(&[paging, hibernation]);

        assert_eq!(
            cleanup_confirmation_warning(&plan),
            "将关闭系统休眠和快速启动，并禁用分页文件配置；虚拟内存文件需重启后才会释放。"
        );
    }

    fn test_candidate(path: &str, category: &str, estimated_size: u64) -> CleanupCandidate {
        CleanupCandidate {
            path: normalize_path(path).unwrap(),
            file_identity: None,
            rule_id: None,
            category: Some(category.into()),
            reason: None,
            estimated_size: Some(estimated_size),
            quick_clean_eligible: false,
            quick_clean_block_reason: None,
            mode: None,
            risk: None,
            confidence: None,
            recovery: None,
            locked: None,
            path_allowed: None,
            file_identity_unchanged: None,
            last_write_time: None,
        }
    }

    fn test_quick_candidate(path: &str, category: &str, estimated_size: u64) -> CleanupCandidate {
        CleanupCandidate {
            mode: Some(CleanupMode::Quick),
            risk: Some(RiskLevel::P3),
            confidence: Some(Confidence::High),
            recovery: Some(RecoveryMode::Recycle),
            ..test_candidate(path, category, estimated_size)
        }
    }

    #[test]
    fn large_file_display_uses_allocated_size_when_logical_size_is_zero() {
        let index = ScanIndex::new(normalize_path(r"C:\").unwrap());
        let file = FileRecord::new(
            normalize_path(r"C:\large.bin").unwrap(),
            index.root_directory_id(),
            FileIdentity {
                volume_serial: 1,
                file_index: 1,
            },
            0,
            4096,
        );

        assert_eq!(display_file_size(&file), 4096);
    }

    #[test]
    fn large_file_records_only_include_cleanup_candidates() {
        let mut index = ScanIndex::new(normalize_path(r"C:\").unwrap());
        let cleanup_file = FileRecord::new(
            normalize_path(r"C:\Temp\cache.tmp").unwrap(),
            index.root_directory_id(),
            FileIdentity {
                volume_serial: 1,
                file_index: 1,
            },
            LARGE_FILE_MIN_BYTES,
            LARGE_FILE_MIN_BYTES,
        );
        let unrelated_file = FileRecord::new(
            normalize_path(r"C:\Program Files\app.bin").unwrap(),
            index.root_directory_id(),
            FileIdentity {
                volume_serial: 1,
                file_index: 2,
            },
            8192,
            8192,
        );
        index.add_file(cleanup_file.clone());
        index.add_file(unrelated_file);
        index.add_cleanup_candidate(super::runtime::explicit_file_candidate(&cleanup_file));

        let rows = cleanup_large_file_records(&index, &HashSet::new());

        assert_eq!(
            rows.iter()
                .map(|file| file.path().display())
                .collect::<Vec<_>>(),
            vec![r"C:\Temp\cache.tmp"]
        );
    }

    #[test]
    fn large_file_records_exclude_candidates_below_100mb() {
        let mut index = ScanIndex::new(normalize_path(r"C:\").unwrap());
        let small_file = FileRecord::new(
            normalize_path(r"C:\Temp\tiny.tmp").unwrap(),
            index.root_directory_id(),
            FileIdentity {
                volume_serial: 1,
                file_index: 77,
            },
            LARGE_FILE_MIN_BYTES - 1,
            LARGE_FILE_MIN_BYTES - 1,
        );
        index.add_file(small_file.clone());
        index.add_cleanup_candidate(super::runtime::explicit_file_candidate(&small_file));

        assert!(cleanup_large_file_records(&index, &HashSet::new()).is_empty());
    }

    #[test]
    fn large_file_records_include_candidates_outside_top_file_capacity() {
        let mut index = ScanIndex::new(normalize_path(r"C:\").unwrap());
        for file_index in 0..ScanIndex::TOP_FILES_CAPACITY as u64 {
            index.add_file(FileRecord::new(
                normalize_path(&format!(r"C:\large-{file_index}.bin")).unwrap(),
                index.root_directory_id(),
                FileIdentity {
                    volume_serial: 1,
                    file_index: file_index.into(),
                },
                8192,
                8192,
            ));
        }
        let cleanup_file = FileRecord::new(
            normalize_path(r"C:\Temp\small-cleanup.tmp").unwrap(),
            index.root_directory_id(),
            FileIdentity {
                volume_serial: 1,
                file_index: 10_001,
            },
            LARGE_FILE_MIN_BYTES,
            LARGE_FILE_MIN_BYTES,
        );
        index.add_file(cleanup_file.clone());
        index.add_cleanup_candidate(super::runtime::explicit_file_candidate(&cleanup_file));

        let rows = cleanup_large_file_records(&index, &HashSet::new());

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].path().display(), r"C:\Temp\small-cleanup.tmp");
    }

    #[test]
    fn large_file_rows_cache_reuses_index_projection_and_filters_removed_files() {
        let mut app = super::CleanerApp::new();
        let mut index = ScanIndex::new(normalize_path(r"C:\").unwrap());
        let file = FileRecord::new(
            normalize_path(r"C:\Temp\cache.tmp").unwrap(),
            index.root_directory_id(),
            FileIdentity {
                volume_serial: 1,
                file_index: 7,
            },
            LARGE_FILE_MIN_BYTES,
            LARGE_FILE_MIN_BYTES,
        );
        index.add_file(file.clone());
        index.add_cleanup_candidate(super::runtime::explicit_file_candidate(&file));
        app.scan_index = Some(index);

        assert_eq!(app.large_file_rows().len(), 1);
        let cached = app
            .large_file_records_cache
            .as_ref()
            .map(Vec::len)
            .unwrap_or_default();
        assert_eq!(cached, 1);

        app.removed_large_files.insert(file.identity());
        app.large_file_rows_cache = None;
        assert!(app.large_file_rows().is_empty());
        assert_eq!(
            app.large_file_records_cache
                .as_ref()
                .map(Vec::len)
                .unwrap_or_default(),
            cached
        );
    }

    #[test]
    fn selected_large_file_cleanup_respects_selected_directory_scope() {
        let mut app = super::CleanerApp::new();
        let mut index = ScanIndex::new(normalize_path(r"C:\").unwrap());
        let selected_file = FileRecord::new(
            normalize_path(r"C:\Users\A\cache.bin").unwrap(),
            index.root_directory_id(),
            FileIdentity {
                volume_serial: 1,
                file_index: 8,
            },
            LARGE_FILE_MIN_BYTES,
            LARGE_FILE_MIN_BYTES,
        );
        let sibling_file = FileRecord::new(
            normalize_path(r"C:\Users\AB\cache.bin").unwrap(),
            index.root_directory_id(),
            FileIdentity {
                volume_serial: 1,
                file_index: 9,
            },
            LARGE_FILE_MIN_BYTES,
            LARGE_FILE_MIN_BYTES,
        );
        index.add_file(selected_file.clone());
        index.add_file(sibling_file.clone());
        index.add_cleanup_candidate(super::runtime::explicit_file_candidate(&selected_file));
        index.add_cleanup_candidate(super::runtime::explicit_file_candidate(&sibling_file));
        app.scan_index = Some(index);
        app.selected_directories.insert(r"C:\Users\A".into());
        app.selected_large_files.insert(selected_file.identity());
        app.selected_large_files.insert(sibling_file.identity());

        let plan = app.prepare_selected_large_file_cleanup_plan();

        assert_eq!(plan.items.len(), 1);
        assert_eq!(plan.estimated_bytes, LARGE_FILE_MIN_BYTES);
        assert_eq!(
            plan.items[0].candidate.path.display(),
            r"C:\Users\A\cache.bin"
        );
    }
}
