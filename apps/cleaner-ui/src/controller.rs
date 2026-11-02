use std::{
    collections::HashSet,
    path::PathBuf,
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender},
    },
    time::Instant,
};

use application::{CancellationToken, CleanupPlan, CleanupProgress, CleanupResult};
use domain::{
    CleanupCandidate, DirectoryId, FileIdentity, FileRecord, NormalizedPath, ScanIndex,
    normalize_path,
};

use crate::{
    commands::{AnalysisCommand, UiCommand},
    screens::settings::SettingsModel,
    space_analysis::model::{MapPreview, Metric, SpaceAnalysisSnapshot},
    state::AppState,
};

#[derive(Clone, Debug)]
pub struct ControllerConfig {
    pub initial_scan_root: NormalizedPath,
    pub data_root: PathBuf,
}

pub enum WorkerMessage {
    FileRevealed(Result<(), String>),
    SettingsSaved(Result<(), String>),
    AnalysisPlanReady {
        scan_generation: u64,
        plan_generation: u64,
        plan: CleanupPlan,
    },
    IndexUpdated {
        scan_generation: u64,
        index: Arc<ScanIndex>,
    },
    AnalysisReady {
        generation: u64,
        snapshot: SpaceAnalysisSnapshot,
    },
    AnalysisFailed {
        generation: u64,
        error: String,
    },
    ScanProgress {
        generation: u64,
        progress: crate::runtime::ScanProgress,
    },
    ScanCompleted {
        generation: u64,
        report: crate::runtime::ScanReport,
        large_files: Vec<FileRecord>,
    },
    ScanFailed {
        generation: u64,
        error: String,
    },
    CleanupPlanReady {
        scan_generation: u64,
        plan_generation: u64,
        plan: CleanupPlan,
    },
    CleanupProgress(CleanupProgress),
    CleanupCompleted(CleanupResult),
    CleanupFailed(String),
    CleanupFinished(CleanupWorkerResult),
}

pub struct CleanupWorkerResult {
    pub result: Result<CleanupResult, String>,
    pub plan: CleanupPlan,
    pub volume_before: Option<u64>,
    pub volume_after: Option<(u64, u64)>,
    pub recycle_stats: Option<(u64, u64)>,
}

fn spawn_worker(
    sender: Sender<WorkerMessage>,
    panic_message: WorkerMessage,
    work: impl FnOnce() -> WorkerMessage + Send + 'static,
) {
    std::thread::spawn(move || {
        let message =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)).unwrap_or(panic_message);
        let _ = sender.send(message);
    });
}

pub enum ControllerEffect {
    None,
    Scan {
        cancel: CancellationToken,
        generation: u64,
    },
    Cleanup {
        plan: CleanupPlan,
        cancel: CancellationToken,
    },
}

