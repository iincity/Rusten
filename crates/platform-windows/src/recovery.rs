use serde::Deserialize;

use crate::powershell::{ReadOnlyQuery, hidden_command, run_read_only};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryQuery {
    SystemRestorePoints,
    VssShadowCopies,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct SystemRestorePoint {
    #[serde(rename = "CreationTime")]
    pub creation_time: String,
    #[serde(rename = "Description")]
    pub description: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VssShadowCopyStatus {
    pub available: bool,
    pub details: String,
}

pub fn fixed_query(query: RecoveryQuery) -> &'static str {
    match query {
        RecoveryQuery::SystemRestorePoints => "Get-ComputerRestorePoint",
        RecoveryQuery::VssShadowCopies => "vssadmin.exe list shadows",
    }
}

pub fn query_is_read_only(query: RecoveryQuery) -> bool {
    matches!(
        query,
        RecoveryQuery::SystemRestorePoints | RecoveryQuery::VssShadowCopies
    )
}

pub fn query_system_restore_points() -> Result<Vec<SystemRestorePoint>, String> {
    let output = run_read_only(ReadOnlyQuery::SystemRestoreStatus)?;
    parse_system_restore_points(&output)
}

pub fn query_vss_shadow_copies() -> Result<VssShadowCopyStatus, String> {
    #[cfg(windows)]
    {
        let output = hidden_command("vssadmin.exe")
            .args(["list", "shadows"])
            .output()
            .map_err(|error| error.to_string())?;
        Ok(VssShadowCopyStatus {
            available: output.status.success(),
            details: String::from_utf8_lossy(&output.stdout).into_owned(),
        })
    }
    #[cfg(not(windows))]
    {
        Err("VSS inspection requires Windows".into())
    }
}

pub fn query_c_drive_shadow_storage_bytes() -> Result<u64, String> {
    #[cfg(windows)]
    {
        #[derive(Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct ShadowStorage {
            volume: String,
            used_space: u64,
        }

        #[derive(Deserialize)]
        #[serde(rename_all = "PascalCase")]
        struct Volume {
            device_id: String,
        }

        let connection = wmi::WMIConnection::new().map_err(|error| error.to_string())?;
        let volumes: Vec<Volume> = connection
            .raw_query("SELECT DeviceID FROM Win32_Volume WHERE DriveLetter='C:'")
            .map_err(|error| error.to_string())?;
        let Some(device_id) = volumes.into_iter().next().map(|volume| volume.device_id) else {
            return Ok(0);
        };
        let storages: Vec<ShadowStorage> = connection
            .raw_query("SELECT Volume, UsedSpace FROM Win32_ShadowStorage")
            .map_err(|error| error.to_string())?;
        let mut used_bytes = 0u64;
        for storage in storages {
            let volume: Volume = connection
                .get_by_path(&storage.volume)
                .map_err(|error| error.to_string())?;
            if volume.device_id.eq_ignore_ascii_case(&device_id) {
                used_bytes = used_bytes.saturating_add(storage.used_space);
            }
        }
        Ok(used_bytes)
    }
    #[cfg(not(windows))]
    {
        Err("shadow storage inspection requires Windows".into())
    }
}

pub fn parse_system_restore_points(output: &str) -> Result<Vec<SystemRestorePoint>, String> {
    let value: serde_json::Value =
        serde_json::from_str(output).map_err(|error| error.to_string())?;
    if value.is_null() {
        return Ok(Vec::new());
    }
    if let Some(point) = value.as_object() {
        return serde_json::from_value(serde_json::Value::Object(point.clone()))
            .map(|point| vec![point])
            .map_err(|error| error.to_string());
    }
    serde_json::from_value(value).map_err(|error| error.to_string())
}
