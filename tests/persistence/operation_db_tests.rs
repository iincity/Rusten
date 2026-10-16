use application::{MaintenanceAuditLog, MaintenanceAuditRecord};
use persistence::OperationDb;

#[test]
fn maintenance_audit_records_round_trip_with_exit_code_and_log_path() {
    let root = tempfile::tempdir().unwrap();
    let mut database = OperationDb::open(root.path().join("operations.sqlite")).unwrap();
    database
        .append_maintenance(MaintenanceAuditRecord {
            operation_id: "maintenance-1".into(),
            action: "StartComponentCleanup".into(),
            command: "dism.exe /Online /Cleanup-Image /StartComponentCleanup".into(),
            log_path: Some(r"C:\ProgramData\Rusten\dism.log".into()),
            preflight: "ready".into(),
            exit_code: Some(0),
            result: "completed".into(),
            timestamp: 123,
        })
        .unwrap();
    let records = database.maintenance_audits().unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].exit_code, Some(0));
    assert_eq!(
        records[0].log_path.as_deref(),
        Some(r"C:\ProgramData\Rusten\dism.log")
    );
}