#[derive(Clone, Debug)]
pub(crate) struct CleanupRunState {
    pub started_at: Instant,
    pub estimated_bytes: u64,
    pub progress: CleanupProgress,
    pub finished_at: Option<Instant>,
    pub cancel: CancellationToken,
    pub finished: bool,
    pub error: Option<String>,
    pub issues: Vec<application::CleanupResultItem>,
    pub disk_available_change: Option<i128>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CleanupConfirmation {
    pub item_count: usize,
    pub estimated_bytes: u64,
    pub paths: Vec<String>,
    pub page: usize,
    pub irreversible_count: usize,
    pub warnings: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CleanupProgressView {
    pub progress: CleanupProgress,
    pub finished: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UiSnapshot {
    pub exit_confirmation: bool,
    pub exit_pending: bool,
    pub state: AppState,
    pub settings: SettingsModel,
    pub settings_pending: bool,
    pub space_analysis: SpaceAnalysisSnapshot,
    pub cleanup_confirmation: Option<CleanupConfirmation>,
    pub cleanup_progress: Option<CleanupProgressView>,
    pub cleanup_issues: Vec<application::CleanupResultItem>,
    pub cleanup_busy: bool,
    pub cleanup_stopping: bool,
    pub cleanup_elapsed_seconds: u64,
    pub cleanup_estimated_bytes: u64,
    pub cleanup_disk_change: Option<i128>,
    pub collapsed_cleanup_groups: HashSet<String>,
    pub large_files: Vec<LargeFileView>,
    pub large_file_total: usize,
    pub large_file_bytes: u64,
    pub selected_large_file_count: usize,
    pub selected_large_file_bytes: u64,
    pub large_file_page: usize,
    pub large_file_min_bytes: u64,
    pub cancel_scan_confirmation: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LargeFileView {
    pub identity: FileIdentity,
    pub path: String,
    pub bytes: u64,
    pub modified: u64,
    pub selected: bool,
}

pub struct UiController {
    exit_confirmation: bool,
    exit_pending: bool,
    config: ControllerConfig,
    pub(crate) state: AppState,
    pub(crate) settings: SettingsModel,
    settings_before_save: Option<SettingsModel>,
    pub(crate) scan_index: Option<Arc<ScanIndex>>,
    pub(crate) cleanup_plan: Option<CleanupPlan>,
    pub(crate) cleanup_result: Option<CleanupResult>,
    pub(crate) cleanup_run: Option<CleanupRunState>,
    pub(crate) cleanup_confirmed: bool,
    confirmation_page: usize,
    pub(crate) scan_cancel: Option<CancellationToken>,
    pub(crate) scan_generation: u64,
    pub(crate) cleanup_plan_generation: u64,
    pub(crate) cleanup_plan_pending: bool,
    pending_cancel_scan: bool,
    pub(crate) cleaned_paths: HashSet<String>,
    pub(crate) selected_directories: HashSet<String>,
    pub(crate) selected_large_files: HashSet<domain::FileIdentity>,
    pub(crate) removed_large_files: HashSet<domain::FileIdentity>,
    pub(crate) large_file_records_cache: Option<Vec<FileRecord>>,
    large_file_page: usize,
    large_file_min_bytes: u64,
    pub(crate) expanded_directories: HashSet<String>,
    pub(crate) analysis_marked: HashSet<NormalizedPath>,
    pub(crate) space_analysis_path: String,
    space_analysis_snapshot: SpaceAnalysisSnapshot,
    analysis_generation: u64,
    pub(crate) active_analysis_category: Option<String>,
    pub(crate) collapsed_cleanup_groups: HashSet<String>,
    cleanup_progress: Option<CleanupProgressView>,
    error: Option<String>,
    worker_sender: Sender<WorkerMessage>,
    worker_receiver: Receiver<WorkerMessage>,
    cleanup_mailbox: crate::runtime::CleanupProgressMailbox,
}

impl Default for UiController {
    fn default() -> Self {
        Self::new()
    }
}

impl UiController {
    fn uses_production_scope(&self) -> bool {
        self.config.data_root == persistence::data_root()
            && self
                .config
                .initial_scan_root
                .display()
                .eq_ignore_ascii_case(r"C:\")
    }

    fn permits_candidate_path(&self, path: &str) -> bool {
        self.uses_production_scope()
            || path_is_within(self.config.initial_scan_root.display(), path)
    }

    fn validate_plan_scope(&self, plan: &CleanupPlan) -> Result<(), String> {
        if plan
            .items
            .iter()
            .all(|item| self.permits_candidate_path(item.candidate.path.display()))
        {
            Ok(())
        } else {
            Err("清理计划包含扫描范围以外的路径".to_owned())
        }
    }
    pub fn new() -> Self {
        let root = normalize_path(r"C:\").expect("valid Windows root");
        Self::with_config(ControllerConfig {
            initial_scan_root: root,
            data_root: persistence::data_root(),
        })
    }

    pub fn with_config(config: ControllerConfig) -> Self {
        let categories = crate::runtime::builtin_scan_categories()
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
        let mut state = AppState::default();
        state.cleanup.selected_categories = categories.iter().map(|c| c.key.clone()).collect();
        state.cleanup.categories = categories;
        let settings_path = if config.data_root == persistence::data_root() {
            SettingsModel::default_path()
        } else {
            Some(config.data_root.join("config.toml"))
        };
        let settings = settings_path
            .as_deref()
            .map(SettingsModel::load_from)
            .unwrap_or_default();
        let index = ScanIndex::new(config.initial_scan_root.clone());
        let directory = index.root_directory_id();
        let space_analysis_snapshot = empty_space_snapshot(&index, directory);
        let (worker_sender, worker_receiver) = mpsc::channel();
        let space_analysis_path = config.initial_scan_root.display().to_owned();
        Self {
            exit_confirmation: false,
            exit_pending: false,
            config,
            state,
            settings,
            settings_before_save: None,
            scan_index: Some(Arc::new(index)),
            cleanup_plan: None,
            cleanup_confirmed: false,
            confirmation_page: 0,
            scan_cancel: None,
            scan_generation: 0,
            cleanup_plan_generation: 0,
            cleanup_plan_pending: false,
            pending_cancel_scan: false,
            cleaned_paths: HashSet::new(),
            selected_directories: HashSet::new(),
            selected_large_files: HashSet::new(),
            removed_large_files: HashSet::new(),
            large_file_records_cache: None,
            large_file_page: 0,
            large_file_min_bytes: 100 * 1024 * 1024,
            expanded_directories: HashSet::new(),
            analysis_marked: HashSet::new(),
            space_analysis_path,
            space_analysis_snapshot,
            analysis_generation: 0,
            active_analysis_category: None,
            collapsed_cleanup_groups: HashSet::new(),
            cleanup_progress: None,
            cleanup_result: None,
            cleanup_run: None,
            error: None,
            worker_sender,
            worker_receiver,
            cleanup_mailbox: crate::runtime::CleanupProgressMailbox::default(),
        }
    }

    pub fn dispatch(&mut self, command: UiCommand) -> ControllerEffect {
        let effect = self.transition(command);
        if let ControllerEffect::Cleanup { plan, .. } = &effect
            && let Err(error) = self.validate_plan_scope(plan)
        {
            self.apply(WorkerMessage::CleanupFailed(error));
            return ControllerEffect::None;
        }
        self.start_effect(&effect);
        effect
    }

    fn start_effect(&self, effect: &ControllerEffect) {
        match effect {
            ControllerEffect::Scan { cancel, generation } => {
                let cancel = cancel.clone();
                let generation = *generation;
                let root = self.config.initial_scan_root.clone();
                let worker_sender = self.worker_sender.clone();
                spawn_worker(
                    worker_sender.clone(),
                    WorkerMessage::ScanFailed {
                        generation,
                        error: "扫描工作线程异常退出".into(),
                    },
                    move || {
                        let (progress_sender, progress_receiver) = mpsc::channel();
                        let progress_messages = worker_sender.clone();
                        let progress_task = std::thread::spawn(move || {
                            for progress in progress_receiver {
                                if progress_messages
                                    .send(WorkerMessage::ScanProgress {
                                        generation,
                                        progress,
                                    })
                                    .is_err()
                                {
                                    break;
                                }
                            }
                        });
                        let result =
                            crate::runtime::scan_system_drive_at(root, cancel, progress_sender);
                        let _ = progress_task.join();
                        match result {
                            Ok(report) => {
                                let large_files =
                                    cleanup_large_file_records(&report.index, &HashSet::new());
                                WorkerMessage::ScanCompleted {
                                    generation,
                                    report,
                                    large_files,
                                }
                            }
                            Err(error) => WorkerMessage::ScanFailed { generation, error },
                        }
                    },
                );
            }
            ControllerEffect::Cleanup { plan, cancel } => {
                let mut plan = plan.clone();
                let index = self.scan_index.clone();
                let cancel = cancel.clone();
                let data_root = self.config.data_root.clone();
                let scan_root = self.config.initial_scan_root.clone();
                let mailbox = self.cleanup_mailbox.clone();
                spawn_worker(
                    self.worker_sender.clone(),
                    WorkerMessage::CleanupFailed("清理工作线程异常退出".into()),
                    move || {
                        if let Some(index) = index {
                            crate::runtime::classify_explicit_cleanup_items(&mut plan, &index);
                        }
                        let volume_before = platform_windows::query_volume_stats(&scan_root).ok();
                        let result = if data_root == persistence::data_root()
                            && scan_root.display().eq_ignore_ascii_case(r"C:\")
                            || plan.items.iter().all(|item| {
                                path_is_within(scan_root.display(), item.candidate.path.display())
                            }) {
                            crate::runtime::execute_cleanup_with_cancellation_at(
                                &data_root,
                                &plan,
                                &cancel,
                                |progress| mailbox.publish(progress),
                            )
                        } else {
                            Err("清理计划包含扫描范围以外的路径".to_owned())
                        };
                        let volume_after = platform_windows::query_volume_stats(&scan_root).ok();
                        let recycle_stats = plan
                            .items
                            .iter()
                            .any(|item| item.candidate.rule_id.as_deref() == Some("recycle_bin"))
                            .then(|| {
                                platform_windows::shell::recycle_bin::query_current_user_stats()
                                    .ok()
                            })
                            .flatten();
                        WorkerMessage::CleanupFinished(CleanupWorkerResult {
                            result,
                            plan,
                            volume_before: volume_before.map(|stats| stats.available_bytes()),
                            volume_after: volume_after
                                .map(|stats| (stats.available_bytes(), stats.total_bytes())),
                            recycle_stats: recycle_stats
                                .map(|stats| (stats.item_count, stats.bytes)),
                        })
                    },
                );
            }
            ControllerEffect::None => {}
        }
    }

    fn transition(&mut self, command: UiCommand) -> ControllerEffect {
        if (self.cleanup_run.as_ref().is_some_and(|run| !run.finished)
            || self.cleanup_confirmed
            || self.cleanup_plan_pending)
            && matches!(
                &command,
                UiCommand::ToggleCleanupCategory(_)
                    | UiCommand::Analysis(
                        AnalysisCommand::ToggleMark(_) | AnalysisCommand::ReviewMarks
                    )
                    | UiCommand::ToggleCleanupGroupSelection(_)
                    | UiCommand::ToggleDirectorySelection(_)
                    | UiCommand::ToggleLargeFileSelection(_)
                    | UiCommand::CreateQuickCleanPreview
            )
        {
            return ControllerEffect::None;
        }
        match command {
            UiCommand::RevealFile(path) => {
                if self.permits_candidate_path(&path) {
                    spawn_worker(
                        self.worker_sender.clone(),
                        WorkerMessage::FileRevealed(Err("打开所在文件夹失败".into())),
                        move || {
                            WorkerMessage::FileRevealed(
                                std::process::Command::new("explorer.exe")
                                    .arg(format!("/select,{path}"))
                                    .spawn()
                                    .map(|_| ())
                                    .map_err(|error| error.to_string()),
                            )
                        },
                    );
                }
            }
            UiCommand::RequestExit => self.exit_confirmation = true,
            UiCommand::CancelExit => {
                if !self.exit_pending {
                    self.exit_confirmation = false;
                }
            }
            UiCommand::ConfirmExit => {
                if self.exit_confirmation {
                    self.exit_pending = true;
                    self.exit_confirmation = false;
                    if let Some(cancel) = &self.scan_cancel {
                        cancel.cancel();
                    }
                    if let Some(run) = &self.cleanup_run {
                        run.cancel.cancel();
                    }
                }
            }
            UiCommand::SetConfirmationPage(page) => {
                self.confirmation_page = page.min(
                    self.cleanup_plan
                        .as_ref()
                        .map_or(0, |plan| plan.items.len().saturating_sub(1) / 80),
                );
            }
            UiCommand::RequestCancelScan => {
                self.pending_cancel_scan = matches!(
                    self.state.scan.status,
                    crate::state::ScanViewStatus::Scanning | crate::state::ScanViewStatus::Paused
                );
            }
            UiCommand::DismissCancelScan => self.pending_cancel_scan = false,
            UiCommand::SetLargeFilePage(page) => {
                let count = self
                    .large_file_records_cache
                    .as_deref()
                    .unwrap_or_default()
                    .iter()
                    .filter(|file| self.large_file_visible(file))
                    .count();
                self.large_file_page = page.min(count.saturating_sub(1) / 80);
            }
            UiCommand::SetLargeFileThreshold(bytes) => {
                if [10, 50, 100, 500, 1024]
                    .map(|n| n * 1024 * 1024)
                    .contains(&bytes)
                {
                    self.large_file_min_bytes = bytes;
                    self.large_file_page = 0;
                }
            }
            UiCommand::Analysis(command) => match command {
                AnalysisCommand::SetCleanupSourcePage(page) => {
                    self.space_analysis_snapshot.cleanup_source.page = page;
                    self.refresh_space_analysis();
                }
                AnalysisCommand::SetDirectory(directory) => {
                    if let Some(index) = &self.scan_index
                        && let Some(path) = index
                            .directory(directory)
                            .map(|d| d.path().display().to_owned())
                    {
                        self.space_analysis_path = path;
                        self.reset_analysis_navigation();
                        self.refresh_space_analysis();
                    }
                }
                AnalysisCommand::SetSelection(selection) => {
                    self.space_analysis_snapshot.selection = selection;
                }
                AnalysisCommand::ToggleMark(path) => {
                    if !self.analysis_marked.remove(&path) {
                        self.analysis_marked.insert(path);
                    }
                    self.space_analysis_snapshot.marked = self.analysis_marked.clone();
                }
                AnalysisCommand::ReviewMarks => {
                    if let Some(index) = self.scan_index.clone() {
                        let marked = self.analysis_marked.clone();
                        self.cleanup_plan_generation = self.cleanup_plan_generation.wrapping_add(1);
                        let plan_generation = self.cleanup_plan_generation;
                        let scan_generation = self.scan_generation;
                        self.cleanup_plan_pending = true;
                        spawn_worker(
                            self.worker_sender.clone(),
                            WorkerMessage::CleanupFailed("标记计划工作线程异常退出".into()),
                            move || WorkerMessage::AnalysisPlanReady {
                                scan_generation,
                                plan_generation,
                                plan: crate::space_analysis::model::marked_cleanup_candidates(
                                    &index, &marked,
                                ),
                            },
                        );
                    }
                }
                AnalysisCommand::SetPage(page) => {
                    self.space_analysis_snapshot.page_number = page;
                    self.refresh_space_analysis();
                }
                AnalysisCommand::SetFilter(filter) => {
                    self.space_analysis_snapshot.filter = filter;
                    self.space_analysis_snapshot.page_number = 0;
                    self.refresh_space_analysis();
                }
                AnalysisCommand::SetMetric(metric) => {
                    self.space_analysis_snapshot.metric = metric;
                    self.refresh_space_analysis();
                }
                AnalysisCommand::SetDepth(depth) => {
                    self.space_analysis_snapshot.depth = depth.clamp(1, 4);
                    self.refresh_space_analysis();
                }
                AnalysisCommand::SelectNext(forward) => {
                    let nodes: Vec<_> = self
                        .space_analysis_snapshot
                        .preview
                        .nodes
                        .iter()
                        .filter(|node| !node.aggregate)
                        .collect();
                    if !nodes.is_empty() {
                        let current =
                            self.space_analysis_snapshot
                                .selection
                                .as_ref()
                                .and_then(|selection| {
                                    nodes.iter().position(|node| node.path == selection.path)
                                });
                        let next = current.map_or(0, |current| {
                            if forward {
                                (current + 1) % nodes.len()
                            } else {
                                (current + nodes.len() - 1) % nodes.len()
                            }
                        });
                        let node = nodes[next];
                        self.space_analysis_snapshot.selection =
                            Some(crate::space_analysis::model::AnalysisSelection {
                                path: node.path.clone(),
                                directory: node.directory,
                                file_identity: node.file_identity,
                            });
                    }
                }
                AnalysisCommand::OpenSelection => {
                    if let Some(directory) = self
                        .space_analysis_snapshot
                        .selection
                        .as_ref()
                        .and_then(|selection| selection.directory)
                    {
                        return self.transition(UiCommand::Analysis(
                            AnalysisCommand::SetDirectory(directory),
                        ));
                    }
                }
                AnalysisCommand::ToggleSelectionMark => {
                    if let Some(selection) = &self.space_analysis_snapshot.selection {
                        return self.transition(UiCommand::Analysis(AnalysisCommand::ToggleMark(
                            selection.path.clone(),
                        )));
                    }
                }
                AnalysisCommand::Zoom(increase) => {
                    self.space_analysis_snapshot.zoom = (self.space_analysis_snapshot.zoom
                        * if increase { 1.25 } else { 0.8 })
                    .clamp(0.5, 4.0);
                }
                AnalysisCommand::Fit => {
                    self.space_analysis_snapshot.zoom = 1.0;
                    self.space_analysis_snapshot.pan = (0.0, 0.0);
                }
                AnalysisCommand::Pan(x, y) => {
                    self.space_analysis_snapshot.pan.0 =
                        (self.space_analysis_snapshot.pan.0 + x as f32).clamp(-5000.0, 5000.0);
                    self.space_analysis_snapshot.pan.1 =
                        (self.space_analysis_snapshot.pan.1 + y as f32).clamp(-5000.0, 5000.0);
                }
            },
            UiCommand::SetTheme(theme) => {
                if self.settings_before_save.is_none() && self.settings.theme != theme {
                    let previous = self.settings.clone();
                    self.settings.theme = theme;
                    self.persist_settings(previous);
                }
            }
            UiCommand::StartScan => {
                if matches!(
                    self.state.scan.status,
                    crate::state::ScanViewStatus::Scanning | crate::state::ScanViewStatus::Paused
                ) || self.cleanup_confirmed
                    || self.cleanup_run.as_ref().is_some_and(|run| !run.finished)
                {
                    return ControllerEffect::None;
                }
                self.state.apply(UiCommand::StartScan);
                self.error = None;
                self.state.scan_error = None;
                self.scan_generation = self.scan_generation.wrapping_add(1);
                self.cleanup_plan_generation = self.cleanup_plan_generation.wrapping_add(1);
                self.cleanup_plan_pending = false;
                self.scan_index = None;
                self.analysis_marked.clear();
                self.analysis_generation = self.analysis_generation.wrapping_add(1);
                let index = ScanIndex::new(self.config.initial_scan_root.clone());
                self.space_analysis_snapshot =
                    empty_space_snapshot(&index, index.root_directory_id());
                self.cleanup_plan = None;
                self.cleanup_result = None;
                self.cleanup_confirmed = false;
                self.cleanup_progress = None;
                self.cleaned_paths.clear();
                self.selected_directories.clear();
                self.selected_large_files.clear();
                self.removed_large_files.clear();
                let cancel = CancellationToken::default();
                self.scan_cancel = Some(cancel.clone());
                self.large_file_records_cache = None;
                self.large_file_page = 0;
                self.cleanup_run = None;
                self.active_analysis_category = None;
                self.collapsed_cleanup_groups.clear();
                return ControllerEffect::Scan {
                    cancel,
                    generation: self.scan_generation,
                };
            }
            UiCommand::CancelScan => {
                self.pending_cancel_scan = false;
                if let Some(cancel) = &self.scan_cancel {
                    cancel.cancel();
                }
                self.state.apply(UiCommand::CancelScan);
            }
            UiCommand::PauseScan => {
                if let Some(cancel) = &self.scan_cancel {
                    cancel.pause();
                }
                self.state.apply(UiCommand::PauseScan);
            }
            UiCommand::ResumeScan => {
                if let Some(cancel) = &self.scan_cancel {
                    cancel.resume();
                }
                self.state.apply(UiCommand::ResumeScan);
            }
            UiCommand::ToggleCleanupCategory(key) => {
                if self
                    .state
                    .cleanup
                    .categories
                    .iter()
                    .any(|category| category.key == key && category.executable_bytes.is_some())
                {
                    self.state.apply(UiCommand::ToggleCleanupCategory(key));
                    self.refresh_cleanup_plan();
                }
            }
            UiCommand::ToggleCleanupGroupSelection(keys) => {
                let selectable = keys
                    .into_iter()
                    .filter(|key| {
                        self.state.cleanup.categories.iter().any(|category| {
                            category.key == *key && category.executable_bytes.is_some()
                        })
                    })
                    .collect::<Vec<_>>();
                if !selectable.is_empty() {
                    toggle_group_selection(
                        &mut self.state.cleanup.selected_categories,
                        &selectable,
                    );
                    self.refresh_cleanup_plan();
                }
            }
            UiCommand::ToggleDirectorySelection(path) => {
                if !self.selected_directories.remove(&path) {
                    self.selected_directories.insert(path);
                }
                self.refresh_cleanup_plan();
            }
            UiCommand::ToggleLargeFileSelection(identity) => {
                if !self.selected_large_files.remove(&identity) {
                    self.selected_large_files.insert(identity);
                }
                self.refresh_cleanup_plan();
            }
            UiCommand::InspectCleanupCategory(key) => {
                self.active_analysis_category = (!key.is_empty()).then_some(key);
                self.space_analysis_snapshot.cleanup_source.page = 0;
                self.refresh_space_analysis();
            }
            UiCommand::ToggleCleanupGroup(group) => {
                if !self.collapsed_cleanup_groups.remove(&group) {
                    self.collapsed_cleanup_groups.insert(group);
                }
            }
            UiCommand::ToggleDirectory(path) => {
                if !self.expanded_directories.remove(&path) {
                    self.expanded_directories.insert(path);
                }
            }
            UiCommand::OpenAnalysisDirectory(path) => {
                if self.scan_index.as_ref().is_none_or(|index| {
                    normalize_path(&path)
                        .ok()
                        .and_then(|path| index.directory_id(&path))
                        .is_none()
                }) {
                    self.error = Some("目录不在扫描索引中，请检查路径或重新扫描".into());
                    return ControllerEffect::None;
                }
                self.error = None;
                self.space_analysis_path = path.clone();
                self.reset_analysis_navigation();
                self.expanded_directories.insert(path);
                self.refresh_space_analysis();
            }
            UiCommand::OpenAnalysisParent => {
                let parent = self.scan_index.as_ref().and_then(|index| {
                    let path = normalize_path(&self.space_analysis_path).ok()?;
                    let node = index.directory(index.directory_id(&path)?)?;
                    Some(index.directory(node.parent()?)?.path().display().to_owned())
                });
                if let Some(parent) = parent {
                    self.space_analysis_path = parent;
                    self.reset_analysis_navigation();
                    self.refresh_space_analysis();
                }
            }
            UiCommand::ExecuteCleanup(plan_id) => {
                if self.cleanup_run.as_ref().is_some_and(|run| !run.finished)
                    || self.cleanup_plan_pending
                    || self.settings_before_save.is_some()
                    || matches!(
                        self.state.scan.status,
                        crate::state::ScanViewStatus::Scanning
                            | crate::state::ScanViewStatus::Paused
                    )
                {
                    return ControllerEffect::None;
                }
                let plan = if plan_id.0 == 2 {
                    self.prepare_selected_large_file_cleanup_plan()
                } else {
                    self.prepare_selected_cleanup_plan()
                };
                if !plan.items.is_empty() && self.validate_plan_scope(&plan).is_ok() {
                    self.state.cleanup.selected_count = plan.items.len();
                    self.state.cleanup.estimated_bytes = plan.estimated_bytes;
                    self.cleanup_plan = Some(plan);
                    self.cleanup_confirmed = true;
                    if plan_id.0 != 2 {
                        return self.transition(UiCommand::ConfirmCleanup);
                    }
                }
            }
            UiCommand::CancelCleanup => {
                self.confirmation_page = 0;
                self.cleanup_confirmed = false;
                self.refresh_cleanup_plan();
            }
            UiCommand::CreateQuickCleanPreview => {
                self.state.apply(UiCommand::CreateQuickCleanPreview);
                self.refresh_cleanup_plan();
                self.cleanup_result = None;
                self.cleanup_confirmed = false;
            }
            UiCommand::StopCleanup => {
                if let Some(run) = &self.cleanup_run {
                    run.cancel.cancel();
                }
            }
            UiCommand::ConfirmCleanup => {
                if self.settings_before_save.is_some() {
                    return ControllerEffect::None;
                }
                if !self.cleanup_confirmed {
                    return ControllerEffect::None;
                }
                if self.cleanup_run.as_ref().is_some_and(|run| !run.finished) {
                    return ControllerEffect::None;
                }
                let Some(plan) = self.cleanup_plan.take() else {
                    self.cleanup_confirmed = false;
                    return ControllerEffect::None;
                };
                self.confirmation_page = 0;
                if plan.items.is_empty() {
                    self.cleanup_confirmed = false;
                    return ControllerEffect::None;
                }
                let cancel = CancellationToken::default();
                let count = plan.items.len();
                self.cleanup_run = Some(CleanupRunState {
                    started_at: Instant::now(),
                    estimated_bytes: plan.estimated_bytes,
                    progress: CleanupProgress {
                        total: count,
                        ..Default::default()
                    },
                    finished_at: None,
                    cancel: cancel.clone(),
                    finished: false,
                    error: None,
                    issues: Vec::new(),
                    disk_available_change: None,
                });
                self.cleanup_plan_generation = self.cleanup_plan_generation.wrapping_add(1);
                self.cleanup_confirmed = false;
                self.cleanup_result = None;
                self.cleanup_progress = Some(CleanupProgressView {
                    progress: CleanupProgress {
                        total: count,
                        ..Default::default()
                    },
                    finished: false,
                });
                self.state.scan_error = None;
                self.state.selected_view = crate::routes::ViewId::Cleanup;
                return ControllerEffect::Cleanup { plan, cancel };
            }
            other => self.state.apply(other),
        }
        ControllerEffect::None
    }

    pub(crate) fn refresh_cleanup_plan(&mut self) {
        self.cleanup_plan_generation = self.cleanup_plan_generation.wrapping_add(1);
        let plan = self.prepare_selected_cleanup_plan();
        self.state.cleanup.selected_count = plan.items.len();
        self.state.cleanup.estimated_bytes = plan.estimated_bytes;
        self.cleanup_plan = Some(plan);
        self.refresh_space_analysis();
    }

    fn reset_analysis_navigation(&mut self) {
        let snapshot = &mut self.space_analysis_snapshot;
        snapshot.page_number = 0;
        snapshot.filter.clear();
        snapshot.depth = 1;
        snapshot.selection = None;
        snapshot.zoom = 1.0;
        snapshot.pan = (0.0, 0.0);
    }

    fn selected_candidates(&self, candidates: &[CleanupCandidate]) -> Vec<CleanupCandidate> {
        candidates
            .iter()
            .filter(|candidate| {
                !self.cleaned_paths.contains(candidate.path.display())
                    && candidate
                        .file_identity
                        .is_none_or(|id| !self.selected_large_files.contains(&id))
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

    fn prepare_selected_cleanup_plan(&mut self) -> CleanupPlan {
        let Some(index) = &self.scan_index else {
            return CleanupPlan::default();
        };
        let mut candidates = self.selected_candidates(index.cleanup_candidates());
        if !self.selected_large_files.is_empty() {
            self.ensure_large_file_records_cache();
            candidates.extend(self.selected_large_file_candidates());
        }
        crate::runtime::prepare_cleanup_plan_with_explicit(&candidates)
    }

    pub(crate) fn prepare_selected_large_file_cleanup_plan(&mut self) -> CleanupPlan {
        if self.scan_index.is_none() {
            return CleanupPlan::default();
        }
        self.ensure_large_file_records_cache();
        crate::runtime::prepare_cleanup_plan_with_explicit(&self.selected_large_file_candidates())
    }

    fn selected_large_file_candidates(&self) -> Vec<CleanupCandidate> {
        self.large_file_records_cache
            .as_ref()
            .into_iter()
            .flat_map(|files| files.iter())
            .filter(|file| {
                self.selected_large_files.contains(&file.identity())
                    && !self.removed_large_files.contains(&file.identity())
                    && (self.selected_directories.is_empty()
                        || self
                            .selected_directories
                            .iter()
                            .any(|directory| path_is_within(directory, file.path().display())))
            })
            .map(crate::runtime::explicit_file_candidate)
            .collect()
    }

    pub(crate) fn ensure_large_file_records_cache(&mut self) {
        if self.large_file_records_cache.is_none() {
            self.large_file_records_cache = Some(
                self.scan_index
                    .as_ref()
                    .map(|index| cleanup_large_file_records(index, &HashSet::new()))
                    .unwrap_or_default(),
            );
        }
    }

    fn persist_settings(&mut self, previous: SettingsModel) {
        let path = if self.config.data_root == persistence::data_root() {
            SettingsModel::default_path()
        } else {
            Some(self.config.data_root.join("config.toml"))
        };
        self.settings_before_save = Some(previous);
        let settings = self.settings.clone();
        spawn_worker(
            self.worker_sender.clone(),
            WorkerMessage::SettingsSaved(Err("设置保存线程异常退出".into())),
            move || {
                WorkerMessage::SettingsSaved(
                    path.ok_or_else(|| {
                        "LOCALAPPDATA is unavailable; config.toml cannot be saved".to_owned()
                    })
                    .and_then(|path| settings.save_to(&path)),
                )
            },
        );
    }

    pub fn apply(&mut self, message: WorkerMessage) {
        match message {
            WorkerMessage::FileRevealed(Err(error)) => {
                self.error = Some(format!("打开所在文件夹失败：{error}"))
            }
            WorkerMessage::FileRevealed(Ok(())) => {}
            WorkerMessage::SettingsSaved(result) => {
                let previous = self.settings_before_save.take();
                if let Err(error) = result {
                    if let Some(previous) = previous {
                        self.settings = previous;
                    }
                    self.error = Some(format!("设置保存失败：{error}"));
                } else if self
                    .error
                    .as_deref()
                    .is_some_and(|error| error.starts_with("设置保存失败"))
                {
                    self.error = None;
                }
            }
            WorkerMessage::IndexUpdated {
                scan_generation,
                index,
            } => {
                if scan_generation == self.scan_generation {
                    self.scan_index = Some(index);
                    self.refresh_cleanup_plan();
                }
            }
            WorkerMessage::AnalysisPlanReady {
                scan_generation,
                plan_generation,
                plan,
            } => {
                if scan_generation == self.scan_generation
                    && plan_generation == self.cleanup_plan_generation
                {
                    self.cleanup_plan_pending = false;
                    match self.validate_plan_scope(&plan) {
                        Ok(()) if !plan.items.is_empty() => {
                            self.state.cleanup.selected_count = plan.items.len();
                            self.state.cleanup.estimated_bytes = plan.estimated_bytes;
                            self.cleanup_plan = Some(plan);
                            self.cleanup_confirmed = true;
                        }
                        Err(error) => self.error = Some(error),
                        _ => self.error = Some("没有可清理的标记文件".into()),
                    }
                }
            }
            WorkerMessage::AnalysisReady {
                generation,
                snapshot,
            } => {
                if generation == self.analysis_generation {
                    let mut snapshot = snapshot;
                    if snapshot.directory == self.space_analysis_snapshot.directory {
                        snapshot.zoom = self.space_analysis_snapshot.zoom;
                        snapshot.pan = self.space_analysis_snapshot.pan;
                        snapshot.selection = self.space_analysis_snapshot.selection.clone();
                    }
                    self.space_analysis_snapshot = snapshot;
                    self.space_analysis_snapshot.marked = self.analysis_marked.clone();
                }
            }
            WorkerMessage::AnalysisFailed { generation, error } => {
                if generation == self.analysis_generation {
                    self.error = Some(error);
                }
            }
            WorkerMessage::ScanProgress {
                generation,
                progress,
            } => {
                if generation == self.scan_generation {
                    self.state.apply_progress(&progress);
                }
            }
            WorkerMessage::ScanCompleted {
                generation,
                report,
                large_files,
            } => {
                if generation != self.scan_generation {
                    return;
                }
                self.scan_cancel = None;
                self.cleanup_plan = None;
                self.pending_cancel_scan = false;
                self.cleanup_result = None;
                self.expanded_directories.clear();
                self.space_analysis_path = report.index.root().display().to_owned();
                self.selected_directories.clear();
                self.selected_large_files.clear();
                self.removed_large_files.clear();
                self.state.apply_report(&report);
                self.active_analysis_category = self
                    .state
                    .cleanup
                    .categories
                    .iter()
                    .filter(|category| {
                        category.estimated_bytes > 0 || category.executable_bytes.is_some()
                    })
                    .max_by_key(|category| category.estimated_bytes)
                    .map(|category| category.key.clone());
                self.scan_index = Some(Arc::new(report.index));
                self.refresh_space_analysis();
                self.large_file_records_cache = Some(large_files);
                self.state.cleanup.estimated_bytes = 0;
                self.state.cleanup.selected_count = 0;
                self.cleanup_plan_pending = true;
                self.error = None;
                if let Some(index) = &self.scan_index {
                    let candidates = index.cleanup_candidates().to_vec();
                    let sender = self.worker_sender.clone();
                    let scan_generation = self.scan_generation;
                    let plan_generation = self.cleanup_plan_generation;
                    spawn_worker(
                        sender,
                        WorkerMessage::CleanupFailed("清理计划工作线程异常退出".into()),
                        move || {
                            let plan =
                                crate::runtime::prepare_cleanup_plan_with_explicit(&candidates);
                            WorkerMessage::CleanupPlanReady {
                                scan_generation,
                                plan_generation,
                                plan,
                            }
                        },
                    );
                }
            }
            WorkerMessage::CleanupPlanReady {
                scan_generation,
                plan_generation,
                plan,
            } => {
                if scan_generation == self.scan_generation
                    && plan_generation == self.cleanup_plan_generation
                {
                    self.state.apply_cleanup_availability(&plan.by_category);
                    let mut plan = plan;
                    plan.items.retain(|item| {
                        item.candidate
                            .category
                            .as_ref()
                            .is_none_or(|key| self.state.cleanup.selected_categories.contains(key))
                    });
                    plan.by_category
                        .retain(|key, _| self.state.cleanup.selected_categories.contains(key));
                    plan.estimated_bytes = plan.items.iter().map(|item| item.estimated_bytes).sum();
                    self.state.cleanup.selected_count = plan.items.len();
                    self.state.cleanup.estimated_bytes = plan.estimated_bytes;
                    self.active_analysis_category = self
                        .state
                        .cleanup
                        .categories
                        .iter()
                        .filter(|category| category.executable_bytes.is_some())
                        .max_by_key(|category| category.executable_bytes)
                        .map(|category| category.key.clone());
                    self.cleanup_plan = Some(plan);
                    self.cleanup_plan_pending = false;
                    self.refresh_space_analysis();
                }
            }
            WorkerMessage::ScanFailed { generation, error } => {
                if generation != self.scan_generation {
                    return;
                }
                self.state.scan.status = crate::state::ScanViewStatus::Failed;
                self.scan_cancel = None;
                self.error = Some(error.clone());
                self.state.scan_error = Some(error);
            }
            WorkerMessage::CleanupFailed(error) => {
                self.cleanup_plan_pending = false;
                if let Some(run) = &mut self.cleanup_run {
                    run.finished = true;
                    run.finished_at = Some(Instant::now());
                    run.error = Some(error.clone());
                }
                self.error = Some(error.clone());
                self.state.scan_error = Some(error);
                if let Some(progress) = &mut self.cleanup_progress {
                    progress.finished = true;
                }
            }
            WorkerMessage::CleanupProgress(progress) => {
                if let Some(run) = &mut self.cleanup_run {
                    run.progress = progress.clone();
                }
                self.cleanup_progress = Some(CleanupProgressView {
                    progress,
                    finished: false,
                });
            }
            WorkerMessage::CleanupCompleted(result) => {
                if let Some(run) = &mut self.cleanup_run {
                    run.progress.processed = result.items.len();
                    run.progress.released_bytes = result.released_bytes;
                    run.progress.skipped = result.skipped;
                    run.progress.failed = result.failed;
                    run.progress.current_path = None;
                    run.finished = true;
                    run.finished_at = Some(Instant::now());
                }
                self.cleanup_result = Some(result.clone());
                self.cleanup_progress = Some(CleanupProgressView {
                    progress: CleanupProgress {
                        processed: result.items.len(),
                        released_bytes: result.released_bytes,
                        skipped: result.skipped,
                        failed: result.failed,
                        ..Default::default()
                    },
                    finished: true,
                });
            }
            WorkerMessage::CleanupFinished(outcome) => self.apply_cleanup_finished(outcome),
        }
    }

    fn apply_cleanup_finished(&mut self, mut outcome: CleanupWorkerResult) {
        if let Err(error) = &outcome.result {
            self.error = Some(error.clone());
            self.state.scan_error = Some(error.clone());
        }
        let mut removed_categories = std::collections::HashMap::<String, (u64, u64)>::new();
        if let Ok(result) = &outcome.result {
            let completed = result
                .items
                .iter()
                .filter(|item| cleanup_item_completed(&item.result))
                .map(|item| item.path.as_str())
                .collect::<HashSet<_>>();
            let removed = result
                .items
                .iter()
                .filter(|item| cleanup_item_removes_scanned_bytes(&item.result))
                .map(|item| item.path.as_str())
                .collect::<HashSet<_>>();
            outcome.plan.items.retain(|item| {
                let path = item.candidate.path.display();
                if !completed.contains(path) {
                    return true;
                }
                if removed.contains(path) {
                    self.cleaned_paths.insert(path.to_owned());
                    if let Some(identity) = item.candidate.file_identity {
                        self.removed_large_files.insert(identity);
                    }
                    if let Some(category) = &item.candidate.category {
                        let counts = removed_categories.entry(category.clone()).or_default();
                        counts.0 += 1;
                        counts.1 = counts.1.saturating_add(item.estimated_bytes);
                    }
                }
                false
            });
            outcome.plan.estimated_bytes = outcome
                .plan
                .items
                .iter()
                .map(|item| item.estimated_bytes)
                .sum();
            outcome.plan.by_category.clear();
            for item in &outcome.plan.items {
                *outcome
                    .plan
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
        if !self.cleaned_paths.is_empty()
            && let Some(index) = self.scan_index.take()
        {
            let removed = self.cleaned_paths.clone();
            let scan_generation = self.scan_generation;
            self.analysis_generation = self.analysis_generation.wrapping_add(1);
            self.analysis_marked.clear();
            let empty = ScanIndex::new(self.config.initial_scan_root.clone());
            self.space_analysis_snapshot = empty_space_snapshot(&empty, empty.root_directory_id());
            spawn_worker(
                self.worker_sender.clone(),
                WorkerMessage::AnalysisFailed {
                    generation: self.analysis_generation,
                    error: "清理后索引更新失败，请重新扫描".into(),
                },
                move || WorkerMessage::IndexUpdated {
                    scan_generation,
                    index: Arc::new(index.without_paths(&removed)),
                },
            );
        }
        self.selected_large_files
            .retain(|id| !self.removed_large_files.contains(id));
        let remaining_large_files = self
            .large_file_records_cache
            .as_deref()
            .unwrap_or_default()
            .iter()
            .filter(|file| self.large_file_visible(file))
            .count();
        self.large_file_page = self
            .large_file_page
            .min(remaining_large_files.saturating_sub(1) / 80);

        for category in &mut self.state.cleanup.categories {
            if let Some((count, bytes)) = removed_categories.get(&category.key) {
                category.count = category.count.saturating_sub(*count);
                category.estimated_bytes = category.estimated_bytes.saturating_sub(*bytes);
            }
            if category.key == "recycle_bin"
                && let Some((count, bytes)) = outcome.recycle_stats
            {
                category.count = count;
                category.estimated_bytes = bytes;
            }
        }
        self.state.cleanup.selected_count = outcome.plan.items.len();
        self.state.cleanup.estimated_bytes = outcome.plan.estimated_bytes;
        self.state
            .apply_remaining_selected_cleanup(&outcome.plan.by_category);
        if let Some((available, total)) = outcome.volume_after {
            self.state.scan.available_bytes = available;
            self.state.scan.total_bytes = total;
        }
        self.cleanup_plan = Some(outcome.plan);
        if let Some(run) = &mut self.cleanup_run {
            run.disk_available_change = outcome
                .volume_before
                .zip(outcome.volume_after)
                .map(|(before, (after, _))| i128::from(after) - i128::from(before));
            run.issues = outcome
                .result
                .as_ref()
                .ok()
                .map(|result| {
                    result
                        .items
                        .iter()
                        .filter(|item| !cleanup_item_completed(&item.result))
                        .take(100)
                        .cloned()
                        .collect()
                })
                .unwrap_or_default();
            run.finished = true;
            run.finished_at = Some(Instant::now());
            run.progress.current_path = None;
            match outcome.result {
                Ok(result) => {
                    run.progress.processed = result.items.len();
                    run.progress.released_bytes = result.released_bytes;
                    run.progress.skipped = result.skipped;
                    run.progress.failed = result.failed;
                    self.cleanup_result = Some(result);
                }
                Err(error) => run.error = Some(error),
            }
            self.cleanup_progress = Some(CleanupProgressView {
                progress: run.progress.clone(),
                finished: true,
            });
        } else {
            self.cleanup_progress = Some(CleanupProgressView {
                progress: CleanupProgress::default(),
                finished: true,
            });
        }
    }

    pub fn drain(&mut self) -> bool {
        let mut changed = false;
        if let Some(progress) = self.cleanup_mailbox.take() {
            self.apply(WorkerMessage::CleanupProgress(progress));
            changed = true;
        }
        while let Ok(message) = self.worker_receiver.try_recv() {
            self.apply(message);
            changed = true;
        }
        changed
    }

    pub fn sender(&self) -> Sender<WorkerMessage> {
        self.worker_sender.clone()
    }

    fn refresh_space_analysis(&mut self) {
        if let Some(index) = &self.scan_index {
            let index = Arc::clone(index);
            let path = self.space_analysis_path.clone();
            let page_number = self.space_analysis_snapshot.page_number;
            let filter = self.space_analysis_snapshot.filter.clone();
            let metric = self.space_analysis_snapshot.metric;
            let category = self.active_analysis_category.clone();
            let source_page = self.space_analysis_snapshot.cleanup_source.page;
            let selected_categories = self.state.cleanup.selected_categories.clone();
            let selected_directories = self.selected_directories.clone();
            let depth = self.space_analysis_snapshot.depth;
            self.analysis_generation = self.analysis_generation.wrapping_add(1);
            let generation = self.analysis_generation;
            spawn_worker(
                self.worker_sender.clone(),
                WorkerMessage::AnalysisFailed {
                    generation,
                    error: "空间分析工作线程异常退出".into(),
                },
                move || {
                    let directory = normalize_path(&path)
                        .ok()
                        .and_then(|path| index.directory_id(&path))
                        .unwrap_or_else(|| index.root_directory_id());
                    let mut snapshot = empty_space_snapshot(&index, directory);
                    snapshot.cleanup_source = crate::space_analysis::model::cleanup_source_details(
                        index.cleanup_candidates(),
                        &path,
                        category,
                        &selected_categories,
                        &selected_directories,
                        source_page,
                    );
                    snapshot.page_number = page_number;
                    snapshot.filter = filter;
                    snapshot.metric = metric;
                    snapshot.depth = depth;
                    snapshot.page = index.directory_page(
                        directory,
                        page_number
                            .saturating_mul(crate::space_analysis::model::DIRECTORY_PAGE_SIZE),
                        crate::space_analysis::model::DIRECTORY_PAGE_SIZE,
                        (!snapshot.filter.is_empty()).then_some(snapshot.filter.as_str()),
                    );
                    snapshot.preview = crate::space_analysis::model::project_directory_at_depth(
                        &index,
                        directory,
                        metric,
                        crate::space_analysis::model::MAP_NODE_LIMIT,
                        depth,
                    );
                    WorkerMessage::AnalysisReady {
                        generation,
                        snapshot,
                    }
                },
            );
        }
    }

    fn large_file_visible(&self, file: &FileRecord) -> bool {
        !self.removed_large_files.contains(&file.identity())
            && file.logical_size().max(file.allocated_size()) >= self.large_file_min_bytes
    }

    pub fn snapshot(&self) -> UiSnapshot {
        let large_files = self
            .large_file_records_cache
            .as_deref()
            .unwrap_or_default()
            .iter()
            .filter(|file| self.large_file_visible(file))
            .skip(self.large_file_page.saturating_mul(80))
            .take(80)
            .map(|file| LargeFileView {
                identity: file.identity(),
                path: file.path().display().to_owned(),
                bytes: file.logical_size().max(file.allocated_size()),
                modified: file.facts().last_write_time,
                selected: self.selected_large_files.contains(&file.identity()),
            })
            .collect();
        UiSnapshot {
            exit_confirmation: self.exit_confirmation,
            exit_pending: self.exit_pending,
            state: self.state.clone(),
            settings: self.settings.clone(),
            settings_pending: self.settings_before_save.is_some(),
            space_analysis: self.space_analysis_snapshot.clone(),
            cleanup_confirmation: self
                .cleanup_confirmed
                .then_some(self.cleanup_plan.as_ref())
                .flatten()
                .map(|plan| CleanupConfirmation {
                    item_count: plan.items.len(),
                    estimated_bytes: plan.estimated_bytes,
                    page: self.confirmation_page,
                    warnings: cleanup_plan_warnings(plan),
                    irreversible_count: plan.items.len(),
                    paths: plan
                        .items
                        .iter()
                        .skip(self.confirmation_page.saturating_mul(80))
                        .take(80)
                        .map(|item| item.candidate.path.display().to_owned())
                        .collect(),
                }),
            cleanup_progress: self.cleanup_progress.clone(),
            cleanup_busy: self.cleanup_plan_pending
                || self.cleanup_confirmed
                || self.cleanup_run.as_ref().is_some_and(|run| !run.finished),
            cleanup_stopping: self
                .cleanup_run
                .as_ref()
                .is_some_and(|run| !run.finished && run.cancel.is_cancelled()),
            cleanup_elapsed_seconds: self.cleanup_run.as_ref().map_or(0, |run| {
                run.finished_at
                    .unwrap_or_else(Instant::now)
                    .duration_since(run.started_at)
                    .as_secs()
            }),
            cleanup_estimated_bytes: self
                .cleanup_run
                .as_ref()
                .map_or(0, |run| run.estimated_bytes),
            cleanup_disk_change: self
                .cleanup_run
                .as_ref()
                .and_then(|run| run.disk_available_change),
            collapsed_cleanup_groups: self.collapsed_cleanup_groups.clone(),
            cleanup_issues: self
                .cleanup_run
                .as_ref()
                .map(|run| run.issues.clone())
                .unwrap_or_default(),
            large_files,
            large_file_total: self
                .large_file_records_cache
                .as_deref()
                .unwrap_or_default()
                .iter()
                .filter(|file| self.large_file_visible(file))
                .count(),
            large_file_bytes: self
                .large_file_records_cache
                .as_deref()
                .unwrap_or_default()
                .iter()
                .filter(|f| self.large_file_visible(f))
                .map(|f| f.logical_size().max(f.allocated_size()))
                .sum(),
            selected_large_file_count: self.selected_large_files.len(),
            selected_large_file_bytes: self
                .large_file_records_cache
                .as_deref()
                .unwrap_or_default()
                .iter()
                .filter(|f| {
                    self.selected_large_files.contains(&f.identity())
                        && !self.removed_large_files.contains(&f.identity())
                })
                .map(|f| f.logical_size().max(f.allocated_size()))
                .sum(),
            large_file_page: self.large_file_page,
            large_file_min_bytes: self.large_file_min_bytes,
            cancel_scan_confirmation: self.pending_cancel_scan,
            error: self.error.clone().or_else(|| self.state.scan_error.clone()),
        }
    }

    pub fn scan_index(&self) -> Option<&ScanIndex> {
        self.scan_index.as_deref()
    }

    pub fn exit_ready(&self) -> bool {
        self.exit_pending
    }

    pub fn config(&self) -> &ControllerConfig {
        &self.config
    }
}

fn cleanup_plan_warnings(plan: &CleanupPlan) -> Vec<String> {
    let has_rule = |rule| {
        plan.items
            .iter()
            .any(|item| item.candidate.rule_id.as_deref() == Some(rule))
    };
    let mut warnings = Vec::new();
    if has_rule("system.hibernate") {
        warnings.push("将关闭系统休眠和快速启动，并删除休眠文件；需要管理员权限。".into());
    }
    if has_rule("system.virtual_memory") {
        warnings.push("将禁用分页文件配置；pagefile.sys 和 swapfile.sys 需重启后才会释放。".into());
    }
    if has_rule("recycle_bin") {
        warnings.push("将清空当前用户在所有本地磁盘上的回收站，此操作无法恢复。".into());
    }
    warnings
}

fn empty_space_snapshot(index: &ScanIndex, directory: DirectoryId) -> SpaceAnalysisSnapshot {
    let mut breadcrumbs = Vec::new();
    let mut ancestor = Some(directory);
    while let Some(id) = ancestor {
        let Some(node) = index.directory(id) else {
            break;
        };
        breadcrumbs.push((id, node.path().display().to_owned()));
        ancestor = node.parent();
    }
    breadcrumbs.reverse();
    SpaceAnalysisSnapshot {
        cleanup_source: Default::default(),
        directory,
        breadcrumbs,
        metric: Metric::Logical,
        depth: 1,
        directory_path: index
            .directory(directory)
            .map(|node| node.path().display().to_owned())
            .unwrap_or_else(|| index.root().display().to_owned()),
        directory_bytes: index
            .directory(directory)
            .map_or(0, |node| node.logical_size()),
        child_directories: index
            .directory(directory)
            .into_iter()
            .flat_map(|node| node.children())
            .filter_map(|id| {
                index.directory(*id).map(|node| domain::DirectoryEntry {
                    name: node
                        .path()
                        .display()
                        .rsplit('\\')
                        .next()
                        .unwrap_or_default()
                        .to_owned(),
                    path: node.path().clone(),
                    directory_id: Some(*id),
                    file_identity: None,
                    logical_size: node.logical_size(),
                    allocated_size: node.allocated_size(),
                    last_write_time: 0,
                })
            })
            .collect(),
        page: index.directory_page(
            directory,
            0,
            crate::space_analysis::model::DIRECTORY_PAGE_SIZE,
            None,
        ),
        preview: MapPreview {
            nodes: Vec::new(),
            total_weight: 0,
            omitted: 0,
        },
        selection: None,
        marked: HashSet::new(),
        filter: String::new(),
        page_number: 0,
        zoom: 1.0,
        pan: (0.0, 0.0),
    }
}

fn toggle_group_selection(selected: &mut std::collections::BTreeSet<String>, keys: &[String]) {
    if keys.iter().all(|key| selected.contains(key)) {
        selected.retain(|key| !keys.contains(key));
    } else {
        selected.extend(keys.iter().cloned());
    }
}

pub(crate) fn path_is_within(parent: &str, child: &str) -> bool {
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
            .is_some_and(|rest| rest.starts_with('\\'))
}

fn cleanup_large_file_records(
    index: &ScanIndex,
    removed: &HashSet<FileIdentity>,
) -> Vec<FileRecord> {
    const MIN_BYTES: u64 = 10 * 1024 * 1024;
    let mut files = index
        .iter_files()
        .filter(|file| {
            !removed.contains(&file.identity())
                && file.logical_size().max(file.allocated_size()) >= MIN_BYTES
        })
        .collect::<Vec<_>>();
    files.sort_unstable_by(|left, right| {
        right
            .logical_size()
            .max(right.allocated_size())
            .cmp(&left.logical_size().max(left.allocated_size()))
            .then_with(|| left.path().display().cmp(right.path().display()))
    });
    files
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

#[cfg(test)]
mod tests {
    use domain::normalize_path;
    use domain::{FileIdentity, FileRecord, ScanIndex};
    use std::sync::Arc;

    use super::{ControllerConfig, ControllerEffect, UiController, WorkerMessage};
    use crate::{
        commands::{AnalysisCommand, UiCommand},
        routes::ViewId,
    };

    #[test]
    fn cleanup_confirmation_preserves_system_action_warnings() {
        let index = domain::ScanIndex::new(normalize_path(r"C:\Fixture").unwrap());
        let file = domain::FileRecord::new(
            normalize_path(r"C:\Fixture\item").unwrap(),
            index.root_directory_id(),
            domain::FileIdentity {
                volume_serial: 1,
                file_index: 1,
            },
            1,
            1,
        );
        let mut plan = application::CleanupPlan::default();
        for rule in ["system.hibernate", "system.virtual_memory", "recycle_bin"] {
            let mut candidate = crate::runtime::explicit_file_candidate(&file);
            candidate.rule_id = Some(rule.into());
            plan.items.push(application::CleanupPlanItem {
                candidate,
                estimated_bytes: 1,
                explicit_confirmation: true,
            });
        }
        let warnings = super::cleanup_plan_warnings(&plan).join(" ");
        assert!(warnings.contains("休眠和快速启动"));
        assert!(warnings.contains("重启后才会释放"));
        assert!(warnings.contains("所有本地磁盘上的回收站"));
        assert!(warnings.contains("无法恢复"));
    }

    #[test]
    fn exit_confirmation_cancels_work_and_closes_immediately() {
        let mut controller = super::UiController::new();
        controller.transition(UiCommand::ConfirmExit);
        assert!(!controller.exit_ready());
        controller.transition(UiCommand::RequestExit);
        assert!(controller.snapshot().exit_confirmation);
        controller.transition(UiCommand::CancelExit);
        assert!(!controller.snapshot().exit_confirmation);
        let cancel = application::CancellationToken::default();
        controller.scan_cancel = Some(cancel.clone());
        controller.transition(UiCommand::RequestExit);
        controller.transition(UiCommand::ConfirmExit);
        assert!(cancel.is_cancelled());
        assert!(!controller.snapshot().exit_confirmation);
        assert!(controller.exit_ready());
    }

    fn await_analysis(controller: &mut UiController) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let message = controller
                .worker_receiver
                .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                .expect("analysis worker must finish");
            let ready = matches!(&message, WorkerMessage::AnalysisReady { generation, .. }
                if *generation == controller.analysis_generation);
            controller.apply(message);
            if ready {
                break;
            }
        }
    }

    #[test]
    fn navigation_does_not_need_a_renderer() {
        let mut controller = UiController::with_config(ControllerConfig {
            initial_scan_root: normalize_path(r"C:\").unwrap(),
            data_root: std::env::temp_dir().join("rusten-controller-navigation"),
        });
        controller.dispatch(UiCommand::Navigate(ViewId::Settings));
        assert_eq!(controller.snapshot().state.selected_view, ViewId::Settings);
    }

    #[test]
    fn worker_failure_becomes_visible_state() {
        let mut controller = UiController::with_config(ControllerConfig {
            initial_scan_root: normalize_path(r"C:\").unwrap(),
            data_root: std::env::temp_dir().join("rusten-controller-failure"),
        });
        controller.apply(WorkerMessage::ScanFailed {
            generation: 0,
            error: "fixture failure".into(),
        });
        assert_eq!(
            controller.snapshot().error.as_deref(),
            Some("fixture failure")
        );
    }

    #[test]
    fn drain_applies_worker_messages_without_renderer_types() {
        let mut controller = UiController::new();
        controller
            .sender()
            .send(WorkerMessage::ScanFailed {
                generation: 0,
                error: "queued failure".into(),
            })
            .unwrap();
        assert!(controller.drain());
        assert_eq!(
            controller.snapshot().error.as_deref(),
            Some("queued failure")
        );
        assert!(!controller.drain());
    }

    #[test]
    fn start_scan_dispatch_owns_scan_state_reset_and_generation() {
        let mut controller = UiController::with_config(ControllerConfig {
            initial_scan_root: normalize_path(r"C:\").unwrap(),
            data_root: std::env::temp_dir().join("rusten-controller-scan"),
        });
        controller.selected_directories.insert(r"C:\Temp".into());
        controller
            .selected_large_files
            .insert(domain::FileIdentity {
                volume_serial: 1,
                file_index: 2,
            });

        assert!(matches!(
            controller.transition(UiCommand::StartScan),
            ControllerEffect::Scan { .. }
        ));

        assert_eq!(controller.scan_generation, 1);
        assert!(controller.scan_index().is_none());
        assert!(controller.selected_directories.is_empty());
        assert!(controller.selected_large_files.is_empty());
    }

    #[test]
    fn direct_cleanup_requires_a_nonempty_plan() {
        let mut controller = UiController::new();
        assert!(matches!(
            controller.transition(UiCommand::ExecuteCleanup(domain::PlanId(1))),
            ControllerEffect::None
        ));
        assert!(!controller.cleanup_confirmed);

        let candidate = domain::CleanupCandidate {
            path: normalize_path(r"C:\Temp\fixture.tmp").unwrap(),
            file_identity: None,
            rule_id: None,
            category: None,
            reason: None,
            estimated_size: Some(10),
            quick_clean_eligible: false,
            quick_clean_block_reason: None,
            mode: Some(domain::CleanupMode::Quick),
            risk: Some(domain::RiskLevel::P3),
            confidence: Some(domain::Confidence::High),
            recovery: Some(domain::RecoveryMode::Recycle),
            locked: None,
            path_allowed: None,
            file_identity_unchanged: None,
            last_write_time: None,
        };
        controller
            .scan_index
            .as_mut()
            .and_then(std::sync::Arc::get_mut)
            .unwrap()
            .add_cleanup_candidate(candidate);
        controller.refresh_cleanup_plan();
        assert!(matches!(
            controller.transition(UiCommand::ExecuteCleanup(domain::PlanId(1))),
            ControllerEffect::Cleanup { .. }
        ));
        assert!(!controller.cleanup_confirmed);
        assert_eq!(controller.state.cleanup.selected_count, 1);
    }

    #[test]
    fn direct_cleanup_uses_every_currently_selected_category() {
        let mut controller = UiController::new();
        let mut temp = test_candidate(r"C:\Temp\selected.tmp");
        temp.category = Some("system.temp".into());
        let mut log = test_candidate(r"C:\Temp\selected.log");
        log.category = Some("system.logs".into());
        Arc::get_mut(controller.scan_index.as_mut().unwrap())
            .unwrap()
            .add_cleanup_candidates_after_scan(vec![temp.clone(), log]);
        controller.state.cleanup.selected_categories =
            ["system.temp".into(), "system.logs".into()].into();
        controller.cleanup_plan = Some(crate::runtime::prepare_cleanup_plan_with_explicit(&[temp]));

        let ControllerEffect::Cleanup { plan, .. } =
            controller.transition(UiCommand::ExecuteCleanup(domain::PlanId(1)))
        else {
            panic!("cleanup must start");
        };
        assert_eq!(plan.items.len(), 2);
        assert!(plan.by_category.contains_key("system.temp"));
        assert!(plan.by_category.contains_key("system.logs"));
    }

    #[test]
    fn every_selectable_builtin_category_reaches_execution_and_operation_log() {
        use application::{CleanupActions, CleanupItemResult, CleanupService, OperationLog};
        use std::sync::Mutex;

        struct Actions;
        impl CleanupActions for Actions {
            fn revalidate(
                &self,
                candidate: &domain::CleanupCandidate,
            ) -> Result<domain::CleanupCandidate, CleanupItemResult> {
                Ok(candidate.clone())
            }
            fn move_to_recycle_bin(&mut self, _: &domain::CleanupCandidate) -> Result<u64, String> {
                panic!("cleanup must delete permanently")
            }
            fn backup_then_delete(&mut self, _: &domain::CleanupCandidate) -> Result<u64, String> {
                panic!("cleanup must not create backups")
            }
            fn delete_permanently(
                &mut self,
                candidate: &domain::CleanupCandidate,
            ) -> Result<u64, String> {
                Ok(candidate.estimated_size.unwrap_or(0))
            }
        }
        struct Log(Arc<Mutex<Vec<String>>>);
        impl OperationLog for Log {
            fn append(
                &mut self,
                record: application::OperationRecord,
            ) -> Result<(), application::AppError> {
                self.0.lock().unwrap().push(record.path);
                Ok(())
            }
        }

        let mut controller = UiController::new();
        let keys = crate::runtime::builtin_scan_categories()
            .into_iter()
            .map(|category| category.key)
            .collect::<Vec<_>>();
        assert!(!keys.is_empty());
        let candidates = keys
            .iter()
            .enumerate()
            .map(|(index, key)| {
                let mut candidate = test_candidate(&format!(r"C:\Temp\category-{index}.tmp"));
                candidate.category = Some(key.clone());
                candidate
            })
            .collect::<Vec<_>>();
        Arc::get_mut(controller.scan_index.as_mut().unwrap())
            .unwrap()
            .add_cleanup_candidates_after_scan(candidates);
        controller.state.cleanup.selected_categories.clear();
        for category in &mut controller.state.cleanup.categories {
            category.executable_bytes = Some(42);
        }
        controller.transition(UiCommand::ToggleCleanupGroupSelection(keys.clone()));
        let ControllerEffect::Cleanup { plan, cancel } =
            controller.transition(UiCommand::ExecuteCleanup(domain::PlanId(1)))
        else {
            panic!("cleanup must start");
        };
        assert_eq!(plan.items.len(), keys.len());
        for key in &keys {
            assert!(
                plan.by_category.contains_key(key),
                "missing category: {key}"
            );
        }

        let records = Arc::new(Mutex::new(Vec::new()));
        let result = CleanupService::new(Actions, Log(records.clone()))
            .execute_permanently_with_cancellation(&plan, &cancel, |_| {});
        assert_eq!(result.items.len(), keys.len());
        assert!(
            result
                .items
                .iter()
                .all(|item| item.result == CleanupItemResult::DeletedPermanently)
        );
        assert_eq!(records.lock().unwrap().len(), keys.len());
    }

    #[test]
    fn initial_cleanup_plan_contains_only_default_selected_categories() {
        let mut controller = UiController::new();
        let keys = ["system.temp", "system.hibernate", "system.virtual_memory"];
        let candidates = keys
            .iter()
            .enumerate()
            .map(|(i, key)| {
                let mut candidate = test_candidate(&format!(r"C:\Temp\{i}.tmp"));
                candidate.category = Some((*key).into());
                candidate
            })
            .collect::<Vec<_>>();
        controller.state.cleanup.categories = keys
            .iter()
            .map(|key| crate::state::CleanupCategoryState {
                key: (*key).into(),
                group: "系统".into(),
                group_order: 0,
                name: (*key).into(),
                display_order: 0,
                count: 1,
                estimated_bytes: 42,
                executable_bytes: None,
            })
            .collect();
        controller.apply(WorkerMessage::CleanupPlanReady {
            scan_generation: 0,
            plan_generation: 0,
            plan: crate::runtime::prepare_cleanup_plan_with_explicit(&candidates),
        });
        let plan = controller.cleanup_plan.as_ref().unwrap();
        assert_eq!(plan.items.len(), 1);
        assert_eq!(
            plan.items[0].candidate.category.as_deref(),
            Some("system.temp")
        );
        assert_eq!(controller.snapshot().state.cleanup.selected_count, 1);
        assert_eq!(plan.estimated_bytes, plan.items[0].estimated_bytes);
        assert!(
            controller.state.cleanup.categories[1]
                .executable_bytes
                .is_some()
        );
    }

    #[test]
    fn direct_cleanup_waits_for_scan_plan_and_settings() {
        for busy in 0..3 {
            let mut controller = UiController::new();
            controller.cleanup_plan = Some(application::CleanupPlan {
                items: vec![application::CleanupPlanItem {
                    candidate: test_candidate(r"C:\Temp\waiting.tmp"),
                    estimated_bytes: 42,
                    explicit_confirmation: true,
                }],
                estimated_bytes: 42,
                ..Default::default()
            });
            match busy {
                0 => controller.state.scan.status = crate::state::ScanViewStatus::Scanning,
                1 => controller.cleanup_plan_pending = true,
                _ => controller.settings_before_save = Some(controller.settings.clone()),
            }
            assert!(matches!(
                controller.transition(UiCommand::ExecuteCleanup(domain::PlanId(1))),
                ControllerEffect::None
            ));
            assert!(controller.snapshot().cleanup_confirmation.is_none());
            assert!(controller.cleanup_run.is_none());
            assert!(controller.cleanup_plan.is_some());
        }
    }

    #[test]
    fn execute_cleanup_starts_without_confirmation_and_ignores_double_click() {
        let mut controller = UiController::new();
        Arc::get_mut(controller.scan_index.as_mut().unwrap())
            .unwrap()
            .add_cleanup_candidate(test_candidate(r"C:\Temp\direct.tmp"));
        controller.cleanup_plan = Some(application::CleanupPlan {
            items: vec![application::CleanupPlanItem {
                candidate: test_candidate(r"C:\Temp\direct.tmp"),
                estimated_bytes: 42,
                explicit_confirmation: true,
            }],
            estimated_bytes: 42,
            ..Default::default()
        });
        assert!(matches!(
            controller.transition(UiCommand::ExecuteCleanup(domain::PlanId(1))),
            ControllerEffect::Cleanup { .. }
        ));
        assert!(controller.snapshot().cleanup_confirmation.is_none());
        assert!(matches!(
            controller.transition(UiCommand::ExecuteCleanup(domain::PlanId(1))),
            ControllerEffect::None
        ));
    }

    #[test]
    fn confirm_cleanup_returns_renderer_independent_worker_effect() {
        let mut controller = UiController::new();
        controller.cleanup_plan = Some(application::CleanupPlan {
            items: vec![application::CleanupPlanItem {
                candidate: test_candidate(r"C:\Temp\effect.tmp"),
                estimated_bytes: 42,
                explicit_confirmation: true,
            }],
            estimated_bytes: 42,
            ..Default::default()
        });
        controller.cleanup_confirmed = true;

        let effect = controller.transition(UiCommand::ConfirmCleanup);

        let ControllerEffect::Cleanup { plan, .. } = effect else {
            panic!("cleanup effect")
        };
        assert_eq!(plan.estimated_bytes, 42);
        assert!(
            controller
                .cleanup_run
                .as_ref()
                .is_some_and(|run| !run.finished)
        );
    }

    #[test]
    fn cleanup_worker_result_updates_remaining_plan_without_renderer() {
        let mut controller = UiController::new();
        let candidate = test_candidate(r"C:\Temp\finished.tmp");
        let plan = application::CleanupPlan {
            items: vec![application::CleanupPlanItem {
                candidate,
                estimated_bytes: 42,
                explicit_confirmation: true,
            }],
            estimated_bytes: 42,
            ..Default::default()
        };
        controller.cleanup_plan = Some(plan.clone());
        controller.cleanup_confirmed = true;
        assert!(matches!(
            controller.transition(UiCommand::ConfirmCleanup),
            ControllerEffect::Cleanup { .. }
        ));
        controller.apply(WorkerMessage::CleanupFinished(super::CleanupWorkerResult {
            result: Ok(application::CleanupResult {
                items: vec![application::CleanupResultItem {
                    path: r"C:\Temp\finished.tmp".into(),
                    result: application::CleanupItemResult::MovedToRecycleBin,
                    recoverable: true,
                    released_bytes: 42,
                }],
                released_bytes: 42,
                recoverable: 1,
                ..Default::default()
            }),
            plan,
            volume_before: None,
            volume_after: None,
            recycle_stats: None,
        }));
        assert!(
            controller
                .cleanup_run
                .as_ref()
                .is_some_and(|run| run.finished)
        );
        assert_eq!(controller.cleanup_plan.as_ref().unwrap().items.len(), 0);
        assert_eq!(
            controller.cleanup_result.as_ref().unwrap().released_bytes,
            42
        );
        assert!(controller.cleaned_paths.contains(r"C:\Temp\finished.tmp"));
    }

    #[test]
    fn analysis_navigation_needs_no_renderer() {
        let mut controller = UiController::new();
        let index = std::sync::Arc::get_mut(controller.scan_index.as_mut().unwrap()).unwrap();
        let users = index
            .add_directory(domain::DirectoryNode::new(
                normalize_path(r"C:\Users").unwrap(),
                index.root_directory_id(),
            ))
            .unwrap();
        index.add_directory(domain::DirectoryNode::new(
            normalize_path(r"C:\Users\A").unwrap(),
            users,
        ));
        controller.dispatch(UiCommand::OpenAnalysisDirectory(r"C:\Users\A".into()));
        assert_eq!(controller.space_analysis_path, r"C:\Users\A");
        controller.space_analysis_snapshot.filter = "hidden".into();
        controller.space_analysis_snapshot.depth = 4;
        controller.space_analysis_snapshot.zoom = 2.0;
        controller.dispatch(UiCommand::OpenAnalysisParent);
        assert_eq!(controller.space_analysis_path, r"C:\Users");
        assert!(controller.space_analysis_snapshot.filter.is_empty());
        assert_eq!(controller.space_analysis_snapshot.depth, 1);
        assert_eq!(controller.space_analysis_snapshot.zoom, 1.0);
        controller.dispatch(UiCommand::OpenAnalysisParent);
        let root = controller.space_analysis_path.clone();
        controller.dispatch(UiCommand::OpenAnalysisParent);
        assert_eq!(controller.space_analysis_path, root);
    }

    #[test]
    fn directory_overview_keeps_every_child_and_recursive_size_beyond_map_limit() {
        let mut index = ScanIndex::new(normalize_path(r"C:\Fixture").unwrap());
        for n in 0..300u64 {
            let child = index
                .add_directory(domain::DirectoryNode::new(
                    normalize_path(&format!(r"C:\Fixture\child-{n}")).unwrap(),
                    index.root_directory_id(),
                ))
                .unwrap();
            if n != 0 {
                index.add_file(FileRecord::new(
                    normalize_path(&format!(r"C:\Fixture\child-{n}\data.bin")).unwrap(),
                    child,
                    FileIdentity {
                        volume_serial: 1,
                        file_index: n as u128,
                    },
                    n,
                    n,
                ));
            }
        }
        let snapshot = super::empty_space_snapshot(&index, index.root_directory_id());
        assert_eq!(snapshot.child_directories.len(), 300);
        assert_eq!(
            snapshot
                .child_directories
                .iter()
                .map(|child| child.logical_size)
                .sum::<u64>(),
            (0..300u64).sum::<u64>()
        );
        assert!(
            snapshot
                .child_directories
                .iter()
                .any(|child| child.name == "child-0" && child.logical_size == 0)
        );
        assert_eq!(snapshot.breadcrumbs.len(), 1);
    }

    #[test]
    fn unknown_directory_is_reported_without_changing_scope() {
        let mut controller = UiController::new();
        let original = controller.space_analysis_path.clone();
        controller.dispatch(UiCommand::OpenAnalysisDirectory(r"C:\not-indexed".into()));
        assert_eq!(controller.space_analysis_path, original);
        assert!(
            controller
                .snapshot()
                .error
                .unwrap()
                .contains("目录不在扫描索引中")
        );
    }

    #[test]
    fn pause_resume_and_cancel_confirmation_preserve_the_worker_token() {
        let mut controller = UiController::new();
        let ControllerEffect::Scan { cancel, .. } = controller.transition(UiCommand::StartScan)
        else {
            panic!("scan effect");
        };
        controller.transition(UiCommand::PauseScan);
        assert!(cancel.is_paused());
        controller.transition(UiCommand::ResumeScan);
        assert!(!cancel.is_paused());
        controller.transition(UiCommand::RequestCancelScan);
        assert!(controller.snapshot().cancel_scan_confirmation);
        assert!(!cancel.is_cancelled());
        controller.transition(UiCommand::DismissCancelScan);
        assert!(!controller.snapshot().cancel_scan_confirmation);
        controller.transition(UiCommand::RequestCancelScan);
        controller.transition(UiCommand::CancelScan);
        assert!(cancel.is_cancelled());
    }

    #[test]
    fn zoom_is_bounded_and_fit_resets_pan() {
        let mut controller = UiController::new();
        for _ in 0..100 {
            controller.transition(UiCommand::Analysis(AnalysisCommand::Zoom(true)));
        }
        assert_eq!(controller.snapshot().space_analysis.zoom, 4.0);
        for _ in 0..100 {
            controller.transition(UiCommand::Analysis(AnalysisCommand::Zoom(false)));
        }
        assert_eq!(controller.snapshot().space_analysis.zoom, 0.5);
        controller.transition(UiCommand::Analysis(AnalysisCommand::Pan(30, 20)));
        assert_eq!(controller.snapshot().space_analysis.pan, (30.0, 20.0));
        controller.transition(UiCommand::Analysis(AnalysisCommand::Fit));
        assert_eq!(controller.snapshot().space_analysis.pan, (0.0, 0.0));
        assert_eq!(controller.snapshot().space_analysis.zoom, 1.0);
    }

    #[test]
    fn confirmation_paths_are_paged_and_stale_mark_plans_are_rejected() {
        let mut controller = UiController::new();
        let candidates = (0..90)
            .map(|id| test_candidate(&format!(r"C:\Fixture\file-{id}.bin")))
            .collect::<Vec<_>>();
        let plan = crate::runtime::prepare_cleanup_plan_with_explicit(&candidates);
        controller.apply(WorkerMessage::AnalysisPlanReady {
            scan_generation: 9,
            plan_generation: 9,
            plan: plan.clone(),
        });
        assert!(controller.snapshot().cleanup_confirmation.is_none());
        controller.cleanup_plan = Some(plan);
        controller.cleanup_confirmed = true;
        assert_eq!(
            controller
                .snapshot()
                .cleanup_confirmation
                .unwrap()
                .paths
                .len(),
            80
        );
        controller.transition(UiCommand::SetConfirmationPage(usize::MAX));
        let confirmation = controller.snapshot().cleanup_confirmation.unwrap();
        assert_eq!(confirmation.item_count, 90);
        assert_eq!(confirmation.page, 1);
        assert_eq!(confirmation.paths.len(), 10);
    }

    #[test]
    fn snapshot_tracks_analysis_scope_and_cleanup_confirmation() {
        let mut controller = UiController::new();
        let root_id = controller.scan_index.as_ref().unwrap().root_directory_id();
        controller
            .scan_index
            .as_mut()
            .and_then(std::sync::Arc::get_mut)
            .unwrap()
            .add_directory(domain::DirectoryNode::new(
                normalize_path(r"C:\Users").unwrap(),
                root_id,
            ));
        controller.dispatch(UiCommand::OpenAnalysisDirectory(r"C:\Users".into()));
        await_analysis(&mut controller);
        assert_eq!(controller.snapshot().space_analysis.directory.index(), 1);
        controller.cleanup_plan = Some(application::CleanupPlan {
            items: vec![application::CleanupPlanItem {
                candidate: test_candidate(r"C:\Temp\preview.tmp"),
                estimated_bytes: 42,
                explicit_confirmation: true,
            }],
            estimated_bytes: 42,
            ..Default::default()
        });
        controller.cleanup_confirmed = true;
        assert_eq!(
            controller
                .snapshot()
                .cleanup_confirmation
                .unwrap()
                .estimated_bytes,
            42
        );
        controller.dispatch(UiCommand::CancelCleanup);
        assert!(controller.snapshot().cleanup_confirmation.is_none());
    }

    #[test]
    fn snapshot_uses_cached_analysis_until_navigation_changes_it() {
        let mut controller = UiController::new();
        assert!(
            controller
                .snapshot()
                .space_analysis
                .preview
                .nodes
                .is_empty()
        );
        let index = std::sync::Arc::get_mut(controller.scan_index.as_mut().unwrap()).unwrap();
        let root = index.root_directory_id();
        index.add_file(domain::FileRecord::new(
            normalize_path(r"C:\cache-test.bin").unwrap(),
            root,
            domain::FileIdentity {
                volume_serial: 1,
                file_index: 71,
            },
            17,
            17,
        ));
        assert!(
            controller
                .snapshot()
                .space_analysis
                .preview
                .nodes
                .is_empty()
        );
        controller.dispatch(UiCommand::OpenAnalysisDirectory(r"C:\".into()));
        assert!(
            controller
                .snapshot()
                .space_analysis
                .preview
                .nodes
                .is_empty()
        );
        await_analysis(&mut controller);
        assert_eq!(controller.snapshot().space_analysis.preview.nodes.len(), 1);
    }

    #[test]
    fn directory_total_includes_files_omitted_by_map_preview() {
        let mut controller = UiController::new();
        let index = Arc::get_mut(controller.scan_index.as_mut().unwrap()).unwrap();
        let root = index.root_directory_id();
        for number in 0..600 {
            index.add_file(domain::FileRecord::new(
                normalize_path(&format!("C:\\item-{number}.bin")).unwrap(),
                root,
                FileIdentity {
                    volume_serial: 1,
                    file_index: number + 1,
                },
                17,
                17,
            ));
        }
        controller.dispatch(UiCommand::OpenAnalysisDirectory(r"C:\".into()));
        await_analysis(&mut controller);
        let analysis = controller.snapshot().space_analysis;
        assert!(analysis.preview.omitted > 0);
        assert_eq!(analysis.directory_bytes, 600 * 17);
    }

    #[test]
    fn old_analysis_results_and_failures_cannot_replace_new_navigation() {
        let mut controller = UiController::new();
        let index = std::sync::Arc::get_mut(controller.scan_index.as_mut().unwrap()).unwrap();
        let child = index
            .add_directory(domain::DirectoryNode::new(
                normalize_path(r"C:\Users").unwrap(),
                index.root_directory_id(),
            ))
            .unwrap();
        controller.dispatch(UiCommand::OpenAnalysisDirectory(r"C:\".into()));
        let old_generation = controller.analysis_generation;
        controller.dispatch(UiCommand::OpenAnalysisDirectory(r"C:\Users".into()));
        await_analysis(&mut controller);
        let mut stale_snapshot = controller.snapshot().space_analysis;
        stale_snapshot.directory = controller.scan_index().unwrap().root_directory_id();
        controller.apply(WorkerMessage::AnalysisReady {
            generation: old_generation,
            snapshot: stale_snapshot,
        });
        controller.apply(WorkerMessage::AnalysisFailed {
            generation: old_generation,
            error: "stale failure".into(),
        });
        assert_eq!(controller.snapshot().space_analysis.directory, child);
        assert!(controller.snapshot().error.is_none());
    }

    #[test]
    fn starting_scan_invalidates_pending_analysis() {
        let mut controller = UiController::new();
        controller.dispatch(UiCommand::OpenAnalysisDirectory(r"C:\".into()));
        let generation = controller.analysis_generation;
        let mut stale_snapshot = controller.snapshot().space_analysis;
        stale_snapshot.filter = "stale result".into();
        // Exercise the state transition without starting a real system scan.
        controller.transition(UiCommand::StartScan);
        controller.apply(WorkerMessage::AnalysisReady {
            generation,
            snapshot: stale_snapshot,
        });
        controller.apply(WorkerMessage::AnalysisFailed {
            generation,
            error: "stale failure".into(),
        });
        assert!(controller.snapshot().space_analysis.filter.is_empty());
        assert!(controller.snapshot().error.is_none());
    }

    #[test]
    fn analysis_worker_failure_becomes_visible() {
        let mut controller = UiController::new();
        controller.apply(WorkerMessage::AnalysisFailed {
            generation: controller.analysis_generation,
            error: "analysis failed".into(),
        });
        assert_eq!(
            controller.snapshot().error.as_deref(),
            Some("analysis failed")
        );
    }

    #[test]
    fn missing_production_settings_path_is_an_error() {
        let mut controller = UiController::new();
        controller.apply(WorkerMessage::SettingsSaved(Err(
            "LOCALAPPDATA is unavailable".into(),
        )));
        assert!(controller.snapshot().error.is_some());
    }

    #[test]
    fn panicking_worker_sends_terminal_failure() {
        let (sender, receiver) = std::sync::mpsc::channel();
        super::spawn_worker(
            sender,
            WorkerMessage::CleanupFailed("worker panicked".into()),
            || panic!("fixture panic"),
        );
        assert!(matches!(
            receiver.recv_timeout(std::time::Duration::from_secs(10)).unwrap(),
            WorkerMessage::CleanupFailed(error) if error == "worker panicked"
        ));
    }

    #[test]
    fn confirm_without_review_does_not_return_cleanup_effect() {
        let mut controller = UiController::new();
        controller.cleanup_plan = Some(application::CleanupPlan {
            items: vec![application::CleanupPlanItem {
                candidate: test_candidate(r"C:\Temp\unreviewed.tmp"),
                estimated_bytes: 42,
                explicit_confirmation: true,
            }],
            estimated_bytes: 42,
            ..Default::default()
        });
        assert!(matches!(
            controller.transition(UiCommand::ConfirmCleanup),
            ControllerEffect::None
        ));
        assert!(controller.cleanup_run.is_none());
    }

    #[test]
    fn stop_cleanup_cancels_worker_and_blocks_selection_changes() {
        let mut controller = UiController::new();
        Arc::get_mut(controller.scan_index.as_mut().unwrap())
            .unwrap()
            .add_cleanup_candidate(test_candidate(r"C:\Temp\reviewed.tmp"));
        controller.cleanup_plan = Some(application::CleanupPlan {
            items: vec![application::CleanupPlanItem {
                candidate: test_candidate(r"C:\Temp\reviewed.tmp"),
                estimated_bytes: 42,
                explicit_confirmation: true,
            }],
            estimated_bytes: 42,
            ..Default::default()
        });
        let ControllerEffect::Cleanup { cancel, .. } =
            controller.transition(UiCommand::ExecuteCleanup(domain::PlanId(1)))
        else {
            panic!("cleanup effect expected")
        };
        controller.transition(UiCommand::ToggleDirectorySelection(r"C:\Temp".into()));
        assert!(controller.selected_directories.is_empty());
        controller.transition(UiCommand::StopCleanup);
        assert!(cancel.is_cancelled());
        assert!(controller.snapshot().cleanup_stopping);
    }

    #[test]
    fn cleanup_finished_sets_snapshot_progress_and_error() {
        let mut controller = UiController::new();
        controller.apply(WorkerMessage::CleanupFinished(super::CleanupWorkerResult {
            result: Err("fixture cleanup failure".into()),
            plan: application::CleanupPlan::default(),
            volume_before: None,
            volume_after: None,
            recycle_stats: None,
        }));
        assert!(controller.snapshot().cleanup_progress.unwrap().finished);
        assert_eq!(
            controller.snapshot().error.as_deref(),
            Some("fixture cleanup failure")
        );
    }

    #[test]
    fn failed_settings_save_rolls_back_toggle() {
        let root = std::env::temp_dir().join(format!(
            "rusten-controller-invalid-settings-{}",
            std::process::id()
        ));
        std::fs::write(&root, b"file blocks settings directory").unwrap();
        let mut controller = UiController::with_config(ControllerConfig {
            initial_scan_root: normalize_path(r"C:\").unwrap(),
            data_root: root.clone(),
        });
        let before = controller.settings.theme;
        controller.dispatch(UiCommand::SetTheme(crate::theme::ThemeMode::Dark));
        let result = controller
            .worker_receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        controller.apply(result);
        assert_eq!(controller.settings.theme, before);
        assert!(controller.snapshot().error.is_some());
        std::fs::remove_file(root).unwrap();
    }

    #[test]
    fn stale_scan_worker_messages_do_not_replace_current_scan() {
        let mut controller = UiController::new();
        controller.transition(UiCommand::StartScan);
        controller.transition(UiCommand::CancelScan);
        controller.transition(UiCommand::StartScan);
        controller.apply(WorkerMessage::ScanFailed {
            generation: 1,
            error: "old scan".into(),
        });
        assert!(controller.snapshot().error.is_none());
        assert_eq!(
            controller.state.scan.status,
            crate::state::ScanViewStatus::Scanning
        );
    }

    #[test]
    fn standalone_dispatch_uses_configured_roots_for_workers() {
        let data_root =
            std::env::temp_dir().join(format!("rusten-controller-worker-{}", std::process::id()));
        let scan_root = data_root.join("scan");
        std::fs::create_dir_all(&scan_root).unwrap();
        let mut controller = UiController::with_config(ControllerConfig {
            initial_scan_root: normalize_path(scan_root.to_str().unwrap()).unwrap(),
            data_root: data_root.clone(),
        });
        controller.dispatch(UiCommand::StartScan);
        for _ in 0..100 {
            if controller.drain()
                && controller.snapshot().state.scan.status != crate::state::ScanViewStatus::Scanning
            {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_ne!(
            controller.snapshot().state.scan.status,
            crate::state::ScanViewStatus::Scanning
        );
        assert!(controller.scan_index().is_some());
        std::fs::remove_dir_all(data_root).unwrap();
    }

    #[test]
    fn isolated_controller_rejects_cleanup_outside_scan_root() {
        let controller = UiController::with_config(ControllerConfig {
            initial_scan_root: normalize_path(r"C:\Fixture").unwrap(),
            data_root: std::env::temp_dir().join("rusten-isolated-cleanup"),
        });
        let plan = application::CleanupPlan {
            items: vec![application::CleanupPlanItem {
                candidate: test_candidate(r"C:\Outside\target.tmp"),
                estimated_bytes: 42,
                explicit_confirmation: true,
            }],
            ..Default::default()
        };
        assert!(controller.validate_plan_scope(&plan).is_err());
    }

    #[test]
    fn production_scope_keeps_explicit_cleanup_outside_system_drive() {
        let controller = UiController::new();
        let plan = application::CleanupPlan {
            items: vec![application::CleanupPlanItem {
                candidate: test_candidate(r"D:\Explicit\target.tmp"),
                estimated_bytes: 42,
                explicit_confirmation: true,
            }],
            ..Default::default()
        };
        assert!(controller.validate_plan_scope(&plan).is_ok());
    }

    #[test]
    fn category_and_group_selection_recompute_the_executable_plan() {
        let mut controller = UiController::new();
        let mut first = test_candidate(r"C:\Temp\first.tmp");
        first.category = Some("system.cache".into());
        let mut second = test_candidate(r"C:\Temp\second.tmp");
        second.category = Some("system.temp".into());
        Arc::get_mut(controller.scan_index.as_mut().unwrap())
            .unwrap()
            .add_cleanup_candidates_after_scan(vec![first, second]);
        for category in &mut controller.state.cleanup.categories {
            category.executable_bytes = Some(42);
        }
        controller.state.cleanup.selected_categories =
            ["system.cache".into(), "system.temp".into()].into();
        controller.refresh_cleanup_plan();
        assert_eq!(controller.snapshot().state.cleanup.selected_count, 2);
        controller.transition(UiCommand::ToggleCleanupCategory("system.cache".into()));
        assert_eq!(controller.snapshot().state.cleanup.selected_count, 1);
        controller.transition(UiCommand::ToggleCleanupGroupSelection(vec![
            "system.cache".into(),
            "system.temp".into(),
        ]));
        assert_eq!(controller.snapshot().state.cleanup.selected_count, 2);
        controller.transition(UiCommand::ToggleCleanupGroupSelection(vec![
            "system.cache".into(),
            "system.temp".into(),
        ]));
        assert_eq!(controller.snapshot().state.cleanup.selected_count, 0);
    }

    fn test_candidate(path: &str) -> domain::CleanupCandidate {
        domain::CleanupCandidate {
            path: normalize_path(path).unwrap(),
            file_identity: None,
            rule_id: None,
            category: None,
            reason: None,
            estimated_size: Some(42),
            quick_clean_eligible: false,
            quick_clean_block_reason: None,
            mode: Some(domain::CleanupMode::Manual),
            risk: Some(domain::RiskLevel::P2),
            confidence: Some(domain::Confidence::High),
            recovery: Some(domain::RecoveryMode::Recycle),
            locked: Some(false),
            path_allowed: Some(true),
            file_identity_unchanged: Some(true),
            last_write_time: None,
        }
    }
}
