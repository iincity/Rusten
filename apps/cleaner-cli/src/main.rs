use std::{
    collections::{BTreeMap, HashSet},
    env, fs,
    path::PathBuf,
    sync::mpsc,
    thread,
};

use application::{ReportData, ReportFormat, RuleService, render_report};
use domain::{RuleMatchContext, ScanEvent, ScanIndex, ScanStatus, normalize_path};
use platform_windows::{FastFileScanner, FileScanner, ScanOptions};

const STATISTICS_SCAN_MAX_ENTRIES: u64 = 10_000_000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Command {
    Scan,
    Preview,
    Export,
    ApplyPolicy,
    Schedule,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OutputFormat {
    Json,
    Csv,
    Html,
}

impl OutputFormat {
    fn report_format(self) -> ReportFormat {
        match self {
            Self::Json => ReportFormat::Json,
            Self::Csv => ReportFormat::Csv,
            Self::Html => ReportFormat::Html,
        }
    }
}

#[derive(Debug)]
struct Options {
    command: Command,
    root: String,
    output: Option<PathBuf>,
    policy: Option<PathBuf>,
    task_name: Option<String>,
    max_entries: u64,
    yes: bool,
    format: OutputFormat,
    detailed_paths: bool,
}

#[derive(Debug)]
enum ErrorClass {
    Rejected,
    Failed,
}

#[derive(Debug)]
struct CliError {
    class: ErrorClass,
    message: String,
}

impl CliError {
    fn rejected(message: impl Into<String>) -> Self {
        Self {
            class: ErrorClass::Rejected,
            message: message.into(),
        }
    }

    fn failed(message: impl Into<String>) -> Self {
        Self {
            class: ErrorClass::Failed,
            message: message.into(),
        }
    }
}

impl From<String> for CliError {
    fn from(message: String) -> Self {
        Self::failed(message)
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

fn parse_command(args: &[String]) -> Result<Command, &'static str> {
    match args.first().map(String::as_str) {
        Some("scan") => Ok(Command::Scan),
        Some("preview") => Ok(Command::Preview),
        Some("export") => Ok(Command::Export),
        Some("apply-policy") => Ok(Command::ApplyPolicy),
        Some("schedule") => Ok(Command::Schedule),
        _ => Err("expected scan, preview, export, apply-policy or schedule"),
    }
}

fn parse_options(args: &[String]) -> Result<Options, String> {
    let command = parse_command(args).map_err(str::to_owned)?;
    let mut options = Options {
        command,
        root: r"C:\".into(),
        output: None,
        policy: None,
        task_name: None,
        max_entries: 10_000,
        yes: false,
        format: OutputFormat::Json,
        detailed_paths: false,
    };
    let mut index = 1;
    while index < args.len() {
        match args[index].as_str() {
            "--yes" => options.yes = true,
            "--detailed-paths" => options.detailed_paths = true,
            "--root" | "--output" | "--policy" | "--max-entries" | "--format" | "--task-name" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| format!("{} requires a value", args[index]))?;
                match args[index].as_str() {
                    "--root" => options.root = value.clone(),
                    "--output" => options.output = Some(value.into()),
                    "--policy" => options.policy = Some(value.into()),
                    "--max-entries" => {
                        options.max_entries = value
                            .parse()
                            .map_err(|_| "--max-entries must be an integer".to_owned())?
                    }
                    "--format" => {
                        options.format = match value.to_ascii_lowercase().as_str() {
                            "json" => OutputFormat::Json,
                            "csv" => OutputFormat::Csv,
                            "html" => OutputFormat::Html,
                            _ => return Err("--format must be json, csv or html".into()),
                        }
                    }
                    "--task-name" => options.task_name = Some(value.clone()),
                    _ => unreachable!(),
                }
                index += 1;
            }
            unknown => return Err(format!("unknown argument: {unknown}")),
        }
        index += 1;
    }
    if options.command == Command::ApplyPolicy && options.policy.is_none() {
        return Err("apply-policy requires --policy".into());
    }
    if options.command == Command::Schedule && options.task_name.is_none() {
        return Err("schedule requires --task-name".into());
    }
    Ok(options)
}

fn main() {
    let args: Vec<_> = env::args().skip(1).collect();
    let options = parse_options(&args).unwrap_or_else(|error| {
        eprintln!("{error}");
        std::process::exit(2);
    });
    match run(options) {
        Ok(warning) => std::process::exit(if warning { 1 } else { 0 }),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(match error.class {
                ErrorClass::Rejected => 2,
                ErrorClass::Failed => 3,
            });
        }
    }
}

