// Template secrets. CI (and clean checkouts) copy this file to
// `app/src/secrets.rs`, which is gitignored and must never contain real
// credentials in the repository.

pub const WIFI_SSID: &str = "CHANGE_ME";
pub const WIFI_PASSWORD: &str = "CHANGE_ME";
pub const MQTT_HOST: &str = "127.0.0.1";
pub const MQTT_PORT: u16 = 1883;
