use std::path::Path;

#[cfg(windows)]
use std::path::PathBuf;

use domain::NormalizedPath;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RecycleBinStats {
    pub item_count: u64,
    pub bytes: u64,
}

pub fn query_current_user_stats() -> Result<RecycleBinStats, String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Shell::{SHQUERYRBINFO, SHQueryRecycleBinW};

        let mut info = SHQUERYRBINFO {
            cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32,
            i64Size: 0,
            i64NumItems: 0,
        };
        let result = unsafe { SHQueryRecycleBinW(std::ptr::null(), &mut info) };
        if result < 0 {
            return Err(format!("SHQueryRecycleBinW failed: {result}"));
        }
        let shell = stats_from_raw(info.i64Size, info.i64NumItems)?;
        Ok(merge_stats(shell, physical_recycle_bin_stats()?))
    }
    #[cfg(not(windows))]
    {
        Err("recycle bin requires Windows".into())
    }
}

pub fn query_current_user() -> Result<u64, String> {
    query_current_user_stats().map(|stats| stats.item_count)
}

pub fn empty_current_user() -> Result<u64, String> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::Shell::{
            SHERB_NOCONFIRMATION, SHERB_NOPROGRESSUI, SHERB_NOSOUND, SHEmptyRecycleBinW,
        };

        let before = query_current_user_stats()?;
        let result = unsafe {
            SHEmptyRecycleBinW(
                std::ptr::null_mut(),
                std::ptr::null(),
                SHERB_NOCONFIRMATION | SHERB_NOSOUND | SHERB_NOPROGRESSUI,
            )
        };
        if result < 0 {
            return Err(format!("SHEmptyRecycleBinW failed: {result}"));
        }
        for directory in current_user_recycle_bin_dirs()? {
            remove_recycle_entries(&directory)?;
        }
        cleared_bytes(before, query_current_user_stats()?)
    }
    #[cfg(not(windows))]
    {
        Err("recycle bin requires Windows".into())
    }
}

fn cleared_bytes(before: RecycleBinStats, after: RecycleBinStats) -> Result<u64, String> {
    if after.item_count != 0 || after.bytes != 0 {
        return Err(format!(
            "recycle bin still contains {} items ({} bytes)",
            after.item_count, after.bytes
        ));
    }
    Ok(before.bytes)
}

fn stats_from_raw(bytes: i64, item_count: i64) -> Result<RecycleBinStats, String> {
    Ok(RecycleBinStats {
        bytes: u64::try_from(bytes).map_err(|_| "recycle-bin size overflow".to_owned())?,
        item_count: u64::try_from(item_count)
            .map_err(|_| "recycle-bin count overflow".to_owned())?,
    })
}

fn merge_stats(shell: RecycleBinStats, physical: RecycleBinStats) -> RecycleBinStats {
    RecycleBinStats {
        bytes: shell.bytes.max(physical.bytes),
        item_count: shell.item_count.max(physical.item_count),
    }
}

#[cfg(windows)]
fn physical_recycle_bin_stats() -> Result<RecycleBinStats, String> {
    let mut stats = RecycleBinStats {
        item_count: 0,
        bytes: 0,
    };
    for directory in current_user_recycle_bin_dirs()? {
        let entries = match std::fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error.to_string()),
        };
        for entry in entries {
            let entry = entry.map_err(|error| error.to_string())?;
            if !entry.file_name().to_string_lossy().starts_with("$R") {
                continue;
            }
            stats.item_count = stats.item_count.saturating_add(1);
            stats.bytes = stats.bytes.saturating_add(entry_bytes(&entry.path()));
        }
    }
    Ok(stats)
}

