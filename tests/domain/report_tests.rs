use application::{ReportData, ReportFormat, report_service::render};

#[test]
fn json_report_redacts_failure_paths_by_default() {
    let report = ReportData {
        failed_items: vec![r"C:\Users\Alice\secret.txt".into()],
        ..Default::default()
    };
    let output = String::from_utf8(render(&report, ReportFormat::Json, false).unwrap()).unwrap();
    assert!(!output.contains("Alice"));
    assert!(output.contains("<redacted-path>"));
}

#[test]
fn every_report_format_retains_auditable_summary_fields() {
    let report = ReportData {
        scanned_at: 123,
        rule_version: "rules-7".into(),
        root: r"C:\".into(),
        warnings: 2,
        candidate_count: 3,
        excluded_count: 4,
        excluded_items: vec!["system".into()],
        estimated_bytes: 50,
        released_bytes: 40,
        failed_items: vec![r"C:\Users\Alice\failed.tmp".into()],
    };

    for format in [ReportFormat::Json, ReportFormat::Csv, ReportFormat::Html] {
        let output = String::from_utf8(render(&report, format, false).unwrap()).unwrap();
        assert!(output.contains("123"));
        assert!(output.contains("rules-7"));
        assert!(output.contains("50"));
        assert!(output.contains("40"));
        assert!(output.contains("<redacted-path>") || output.contains("&lt;redacted-path&gt;"));
        assert!(!output.contains("Alice"));
    }
}

#[test]
fn detailed_html_report_escapes_untrusted_values() {
    let report = ReportData {
        root: "<root>".into(),
        rule_version: "<rules>".into(),
        failed_items: vec!["<failed>".into()],
        ..Default::default()
    };
    let output = String::from_utf8(render(&report, ReportFormat::Html, true).unwrap()).unwrap();
    assert!(output.contains("&lt;root&gt;"));
    assert!(output.contains("&lt;rules&gt;"));
    assert!(output.contains("&lt;failed&gt;"));
    assert!(!output.contains("<root>"));
}
