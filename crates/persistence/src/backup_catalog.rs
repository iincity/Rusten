use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};

use application::{
    BackupQuota, RecoveryError, RecoveryId, RecoveryItem, RecoveryStore, RestoreDestination,
    choose_destination,
};
use domain::normalize_path;

pub struct BackupCatalog {
    path: PathBuf,
    items: Mutex<BTreeMap<RecoveryId, RecoveryItem>>,
    corrupt_records: Vec<CorruptRecord>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CorruptRecord {
    pub line: String,
    pub reason: String,
}

impl BackupCatalog {
    pub fn open(path: impl Into<PathBuf>) -> io::Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut items = BTreeMap::new();
        let mut corrupt_records = Vec::new();
        if path.exists() {
            for line in fs::read_to_string(&path)?.lines() {
                if let Ok(item) = serde_json::from_str::<RecoveryItem>(line) {
                    let valid_backup = !item.recoverable
                        || item.backup_path.as_ref().is_some_and(|backup| {
                            native_path(backup).metadata().is_ok_and(|metadata| {
                                metadata.is_file() && metadata.len() == item.size
                            })
                        });
                    if valid_backup {
                        items.insert(item.id, item);
                    } else {
                        corrupt_records.push(CorruptRecord {
                            line: line.to_owned(),
                            reason: "recoverable backup is missing or size-mismatched".into(),
                        });
                    }
                } else if !line.trim().is_empty() {
                    corrupt_records.push(CorruptRecord {
                        line: line.to_owned(),
                        reason: "invalid recovery record".into(),
                    });
                }
            }
        }
        Ok(Self {
            path,
            items: Mutex::new(items),
            corrupt_records,
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn corrupt_record_count(&self) -> usize {
        self.corrupt_records.len()
    }

    pub fn corrupt_records(&self) -> &[CorruptRecord] {
        &self.corrupt_records
    }

    pub fn items(&self) -> Vec<RecoveryItem> {
        self.items
            .lock()
            .map(|items| items.values().cloned().collect())
            .unwrap_or_default()
    }

    pub fn used_bytes(&self) -> u64 {
        self.items
            .lock()
            .map(|items| {
                items
                    .values()
                    .filter(|item| item.recoverable)
                    .fold(0_u64, |total, item| total.saturating_add(item.size))
            })
            .unwrap_or(0)
    }

    pub fn quota(&self, max_bytes: u64) -> BackupQuota {
        let items = self.items();
        BackupQuota {
            max_bytes,
            used_bytes: self.used_bytes(),
            recoverable_items: items.iter().filter(|item| item.recoverable).count() as u64,
        }
    }

    pub fn can_add(&self, max_bytes: u64, size: u64) -> bool {
        self.used_bytes().saturating_add(size) <= max_bytes
    }

    pub fn record_with_quota(
        &self,
        item: RecoveryItem,
        max_bytes: u64,
    ) -> Result<RecoveryId, RecoveryError> {
        validate_recoverable_backup(&item)?;
        let mut items = self
            .items
            .lock()
            .map_err(|_| RecoveryError::Io("backup catalog lock unavailable".into()))?;
        if items.contains_key(&item.id) {
            return Err(RecoveryError::Io("duplicate recovery ID".into()));
        }
        let used = items
            .values()
            .filter(|existing| existing.recoverable)
            .fold(0_u64, |total, existing| total.saturating_add(existing.size));
        if item.recoverable && used.saturating_add(item.size) > max_bytes {
            return Err(RecoveryError::QuotaExceeded);
        }
        self.append(&item)?;
        let id = item.id;
        items.insert(id, item);
        Ok(id)
    }

    pub fn purge_expired(
        &self,
        now: u64,
        retention_secs: u64,
        max_bytes: u64,
    ) -> Result<usize, RecoveryError> {
        let ids = self
            .items
            .lock()
            .map_err(|_| RecoveryError::Io("backup catalog lock unavailable".into()))?
            .values()
            .filter(|item| {
                !item.recoverable
                    && item.backup_path.is_some()
                    && item.created_at.saturating_add(retention_secs) <= now
            })
            .map(|item| item.id)
            .collect::<Vec<_>>();
        let mut purged = 0;
        for id in ids {
            self.purge(id)?;
            purged += 1;
        }
        while self.used_bytes() > max_bytes {
            let next = self
                .items
                .lock()
                .map_err(|_| RecoveryError::Io("backup catalog lock unavailable".into()))?
                .values()
                .filter(|item| !item.recoverable && item.backup_path.is_some())
                .min_by_key(|item| (item.created_at, item.id))
                .map(|item| item.id);
            let Some(id) = next else {
                break;
            };
            self.purge(id)?;
            purged += 1;
        }
        Ok(purged)
    }

    pub fn next_recovery_id(&self) -> RecoveryId {
        self.items
            .lock()
            .ok()
            .and_then(|items| items.keys().next_back().map(|id| id.0.saturating_add(1)))
            .map(RecoveryId)
            .unwrap_or(RecoveryId(1))
    }

    pub fn purge(&self, id: RecoveryId) -> Result<(), RecoveryError> {
        let mut items = self
            .items
            .lock()
            .map_err(|_| RecoveryError::Io("backup catalog lock unavailable".into()))?;
        let item = items.get(&id).ok_or(RecoveryError::NotFound)?.clone();
        if item.recoverable {
            return Err(RecoveryError::RecoveryChainRequired);
        }
        if let Some(backup) = &item.backup_path {
            fs::remove_file(native_path(backup)).map_err(|error| {
                if error.kind() == io::ErrorKind::NotFound {
                    RecoveryError::Io("backup is already missing".into())
                } else {
                    RecoveryError::Io(error.to_string())
                }
            })?;
        }
        let mut purged = item;
        purged.backup_path = None;
        self.append(&purged)?;
        items.insert(id, purged);
        Ok(())
    }

    fn append(&self, item: &RecoveryItem) -> Result<(), RecoveryError> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|error| RecoveryError::Io(error.to_string()))?;
        serde_json::to_writer(&mut file, item)
            .map_err(|error| RecoveryError::Io(error.to_string()))?;
        file.write_all(b"\n")
            .map_err(|error| RecoveryError::Io(error.to_string()))
    }
}

impl RecoveryStore for BackupCatalog {
    fn record(&self, item: RecoveryItem) -> Result<RecoveryId, RecoveryError> {
        validate_recoverable_backup(&item)?;
        let mut items = self
            .items
            .lock()
            .map_err(|_| RecoveryError::Io("backup catalog lock unavailable".into()))?;
        if items.contains_key(&item.id) {
            return Err(RecoveryError::Io("duplicate recovery ID".into()));
        }
        self.append(&item)?;
        let id = item.id;
        items.insert(id, item);
        Ok(id)
    }

