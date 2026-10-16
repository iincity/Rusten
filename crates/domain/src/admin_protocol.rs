use serde::{Deserialize, Serialize};

use crate::NormalizedPath;

pub const PROTOCOL_VERSION: u16 = 1;
pub const MAX_FRAME_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct PlanId(pub u64);

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct RequestId(pub u64);

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct OperationId(pub u64);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum UserScope {
    CurrentUser,
    AllUsers,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum AdminOperation {
    CleanSystemTemp {
        plan_id: PlanId,
    },
    QueryRecycleBin {
        user_scope: UserScope,
    },
    MoveToRecycleBin {
        path: NormalizedPath,
    },
    DeleteFile {
        path: NormalizedPath,
        identity: crate::FileIdentity,
        size: u64,
        last_write_time: u64,
        explicit: bool,
    },
    DisableHibernation,
    DisablePagingFiles,
    AnalyzeComponentStore,
    CleanComponentStore {
        plan_id: PlanId,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AdminRequest {
    pub protocol_version: u16,
    pub request_id: RequestId,
    pub operation: AdminOperation,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum AdminRejectReason {
    InvalidRequest,
    UnauthorizedClient,
    ForbiddenPath,
    UnsupportedScope,
    DuplicateRequest,
    Timeout,
    OperationFailed,
    NotImplemented,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum AdminResult {
    Completed { affected: u64 },
    PermanentlyDeleted { released_bytes: u64 },
    Rejected { reason: AdminRejectReason },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AdminResponse {
    pub protocol_version: u16,
    pub request_id: RequestId,
    pub result: AdminResult,
}

#[derive(Debug, Eq, PartialEq)]
pub enum ProtocolError {
    FrameTooLarge,
    InvalidFrame,
    InvalidJson,
}

pub fn encode_request(request: &AdminRequest) -> Result<Vec<u8>, ProtocolError> {
    encode(request)
}

pub fn decode_request(frame: &[u8]) -> Result<AdminRequest, ProtocolError> {
    decode(frame)
}

pub fn encode_response(response: &AdminResponse) -> Result<Vec<u8>, ProtocolError> {
    encode(response)
}

pub fn decode_response(frame: &[u8]) -> Result<AdminResponse, ProtocolError> {
    decode(frame)
}

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, ProtocolError> {
    let payload = serde_json::to_vec(value).map_err(|_| ProtocolError::InvalidJson)?;
    if payload.len() > MAX_FRAME_BYTES {
        return Err(ProtocolError::FrameTooLarge);
    }
    let length = u32::try_from(payload.len()).map_err(|_| ProtocolError::FrameTooLarge)?;
    let mut frame = Vec::with_capacity(4 + payload.len());
    frame.extend_from_slice(&length.to_le_bytes());
    frame.extend_from_slice(&payload);
    Ok(frame)
}

fn decode<T: for<'de> Deserialize<'de>>(frame: &[u8]) -> Result<T, ProtocolError> {
    if frame.len() < 4 {
        return Err(ProtocolError::InvalidFrame);
    }
    let length = u32::from_le_bytes(frame[..4].try_into().unwrap()) as usize;
    if length > MAX_FRAME_BYTES || frame.len() != length + 4 {
        return Err(ProtocolError::InvalidFrame);
    }
    serde_json::from_slice(&frame[4..]).map_err(|_| ProtocolError::InvalidJson)
}

#[cfg(test)]
mod tests {
    use super::{
        AdminOperation, AdminRequest, PlanId, ProtocolError, RequestId, UserScope, decode_request,
        encode_request,
    };

    #[test]
    fn fixed_protocol_round_trips_and_rejects_trailing_bytes() {
        let request = AdminRequest {
            protocol_version: super::PROTOCOL_VERSION,
            request_id: RequestId(7),
            operation: AdminOperation::CleanSystemTemp { plan_id: PlanId(9) },
        };
        let frame = encode_request(&request).unwrap();
        assert_eq!(decode_request(&frame).unwrap(), request);
        assert_eq!(
            decode_request(&[0, 0, 0, 0, 1]),
            Err(ProtocolError::InvalidFrame)
        );
        let _ = UserScope::CurrentUser;
    }

    #[test]
    fn permanent_delete_request_preserves_the_scanned_identity_and_timestamp() {
        let request = AdminRequest {
            protocol_version: super::PROTOCOL_VERSION,
            request_id: RequestId(8),
            operation: AdminOperation::DeleteFile {
                path: crate::normalize_path(r"C:\Windows\Temp\disposable.tmp").unwrap(),
                identity: crate::FileIdentity {
                    volume_serial: 123,
                    file_index: u64::MAX as u128,
                },
                size: 4096,
                last_write_time: 123456789,
                explicit: false,
            },
        };
        assert_eq!(
            decode_request(&encode_request(&request).unwrap()).unwrap(),
            request
        );
    }

    #[test]
    fn fixed_system_storage_operations_round_trip_without_arguments() {
        for operation in [
            AdminOperation::DisableHibernation,
            AdminOperation::DisablePagingFiles,
        ] {
            let request = AdminRequest {
                protocol_version: super::PROTOCOL_VERSION,
                request_id: RequestId(9),
                operation,
            };
            assert_eq!(
                decode_request(&encode_request(&request).unwrap()).unwrap(),
                request
            );
        }
    }
}
