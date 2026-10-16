use domain::{
    CleanupCandidate, CleanupMode, Confidence, DomainError, RecoveryMode, RiskLevel,
    can_enter_quick_clean, is_explicit_cleanup_allowed, is_forbidden_path, normalize_path,
};

fn safe_candidate(path: &str) -> CleanupCandidate {
    CleanupCandidate {
        path: normalize_path(path).unwrap(),
        file_identity: None,
        rule_id: None,
        category: None,
        reason: None,
        estimated_size: None,
        quick_clean_eligible: false,
        quick_clean_block_reason: None,
        mode: Some(CleanupMode::Quick),
        risk: Some(RiskLevel::P3),
        confidence: Some(Confidence::High),
        recovery: Some(RecoveryMode::Recycle),
        locked: Some(false),
        path_allowed: Some(true),
        file_identity_unchanged: Some(true),
        last_write_time: None,
    }
}

#[test]
fn normalizes_local_windows_paths_without_touching_the_filesystem() {
    let path = normalize_path("c:/Users/Alice/AppData/Local/Temp/./cache.tmp").unwrap();

    assert_eq!(
        path.display(),
        r"C:\Users\Alice\AppData\Local\Temp\cache.tmp"
    );
    assert_eq!(
        path.extended(),
        r"\\?\C:\Users\Alice\AppData\Local\Temp\cache.tmp"
    );
}

#[test]
fn supports_paths_longer_than_max_path() {
    let input = format!(r"C:\Temp\{}\cache.bin", "segment".repeat(40));

    let path = normalize_path(&input).unwrap();

    assert!(path.display().len() > 260);
    assert_eq!(path.extended(), format!(r"\\?\{}", path.display()));
}

#[test]
fn rejects_unsafe_windows_path_forms() {
    let cases = [
        ("", DomainError::EmptyPath),
        (r"server\share\file.tmp", DomainError::UnsupportedVolume),
        (r"\\server\share\file.tmp", DomainError::UnsupportedVolume),
        (r"\\.\PhysicalDrive0", DomainError::DevicePath),
        (
            r"\\?\GLOBALROOT\Device\HarddiskVolume1",
            DomainError::DevicePath,
        ),
        (r"C:\Temp\CON.txt", DomainError::ReservedName),
        (r"C:\Temp\cache. ", DomainError::InvalidPath),
        (r"C:\Temp\cache.", DomainError::InvalidPath),
        (r"C:\..\Windows", DomainError::InvalidPath),
    ];

    for (input, expected) in cases {
        assert_eq!(normalize_path(input), Err(expected), "input: {input:?}");
    }
}

#[test]
fn rejects_superscript_reserved_device_names() {
    for name in ["COM¹", "COM²", "COM³", "LPT¹", "LPT²", "LPT³"] {
        let input = format!(r"C:\Temp\{name}.txt");
        assert_eq!(
            normalize_path(&input),
            Err(DomainError::ReservedName),
            "input: {input}"
        );
    }
}

#[test]
fn rejects_invalid_windows_component_characters() {
    for input in [
        r"C:\Temp\bad<name.tmp",
        r"C:\Temp\bad>name.tmp",
        r#"C:\Temp\bad"name.tmp"#,
        r"C:\Temp\bad|name.tmp",
        r"C:\Temp\bad?name.tmp",
        r"C:\Temp\bad*name.tmp",
    ] {
        assert_eq!(
            normalize_path(input),
            Err(DomainError::InvalidPath),
            "input: {input}"
        );
    }
}

#[test]
fn forbidden_paths_cover_system_browser_and_user_data() {
    let paths = [
        r"C:\Windows\System32\config\SAM",
        r"C:\Windows\WinSxS\manifest.xml",
        r"C:\Windows\SoftwareDistribution\DataStore\DataStore.edb",
        r"C:\Program Files\Example\app.exe",
        r"C:\Users\Alice\Documents\budget.xlsx",
        r"C:\Users\Alice\Downloads\archive.zip",
        r"C:\Users\Alice\AppData\Local\Google\Chrome\User Data\Default\Login Data",
        r"C:\Users\Alice\AppData\Roaming\Mozilla\Firefox\Profiles\abc\key4.db",
    ];

    for input in paths {
        assert!(
            is_forbidden_path(&normalize_path(input).unwrap()),
            "path should be forbidden: {input}"
        );
    }

    assert!(!is_forbidden_path(
        &normalize_path(r"C:\Windows\Temp\old.tmp").unwrap()
    ));
    assert!(!is_forbidden_path(
        &normalize_path(r"C:\Users\Alice\AppData\Local\Temp\old.tmp").unwrap()
    ));
    assert!(!is_forbidden_path(
        &normalize_path(r"C:\ProgramData\Microsoft\Windows\WER\ReportArchive\report.wer").unwrap()
    ));
    assert!(is_forbidden_path(
        &normalize_path(r"C:\ProgramData\Example\state.dat").unwrap()
    ));
}

