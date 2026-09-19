// Template secrets. Copy this file to `app/src/secrets.rs`, which is
// gitignored: `cp app/secrets.example.rs app/src/secrets.rs`.
//
// Real credentials must never be committed. Only this template (with
// `CHANGE_ME` placeholders) belongs in the repository.
//
// Secrets are never delivered over the air: the OTA image contains only
// firmware, and no credential is ever written to the telemetry payload.

pub const WIFI_SSID: &str = "CHANGE_ME";
pub const WIFI_PASSWORD: &str = "CHANGE_ME";
pub const MQTT_BROKER: &str = "mqtt://CHANGE_ME:1883";
pub const OTA_URL: &str = "CHANGE_ME";
pub const OTA_PROJECT: &str = "CHANGE_ME";
pub const OTA_USER: &str = "CHANGE_ME";
pub const OTA_PASSWORD: &str = "CHANGE_ME";
