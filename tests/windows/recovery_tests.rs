use platform_windows::recovery::{
    RecoveryQuery, fixed_query, parse_system_restore_points, query_is_read_only,
};

#[cfg(windows)]
#[test]
#[ignore = "requires the local WMI service"]
fn c_drive_shadow_storage_query_uses_wmi() {
    let bytes = platform_windows::recovery::query_c_drive_shadow_storage_bytes().unwrap();
    println!("C: shadow storage used bytes: {bytes}");
}

#[test]
fn advanced_recovery_exposes_only_read_only_queries() {
    assert!(query_is_read_only(RecoveryQuery::SystemRestorePoints));
    assert!(query_is_read_only(RecoveryQuery::VssShadowCopies));
    assert_eq!(
        fixed_query(RecoveryQuery::SystemRestorePoints),
        "Get-ComputerRestorePoint"
    );
    assert_eq!(
        fixed_query(RecoveryQuery::VssShadowCopies),
        "vssadmin.exe list shadows"
    );
}

#[test]
fn system_restore_point_parser_handles_empty_and_structured_rows() {
    assert!(parse_system_restore_points("[]").unwrap().is_empty());
    let points = parse_system_restore_points(
        r#"[{"CreationTime":"2026-09-25T01:02:03Z","Description":"Before update"}]"#,
    )
    .unwrap();
    assert_eq!(points.len(), 1);
    assert_eq!(points[0].description, "Before update");
}
