use persistence::ConfigStore;
use std::{
    fs,
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[test]
fn repeated_writes_replace_existing_config_without_temp_files() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let directory = std::env::temp_dir().join(format!(
        "rusten-config-{unique}-{}",
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    ));
    let path = directory.join("config.toml");

    ConfigStore::write(&path, "first = true\n").unwrap();
    ConfigStore::write(&path, "second = true\n").unwrap();
    assert_eq!(fs::read_to_string(&path).unwrap(), "second = true\n");
    assert_eq!(fs::read_dir(&directory).unwrap().count(), 1);

    fs::remove_dir_all(directory).unwrap();
}
