use std::{
    fs, io,
    path::{Path, PathBuf},
};

use crate::manifest::Manifest;

pub fn stage_local_package(source: &Path, staging: &Path) -> io::Result<PathBuf> {
    fs::create_dir_all(staging)?;
    let name = source
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "package name missing"))?;
    let source_name = name.to_string_lossy();
    if !["rusten-product", "rusten-rules", "rusten-config"]
        .iter()
        .any(|allowed| {
            source_name == *allowed
                || source_name.starts_with(&format!("{allowed}."))
                || source_name.starts_with(&format!("{allowed}-"))
                || source_name.starts_with(&format!("{allowed}_"))
        })
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "package is outside the allowlist",
        ));
    }
    let destination = staging.join(name);
    fs::copy(source, &destination)?;
    Ok(destination)
}

pub fn stage_manifest_package(
    source: &Path,
    staging: &Path,
    manifest: &Manifest,
) -> io::Result<PathBuf> {
    if !manifest.allowed_name() || !manifest.allowed_kind() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "package manifest is outside the allowlist",
        ));
    }
    let name = source
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "package name missing"))?
        .to_string_lossy();
    if !(name == manifest.name
        || name.starts_with(&format!("{}.", manifest.name))
        || name.starts_with(&format!("{}-", manifest.name))
        || name.starts_with(&format!("{}_", manifest.name)))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "package filename does not match manifest",
        ));
    }
    stage_local_package(source, staging)
}
