use std::path::{Path, PathBuf};

use application::ConfigService;
use persistence::ConfigStore;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SettingsModel {
    pub auto_clean: bool,
    pub backup_enabled: bool,
}

impl Default for SettingsModel {
    fn default() -> Self {
        Self {
            auto_clean: false,
            backup_enabled: true,
        }
    }
}

impl SettingsModel {
    pub fn load_from(path: &Path) -> Self {
        ConfigStore::read(path)
            .ok()
            .map(|contents| {
                let config = ConfigService::load_toml(&contents).config;
                Self {
                    auto_clean: config.auto_clean,
                    backup_enabled: config.backup_enabled,
                }
            })
            .unwrap_or_default()
    }

    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        let mut config = ConfigStore::read(path)
            .ok()
            .map(|contents| ConfigService::load_toml(&contents).config)
            .unwrap_or_default();
        config.auto_clean = self.auto_clean;
        config.backup_enabled = self.backup_enabled;
        let contents = ConfigService::serialize(&config)?;
        ConfigStore::write(path, &contents).map_err(|error| error.to_string())
    }

    pub fn default_path() -> Option<PathBuf> {
        std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .map(|root| root.join("Rusten").join("config.toml"))
    }
}

#[cfg(test)]
mod tests {
    use super::SettingsModel;
    use application::{AppConfig, ConfigService};
    use persistence::ConfigStore;

    #[test]
    fn settings_round_trip_through_config_store() {
        let path =
            std::env::temp_dir().join(format!("rusten-settings-test-{}.toml", std::process::id()));
        let mut settings = SettingsModel::load_from(&path);
        settings.auto_clean = true;
        settings.backup_enabled = false;
        settings.save_to(&path).unwrap();

        assert_eq!(SettingsModel::load_from(&path), settings);
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn settings_save_preserves_backup_quota() {
        let path = std::env::temp_dir().join(format!(
            "rusten-settings-quota-test-{}.toml",
            std::process::id()
        ));
        let config = AppConfig {
            backup_quota_bytes: 123,
            ..AppConfig::default()
        };
        ConfigStore::write(&path, &ConfigService::serialize(&config).unwrap()).unwrap();
        let settings = SettingsModel {
            auto_clean: true,
            backup_enabled: false,
        };
        settings.save_to(&path).unwrap();
        let loaded = ConfigService::load_toml(&ConfigStore::read(&path).unwrap()).config;
        assert_eq!(loaded.backup_quota_bytes, 123);
        let _ = std::fs::remove_file(path);
    }
}
