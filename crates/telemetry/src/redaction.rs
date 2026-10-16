use crate::event_model::TelemetryEvent;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DiagnosticRecord {
    pub category: String,
    pub message: String,
    pub path: Option<String>,
    pub file_name: Option<String>,
    pub content: Option<String>,
    pub hash: Option<String>,
}

pub fn redact_text(value: &str) -> String {
    let mut output = value.replace(['\r', '\n'], " ");
    let lower = output.to_ascii_lowercase();
    if looks_like_hash(&output) {
        output = "<redacted-hash>".into();
    } else if output.contains('\\')
        || output.contains('/')
        || looks_like_file_name(&output)
        || lower.contains("users")
        || lower.contains("appdata")
        || lower.contains("login data")
        || lower.contains("cookies")
        || lower.contains("password")
    {
        output = "<redacted-path>".into();
    }
    output
}

pub fn redact_diagnostic_record(mut record: DiagnosticRecord) -> DiagnosticRecord {
    record.category = redact_text(&record.category);
    record.message = redact_text(&record.message);
    record.path = record.path.map(|_| "<redacted-path>".into());
    record.file_name = record.file_name.map(|_| "<redacted-path>".into());
    record.content = record.content.map(|_| "<redacted-content>".into());
    record.hash = record.hash.map(|_| "<redacted-hash>".into());
    record
}

pub fn redact_event(mut event: TelemetryEvent) -> TelemetryEvent {
    event.product_version = redact_text(&event.product_version);
    event.os_version = redact_text(&event.os_version);
    event.architecture = redact_text(&event.architecture);
    event.duration_bucket = redact_text(&event.duration_bucket);
    event.error_code = event.error_code.map(|value| redact_error_code(&value));
    event.result_category = redact_text(&event.result_category);
    event
}

fn redact_error_code(value: &str) -> String {
    let value = redact_text(value);
    if value == "<redacted-path>"
        || value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '_' | '-' | '.')
        }) && !value.contains('.')
    {
        value
    } else {
        "<redacted-error>".into()
    }
}

fn looks_like_hash(value: &str) -> bool {
    matches!(value.len(), 32 | 40 | 64)
        && value.chars().all(|character| character.is_ascii_hexdigit())
}

fn looks_like_file_name(value: &str) -> bool {
    let Some((stem, extension)) = value.rsplit_once('.') else {
        return false;
    };
    !stem.is_empty()
        && !extension.is_empty()
        && extension.len() <= 5
        && extension
            .chars()
            .all(|character| character.is_ascii_alphabetic())
        && !value
            .chars()
            .all(|character| character.is_ascii_digit() || character == '.')
}
