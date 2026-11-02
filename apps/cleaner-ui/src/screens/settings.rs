use std::path::{Path, PathBuf};

use application::ConfigService;
use persistence::ConfigStore;

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SettingsModel {
    pub auto_clean: bool,
    pub theme: crate::theme::ThemeMode,
}

impl SettingsModel {
    pub fn load_from(path: &Path) -> Self {
        ConfigStore::read(path)
            .ok()
            .map(|contents| {
                let config = ConfigService::load_toml(&contents).config;
                Self {
                    auto_clean: config.auto_clean,
                    theme: crate::theme::ThemeMode::from_key(&config.theme),
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
        config.theme = self.theme.key().to_owned();
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
            theme: crate::theme::ThemeMode::Dark,
        };
        settings.save_to(&path).unwrap();
        let loaded = ConfigService::load_toml(&ConfigStore::read(&path).unwrap()).config;
        assert_eq!(loaded.backup_quota_bytes, 123);
        let _ = std::fs::remove_file(path);
    }
}
