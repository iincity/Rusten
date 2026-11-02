use crate::routes::ViewId;
use domain::FileIdentity;
pub use domain::{OperationId, PlanId};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AnalysisCommand {
    SetDirectory(domain::DirectoryId),
    SetSelection(Option<crate::space_analysis::model::AnalysisSelection>),
    ToggleMark(domain::NormalizedPath),
    ReviewMarks,
    SetPage(usize),
    SetCleanupSourcePage(usize),
    SetFilter(String),
    SetMetric(crate::space_analysis::model::Metric),
    SetDepth(u8),
    SelectNext(bool),
    OpenSelection,
    ToggleSelectionMark,
    Zoom(bool),
    Fit,
    Pan(i32, i32),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UiCommand {
    RequestExit,
    ConfirmExit,
    CancelExit,
    Analysis(AnalysisCommand),
    Navigate(ViewId),
    StartScan,
    PauseScan,
    ResumeScan,
    CancelScan,
    RequestCancelScan,
    DismissCancelScan,
    CreateQuickCleanPreview,
    ExecuteCleanup(PlanId),
    ConfirmCleanup,
    CancelCleanup,
    SetConfirmationPage(usize),
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
    SetLargeFilePage(usize),
    SetLargeFileThreshold(u64),
    RevealFile(String),
    SetTheme(crate::theme::ThemeMode),
}
