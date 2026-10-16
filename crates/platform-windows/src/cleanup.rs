use domain::{CleanupCandidate, is_explicit_cleanup_allowed, is_forbidden_path};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VerificationFailure {
    Forbidden,
    Missing,
    IdentityChanged,
    SizeChanged,
    ModifiedChanged,
    Locked,
    Permission,
}

pub fn revalidate(candidate: &CleanupCandidate) -> Result<CleanupCandidate, VerificationFailure> {
    if candidate.rule_id.as_deref() == Some("recycle_bin") {
        return Ok(candidate.clone().with_execution_facts(false, true, true));
    }
    if is_forbidden_path(&candidate.path)
        && !(candidate.mode == Some(domain::CleanupMode::Manual)
            && is_explicit_cleanup_allowed(&candidate.path))
    {
        return Err(VerificationFailure::Forbidden);
    }
    let metadata = super::filesystem::read_metadata(&candidate.path).map_err(|error| {
        if error.code() == Some(windows_sys::Win32::Foundation::ERROR_ACCESS_DENIED) {
            VerificationFailure::Permission
        } else {
            VerificationFailure::Missing
        }
    })?;
    if candidate
        .file_identity
        .is_some_and(|identity| identity != metadata.identity())
    {
        return Err(VerificationFailure::IdentityChanged);
    }
    if candidate
        .estimated_size
        .is_some_and(|size| !size_matches(size, metadata.logical_size(), metadata.allocated_size()))
    {
        return Err(VerificationFailure::SizeChanged);
    }
    if candidate
        .last_write_time
        .is_some_and(|time| time != metadata.last_write_time())
    {
        return Err(VerificationFailure::ModifiedChanged);
    }
    match super::filesystem::lock_status(&candidate.path) {
        super::filesystem::LockStatus::Available => {}
        super::filesystem::LockStatus::AccessDenied => return Err(VerificationFailure::Permission),
        _ => return Err(VerificationFailure::Locked),
    }
    let candidate = candidate.clone().with_execution_facts(false, true, true);
    Ok(candidate)
}

pub fn move_to_recycle_bin(candidate: &CleanupCandidate) -> Result<u64, String> {
    super::shell::recycle_bin::move_to_recycle_bin(&candidate.path)?;
    Ok(candidate.estimated_size.unwrap_or(0))
}

pub fn empty_recycle_bin() -> Result<u64, String> {
    super::shell::recycle_bin::empty_current_user()
}

fn size_matches(estimated_size: u64, logical_size: u64, allocated_size: u64) -> bool {
    estimated_size == logical_size.max(allocated_size)
}

#[cfg(test)]
mod tests {
    use super::size_matches;

    #[test]
    fn accepts_logical_size_when_it_exceeds_allocation_size() {
        assert!(size_matches(4096, 4096, 1024));
    }

    #[test]
    fn rejects_a_changed_file_size() {
        assert!(!size_matches(4096, 2048, 1024));
    }
}
