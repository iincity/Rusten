use domain::{AdminRejectReason, AdminResult, NormalizedPath};

pub fn query() -> AdminResult {
    match platform_windows::shell::recycle_bin::query_current_user() {
        Ok(count) => AdminResult::Completed { affected: count },
        Err(_) => AdminResult::Rejected {
            reason: AdminRejectReason::OperationFailed,
        },
    }
}

pub fn move_to_recycle_bin(path: NormalizedPath) -> AdminResult {
    match platform_windows::shell::recycle_bin::move_to_recycle_bin(&path) {
        Ok(()) => AdminResult::Completed { affected: 1 },
        Err(_) => AdminResult::Rejected {
            reason: AdminRejectReason::OperationFailed,
        },
    }
}