fn run(options: Options) -> Result<bool, CliError> {
    if options.command == Command::Schedule {
        let executable = env::current_exe().map_err(|error| error.to_string())?;
        platform_windows::services::register_read_only_scan_task(
            &executable,
            options.task_name.as_deref().expect("validated task name"),
        )?;
        println!("scheduled read-only scan");
        return Ok(false);
    }
    if options.command == Command::ApplyPolicy {
        let path = options.policy.expect("validated policy path");
        let envelope: policy::PolicyEnvelope =
            toml::from_str(&fs::read_to_string(path).map_err(|error| error.to_string())?)
                .map_err(|error| error.to_string())?;
        let envelope_for_storage = envelope.clone();
        let trust_root = policy::Ed25519TrustRoot::from_hex(
            "rusten-default",
            "ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c",
        )?;
        let policy = policy::verify_embedded_signature(envelope, &trust_root)
            .inspect_err(|error| {
                let _ = append_policy_audit("rejected", Some(error), "unknown");
            })
            .map_err(|error| CliError::rejected(error.to_string()))?;
        let audit_root = std::env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .ok_or_else(|| {
                "LOCALAPPDATA is unavailable; managed policy audit cannot be written".to_owned()
            })?
            .join("Rusten");
        let audit = audit::AuditLog::open(audit_root.join("audit.jsonl"))
            .map_err(|error| error.to_string())?;
        if options.yes {
            let managed_policy = audit_root.join("managed-policy.toml");
            if managed_policy.is_file() {
                let current = toml::from_str::<policy::EnterprisePolicy>(
                    &fs::read_to_string(&managed_policy).map_err(|error| error.to_string())?,
                )
                .map_err(|error| error.to_string())?;
                if !version_at_least(&policy.version, &current.version) {
                    let error = "managed policy rollback rejected";
                    let _ = append_policy_audit("rejected", Some(error), &policy.version);
                    return Err(CliError::rejected(error));
                }
            }
            let serialized =
                toml::to_string_pretty(&envelope_for_storage).map_err(|error| error.to_string())?;
            if let Err(error) = persistence::ConfigStore::write(&managed_policy, &serialized) {
                let message = error.to_string();
                let _ = append_policy_audit("failed", Some(&message), &policy.version);
                return Err(message.into());
            }
        }
        audit
            .append(&audit::AuditRecord {
                timestamp: now_secs(),
                command: "apply-policy".into(),
                policy_version: policy.version.clone(),
                operator: std::env::var("USERNAME").unwrap_or_else(|_| "unknown".into()),
                approver: None,
                device: "local".into(),
                result: if options.yes {
                    "applied".into()
                } else {
                    "dry-run".into()
                },
                error_code: None,
            })
            .map_err(|error| error.to_string())?;
        println!(
            "{} policy version {}",
            if options.yes { "applied" } else { "dry-run" },
            policy.version
        );
        return Ok(false);
    }
    let managed = load_managed_policy()?;
    let scan_root = managed
        .as_ref()
        .and_then(|policy| policy.scan_roots.first())
        .map(|root| root.display().to_owned())
        .unwrap_or_else(|| options.root.clone());
    let run = scan(&scan_root, options.max_entries)?;
    let report = report(
        &run,
        options.command == Command::Preview,
        options.format,
        options.detailed_paths,
        managed.as_ref(),
    )?;
    match options.command {
        Command::Export => {
            let output = options
                .output
                .unwrap_or_else(|| "rusten-report.json".into());
            let root = output
                .parent()
                .filter(|path| !path.as_os_str().is_empty())
                .unwrap_or_else(|| std::path::Path::new("."));
            let name = output
                .file_name()
                .and_then(|value| value.to_str())
                .ok_or_else(|| "report output must be a valid file name".to_owned())?;
            let path = persistence::ReportStore::open(root)
                .and_then(|store| store.write(name, report.as_bytes()))
                .map_err(|error| error.to_string())?;
            println!("exported {}", path.display());
        }
        _ => println!("{}", report),
    }
    Ok(run.status != ScanStatus::Complete || run.warnings > 0)
}

fn load_managed_policy() -> Result<Option<policy::EnterprisePolicy>, CliError> {
    let Some(root) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) else {
        return Ok(None);
    };
    let path = root.join("Rusten").join("managed-policy.toml");
    if !path.is_file() {
        return Ok(None);
    }
    let envelope: policy::PolicyEnvelope = toml::from_str(
        &fs::read_to_string(path).map_err(|error| CliError::failed(error.to_string()))?,
    )
    .map_err(|error| CliError::failed(error.to_string()))?;
    let trust_root = policy::Ed25519TrustRoot::from_hex(
        "rusten-default",
        "ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c",
    )
    .map_err(CliError::failed)?;
    policy::verify_embedded_signature(envelope, &trust_root)
        .map(Some)
        .map_err(|error| CliError::rejected(format!("managed policy rejected: {error}")))
}

