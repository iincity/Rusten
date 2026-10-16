use crate::audit_upload::{AuditUpload, OfflineQueue};
use crate::readonly_command::ReadOnlyCommand;
use crate::session::{RemoteSession, SessionStatus};
use domain::{CleanupCandidate, can_enter_quick_clean};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalApproval {
    pub request_id: u64,
    pub session_id: String,
    pub approved: bool,
    pub expires_at: u64,
    pub consumed: bool,
}

pub struct AuditContext<'a> {
    pub queue: &'a mut OfflineQueue,
    pub device_id: &'a str,
    pub actor: &'a str,
    pub policy_version: &'a str,
}

pub fn authorize_cleanup(
    approval: &mut LocalApproval,
    requested: ReadOnlyCommand,
) -> Result<(), String> {
    authorize_cleanup_at(
        approval,
        approval.request_id,
        &approval.session_id.clone(),
        requested,
        now(),
    )
}

pub fn authorize_cleanup_for(
    approval: &mut LocalApproval,
    request_id: u64,
    requested: ReadOnlyCommand,
) -> Result<(), String> {
    authorize_cleanup_at(
        approval,
        request_id,
        &approval.session_id.clone(),
        requested,
        now(),
    )
}

pub fn authorize_cleanup_at(
    approval: &mut LocalApproval,
    request_id: u64,
    session_id: &str,
    requested: ReadOnlyCommand,
    now: u64,
) -> Result<(), String> {
    if !matches!(requested, ReadOnlyCommand::RequestCleanup) {
        return Err("remote command is read-only".into());
    }
    if approval.session_id.trim().is_empty() || session_id.trim().is_empty() {
        return Err("session ID is required".into());
    }
    if approval.request_id == 0 || approval.request_id != request_id {
        return Err("approval does not match request".into());
    }
    if approval.session_id != session_id {
        return Err("approval does not match session".into());
    }
    if now >= approval.expires_at {
        return Err("approval expired".into());
    }
    if approval.consumed {
        return Err("approval already consumed".into());
    }
    if approval.approved {
        approval.consumed = true;
        Ok(())
    } else {
        Err("device-local approval required".into())
    }
}

pub fn authorize_cleanup_with_session(
    approval: &mut LocalApproval,
    request_id: u64,
    requested: ReadOnlyCommand,
    session: &RemoteSession,
    now: u64,
) -> Result<(), String> {
    if session.status_at(now) != SessionStatus::Active {
        return Err("remote session is not active".into());
    }
    authorize_cleanup_at(approval, request_id, &session.session_id, requested, now)
}

pub fn authorize_cleanup_with_audit(
    approval: &mut LocalApproval,
    request_id: u64,
    requested: ReadOnlyCommand,
    session: &RemoteSession,
    now: u64,
    audit: &mut AuditContext<'_>,
) -> Result<(), String> {
    let result = authorize_cleanup_with_session(approval, request_id, requested, session, now);
    let (category, error_code) = match &result {
        Ok(()) => ("approved", None),
        Err(error) => ("rejected", Some(error.clone())),
    };
    audit
        .queue
        .append(AuditUpload {
            event_id: format!("approval-{request_id}-{now}"),
            timestamp: now,
            device_id: audit.device_id.into(),
            actor: audit.actor.into(),
            session_id: session.session_id.clone(),
            request_id,
            command: "RequestCleanup".into(),
            policy_version: audit.policy_version.into(),
            approval: category.into(),
            result_category: category.into(),
            error_code,
        })
        .map_err(|error| error.to_string())?;
    result
}

pub fn authorize_cleanup_candidates(
    approval: &mut LocalApproval,
    request_id: u64,
    session: &RemoteSession,
    now: u64,
    candidates: &[CleanupCandidate],
) -> Result<Vec<CleanupCandidate>, String> {
    if candidates
        .iter()
        .any(|candidate| !can_enter_quick_clean(candidate))
    {
        return Err("local cleanup policy rejected one or more candidates".into());
    }
    authorize_cleanup_with_session(
        approval,
        request_id,
        ReadOnlyCommand::RequestCleanup,
        session,
        now,
    )?;
    Ok(candidates.to_vec())
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::readonly_command::ReadOnlyCommand;
    #[test]
    fn cleanup_requires_device_local_approval() {
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
    fn approval_is_single_use_and_expires() {
        let mut approval = LocalApproval {
            request_id: 1,
            session_id: "s".into(),
            approved: true,
            expires_at: 10,
            consumed: false,
        };
        assert!(
            authorize_cleanup_at(&mut approval, 1, "s", ReadOnlyCommand::RequestCleanup, 9).is_ok()
        );
        assert!(
            authorize_cleanup_at(&mut approval, 1, "s", ReadOnlyCommand::RequestCleanup, 9)
                .is_err()
        );
        approval.consumed = false;
        assert!(
            authorize_cleanup_at(&mut approval, 1, "s", ReadOnlyCommand::RequestCleanup, 10)
                .is_err()
        );
    }

    #[test]
    fn empty_session_ids_are_rejected() {
        let mut approval = LocalApproval {
            request_id: 1,
            session_id: String::new(),
            approved: true,
            expires_at: u64::MAX,
            consumed: false,
        };
        assert!(
            authorize_cleanup_at(&mut approval, 1, "", ReadOnlyCommand::RequestCleanup, 1).is_err()
        );
    }
}
