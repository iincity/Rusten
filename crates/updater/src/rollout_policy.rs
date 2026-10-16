pub fn allowed_for_rollout(bucket: u8, percentage: u8, maintenance_window: bool) -> bool {
    allowed_for_rollout_state(bucket, percentage, maintenance_window, ReleaseState::Active)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReleaseState {
    Paused,
    Active,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RolloutPolicy {
    pub target_version: String,
    pub rollout_percentage: u8,
    pub state: ReleaseState,
    pub maintenance_window: bool,
    pub verified_rollback_version: Option<String>,
}

impl RolloutPolicy {
    pub fn allows(&self, bucket: u8, current_version: &str) -> bool {
        self.target_version != current_version
            && allowed_for_rollout_state(
                bucket,
                self.rollout_percentage,
                self.maintenance_window,
                self.state,
            )
    }

    pub fn can_rollback_to(&self, version: &str) -> bool {
        self.verified_rollback_version.as_deref() == Some(version)
    }
}

pub fn allowed_for_rollout_state(
    bucket: u8,
    percentage: u8,
    maintenance_window: bool,
    state: ReleaseState,
) -> bool {
    state == ReleaseState::Active
        && maintenance_window
        && percentage > 0
        && percentage <= 100
        && bucket < percentage
}
