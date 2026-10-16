use std::process::{Command, Output};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ApplicationSource {
    Arp,
    Msi,
    Winget,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplicationRecord {
    pub display_name: String,
    pub version: Option<String>,
    pub uninstall_command: Option<String>,
    pub source: ApplicationSource,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UninstallError {
    ConfirmationRequired,
    MissingCommand,
    Failed(String),
}

pub fn inventory_command() -> (&'static str, &'static [&'static str]) {
    (
        "reg.exe",
        &[
            "query",
            r"HKLM\Software\Microsoft\Windows\CurrentVersion\Uninstall",
            "/s",
        ],
    )
}

pub fn query_arp() -> Result<String, String> {
    let (program, args) = inventory_command();
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into())
}

pub fn inventory_roots() -> [&'static str; 3] {
    [
        r"HKLM\Software\Microsoft\Windows\CurrentVersion\Uninstall",
        r"HKLM\Software\Wow6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
        r"HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall",
    ]
}

pub fn query_arp_all() -> Result<Vec<ApplicationRecord>, String> {
    let mut records = Vec::new();
    for root in inventory_roots() {
        let output = Command::new("reg.exe")
            .args(["query", root, "/s"])
            .output()
            .map_err(|error| error.to_string())?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).into());
        }
        records.extend(parse_arp(&String::from_utf8_lossy(&output.stdout)));
    }
    Ok(records)
}

pub fn winget_inventory_command() -> (&'static str, &'static [&'static str]) {
    (
        "winget.exe",
        &[
            "list",
            "--disable-interactivity",
            "--accept-source-agreements",
        ],
    )
}

pub fn query_winget() -> Result<String, String> {
    let (program, args) = winget_inventory_command();
    let output = Command::new(program)
        .args(args)
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into())
}

pub fn query_application_inventory() -> Result<Vec<ApplicationRecord>, String> {
    let mut records = query_arp_all()?;
    if let Ok(output) = query_winget() {
        records.extend(parse_winget(&output));
    }
    Ok(records)
}

pub fn parse_arp(input: &str) -> Vec<ApplicationRecord> {
    let mut records = Vec::new();
    let mut current: Option<ApplicationRecord> = None;
    for line in input.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("HKEY_") {
            if let Some(record) = current
                .take()
                .filter(|record| !record.display_name.is_empty())
            {
                records.push(record);
            }
            current = Some(ApplicationRecord {
                display_name: String::new(),
                version: None,
                uninstall_command: None,
                source: ApplicationSource::Arp,
            });
            continue;
        }
        let Some(record) = current.as_mut() else {
            continue;
        };
        if let Some((name, value)) = trimmed.split_once("REG_DWORD")
            && name.trim() == "WindowsInstaller"
            && value.trim().eq_ignore_ascii_case("0x1")
        {
            record.source = ApplicationSource::Msi;
            continue;
        }
        let Some((name, value)) = trimmed.split_once("REG_SZ") else {
            continue;
        };
        let value = value.trim().to_owned();
        match name.trim() {
            "DisplayName" => record.display_name = value,
            "DisplayVersion" => record.version = Some(value),
            "UninstallString" => record.uninstall_command = Some(value),
            _ => {}
        }
    }
    if let Some(record) = current.filter(|record| !record.display_name.is_empty()) {
        records.push(record);
    }
    records
}

pub fn parse_winget(input: &str) -> Vec<ApplicationRecord> {
    input
        .lines()
        .skip_while(|line| !line.contains("Name") || !line.contains("Version"))
        .skip(1)
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty()
                || line
                    .chars()
                    .filter(|character| !character.is_whitespace())
                    .all(|character| character == '-')
            {
                return None;
            }
            let (name, fields) = line.split_once("  ")?;
            let version = fields.split_whitespace().next().map(str::to_owned);
            Some(ApplicationRecord {
                display_name: name.trim().to_owned(),
                version,
                uninstall_command: None,
                source: ApplicationSource::Winget,
            })
        })
        .collect()
}

pub fn uninstall_one(
    record: &ApplicationRecord,
    confirmed: bool,
) -> Result<Output, UninstallError> {
    if !confirmed {
        return Err(UninstallError::ConfirmationRequired);
    }
    let command = record
        .uninstall_command
        .as_deref()
        .filter(|command| !command.trim().is_empty())
        .ok_or(UninstallError::MissingCommand)?;
    Command::new("cmd.exe")
        .args(["/D", "/S", "/C", command])
        .output()
        .map_err(|error| UninstallError::Failed(error.to_string()))
}

pub fn uninstall_batch(
    records: &[ApplicationRecord],
    confirmed: bool,
) -> Vec<Result<Output, UninstallError>> {
    let mut results = Vec::new();
    for record in records {
        let result = uninstall_one(record, confirmed);
        let failed = result.is_err()
            || result
                .as_ref()
                .is_ok_and(|output| !output.status.success() || uninstall_requires_reboot(output));
        results.push(result);
        if failed {
            break;
        }
    }
    results
}

pub fn uninstall_requires_reboot(output: &Output) -> bool {
    matches!(output.status.code(), Some(3010 | 1641))
        || String::from_utf8_lossy(&output.stdout)
            .to_ascii_lowercase()
            .contains("restart")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inventory_avoids_repair_triggering_provider() {
        let (program, args) = inventory_command();
        assert_eq!(program, "reg.exe");
        assert!(!args.iter().any(|arg| arg.contains("Win32_Product")));
        let (program, args) = winget_inventory_command();
        assert_eq!(program, "winget.exe");
        assert!(args.contains(&"--disable-interactivity"));
    }

    #[test]
    fn parses_arp_records_and_requires_confirmation_before_uninstall() {
        let records = parse_arp(
            "HKEY_LOCAL_MACHINE\\Software\\Test\n    DisplayName    REG_SZ    Demo\n    DisplayVersion REG_SZ    1.2\n    UninstallString REG_SZ    demo-uninstall.exe /quiet\n",
        );
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].display_name, "Demo");
        assert_eq!(records[0].source, ApplicationSource::Arp);
        assert_eq!(records[0].version.as_deref(), Some("1.2"));
        assert_eq!(
            uninstall_one(&records[0], false),
            Err(UninstallError::ConfirmationRequired)
        );
    }

    #[test]
    fn parses_winget_rows_without_treating_display_values_as_commands() {
        let records = parse_winget(
            "Name                 Version\n-------------------  -------\nDemo                 1.2\n",
        );
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].display_name, "Demo");
        assert_eq!(records[0].version.as_deref(), Some("1.2"));
        assert!(records[0].uninstall_command.is_none());
    }
}
