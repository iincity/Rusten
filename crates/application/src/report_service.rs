use serde::Serialize;

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct ReportData {
    pub scanned_at: u64,
    pub rule_version: String,
    pub root: String,
    pub warnings: u64,
    pub candidate_count: u64,
    pub excluded_count: u64,
    pub excluded_items: Vec<String>,
    pub estimated_bytes: u64,
    pub released_bytes: u64,
    pub failed_items: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReportFormat {
    Json,
    Csv,
    Html,
}

pub fn redact_path(path: &str, detailed: bool) -> String {
    if detailed {
        path.into()
    } else {
        "<redacted-path>".into()
    }
}

pub fn render(data: &ReportData, format: ReportFormat, detailed: bool) -> Result<Vec<u8>, String> {
    let mut safe = data.clone();
    safe.root = redact_path(&data.root, detailed);
    safe.failed_items = data
        .failed_items
        .iter()
        .map(|path| redact_path(path, detailed))
        .collect();
    safe.excluded_items = data
        .excluded_items
        .iter()
        .map(|path| redact_path(path, detailed))
        .collect();
    match format {
        ReportFormat::Json => serde_json::to_vec_pretty(&safe).map_err(|error| error.to_string()),
        ReportFormat::Csv => Ok(format!("scanned_at,root,rule_version,warnings,candidate_count,excluded_count,excluded_items,estimated_bytes,released_bytes,failed_count,failed_items\n{},{},{},{},{},{},{},{},{},{},{}\n", data.scanned_at, csv(&safe.root), csv(&data.rule_version), data.warnings, data.candidate_count, data.excluded_count, csv(&safe.excluded_items.join(";")), data.estimated_bytes, data.released_bytes, data.failed_items.len(), csv(&safe.failed_items.join(";"))).into_bytes()),
        ReportFormat::Html => Ok(format!("<!doctype html><meta charset=\"utf-8\"><h1>Rusten report</h1><p>Scanned: {}</p><p>Root: {}</p><p>Rule version: {}</p><p>Warnings: {}</p><p>Candidates: {}</p><p>Excluded: {}</p><p>Estimated: {} bytes</p><p>Released: {} bytes</p><p>Failed: {}</p><p>{}</p>", safe.scanned_at, escape(&safe.root), escape(&safe.rule_version), safe.warnings, safe.candidate_count, safe.excluded_count, safe.estimated_bytes, safe.released_bytes, safe.failed_items.len(), safe.failed_items.iter().map(|path| escape(path)).collect::<Vec<_>>().join("<br>" )).into_bytes()),
    }
}

fn csv(value: &str) -> String {
    format!("\"{}\"", value.replace('"', "\"\""))
}
fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn report_defaults_to_redacted_paths() {
        let path = redact_path(r"C:\Users\Alice\secret.txt", false);
        assert_eq!(path, "<redacted-path>");
        let data = ReportData {
            root: r"C:\Users\Alice".into(),
            failed_items: vec![r"C:\Users\Alice\<x>".into()],
            ..Default::default()
        };
        let html = render(&data, ReportFormat::Html, false).unwrap();
        let text = String::from_utf8(html).unwrap();
        assert!(text.contains("&lt;redacted-path&gt;"));
        assert!(!text.contains("Alice"));
        let json = String::from_utf8(render(&data, ReportFormat::Json, false).unwrap()).unwrap();
        assert!(!json.contains("Alice"));
        assert!(!text.contains("Alice"));
    }
}
