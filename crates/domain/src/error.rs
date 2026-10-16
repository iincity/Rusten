use std::{error::Error, fmt};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DomainError {
    EmptyPath,
    UnsupportedVolume,
    DevicePath,
    ReservedName,
    InvalidPath,
}

impl fmt::Display for DomainError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::EmptyPath => "path is empty",
            Self::UnsupportedVolume => "path is not on a supported local volume",
            Self::DevicePath => "device paths are not allowed",
            Self::ReservedName => "path contains a reserved Windows name",
            Self::InvalidPath => "path is not a valid absolute Windows path",
        })
    }
}

impl Error for DomainError {}
