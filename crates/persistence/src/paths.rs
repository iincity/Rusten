use std::{env, path::PathBuf};

pub fn data_root() -> PathBuf {
    env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| env::temp_dir().join("Rusten"))
        .join("Rusten")
}

pub fn config_path() -> PathBuf {
    data_root().join("config.toml")
}
pub fn operation_log_path() -> PathBuf {
    data_root().join("operations.sqlite")
}
