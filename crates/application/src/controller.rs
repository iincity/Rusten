use domain::{CleanupCandidate, RuleMatchContext, generate_candidates};

use crate::{
    AppError, OperationLog, OperationRecord, RulePort, ScanHandle, ScanPort, ScanRequest,
    ScanService,
};

pub struct ApplicationController<S, R, L> {
    scans: ScanService<S, R>,
    log: L,
}

impl<S, R, L> ApplicationController<S, R, L>
where
    S: ScanPort + Send + Sync + 'static,
    R: RulePort,
    L: OperationLog,
{
    pub fn new(scanner: S, rules: R, log: L) -> Self {
        Self {
            scans: ScanService::new(scanner, rules),
            log,
        }
    }

    pub fn start_scan(&self, request: ScanRequest) -> ScanHandle {
        self.scans.start(request)
    }

    pub fn preview(
        &self,
        rules: &[domain::Rule],
        result: &crate::ScanResult,
        context: &RuleMatchContext,
    ) -> Vec<CleanupCandidate> {
        let files = result.files();
        generate_candidates(rules, &files, context)
    }

    pub fn record(&mut self, record: OperationRecord) -> Result<(), AppError> {
        self.log.append(record)
    }
}
