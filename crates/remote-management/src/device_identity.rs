use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct DeviceIdentity {
    pub device_id: Option<String>,
}

impl DeviceIdentity {
    pub fn disabled() -> Self {
        Self { device_id: None }
    }
}
