use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use application::{
    AppError, CancellationToken, OperationLog, OperationRecord, RulePort, ScanPort, ScanRequest,
    ScanResult, ScanService,
};
use domain::{FileIdentity, FileRecord, Rule, ScanIndex, ScanStatus, normalize_path};

struct FakeScanner {
    canceled: AtomicBool,
}

impl ScanPort for FakeScanner {
    fn scan(
        &self,
        request: &ScanRequest,
        cancel: &CancellationToken,
    ) -> Result<ScanResult, AppError> {
        let mut index = ScanIndex::new(request.root.clone());
        let root = index.root_directory_id();
        let file = FileRecord::new(
            normalize_path(r"C:\Temp\cache.tmp").unwrap(),
            root,
            FileIdentity {
                volume_serial: 1,
                file_index: 1,
            },
            10,
            16,
        );
        index.add_file(file.clone());
        if cancel.is_cancelled() {
            self.canceled.store(true, Ordering::Release);
            index.mark_canceled();
        } else {
            index.mark_complete();
        }
        ScanResult::from_parts(
            index,
            if self.canceled.load(Ordering::Acquire) {
                ScanStatus::Canceled
            } else {
                ScanStatus::Complete
            },
            vec![file],
        )
    }
}

struct FakeRules;
impl RulePort for FakeRules {
    fn rules(&self) -> Result<Vec<Rule>, AppError> {
        Ok(Vec::new())
    }
}

#[test]
fn scan_service_preserves_shared_index_and_terminal_status() {
    let service = ScanService::new(
        FakeScanner {
            canceled: AtomicBool::new(false),
        },
        FakeRules,
    );
    let handle = service.start(ScanRequest {
        root: normalize_path(r"C:\").unwrap(),
        max_depth: 4,
        max_entries: 100,
    });
    let result = handle.finish().unwrap();
    assert_eq!(result.status(), ScanStatus::Complete);
    assert_eq!(result.index().statistics().file_count(), 1);
    assert_eq!(result.files().len(), 1);
}

#[test]
fn scan_result_projects_files_from_its_index() {
    let mut index = ScanIndex::new(normalize_path(r"C:\").unwrap());
    let root = index.root_directory_id();
    index.add_file(FileRecord::new(
        normalize_path(r"C:\Temp\cache.tmp").unwrap(),
        root,
        FileIdentity {
            volume_serial: 1,
            file_index: 2,
        },
        10,
        16,
    ));
    index.mark_complete();

    let result = ScanResult::from_index(index, ScanStatus::Complete).unwrap();

    assert_eq!(result.files().len(), 1);
}

#[test]
fn operation_log_contract_accepts_structured_records() {
    struct FakeLog {
        records: Vec<OperationRecord>,
    }
    impl OperationLog for FakeLog {
        fn append(&mut self, record: OperationRecord) -> Result<(), AppError> {
            self.records.push(record);
            Ok(())
        }
    }
    let mut log = FakeLog {
        records: Vec::new(),
    };
    log.append(OperationRecord {
        operation_id: "op-1".into(),
        rule_id: Some("cache".into()),
        path: r"C:\Temp\cache.tmp".into(),
        file_identity: "1:1".into(),
        backup_id: None,
        size: 16,
        timestamp: 1,
        result: "preview".into(),
    })
    .unwrap();
    assert_eq!(log.records.len(), 1);
}

#[test]
fn invalid_configuration_falls_back_to_safe_defaults() {
    let loaded = application::ConfigService::load_toml("auto_clean = [broken");
    assert!(!loaded.config.auto_clean);
    assert!(loaded.config.backup_enabled);
    assert!(loaded.warning.is_some());
}

#[test]
fn valid_configuration_filters_invalid_paths_and_preserves_forbidden_cleanup_boundary() {
    let loaded = application::ConfigService::load_toml(
        "scan_root = 'C:/Windows'\nexcluded_paths = ['C:/Temp', 'not a path', 'C:/Users']\nauto_clean = true\n",
    );
    assert_eq!(loaded.config.scan_root, r"C:\Windows");
    assert_eq!(loaded.config.excluded_paths, vec![r"C:\Temp", r"C:\Users"]);
    assert!(loaded.warning.is_some());
}

struct CompletionAfterCancelScanner {
    started: Arc<AtomicBool>,
    release: Arc<AtomicBool>,
}

impl ScanPort for CompletionAfterCancelScanner {
    fn scan(
        &self,
        request: &ScanRequest,
        _cancel: &CancellationToken,
    ) -> Result<ScanResult, AppError> {
        self.started.store(true, Ordering::Release);
        while !self.release.load(Ordering::Acquire) {
            thread::sleep(Duration::from_millis(1));
        }
        let mut index = ScanIndex::new(request.root.clone());
        index.mark_complete();
        ScanResult::from_parts(index, ScanStatus::Complete, Vec::new())
    }
}

#[test]
fn cancellation_overrides_a_completed_scanner_result() {
    let started = Arc::new(AtomicBool::new(false));
    let release = Arc::new(AtomicBool::new(false));
    let service = ScanService::new(
        CompletionAfterCancelScanner {
            started: Arc::clone(&started),
            release: Arc::clone(&release),
        },
        FakeRules,
    );
    let handle = service.start(ScanRequest {
        root: normalize_path(r"C:\").unwrap(),
        max_depth: 4,
        max_entries: 100,
    });
    while !started.load(Ordering::Acquire) {
        thread::yield_now();
    }
    handle.cancel();
    release.store(true, Ordering::Release);
    let result = handle.finish().unwrap();
    assert_eq!(result.status(), ScanStatus::Canceled);
    assert_eq!(result.index().status(), ScanStatus::Canceled);
    assert!(!result.index().is_complete());
}

#[test]
fn scan_result_rejects_files_missing_from_index() {
    let index = ScanIndex::new(normalize_path(r"C:\").unwrap());
    let file = FileRecord::new(
        normalize_path(r"C:\Temp\unindexed.tmp").unwrap(),
        index.root_directory_id(),
        FileIdentity {
            volume_serial: 1,
            file_index: 404,
        },
        1,
        1,
    );
    assert!(ScanResult::from_parts(index, ScanStatus::Complete, vec![file]).is_err());
}

struct IgnoresCancellationScanner {
    release: Arc<AtomicBool>,
}

impl ScanPort for IgnoresCancellationScanner {
    fn scan(
        &self,
        _request: &ScanRequest,
        _cancel: &CancellationToken,
    ) -> Result<ScanResult, AppError> {
        while !self.release.load(Ordering::Acquire) {
            thread::sleep(Duration::from_millis(1));
        }
        let index = ScanIndex::new(normalize_path(r"C:\").unwrap());
        ScanResult::from_parts(index, ScanStatus::Complete, Vec::new())
    }
}

#[test]
fn dropping_scan_handle_does_not_wait_for_unresponsive_scanner() {
    let release = Arc::new(AtomicBool::new(false));
    let service = ScanService::new(
        IgnoresCancellationScanner {
            release: Arc::clone(&release),
        },
        FakeRules,
    );
    let (done_sender, done_receiver) = std::sync::mpsc::channel();
    let thread = thread::spawn(move || {
        let handle = service.start(ScanRequest {
            root: normalize_path(r"C:\").unwrap(),
            max_depth: 1,
            max_entries: 1,
        });
        drop(handle);
        done_sender.send(()).unwrap();
    });
    assert!(done_receiver.recv_timeout(Duration::from_secs(1)).is_ok());
    release.store(true, Ordering::Release);
    thread.join().unwrap();
}
