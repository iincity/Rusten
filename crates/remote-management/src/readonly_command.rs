use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum ReadOnlyCommand {
    StartScan,
    GetRedactedReport,
    GetExecutionResult,
    RequestCleanup,
}

pub fn is_read_only(command: ReadOnlyCommand) -> bool {
    !matches!(command, ReadOnlyCommand::RequestCleanup)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadOnlyRequest {
    pub request_id: u64,
    pub command: ReadOnlyCommand,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReadOnlyResponse {
    ScanStarted,
    RedactedReport(String),
    ExecutionResult(String),
    ApprovalRequired,
}

pub trait ReadOnlyCommandHandler {
    fn start_scan(&mut self) -> Result<(), String>;
    fn redacted_report(&self) -> Result<String, String>;
    fn execution_result(&self) -> Result<String, String>;
}

pub fn dispatch_read_only<H: ReadOnlyCommandHandler>(
    request: &ReadOnlyRequest,
    handler: &mut H,
) -> Result<ReadOnlyResponse, String> {
    if request.request_id == 0 {
        return Err("request ID is required".into());
    }
    match request.command {
        ReadOnlyCommand::StartScan => {
            handler.start_scan()?;
            Ok(ReadOnlyResponse::ScanStarted)
        }
        ReadOnlyCommand::GetRedactedReport => {
            Ok(ReadOnlyResponse::RedactedReport(handler.redacted_report()?))
        }
        ReadOnlyCommand::GetExecutionResult => Ok(ReadOnlyResponse::ExecutionResult(
            handler.execution_result()?,
        )),
        ReadOnlyCommand::RequestCleanup => Ok(ReadOnlyResponse::ApprovalRequired),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Handler;
    impl ReadOnlyCommandHandler for Handler {
        fn start_scan(&mut self) -> Result<(), String> {
            Ok(())
        }
        fn redacted_report(&self) -> Result<String, String> {
            Ok("report".into())
        }
        fn execution_result(&self) -> Result<String, String> {
            Ok("result".into())
        }
    }

    #[test]
    fn cleanup_never_executes_through_read_only_dispatch() {
        let mut handler = Handler;
        let request = ReadOnlyRequest {
            request_id: 1,
            command: ReadOnlyCommand::RequestCleanup,
        };
        assert_eq!(
            dispatch_read_only(&request, &mut handler),
            Ok(ReadOnlyResponse::ApprovalRequired)
        );
    }
}
