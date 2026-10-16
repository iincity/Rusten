use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

#[cfg(not(windows))]
use std::sync::{Mutex, OnceLock};

#[cfg(not(windows))]
static WRITE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[cfg(windows)]
struct WriteGuard(isize);

#[cfg(not(windows))]
struct WriteGuard(std::sync::MutexGuard<'static, ()>);

pub struct ConfigStore;

impl ConfigStore {
    pub fn read(path: &Path) -> io::Result<String> {
        fs::read_to_string(path)
    }

    pub fn write(path: &Path, contents: &str) -> io::Result<()> {
        let _guard = acquire_write_lock()?;
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let temporary = temporary_path(path);
        let result = fs::write(&temporary, contents).and_then(|()| replace_file(&temporary, path));
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result
    }
}

#[cfg(not(windows))]
fn acquire_write_lock() -> io::Result<WriteGuard> {
    WRITE_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map(WriteGuard)
        .map_err(|_| io::Error::other("configuration write lock poisoned"))
}

#[cfg(windows)]
fn acquire_write_lock() -> io::Result<WriteGuard> {
    use std::os::windows::ffi::OsStrExt;

    const WAIT_OBJECT_0: u32 = 0;
    const WAIT_ABANDONED: u32 = 0x80;
    const WAIT_TIMEOUT: u32 = 0x102;
    unsafe extern "system" {
        fn CreateMutexW(attributes: *const (), initial_owner: i32, name: *const u16) -> isize;
        fn WaitForSingleObject(handle: isize, milliseconds: u32) -> u32;
    }

    let name: Vec<u16> = std::ffi::OsStr::new("Local\\Rusten.ConfigStore")
        .encode_wide()
        .chain(Some(0))
        .collect();
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
    if handle == 0 {
        return Err(io::Error::last_os_error());
    }
    let result = unsafe { WaitForSingleObject(handle, 5_000) };
    if matches!(result, WAIT_OBJECT_0 | WAIT_ABANDONED) {
        Ok(WriteGuard(handle))
    } else {
        unsafe { CloseHandle(handle) };
        Err(io::Error::new(
            if result == WAIT_TIMEOUT {
                io::ErrorKind::TimedOut
            } else {
                io::ErrorKind::Other
            },
            "configuration write lock unavailable",
        ))
    }
}

#[cfg(windows)]
impl Drop for WriteGuard {
    fn drop(&mut self) {
        unsafe {
            ReleaseMutex(self.0);
            CloseHandle(self.0);
        }
    }
}

#[cfg(windows)]
unsafe extern "system" {
    fn ReleaseMutex(handle: isize) -> i32;
    fn CloseHandle(handle: isize) -> i32;
}

fn temporary_path(path: &Path) -> PathBuf {
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("config");
    path.with_file_name(format!(".{name}.tmp-{}-{sequence}", std::process::id()))
}

#[cfg(windows)]
fn replace_file(temporary: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, replacement: *const u16, flags: u32) -> i32;
    }

    let existing: Vec<u16> = temporary.as_os_str().encode_wide().chain(Some(0)).collect();
    let replacement: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    if unsafe { MoveFileExW(existing.as_ptr(), replacement.as_ptr(), 0x1 | 0x8) } != 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(not(windows))]
fn replace_file(temporary: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(temporary, destination)
}
