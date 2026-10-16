mod admin_protocol;
mod directories;
mod duplicate;
mod error;
mod files;
mod ids;
mod paths;
mod policy;
mod risk;
mod rules;
mod scan;
mod scan_index;

pub use admin_protocol::{
    AdminOperation, AdminRejectReason, AdminRequest, AdminResponse, AdminResult, MAX_FRAME_BYTES,
    OperationId, PROTOCOL_VERSION, PlanId, ProtocolError, RequestId, UserScope, decode_request,
    decode_response, encode_request, encode_response,
};
pub use directories::{DirectoryId, DirectoryNode};
pub use duplicate::{DuplicateGroup, find_duplicate_groups, find_duplicate_groups_with_progress};
pub use error::DomainError;
pub use files::{FileFacts, FileIdentity, FileRecord, LockState, ReparseKind};
pub use paths::{NormalizedPath, is_explicit_cleanup_allowed, is_forbidden_path, normalize_path};
pub use policy::{CleanupCandidate, can_enter_quick_clean};
pub use risk::{CleanupMode, Confidence, RecoveryMode, RiskLevel};
pub use rules::{
    Rule, RuleError, RuleMatchContext, RuleMatchPaths, RuleOperation, generate_candidates,
};
pub use scan::{
    ScanEngine, ScanEvent, ScanPhase, ScanStatistics, ScanStatus, ScanWarning, ScanWarningKind,
};
pub use scan_index::{FileFilter, ScanIndex};
