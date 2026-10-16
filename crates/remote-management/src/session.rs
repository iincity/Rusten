#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionStatus {
    Active,
    Revoked,
    Expired,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteSession {
    pub session_id: String,
    pub expires_at: u64,
    pub status: SessionStatus,
}

impl RemoteSession {
    pub fn active(session_id: impl Into<String>, expires_at: u64) -> Self {
        Self {
            session_id: session_id.into(),
            expires_at,
            status: SessionStatus::Active,
        }
    }

    pub fn revoke(&mut self) {
        self.status = SessionStatus::Revoked;
    }

    pub fn status_at(&self, now: u64) -> SessionStatus {
        if self.status == SessionStatus::Active && now >= self.expires_at {
            SessionStatus::Expired
        } else {
            self.status
        }
    }

    pub fn revoke_with_audit(
        &mut self,
        now: u64,
        queue: &mut crate::audit_upload::OfflineQueue,
        device_id: &str,
        actor: &str,
        policy_version: &str,
    ) -> Result<(), String> {
        self.revoke();
        queue
            .append(crate::audit_upload::AuditUpload {
                event_id: format!("revoke-{}-{now}", self.session_id),
                timestamp: now,
                device_id: device_id.into(),
                actor: actor.into(),
                session_id: self.session_id.clone(),
                request_id: 0,
                command: "RevokeSession".into(),
                policy_version: policy_version.into(),
                approval: "revoked".into(),
                result_category: "revoked".into(),
                error_code: None,
            })
            .map_err(|error| error.to_string())
    }
}
