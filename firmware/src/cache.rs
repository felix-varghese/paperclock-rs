//! Keeps the last-fetched network data and the last-drawn frame values in
//! RTC memory, which survives deep sleep and resets only on power-on.
//! Deep sleep is a full reboot, so a plain local can't carry state between
//! wakes; without this, wakes that skip the network refresh would draw
//! "--" for data that's still valid.
//!
//! The heap is torn down on every wake, so nothing here can hold a
//! `String` or any other pointer - hence the fixed-size byte buffers.

use crate::weather::Weather;

#[link_section = ".rtc.data"]
static mut WEATHER_VALID: bool = false;
#[link_section = ".rtc.data"]
static mut TEMPERATURE_C: f32 = 0.0;
#[link_section = ".rtc.data"]
static mut WEATHER_CODE: u32 = 0;

const CITY_BUF_LEN: usize = 32;
#[link_section = ".rtc.data"]
static mut CITY_LEN: u8 = 0;
#[link_section = ".rtc.data"]
static mut CITY_BUF: [u8; CITY_BUF_LEN] = [0; CITY_BUF_LEN];

#[link_section = ".rtc.data"]
static mut WIFI_CONNECTED: bool = false;
#[link_section = ".rtc.data"]
static mut WIFI_RSSI_VALID: bool = false;
#[link_section = ".rtc.data"]
static mut WIFI_RSSI: i32 = 0;

// Weather and geoip refresh on different cadences, so a weather-only wake
// needs the last-resolved lat/lon to query with.
#[link_section = ".rtc.data"]
static mut LOCATION_VALID: bool = false;
#[link_section = ".rtc.data"]
static mut LATITUDE: f32 = 0.0;
#[link_section = ".rtc.data"]
static mut LONGITUDE: f32 = 0.0;

pub fn load_location() -> Option<(f64, f64)> {
    unsafe {
        (*core::ptr::addr_of!(LOCATION_VALID)).then(|| {
            (
                *core::ptr::addr_of!(LATITUDE) as f64,
                *core::ptr::addr_of!(LONGITUDE) as f64,
            )
        })
    }
}

pub fn store_location(lat: f64, lon: f64) {
    unsafe {
        *core::ptr::addr_of_mut!(LATITUDE) = lat as f32;
        *core::ptr::addr_of_mut!(LONGITUDE) = lon as f32;
        *core::ptr::addr_of_mut!(LOCATION_VALID) = true;
    }
}

// "HH:MM" is always 5 bytes, so no length field is needed. Used to
// reconstruct the previous frame - see `RefreshMode` in display.rs.
#[link_section = ".rtc.data"]
static mut PREV_TIME: [u8; 5] = *b"--:--";

pub fn load_prev_time() -> String {
    unsafe { String::from_utf8_lossy(&*core::ptr::addr_of!(PREV_TIME)).into_owned() }
}

pub fn store_time(time_str: &str) {
    unsafe {
        let buf = &mut *core::ptr::addr_of_mut!(PREV_TIME);
        let bytes = time_str.as_bytes();
        let len = bytes.len().min(buf.len());
        buf[..len].copy_from_slice(&bytes[..len]);
    }
}

const PREV_BATTERY_LEN: usize = 4; // "100%" is the longest value
#[link_section = ".rtc.data"]
static mut PREV_BATTERY: [u8; PREV_BATTERY_LEN] = *b"--%\0";

pub fn load_prev_battery() -> String {
    unsafe {
        String::from_utf8_lossy(&*core::ptr::addr_of!(PREV_BATTERY))
            .trim_end_matches('\0')
            .to_string()
    }
}

pub fn store_battery(battery_str: &str) {
    unsafe {
        let buf = &mut *core::ptr::addr_of_mut!(PREV_BATTERY);
        *buf = [0; PREV_BATTERY_LEN];
        let bytes = battery_str.as_bytes();
        let len = bytes.len().min(PREV_BATTERY_LEN);
        buf[..len].copy_from_slice(&bytes[..len]);
    }
}

