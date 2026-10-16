use crate::powershell::{ReadOnlyQuery, run_read_only};
use std::{path::Path, process::Command};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MaintenancePreflight {
    pub update_busy: bool,
    pub system_restore_available: bool,
    pub assistant_available: bool,
}

impl MaintenancePreflight {
    pub fn allows_high_risk(&self) -> Result<(), String> {
        if self.update_busy {
            return Err("Windows Update is busy".into());
        }
        if !self.system_restore_available {
            return Err("system restore is unavailable".into());
        }
        if !self.assistant_available {
            return Err("administrator helper is unavailable".into());
        }
        Ok(())
    }
}

pub fn evaluate_preflight(
    update_status: &str,
    system_restore_status: &str,
    assistant_available: bool,
) -> MaintenancePreflight {
    let update_busy = update_status.to_ascii_lowercase().contains("running");
    let restore = system_restore_status.to_ascii_lowercase();
    MaintenancePreflight {
        update_busy,
        system_restore_available: !restore.trim().is_empty()
            && !restore.contains("error")
            && !restore.contains("unavailable"),
        assistant_available,
    }
}

pub fn query_read_only_status() -> Result<(String, String, String), String> {
    Ok((
        run_read_only(ReadOnlyQuery::WindowsUpdateStatus)?,
        run_read_only(ReadOnlyQuery::ServiceStatus)?,
        run_read_only(ReadOnlyQuery::SystemRestoreStatus)?,
    ))
}

pub fn register_read_only_scan_task(executable: &Path, task_name: &str) -> Result<(), String> {
    if !executable.is_file()
        || task_name.is_empty()
        || task_name
            .chars()
            .any(|character| matches!(character, '\\' | '/' | '"'))
    {
        return Err("invalid read-only scan task registration".into());
    }
    let trigger = read_only_scan_trigger(executable);
    let status = Command::new("schtasks.exe")
        .args([
            "/Create", "/F", "/SC", "DAILY", "/TN", task_name, "/TR", &trigger,
        ])
        .status()
        .map_err(|error| error.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!(
            "schtasks.exe exited with {}",
            status.code().unwrap_or(-1)
        ))
    }
}

pub fn read_only_scan_trigger(executable: &Path) -> String {
    format!(
        "\"{}\" scan --root C:\\ --max-entries 1000000",
        executable.display()
    )
}

#[cfg(test)]
mod tests {
    use super::{MaintenancePreflight, evaluate_preflight, read_only_scan_trigger};
    use std::path::Path;

    #[test]
    fn high_risk_actions_fail_closed_when_preflight_is_not_ready() {
        assert!(
            MaintenancePreflight {
                update_busy: true,
                system_restore_available: true,
                assistant_available: true
            }
            .allows_high_risk()
            .is_err()
        );
        assert!(
            MaintenancePreflight {
                update_busy: false,
                system_restore_available: false,
                assistant_available: true
            }
            .allows_high_risk()
            .is_err()
        );
        assert!(
            MaintenancePreflight {
                update_busy: false,
                system_restore_available: true,
                assistant_available: true
            }
            .allows_high_risk()
            .is_ok()
        );
    }

    #[test]
    fn preflight_maps_read_only_status_to_fail_closed_flags() {
        let preflight = evaluate_preflight("Status Running", "CreationTime", true);
        assert!(preflight.update_busy);
        assert!(preflight.system_restore_available);
        assert!(
            evaluate_preflight("Stopped", "unavailable", true)
                .allows_high_risk()
                .is_err()
        );
    }

    #[test]
    fn scheduled_command_is_read_only_and_bounded() {
        let trigger = read_only_scan_trigger(Path::new(r"C:\Program Files\Rusten\cleaner-cli.exe"));
        assert!(trigger.contains(" scan --root C:\\ --max-entries 1000000"));
        assert!(!trigger.contains(" apply-policy"));
        assert!(!trigger.contains(" --yes"));
    }
}