fn version_at_least(candidate: &str, current: &str) -> bool {
    let parse = |value: &str| {
        let value = value.trim();
        (!value.is_empty())
            .then(|| {
                value
                    .split('.')
                    .map(str::parse)
                    .collect::<Result<Vec<u64>, _>>()
                    .ok()
            })
            .flatten()
    };
    parse(candidate)
        .zip(parse(current))
        .is_some_and(|(left, right)| left >= right)
}

fn append_policy_audit(
    result: &str,
    error: Option<&str>,
    policy_version: &str,
) -> Result<(), String> {
    let root = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .ok_or_else(|| "LOCALAPPDATA is unavailable".to_owned())?
        .join("Rusten");
    let audit =
        audit::AuditLog::open(root.join("audit.jsonl")).map_err(|value| value.to_string())?;
    audit
        .append(&audit::AuditRecord {
            timestamp: now_secs(),
            command: "apply-policy".into(),
            policy_version: policy_version.into(),
            operator: std::env::var("USERNAME").unwrap_or_else(|_| "unknown".into()),
            approver: None,
            device: "local".into(),
            result: result.into(),
            error_code: error.map(str::to_owned),
        })
        .map_err(|value| value.to_string())
}

struct ScanRun {
    root: domain::NormalizedPath,
    index: ScanIndex,
    statistics_files: Vec<domain::FileRecord>,
    engine: domain::ScanEngine,
    status: ScanStatus,
    file_count: u64,
    logical_bytes: u64,
    allocated_bytes: u64,
    warnings: usize,
    total_bytes: u64,
    available_bytes: u64,
}

fn scan(root: &str, max_entries: u64) -> Result<ScanRun, String> {
    let root = normalize_path(root).map_err(|error| error.to_string())?;
    let (sender, receiver) = mpsc::sync_channel(1024);
    let scanner_root = root.clone();
    let worker = thread::spawn(move || {
        FastFileScanner.scan(
            scanner_root,
            ScanOptions {
                max_depth: 512,
                max_entries,
                // Cleanup revalidates lock state; the scanner should stay a cheap read-only pass.
                collect_lock_state: false,
            },
            sender,
            Default::default(),
        )
    });
    let mut index =
        ScanIndex::new_with_path_matcher(root.clone(), platform_windows::paths_equal_ordinal);
    let mut engine = domain::ScanEngine::Win32;
    for event in receiver {
        match event {
            ScanEvent::Directory(directory) => {
                index.add_directory(directory);
            }
            ScanEvent::File(file) => {
                index.add_file(file);
            }
            ScanEvent::Warning(warning) => {
                index.add_warning(warning);
            }
            ScanEvent::Completed => {
                index.mark_complete();
            }
            ScanEvent::Partial => {
                index.mark_partial();
            }
            ScanEvent::Canceled => {
                index.mark_canceled();
            }
            ScanEvent::ReparsePoint(file) => {
                index.add_reparse_point(file);
            }
            ScanEvent::ScannerStatus { engine: next, .. } => {
                engine = next;
            }
            ScanEvent::CleanupCandidate(_) | ScanEvent::Progress(_) => {}
        }
    }
    let summary = worker
        .join()
        .map_err(|_| "scanner thread panicked".to_owned())?
        .map_err(|error| error.to_string())?;
    let rules = RuleService::load_builtin().map_err(|error| error.to_string())?;
    let status = summary.status();
    let statistics_files = if status == ScanStatus::Complete {
        Vec::new()
    } else {
        let statistics_patterns = supplemental_statistics_patterns(rules.rules());
        platform_windows::enumerate_statistics_files_for_patterns(
            &statistics_patterns,
            STATISTICS_SCAN_MAX_ENTRIES,
        )
        .map_err(|error| error.to_string())?
    };
    Ok(ScanRun {
        root,
        index,
        statistics_files,
        engine,
        status,
        file_count: summary.file_count(),
        logical_bytes: summary.logical_size(),
        allocated_bytes: summary.allocated_size(),
        warnings: summary.issues().len() + summary.omitted_issues() as usize,
        total_bytes: summary.volume().total_bytes(),
        available_bytes: summary.volume().available_bytes(),
    })
}

