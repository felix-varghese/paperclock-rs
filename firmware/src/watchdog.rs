use esp_idf_svc::sys::{
    esp_task_wdt_add, esp_task_wdt_config_t, esp_task_wdt_init, esp_task_wdt_reconfigure,
    esp_task_wdt_reset, ESP_ERR_INVALID_STATE, ESP_OK,
};

/// Needs headroom above the longest legitimate gap between feeds - the
/// stay-awake delay (up to SLEEP_SECONDS) and the network refresh, which
/// includes NTP's own 15s timeout.
const TIMEOUT_MS: u32 = 90_000;

/// Subscribes the calling task to the task watchdog. Best-effort: a
/// watchdog that fails to set up shouldn't stop the clock from running.
pub fn enable_for_current_task() {
    let config = esp_task_wdt_config_t {
        timeout_ms: TIMEOUT_MS,
        idle_core_mask: 0b11, // both cores' idle tasks
        trigger_panic: true,
    };
    unsafe {
        let init_result = esp_task_wdt_init(&config);
        if init_result != ESP_OK {
            if init_result != ESP_ERR_INVALID_STATE {
                log::warn!("esp_task_wdt_init failed: {init_result}");
                return;
            }
            // ESP-IDF's startup usually initializes it for the idle tasks
            // already; apply our timeout to that instance.
            let reconfig_result = esp_task_wdt_reconfigure(&config);
            if reconfig_result != ESP_OK {
                log::warn!("esp_task_wdt_reconfigure failed: {reconfig_result}");
            }
        }
        let add_result = esp_task_wdt_add(core::ptr::null_mut());
        if add_result != ESP_OK {
            log::warn!("esp_task_wdt_add failed: {add_result}");
            return;
        }
    }
    log::info!("Task watchdog enabled ({TIMEOUT_MS}ms timeout)");
}

/// Resets the watchdog. Call at least once per `TIMEOUT_MS` from points
/// that only run when work is actually progressing.
pub fn feed() {
    unsafe {
        esp_task_wdt_reset();
    }
}
