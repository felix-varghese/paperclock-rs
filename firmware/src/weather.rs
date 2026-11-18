use embedded_svc::http::client::Client as HttpClient;
use embedded_svc::utils::io;
use esp_idf_svc::http::client::{Configuration as HttpConfiguration, EspHttpConnection};
use serde::Deserialize;

#[derive(Deserialize)]
struct ForecastResponse {
    current: CurrentWeather,
}

#[derive(Deserialize)]
struct CurrentWeather {
    temperature_2m: f64,
    weather_code: u32,
}

#[derive(Clone, Copy)]
pub struct Weather {
    pub temperature_c: f64,
    /// WMO weather code; mapped to an icon by `WeatherIcon::from_wmo_code`.
    pub weather_code: u32,
}

/// Fetches current conditions for (lat, lon) from Open-Meteo. Returns None
/// on any failure, logged.
pub fn fetch(lat: f64, lon: f64) -> Option<Weather> {
    let url = format!(
        "https://api.open-meteo.com/v1/forecast?latitude={lat}&longitude={lon}&current=temperature_2m,weather_code&timezone=auto"
    );

    let config = HttpConfiguration {
        crt_bundle_attach: Some(esp_idf_svc::sys::esp_crt_bundle_attach),
        ..Default::default()
    };
    let connection = match EspHttpConnection::new(&config) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("Weather: failed to create HTTP connection: {e:?}");
            return None;
        }
    };
    let mut client = HttpClient::wrap(connection);

    let request = match client.get(&url) {
        Ok(r) => r,
        Err(e) => {
            log::warn!("Weather: failed to build request: {e:?}");
            return None;
        }
    };
    log::info!("Weather: GET {url}");
    let mut response = match request.submit() {
        Ok(r) => r,
        Err(e) => {
            log::warn!("Weather: request failed: {e:?}");
            return None;
        }
    };

    let status = response.status();
    if status != 200 {
        log::warn!("Weather: unexpected HTTP status {status}");
        return None;
    }

    let mut buf = [0u8; 512];
    let bytes_read = match io::try_read_full(&mut response, &mut buf) {
        Ok(n) => n,
        Err((e, _)) => {
            log::warn!("Weather: failed to read response body: {e:?}");
            return None;
        }
    };

    let body = match std::str::from_utf8(&buf[..bytes_read]) {
        Ok(s) => s,
        Err(e) => {
            log::warn!("Weather: response body wasn't valid UTF-8: {e:?}");
            return None;
        }
    };

    match serde_json::from_str::<ForecastResponse>(body) {
        Ok(parsed) => {
            log::info!(
                "Weather: {:.1}C, WMO code {}",
                parsed.current.temperature_2m,
                parsed.current.weather_code
            );
            Some(Weather {
                temperature_c: parsed.current.temperature_2m,
                weather_code: parsed.current.weather_code,
            })
        }
        Err(e) => {
            log::warn!("Weather: failed to parse response JSON: {e}");
            None
        }
    }
}
