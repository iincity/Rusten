use crate::{consent::Consent, event_model::TelemetryEvent, redaction::redact_event};
use ring::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EncryptedTelemetry(pub Vec<u8>);

pub fn encrypt_bytes(
    payload: &[u8],
    key: &[u8; 32],
    nonce: [u8; 12],
) -> Result<EncryptedTelemetry, String> {
    let mut bytes = payload.to_vec();
    let key =
        LessSafeKey::new(UnboundKey::new(&AES_256_GCM, key).map_err(|_| "invalid telemetry key")?);
    key.seal_in_place_append_tag(
        Nonce::assume_unique_for_key(nonce),
        Aad::empty(),
        &mut bytes,
    )
    .map_err(|_| "telemetry encryption failed")?;
    Ok(EncryptedTelemetry(bytes))
}

pub fn encrypt_event(
    event: TelemetryEvent,
    key: &[u8; 32],
    nonce: [u8; 12],
) -> Result<EncryptedTelemetry, String> {
    let payload = serde_json::to_vec(&redact_event(event)).map_err(|error| error.to_string())?;
    encrypt_bytes(&payload, key, nonce)
}

pub trait Transport {
    fn send(&self, event: &EncryptedTelemetry) -> Result<(), String>;
}

pub fn send_with_consent<T: Transport>(
    transport: &T,
    consent: Consent,
    event: TelemetryEvent,
    key: &[u8; 32],
    nonce: [u8; 12],
) -> Result<bool, String> {
    if !consent.is_enabled() {
        return Ok(false);
    }
    let encrypted = encrypt_event(event, key, nonce)?;
    transport.send(&encrypted).map(|()| true)
}
pub struct DisabledTransport;
impl Transport for DisabledTransport {
    fn send(&self, _: &EncryptedTelemetry) -> Result<(), String> {
        Err("telemetry transport disabled".into())
    }
}
