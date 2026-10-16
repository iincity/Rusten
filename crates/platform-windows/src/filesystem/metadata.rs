use std::{ffi::OsStr, mem::size_of, os::windows::ffi::OsStrExt};

use domain::{FileIdentity, NormalizedPath};
use windows_sys::Win32::{
    Foundation::{CloseHandle, FILETIME, HANDLE, INVALID_HANDLE_VALUE},
    Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_FLAG_BACKUP_SEMANTICS,
        FILE_FLAG_OPEN_REPARSE_POINT, FILE_READ_ATTRIBUTES, FILE_SHARE_DELETE, FILE_SHARE_READ,
        FILE_SHARE_WRITE, FILE_STANDARD_INFO, FileStandardInfo, GetFileInformationByHandle,
        GetFileInformationByHandleEx, GetFinalPathNameByHandleW, OPEN_EXISTING, VOLUME_NAME_DOS,
    },
};

use super::PlatformError;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileMetadata {
    identity: FileIdentity,
    logical_size: u64,
    allocated_size: u64,
    creation_time: u64,
    last_access_time: u64,
    last_write_time: u64,
    attributes: u32,
    link_count: u32,
    directory: bool,
    physical_path: String,
}

impl FileMetadata {
    pub fn identity(&self) -> FileIdentity {
        self.identity
    }

    pub fn logical_size(&self) -> u64 {
        self.logical_size
    }

    pub fn allocated_size(&self) -> u64 {
        self.allocated_size
    }

    pub fn creation_time(&self) -> u64 {
        self.creation_time
    }

    pub fn last_access_time(&self) -> u64 {
        self.last_access_time
    }

    pub fn last_write_time(&self) -> u64 {
        self.last_write_time
    }

    pub fn attributes(&self) -> u32 {
        self.attributes
    }

    pub fn link_count(&self) -> u32 {
        self.link_count
    }

    pub fn is_directory(&self) -> bool {
        self.directory
    }

    pub fn physical_path(&self) -> &str {
        &self.physical_path
    }
}

pub fn read_metadata(path: &NormalizedPath) -> Result<FileMetadata, PlatformError> {
    read_metadata_with_flags(path, 0, true)
}

pub(crate) fn read_reparse_metadata(path: &NormalizedPath) -> Result<FileMetadata, PlatformError> {
    read_metadata_with_flags(path, FILE_FLAG_OPEN_REPARSE_POINT, true)
}

pub(crate) fn read_statistics_metadata(
    path: &NormalizedPath,
) -> Result<FileMetadata, PlatformError> {
    read_metadata_with_flags(path, 0, false)
}

fn read_metadata_with_flags(
    path: &NormalizedPath,
    flags: u32,
    resolve_physical_path: bool,
) -> Result<FileMetadata, PlatformError> {
    let handle = NativeHandle::open(path, flags)?;
    read_metadata_from_handle(path, &handle, resolve_physical_path)
}

fn read_metadata_from_handle(
    path: &NormalizedPath,
    handle: &NativeHandle,
    resolve_physical_path: bool,
) -> Result<FileMetadata, PlatformError> {
    let mut basic = BY_HANDLE_FILE_INFORMATION::default();
    if unsafe { GetFileInformationByHandle(handle.0, &mut basic) } == 0 {
        return Err(PlatformError::from_last_os_error(Some(path.clone())));
    }
    let mut standard = FILE_STANDARD_INFO::default();
    if unsafe {
        GetFileInformationByHandleEx(
            handle.0,
            FileStandardInfo,
            (&mut standard as *mut FILE_STANDARD_INFO).cast(),
            size_of::<FILE_STANDARD_INFO>() as u32,
        )
    } == 0
    {
        return Err(PlatformError::from_last_os_error(Some(path.clone())));
    }

    Ok(FileMetadata {
        identity: FileIdentity {
            volume_serial: basic.dwVolumeSerialNumber as u64,
            file_index: ((basic.nFileIndexHigh as u128) << 32) | basic.nFileIndexLow as u128,
        },
        logical_size: ((basic.nFileSizeHigh as u64) << 32) | basic.nFileSizeLow as u64,
        allocated_size: standard.AllocationSize.max(0) as u64,
        creation_time: filetime(basic.ftCreationTime),
        last_access_time: filetime(basic.ftLastAccessTime),
        last_write_time: filetime(basic.ftLastWriteTime),
        attributes: basic.dwFileAttributes,
        link_count: basic.nNumberOfLinks,
        directory: standard.Directory,
        physical_path: if resolve_physical_path {
            final_path(handle.0, path)?
        } else {
            path.extended().to_owned()
        },
    })
}

