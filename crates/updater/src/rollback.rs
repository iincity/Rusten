use crate::{
    manifest::Manifest,
    signature::{Ed25519TrustRoot, VerifiedPackage, verify_package_for},
    staged_install::replace_atomically,
};
use sha2::Digest;
use std::{fs, io, path::Path};

pub fn rollback(current: &Path, previous: &Path) -> io::Result<()> {
    if !previous.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            "previous package missing",
        ));
    }
    let metadata = fs::metadata(previous)?;
    if metadata.len() == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "previous package is empty",
        ));
    }
    let temporary = current.with_extension("rollback.pending");
    fs::copy(previous, &temporary)?;
    fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&temporary)?
        .sync_all()?;
    replace_atomically(&temporary, current, None)
}

pub fn rollback_verified(
    current: &Path,
    previous: &Path,
    manifest: &Manifest,
    trust_root: &Ed25519TrustRoot,
    os_version: &str,
    now: u64,
) -> io::Result<()> {
    let bytes = fs::read(previous)?;
    let verified = verify_package_for(manifest, &bytes, trust_root, os_version, now)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    rollback_package(current, previous, &verified)
}

fn rollback_package(current: &Path, previous: &Path, verified: &VerifiedPackage) -> io::Result<()> {
    let bytes = fs::read(previous)?;
    let digest: [u8; 32] = sha2::Digest::finalize(sha2::Sha256::new_with_prefix(&bytes)).into();
    if &digest != verified.digest() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "rollback package changed after verification",
        ));
    }
    rollback(current, previous)
}