#[test]
fn allows_rebuildable_windows_logs_dumps_and_caches() {
    for input in [
        r"C:\Windows\Logs\CBS\CBS.log",
        r"C:\Windows\System32\LogFiles\WMI\trace.etl",
        r"C:\Windows\Panther\setupact.log",
        r"C:\Windows\Minidump\mini.dmp",
        r"C:\Windows\LiveKernelReports\WATCHDOG\dump.dmp",
        r"C:\Windows\MEMORY.DMP",
        r"C:\Windows\SoftwareDistribution\Download\update.cab",
        r"C:\ProgramData\Microsoft\Windows\DeliveryOptimization\Cache\chunk.bin",
        r"C:\Users\Alice\AppData\Local\Microsoft\Windows\INetCache\cache.bin",
        r"C:\Users\Alice\AppData\Local\Microsoft\Windows\Explorer\thumbcache_96.db",
        r"C:\Windows\Prefetch\APP.EXE-1234.pf",
    ] {
        assert!(
            !is_forbidden_path(&normalize_path(input).unwrap()),
            "quick cleanup blocked: {input}"
        );
        assert!(
            is_explicit_cleanup_allowed(&normalize_path(input).unwrap()),
            "rebuildable path should be allowed: {input}"
        );
    }
}

#[test]
fn keeps_system_databases_and_unrelated_windows_files_protected() {
    for input in [
        r"C:\Windows\SoftwareDistribution\DataStore\DataStore.edb",
        r"C:\Windows\System32\config\SAM",
        r"C:\Windows\System32\kernel32.dll",
        r"C:\ProgramData\Microsoft\Windows\AppRepository\StateRepository-Machine.srd",
        r"C:\Users\Alice\AppData\Local\Microsoft\Windows\Explorer\state.db",
        r"C:\Windows\Logs\service.dll",
        r"C:\Windows\Minidump\driver.sys",
        r"C:\Windows\SoftwareDistribution\Downloads\update.cab",
        r"C:\ProgramData\Microsoft\Windows\DeliveryOptimization\State\state.dat",
        r"C:\Users\Alice\AppData\Local\Microsoft\Windows\ExplorerOther\thumbcache_96.db",
    ] {
        assert!(
            is_forbidden_path(&normalize_path(input).unwrap()),
            "protected path should remain forbidden: {input}"
        );
    }
}

#[test]
fn wechat_policy_only_allows_rebuildable_cache_subtrees() {
    assert!(!is_forbidden_path(
        &normalize_path(
            r"C:\Users\Alice\Documents\WeChat Files\wxid_demo\FileStorage\Image\2026\image.jpg"
        )
        .unwrap()
    ));
    assert!(is_forbidden_path(
        &normalize_path(r"C:\Users\Alice\Documents\WeChat Files\wxid_demo\Msg\message.db").unwrap()
    ));
}

#[test]
fn explicit_large_file_cleanup_allows_user_data_but_keeps_hard_blocks() {
    assert!(is_forbidden_path(
        &normalize_path(r"C:\Users\Alice\Downloads\archive.zip").unwrap()
    ));
    assert!(is_explicit_cleanup_allowed(
        &normalize_path(r"C:\Users\Alice\Downloads\archive.zip").unwrap()
    ));
    assert!(!is_explicit_cleanup_allowed(
        &normalize_path(r"C:\Users\Alice\AppData\Local\Google\Chrome\User Data\Default\Login Data")
            .unwrap()
    ));
    assert!(!is_explicit_cleanup_allowed(
        &normalize_path(r"C:\Windows\System32\config\SAM").unwrap()
    ));
}

