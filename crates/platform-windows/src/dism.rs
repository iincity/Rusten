use std::{path::PathBuf, process::Command};

use crate::services::MaintenancePreflight;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DismAction {
    AnalyzeComponentStore,
    StartComponentCleanup,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DismResult {
    pub action: DismAction,
    pub exit_code: Option<i32>,
    pub log_path: PathBuf,
    pub stdout: String,
    pub stderr: String,
}

pub fn fixed_args(action: DismAction) -> &'static [&'static str] {
    match action {
        DismAction::AnalyzeComponentStore => {
            &["/Online", "/Cleanup-Image", "/AnalyzeComponentStore"]
        }
        DismAction::StartComponentCleanup => {
            &["/Online", "/Cleanup-Image", "/StartComponentCleanup"]
        }
    }
}

pub fn audit_command(action: DismAction, log_path: &str) -> String {
    format!(
        "dism.exe {} /LogPath:{log_path}",
        fixed_args(action).join(" ")
    )
}

pub fn run_checked(
    action: DismAction,
    log_path: impl Into<PathBuf>,
    confirmed: bool,
    preflight: MaintenancePreflight,
) -> Result<DismResult, String> {
    if action == DismAction::StartComponentCleanup && !confirmed {
        return Err("DISM cleanup requires explicit confirmation".into());
    }
    if action == DismAction::StartComponentCleanup {
        preflight.allows_high_risk()?;
    }
    let log_path = log_path.into();
    let output = Command::new("dism.exe")
        .args(fixed_args(action))
        .arg(format!("/LogPath:{}", log_path.display()))
        .output()
        .map_err(|error| error.to_string())?;
    Ok(DismResult {
        action,
        exit_code: output.status.code(),
        log_path,
        stdout: String::from_utf8_lossy(&output.stdout).into(),
        stderr: String::from_utf8_lossy(&output.stderr).into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cleanup_is_fixed_and_confirmed() {
        assert_eq!(
            fixed_args(DismAction::StartComponentCleanup),
            &["/Online", "/Cleanup-Image", "/StartComponentCleanup"]
        );
        assert!(
            run_checked(
                DismAction::StartComponentCleanup,
                "dism.log",
                false,
                MaintenancePreflight {
                    update_busy: false,
                    system_restore_available: true,
                    assistant_available: true,
                }
            )
            .is_err()
        );
        assert!(
            run_checked(
                DismAction::StartComponentCleanup,
                "dism.log",
                true,
                MaintenancePreflight {
                    update_busy: true,
                    system_restore_available: true,
                    assistant_available: true,
                }
            )
            .is_err()
        );
    }
}
