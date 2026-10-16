use crate::runtime::{ScanProgress, ScanReport};
use crate::{commands::UiCommand, routes::ViewId};
use domain::{ScanEngine, ScanPhase};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScanViewStatus {
    NotScanned,
    Scanning,
    Paused,
    Partial,
    Complete,
    Canceled,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScanViewState {
    pub engine: ScanEngine,
    pub phase: ScanPhase,
    pub status: ScanViewStatus,
    pub files: u64,
    pub discovered_bytes: u64,
    pub progress_percent: u8,
    pub current_path: Option<String>,
    pub warnings: u64,
    pub total_bytes: u64,
    pub available_bytes: u64,
}

impl Default for ScanViewState {
    fn default() -> Self {
        Self {
            engine: ScanEngine::Uffs,
            phase: ScanPhase::Starting,
            status: ScanViewStatus::NotScanned,
            files: 0,
            discovered_bytes: 0,
            progress_percent: 0,
            current_path: None,
            warnings: 0,
            total_bytes: 0,
            available_bytes: 0,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CleanupCategoryState {
    pub key: String,
    pub group: String,
    pub group_order: u32,
    pub name: String,
    pub display_order: u32,
    pub count: u64,
    pub estimated_bytes: u64,
    pub executable_bytes: Option<u64>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CleanupViewState {
    pub selected_count: usize,
    pub estimated_bytes: u64,
    pub blocked_count: usize,
    pub categories: Vec<CleanupCategoryState>,
    pub selected_categories: std::collections::BTreeSet<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RecoveryViewState {
    pub recoverable_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppState {
    pub scan: ScanViewState,
    pub selected_view: ViewId,
    pub cleanup: CleanupViewState,
    pub recovery: RecoveryViewState,
    pub scan_error: Option<String>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            scan: ScanViewState::default(),
            selected_view: ViewId::Home,
            cleanup: CleanupViewState::default(),
            recovery: RecoveryViewState::default(),
            scan_error: None,
        }
    }
}

impl AppState {
    pub fn apply_cleanup_availability(
        &mut self,
        by_category: &std::collections::BTreeMap<String, u64>,
    ) {
        for category in &mut self.cleanup.categories {
            category.executable_bytes = by_category.get(&category.key).copied();
        }
        self.cleanup.selected_categories = self
            .cleanup
            .categories
            .iter()
            .filter(|category| {
                category.executable_bytes.is_some()
                    && !matches!(
                        category.key.as_str(),
                        "system.hibernate" | "system.virtual_memory"
                    )
            })
            .map(|category| category.key.clone())
            .collect();
    }

    pub fn apply_remaining_selected_cleanup(
        &mut self,
        by_category: &std::collections::BTreeMap<String, u64>,
    ) {
        let selected = self.cleanup.selected_categories.clone();
        for category in &mut self.cleanup.categories {
            if selected.contains(&category.key) {
                category.executable_bytes = by_category.get(&category.key).copied();
            }
        }
        self.cleanup
            .selected_categories
            .retain(|key| by_category.contains_key(key));
    }

    pub fn apply_report(&mut self, report: &ScanReport) {
        self.scan.status = match report.status {
            domain::ScanStatus::Complete => ScanViewStatus::Complete,
            domain::ScanStatus::Partial => ScanViewStatus::Partial,
            domain::ScanStatus::Canceled => ScanViewStatus::Canceled,
            domain::ScanStatus::InProgress => ScanViewStatus::Scanning,
        };
        self.scan.engine = report.engine;
        self.scan.phase = ScanPhase::Finalizing;
        self.scan.files = report.files;
        self.scan.discovered_bytes = report.logical_bytes;
        self.scan.progress_percent = 100;
        self.scan.current_path = None;
        self.scan.warnings = report.warnings;
        self.scan.total_bytes = report.total_bytes;
        self.scan.available_bytes = report.available_bytes;
        self.cleanup.selected_count = report.candidate_count;
        self.cleanup.estimated_bytes = report.estimated_bytes;
        self.cleanup.categories = report
            .categories
            .iter()
            .map(|category| CleanupCategoryState {
                key: category.key.clone(),
                group: category.group.clone(),
                group_order: category.group_order,
                name: category.name.clone(),
                display_order: category.display_order,
                count: category.count,
                estimated_bytes: category.estimated_bytes,
                executable_bytes: None,
            })
            .collect();
        self.cleanup.selected_categories = self
            .cleanup
            .categories
            .iter()
            .map(|category| category.key.clone())
            .collect();
    }

    pub fn apply_progress(&mut self, progress: &ScanProgress) {
        self.scan.status = ScanViewStatus::Scanning;
        self.scan.engine = progress.engine;
        self.scan.phase = progress.phase;
        self.scan.files = progress.files;
        self.scan.discovered_bytes = progress.logical_bytes;
        self.scan.progress_percent = progress.progress_percent;
        self.scan.current_path = progress.current_path.clone();
        self.scan.warnings = progress.warnings;
        self.cleanup.categories = progress
            .categories
            .iter()
            .map(|category| CleanupCategoryState {
                key: category.key.clone(),
                group: category.group.clone(),
                group_order: category.group_order,
                name: category.name.clone(),
                display_order: category.display_order,
                count: category.count,
                estimated_bytes: category.estimated_bytes,
                executable_bytes: None,
            })
            .collect();
    }

    pub fn apply(&mut self, command: UiCommand) {
        match command {
            UiCommand::Navigate(view) => self.selected_view = view,
            UiCommand::StartScan => {
                self.scan.status = ScanViewStatus::Scanning;
                self.scan.phase = ScanPhase::Starting;
                self.scan.files = 0;
                self.scan.discovered_bytes = 0;
                self.scan.progress_percent = 0;
                self.scan.current_path = None;
                self.scan.warnings = 0;
                let categories = self
                    .cleanup
                    .categories
                    .iter()
                    .cloned()
                    .map(|mut category| {
                        category.count = 0;
                        category.estimated_bytes = 0;
                        category.executable_bytes = None;
                        category
                    })
                    .collect::<Vec<_>>();
                let selected_categories = categories
                    .iter()
                    .map(|category| category.key.clone())
                    .collect();
                self.cleanup = CleanupViewState {
                    categories,
                    selected_categories,
                    ..CleanupViewState::default()
                };
            }
            UiCommand::ResumeScan => self.scan.status = ScanViewStatus::Scanning,
            UiCommand::PauseScan if self.scan.status == ScanViewStatus::Scanning => {
                self.scan.status = ScanViewStatus::Paused
            }
            UiCommand::CancelScan => self.scan.status = ScanViewStatus::Canceled,
            UiCommand::CreateQuickCleanPreview => self.selected_view = ViewId::Cleanup,
            UiCommand::RestoreOperation(_) => self.selected_view = ViewId::Recovery,
            UiCommand::ToggleCleanupCategory(category) => {
                if !self.cleanup.selected_categories.remove(&category) {
                    self.cleanup.selected_categories.insert(category);
                }
            }
            UiCommand::InspectCleanupCategory(_) => {}
            UiCommand::ToggleCleanupGroup(_) | UiCommand::ToggleCleanupGroupSelection(_) => {}
            UiCommand::ToggleDirectory(_) => {}
            UiCommand::ToggleDirectorySelection(_) => {}
            UiCommand::OpenAnalysisDirectory(_) | UiCommand::OpenAnalysisParent => {}
            UiCommand::ToggleLargeFileSelection(_) => {}
            UiCommand::ToggleAutoClean | UiCommand::ToggleBackup => {}
            UiCommand::PurgeExpiredRecovery => {}
            UiCommand::ExecuteCleanup(_)
            | UiCommand::ConfirmCleanup
            | UiCommand::CancelCleanup
            | UiCommand::StopCleanup
            | UiCommand::PauseScan => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AppState, ScanViewStatus};
    use crate::{
        commands::UiCommand,
        routes::ViewId,
        runtime::{ScanCategory, ScanProgress, ScanReport},
    };
    use domain::{ScanEngine, ScanIndex, ScanPhase, ScanStatus, normalize_path};

    #[test]
    fn commands_and_events_update_rendered_state() {
        let mut state = AppState::default();
        state.apply(UiCommand::Navigate(ViewId::Recovery));
        state.apply(UiCommand::StartScan);
        assert_eq!(state.selected_view, ViewId::Recovery);
        assert_eq!(state.scan.status, ScanViewStatus::Scanning);

        state.apply(UiCommand::CancelScan);
        assert_eq!(state.scan.status, ScanViewStatus::Canceled);
    }

    #[test]
    fn scan_progress_keeps_the_reported_percentage() {
        let mut state = AppState::default();
        state.apply_progress(&ScanProgress {
            engine: ScanEngine::Uffs,
            phase: ScanPhase::Enumerating,
            files: 42,
            logical_bytes: 100,
            warnings: 1,
            progress_percent: 37,
            current_path: Some(r"C:\Windows\Temp\file.tmp".into()),
            categories: Vec::new(),
        });

        assert_eq!(state.scan.progress_percent, 37);
    }

    #[test]
    fn completed_scan_preserves_nonzero_category_size() {
        let mut state = AppState::default();
        state.apply_report(&ScanReport {
            index: ScanIndex::new(normalize_path(r"C:\").unwrap()),
            engine: ScanEngine::Uffs,
            status: ScanStatus::Complete,
            files: 1,
            logical_bytes: 2048,
            candidate_count: 1,
            estimated_bytes: 2048,
            warnings: 0,
            total_bytes: 4096,
            available_bytes: 2048,
            categories: vec![ScanCategory {
                key: "system.temp".into(),
                group: "系统垃圾".into(),
                group_order: 1,
                name: "临时文件".into(),
                display_order: 1,
                count: 1,
                estimated_bytes: 2048,
            }],
        });

        assert_eq!(state.scan.status, ScanViewStatus::Complete);
        assert_eq!(state.cleanup.estimated_bytes, 2048);
        assert_eq!(state.cleanup.categories[0].estimated_bytes, 2048);
    }

    #[test]
    fn cleanup_sources_including_recycle_bin_are_selected_by_default() {
        let mut state = AppState::default();
        state.cleanup.categories = [
            "system.temp",
            "recycle_bin",
            "zero_size",
            "statistics_only",
            "system.hibernate",
            "system.virtual_memory",
        ]
        .into_iter()
        .map(|key| super::CleanupCategoryState {
            key: key.into(),
            group: "测试".into(),
            group_order: 1,
            name: key.into(),
            display_order: 1,
            count: 1,
            estimated_bytes: 0,
            executable_bytes: None,
        })
        .collect();
        state.apply_cleanup_availability(
            &[
                ("system.temp".into(), 100),
                ("recycle_bin".into(), 900),
                ("zero_size".into(), 0),
                ("system.hibernate".into(), 2_000),
                ("system.virtual_memory".into(), 3_000),
            ]
            .into_iter()
            .collect(),
        );
        assert_eq!(state.cleanup.categories.len(), 6);
        assert_eq!(state.cleanup.categories[2].executable_bytes, Some(0));
        assert!(state.cleanup.selected_categories.contains("zero_size"));
        assert_eq!(state.cleanup.categories[3].executable_bytes, None);
        assert!(
            !state
                .cleanup
                .selected_categories
                .contains("statistics_only")
        );
        assert!(state.cleanup.selected_categories.contains("system.temp"));
        assert!(state.cleanup.selected_categories.contains("recycle_bin"));
        assert_eq!(state.cleanup.categories[4].executable_bytes, Some(2_000));
        assert_eq!(state.cleanup.categories[5].executable_bytes, Some(3_000));
        assert!(
            !state
                .cleanup
                .selected_categories
                .contains("system.hibernate")
        );
        assert!(
            !state
                .cleanup
                .selected_categories
                .contains("system.virtual_memory")
        );
        state.apply(UiCommand::ToggleCleanupCategory("recycle_bin".into()));
        assert!(!state.cleanup.selected_categories.contains("recycle_bin"));
        state.apply(UiCommand::ToggleCleanupCategory("recycle_bin".into()));
        assert!(state.cleanup.selected_categories.contains("recycle_bin"));
    }

    #[test]
    fn cleanup_completion_updates_selected_categories_without_hiding_unselected_work() {
        let mut state = AppState::default();
        state.cleanup.categories = ["deleted", "failed", "unselected"]
            .into_iter()
            .map(|key| super::CleanupCategoryState {
                key: key.into(),
                group: "测试".into(),
                group_order: 1,
                name: key.into(),
                display_order: 1,
                count: 1,
                estimated_bytes: 10,
                executable_bytes: Some(10),
            })
            .collect();
        state.cleanup.selected_categories =
            ["deleted".into(), "failed".into()].into_iter().collect();

        state.apply_remaining_selected_cleanup(&[("failed".into(), 0)].into_iter().collect());

        assert_eq!(state.cleanup.categories[0].executable_bytes, None);
        assert_eq!(state.cleanup.categories[1].executable_bytes, Some(0));
        assert_eq!(state.cleanup.categories[2].executable_bytes, Some(10));
        assert_eq!(
            state.cleanup.selected_categories,
            ["failed".into()].into_iter().collect()
        );
    }

    #[test]
    fn cleanup_category_selection_can_be_removed_and_restored() {
        let mut state = AppState::default();
        state
            .cleanup
            .selected_categories
            .insert("system.temp".into());

        state.apply(UiCommand::ToggleCleanupCategory("system.temp".into()));
        assert!(!state.cleanup.selected_categories.contains("system.temp"));

        state.apply(UiCommand::ToggleCleanupCategory("system.temp".into()));
        assert!(state.cleanup.selected_categories.contains("system.temp"));
    }

    #[test]
    fn starting_new_scan_clears_previous_category_totals() {
        let mut state = AppState::default();
        state.cleanup.estimated_bytes = 2048;
        state.cleanup.selected_count = 1;
        state.cleanup.categories.push(super::CleanupCategoryState {
            key: "system.temp".into(),
            group: "系统垃圾".into(),
            group_order: 1,
            name: "临时文件".into(),
            display_order: 1,
            count: 1,
            estimated_bytes: 2048,
            executable_bytes: None,
        });
        state
            .cleanup
            .selected_categories
            .insert("system.temp".into());

        state.apply(UiCommand::StartScan);

        assert_eq!(state.cleanup.estimated_bytes, 0);
        assert_eq!(state.cleanup.selected_count, 0);
        assert_eq!(state.cleanup.categories.len(), 1);
        assert_eq!(state.cleanup.categories[0].count, 0);
        assert_eq!(state.cleanup.categories[0].estimated_bytes, 0);
        assert!(state.cleanup.selected_categories.contains("system.temp"));
    }
}