fn report(
    run: &ScanRun,
    include_candidates: bool,
    format: OutputFormat,
    detailed_paths: bool,
    managed: Option<&policy::EnterprisePolicy>,
) -> Result<String, String> {
    let rules = RuleService::load_builtin().map_err(|error| error.to_string())?;
    let all_candidates = if include_candidates {
        let mut files = run.index.files();
        files.extend(run.statistics_files.iter().cloned());
        rules.candidates(&files, &RuleMatchContext::default())
    } else {
        Vec::new()
    };
    let mut excluded_items = Vec::new();
    let candidates = all_candidates
        .into_iter()
        .filter(|candidate| {
            let excluded = managed.is_some_and(|managed| {
                managed
                    .exclusions
                    .iter()
                    .any(|path| path_prefix_equal(candidate.path.display(), path))
                    || policy::is_forced_forbidden(managed, &candidate.path)
            });
            if excluded {
                excluded_items.push(candidate.path.display().to_owned());
            }
            !excluded
        })
        .collect::<Vec<_>>();
    let data = ReportData {
        scanned_at: now_secs(),
        rule_version: "builtin-1".into(),
        root: run.root.display().into(),
        warnings: run.warnings as u64,
        candidate_count: candidates.len() as u64,
        excluded_count: excluded_items.len() as u64,
        excluded_items,
        estimated_bytes: candidates
            .iter()
            .map(|candidate| candidate.estimated_size.unwrap_or(0))
            .sum(),
        released_bytes: 0,
        failed_items: Vec::new(),
    };
    let input_method_reparse_bytes: u64 = run
        .index
        .reparse_points()
        .iter()
        .filter(|file| is_input_method_path(file.path().display()))
        .map(statistic_file_size)
        .sum();
    let input_method_reparse_paths: Vec<String> = run
        .index
        .reparse_points()
        .iter()
        .filter(|file| is_input_method_path(file.path().display()))
        .take(8)
        .map(|file| file.path().display().to_owned())
        .collect();
    let mut existing_input_method_paths = run
        .index
        .files()
        .into_iter()
        .chain(run.index.reparse_points().iter().cloned())
        .filter(|file| is_input_method_path(file.path().display()))
        .map(|file| file.path().display().to_ascii_lowercase())
        .collect::<HashSet<_>>();
    let mut input_method_combined_bytes = run
        .index
        .files()
        .into_iter()
        .chain(run.index.reparse_points().iter().cloned())
        .filter(|file| is_input_method_path(file.path().display()))
        .map(|file| {
            existing_input_method_paths.insert(file.path().display().to_ascii_lowercase());
            statistic_file_size(&file)
        })
        .sum::<u64>();
    input_method_combined_bytes = input_method_combined_bytes.saturating_add(
        run.statistics_files
            .iter()
            .filter(|file| is_input_method_path(file.path().display()))
            .filter(|file| {
                existing_input_method_paths.insert(file.path().display().to_ascii_lowercase())
            })
            .map(statistic_file_size)
            .sum::<u64>(),
    );
    let statistics_categories = statistics_category_totals(&rules, run);
    let mut value = serde_json::json!({
        "status": format!("{:?}", run.status),
        "engine": format!("{:?}", run.engine),
        "files": run.file_count,
        "reparse_points": run.index.reparse_points().len(),
        "input_method_reparse_bytes": input_method_reparse_bytes,
        "input_method_combined_bytes": input_method_combined_bytes,
        "input_method_reparse_paths": input_method_reparse_paths,
        "statistics_files": run.statistics_files.len(),
        "statistics_categories": statistics_categories,
        "logical_bytes": run.logical_bytes,
        "allocated_bytes": run.allocated_bytes,
        "total_bytes": run.total_bytes,
        "available_bytes": run.available_bytes,
    });
    let base = render_report(&data, format.report_format(), detailed_paths)?;
    if format == OutputFormat::Json {
        value = serde_json::from_str(&String::from_utf8(base).map_err(|error| error.to_string())?)
            .map_err(|error| error.to_string())?;
        value["status"] = format!("{:?}", run.status).into();
        value["engine"] = format!("{:?}", run.engine).into();
        value["files"] = run.file_count.into();
        value["reparse_points"] = run.index.reparse_points().len().into();
        value["input_method_reparse_bytes"] = input_method_reparse_bytes.into();
        value["input_method_combined_bytes"] = input_method_combined_bytes.into();
        value["input_method_reparse_paths"] = serde_json::json!(input_method_reparse_paths);
        value["statistics_files"] = run.statistics_files.len().into();
        value["statistics_categories"] = serde_json::json!(statistics_categories);
        value["logical_bytes"] = run.logical_bytes.into();
        value["allocated_bytes"] = run.allocated_bytes.into();
        value["total_bytes"] = run.total_bytes.into();
        value["available_bytes"] = run.available_bytes.into();
    } else {
        return String::from_utf8(base).map_err(|error| error.to_string());
    }
    if include_candidates {
        value["candidate_count"] = candidates.len().into();
        value["estimated_bytes"] = candidates
            .iter()
            .map(|candidate| candidate.estimated_size.unwrap_or(0))
            .sum::<u64>()
            .into();
        value["candidates"] = candidates
            .iter()
            .map(|candidate| {
                serde_json::json!({
                    "rule_id": candidate.rule_id,
                    "category": candidate.category,
                    "size": candidate.estimated_size,
                    "risk": candidate.risk.map(|risk| format!("{risk:?}")),
                    "recovery": candidate.recovery.map(|recovery| format!("{recovery:?}")),
                    "quick_clean_eligible": candidate.quick_clean_eligible,
                    "reason": candidate.reason,
                })
            })
            .collect::<Vec<_>>()
            .into();
    }
    serde_json::to_string_pretty(&value).map_err(|error| error.to_string())
}

