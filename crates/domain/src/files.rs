use crate::{DirectoryId, NormalizedPath};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
pub struct FileIdentity {
    pub volume_serial: u64,
    pub file_index: u128,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileRecord {
    path: NormalizedPath,
    parent: DirectoryId,
    identity: FileIdentity,
    logical_size: u64,
    allocated_size: u64,
    facts: FileFacts,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReparseKind {
    SymbolicLink,
    Junction,
    MountPoint,
    JunctionOrMountPoint,
    Cloud,
    Other(u32),
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum LockState {
    Available,
    Locked,
    AccessDenied,
    #[default]
    Unknown,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct FileFacts {
    pub creation_time: u64,
    pub last_access_time: u64,
    pub last_write_time: u64,
    pub attributes: u32,
    pub link_count: u32,
    pub lock_state: LockState,
    pub reparse_tag: Option<u32>,
    pub reparse_kind: Option<ReparseKind>,
    pub physical_path: Option<String>,
    pub target_path: Option<String>,
    pub target_volume_serial: Option<u64>,
    pub owner: Option<String>,
}

impl FileRecord {
    pub fn new(
        path: NormalizedPath,
        parent: DirectoryId,
        identity: FileIdentity,
        logical_size: u64,
        allocated_size: u64,
    ) -> Self {
        Self {
            path,
            parent,
            identity,
            logical_size,
            allocated_size,
            facts: FileFacts::default(),
        }
    }

    pub fn with_facts(mut self, facts: FileFacts) -> Self {
        self.facts = facts;
        self
    }

    pub fn facts(&self) -> &FileFacts {
        &self.facts
    }

    pub fn locked(&self) -> bool {
        self.facts.lock_state != LockState::Available
    }

    pub fn reparse_tag(&self) -> Option<u32> {
        self.facts.reparse_tag
    }

    pub fn path(&self) -> &NormalizedPath {
        &self.path
    }

    pub fn parent(&self) -> DirectoryId {
        self.parent
    }

    pub fn identity(&self) -> FileIdentity {
        self.identity
    }

    pub fn logical_size(&self) -> u64 {
        self.logical_size
    }

    pub fn allocated_size(&self) -> u64 {
        self.allocated_size
    }
}
