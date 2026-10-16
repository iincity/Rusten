use domain::NormalizedPath;

#[cfg(windows)]
pub fn move_to_recycle_bin(path: &NormalizedPath) -> Result<(), String> {
    use windows_sys::Win32::UI::Shell::{
        FO_DELETE, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, SHFILEOPSTRUCTW,
        SHFileOperationW,
    };

    let mut source: Vec<u16> = path.display().encode_utf16().collect();
    source.extend([0, 0]);
    let mut operation = SHFILEOPSTRUCTW {
        hwnd: std::ptr::null_mut(),
        wFunc: FO_DELETE,
        pFrom: source.as_ptr(),
        pTo: std::ptr::null(),
        // The app owns progress and errors; Shell dialogs would block the worker.
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_NOERRORUI | FOF_SILENT) as u16,
        fAnyOperationsAborted: 0,
        hNameMappings: std::ptr::null_mut(),
        lpszProgressTitle: std::ptr::null(),
    };
    let code = unsafe { SHFileOperationW(&mut operation) };
    if code == 0 && operation.fAnyOperationsAborted == 0 {
        Ok(())
    } else {
        Err(format!("Shell file operation failed: {code}"))
    }
}

#[cfg(not(windows))]
pub fn move_to_recycle_bin(_path: &NormalizedPath) -> Result<(), String> {
    Err("recycle bin requires Windows".into())
}