#[test]
fn quick_clean_blocks_windows_except_temp_children() {
    for input in [
        r"C:\Windows\explorer.exe",
        r"C:\Windows\SystemApps\Microsoft.Windows.Example\app.exe",
        r"C:\Windows\Temp",
    ] {
        let candidate = safe_candidate(input);
        assert!(is_forbidden_path(&candidate.path), "path: {input}");
        assert!(!can_enter_quick_clean(&candidate), "path: {input}");
    }

    let temp_file = safe_candidate(r"C:\Windows\Temp\old.tmp");
    assert!(!is_forbidden_path(&temp_file.path));
    assert!(can_enter_quick_clean(&temp_file));
}

#[test]
fn quick_clean_requires_every_safety_gate() {
    assert!(can_enter_quick_clean(&safe_candidate(
        r"C:\Users\Alice\AppData\Local\Temp\old.tmp"
    )));

    let mut candidate = safe_candidate(r"C:\Users\Alice\AppData\Local\Temp\old.tmp");
    candidate.recovery = Some(RecoveryMode::BackupThenDelete);
    assert!(can_enter_quick_clean(&candidate));

    let mut rejected = Vec::new();

    let mut candidate = safe_candidate(r"C:\Users\Alice\AppData\Local\Temp\old.tmp");
    candidate.mode = Some(CleanupMode::Manual);
    rejected.push(candidate);

    let mut candidate = safe_candidate(r"C:\Users\Alice\AppData\Local\Temp\old.tmp");
    candidate.risk = Some(RiskLevel::P2);
    rejected.push(candidate);

    let mut candidate = safe_candidate(r"C:\Users\Alice\AppData\Local\Temp\old.tmp");
    candidate.confidence = Some(Confidence::Low);
    rejected.push(candidate);

    let mut candidate = safe_candidate(r"C:\Users\Alice\AppData\Local\Temp\old.tmp");
    candidate.recovery = Some(RecoveryMode::DeleteOnly);
    rejected.push(candidate);

    let mut candidate = safe_candidate(r"C:\Users\Alice\AppData\Local\Temp\old.tmp");
    candidate.locked = Some(true);
    rejected.push(candidate);

    let mut candidate = safe_candidate(r"C:\Users\Alice\AppData\Local\Temp\old.tmp");
    candidate.path_allowed = Some(false);
    rejected.push(candidate);

    let mut candidate = safe_candidate(r"C:\Users\Alice\AppData\Local\Temp\old.tmp");
    candidate.file_identity_unchanged = Some(false);
    rejected.push(candidate);

    assert!(rejected.iter().all(|item| !can_enter_quick_clean(item)));
}

#[test]
fn quick_clean_directly_rejects_p0_and_p1_risk() {
    for risk in [RiskLevel::P0, RiskLevel::P1] {
        let mut candidate = safe_candidate(r"C:\Users\Alice\AppData\Local\Temp\old.tmp");
        candidate.risk = Some(risk);
        assert!(!can_enter_quick_clean(&candidate), "risk: {risk:?}");
    }
}

#[test]
fn quick_clean_fails_closed_for_unknown_state_and_hard_blocks_forbidden_paths() {
    let unknowns = [
        |candidate: &mut CleanupCandidate| candidate.mode = None,
        |candidate: &mut CleanupCandidate| candidate.risk = None,
        |candidate: &mut CleanupCandidate| candidate.confidence = None,
        |candidate: &mut CleanupCandidate| candidate.recovery = None,
        |candidate: &mut CleanupCandidate| candidate.locked = None,
        |candidate: &mut CleanupCandidate| candidate.path_allowed = None,
        |candidate: &mut CleanupCandidate| candidate.file_identity_unchanged = None,
    ];

    for make_unknown in unknowns {
        let mut candidate = safe_candidate(r"C:\Users\Alice\AppData\Local\Temp\old.tmp");
        make_unknown(&mut candidate);
        assert!(!can_enter_quick_clean(&candidate));
    }

    let forbidden = safe_candidate(r"C:\Users\Alice\Documents\notes.txt");
    assert!(forbidden.path_allowed == Some(true));
    assert!(!can_enter_quick_clean(&forbidden));
}
