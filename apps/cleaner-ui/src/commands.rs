use crate::routes::ViewId;
use domain::FileIdentity;
pub use domain::{OperationId, PlanId};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UiCommand {
    Navigate(ViewId),
    StartScan,
    PauseScan,
    ResumeScan,
    CancelScan,
    CreateQuickCleanPreview,
    ExecuteCleanup(PlanId),
    ConfirmCleanup,
    CancelCleanup,
    StopCleanup,
    ToggleCleanupCategory(String),
    InspectCleanupCategory(String),
    ToggleCleanupGroup(String),
    ToggleCleanupGroupSelection(Vec<String>),
    ToggleDirectory(String),
    ToggleDirectorySelection(String),
    OpenAnalysisDirectory(String),
    OpenAnalysisParent,
    ToggleLargeFileSelection(FileIdentity),
    ToggleAutoClean,
    ToggleBackup,
    PurgeExpiredRecovery,
    RestoreOperation(OperationId),
}
