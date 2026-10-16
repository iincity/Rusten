use std::collections::HashSet;

use domain::{
    AdminOperation, AdminRejectReason, AdminRequest, AdminResponse, AdminResult, RequestId,
    UserScope,
};

use crate::{
    authorization::validate_request,
    operations::{dism, recycle_bin, system_storage, system_temp},
};

#[derive(Default)]
pub struct RequestHandler {
    seen: HashSet<RequestId>,
}

impl RequestHandler {
    pub fn handle(&mut self, request: AdminRequest) -> AdminResponse {
        let request_id = request.request_id;
        let result = if let Err(reason) = validate_request(&request) {
            AdminResult::Rejected { reason }
        } else if !self.seen.insert(request_id) {
            AdminResult::Rejected {
                reason: AdminRejectReason::DuplicateRequest,
            }
        } else {
            match request.operation {
                AdminOperation::CleanSystemTemp { plan_id } => system_temp::clean(plan_id),
                AdminOperation::QueryRecycleBin {
                    user_scope: UserScope::CurrentUser,
                } => recycle_bin::query(),
                AdminOperation::QueryRecycleBin { .. } => AdminResult::Rejected {
                    reason: AdminRejectReason::UnsupportedScope,
                },
                AdminOperation::MoveToRecycleBin { path } => recycle_bin::move_to_recycle_bin(path),
                AdminOperation::DeleteFile {
                    path,
                    identity,
                    size,
                    last_write_time,
                    explicit,
                } => {
                    match platform_windows::filesystem::delete_verified_file(
                        &path,
                        identity,
                        size,
                        last_write_time,
                        explicit,
                    ) {
                        Ok(released_bytes) => AdminResult::PermanentlyDeleted { released_bytes },
                        Err(_) => AdminResult::Rejected {
                            reason: AdminRejectReason::OperationFailed,
                        },
                    }
                }
                AdminOperation::DisableHibernation => system_storage::disable_hibernation(),
                AdminOperation::DisablePagingFiles => system_storage::disable_paging_files(),
                AdminOperation::AnalyzeComponentStore => dism::analyze(),
                AdminOperation::CleanComponentStore { plan_id } => dism::clean(plan_id),
            }
        };
        AdminResponse {
            protocol_version: domain::PROTOCOL_VERSION,
            request_id,
            result,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::RequestHandler;
    use domain::{
        AdminOperation, AdminRejectReason, AdminRequest, AdminResult, PROTOCOL_VERSION, RequestId,
        UserScope,
    };

    #[test]
    fn delete_request_revalidates_and_permanently_deletes_a_disposable_file() {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let file = std::env::temp_dir().join(format!("rusten-admin-delete-{suffix}.tmp"));
        std::fs::write(&file, b"disposable admin cleanup test").unwrap();
        let path = domain::normalize_path(&file.to_string_lossy()).unwrap();
        let metadata = platform_windows::read_metadata(&path).unwrap();
        let request = AdminRequest {
            protocol_version: PROTOCOL_VERSION,
            request_id: RequestId(100),
            operation: AdminOperation::DeleteFile {
                path,
                identity: metadata.identity(),
                size: metadata.logical_size().max(metadata.allocated_size()),
                last_write_time: metadata.last_write_time(),
                explicit: true,
            },
        };
        let decoded = domain::decode_request(&domain::encode_request(&request).unwrap()).unwrap();
        let response = RequestHandler::default().handle(decoded);
        assert_eq!(
            response.result,
            AdminResult::PermanentlyDeleted {
                released_bytes: metadata.allocated_size()
            }
        );
        assert!(!file.exists());
    }

    #[test]
    fn rejects_unauthorized_scope_and_duplicate_request_ids() {
        let mut handler = RequestHandler::default();
        let unsupported = handler.handle(AdminRequest {
            protocol_version: PROTOCOL_VERSION,
            request_id: RequestId(1),
            operation: AdminOperation::QueryRecycleBin {
                user_scope: UserScope::AllUsers,
            },
        });
        assert_eq!(
            unsupported.result,
            AdminResult::Rejected {
                reason: AdminRejectReason::UnsupportedScope
            }
        );

        let request = AdminRequest {
            protocol_version: PROTOCOL_VERSION,
            request_id: RequestId(2),
            operation: AdminOperation::QueryRecycleBin {
                user_scope: UserScope::CurrentUser,
            },
        };
        let first = handler.handle(request.clone());
        let second = handler.handle(request);
        assert!(matches!(first.result, AdminResult::Completed { .. }));
        assert_eq!(
            second.result,
            AdminResult::Rejected {
                reason: AdminRejectReason::DuplicateRequest
            }
        );
    }
}
