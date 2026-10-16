use std::process::Command;

#[cfg(windows)]
use std::os::windows::process::CommandExt;

#[cfg(windows)]
use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReadOnlyQuery {
    WindowsUpdateStatus,
    ServiceStatus,
    SystemRestoreStatus,
}

pub fn script(query: ReadOnlyQuery) -> &'static str {
    match query {
        ReadOnlyQuery::WindowsUpdateStatus => {
            "Get-Service -Name wuauserv | Select-Object Status,Name"
        }
        ReadOnlyQuery::ServiceStatus => "Get-Service | Select-Object Status,Name",
        ReadOnlyQuery::SystemRestoreStatus => {
            "Get-ComputerRestorePoint | Select-Object CreationTime,Description | ConvertTo-Json -Compress"
        }
    }
}

pub fn run_read_only(query: ReadOnlyQuery) -> Result<String, String> {
    let output = hidden_command("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "RemoteSigned",
            "-Command",
            script(query),
        ])
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into())
}

pub fn hidden_command(program: &str) -> Command {
    let mut command = Command::new(program);
    #[cfg(windows)]
    command.creation_flags(CREATE_NO_WINDOW);
    command
}
