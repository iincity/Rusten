use serde::Deserialize;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
pub enum RiskLevel {
    #[serde(rename = "P0", alias = "p0")]
    P0,
    #[serde(rename = "P1", alias = "p1")]
    P1,
    #[serde(rename = "P2", alias = "p2")]
    P2,
    #[serde(rename = "P3", alias = "p3")]
    P3,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
pub enum RecoveryMode {
    #[serde(rename = "recycle")]
    Recycle,
    #[serde(rename = "backup_then_delete")]
    BackupThenDelete,
    #[serde(rename = "delete_only")]
    DeleteOnly,
    #[serde(rename = "shred_once")]
    ShredOnce,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
pub enum CleanupMode {
    #[serde(rename = "quick")]
    Quick,
    #[serde(rename = "manual")]
    Manual,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
pub enum Confidence {
    #[serde(rename = "high")]
    High,
    #[serde(rename = "low")]
    Low,
}