    fn restore(
        &self,
        id: RecoveryId,
        destination: RestoreDestination,
    ) -> Result<(), RecoveryError> {
        let mut items = self
            .items
            .lock()
            .map_err(|_| RecoveryError::Io("backup catalog lock unavailable".into()))?;
        let item = items.get(&id).ok_or(RecoveryError::NotFound)?.clone();
        if !item.recoverable {
            return Err(RecoveryError::NotRecoverable);
        }
        let destination = choose_destination(&item, destination);
        let source = item
            .backup_path
            .as_ref()
            .ok_or(RecoveryError::NotRecoverable)?;
        if normalize_path(destination.display()).is_err() {
            return Err(RecoveryError::InvalidDestination);
        }
        let destination_path = native_path(&destination);
        if destination_path.exists() {
            return Err(RecoveryError::DestinationExists);
        }
        if let Some(parent) = destination_path.parent() {
            fs::create_dir_all(parent).map_err(|error| RecoveryError::Io(error.to_string()))?;
        }
        fs::copy(native_path(source), &destination_path)
            .map_err(|error| RecoveryError::Io(error.to_string()))?;
        let mut restored = item;
        restored.recoverable = false;
        if let Err(error) = self.append(&restored) {
            let _ = fs::remove_file(&destination_path);
            return Err(error);
        }
        items.insert(id, restored);
        Ok(())
    }
}

fn validate_recoverable_backup(item: &RecoveryItem) -> Result<(), RecoveryError> {
    if !item.recoverable {
        return Ok(());
    }
    let backup = item
        .backup_path
        .as_ref()
        .ok_or_else(|| RecoveryError::Io("recoverable backup path is missing".into()))?;
    let actual_size = native_path(backup)
        .metadata()
        .map_err(|error| RecoveryError::Io(error.to_string()))?
        .len();
    if actual_size != item.size {
        return Err(RecoveryError::Io(format!(
            "recoverable backup size mismatch: expected {}, found {}",
            item.size, actual_size
        )));
    }
    Ok(())
}

fn native_path(path: &domain::NormalizedPath) -> PathBuf {
    #[cfg(windows)]
    {
        PathBuf::from(path.extended())
    }
    #[cfg(not(windows))]
    {
        PathBuf::from(path.display())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use application::RecoveryStore;
    use domain::{FileIdentity, normalize_path};

    #[test]
    fn catalog_round_trips_record_and_rejects_missing_backup() {
        let root = tempfile::tempdir().unwrap();
        let catalog = BackupCatalog::open(root.path().join("backup.jsonl")).unwrap();
        let original = normalize_path(r"C:\Windows\Temp\cache.tmp").unwrap();
        let item = RecoveryItem {
            id: RecoveryId(1),
            original_path: original,
            current_path: None,
            file_identity: FileIdentity {
                volume_serial: 1,
                file_index: 1,
            },
            size: 4,
            backup_path: None,
            recoverable: true,
            created_at: 0,
        };
        assert!(matches!(
            catalog.record(item),
            Err(RecoveryError::Io(message)) if message.contains("backup path")
        ));
    }

    #[test]
    fn restored_state_survives_reopen_and_recoverable_items_cannot_be_purged() {
        let root = tempfile::tempdir().unwrap();
        let backup = root.path().join("cache.bin");
        fs::write(&backup, b"data").unwrap();
        let catalog_path = root.path().join("backup.jsonl");
        let catalog = BackupCatalog::open(&catalog_path).unwrap();
        let original = normalize_path(r"C:\Windows\Temp\cache.bin").unwrap();
        let backup_path = normalize_path(&backup.to_string_lossy()).unwrap();
        catalog
            .record(RecoveryItem {
                id: RecoveryId(2),
                original_path: original,
                current_path: None,
                file_identity: FileIdentity {
                    volume_serial: 1,
                    file_index: 2,
                },
                size: 4,
                backup_path: Some(backup_path),
                recoverable: true,
                created_at: 0,
            })
            .unwrap();
        assert_eq!(
            catalog.purge(RecoveryId(2)),
            Err(RecoveryError::RecoveryChainRequired)
        );
        assert!(
            catalog
                .restore(
                    RecoveryId(2),
                    RestoreDestination::Path(
                        normalize_path(&root.path().join("restored.bin").to_string_lossy())
                            .unwrap()
                    )
                )
                .is_ok()
        );
        let reopened = BackupCatalog::open(catalog_path).unwrap();
        assert!(!reopened.items()[0].recoverable);
    }

    #[test]
    fn retention_purges_only_expired_non_recoverable_backups() {
        let root = tempfile::tempdir().unwrap();
        let backup = root.path().join("old.bin");
        fs::write(&backup, b"data").unwrap();
        let catalog = BackupCatalog::open(root.path().join("backup.jsonl")).unwrap();
        let original = normalize_path(r"C:\Windows\Temp\old.bin").unwrap();
        let backup_path = normalize_path(&backup.to_string_lossy()).unwrap();
        catalog
            .record(RecoveryItem {
                id: RecoveryId(3),
                original_path: original,
                current_path: None,
                file_identity: FileIdentity {
                    volume_serial: 1,
                    file_index: 3,
                },
                size: 4,
                backup_path: Some(backup_path),
                recoverable: false,
                created_at: 1,
            })
            .unwrap();
        assert_eq!(catalog.purge_expired(100, 10, 0).unwrap(), 1);
        assert!(!backup.exists());
        assert!(catalog.items()[0].backup_path.is_none());
    }

    #[test]
    fn recoverable_backup_size_must_match_the_backup_file() {
        let root = tempfile::tempdir().unwrap();
        let backup = root.path().join("backup.bin");
        fs::write(&backup, b"data").unwrap();
        let catalog = BackupCatalog::open(root.path().join("backup.jsonl")).unwrap();
        let item = RecoveryItem {
            id: RecoveryId(4),
            original_path: normalize_path(r"C:\Windows\Temp\cache.bin").unwrap(),
            current_path: None,
            file_identity: FileIdentity {
                volume_serial: 1,
                file_index: 4,
            },
            size: 8,
            backup_path: Some(normalize_path(&backup.to_string_lossy()).unwrap()),
            recoverable: true,
            created_at: 0,
        };

        assert!(matches!(
            catalog.record(item),
            Err(RecoveryError::Io(message)) if message.contains("size")
        ));
    }
}
