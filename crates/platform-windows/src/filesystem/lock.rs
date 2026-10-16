use std::{ffi::OsStr, os::windows::ffi::OsStrExt};

use domain::NormalizedPath;
use windows_sys::Win32::{
    Foundation::{
        CloseHandle, ERROR_ACCESS_DENIED, ERROR_SHARING_VIOLATION, GetLastError,
        INVALID_HANDLE_VALUE,
    },
    Storage::FileSystem::{
        CreateFileW, DELETE, FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES, FILE_SHARE_READ,
        OPEN_EXISTING,
    },
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LockStatus {
    Available,
    Locked,
    AccessDenied,
    Unknown(u32),
}

pub fn lock_status(path: &NormalizedPath) -> LockStatus {
    let wide = wide_null(path.extended());
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            DELETE | FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        )
    };
    if handle != INVALID_HANDLE_VALUE {
        unsafe {
            CloseHandle(handle);
        }
        return LockStatus::Available;
    }
    match unsafe { GetLastError() } {
        ERROR_SHARING_VIOLATION => LockStatus::Locked,
        ERROR_ACCESS_DENIED => LockStatus::AccessDenied,
        code => LockStatus::Unknown(code),
    }
}

fn wide_null(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain(Some(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::{LockStatus, lock_status};
    use domain::normalize_path;
    use std::{
        fs,
        os::windows::fs::OpenOptionsExt,
        time::{SystemTime, UNIX_EPOCH},
    };

    #[test]
    fn delete_probe_reports_a_file_without_delete_sharing_as_locked() {
        let path = std::env::temp_dir().join(format!(
            "rusten-delete-probe-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::write(&path, b"locked").unwrap();
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&path)
            .unwrap();

        assert_eq!(
            lock_status(&normalize_path(&path.to_string_lossy()).unwrap()),
            LockStatus::Locked
        );

        drop(held);
        fs::remove_file(path).unwrap();
    }
}
