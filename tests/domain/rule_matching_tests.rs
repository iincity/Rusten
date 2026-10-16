use domain::{
    FileFacts, FileIdentity, FileRecord, RecoveryMode, ReparseKind, RiskLevel, Rule,
    RuleMatchContext, generate_candidates, normalize_path,
};

fn file(path: &str, size: u64) -> FileRecord {
    let index = domain::ScanIndex::new(normalize_path(r"C:\").unwrap());
    FileRecord::new(
        normalize_path(path).unwrap(),
        index.root_directory_id(),
        FileIdentity {
            volume_serial: 1,
            file_index: size as u128,
        },
        size,
        size,
    )
}

fn file_with_sizes(path: &str, logical_size: u64, allocated_size: u64) -> FileRecord {
    let index = domain::ScanIndex::new(normalize_path(r"C:\").unwrap());
    FileRecord::new(
        normalize_path(path).unwrap(),
        index.root_directory_id(),
        FileIdentity {
            volume_serial: 1,
            file_index: logical_size as u128,
        },
        logical_size,
        allocated_size,
    )
}

fn quick_rule(paths: &str) -> Rule {
    let paths = paths.replace('\\', "/");
    Rule::from_toml(&format!(
        r#"
id = "test.cache"
name = "Test cache"
category = "cache"
mode = "quick"
risk = "P3"
confidence = "high"
recovery = "recycle"
requires_admin = false
warning = "Rebuildable cache"
version = "1"

[match]
paths = ["{paths}"]
"#
    ))
    .unwrap()
}

#[test]
fn missing_required_schema_fields_are_structured() {
    let error = Rule::from_toml("name = \"missing id\"\n").unwrap_err();
    assert!(matches!(error, domain::RuleError::MissingField("id")));
}

#[test]
fn builtin_system_rules_create_candidates_for_logs_dumps_and_caches() {
    let rules = [
        Rule::from_toml(include_str!("../../rules/builtin/system_temp.toml")).unwrap(),
        Rule::from_toml(include_str!("../../rules/builtin/system_logs.toml")).unwrap(),
    ];
    let paths = [
        (r"C:\Windows\Logs\CBS\CBS.log", "system.logs"),
        (r"C:\Windows\System32\LogFiles\WMI\trace.etl", "system.logs"),
        (r"C:\Windows\Panther\setupact.log", "system.logs"),
        (r"C:\Windows\Minidump\mini.dmp", "system.debug"),
        (
            r"C:\Windows\LiveKernelReports\WATCHDOG\dump.dmp",
            "system.debug",
        ),
        (r"C:\Windows\MEMORY.DMP", "system.debug"),
        (
            r"C:\Users\Alice\AppData\Local\CrashDumps\app.dmp",
            "system.debug",
        ),
        (
            r"C:\Windows\SoftwareDistribution\Download\update.cab",
            "system.cache",
        ),
        (
            r"C:\ProgramData\Microsoft\Windows\DeliveryOptimization\Cache\chunk.bin",
            "system.cache",
        ),
        (
            r"C:\Users\Alice\AppData\Local\Microsoft\Windows\INetCache\cache.bin",
            "system.cache",
        ),
        (
            r"C:\Users\Alice\AppData\Local\Microsoft\Windows\Explorer\thumbcache_96.db",
            "system.cache",
        ),
        (
            r"C:\Users\Alice\AppData\Local\Microsoft\Windows\WebCache\WebCacheV01.dat",
            "system.cache",
        ),
        (
            r"C:\Users\Alice\AppData\Local\Packages\App\LocalCache\cache.bin",
            "system.store",
        ),
        (
            r"C:\Users\Alice\AppData\Roaming\Microsoft\Windows\Recent\item.lnk",
            "system.history",
        ),
        (
            r"C:\Windows\Prefetch\APP.EXE-1234.pf",
            "system.legacy_cache",
        ),
    ];
    for (path, category) in paths {
        let candidates =
            generate_candidates(&rules, &[file(path, 4096)], &RuleMatchContext::default());
        assert_eq!(
            candidates.len(),
            1,
            "expected a cleanup candidate for {path}"
        );
        assert!(candidates[0].estimated_size == Some(4096));
        assert_eq!(candidates[0].category.as_deref(), Some(category));
    }
}

#[test]
fn exclusions_take_priority_over_matching_paths() {
    let rule = Rule::from_toml(
        r#"
id = "browser.cache"
name = "Browser cache"
category = "browser_cache"
mode = "quick"
risk = "P3"
confidence = "high"
recovery = "recycle"
requires_admin = false
warning = "Rebuildable"
version = "1"

[match]
paths = ["C:/Users/*/AppData/Local/Browser/User Data/*/Cache/*"]

[exclude]
paths = ["*/Login Data", "*/Cookies"]
"#,
    )
    .unwrap();

    assert!(rule.matches_path(
        &normalize_path(r"C:\Users\A\AppData\Local\Browser\User Data\Default\Cache\x").unwrap()
    ));
    assert!(!rule.matches_path(
        &normalize_path(r"C:\Users\A\AppData\Local\Browser\User Data\Default\Login Data").unwrap()
    ));
    assert!(!rule.matches_path(
        &normalize_path(r"C:\Users\A\AppData\Local\Browser\User Data\Default\Cookies").unwrap()
    ));
}

#[test]
fn environment_variables_are_expanded_from_supplied_normalized_paths_only() {
    let rule = quick_rule(r"%CACHE_ROOT%\*");
    let root = normalize_path(r"C:\Users\A\Cache").unwrap();
    let candidate_path = normalize_path(r"C:\Users\A\Cache\old.tmp").unwrap();

    assert!(rule.matches_path_with_variables(&candidate_path, &[("CACHE_ROOT", root)]));
}

#[test]
fn rule_path_prefilter_rejects_unrelated_roots_before_glob_matching() {
    let rule = quick_rule(r"C:\Users\*\AppData\Local\Temp\*");

    assert!(rule.may_match_path(&normalize_path(r"C:\Users\A\AppData\Local\Temp\x.tmp").unwrap()));
    assert!(!rule.may_match_path(&normalize_path(r"C:\Program Files\app\app.exe").unwrap()));
}

#[test]
fn trailing_directory_wildcard_includes_nested_files() {
    let rule = quick_rule(r"C:\Windows\Temp\*");
    assert!(!domain::is_forbidden_path(
        &normalize_path(r"C:\Windows\Temp\direct.tmp").unwrap()
    ));

    assert!(rule.matches_path(&normalize_path(r"C:\Windows\Temp\direct.tmp").unwrap()));
    assert!(rule.matches_path(&normalize_path(r"C:\Windows\Temp\nested\deep.tmp").unwrap()));
}

#[test]
fn browser_sensitive_files_are_hard_excluded() {
    let rule = quick_rule(r"C:\Users\*\AppData\Local\Browser\User Data\*\Cache\*");
    let files = [
        file(
            r"C:\Users\A\AppData\Local\Browser\User Data\Default\Cache\x",
            10,
        ),
        file(
            r"C:\Users\A\AppData\Local\Browser\User Data\Default\Login Data",
            20,
        ),
        file(
            r"C:\Users\A\AppData\Local\Browser\User Data\Default\Cookies",
            30,
        ),
    ];

    let candidates = generate_candidates(&[rule], &files, &RuleMatchContext::default());
    assert_eq!(candidates.len(), 1);
    assert_eq!(
        candidates[0].path.display(),
        r"C:\Users\A\AppData\Local\Browser\User Data\Default\Cache\x"
    );
    assert_eq!(candidates[0].rule_id.as_deref(), Some("test.cache"));
    assert_eq!(candidates[0].estimated_size, Some(10));
}

#[test]
fn cleanup_candidate_keeps_logical_size_when_allocation_is_unknown() {
    let rule = quick_rule(r"C:\Users\*\AppData\Local\Browser\User Data\*\Cache\*");
    let candidates = generate_candidates(
        &[rule],
        &[file_with_sizes(
            r"C:\Users\A\AppData\Local\Browser\User Data\Default\Cache\x",
            4096,
            0,
        )],
        &RuleMatchContext::default(),
    );

    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].estimated_size, Some(4096));
}

#[test]
fn sensitive_database_and_recycle_bin_paths_are_hard_excluded() {
    let rule = Rule::from_toml(
        r#"
id = "sensitive"
name = "Sensitive"
category = "cache"
mode = "quick"
risk = "P3"
confidence = "high"
recovery = "recycle"
warning = "sensitive"
version = "1"

[match]
paths = [
  "C:/Users/*/AppData/Local/Browser/User Data/*/*",
  "C:/Users/*/AppData/Local/Mail/*",
  "C:/Users/*/AppData/Local/WeChat/*",
  "C:/Users/*/$Recycle.Bin/*/*",
]
"#,
    )
    .unwrap();
    let files = [
        file(
            r"C:\Users\A\AppData\Local\Browser\User Data\Default\main.db",
            10,
        ),
        file(r"C:\Users\A\AppData\Local\Mail\mailbox.pst", 20),
        file(r"C:\Users\A\AppData\Local\WeChat\messages.db", 30),
        file(r"C:\Users\A\$Recycle.Bin\S-1-5-21\$I1", 40),
    ];

    assert!(generate_candidates(&[rule], &files, &RuleMatchContext::default()).is_empty());
}

#[test]
fn candidate_contains_explanation_and_rule_metadata() {
    let candidate = generate_candidates(
        &[quick_rule(r"C:\Temp\*")],
        &[file(r"C:\Temp\old.tmp", 42)],
        &RuleMatchContext::default(),
    )
    .pop()
    .unwrap();

    assert_eq!(candidate.risk, Some(RiskLevel::P3));
    assert_eq!(candidate.recovery, Some(RecoveryMode::Recycle));
    assert_eq!(candidate.reason.as_deref(), Some("Rebuildable cache"));
    assert!(!candidate.quick_clean_eligible);
}

#[test]
fn variables_are_used_by_candidate_generation_entrypoint() {
    let rule = quick_rule(r"%CACHE_ROOT%\*");
    let root = normalize_path(r"C:\Users\A\Cache").unwrap();
    let files = [file(r"C:\Users\A\Cache\old.tmp", 10)];
    let context = RuleMatchContext::default().with_variable("CACHE_ROOT", root);

    assert_eq!(generate_candidates(&[rule], &files, &context).len(), 1);
}

#[test]
fn running_processes_and_admin_requirements_block_quick_clean_with_reasons() {
    let process_rule = Rule::from_toml(
        r#"
id = "process.cache"
name = "Process cache"
category = "cache"
mode = "quick"
risk = "P3"
confidence = "high"
recovery = "recycle"
processes = ["browser.exe"]
requires_admin = false
warning = "Cache cleanup"
version = "1"

[match]
paths = ["C:/Temp/*"]
"#,
    )
    .unwrap();
    let context = RuleMatchContext::default().with_running_processes(["browser.exe"]);
    let candidate = generate_candidates(&[process_rule], &[file(r"C:\Temp\old.tmp", 10)], &context)
        .pop()
        .unwrap();
    assert!(!candidate.quick_clean_eligible);
    assert!(candidate.reason.unwrap().contains("browser.exe"));

    let admin_rule = Rule::from_toml(
        r#"
id = "admin.cache"
name = "Admin cache"
category = "cache"
mode = "quick"
risk = "P3"
confidence = "high"
recovery = "recycle"
requires_admin = true
warning = "Admin cache cleanup"
version = "1"

[match]
paths = ["C:/Temp/*"]
"#,
    )
    .unwrap();
    let context = RuleMatchContext::default().with_admin_available(true);
    let candidate = generate_candidates(&[admin_rule], &[file(r"C:\Temp\old.tmp", 10)], &context)
        .pop()
        .unwrap();
    assert!(!candidate.quick_clean_eligible);
    assert!(
        !candidate
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("administrator"))
    );

    let candidate = candidate.with_execution_facts(false, true, true);
    assert!(candidate.quick_clean_eligible);
}

#[test]
fn unknown_process_or_admin_state_fails_closed() {
    let rule = Rule::from_toml(
        r#"
id = "unknown.state"
name = "Unknown state"
category = "cache"
mode = "quick"
risk = "P3"
confidence = "high"
recovery = "recycle"
processes = ["browser.exe"]
requires_admin = true
warning = "Cache cleanup"
version = "1"

[match]
paths = ["C:/Temp/*"]
"#,
    )
    .unwrap();
    let candidate = generate_candidates(
        &[rule],
        &[file(r"C:\Temp\old.tmp", 10)],
        &RuleMatchContext::default(),
    )
    .pop()
    .unwrap();
    assert!(!candidate.quick_clean_eligible);
    assert!(candidate.reason.unwrap().contains("unknown"));
}

#[test]
fn reparse_points_are_not_candidate_inputs() {
    let rule = quick_rule(r"C:\Temp\*");
    let reparse = file(r"C:\Temp\junction", 20).with_facts(FileFacts {
        reparse_kind: Some(ReparseKind::Junction),
        ..FileFacts::default()
    });
    let candidates = generate_candidates(
        &[rule],
        &[file(r"C:\Temp\normal.tmp", 10), reparse],
        &RuleMatchContext::default(),
    );
    assert_eq!(candidates.len(), 1);
    assert!(candidates[0].path.display().ends_with("normal.tmp"));
}

#[test]
fn wildcard_does_not_cross_directory_boundaries() {
    let rule = quick_rule(r"C:\Temp\*\cache\*");
    assert!(rule.matches_path(&normalize_path(r"C:\Temp\one\cache\a.tmp").unwrap()));
    assert!(!rule.matches_path(&normalize_path(r"C:\Temp\one\deep\cache\a.tmp").unwrap()));
    assert!(rule.matches_path(&normalize_path(r"C:\Temp\one\cache\nested\a.tmp").unwrap()));
}

#[test]
fn duplicate_matches_keep_highest_priority_rule() {
    let low = Rule::from_toml(&quick_rule_toml("low.rule", 1, "low warning")).unwrap();
    let high = Rule::from_toml(&quick_rule_toml("high.rule", 10, "high warning")).unwrap();
    let candidates = generate_candidates(
        &[low, high],
        &[file(r"C:\Temp\same.tmp", 10)],
        &RuleMatchContext::default(),
    );
    assert_eq!(candidates.len(), 1);
    assert_eq!(candidates[0].rule_id.as_deref(), Some("high.rule"));
}

#[test]
fn identity_deduplication_keeps_distinct_case_sensitive_files() {
    let rule = quick_rule(r"C:\Temp\*");
    let first = file(r"C:\Temp\same.tmp", 10);
    let second = FileRecord::new(
        normalize_path(r"C:\Temp\SAME.tmp").unwrap(),
        first.parent(),
        FileIdentity {
            volume_serial: 1,
            file_index: 11,
        },
        10,
        10,
    );
    let candidates = generate_candidates(&[rule], &[first, second], &RuleMatchContext::default());
    assert_eq!(candidates.len(), 2);
}

#[test]
fn literal_rule_prefix_is_case_insensitive_and_component_bounded() {
    let rule = quick_rule(r"C:\Temp");
    assert!(rule.matches_path(&normalize_path(r"c:\temp\old.tmp").unwrap()));
    assert!(!rule.matches_path(&normalize_path(r"C:\Templates\old.tmp").unwrap()));
}

fn quick_rule_toml(id: &str, priority: u32, warning: &str) -> String {
    format!(
        r#"
id = "{id}"
name = "Priority rule"
category = "cache"
priority = {priority}
mode = "quick"
risk = "P3"
confidence = "high"
recovery = "recycle"
warning = "{warning}"
version = "1"

[match]
paths = ["C:/Temp/*"]
"#
    )
}

#[test]
fn rule_metadata_and_application_name_come_from_configuration_and_path() {
    let rule = Rule::from_toml(
        r#"
id = "application_cache"
name = "Application cache"
category = "application_cache"
group = "常用软件专清"
display_name = "应用缓存"
display_order = 30
item_from_path_after = "AppData/Local"
mode = "quick"
risk = "P3"
confidence = "high"
recovery = "recycle"
warning = "Rebuildable"
version = "1"
[match]
paths = ["C:/Users/*/AppData/Local/*/Cache/*"]
"#,
    )
    .unwrap();
    let candidate = rule
        .candidate_for(&file(
            r"C:\Users\A\AppData\Local\ExampleTool\Cache\old.bin",
            42,
        ))
        .unwrap();

    assert_eq!(rule.group(), "常用软件专清");
    assert_eq!(rule.display_order(), 30);
    assert_eq!(
        candidate.category.as_deref(),
        Some("application_cache/ExampleTool")
    );
    assert_eq!(rule.display_name_for_path(&candidate.path), "ExampleTool");
}

#[test]
fn configured_display_items_provide_zero_size_entries_without_hardcoding_ui() {
    let rule = Rule::from_toml(
        r#"
id = "system_cleanup"
name = "System cleanup"
category = "system_cleanup"
group = "系统专清"
display_name = "临时文件"
display_order = 10
mode = "quick"
risk = "P3"
confidence = "high"
recovery = "recycle"
warning = "Rebuildable"
version = "1"

[[display_items]]
key = "temp"
name = "临时文件"
order = 10
paths = ["C:/Windows/Temp/*"]

[[display_items]]
key = "old_system_cache"
name = "旧版系统缓存"
order = 20
"#,
    )
    .unwrap();

    assert_eq!(rule.display_items().len(), 2);
    assert_eq!(rule.display_items()[1].name, "旧版系统缓存");
    assert_eq!(
        rule.display_item_for_path(&normalize_path(r"C:\Windows\Temp\a.tmp").unwrap())
            .unwrap()
            .key,
        "temp"
    );
}
