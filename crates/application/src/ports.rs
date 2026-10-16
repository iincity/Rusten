use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use domain::{FileRecord, NormalizedPath, Rule, ScanIndex, ScanStatus};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppError {
    Scanner(String),
    Rules(String),
    Log(String),
    Config(String),
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (kind, message) = match self {
            Self::Scanner(message) => ("scanner", message),
            Self::Rules(message) => ("rules", message),
            Self::Log(message) => ("log", message),
            Self::Config(message) => ("config", message),
        };
        write!(f, "{kind} error: {message}")
    }
}

impl std::error::Error for AppError {}

#[derive(Clone, Debug)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
}

impl CancellationToken {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
    pub fn pause(&self) {
        if !self.is_cancelled() {
            self.paused.store(true, Ordering::Release);
        }
    }
    pub fn resume(&self) {
        self.paused.store(false, Ordering::Release);
    }
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Acquire)
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            paused: Arc::new(AtomicBool::new(false)),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScanRequest {
    pub root: NormalizedPath,
    pub max_depth: usize,
    pub max_entries: u64,
}

#[derive(Debug)]
pub struct ScanResult {
    index: ScanIndex,
    status: ScanStatus,
}

impl ScanResult {
    pub fn from_parts(
        index: ScanIndex,
        status: ScanStatus,
        files: Vec<FileRecord>,
    ) -> Result<Self, AppError> {
        let status_matches = match status {
            ScanStatus::Complete => index.status() == ScanStatus::Complete,
            ScanStatus::Partial => index.status() == ScanStatus::Partial,
            ScanStatus::Canceled => index.status() == ScanStatus::Canceled,
            ScanStatus::InProgress => false,
        };
        if !status_matches {
            return Err(AppError::Scanner(
                "scan result status does not match its index".into(),
            ));
        }
        if files.iter().any(|file| !index.contains_file_record(file)) {
            return Err(AppError::Scanner(
                "scan files are not backed by the scan index".into(),
            ));
        }
        Ok(Self { index, status })
    }

    pub fn from_index(index: ScanIndex, status: ScanStatus) -> Result<Self, AppError> {
        Self::from_parts(index, status, Vec::new())
    }

    pub fn index(&self) -> &ScanIndex {
        &self.index
    }
    pub fn status(&self) -> ScanStatus {
        self.status
    }
    pub fn files(&self) -> Vec<FileRecord> {
        self.index.files()
    }

    pub fn into_parts(self) -> (ScanIndex, ScanStatus) {
        (self.index, self.status)
    }

    pub(crate) fn cancel_after_request(&mut self) {
        self.index.cancel_after_request();
        self.status = ScanStatus::Canceled;
    }
}

pub trait ScanPort {
    fn scan(
        &self,
        request: &ScanRequest,
        cancel: &CancellationToken,
    ) -> Result<ScanResult, AppError>;
}

pub trait RulePort {
    fn rules(&self) -> Result<Vec<Rule>, AppError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationRecord {
    pub operation_id: String,
    pub rule_id: Option<String>,
    pub path: String,
    pub file_identity: String,
    pub backup_id: Option<String>,
    pub size: u64,
    pub timestamp: u64,
    pub result: String,
}

pub trait OperationLog {
    fn append(&mut self, record: OperationRecord) -> Result<(), AppError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaintenanceAuditRecord {
    pub operation_id: String,
    pub action: String,
    pub command: String,
    pub log_path: Option<String>,
    pub preflight: String,
    pub exit_code: Option<i32>,
    pub result: String,
    pub timestamp: u64,
}

pub trait MaintenanceAuditLog {
    fn append_maintenance(&mut self, record: MaintenanceAuditRecord) -> Result<(), AppError>;
}
