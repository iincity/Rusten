use admin_helper::{
    authorization::validate_request, ipc_server::PIPE_NAME, request_handler::RequestHandler,
};
use domain::{
    AdminOperation, AdminRejectReason, AdminRequest, AdminResult, PROTOCOL_VERSION, PlanId,
    RequestId, UserScope, decode_request, encode_request,
};

#[test]
fn rejects_zero_request_ids_before_dispatch() {
    let request = AdminRequest {
        protocol_version: PROTOCOL_VERSION,
        request_id: RequestId(0),
        operation: AdminOperation::CleanSystemTemp { plan_id: PlanId(1) },
    };
    assert_eq!(
        validate_request(&request),
        Err(AdminRejectReason::InvalidRequest)
    );
}

#[test]
fn fixed_pipe_protocol_round_trips_and_rejects_duplicate_requests() {
    assert_eq!(PIPE_NAME, r"\\.\pipe\RustenAdminV1");
    let request = AdminRequest {
        protocol_version: PROTOCOL_VERSION,
        request_id: RequestId(7),
        operation: AdminOperation::QueryRecycleBin {
            user_scope: UserScope::CurrentUser,
        },
    };
    let frame = encode_request(&request).unwrap();
    assert_eq!(decode_request(&frame).unwrap(), request);

    let mut handler = RequestHandler::default();
    let first = handler.handle(request.clone());
    let second = handler.handle(request);
    assert!(!matches!(
        first.result,
        AdminResult::Rejected {
            reason: AdminRejectReason::DuplicateRequest
        }
    ));
    assert_eq!(
        second.result,
        AdminResult::Rejected {
            reason: AdminRejectReason::DuplicateRequest
        }
    );
}

#[test]
fn real_pipe_handles_multiple_permanent_deletes_with_one_helper() {
    use std::time::Duration;
    use windows_sys::Win32::System::Pipes::WaitNamedPipeW;
    let server = std::thread::spawn(|| {
        let mut handler = RequestHandler::default();
        for _ in 0..2 {
            assert!(admin_helper::ipc_server::serve_once(&mut handler).unwrap());
        }
    });
    let client = platform_windows::admin_client::AdminClient::new(Duration::from_secs(3));
    let pipe: Vec<u16> = PIPE_NAME.encode_utf16().chain(Some(0)).collect();
    for id in 1..=2 {
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while unsafe { WaitNamedPipeW(pipe.as_ptr(), 50) } == 0 {
            assert!(
                std::time::Instant::now() < deadline,
                "helper did not create its pipe"
            );
            std::thread::sleep(Duration::from_millis(10));
        }
        let path = std::env::temp_dir().join(format!(
            "rusten-ipc-disposable-{}-{id}.tmp",
            std::process::id()
        ));
        std::fs::write(&path, b"disposable pipe cleanup verification").unwrap();
        let normalized = domain::normalize_path(&path.to_string_lossy()).unwrap();
        let metadata = platform_windows::read_metadata(&normalized).unwrap();
        let response = client
            .request(&AdminRequest {
                protocol_version: PROTOCOL_VERSION,
                request_id: RequestId(1000 + id),
                operation: AdminOperation::DeleteFile {
                    path: normalized,
                    identity: metadata.identity(),
                    size: metadata.logical_size().max(metadata.allocated_size()),
                    last_write_time: metadata.last_write_time(),
                    explicit: true,
                },
            })
            .unwrap();
        assert_eq!(
            response.result,
            AdminResult::PermanentlyDeleted {
                released_bytes: metadata.allocated_size()
            }
        );
        assert!(!path.exists());
    }
    server.join().unwrap();
}