fn remove_recycle_entries(directory: &Path) -> Result<(), String> {
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.to_string()),
    };
    for entry in entries {
        let entry = entry.map_err(|error| error.to_string())?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !(name.starts_with("$R") || name.starts_with("$I")) {
            continue;
        }
        let metadata =
            std::fs::symlink_metadata(entry.path()).map_err(|error| error.to_string())?;
        if metadata.is_dir() && !metadata.file_type().is_symlink() {
            std::fs::remove_dir_all(entry.path()).map_err(|error| error.to_string())?;
        } else {
            std::fs::remove_file(entry.path()).map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

#[cfg(windows)]
fn current_user_recycle_bin_dirs() -> Result<Vec<PathBuf>, String> {
    let sid = current_user_sid()?;
    Ok(fixed_drive_roots()?
        .into_iter()
        .map(|root| Path::new(&root).join("$Recycle.Bin").join(&sid))
        .collect())
}

#[cfg(windows)]
fn fixed_drive_roots() -> Result<Vec<String>, String> {
    use windows_sys::Win32::{
        Foundation::GetLastError,
        Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives},
        System::WindowsProgramming::DRIVE_FIXED,
    };

    let mask = unsafe { GetLogicalDrives() };
    if mask == 0 {
        return Err(format!("GetLogicalDrives failed: {}", unsafe {
            GetLastError()
        }));
    }
    Ok((0..26)
        .filter(|index| mask & (1 << index) != 0)
        .filter_map(|index| {
            let root = format!("{}:\\", (b'A' + index as u8) as char);
            let wide = root.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
            (unsafe { GetDriveTypeW(wide.as_ptr()) } == DRIVE_FIXED).then_some(root)
        })
        .collect())
}

#[cfg(windows)]
fn current_user_sid() -> Result<String, String> {
    use std::{ptr, slice};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, GetLastError, HANDLE, LocalFree},
        Security::Authorization::ConvertSidToStringSidW,
        Security::{GetTokenInformation, TOKEN_QUERY, TOKEN_USER, TokenUser},
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };

    struct Token(HANDLE);
    impl Drop for Token {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.0) };
        }
    }

    let mut token = std::ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(format!("OpenProcessToken failed: {}", unsafe {
            GetLastError()
        }));
    }
    let token = Token(token);
    let mut required = 0;
    unsafe { GetTokenInformation(token.0, TokenUser, ptr::null_mut(), 0, &mut required) };
    if required == 0 {
        return Err(format!("GetTokenInformation size failed: {}", unsafe {
            GetLastError()
        }));
    }
    let mut buffer = vec![0u8; required as usize];
    if unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            buffer.as_mut_ptr().cast(),
            required,
            &mut required,
        )
    } == 0
    {
        return Err(format!("GetTokenInformation failed: {}", unsafe {
            GetLastError()
        }));
    }
    let user = unsafe { &*(buffer.as_ptr().cast::<TOKEN_USER>()) };
    let mut pointer = ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(user.User.Sid, &mut pointer) } == 0 {
        return Err(format!("ConvertSidToStringSidW failed: {}", unsafe {
            GetLastError()
        }));
    }
    let mut length = 0;
    while unsafe { *pointer.add(length) } != 0 {
        length += 1;
    }
    let sid = String::from_utf16_lossy(unsafe { slice::from_raw_parts(pointer, length) });
    unsafe { LocalFree(pointer.cast()) };
    Ok(sid)
}

#[cfg(windows)]
fn entry_bytes(path: &Path) -> u64 {
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return 0;
    };
    if metadata.file_type().is_symlink() {
        return 0;
    }
    if metadata.is_file() {
        return metadata.len();
    }
    if !metadata.is_dir() {
        return 0;
    }
    std::fs::read_dir(path)
        .into_iter()
        .flat_map(|entries| entries.filter_map(Result::ok))
        .map(|entry| entry_bytes(&entry.path()))
        .sum()
}

pub fn move_to_recycle_bin(path: &NormalizedPath) -> Result<(), String> {
    super::file_operation::move_to_recycle_bin(path)
}

#[cfg(test)]
mod tests {
    use super::{
        RecycleBinStats, cleared_bytes, merge_stats, remove_recycle_entries, stats_from_raw,
    };

    #[test]
    fn converts_recycle_bin_size_and_count() {
        let stats = stats_from_raw(17_000, 3).unwrap();
        assert_eq!(stats.bytes, 17_000);
        assert_eq!(stats.item_count, 3);
    }

    #[test]
    fn clearing_succeeds_only_when_the_global_recycle_bin_is_empty() {
        let before = RecycleBinStats {
            item_count: 3,
            bytes: 17_000,
        };
        assert_eq!(
            cleared_bytes(
                before,
                RecycleBinStats {
                    item_count: 0,
                    bytes: 0,
                }
            )
            .unwrap(),
            17_000
        );
        assert!(
            cleared_bytes(
                before,
                RecycleBinStats {
                    item_count: 1,
                    bytes: 4_096,
                }
            )
            .is_err()
        );
    }

    #[test]
    fn physical_entries_keep_recycle_bin_visible_when_shell_reports_zero() {
        let shell = RecycleBinStats {
            item_count: 0,
            bytes: 0,
        };
        let physical = RecycleBinStats {
            item_count: 4,
            bytes: 12_345,
        };

        assert_eq!(merge_stats(shell, physical), physical);
    }

    #[test]
    fn fallback_removes_every_counted_recycle_entry() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("$Rfile.bin"), b"garbage").unwrap();
        std::fs::write(root.path().join("$Ifile.bin"), b"metadata").unwrap();
        std::fs::create_dir(root.path().join("$Rfolder")).unwrap();
        std::fs::write(root.path().join("$Rfolder").join("nested.bin"), b"garbage").unwrap();
        std::fs::write(root.path().join("$Other"), b"unrelated").unwrap();
        std::fs::write(root.path().join("keep.txt"), b"unrelated").unwrap();

        remove_recycle_entries(root.path()).unwrap();

        assert!(!root.path().join("$Rfile.bin").exists());
        assert!(!root.path().join("$Ifile.bin").exists());
        assert!(!root.path().join("$Rfolder").exists());
        assert!(root.path().join("$Other").exists());
        assert!(root.path().join("keep.txt").exists());
    }
}
