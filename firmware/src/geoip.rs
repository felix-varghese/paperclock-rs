use embedded_svc::http::client::Client as HttpClient;
use embedded_svc::utils::io;
use esp_idf_svc::http::client::{Configuration as HttpConfiguration, EspHttpConnection};
use serde::Deserialize;

#[derive(Deserialize)]
struct GeoIpResponse {
    status: String,
    lat: f64,
    lon: f64,
    city: Option<String>,
}

pub struct GeoResult {
    pub lat: f64,
    pub lon: f64,
    pub city: Option<String>,
}

/// City-level location from the requesting IP, via ip-api.com's free tier
/// (no key, plain HTTP). Used when wifi_config.json doesn't set lat/lon.
/// Accurate enough for weather. Returns None on any failure, logged.
pub fn lookup() -> Option<GeoResult> {
    let connection = match EspHttpConnection::new(&HttpConfiguration::default()) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("GeoIP: failed to create HTTP connection: {e:?}");
            return None;
        }
    };
    let mut client = HttpClient::wrap(connection);

    let url = "http://ip-api.com/json/";
    let request = match client.get(url) {
        Ok(r) => r,
        Err(e) => {
            log::warn!("GeoIP: failed to build request: {e:?}");
            return None;
        }
    };
    log::info!("GeoIP: GET {url}");
    let mut response = match request.submit() {
        Ok(r) => r,
        Err(e) => {
            log::warn!("GeoIP: request failed: {e:?}");
            return None;
        }
    };

    let status = response.status();
    if status != 200 {
        log::warn!("GeoIP: unexpected HTTP status {status}");
        return None;
    }

    let mut buf = [0u8; 512];
    let bytes_read = match io::try_read_full(&mut response, &mut buf) {
        Ok(n) => n,
        Err((e, _)) => {
            log::warn!("GeoIP: failed to read response body: {e:?}");
            return None;
        }
    };

    let body = match std::str::from_utf8(&buf[..bytes_read]) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("GeoIP: response body wasn't valid UTF-8: {e:?}");
            return None;
        }
    };

    match serde_json::from_str::<GeoIpResponse>(body) {
        Ok(parsed) if parsed.status == "success" => {
            log::info!(
                "GeoIP: approx location lat={:.2} lon={:.2} city={:?}",
                parsed.lat,
                parsed.lon,
                parsed.city
            );
            Some(GeoResult {
                lat: parsed.lat,
                lon: parsed.lon,
                city: parsed.city,
            })
        }
        Ok(_) => {
            log::warn!("GeoIP: lookup returned non-success status");
            None
        }
        Err(e) => {
            log::warn!("GeoIP: failed to parse response JSON: {e}");
            None
        }
    }
}