fn statistics_category_totals(
    rules: &RuleService,
    run: &ScanRun,
) -> BTreeMap<String, serde_json::Value> {
    let mut totals = BTreeMap::<String, (u64, u64)>::new();
    let mut seen = HashSet::new();
    for file in run
        .index
        .files()
        .into_iter()
        .chain(run.index.reparse_points().iter().cloned())
        .chain(run.statistics_files.iter().cloned())
    {
        let Some((key, bytes)) = rules.rules().iter().find_map(|rule| {
            rule.display_item_for_path(file.path())
                .map(|item| (item.key.clone(), statistic_file_size(&file)))
        }) else {
            continue;
        };
        if seen.insert((key.clone(), file.path().display().to_ascii_lowercase())) {
            let entry = totals.entry(key).or_default();
            entry.0 += 1;
            entry.1 = entry.1.saturating_add(bytes);
        }
    }
    totals
        .into_iter()
        .map(|(key, (count, bytes))| (key, serde_json::json!({ "count": count, "bytes": bytes })))
        .collect()
}

fn statistic_file_size(file: &domain::FileRecord) -> u64 {
    if file.allocated_size() == 0 {
        file.logical_size()
    } else {
        file.allocated_size()
    }
}

fn is_input_method_path(path: &str) -> bool {
    let path = path.to_ascii_lowercase();
    path.contains(r"\windows\system32\ime\")
        || path.contains(r"\windows\ime\")
        || path.contains(r"\windows\system32\inputmethod\")
}

fn supplemental_statistics_patterns(rules: &[domain::Rule]) -> Vec<String> {
    rules
        .iter()
        .flat_map(|rule| rule.display_items())
        .flat_map(|item| item.paths.iter().cloned())
        .collect()
}

fn path_prefix_equal(path: &str, prefix: &str) -> bool {
    let path = path.trim_end_matches(['\\', '/']).to_ascii_lowercase();
    let prefix = prefix.trim_end_matches(['\\', '/']).to_ascii_lowercase();
    path == prefix
        || path
            .strip_prefix(&prefix)
            .is_some_and(|rest| rest.starts_with('\\') || rest.starts_with('/'))
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_defaults_to_offline_dry_run() {
        let options = parse_options(&["preview".into()]).unwrap();
        assert_eq!(options.command, Command::Preview);
        assert!(!options.yes);
        assert!(!options.detailed_paths);
        assert_eq!(options.root, r"C:\");
    }

    #[test]
    fn removed_boolean_does_not_bypass_policy_verification() {
        assert!(parse_options(&["preview".into(), "--verified-policy".into()]).is_err());
    }

    #[test]
    fn detailed_paths_are_an_explicit_opt_in() {
        let options = parse_options(&["export".into(), "--detailed-paths".into()]).unwrap();
        assert!(options.detailed_paths);
    }

    #[test]
    fn managed_policy_versions_cannot_roll_back() {
        assert!(version_at_least("2.1", "2.0"));
        assert!(!version_at_least("1.9", "2.0"));
        assert!(!version_at_least("release", "2.0"));
    }

    #[test]
    fn supplemental_statistics_walks_all_display_paths() {
        let rules = RuleService::load_builtin().unwrap();
        let patterns = supplemental_statistics_patterns(rules.rules());

        assert!(!patterns.is_empty());
        assert!(
            patterns
                .iter()
                .any(|pattern| pattern.contains("SoftwareDistribution"))
        );
        assert!(
            patterns
                .iter()
                .any(|pattern| pattern.replace('/', "\\").contains("AppData\\Local\\Temp"))
        );
    }
}
