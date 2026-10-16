use application::{CleanupPlan, CleanupResult};
use crate::components::{cleanup_list::CleanupCategory, confirm_dialog::ConfirmDialog};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CleanupModel {
    pub categories: Vec<CleanupCategory>,
    pub confirmation: Option<ConfirmDialog>,
    pub plan: Option<CleanupPlan>,
    pub result: Option<CleanupResult>,
}
