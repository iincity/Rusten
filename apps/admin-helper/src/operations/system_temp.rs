use std::{
    fs,
    path::{Path, PathBuf},
};

use domain::{AdminRejectReason, AdminResult, PlanId, is_forbidden_path, normalize_path};

pub fn clean(plan_id: PlanId) -> AdminResult {
    if plan_id.0 == 0 {
        return AdminResult::Rejected {
            reason: AdminRejectReason::InvalidRequest,
        };
    }
    let Ok(root) = system_temp_root() else {
        return AdminResult::Rejected {
            reason: AdminRejectReason::OperationFailed,
        };
    };
    clean_files(&root)
}

fn clean_files(root: &Path) -> AdminResult {
    let Ok(normalized) = normalize_path(&root.to_string_lossy()) else {
        return AdminResult::Rejected {
            reason: AdminRejectReason::ForbiddenPath,
        };
    };
    if !is_system_temp_root(&normalized) || !root.is_dir() {
        return AdminResult::Rejected {
            reason: AdminRejectReason::ForbiddenPath,
        };
    }
    let Ok(entries) = fs::read_dir(root) else {
        return AdminResult::Rejected {
            reason: AdminRejectReason::OperationFailed,
        };
    };
    let mut affected = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(child) = normalize_path(&path.to_string_lossy()) else {
            continue;
        };
        let is_regular_file = fs::symlink_metadata(&path)
            .map(|metadata| metadata.file_type().is_file() && !metadata.file_type().is_symlink())
            .unwrap_or(false);
        if is_regular_file
            && is_safe_temp_child(&normalized, &child)
            && fs::remove_file(path).is_ok()
        {
            affected += 1;
        }
    }
    AdminResult::Completed { affected }
}

fn system_temp_root() -> Result<PathBuf, String> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStringExt;
        use windows_sys::Win32::System::SystemInformation::GetSystemWindowsDirectoryW;

        let mut buffer = vec![0_u16; 260];
        let length =
            unsafe { GetSystemWindowsDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) };
        if length == 0 || length as usize >= buffer.len() {
            return Err("cannot determine the system Windows directory".into());
        }
        buffer.truncate(length as usize);
        Ok(PathBuf::from(std::ffi::OsString::from_wide(&buffer)).join("Temp"))
    }
    #[cfg(not(windows))]
    {
        Err("system temp cleanup requires Windows".into())
    }
}

fn is_system_temp_root(path: &domain::NormalizedPath) -> bool {
    let components = path
        .display()
        .split('\\')
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>();
    components.len() == 3
        && components[1].eq_ignore_ascii_case("Windows")
        && components[2].eq_ignore_ascii_case("Temp")
}

fn is_safe_temp_child(root: &domain::NormalizedPath, child: &domain::NormalizedPath) -> bool {
    if is_forbidden_path(child) {
        return false;
    }
    let root = root.display().trim_end_matches(['\\', '/']);
    let child = child.display();
    child.len() > root.len()
        && child[..root.len()].eq_ignore_ascii_case(root)
        && matches!(child.as_bytes().get(root.len()), Some(b'\\'))
}

#[cfg(test)]
mod tests {
    use super::{is_safe_temp_child, is_system_temp_root};
    use domain::normalize_path;

    #[test]
    fn temp_guard_accepts_only_windows_temp_children() {
        let root = normalize_path(r"C:\Windows\Temp").unwrap();
        let child = normalize_path(r"C:\Windows\Temp\cache.tmp").unwrap();
        let sensitive = normalize_path(r"C:\Windows\Temp\System\cache.tmp").unwrap();
        let other = normalize_path(r"C:\Users\Alice\AppData\Local\Temp\cache.tmp").unwrap();

        assert!(is_system_temp_root(&root));
        assert!(is_safe_temp_child(&root, &child));
        assert!(!is_safe_temp_child(&root, &sensitive));
        assert!(!is_safe_temp_child(&root, &other));
    }
}