pub fn delete_verified_file(
    path: &NormalizedPath,
    expected_identity: FileIdentity,
    expected_size: u64,
    expected_last_write: u64,
    explicit: bool,
) -> std::io::Result<u64> {
    use std::io::{Error, ErrorKind};
    use windows_sys::Win32::Storage::FileSystem::{
        DELETE, FILE_ATTRIBUTE_REPARSE_POINT, FILE_DISPOSITION_INFO, FileDispositionInfo,
        SetFileInformationByHandle,
    };
    let allowed = |path: &NormalizedPath| {
        !domain::is_forbidden_path(path) || (explicit && domain::is_explicit_cleanup_allowed(path))
    };
    if !allowed(path) {
        return Err(Error::new(ErrorKind::PermissionDenied, "protected path"));
    }
    let wide = wide_null(path.extended());
    // Use one handle and deny sharing for writes/deletion to prevent a change after validation.
    let raw = unsafe {
        CreateFileW(
            wide.as_ptr(),
            DELETE | FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ,
            std::ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS,
            std::ptr::null_mut(),
        )
    };
    if raw == INVALID_HANDLE_VALUE {
        return Err(Error::last_os_error());
    }
    let handle = NativeHandle(raw);
    let metadata = read_metadata_from_handle(path, &handle, true)
        .map_err(|error| Error::other(error.to_string()))?;
    let physical = domain::normalize_path(
        metadata
            .physical_path()
            .strip_prefix(r"\\?\")
            .unwrap_or(metadata.physical_path()),
    )
    .map_err(|error| Error::other(error.to_string()))?;
    if metadata.is_directory()
        || metadata.attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
        || !allowed(&physical)
    {
        return Err(Error::new(
            ErrorKind::PermissionDenied,
            "protected or redirected target",
        ));
    }
    if metadata.identity() != expected_identity
        || metadata.logical_size().max(metadata.allocated_size()) != expected_size
        || metadata.last_write_time() != expected_last_write
    {
        return Err(Error::other("file changed after scanning"));
    }
    let disposition = FILE_DISPOSITION_INFO { DeleteFile: true };
    if unsafe {
        SetFileInformationByHandle(
            handle.0,
            FileDispositionInfo,
            (&disposition as *const FILE_DISPOSITION_INFO).cast(),
            size_of::<FILE_DISPOSITION_INFO>() as u32,
        )
    } == 0
    {
        return Err(Error::last_os_error());
    }
    Ok(if metadata.link_count() > 1 {
        0
    } else {
        metadata.allocated_size()
    })
}

struct NativeHandle(HANDLE);

impl NativeHandle {
    fn open(path: &NormalizedPath, flags: u32) -> Result<Self, PlatformError> {
        let wide = wide_null(path.extended());
        let handle = unsafe {
            CreateFileW(
                wide.as_ptr(),
                FILE_READ_ATTRIBUTES,
                FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_FLAG_BACKUP_SEMANTICS | flags,
                std::ptr::null_mut(),
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            return Err(PlatformError::from_last_os_error(Some(path.clone())));
        }
        Ok(Self(handle))
    }
}

impl Drop for NativeHandle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

fn final_path(handle: HANDLE, path: &NormalizedPath) -> Result<String, PlatformError> {
    let mut buffer = vec![0_u16; 512];
    loop {
        let length = unsafe {
            GetFinalPathNameByHandleW(
                handle,
                buffer.as_mut_ptr(),
                buffer.len() as u32,
                VOLUME_NAME_DOS,
            )
        };
        if length == 0 {
            return Err(PlatformError::from_last_os_error(Some(path.clone())));
        }
        if length < buffer.len() as u32 {
            return Ok(String::from_utf16_lossy(&buffer[..length as usize]));
        }
        buffer.resize(length as usize + 1, 0);
    }
}

fn filetime(value: FILETIME) -> u64 {
    ((value.dwHighDateTime as u64) << 32) | value.dwLowDateTime as u64
}

fn wide_null(value: &str) -> Vec<u16> {
    OsStr::new(value).encode_wide().chain(Some(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delete(path: &NormalizedPath, metadata: &FileMetadata) -> std::io::Result<u64> {
        delete_verified_file(
            path,
            metadata.identity(),
            metadata.logical_size().max(metadata.allocated_size()),
            metadata.last_write_time(),
            true,
        )
    }

    #[test]
    fn permanent_delete_revalidates_and_removes_only_the_scanned_file() {
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("disposable.tmp");
        std::fs::write(&file, b"original").unwrap();
        let path = domain::normalize_path(&file.to_string_lossy()).unwrap();
        let metadata = read_metadata(&path).unwrap();
        assert!(
            delete_verified_file(
                &path,
                FileIdentity {
                    file_index: metadata.identity().file_index + 1,
                    ..metadata.identity()
                },
                metadata.logical_size().max(metadata.allocated_size()),
                metadata.last_write_time(),
                true
            )
            .is_err()
        );
        assert!(file.exists());
        std::fs::write(&file, b"changed contents").unwrap();
        assert!(delete(&path, &metadata).is_err());
        assert!(file.exists());
        let metadata = read_metadata(&path).unwrap();
        let writer = std::fs::OpenOptions::new().write(true).open(&file).unwrap();
        assert!(delete(&path, &metadata).is_err());
        assert!(file.exists());
        drop(writer);
        assert_eq!(delete(&path, &metadata).unwrap(), metadata.allocated_size());
        assert!(!file.exists());
        assert!(folder.path().exists());
    }

    #[test]
    fn permanent_delete_rejects_directories_and_reports_no_release_for_hardlinks() {
        let folder = tempfile::tempdir().unwrap();
        let dir = domain::normalize_path(&folder.path().to_string_lossy()).unwrap();
        let metadata = read_metadata(&dir).unwrap();
        assert!(delete(&dir, &metadata).is_err());
        let file = folder.path().join("original.tmp");
        let link = folder.path().join("link.tmp");
        std::fs::write(&file, b"linked contents").unwrap();
        std::fs::hard_link(&file, &link).unwrap();
        let path = domain::normalize_path(&file.to_string_lossy()).unwrap();
        let metadata = read_metadata(&path).unwrap();
        assert_eq!(delete(&path, &metadata).unwrap(), 0);
        assert!(!file.exists());
        assert_eq!(std::fs::read(&link).unwrap(), b"linked contents");
    }
}
