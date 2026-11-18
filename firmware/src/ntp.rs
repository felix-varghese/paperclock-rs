use esp_idf_svc::hal::delay::FreeRtos;
use esp_idf_svc::hal::i2c::I2cDriver;
use esp_idf_svc::sntp::{EspSntp, SyncStatus};
use esp_idf_svc::sys::{localtime_r, setenv, time, time_t, tm, tzset};
use std::ffi::CString;

const ADDR_RTC_BM8563: u8 = 0x51;
const SYNC_TIMEOUT_MS: u32 = 15_000;

fn dec_to_bcd(dec: u8) -> u8 {
    ((dec / 10) << 4) | (dec % 10)
}

/// Syncs time over SNTP and writes it, as local time (Europe/Amsterdam),
/// into the BM8563 RTC. Best-effort: logs and returns on failure, leaving
/// the RTC with whatever time it already had.
pub fn sync_time_to_rtc(i2c: &mut I2cDriver) {
    unsafe {
        let name = CString::new("TZ").unwrap();
        // Europe/Amsterdam: CET (UTC+1) / CEST (UTC+2) with EU DST rules.
        let value = CString::new("CET-1CEST,M3.5.0,M10.5.0/3").unwrap();
        setenv(name.as_ptr(), value.as_ptr(), 1);
        tzset();
    }

    let sntp = match EspSntp::new_default() {
        Ok(s) => s,
        Err(e) => {
            log::warn!("NTP: failed to init SNTP client: {e:?}");
            return;
        }
    };

    log::info!("NTP: waiting for time sync...");
    let mut waited_ms = 0;
    while sntp.get_sync_status() != SyncStatus::Completed {
        if waited_ms >= SYNC_TIMEOUT_MS {
            log::warn!("NTP: sync did not complete within {SYNC_TIMEOUT_MS}ms, giving up");
            return;
        }
        FreeRtos::delay_ms(200);
        waited_ms += 200;
    }
    log::info!("NTP: sync completed");

    let local = unsafe {
        let mut now: time_t = 0;
        time(&mut now);
        let mut result: tm = std::mem::zeroed();
        localtime_r(&now, &mut result);
        result
    };

    let year = (local.tm_year + 1900) as u32;
    let month = (local.tm_mon + 1) as u8;
    let day = local.tm_mday as u8;
    let weekday = local.tm_wday as u8;
    let hour = local.tm_hour as u8;
    let min = local.tm_min as u8;
    let sec = local.tm_sec as u8;

    log::info!(
        "NTP: local time is {year:04}-{month:02}-{day:02} {hour:02}:{min:02}:{sec:02}, writing to RTC"
    );

    // BM8563 register map, written starting at 0x02: sec, min, hour, day,
    // weekday, month, year - all BCD.
    let regs = [
        0x02,
        dec_to_bcd(sec),
        dec_to_bcd(min),
        dec_to_bcd(hour),
        dec_to_bcd(day),
        dec_to_bcd(weekday),
        dec_to_bcd(month),
        dec_to_bcd((year - 2000) as u8),
    ];
    if let Err(e) = i2c.write(ADDR_RTC_BM8563, &regs, 1000) {
        log::warn!("NTP: failed to write synced time to RTC: {e:?}");
    } else {
        log::info!("NTP: RTC updated from NTP");
    }
}