// Separate from PREV_BATTERY (the label text) because the icon's fill bar
// is drawn from the percent, not the formatted string.
#[link_section = ".rtc.data"]
static mut PREV_BATTERY_PERCENT: u8 = 0;

pub fn load_prev_battery_percent() -> u8 {
    unsafe { *core::ptr::addr_of!(PREV_BATTERY_PERCENT) }
}

pub fn store_battery_percent(percent: u8) {
    unsafe { *core::ptr::addr_of_mut!(PREV_BATTERY_PERCENT) = percent }
}

// "Wednesday, 31-12-2026" (22 chars) is the longest value.
const PREV_DATE_LEN: usize = 24;
#[link_section = ".rtc.data"]
static mut PREV_DATE: [u8; PREV_DATE_LEN] = [0; PREV_DATE_LEN];
#[link_section = ".rtc.data"]
static mut PREV_DATE_LEN_USED: u8 = 0;

pub fn load_prev_date() -> String {
    unsafe {
        let buf = &*core::ptr::addr_of!(PREV_DATE);
        let len = *core::ptr::addr_of!(PREV_DATE_LEN_USED) as usize;
        String::from_utf8_lossy(&buf[..len]).into_owned()
    }
}

pub fn store_date(date_str: &str) {
    unsafe {
        let buf = &mut *core::ptr::addr_of_mut!(PREV_DATE);
        let bytes = date_str.as_bytes();
        let len = bytes.len().min(PREV_DATE_LEN);
        buf[..len].copy_from_slice(&bytes[..len]);
        *core::ptr::addr_of_mut!(PREV_DATE_LEN_USED) = len as u8;
    }
}

// Whether the charging bolt was drawn last wake - its presence changes,
// so the reconstructed previous frame needs to know.
#[link_section = ".rtc.data"]
static mut PREV_CHARGING: bool = false;

pub fn load_prev_charging() -> bool {
    unsafe { *core::ptr::addr_of!(PREV_CHARGING) }
}

pub fn store_charging(charging: bool) {
    unsafe { *core::ptr::addr_of_mut!(PREV_CHARGING) = charging }
}

pub struct Cached {
    pub weather: Option<Weather>,
    pub city: Option<String>,
    pub wifi_status: Option<crate::wifi::WifiStatus>,
}

/// Reads back whatever was last stored. Call before the network refresh,
/// so a wake that skips it still has data to draw.
pub fn load() -> Cached {
    unsafe {
        let weather = WEATHER_VALID.then(|| Weather {
            temperature_c: TEMPERATURE_C as f64,
            weather_code: WEATHER_CODE,
        });
        let city = (CITY_LEN > 0).then(|| {
            String::from_utf8_lossy(&CITY_BUF[..CITY_LEN as usize]).into_owned()
        });
        let wifi_status = (WIFI_CONNECTED || WIFI_RSSI_VALID).then(|| crate::wifi::WifiStatus {
            connected: WIFI_CONNECTED,
            rssi: WIFI_RSSI_VALID.then_some(WIFI_RSSI),
        });
        Cached { weather, city, wifi_status }
    }
}

/// Call after a network refresh to persist the values for later wakes.
pub fn store(weather: Option<&Weather>, city: Option<&str>, wifi_status: &crate::wifi::WifiStatus) {
    unsafe {
        match weather {
            Some(w) => {
                WEATHER_VALID = true;
                TEMPERATURE_C = w.temperature_c as f32;
                WEATHER_CODE = w.weather_code;
            }
            None => WEATHER_VALID = false,
        }

        match city {
            Some(c) => {
                let bytes = c.as_bytes();
                let len = bytes.len().min(CITY_BUF_LEN);
                CITY_BUF[..len].copy_from_slice(&bytes[..len]);
                CITY_LEN = len as u8;
            }
            None => CITY_LEN = 0,
        }

        WIFI_CONNECTED = wifi_status.connected;
        WIFI_RSSI_VALID = wifi_status.rssi.is_some();
        WIFI_RSSI = wifi_status.rssi.unwrap_or(0);
    }
}
