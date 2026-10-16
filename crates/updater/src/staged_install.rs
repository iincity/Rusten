use crate::{manifest::PackageKind, signature::VerifiedPackage};
use sha2::{Digest, Sha256};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

pub fn target_path(root: &Path, kind: PackageKind) -> PathBuf {
    root.join(match kind {
        PackageKind::Product => "product/rusten.exe",
        PackageKind::Rules => "rules/builtin.toml",
        PackageKind::Config => "config/policy.toml",
    })
}

pub fn install_verified_at_root(
    package: &VerifiedPackage,
    staged: &Path,
    root: &Path,
    os_version: &str,
    now: u64,
) -> io::Result<PathBuf> {
    install_verified(
        package,
        staged,
        &target_path(root, package.manifest().kind),
        os_version,
        now,
    )
}

pub fn recover_pending_at_root(root: &Path, kind: PackageKind) -> io::Result<bool> {
    recover_pending(&target_path(root, kind))
}

pub fn recover_pending(current: &Path) -> io::Result<bool> {
    let pending = current.with_extension("pending");
    match fs::remove_file(pending) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn install_verified(
    package: &VerifiedPackage,
    staged: &Path,
    current: &Path,
    os_version: &str,
    now: u64,
) -> io::Result<PathBuf> {
    recover_pending(current).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("recovering pending package failed: {error}"),
        )
    })?;
    if !package.manifest().allowed_name() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "package name not allowed",
        ));
    }
    if !staged.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "staged package missing",
        ));
    }
    if !package.manifest().valid_at(os_version, now) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "package is expired or incompatible with this system",
        ));
    }
    let bytes = fs::read(staged).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("reading staged package failed: {error}"),
        )
    })?;
    let digest: [u8; 32] = Sha256::digest(&bytes).into();
    if &digest != package.digest() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "staged package changed after verification",
        ));
    }
    fs::create_dir_all(current.parent().unwrap_or_else(|| Path::new("."))).map_err(|error| {
        io::Error::new(
            error.kind(),
            format!("creating install directory failed: {error}"),
        )
    })?;
    let backup = current.with_extension("previous");
    let temporary = current.with_extension("pending");
    let result = fs::copy(staged, &temporary)
        .map_err(|error| io::Error::new(error.kind(), format!("staging package failed: {error}")))
        .and_then(|_| sync_file(&temporary))
        .and_then(|_| {
            replace_atomically(
                &temporary,
                current,
                current.exists().then_some(backup.as_path()),
            )
        });
    if let Err(error) = result {
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    Ok(backup)
}

pub fn install_verified_with_fallback_at_root(
    package: &VerifiedPackage,
    staged: &Path,
    root: &Path,
    fallback: &VerifiedPackage,
    fallback_staged: &Path,
    os_version: &str,
    now: u64,
) -> io::Result<PathBuf> {
    install_verified_with_fallback(
        package,
        staged,
        &target_path(root, package.manifest().kind),
        fallback,
        fallback_staged,
        os_version,
        now,
    )
}

fn install_verified_with_fallback(
    package: &VerifiedPackage,
    staged: &Path,
    current: &Path,
    fallback: &VerifiedPackage,
    fallback_staged: &Path,
    os_version: &str,
    now: u64,
) -> io::Result<PathBuf> {
    match install_verified(package, staged, current, os_version, now) {
        Ok(previous) => Ok(previous),
        Err(error) if package.manifest().kind == PackageKind::Rules => {
            if fallback.manifest().kind != PackageKind::Rules
                || fallback.manifest().version
                    != package
                        .manifest()
                        .rollback_version
                        .as_deref()
                        .unwrap_or_default()
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "fallback is not the verified rollback version",
                ));
            }
            if !fallback.manifest().valid_at(os_version, now) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "verified fallback is expired or incompatible",
                ));
            }
            let bytes = fs::read(fallback_staged)?;
            let digest: [u8; 32] = Sha256::digest(&bytes).into();
            if &digest != fallback.digest() {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "verified fallback changed after verification",
                ));
            }
            let temporary = current.with_extension("fallback.pending");
            let result = fs::copy(fallback_staged, &temporary)
                .map_err(|error| {
                    io::Error::new(error.kind(), format!("staging fallback failed: {error}"))
                })
                .and_then(|_| sync_file(&temporary))
                .and_then(|_| replace_atomically(&temporary, current, None));
            if let Err(fallback_error) = result {
                let _ = fs::remove_file(&temporary);
                return Err(io::Error::new(
                    fallback_error.kind(),
                    format!(
                        "rules package and verified fallback failed: {error}; fallback: {fallback_error}"
                    ),
                ));
            }
            Ok(current.to_path_buf())
        }
        Err(error) => Err(error),
    }
}

pub(crate) fn replace_atomically(
    replacement: &Path,
    destination: &Path,
    backup: Option<&Path>,
) -> io::Result<()> {
    #[cfg(windows)]
    {
        let replacement = wide(replacement);
        let destination = wide(destination);
        let backup = backup.map(wide);
        if destination[..destination.len().saturating_sub(1)].is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "destination missing",
            ));
        }
        let flags = windows_sys::Win32::Storage::FileSystem::MOVEFILE_REPLACE_EXISTING
            | windows_sys::Win32::Storage::FileSystem::MOVEFILE_WRITE_THROUGH;
        if destination_path_exists(&destination)
            && let Some(backup) = backup.as_ref()
        {
            let backup_path =
                PathBuf::from(String::from_utf16_lossy(backup).trim_end_matches('\0'));
            if backup_path.exists() {
                fs::remove_file(&backup_path)?;
            }
            let moved = unsafe {
                windows_sys::Win32::Storage::FileSystem::MoveFileExW(
                    destination.as_ptr(),
                    backup.as_ptr(),
                    flags,
                )
            };
            if moved == 0 {
                let error = io::Error::last_os_error();
                return Err(io::Error::new(
                    error.kind(),
                    format!("moving current package to backup failed: {error}"),
                ));
            }
        }
        let ok = unsafe {
            windows_sys::Win32::Storage::FileSystem::MoveFileExW(
                replacement.as_ptr(),
                destination.as_ptr(),
                flags,
            )
        };
        if ok == 0 {
            let error = io::Error::last_os_error();
            if let Some(backup) = backup.as_ref()
                && !destination_path_exists(&destination)
            {
                let _ = unsafe {
                    windows_sys::Win32::Storage::FileSystem::MoveFileExW(
                        backup.as_ptr(),
                        destination.as_ptr(),
                        flags,
                    )
                };
            }
            return Err(io::Error::new(
                error.kind(),
                format!("moving pending package into place failed: {error}"),
            ));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        if let Some(backup) = backup.filter(|_| destination.exists()) {
            fs::rename(destination, backup)?;
        }
        fs::rename(replacement, destination)
    }
}

#[cfg(windows)]
fn wide(path: &Path) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    path.as_os_str().encode_wide().chain([0]).collect()
}

#[cfg(windows)]
fn destination_path_exists(path: &[u16]) -> bool {
    let path = String::from_utf16_lossy(path);
    Path::new(path.trim_end_matches('\0')).exists()
}

fn sync_file(path: &Path) -> io::Result<()> {
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)?
        .sync_all()
}
