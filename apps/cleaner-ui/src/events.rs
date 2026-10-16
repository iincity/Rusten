#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UiEvent {
    ScanStarted,
    ScanPaused,
    ScanCompleted,
    ScanCanceled,
    ScanFailed(String),
}
