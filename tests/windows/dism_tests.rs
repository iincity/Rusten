use platform_windows::dism::{DismAction, audit_command, fixed_args};

#[test]
fn dism_uses_only_official_fixed_actions() {
    assert_eq!(
        fixed_args(DismAction::AnalyzeComponentStore),
        &["/Online", "/Cleanup-Image", "/AnalyzeComponentStore"]
    );
    assert_eq!(
        fixed_args(DismAction::StartComponentCleanup),
        &["/Online", "/Cleanup-Image", "/StartComponentCleanup"]
    );
}

#[test]
fn dism_audit_command_contains_fixed_action_and_log_path() {
    let command = audit_command(
        DismAction::StartComponentCleanup,
        r"C:\ProgramData\Rusten\dism.log",
    );
    assert_eq!(
        command,
        r#"dism.exe /Online /Cleanup-Image /StartComponentCleanup /LogPath:C:\ProgramData\Rusten\dism.log"#
    );
}
