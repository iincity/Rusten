use remote_management::{
    approval::{
        AuditContext, LocalApproval, authorize_cleanup, authorize_cleanup_at,
        authorize_cleanup_with_audit, authorize_cleanup_with_session,
    },
    audit_upload::OfflineQueue,
    readonly_command::ReadOnlyCommand,
    session::{RemoteSession, SessionStatus},
};

#[test]
fn remote_cleanup_requires_device_approval() {
    assert!(
        authorize_cleanup(
            &mut LocalApproval {
                request_id: 1,
                session_id: "s".into(),
                approved: false,
                expires_at: u64::MAX,
                consumed: false,
            },
            ReadOnlyCommand::RequestCleanup
        )
        .is_err()
    );
    assert!(
        authorize_cleanup(
            &mut LocalApproval {
                request_id: 1,
                session_id: "s".into(),
                approved: true,
                expires_at: u64::MAX,
                consumed: false,
            },
            ReadOnlyCommand::RequestCleanup
        )
        .is_ok()
    );
}

#[test]
fn approval_and_revocation_are_persisted_to_the_offline_audit_queue() {
    let root = tempfile::tempdir().unwrap();
    let mut queue = OfflineQueue::open(root.path().join("audit.jsonl")).unwrap();
    let session = RemoteSession::active("session-2", 100);
    let mut approval = LocalApproval {
        request_id: 3,
        session_id: "session-2".into(),
        approved: true,
        expires_at: 100,
        consumed: false,
    };
    authorize_cleanup_with_audit(
        &mut approval,
        3,
        ReadOnlyCommand::RequestCleanup,
        &session,
        4,
        &mut AuditContext {
            queue: &mut queue,
            device_id: "device",
            actor: "operator",
            policy_version: "7",
        },
    )
    .unwrap();
    let mut session = session;
    session
        .revoke_with_audit(5, &mut queue, "device", "operator", "7")
        .unwrap();
    assert_eq!(queue.len(), 2);
    assert!(
        queue
            .drain()
            .iter()
            .any(|entry| entry.command == "RevokeSession")
    );
}

#[test]
fn revoked_session_cannot_consume_approval() {
    let mut session = RemoteSession::active("session-1", 100);
    let mut approval = LocalApproval {
        request_id: 2,
        session_id: "session-1".into(),
        approved: true,
        expires_at: 99,
        consumed: false,
    };
    session.revoke();
    assert_eq!(session.status_at(1), SessionStatus::Revoked);
    assert!(
        authorize_cleanup_with_session(
            &mut approval,
            2,
            ReadOnlyCommand::RequestCleanup,
            &session,
            1,
        )
        .is_err()
    );
    assert!(
        authorize_cleanup_at(
            &mut approval,
            2,
            "other-session",
            ReadOnlyCommand::RequestCleanup,
            1,
        )
        .is_err()
    );
}
