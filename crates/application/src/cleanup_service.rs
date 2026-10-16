use std::{
    collections::BTreeMap,
    sync::Mutex,
    time::{SystemTime, UNIX_EPOCH},
};

use domain::{
    CleanupCandidate, CleanupMode, FileIdentity, RecoveryMode, can_enter_quick_clean,
    is_explicit_cleanup_allowed, is_forbidden_path,
};

use crate::{OperationLog, OperationRecord};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CleanupPlanItem {
    pub candidate: CleanupCandidate,
    pub estimated_bytes: u64,
    pub explicit_confirmation: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CleanupPlan {
    pub items: Vec<CleanupPlanItem>,
    pub by_category: BTreeMap<String, u64>,
    pub estimated_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CleanupItemResult {
    DeletedPermanently,
    MovedToRecycleBin,
    RecycleBinEmptied,
    RestartRequired,
    BackedUp,
    SkippedLocked,
    SkippedPermission,
    SkippedChanged,
    SkippedForbidden,
    Failed(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CleanupResultItem {
    pub path: String,
    pub result: CleanupItemResult,
    pub recoverable: bool,
    pub released_bytes: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CleanupResult {
    pub items: Vec<CleanupResultItem>,
    pub released_bytes: u64,
    pub skipped: u64,
    pub failed: u64,
    pub recoverable: u64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CleanupProgress {
    pub processed: usize,
    pub total: usize,
    pub current_path: Option<String>,
    pub released_bytes: u64,
    pub skipped: u64,
    pub failed: u64,
}

pub trait ActionExecutor {
    fn preview(&self, candidates: &[CleanupCandidate]) -> CleanupPlan;
    fn execute(&self, plan: &CleanupPlan) -> CleanupResult;
}

pub trait CleanupActions: Send {
    fn revalidate(
        &self,
        candidate: &CleanupCandidate,
    ) -> Result<CleanupCandidate, CleanupItemResult>;
    fn move_to_recycle_bin(&mut self, candidate: &CleanupCandidate) -> Result<u64, String>;
    fn backup_then_delete(&mut self, candidate: &CleanupCandidate) -> Result<u64, String>;
    fn delete_permanently(&mut self, _candidate: &CleanupCandidate) -> Result<u64, String> {
        Err("permanent cleanup is not supported by this executor".into())
    }
    fn take_backup_id(&mut self) -> Option<String> {
        None
    }
}

pub fn preview_cleanup(candidates: &[CleanupCandidate]) -> CleanupPlan {
    let mut plan = CleanupPlan::default();
    for candidate in candidates
        .iter()
        .filter(|candidate| can_enter_quick_clean(candidate))
    {
        let bytes = candidate.estimated_size.unwrap_or(0);
        let category = candidate
            .category
            .clone()
            .unwrap_or_else(|| "未分类".into());
        *plan.by_category.entry(category).or_default() += bytes;
        plan.estimated_bytes = plan.estimated_bytes.saturating_add(bytes);
        plan.items.push(CleanupPlanItem {
            candidate: candidate.clone(),
            estimated_bytes: bytes,
            explicit_confirmation: false,
        });
    }
    plan
}

pub fn preview_cleanup_with_explicit(candidates: &[CleanupCandidate]) -> CleanupPlan {
    let mut plan = CleanupPlan::default();
    for candidate in candidates {
        let explicit_confirmation = candidate.mode == Some(CleanupMode::Manual);
        if !explicit_confirmation && !can_enter_quick_clean(candidate) {
            continue;
        }
        let bytes = candidate.estimated_size.unwrap_or(0);
        let category = candidate
            .category
            .clone()
            .unwrap_or_else(|| "未分类".into());
        *plan.by_category.entry(category).or_default() += bytes;
        plan.estimated_bytes = plan.estimated_bytes.saturating_add(bytes);
        plan.items.push(CleanupPlanItem {
            candidate: candidate.clone(),
            estimated_bytes: bytes,
            explicit_confirmation,
        });
    }
    plan
}

pub struct CleanupService<A, L> {
    actions: Mutex<A>,
    log: Mutex<L>,
}

impl<A, L> CleanupService<A, L> {
    pub fn new(actions: A, log: L) -> Self {
        Self {
            actions: Mutex::new(actions),
            log: Mutex::new(log),
        }
    }
}

impl<A, L> CleanupService<A, L>
where
    A: CleanupActions,
    L: OperationLog,
{
    pub fn execute_with_progress<F>(&self, plan: &CleanupPlan, on_progress: F) -> CleanupResult
    where
        F: FnMut(CleanupProgress),
    {
        self.execute_with_cancellation(plan, &crate::CancellationToken::default(), on_progress)
    }

    pub fn execute_with_cancellation<F>(
        &self,
        plan: &CleanupPlan,
        cancel: &crate::CancellationToken,
        on_progress: F,
    ) -> CleanupResult
    where
        F: FnMut(CleanupProgress),
    {
        self.execute_plan(plan, cancel, false, on_progress)
    }

    pub fn execute_permanently_with_cancellation<F>(
        &self,
        plan: &CleanupPlan,
        cancel: &crate::CancellationToken,
        on_progress: F,
    ) -> CleanupResult
    where
        F: FnMut(CleanupProgress),
    {
        self.execute_plan(plan, cancel, true, on_progress)
    }

    fn execute_plan<F>(
        &self,
        plan: &CleanupPlan,
        cancel: &crate::CancellationToken,
        permanent: bool,
        mut on_progress: F,
    ) -> CleanupResult
    where
        F: FnMut(CleanupProgress),
    {
        let mut result = CleanupResult::default();
        let total = plan.items.len();
        on_progress(CleanupProgress {
            total,
            ..CleanupProgress::default()
        });
        let Ok(mut actions) = self.actions.lock() else {
            return failed_result(plan, "cleanup action lock unavailable");
        };
        let Ok(mut log) = self.log.lock() else {
            return failed_result(plan, "operation log lock unavailable");
        };
        for (index, item) in plan.items.iter().enumerate() {
            if cancel.is_cancelled() {
                break;
            }
            let path = item.candidate.path.display().to_owned();
            on_progress(CleanupProgress {
                processed: index,
                total,
                current_path: Some(path.clone()),
                released_bytes: result.released_bytes,
                skipped: result.skipped,
                failed: result.failed,
            });
            let _ = actions.take_backup_id();
            let (outcome, released) = match actions.revalidate(&item.candidate) {
                Err(outcome) => (outcome, 0),
                Ok(candidate)
                    if is_forbidden_path(&candidate.path)
                        && candidate.rule_id.as_deref() != Some("recycle_bin")
                        && !is_fixed_system_storage_cleanup(&candidate)
                        && !(item.explicit_confirmation
                            && is_explicit_cleanup_allowed(&candidate.path)) =>
                {
                    (CleanupItemResult::SkippedForbidden, 0)
                }
                Ok(candidate)
                    if item.explicit_confirmation && !can_enter_explicit_cleanup(&candidate) =>
                {
                    (CleanupItemResult::SkippedChanged, 0)
                }
                Ok(candidate)
                    if !item.explicit_confirmation && !can_enter_quick_clean(&candidate) =>
                {
                    (CleanupItemResult::SkippedChanged, 0)
                }
                Ok(candidate) if permanent => actions
                    .delete_permanently(&candidate)
                    .map(|bytes| {
                        (
                            if candidate.rule_id.as_deref() == Some("system.virtual_memory") {
                                CleanupItemResult::RestartRequired
                            } else if candidate.rule_id.as_deref() == Some("recycle_bin") {
                                CleanupItemResult::RecycleBinEmptied
                            } else {
                                CleanupItemResult::DeletedPermanently
                            },
                            if candidate.rule_id.as_deref() == Some("system.virtual_memory") {
                                0
                            } else {
                                bytes
                            },
                        )
                    })
                    .unwrap_or_else(|error| (CleanupItemResult::Failed(error), 0)),
                Ok(candidate) => match candidate.recovery.unwrap_or(RecoveryMode::Recycle) {
                    RecoveryMode::Recycle => actions
                        .move_to_recycle_bin(&candidate)
                        .map(|bytes| {
                            let outcome = if candidate.rule_id.as_deref() == Some("recycle_bin") {
                                CleanupItemResult::RecycleBinEmptied
                            } else {
                                CleanupItemResult::MovedToRecycleBin
                            };
                            (outcome, bytes)
                        })
                        .unwrap_or_else(|error| (CleanupItemResult::Failed(error), 0)),
                    RecoveryMode::BackupThenDelete => actions
                        .backup_then_delete(&candidate)
                        .map(|bytes| (CleanupItemResult::BackedUp, bytes))
                        .unwrap_or_else(|error| (CleanupItemResult::Failed(error), 0)),
                    RecoveryMode::DeleteOnly | RecoveryMode::ShredOnce => {
                        (CleanupItemResult::SkippedChanged, 0)
                    }
                },
            };
            let backup_id = actions.take_backup_id();
            let recoverable = backup_id.is_some()
                || matches!(
                    outcome,
                    CleanupItemResult::MovedToRecycleBin | CleanupItemResult::BackedUp
                );
            if matches!(outcome, CleanupItemResult::Failed(_)) {
                result.failed += 1;
            }
            if matches!(
                outcome,
                CleanupItemResult::SkippedLocked
                    | CleanupItemResult::SkippedPermission
                    | CleanupItemResult::SkippedChanged
                    | CleanupItemResult::SkippedForbidden
            ) {
                result.skipped += 1;
            }
            if recoverable {
                result.recoverable += 1;
            }
            result.released_bytes = result.released_bytes.saturating_add(released);
            let _ = log.append(operation_record(
                &item.candidate,
                &outcome,
                released,
                backup_id,
            ));
            result.items.push(CleanupResultItem {
                path,
                result: outcome,
                recoverable,
                released_bytes: released,
            });
            on_progress(CleanupProgress {
                processed: index + 1,
                total,
                current_path: None,
                released_bytes: result.released_bytes,
                skipped: result.skipped,
                failed: result.failed,
            });
        }
        result
    }
}

impl<A, L> ActionExecutor for CleanupService<A, L>
where
    A: CleanupActions,
    L: OperationLog,
{
    fn preview(&self, candidates: &[CleanupCandidate]) -> CleanupPlan {
        preview_cleanup(candidates)
    }

    fn execute(&self, plan: &CleanupPlan) -> CleanupResult {
        self.execute_with_progress(plan, |_| {})
    }
}

fn can_enter_explicit_cleanup(candidate: &CleanupCandidate) -> bool {
    let fixed_system_storage = is_fixed_system_storage_cleanup(candidate);
    candidate.mode == Some(CleanupMode::Manual)
        && (fixed_system_storage
            || matches!(
                candidate.recovery,
                Some(RecoveryMode::Recycle | RecoveryMode::BackupThenDelete)
            ))
        && candidate.locked == Some(false)
        && candidate.path_allowed == Some(true)
        && candidate.file_identity_unchanged == Some(true)
        && (fixed_system_storage
            || candidate.rule_id.as_deref() == Some("recycle_bin")
            || is_explicit_cleanup_allowed(&candidate.path))
}

fn is_fixed_system_storage_cleanup(candidate: &CleanupCandidate) -> bool {
    matches!(
        candidate.rule_id.as_deref(),
        Some("system.hibernate" | "system.virtual_memory")
    )
}

fn operation_record(
    candidate: &CleanupCandidate,
    result: &CleanupItemResult,
    size: u64,
    backup_id: Option<String>,
) -> OperationRecord {
    OperationRecord {
        operation_id: format!("op-{}-{}", now(), operation_nonce()),
        rule_id: candidate.rule_id.clone(),
        path: candidate.path.display().into(),
        file_identity: candidate
            .file_identity
            .map(identity_label)
            .unwrap_or_else(|| "unknown".into()),
        backup_id,
        size,
        timestamp: now(),
        result: format!("{result:?}"),
    }
}

fn failed_result(plan: &CleanupPlan, reason: &str) -> CleanupResult {
    CleanupResult {
        items: plan
            .items
            .iter()
            .map(|item| CleanupResultItem {
                path: item.candidate.path.display().into(),
                result: CleanupItemResult::Failed(reason.into()),
                recoverable: false,
                released_bytes: 0,
            })
            .collect(),
        failed: plan.items.len() as u64,
        ..CleanupResult::default()
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

fn operation_nonce() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default()
}

pub fn identity_label(identity: FileIdentity) -> String {
    format!("{}:{:032x}", identity.volume_serial, identity.file_index)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AppError;
    use domain::{CleanupMode, Confidence, RecoveryMode, RiskLevel, normalize_path};

    #[derive(Default)]
    struct Log(Vec<OperationRecord>);
    impl OperationLog for Log {
        fn append(&mut self, record: OperationRecord) -> Result<(), AppError> {
            self.0.push(record);
            Ok(())
        }
    }
    struct Actions;
    impl CleanupActions for Actions {
        fn revalidate(
            &self,
            candidate: &CleanupCandidate,
        ) -> Result<CleanupCandidate, CleanupItemResult> {
            Ok(candidate.clone())
        }
        fn move_to_recycle_bin(&mut self, candidate: &CleanupCandidate) -> Result<u64, String> {
            Ok(candidate.estimated_size.unwrap_or(0))
        }
        fn backup_then_delete(&mut self, candidate: &CleanupCandidate) -> Result<u64, String> {
            Ok(candidate.estimated_size.unwrap_or(0))
        }
    }
    fn candidate() -> CleanupCandidate {
        CleanupCandidate {
            path: normalize_path(r"C:\Windows\Temp\cache.tmp").unwrap(),
            file_identity: None,
            rule_id: Some("temp".into()),
            category: Some("system_temp".into()),
            reason: None,
            estimated_size: Some(12),
            quick_clean_eligible: true,
            quick_clean_block_reason: None,
            mode: Some(CleanupMode::Quick),
            risk: Some(RiskLevel::P3),
            confidence: Some(Confidence::High),
            recovery: Some(RecoveryMode::Recycle),
            locked: Some(false),
            path_allowed: Some(true),
            file_identity_unchanged: Some(true),
            last_write_time: None,
        }
    }

    #[test]
    fn operation_records_have_unique_ids_within_the_same_second() {
        let candidate = candidate();
        let first = operation_record(&candidate, &CleanupItemResult::DeletedPermanently, 1, None);
        let second = operation_record(&candidate, &CleanupItemResult::DeletedPermanently, 1, None);

        assert_ne!(first.operation_id, second.operation_id);
    }

    struct PermanentActions;
    impl CleanupActions for PermanentActions {
        fn revalidate(
            &self,
            candidate: &CleanupCandidate,
        ) -> Result<CleanupCandidate, CleanupItemResult> {
            if candidate.path.display().contains("changed") {
                return Err(CleanupItemResult::SkippedChanged);
            }
            Ok(candidate.clone())
        }
        fn move_to_recycle_bin(&mut self, _: &CleanupCandidate) -> Result<u64, String> {
            panic!("permanent cleanup must never recycle files")
        }
        fn backup_then_delete(&mut self, _: &CleanupCandidate) -> Result<u64, String> {
            panic!("permanent cleanup must never create backups")
        }
        fn delete_permanently(&mut self, candidate: &CleanupCandidate) -> Result<u64, String> {
            if candidate.path.display().contains("failure") {
                return Err("delete failed".into());
            }
            Ok(candidate.estimated_size.unwrap_or(0))
        }
    }

    #[test]
    fn permanent_cleanup_deletes_quick_manual_and_recycle_bin_without_recovery() {
        let quick = candidate();
        let mut manual = candidate();
        manual.mode = Some(CleanupMode::Manual);
        manual.recovery = Some(RecoveryMode::BackupThenDelete);
        let mut recycle = manual.clone();
        recycle.path = normalize_path(r"C:\").unwrap();
        recycle.rule_id = Some("recycle_bin".into());
        let plan = preview_cleanup_with_explicit(&[quick, manual, recycle]);
        let service = CleanupService::new(PermanentActions, Log::default());
        let mut progress = Vec::new();
        let result = service.execute_permanently_with_cancellation(
            &plan,
            &crate::CancellationToken::default(),
            |event| progress.push(event),
        );
        assert_eq!(result.items.len(), 3);
        assert_eq!(result.released_bytes, plan.estimated_bytes);
        assert_eq!(result.recoverable, 0);
        assert!(result.items.iter().all(|item| !item.recoverable));
        assert_eq!(
            result.items[0].result,
            CleanupItemResult::DeletedPermanently
        );
        assert_eq!(
            result.items[1].result,
            CleanupItemResult::DeletedPermanently
        );
        assert_eq!(result.items[2].result, CleanupItemResult::RecycleBinEmptied);
        assert_eq!(progress.last().unwrap().processed, 3);
    }

    #[test]
    fn permanent_cleanup_keeps_validation_errors_failures_and_cancellation() {
        let mut changed = candidate();
        changed.path = normalize_path(r"C:\Windows\Temp\changed.tmp").unwrap();
        let mut failure = candidate();
        failure.path = normalize_path(r"C:\Windows\Temp\failure.tmp").unwrap();
        let mut forbidden = candidate();
        forbidden.path = normalize_path(r"C:\Windows\System32\protected.dll").unwrap();
        forbidden.mode = Some(CleanupMode::Manual);
        let service = CleanupService::new(PermanentActions, Log::default());
        let plan =
            preview_cleanup_with_explicit(&[changed, failure, forbidden, candidate(), candidate()]);
        let cancel = crate::CancellationToken::default();
        let result = service.execute_permanently_with_cancellation(&plan, &cancel, |event| {
            if event.processed == 4 {
                cancel.cancel();
            }
        });
        assert_eq!(result.items.len(), 4);
        assert_eq!(result.skipped, 2);
        assert_eq!(result.failed, 1);
        assert_eq!(result.released_bytes, 12);
        assert_eq!(result.items[2].result, CleanupItemResult::SkippedForbidden);
    }

    #[test]
    fn disabling_paging_files_requires_restart_and_releases_no_bytes_yet() {
        let mut paging = candidate();
        paging.path = normalize_path(r"C:\pagefile.sys").unwrap();
        paging.rule_id = Some("system.virtual_memory".into());
        paging.category = Some("system.virtual_memory".into());
        paging.mode = Some(CleanupMode::Manual);
        paging.risk = Some(RiskLevel::P2);
        paging.quick_clean_eligible = false;
        paging.quick_clean_block_reason = Some("需要用户明确确认并重启".into());
        let plan = preview_cleanup_with_explicit(&[paging]);
        let service = CleanupService::new(PermanentActions, Log::default());

        let result = service.execute_permanently_with_cancellation(
            &plan,
            &crate::CancellationToken::default(),
            |_| {},
        );

        assert_eq!(result.items.len(), 1);
        assert_eq!(result.items[0].result, CleanupItemResult::RestartRequired);
        assert_eq!(result.items[0].released_bytes, 0);
        assert_eq!(result.released_bytes, 0);
        assert_eq!(result.skipped, 0);
        assert_eq!(result.failed, 0);
    }
    #[test]
    fn emptying_recycle_bin_is_not_recoverable() {
        let mut recycle = candidate();
        recycle.path = normalize_path(r"C:\").unwrap();
        recycle.rule_id = Some("recycle_bin".into());
        recycle.category = Some("recycle_bin".into());
        recycle.mode = Some(CleanupMode::Manual);
        let service = CleanupService::new(Actions, Log::default());
        let result = service.execute(&preview_cleanup_with_explicit(&[recycle]));
        assert_eq!(result.recoverable, 0);
        assert!(!result.items[0].recoverable);
    }

    #[test]
    fn preview_filters_and_execute_revalidates() {
        let service = CleanupService::new(Actions, Log::default());
        let plan = service.preview(&[candidate()]);
        assert_eq!(plan.estimated_bytes, 12);
        let result = service.execute(&plan);
        assert_eq!(result.released_bytes, 12);
        assert_eq!(result.recoverable, 1);
    }

    #[test]
    fn execute_reports_cleanup_progress() {
        let service = CleanupService::new(Actions, Log::default());
        let plan = service.preview(&[candidate()]);
        let mut progress = Vec::new();

        let result = service.execute_with_progress(&plan, |event| progress.push(event));

        assert_eq!(result.released_bytes, 12);
        assert_eq!(progress[0].processed, 0);
        assert_eq!(progress[0].total, 1);
        assert_eq!(
            progress[1].current_path.as_deref(),
            Some(r"C:\Windows\Temp\cache.tmp")
        );
        assert_eq!(progress.last().unwrap().processed, 1);
        assert_eq!(progress.last().unwrap().released_bytes, 12);
    }

    #[test]
    fn cancellation_finishes_current_item_and_preserves_remaining_items() {
        let service = CleanupService::new(Actions, Log::default());
        let plan = service.preview(&[candidate(), candidate(), candidate()]);
        let cancel = crate::CancellationToken::default();
        let result = service.execute_with_cancellation(&plan, &cancel, |event| {
            if event.processed == 1 {
                cancel.cancel();
            }
        });
        assert_eq!(result.items.len(), 1);
        assert_eq!(result.released_bytes, 12);
        assert_eq!(result.failed, 0);
        assert_eq!(result.skipped, 0);
    }

    #[test]
    fn explicit_preview_allows_only_manual_recoverable_candidates() {
        let mut manual = candidate();
        manual.mode = Some(CleanupMode::Manual);
        manual.quick_clean_eligible = false;
        manual.quick_clean_block_reason = Some("需要用户明确确认".into());

        let plan = preview_cleanup_with_explicit(&[manual]);

        assert_eq!(plan.items.len(), 1);
        assert!(plan.items[0].explicit_confirmation);
    }

    #[test]
    fn explicit_confirmation_executes_manual_recycle_candidate() {
        let mut manual = candidate();
        manual.mode = Some(CleanupMode::Manual);
        manual.risk = Some(RiskLevel::P2);
        manual.quick_clean_eligible = false;
        manual.quick_clean_block_reason = Some("需要用户明确确认".into());
        let service = CleanupService::new(Actions, Log::default());
        let plan = preview_cleanup_with_explicit(&[manual]);

        let result = service.execute(&plan);

        assert!(matches!(
            result.items[0].result,
            CleanupItemResult::MovedToRecycleBin
        ));
        assert_eq!(result.released_bytes, 12);
    }

    #[derive(Default)]
    struct BackupFailure {
        backup_id: Option<String>,
    }

    impl CleanupActions for BackupFailure {
        fn revalidate(
            &self,
            candidate: &CleanupCandidate,
        ) -> Result<CleanupCandidate, CleanupItemResult> {
            Ok(candidate.clone().with_execution_facts(false, true, true))
        }

        fn move_to_recycle_bin(&mut self, _: &CleanupCandidate) -> Result<u64, String> {
            unreachable!("the backup failure fixture must use backup_then_delete")
        }

        fn backup_then_delete(&mut self, _: &CleanupCandidate) -> Result<u64, String> {
            self.backup_id = Some("backup-1".into());
            Err("source deletion failed after backup".into())
        }

        fn take_backup_id(&mut self) -> Option<String> {
            self.backup_id.take()
        }
    }

    #[test]
    fn failed_source_delete_keeps_successful_backup_recoverable() {
        let mut item = candidate();
        item.recovery = Some(RecoveryMode::BackupThenDelete);
        let service = CleanupService::new(BackupFailure::default(), Log::default());
        let result = service.execute(&service.preview(&[item]));

        assert_eq!(result.failed, 1);
        assert_eq!(result.recoverable, 1);
        assert!(result.items[0].recoverable);
    }
}
