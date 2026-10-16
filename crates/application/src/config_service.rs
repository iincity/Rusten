use domain::{is_forbidden_path, normalize_path};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct AppConfig {
    pub scan_root: String,
    pub excluded_paths: Vec<String>,
    pub auto_clean: bool,
    pub backup_enabled: bool,
    pub backup_quota_bytes: u64,
    pub backup_retention_secs: u64,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            scan_root: String::from(r"C:\"),
            excluded_paths: Vec::new(),
            auto_clean: false,
            backup_enabled: true,
            backup_quota_bytes: 10 * 1024 * 1024 * 1024,
            backup_retention_secs: 30 * 24 * 60 * 60,
        }
    }
}

pub struct ConfigLoad {
    pub config: AppConfig,
    pub warning: Option<String>,
}

pub struct ConfigService;

impl ConfigService {
    pub fn load_toml(input: &str) -> ConfigLoad {
        match toml::from_str::<AppConfig>(input) {
            Ok(mut config) => {
                let mut warnings = Vec::new();
                if normalize_path(&config.scan_root).is_err() {
                    warnings.push("scan_root was invalid and reset to the safe default".to_owned());
                    config.scan_root = AppConfig::default().scan_root;
                } else if let Ok(path) = normalize_path(&config.scan_root) {
                    config.scan_root = path.display().to_owned();
                }

                config.excluded_paths = config
                    .excluded_paths
                    .into_iter()
                    .filter_map(|excluded| match normalize_path(&excluded) {
                        Ok(path) => Some(path.display().to_owned()),
                        Err(_) => {
                            warnings
                                .push(format!("excluded path was invalid and removed: {excluded}"));
                            None
                        }
                    })
                    .collect();

                if is_forbidden_path(
                    &normalize_path(&config.scan_root).expect("validated scan root"),
                ) {
                    warnings.push("forbidden paths remain blocked by cleanup policy".to_owned());
                }

                ConfigLoad {
                    config,
                    warning: (!warnings.is_empty()).then(|| warnings.join("; ")),
                }
            }
            Err(error) => ConfigLoad {
                config: AppConfig::default(),
                warning: Some(format!("configuration reset to safe defaults: {error}")),
            },
        }
    }

    pub fn serialize(config: &AppConfig) -> Result<String, String> {
        toml::to_string_pretty(config).map_err(|error| error.to_string())
    }
}
