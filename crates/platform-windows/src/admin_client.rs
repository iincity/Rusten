use std::time::Duration;

use domain::{AdminRequest, AdminResponse, ProtocolError, decode_response, encode_request};

#[derive(Debug)]
pub enum AdminClientError {
    Protocol(ProtocolError),
    Transport(String),
}

pub struct AdminClient {
    pipe_name: String,
    timeout: Duration,
    failure: std::sync::Mutex<Option<String>>,
}

impl AdminClient {
    pub fn new(timeout: Duration) -> Self {
        Self {
            pipe_name: r"\\.\pipe\RustenAdminV1".into(),
            timeout,
            failure: std::sync::Mutex::new(None),
        }
    }

    pub fn request(&self, request: &AdminRequest) -> Result<AdminResponse, AdminClientError> {
        if let Some(reason) = self.failure.lock().unwrap().as_ref() {
            return Err(AdminClientError::Transport(reason.clone()));
        }
        let result = self.request_once(request);
        if let Err(error) = &result {
            *self.failure.lock().unwrap() =
                Some(format!("管理员助手不可用，本轮不再重复等待：{error:?}"));
        }
        result
    }

    fn request_once(&self, request: &AdminRequest) -> Result<AdminResponse, AdminClientError> {
        let input = encode_request(request).map_err(AdminClientError::Protocol)?;
        #[cfg(windows)]
        {
            use std::{
                thread,
                time::{Duration, Instant},
            };
            use windows_sys::Win32::{
                Foundation::{
                    ERROR_FILE_NOT_FOUND, ERROR_PIPE_BUSY, GENERIC_READ, GENERIC_WRITE,
                    INVALID_HANDLE_VALUE,
                },
                Storage::FileSystem::{CreateFileW, FILE_FLAG_OVERLAPPED, OPEN_EXISTING},
            };
            let name: Vec<u16> = self.pipe_name.encode_utf16().chain(Some(0)).collect();
            let deadline = Instant::now() + self.timeout;
            let connect_started = Instant::now();
            let mut started = false;
            let pipe = loop {
                let handle = unsafe {
                    CreateFileW(
                        name.as_ptr(),
                        GENERIC_READ | GENERIC_WRITE,
                        0,
                        std::ptr::null(),
                        OPEN_EXISTING,
                        FILE_FLAG_OVERLAPPED,
                        std::ptr::null_mut(),
                    )
                };
                if handle != INVALID_HANDLE_VALUE {
                    break PipeHandle(handle);
                }
                let error = std::io::Error::last_os_error();
                if !matches!(
                    error.raw_os_error().map(|code| code as u32),
                    Some(ERROR_FILE_NOT_FOUND | ERROR_PIPE_BUSY)
                ) {
                    return Err(AdminClientError::Transport(error.to_string()));
                }
                if !started && connect_started.elapsed() >= Duration::from_millis(250) {
                    let helper = std::env::current_exe()
                        .map_err(|error| AdminClientError::Transport(error.to_string()))?
                        .with_file_name("admin-helper.exe");
                    if !helper.is_file() {
                        return Err(AdminClientError::Transport(
                            "admin-helper.exe is not installed next to the client".into(),
                        ));
                    }
                    launch_elevated(&helper)?;
                    started = true;
                }
                if Instant::now() >= deadline {
                    return Err(AdminClientError::Transport(format!(
                        "等待管理员助手连接超时：{error}"
                    )));
                }
                thread::sleep(Duration::from_millis(25));
            };
            let output = transact(&pipe, &input, self.timeout)?;
            let response = decode_response(&output).map_err(AdminClientError::Protocol)?;
            if response.request_id != request.request_id
                || response.protocol_version != domain::PROTOCOL_VERSION
            {
                return Err(AdminClientError::Transport(
                    "admin helper response ID or version mismatch".into(),
                ));
            }
            Ok(response)
        }
        #[cfg(not(windows))]
        {
            let _ = (input, &self.pipe_name, self.timeout);
            Err(AdminClientError::Transport(
                "admin helper requires Windows".into(),
            ))
        }
    }
}

