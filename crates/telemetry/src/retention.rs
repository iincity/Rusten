use std::{fs, io, path::Path};

use crate::event_model::TelemetryEvent;

pub fn prune(path: &Path, max_bytes: u64) -> io::Result<()> {
    if fs::metadata(path).map(|meta| meta.len()).unwrap_or(0) > max_bytes {
        fs::write(path, [])?;
    }
    Ok(())
}

pub fn prune_older_than(path: &Path, now: u64, max_age_secs: u64) -> io::Result<()> {
    if !path.is_file() {
        return Ok(());
    }
    let cutoff = now.saturating_sub(max_age_secs);
    let kept = fs::read_to_string(path)?
        .lines()
        .filter_map(|line| serde_json::from_str::<TelemetryEvent>(line).ok())
        .filter(|event| event.created_at >= cutoff)
        .map(|event| serde_json::to_string(&event).map_err(io::Error::other))
        .collect::<Result<Vec<_>, _>>()?;
    fs::write(path, kept.join("\n").as_bytes())
}

pub fn delete(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}
