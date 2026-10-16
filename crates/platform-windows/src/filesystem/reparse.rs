use std::{ffi::OsStr, os::windows::ffi::OsStrExt};

use domain::{NormalizedPath, ReparseKind};
use windows_sys::Win32::{
    Foundation::{ERROR_FILE_NOT_FOUND, GetLastError, INVALID_HANDLE_VALUE},
    Storage::FileSystem::{
        FILE_ATTRIBUTE_REPARSE_POINT, FindClose, FindExInfoBasic, FindExSearchNameMatch,
        FindFirstFileExW, WIN32_FIND_DATAW,
    },
    System::SystemServices::{
        IO_REPARSE_TAG_CLOUD, IO_REPARSE_TAG_CLOUD_F, IO_REPARSE_TAG_MOUNT_POINT,
        IO_REPARSE_TAG_SYMLINK,
    },
};

use super::PlatformError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReparsePointKind {
    SymbolicLink,
    MountPoint,
    Cloud,
    Other(u32),
}

pub fn reparse_point_kind(
    path: &NormalizedPath,
) -> Result<Option<ReparsePointKind>, PlatformError> {
    let wide = wide_null(path.extended());
    let mut data = WIN32_FIND_DATAW::default();
    let handle = unsafe {
        FindFirstFileExW(
            wide.as_ptr(),
            FindExInfoBasic,
            (&mut data as *mut WIN32_FIND_DATAW).cast(),
            FindExSearchNameMatch,
            std::ptr::null(),
            0,
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        let code = unsafe { GetLastError() };
        if code == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        return Err(PlatformError::from_last_os_error(Some(path.clone())));
    }
    unsafe {
        FindClose(handle);
    }
    Ok(classify(data.dwFileAttributes, data.dwReserved0))
}

pub(crate) fn classify(attributes: u32, tag: u32) -> Option<ReparsePointKind> {
    if attributes & FILE_ATTRIBUTE_REPARSE_POINT == 0 {
        return None;
    }
    Some(match tag {
        IO_REPARSE_TAG_SYMLINK => ReparsePointKind::SymbolicLink,
        IO_REPARSE_TAG_MOUNT_POINT => ReparsePointKind::MountPoint,
        IO_REPARSE_TAG_CLOUD..=IO_REPARSE_TAG_CLOUD_F
            if tag & 0xfff == IO_REPARSE_TAG_CLOUD & 0xfff =>
        {
            ReparsePointKind::Cloud
        }
        other => ReparsePointKind::Other(other),
    })
}

pub(crate) fn mount_target_kind(target: Option<&str>) -> ReparseKind {
    match target {
        Some(path) if path.starts_with(r"\\?\Volume{") || path.starts_with(r"\??\Volume{") => {
            ReparseKind::MountPoint
        }
        Some(_) => ReparseKind::Junction,
        None => ReparseKind::JunctionOrMountPoint,
    }
}

fn wide_null(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain(Some(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::{ReparsePointKind, classify};
    use windows_sys::Win32::{
        Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT,
        System::SystemServices::{
            IO_REPARSE_TAG_CLOUD, IO_REPARSE_TAG_CLOUD_F, IO_REPARSE_TAG_MOUNT_POINT,
        },
    };

    #[test]
    fn cloud_and_mount_tags_have_distinct_kinds() {
        assert_eq!(
            classify(FILE_ATTRIBUTE_REPARSE_POINT, IO_REPARSE_TAG_CLOUD),
            Some(ReparsePointKind::Cloud)
        );
        assert_eq!(
            classify(FILE_ATTRIBUTE_REPARSE_POINT, IO_REPARSE_TAG_CLOUD_F),
            Some(ReparsePointKind::Cloud)
        );
        assert_eq!(
            classify(FILE_ATTRIBUTE_REPARSE_POINT, IO_REPARSE_TAG_MOUNT_POINT),
            Some(ReparsePointKind::MountPoint)
        );
    }

    #[test]
    fn mount_tag_target_distinguishes_volume_mounts_from_junctions() {
        assert_eq!(
            super::mount_target_kind(Some(r"C:\data")),
            domain::ReparseKind::Junction
        );
        assert_eq!(
            super::mount_target_kind(Some(r"\\?\Volume{1234}\")),
            domain::ReparseKind::MountPoint
        );
        assert_eq!(
            super::mount_target_kind(None),
            domain::ReparseKind::JunctionOrMountPoint
        );
    }
}
