use std::sync::atomic::{AtomicBool, AtomicIsize, Ordering};

use nsh_platform_windows::windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE},
};

static REQUESTED: AtomicIsize = AtomicIsize::new(0);
static APPROVED: AtomicBool = AtomicBool::new(false);

/// Routes native close requests to the themed application confirmation.
pub fn request_window_close(hwnd: HWND) -> bool {
    if APPROVED.swap(false, Ordering::AcqRel) {
        return true;
    }
    REQUESTED.store(hwnd.0 as isize, Ordering::Release);
    false
}

pub(crate) fn take_request() -> Option<isize> {
    let hwnd = REQUESTED.swap(0, Ordering::AcqRel);
    (hwnd != 0).then_some(hwnd)
}

pub(crate) fn finish_close(hwnd: isize) {
    APPROVED.store(true, Ordering::Release);
    unsafe {
        if PostMessageW(Some(HWND(hwnd as _)), WM_CLOSE, WPARAM(0), LPARAM(0)).is_err() {
            APPROVED.store(false, Ordering::Release);
        }
    }
}