#[cfg(windows)]
struct PipeHandle(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl Drop for PipeHandle {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

#[cfg(windows)]
fn transact(
    pipe: &PipeHandle,
    input: &[u8],
    timeout: Duration,
) -> Result<Vec<u8>, AdminClientError> {
    use windows_sys::Win32::{
        Foundation::{ERROR_IO_PENDING, GetLastError},
        System::{
            IO::{CancelIoEx, GetOverlappedResult, GetOverlappedResultEx, OVERLAPPED},
            Pipes::{PIPE_READMODE_MESSAGE, SetNamedPipeHandleState, TransactNamedPipe},
        },
    };
    let mode = PIPE_READMODE_MESSAGE;
    if unsafe { SetNamedPipeHandleState(pipe.0, &mode, std::ptr::null(), std::ptr::null()) } == 0 {
        return Err(AdminClientError::Transport(
            std::io::Error::last_os_error().to_string(),
        ));
    }
    let mut output = vec![0u8; domain::MAX_FRAME_BYTES + 4];
    let mut read = 0;
    let mut overlapped = OVERLAPPED::default();
    let ok = unsafe {
        TransactNamedPipe(
            pipe.0,
            input.as_ptr().cast(),
            input.len() as u32,
            output.as_mut_ptr().cast(),
            output.len() as u32,
            &mut read,
            &mut overlapped,
        )
    };
    if ok == 0 {
        if unsafe { GetLastError() } != ERROR_IO_PENDING {
            return Err(AdminClientError::Transport(
                std::io::Error::last_os_error().to_string(),
            ));
        }
        if unsafe {
            GetOverlappedResultEx(
                pipe.0,
                &overlapped,
                &mut read,
                timeout.as_millis().clamp(1, u32::MAX as u128) as u32,
                0,
            )
        } == 0
        {
            let error = std::io::Error::last_os_error();
            // Keep the buffers alive until cancellation has completed; never resubmit a delete.
            unsafe {
                CancelIoEx(pipe.0, &overlapped);
                GetOverlappedResult(pipe.0, &overlapped, &mut read, 1);
            }
            return Err(AdminClientError::Transport(format!(
                "管理员助手响应失败或超时：{error}"
            )));
        }
    }
    output.truncate(read as usize);
    Ok(output)
}

#[cfg(windows)]
fn launch_elevated(path: &std::path::Path) -> Result<(), AdminClientError> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::ShellExecuteW;

    let verb: Vec<u16> = std::ffi::OsStr::new("runas")
        .encode_wide()
        .chain(Some(0))
        .collect();
    let file: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
        )
    };
    if result as usize <= 32 {
        return Err(AdminClientError::Transport(format!(
            "failed to launch elevated admin helper: {}",
            result as usize
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stalled_helper_times_out_once_and_later_files_fail_immediately() {
        use windows_sys::Win32::{
            Foundation::INVALID_HANDLE_VALUE,
            Storage::FileSystem::PIPE_ACCESS_DUPLEX,
            System::Pipes::{
                ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_MESSAGE, PIPE_TYPE_MESSAGE,
                PIPE_WAIT,
            },
        };
        let pipe_name = format!(r"\\.\pipe\RustenTimeoutTest-{}", std::process::id());
        let server_name = pipe_name.clone();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let (stop_tx, stop_rx) = std::sync::mpsc::channel();
        let server = std::thread::spawn(move || {
            let name: Vec<u16> = server_name.encode_utf16().chain(Some(0)).collect();
            let raw = unsafe {
                CreateNamedPipeW(
                    name.as_ptr(),
                    PIPE_ACCESS_DUPLEX,
                    PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT,
                    1,
                    65540,
                    65540,
                    1000,
                    std::ptr::null(),
                )
            };
            assert_ne!(raw, INVALID_HANDLE_VALUE);
            let pipe = PipeHandle(raw);
            ready_tx.send(()).unwrap();
            unsafe {
                ConnectNamedPipe(pipe.0, std::ptr::null_mut());
            }
            let _ = stop_rx.recv_timeout(Duration::from_secs(5));
        });
        ready_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let client = AdminClient {
            pipe_name,
            timeout: Duration::from_millis(100),
            failure: std::sync::Mutex::new(None),
        };
        let request = AdminRequest {
            protocol_version: domain::PROTOCOL_VERSION,
            request_id: domain::RequestId(1),
            operation: domain::AdminOperation::QueryRecycleBin {
                user_scope: domain::UserScope::CurrentUser,
            },
        };
        let start = std::time::Instant::now();
        assert!(client.request(&request).is_err());
        assert!(start.elapsed() < Duration::from_secs(1));
        let start = std::time::Instant::now();
        for _ in 0..100 {
            assert!(client.request(&request).is_err());
        }
        assert!(start.elapsed() < Duration::from_millis(100));
        stop_tx.send(()).unwrap();
        server.join().unwrap();
    }
}
