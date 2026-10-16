use std::{ffi::OsStr, os::windows::ffi::OsStrExt};

use domain::NormalizedPath;
use windows_sys::Win32::Globalization::{CSTR_EQUAL, CompareStringOrdinal};

use super::{PlatformError, PlatformErrorKind, metadata::read_metadata};

pub fn paths_equal_ordinal(left: &str, right: &str) -> bool {
    let left: Vec<u16> = OsStr::new(left).encode_wide().collect();
    let right: Vec<u16> = OsStr::new(right).encode_wide().collect();
    ordinal_equal(&left, &right)
}

pub fn validate_physical_target(
    root: &NormalizedPath,
    target: &NormalizedPath,
) -> Result<(), PlatformError> {
    let root_metadata = read_metadata(root)?;
    let target_metadata = read_metadata(target)?;
    if root_metadata.identity().volume_serial != target_metadata.identity().volume_serial
        || !physical_path_is_within(
            root_metadata.physical_path(),
            target_metadata.physical_path(),
        )
    {
        return Err(PlatformError::new(
            PlatformErrorKind::PhysicalTargetOutsideRoot,
            Some(target.clone()),
            "physical target is outside the scan root",
        ));
    }
    Ok(())
}

pub(crate) fn physical_path_is_within(root: &str, target: &str) -> bool {
    let mut root: Vec<u16> = OsStr::new(root).encode_wide().collect();
    let target: Vec<u16> = OsStr::new(target).encode_wide().collect();
    while root.len() > 7 && root.last() == Some(&(b'\\' as u16)) {
        root.pop();
    }
    if target.len() < root.len() || !ordinal_equal(&root, &target[..root.len()]) {
        return false;
    }
    target.len() == root.len()
        || root.last() == Some(&(b'\\' as u16))
        || target.get(root.len()) == Some(&(b'\\' as u16))
}

fn ordinal_equal(left: &[u16], right: &[u16]) -> bool {
    unsafe {
        CompareStringOrdinal(
            left.as_ptr(),
            left.len() as i32,
            right.as_ptr(),
            right.len() as i32,
            1,
        ) == CSTR_EQUAL
    }
}
