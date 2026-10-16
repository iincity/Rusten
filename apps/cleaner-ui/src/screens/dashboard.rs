use crate::components::{cleanup_list::CleanupCategory, disk_overview::DiskOverview};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct DashboardModel { pub disk: Option<DiskOverview>, pub categories: Vec<CleanupCategory> }
