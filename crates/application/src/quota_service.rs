#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BackupQuota {
    pub max_bytes: u64,
    pub used_bytes: u64,
    pub recoverable_items: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QuotaDecision {
    Allowed,
    Full,
    WouldBreakRecoveryChain,
}

impl BackupQuota {
    pub fn can_add(&self, bytes: u64) -> QuotaDecision {
        if self.used_bytes.saturating_add(bytes) > self.max_bytes {
            QuotaDecision::Full
        } else {
            QuotaDecision::Allowed
        }
    }
    pub fn can_remove(&self, recoverable: bool, remaining_items: u64) -> QuotaDecision {
        if recoverable && remaining_items == 0 {
            QuotaDecision::WouldBreakRecoveryChain
        } else {
            QuotaDecision::Allowed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quota_never_drops_last_recoverable_backup() {
        let quota = BackupQuota {
            max_bytes: 10,
            used_bytes: 10,
            recoverable_items: 1,
        };
        assert_eq!(quota.can_add(1), QuotaDecision::Full);
        assert_eq!(
            quota.can_remove(true, 0),
            QuotaDecision::WouldBreakRecoveryChain
        );
    }
}
