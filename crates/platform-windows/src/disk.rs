use std::{ffi::OsStr, os::windows::ffi::OsStrExt};

use domain::NormalizedPath;
use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;

use crate::filesystem::PlatformError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VolumeStats {
    available_bytes: u64,
    total_bytes: u64,
    total_free_bytes: u64,
}

impl VolumeStats {
    pub fn available_bytes(&self) -> u64 {
        self.available_bytes
    }

    pub fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    pub fn total_free_bytes(&self) -> u64 {
        self.total_free_bytes
    }
}

pub fn query_volume_stats(path: &NormalizedPath) -> Result<VolumeStats, PlatformError> {
    let wide = wide_null(path.extended());
    let mut stats = VolumeStats {
        available_bytes: 0,
        total_bytes: 0,
        total_free_bytes: 0,
    };
    let succeeded = unsafe {
        GetDiskFreeSpaceExW(
            wide.as_ptr(),
            &mut stats.available_bytes,
            &mut stats.total_bytes,
            &mut stats.total_free_bytes,
        )
    };
    if succeeded == 0 {
        return Err(PlatformError::from_last_os_error(Some(path.clone())));
    }
    Ok(stats)
}

fn wide_null(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain(Some(0)).collect()
}
