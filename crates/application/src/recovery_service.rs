use domain::{FileIdentity, NormalizedPath};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
pub struct RecoveryId(pub u64);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct RecoveryItem {
    pub id: RecoveryId,
    pub original_path: NormalizedPath,
    pub current_path: Option<NormalizedPath>,
    pub file_identity: FileIdentity,
    pub size: u64,
    pub backup_path: Option<NormalizedPath>,
    pub recoverable: bool,
    #[serde(default)]
    pub created_at: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub enum RestoreDestination {
    Original,
    Path(NormalizedPath),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecoveryError {
    NotFound,
    NotRecoverable,
    QuotaExceeded,
    DestinationExists,
    RecoveryChainRequired,
    InvalidDestination,
    Io(String),
}

pub trait RecoveryStore: Send + Sync {
    fn record(&self, item: RecoveryItem) -> Result<RecoveryId, RecoveryError>;
    fn restore(&self, id: RecoveryId, destination: RestoreDestination)
    -> Result<(), RecoveryError>;

    fn restore_many(
        &self,
        ids: &[RecoveryId],
        destination: RestoreDestination,
    ) -> Vec<(RecoveryId, Result<(), RecoveryError>)> {
        ids.iter()
            .copied()
            .map(|id| (id, self.restore(id, destination.clone())))
            .collect()
    }
}

pub fn choose_destination(item: &RecoveryItem, destination: RestoreDestination) -> NormalizedPath {
    match destination {
        RestoreDestination::Original => item.original_path.clone(),
        RestoreDestination::Path(path) => path,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use domain::normalize_path;

    #[test]
    fn original_destination_is_explicit_and_stable() {
        let path = normalize_path(r"C:\Windows\Temp\cache.tmp").unwrap();
        let item = RecoveryItem {
            id: RecoveryId(1),
            original_path: path.clone(),
            current_path: None,
            file_identity: FileIdentity {
                volume_serial: 1,
                file_index: 2,
            },
            size: 3,
            backup_path: None,
            recoverable: true,
            created_at: 0,
        };
        assert_eq!(
            choose_destination(&item, RestoreDestination::Original),
            path
        );
    }
}
