use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use application::{MaintenanceAuditLog, MaintenanceAuditRecord};
use domain::{AdminRejectReason, AdminResult, PlanId};
use persistence::{OperationDb, data_root};

const LOG_PATH: &str = r"C:\ProgramData\Rusten\dism.log";
static NEXT_AUDIT_ID: AtomicU64 = AtomicU64::new(1);

pub fn analyze() -> AdminResult {
    execute(
        platform_windows::dism::DismAction::AnalyzeComponentStore,
        false,
    )
}

pub fn clean(plan_id: PlanId) -> AdminResult {
    if plan_id.0 == 0 {
        return AdminResult::Rejected {
            reason: AdminRejectReason::InvalidRequest,
        };
    }
    execute(
        platform_windows::dism::DismAction::StartComponentCleanup,
        true,
    )
}

fn execute(action: platform_windows::dism::DismAction, confirmed: bool) -> AdminResult {
    let (preflight, query_error) =
        if action == platform_windows::dism::DismAction::StartComponentCleanup {
            match platform_windows::services::query_read_only_status() {
                Ok((update, _, restore)) => (
                    platform_windows::services::evaluate_preflight(&update, &restore, true),
                    None,
                ),
                Err(error) => (
                    platform_windows::services::MaintenancePreflight {
                        update_busy: false,
                        system_restore_available: false,
                        assistant_available: true,
                    },
                    Some(error),
                ),
            }
        } else {
            (
                platform_windows::services::MaintenancePreflight {
                    update_busy: false,
                    system_restore_available: true,
                    assistant_available: true,
                },
                None,
            )
        };
    if let Some(error) = query_error {
        let record = audit_record(
            action,
            &preflight,
            None,
            format!("preflight query failed: {error}"),
        );
        if append_audit(record).is_err() {
            return AdminResult::Rejected {
                reason: AdminRejectReason::OperationFailed,
            };
        }
        return AdminResult::Rejected {
            reason: AdminRejectReason::OperationFailed,
        };
    }
    let execution =
        platform_windows::dism::run_checked(action, PathBuf::from(LOG_PATH), confirmed, preflight);
    let (exit_code, result_label, outcome) = match &execution {
        Ok(result) if result.exit_code == Some(0) => (
            result.exit_code,
            "completed".to_owned(),
            AdminResult::Completed { affected: 0 },
        ),
        Ok(result) => (
            result.exit_code,
            "dism-exit-failure".to_owned(),
            AdminResult::Rejected {
                reason: AdminRejectReason::OperationFailed,
            },
        ),
        Err(error) => (
            None,
            format!("rejected: {error}"),
            AdminResult::Rejected {
                reason: AdminRejectReason::OperationFailed,
            },
        ),
    };
    let record = audit_record(action, &preflight, exit_code, result_label);
    if append_audit(record).is_err() {
        return AdminResult::Rejected {
            reason: AdminRejectReason::OperationFailed,
        };
    }
    outcome
}

fn audit_record(
    action: platform_windows::dism::DismAction,
    preflight: &platform_windows::services::MaintenancePreflight,
    exit_code: Option<i32>,
    result: impl Into<String>,
) -> MaintenanceAuditRecord {
    MaintenanceAuditRecord {
        operation_id: format!(
            "maintenance-{}",
            NEXT_AUDIT_ID.fetch_add(1, Ordering::Relaxed)
        ),
        action: format!("{action:?}"),
        command: platform_windows::dism::audit_command(action, LOG_PATH),
        log_path: Some(LOG_PATH.into()),
        preflight: format!("{preflight:?}"),
        exit_code,
        result: result.into(),
        timestamp: now(),
    }
}

fn append_audit(record: MaintenanceAuditRecord) -> Result<(), String> {
    let mut database = OperationDb::open(data_root().join("operations.sqlite"))
        .map_err(|error| error.to_string())?;
    database
        .append_maintenance(record)
        .map_err(|error| error.to_string())
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_record_identifies_fixed_dism_action() {
        let preflight = platform_windows::services::MaintenancePreflight {
            update_busy: false,
            system_restore_available: true,
            assistant_available: true,
        };
        let record = audit_record(
            platform_windows::dism::DismAction::AnalyzeComponentStore,
            &preflight,
            Some(0),
            "completed",
        );
        assert!(
            record
                .command
                .starts_with("dism.exe /Online /Cleanup-Image /AnalyzeComponentStore")
        );
        assert_eq!(record.exit_code, Some(0));
    }
}
