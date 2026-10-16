use std::{fs, io::ErrorKind, path::Path};

use domain::{AdminRejectReason, AdminResult};

const HIBERFILE: &str = r"C:\hiberfil.sys";
const HIBERNATION_ARGS: &[&str] = &["/hibernate", "off"];
const PAGING_FILES_SCRIPT: &str = "$computer = Get-CimInstance -ClassName Win32_ComputerSystem -ErrorAction Stop; Set-CimInstance -InputObject $computer -Property @{AutomaticManagedPagefile=$false} -ErrorAction Stop | Out-Null; Get-CimInstance -ClassName Win32_PageFileSetting -ErrorAction Stop | Remove-CimInstance -ErrorAction Stop; $computer = Get-CimInstance -ClassName Win32_ComputerSystem -ErrorAction Stop; $remaining = @(Get-CimInstance -ClassName Win32_PageFileSetting -ErrorAction Stop); if ($computer.AutomaticManagedPagefile -or $remaining.Count -ne 0) { exit 1 }";
const PAGING_FILES_ARGS: &[&str] = &[
    "-NoProfile",
    "-NonInteractive",
    "-ExecutionPolicy",
    "Bypass",
    "-Command",
    PAGING_FILES_SCRIPT,
];

fn hibernation_command_spec() -> (&'static str, &'static [&'static str]) {
    ("powercfg.exe", HIBERNATION_ARGS)
}

fn paging_files_command_spec() -> (&'static str, &'static [&'static str]) {
    ("powershell.exe", PAGING_FILES_ARGS)
}

pub fn disable_hibernation() -> AdminResult {
    let released_bytes = match fs::metadata(HIBERFILE) {
        Ok(metadata) => metadata.len(),
        Err(error) if error.kind() == ErrorKind::NotFound => 0,
        Err(_) => return operation_failed(),
    };
    let (program, args) = hibernation_command_spec();
    let Ok(status) = platform_windows::powershell::hidden_command(program)
        .args(args)
        .status()
    else {
        return operation_failed();
    };
    if !status.success() {
        return operation_failed();
    }
    match fs::symlink_metadata(Path::new(HIBERFILE)) {
        Err(error) if error.kind() == ErrorKind::NotFound => {
            AdminResult::PermanentlyDeleted { released_bytes }
        }
        _ => operation_failed(),
    }
}

pub fn disable_paging_files() -> AdminResult {
    let (program, args) = paging_files_command_spec();
    match platform_windows::powershell::hidden_command(program)
        .args(args)
        .status()
    {
        Ok(status) if status.success() => AdminResult::Completed { affected: 0 },
        _ => operation_failed(),
    }
}

fn operation_failed() -> AdminResult {
    AdminResult::Rejected {
        reason: AdminRejectReason::OperationFailed,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn hibernation_uses_only_the_fixed_powercfg_action() {
        assert_eq!(
            super::hibernation_command_spec(),
            ("powercfg.exe", &["/hibernate", "off"][..])
        );
    }

    #[test]
    fn paging_files_use_only_the_fixed_verified_cim_action() {
        assert_eq!(
            super::paging_files_command_spec(),
            (
                "powershell.exe",
                &[
                    "-NoProfile",
                    "-NonInteractive",
                    "-ExecutionPolicy",
                    "Bypass",
                    "-Command",
                    "$computer = Get-CimInstance -ClassName Win32_ComputerSystem -ErrorAction Stop; Set-CimInstance -InputObject $computer -Property @{AutomaticManagedPagefile=$false} -ErrorAction Stop | Out-Null; Get-CimInstance -ClassName Win32_PageFileSetting -ErrorAction Stop | Remove-CimInstance -ErrorAction Stop; $computer = Get-CimInstance -ClassName Win32_ComputerSystem -ErrorAction Stop; $remaining = @(Get-CimInstance -ClassName Win32_PageFileSetting -ErrorAction Stop); if ($computer.AutomaticManagedPagefile -or $remaining.Count -ne 0) { exit 1 }",
                ][..],
            )
        );
    }
}
