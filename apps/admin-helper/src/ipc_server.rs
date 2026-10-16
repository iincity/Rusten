use std::io::{Read, Write};

use domain::{
    AdminRejectReason, AdminResponse, AdminResult, MAX_FRAME_BYTES, ProtocolError, decode_request,
    encode_response,
};

use crate::{authorization, request_handler::RequestHandler};

pub const PIPE_NAME: &str = r"\\.\pipe\RustenAdminV1";

#[cfg(windows)]
pub fn serve_once(handler: &mut RequestHandler) -> Result<bool, String> {
    use std::{fs::File, os::windows::io::FromRawHandle, ptr};
    use windows_sys::Win32::{
        Foundation::{
            CloseHandle, ERROR_PIPE_CONNECTED, ERROR_PIPE_LISTENING, GetLastError,
            INVALID_HANDLE_VALUE,
        },
        Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX},
        System::Pipes::{
            ConnectNamedPipe, CreateNamedPipeW, PIPE_NOWAIT, PIPE_READMODE_BYTE, PIPE_TYPE_MESSAGE,
            PIPE_WAIT, SetNamedPipeHandleState,
        },
    };

    let security = authorization::PipeSecurity::current_user()?;
    let name: Vec<u16> = PIPE_NAME.encode_utf16().chain(Some(0)).collect();
    let pipe = unsafe {
        CreateNamedPipeW(
            name.as_ptr(),
            PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_MESSAGE | PIPE_READMODE_BYTE | PIPE_NOWAIT,
            1,
            (MAX_FRAME_BYTES + 4) as u32,
            (MAX_FRAME_BYTES + 4) as u32,
            5_000,
            &security.attributes,
        )
    };
    if pipe == INVALID_HANDLE_VALUE {
        return Err(format!("CreateNamedPipeW failed: {}", unsafe {
            GetLastError()
        }));
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        let connected = unsafe { ConnectNamedPipe(pipe, ptr::null_mut()) } != 0;
        let error = unsafe { GetLastError() };
        if connected || error == ERROR_PIPE_CONNECTED {
            break;
        }
        if error != ERROR_PIPE_LISTENING {
            unsafe {
                CloseHandle(pipe);
            }
            return Err(format!("ConnectNamedPipe failed: {error}"));
        }
        if std::time::Instant::now() >= deadline {
            unsafe {
                CloseHandle(pipe);
            }
            return Ok(false);
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let mode = PIPE_READMODE_BYTE | PIPE_WAIT;
    if unsafe { SetNamedPipeHandleState(pipe, &mode, ptr::null(), ptr::null()) } == 0 {
        let error = std::io::Error::last_os_error();
        unsafe {
            CloseHandle(pipe);
        }
        return Err(error.to_string());
    }
    if let Err(error) = security.validate_client(pipe) {
        unsafe {
            CloseHandle(pipe);
        }
        return Err(error);
    }
    let mut stream = unsafe { File::from_raw_handle(pipe as *mut _) };
    let request = read_frame(&mut stream).map_err(|error| format!("read request: {error:?}"))?;
    let response = decode_request(&request)
        .map(|request| handler.handle(request))
        .unwrap_or_else(|_| AdminResponse {
            protocol_version: domain::PROTOCOL_VERSION,
            request_id: domain::RequestId(0),
            result: AdminResult::Rejected {
                reason: AdminRejectReason::InvalidRequest,
            },
        });
    let frame =
        encode_response(&response).map_err(|error| format!("encode response: {error:?}"))?;
    stream
        .write_all(&frame)
        .map_err(|error| error.to_string())?;
    if unsafe { windows_sys::Win32::Storage::FileSystem::FlushFileBuffers(pipe) } == 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(true)
}

#[cfg(not(windows))]
pub fn serve_once(_handler: &mut RequestHandler) -> Result<bool, String> {
    Err("admin helper requires Windows".into())
}

fn read_frame(reader: &mut impl Read) -> Result<Vec<u8>, ProtocolError> {
    let mut length = [0; 4];
    reader
        .read_exact(&mut length)
        .map_err(|_| ProtocolError::InvalidFrame)?;
    let length = u32::from_le_bytes(length) as usize;
    if length > MAX_FRAME_BYTES {
        return Err(ProtocolError::FrameTooLarge);
    }
    let mut payload = vec![0; 4 + length];
    payload[..4].copy_from_slice(&(length as u32).to_le_bytes());
    reader
        .read_exact(&mut payload[4..])
        .map_err(|_| ProtocolError::InvalidFrame)?;
    Ok(payload)
}
