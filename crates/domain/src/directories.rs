use crate::NormalizedPath;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct DirectoryId(pub(crate) usize);

impl DirectoryId {
    pub fn index(self) -> usize {
        self.0
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DirectoryNode {
    path: NormalizedPath,
    parent: Option<DirectoryId>,
    children: Vec<DirectoryId>,
    logical_size: u64,
    allocated_size: u128,
    file_count: u64,
}

impl DirectoryNode {
    pub fn new(path: NormalizedPath, parent: DirectoryId) -> Self {
        Self {
            path,
            parent: Some(parent),
            children: Vec::new(),
            logical_size: 0,
            allocated_size: 0,
            file_count: 0,
        }
    }

    pub fn path(&self) -> &NormalizedPath {
        &self.path
    }

    pub fn parent(&self) -> Option<DirectoryId> {
        self.parent
    }

    pub fn children(&self) -> &[DirectoryId] {
        &self.children
    }

    pub fn logical_size(&self) -> u64 {
        self.logical_size
    }

    pub fn allocated_size(&self) -> u64 {
        self.allocated_size.min(u64::MAX as u128) as u64
    }

    pub fn file_count(&self) -> u64 {
        self.file_count
    }

    pub(crate) fn root(path: NormalizedPath) -> Self {
        Self {
            path,
            parent: None,
            children: Vec::new(),
            logical_size: 0,
            allocated_size: 0,
            file_count: 0,
        }
    }

    pub(crate) fn add_child(&mut self, child: DirectoryId) {
        self.children.push(child);
    }

    pub(crate) fn add_file(&mut self, logical_size: u64, allocated_size: u64) {
        self.logical_size = self.logical_size.saturating_add(logical_size);
        self.allocated_size += allocated_size as u128;
        self.file_count = self.file_count.saturating_add(1);
    }

    pub(crate) fn adjust_allocated_size(&mut self, old: u64, new: u64) {
        self.allocated_size = self.allocated_size - old as u128 + new as u128;
    }
}
