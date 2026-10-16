use std::{
    sync::{
        Arc,
        mpsc::{self, Receiver},
    },
    thread::JoinHandle,
};

use crate::{AppError, CancellationToken, RulePort, ScanPort, ScanRequest, ScanResult};

pub struct ScanService<S, R> {
    scanner: Arc<S>,
    rules: R,
}

pub struct ScanHandle {
    cancellation: CancellationToken,
    result: Receiver<Result<ScanResult, AppError>>,
    worker: Option<JoinHandle<()>>,
}

impl<S, R> ScanService<S, R>
where
    S: ScanPort + Send + Sync + 'static,
    R: RulePort,
{
    pub fn new(scanner: S, rules: R) -> Self {
        Self {
            scanner: Arc::new(scanner),
            rules,
        }
    }

    pub fn start(&self, request: ScanRequest) -> ScanHandle {
        self.start_with_cancellation(request, CancellationToken::default())
    }

    pub fn start_with_cancellation(
        &self,
        request: ScanRequest,
        cancellation: CancellationToken,
    ) -> ScanHandle {
        let (sender, result) = mpsc::channel();
        let scanner = Arc::clone(&self.scanner);
        let worker_cancellation = cancellation.clone();
        let worker = std::thread::spawn(move || {
            let _ = sender.send(scanner.scan(&request, &worker_cancellation));
        });
        ScanHandle {
            cancellation,
            result,
            worker: Some(worker),
        }
    }

    pub fn rules(&self) -> Result<Vec<domain::Rule>, AppError> {
        self.rules.rules()
    }
}

impl ScanHandle {
    pub fn cancel(&self) {
        self.cancellation.cancel();
    }
    pub fn pause(&self) {
        self.cancellation.pause();
    }
    pub fn resume(&self) {
        self.cancellation.resume();
    }
    pub fn is_paused(&self) -> bool {
        self.cancellation.is_paused()
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancellation.is_cancelled()
    }
    pub fn finish(mut self) -> Result<ScanResult, AppError> {
        let worker = self.worker.take().expect("scan worker must be present");
        let result = self.result.recv();
        let worker_result = worker.join();
        let result =
            result.map_err(|_| AppError::Scanner("scan worker stopped unexpectedly".into()))?;
        worker_result.map_err(|_| AppError::Scanner("scan worker panicked".into()))?;
        let mut result = result?;
        if self.is_cancelled() {
            result.cancel_after_request();
        }
        Ok(result)
    }
}

impl Drop for ScanHandle {
    fn drop(&mut self) {
        self.cancellation.cancel();
        let _ = self.worker.take();
    }
}
