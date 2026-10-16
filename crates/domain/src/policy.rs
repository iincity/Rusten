use crate::{Confidence, FileIdentity, NormalizedPath, RecoveryMode, RiskLevel, is_forbidden_path};

use crate::CleanupMode;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CleanupCandidate {
    pub path: NormalizedPath,
    pub file_identity: Option<FileIdentity>,
    pub rule_id: Option<String>,
    pub category: Option<String>,
    pub reason: Option<String>,
    pub estimated_size: Option<u64>,
    pub quick_clean_eligible: bool,
    pub quick_clean_block_reason: Option<String>,
    pub mode: Option<CleanupMode>,
    pub risk: Option<RiskLevel>,
    pub confidence: Option<Confidence>,
    pub recovery: Option<RecoveryMode>,
    pub locked: Option<bool>,
    pub path_allowed: Option<bool>,
    pub file_identity_unchanged: Option<bool>,
    pub last_write_time: Option<u64>,
}

impl CleanupCandidate {
    pub fn with_execution_facts(
        mut self,
        locked: bool,
        path_allowed: bool,
        file_identity_unchanged: bool,
    ) -> Self {
        self.locked = Some(locked);
        self.path_allowed = Some(path_allowed);
        self.file_identity_unchanged = Some(file_identity_unchanged);
        self.quick_clean_eligible =
            self.quick_clean_block_reason.is_none() && can_enter_quick_clean(&self);
        self
    }
}

pub fn can_enter_quick_clean(candidate: &CleanupCandidate) -> bool {
    candidate.quick_clean_block_reason.is_none()
        && candidate.mode == Some(CleanupMode::Quick)
        && candidate.risk == Some(RiskLevel::P3)
        && candidate.confidence == Some(Confidence::High)
        && matches!(
            candidate.recovery,
            Some(RecoveryMode::Recycle | RecoveryMode::BackupThenDelete)
        )
        && candidate.locked == Some(false)
        && candidate.path_allowed == Some(true)
        && candidate.file_identity_unchanged == Some(true)
        && !is_forbidden_path(&candidate.path)
}
