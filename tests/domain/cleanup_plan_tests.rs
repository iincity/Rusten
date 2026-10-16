use application::{
    ActionExecutor, CleanupActions, CleanupItemResult, CleanupService, OperationLog,
    OperationRecord,
};
use domain::{
    CleanupCandidate, CleanupMode, Confidence, RecoveryMode, RiskLevel, can_enter_quick_clean,
    normalize_path,
};

#[derive(Default)]
struct Log;
impl OperationLog for Log {
    fn append(&mut self, _: OperationRecord) -> Result<(), application::AppError> {
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

#[test]
fn plan_excludes_non_quick_candidates() {
    let candidate = CleanupCandidate {
        path: normalize_path(r"C:\Windows\Temp\cache.tmp").unwrap(),
        file_identity: None,
        rule_id: None,
        category: Some("temp".into()),
        reason: None,
        estimated_size: Some(10),
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
    };
    let service = CleanupService::new(Actions, Log);
    assert_eq!(service.preview(&[candidate]).items.len(), 1);
}

#[test]
fn plan_excludes_candidates_with_rule_block_reasons() {
    let mut candidate = CleanupCandidate {
        path: normalize_path(r"C:\Windows\Temp\cache.tmp").unwrap(),
        file_identity: None,
        rule_id: None,
        category: Some("temp".into()),
        reason: Some("browser.exe is running".into()),
        estimated_size: Some(10),
        quick_clean_eligible: false,
        quick_clean_block_reason: Some("related process is running: browser.exe".into()),
        mode: Some(CleanupMode::Quick),
        risk: Some(RiskLevel::P3),
        confidence: Some(Confidence::High),
        recovery: Some(RecoveryMode::Recycle),
        locked: Some(false),
        path_allowed: Some(true),
        file_identity_unchanged: Some(true),
        last_write_time: None,
    };
    candidate.quick_clean_eligible = can_enter_quick_clean(&candidate);
    let service = CleanupService::new(Actions, Log);
    assert!(service.preview(&[candidate]).items.is_empty());
}
