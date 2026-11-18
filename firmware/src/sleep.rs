/// True if this boot came from the deep-sleep timer rather than a
/// power-on or reset. Deep sleep restarts from the top of main(), so this
/// is what distinguishes a wake from a cold boot.
pub fn is_timer_wake() -> bool {
    let cause = unsafe { esp_idf_svc::sys::esp_sleep_get_wakeup_cause() };
    cause == esp_idf_svc::sys::esp_sleep_source_t_ESP_SLEEP_WAKEUP_TIMER
}

// `.rtc.data` (ESP-IDF's RTC_DATA_ATTR) stays powered across deep sleep
// and resets to its declared value only on power-on, which makes this a
// count of wakes since the last cold boot.
#[link_section = ".rtc.data"]
static mut WAKE_COUNT: u32 = 0;

/// Increments and returns the wake counter. 0 on the first call after a
/// cold boot.
pub fn next_wake_count() -> u32 {
    unsafe {
        let ptr = core::ptr::addr_of_mut!(WAKE_COUNT);
        let count = *ptr;
        *ptr = count.wrapping_add(1);
        count
    }
}

pub fn wakeup_cause_str() -> &'static str {
    let cause = unsafe { esp_idf_svc::sys::esp_sleep_get_wakeup_cause() };
    match cause {
        esp_idf_svc::sys::esp_sleep_source_t_ESP_SLEEP_WAKEUP_UNDEFINED => "cold boot / reset",
        esp_idf_svc::sys::esp_sleep_source_t_ESP_SLEEP_WAKEUP_TIMER => "timer (deep sleep wake)",
        esp_idf_svc::sys::esp_sleep_source_t_ESP_SLEEP_WAKEUP_EXT0 => "ext0 (RTC GPIO)",
        esp_idf_svc::sys::esp_sleep_source_t_ESP_SLEEP_WAKEUP_EXT1 => "ext1 (RTC GPIO)",
        other => {
            log::warn!("unrecognized wakeup cause: {other}");
            "unknown"
        }
    }
}

/// Enters deep sleep for `seconds`. Never returns - the chip resets and
/// re-runs main() on wake.
pub fn enter_deep_sleep_for(seconds: u64) -> ! {
    log::info!("Entering deep sleep for {seconds}s...");
    unsafe {
        esp_idf_svc::sys::esp_sleep_enable_timer_wakeup(seconds * 1_000_000);
        esp_idf_svc::sys::esp_deep_sleep_start();
    }
}
