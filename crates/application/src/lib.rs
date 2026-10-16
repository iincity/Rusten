pub mod analysis_service;
pub mod cleanup_service;
mod config_service;
mod controller;
mod ports;
pub mod quota_service;
pub mod recovery_service;
pub mod report_service;
pub mod rule_service;
mod scan_service;

pub use analysis_service::{AnalysisService, FileHashPort};
pub use cleanup_service::{
    ActionExecutor, CleanupActions, CleanupItemResult, CleanupPlan, CleanupPlanItem,
    CleanupProgress, CleanupResult, CleanupResultItem, CleanupService, preview_cleanup,
    preview_cleanup_with_explicit,
};
pub use config_service::{AppConfig, ConfigLoad, ConfigService};
pub use controller::ApplicationController;
pub use ports::{
    AppError, CancellationToken, MaintenanceAuditLog, MaintenanceAuditRecord, OperationLog,
    OperationRecord, RulePort, ScanPort, ScanRequest, ScanResult,
};
pub use quota_service::{BackupQuota, QuotaDecision};
pub use recovery_service::{
    RecoveryError, RecoveryId, RecoveryItem, RecoveryStore, RestoreDestination, choose_destination,
};
pub use report_service::{ReportData, ReportFormat, redact_path, render as render_report};
pub use rule_service::RuleService;
pub use scan_service::{ScanHandle, ScanService};
