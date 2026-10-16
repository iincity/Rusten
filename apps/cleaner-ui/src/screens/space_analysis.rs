use crate::components::{directory_tree::DirectoryRow, large_file_list::LargeFileRow};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SpaceAnalysisModel { pub directories: Vec<DirectoryRow>, pub large_files: Vec<LargeFileRow> }
