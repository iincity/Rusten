use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct TelemetryEvent {
    pub created_at: u64,
    pub product_version: String,
    pub os_version: String,
    pub architecture: String,
    pub duration_bucket: String,
    pub error_code: Option<String>,
    pub result_category: String,
}
