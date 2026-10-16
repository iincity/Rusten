use application::RecoveryItem;
use application::BackupQuota;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RecoveryModel { pub recoverable_operations: usize, pub items: Vec<RecoveryItem>, pub quota: Option<BackupQuota> }
