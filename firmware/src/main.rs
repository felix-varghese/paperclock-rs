use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::delay::FreeRtos;
use esp_idf_svc::hal::gpio::{Pull, PinDriver};
use esp_idf_svc::hal::i2c::{I2cConfig, I2cDriver};
use esp_idf_svc::hal::peripherals::Peripherals;
use esp_idf_svc::hal::units::FromValueType;
use esp_idf_svc::nvs::EspDefaultNvsPartition;

mod battery;
mod cache;
mod display;
mod geoip;
mod ntp;
mod sd;
mod sleep;
mod watchdog;
mod weather;
mod wifi;

// Known I2C addresses on the shared PaperS3 bus (G41 = SDA, G42 = SCL).
const ADDR_RTC_BM8563: u8 = 0x51;
const ADDR_GYRO_BMI270: u8 = 0x68;

fn bcd_to_dec(bcd: u8) -> u8 {
    (bcd & 0x0F) + ((bcd >> 4) * 10)
}

// Index matches POSIX tm_wday (0 = Sunday), which is what ntp.rs writes
// into the BM8563's weekday register.
const WEEKDAY_NAMES: [&str; 7] =
    ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

const SLEEP_SECONDS: u64 = 60;

/// Weather is worth refreshing often; NTP and geolocation aren't. Both run
/// on cold boot, then every N wakes.
const WEATHER_REFRESH_EVERY_N_WAKES: u32 = 10; // ~10 minutes
const NETWORK_REFRESH_EVERY_N_WAKES: u32 = 60; // ~1 hour

/// Forces deep sleep even while charging. Set true to exercise the
/// wake/reconnect/refresh/sleep cycle on bench power; false otherwise.
const FORCE_DEEP_SLEEP: bool = false;

// Widget positions. 480 = panel width (960) / 2.
const CENTER_X: i32 = 480;
const CLOCK_Y: i32 = 330;
const WEATHER_Y: i32 = 485;
const WEATHER_GAP: i32 = 12;
const WIFI_ICON_X: i32 = 20;
const WIFI_ICON_Y: i32 = 36;
const CITY_X: i32 = 20;
const CITY_Y: i32 = 510;
const BATTERY_ICON_X: i32 = 790;
const BATTERY_ICON_Y: i32 = 498;
const BOLT_GLYPH: &str = "\u{f0e7}";
const WIFI_GLYPH: &str = "\u{f1eb}";

/// Everything drawn on screen for one wake. A partial refresh draws two of
/// these: the previous frame into `back_fb`, then the current one into
/// `front_fb` - see `RefreshMode` in display.rs.
struct FrameValues<'a> {
    date_str: &'a str,
    time_str: &'a str,
    weather_icon: display::WeatherIcon,
    weather_str: &'a str,
    wifi_str: &'a str,
    city_str: &'a str,
    battery_percent: u8,
    battery_str: &'a str,
    is_charging: bool,
}

/// Draws one complete frame into `target`. The only place layout is
/// expressed as drawing calls.
fn draw_frame(display: &mut display::Display, target: display::FbTarget, v: &FrameValues) {
    let bounds = display.measure_text(display::Font::Date, v.date_str);
    display.draw_text_to(target, display::Font::Date, v.date_str, CENTER_X - bounds.width / 2, 85);

    let bounds = display.measure_text(display::Font::Large, v.time_str);
    display.draw_text_to(target, display::Font::Large, v.time_str, CENTER_X - bounds.width / 2, CLOCK_Y);

    // Weather: icon + temperature side by side, positioned as one pair.
    let icon_bounds = display.measure_weather_icon(v.weather_icon);
    let temp_bounds = display.measure_text(display::Font::Small, v.weather_str);
    let pair_left = CENTER_X - (icon_bounds.width + WEATHER_GAP + temp_bounds.width) / 2;
    display.draw_weather_icon_to(target, v.weather_icon, pair_left, WEATHER_Y);
    display.draw_text_to(
        target,
        display::Font::Small,
        v.weather_str,
        pair_left + icon_bounds.width + WEATHER_GAP,
        WEATHER_Y,
    );

    // Corners: WiFi top-left, city bottom-left, battery bottom-right.
    display.draw_text_to(target, display::Font::WifiIcon, WIFI_GLYPH, WIFI_ICON_X, WIFI_ICON_Y);
    let wifi_icon_bounds = display.measure_text(display::Font::WifiIcon, WIFI_GLYPH);
    display.draw_text_to(
        target,
        display::Font::Micro,
        v.wifi_str,
        WIFI_ICON_X + wifi_icon_bounds.width + 6,
        WIFI_ICON_Y,
    );

    display.draw_text_to(target, display::Font::Tiny, v.city_str, CITY_X, CITY_Y);

    display.draw_battery_icon_to(target, BATTERY_ICON_X, BATTERY_ICON_Y, v.battery_percent);
    let battery_text_x = BATTERY_ICON_X + display::Display::BATTERY_ICON_WIDTH + 8;
    let battery_text_y = BATTERY_ICON_Y + display::Display::BATTERY_ICON_HEIGHT + 4;
    if v.is_charging {
        let bounds = display.measure_text(display::Font::ChargeIcon, BOLT_GLYPH);
        display.draw_text_to(target, display::Font::ChargeIcon, BOLT_GLYPH, BATTERY_ICON_X - 6 - bounds.width, battery_text_y);
    }
    display.draw_text_to(target, display::Font::Tiny, v.battery_str, battery_text_x, battery_text_y);
}

fn main() -> anyhow::Result<()> {
    esp_idf_svc::sys::link_patches();
    esp_idf_svc::log::EspLogger::initialize_default();
    watchdog::enable_for_current_task();

    let is_timer_wake = sleep::is_timer_wake();
    let wake_count = sleep::next_wake_count();
    let needs_weather_refresh = !is_timer_wake || wake_count % WEATHER_REFRESH_EVERY_N_WAKES == 0;
    let needs_full_network_refresh =
        !is_timer_wake || wake_count % NETWORK_REFRESH_EVERY_N_WAKES == 0;
    let needs_wifi = needs_weather_refresh || needs_full_network_refresh;
    log::info!(
        "paperclock-rs: booting (wakeup cause: {}, wake_count={wake_count}, needs_weather_refresh={needs_weather_refresh}, needs_full_network_refresh={needs_full_network_refresh})",
        sleep::wakeup_cause_str()
    );

    let peripherals = Peripherals::take()?;
    let i2c_config = I2cConfig::new().baudrate(100u32.kHz().into());
    let mut i2c = I2cDriver::new(
        peripherals.i2c0,
        peripherals.pins.gpio41, // SDA
        peripherals.pins.gpio42, // SCL
        &i2c_config,
    )?;

    // M5Unified's Power_Class.cpp: M5PaperS3_CHG_STAT_PIN = GPIO4, digital
    // read from the battery charger IC's STAT output, active-low
    // (`gpio_in(CHG_STAT_PIN) == false` means charging).
    let chg_stat = PinDriver::input(peripherals.pins.gpio4, Pull::Up)?;

    // Seeded from RTC memory so wakes that skip the network refresh below
    // still have data to draw - see cache.rs.
    let cached = cache::load();
    let mut weather = cached.weather;
    let mut city = cached.city;
    let mut wifi_status = cached.wifi_status;
    // These only change in the refresh block below, so their current
    // values are what's on screen from last wake - needed to reconstruct
    // the previous frame.
    let old_weather = weather;
    let old_city = city.clone();
    let old_wifi_status = wifi_status.as_ref().map(|s| wifi::WifiStatus { connected: s.connected, rssi: s.rssi });

    // Diagnostics on cold boot only - a timer wake should do nothing but
    // read the RTC and draw.
    if !is_timer_wake {
        unsafe {
            let free_internal =
                esp_idf_svc::sys::heap_caps_get_free_size(esp_idf_svc::sys::MALLOC_CAP_INTERNAL);
            let free_spiram =
                esp_idf_svc::sys::heap_caps_get_free_size(esp_idf_svc::sys::MALLOC_CAP_SPIRAM);
            log::info!(
                "heap: internal_free={free_internal} bytes, spiram_free={free_spiram} bytes ({} MB)",
                free_spiram / (1024 * 1024)
            );
            if free_spiram == 0 {
                log::warn!(
                    "spiram_free is 0 - octal PSRAM did not initialize; check CONFIG_SPIRAM_MODE_OCT"
                );
            }
        }

        log::info!("Scanning I2C bus (G41=SDA, G42=SCL)...");
        let mut found = Vec::new();
        for addr in 0x08u8..0x78u8 {
            // A zero-length write only completes if a device ACKs its address.
            if i2c.write(addr, &[], 100).is_ok() {
                log::info!("  found device at 0x{addr:02X}");
                found.push(addr);
            }
            FreeRtos::delay_ms(2);
        }
        if found.is_empty() {
            log::warn!("No I2C devices responded - check G41/G42 wiring/pin mapping");
        }
        if !found.contains(&ADDR_RTC_BM8563) {
            log::warn!("BM8563 RTC (0x{ADDR_RTC_BM8563:02X}) not found on bus");
        }
        if found.contains(&ADDR_GYRO_BMI270) {
            log::info!("BMI270 gyro (0x{ADDR_GYRO_BMI270:02X}) present");
        }
    }

    // WiFi connect, NTP sync, geolocation and weather fetch. WiFi doesn't
    // survive deep sleep, so this is a full reconnect every time. Runs at
    // most once per boot; `next_wake_due_for_refresh` below forces a
    // reboot when the next wake is due, rather than making this
    // re-entrant.
    if needs_wifi {
        watchdog::feed();
        // Best-effort; a missing or unreadable card just means no
        // credentials. See sd.rs - never logs the password.
        let config = sd::read_config(
            peripherals.spi2,
            peripherals.pins.gpio39,
            peripherals.pins.gpio38,
            peripherals.pins.gpio40,
            peripherals.pins.gpio47,
        );

        // A closure rather than `?` directly: a transient WiFi failure
        // must not propagate out of main(), because a wake that never
        // reaches the sleep call at the bottom of the loop never wakes
        // again. Draw with cached data and retry next wake instead.
        let network_result: anyhow::Result<()> = (|| {
            let sys_loop = EspSystemEventLoop::take()?;
            let nvs = EspDefaultNvsPartition::take()?;
            let mut wifi_driver = wifi::init(peripherals.modem, sys_loop, nvs)?;
            // Diagnostic only - the connect below goes straight for the
            // configured SSID. Costs ~2.5s of radio time, so cold boot only.
            if !is_timer_wake {
                wifi::scan_and_log(&mut wifi_driver)?;
            }
            let status = wifi::connect_if_configured(&mut wifi_driver, config.as_ref())?;
            let connected = status.connected;
            wifi_status = Some(status);

            if connected {
                if needs_full_network_refresh {
                    ntp::sync_time_to_rtc(&mut i2c);

                    let configured_location = config.as_ref().and_then(|c| c.lat.zip(c.lon));
                    let (lat_lon, resolved_city) = match configured_location {
                        Some((lat, lon)) => {
                            (Some((lat, lon)), config.as_ref().and_then(|c| c.city.clone()))
                        }
                        None => {
                            log::info!("no lat/lon in wifi_config.json, trying IP geolocation");
                            match geoip::lookup() {
                                Some(g) => (Some((g.lat, g.lon)), g.city),
                                None => (None, None),
                            }
                        }
                    };
                    city = resolved_city;
                    if let Some((lat, lon)) = lat_lon {
                        cache::store_location(lat, lon);
                    }
                }

                if needs_weather_refresh {
                    match cache::load_location() {
                        Some((lat, lon)) => weather = weather::fetch(lat, lon),
                        None => log::warn!(
                            "no location available yet (config or IP geolocation), skipping weather"
                        ),
                    }
                }
            }
            Ok(())
        })();

        if let Err(e) = network_result {
            log::warn!("Network refresh failed, continuing with cached/local data: {e:?}");
        }

        // Persist whatever we got, even partial, for the wakes that skip
        // this block.
        if let Some(status) = &wifi_status {
            cache::store(weather.as_ref(), city.as_deref(), status);
        }
        watchdog::feed();
    }

    let mut display = display::Display::init();
    let mut read_battery = battery::setup(peripherals.adc1, peripherals.pins.gpio3)?;
    // Distinct from `is_timer_wake`: a deep-sleep wake is always a fresh
    // boot, but the stay-awake path below loops many times within one.
    let mut is_first_draw = true;
    // `wake_count` only advances on a real reboot, so while staying awake
    // it would freeze - and freeze the refresh cadence with it. Counting
    // loop passes keeps the prediction below honest.
    let mut loop_iters: u32 = 0;

    loop {
        watchdog::feed();

        let is_charging = chg_stat.is_low();
        log::info!("CHG_STAT (G4): charging={is_charging}");

        // BM8563 register map: 0x02 sec, 0x03 min, 0x04 hour, 0x05 day,
        // 0x06 weekday, 0x07 month, 0x08 year - all BCD.
        let mut regs = [0u8; 7];
        let (time_str, date_str, minute) = match i2c.write_read(ADDR_RTC_BM8563, &[0x02], &mut regs, 1000) {
            Ok(()) => {
                let sec = bcd_to_dec(regs[0] & 0x7F);
                let min = bcd_to_dec(regs[1] & 0x7F);
                let hour = bcd_to_dec(regs[2] & 0x3F);
                let day = bcd_to_dec(regs[3] & 0x3F);
                // Weekday register is 0-6, not BCD.
                let weekday = (regs[4] & 0x07) as usize;
                let month = bcd_to_dec(regs[5] & 0x1F);
                let year = 2000 + bcd_to_dec(regs[6]) as u32;
                log::info!(
                    "BM8563 RTC time: {year:04}-{month:02}-{day:02} {hour:02}:{min:02}:{sec:02} (weekday={weekday})"
                );
                let weekday_name = WEEKDAY_NAMES.get(weekday).copied().unwrap_or("---");
                (
                    format!("{hour:02}:{min:02}"),
                    format!("{weekday_name}, {day:02}-{month:02}-{year:04}"),
                    Some(min),
                )
            }
            Err(e) => {
                log::warn!("RTC read failed: {e:?}");
                (String::from("--:--"), String::from("---, --------"), None)
            }
        };
        watchdog::feed();

        let battery_percent = match read_battery() {
            Ok(b) => {
                match battery::track_usage(b.voltage_mv) {
                    Some(delta_mv) => log::info!(
                        "Battery usage: {delta_mv:+}mV since last wake ({}mV now)",
                        b.voltage_mv
                    ),
                    None => log::info!("Battery usage: first reading since power-on/reset, no delta yet"),
                }
                Some(b.percent)
            }
            Err(e) => {
                log::warn!("Battery read failed: {e:?}");
                None
            }
        };
        let battery_str = match battery_percent {
            Some(p) => format!("{p}%"),
            None => String::from("--%"),
        };

        let weather_str = match &weather {
            Some(w) => format!("{:.1}°C", w.temperature_c),
            None => String::from("--.-°C"),
        };
        let old_weather_str = match &old_weather {
            Some(w) => format!("{:.1}°C", w.temperature_c),
            None => String::from("--.-°C"),
        };

        let city_str = city.as_deref().unwrap_or("--").to_string();
        let old_city_str = old_city.as_deref().unwrap_or("--").to_string();

        fn format_wifi_str(status: &Option<wifi::WifiStatus>) -> String {
            match status {
                Some(s) if s.connected => match s.rssi {
                    Some(r) => format!("{r}dBm"),
                    None => "on".to_string(),
                },
                Some(_) => "--".to_string(),
                None => "--".to_string(),
            }
        }
        let wifi_str = format_wifi_str(&wifi_status);
        let old_wifi_str = format_wifi_str(&old_wifi_status);

        log::info!(
            "Drawing to e-ink panel: {time_str} {date_str} {city_str} {battery_str} {weather_str} WiFi={wifi_str}"
        );

        let icon = weather
            .as_ref()
            .map(|w| display::WeatherIcon::from_wmo_code(w.weather_code))
            .unwrap_or(display::WeatherIcon::Cloud);
        let old_icon = old_weather
            .as_ref()
            .map(|w| display::WeatherIcon::from_wmo_code(w.weather_code))
            .unwrap_or(display::WeatherIcon::Cloud);

        let current = FrameValues {
            date_str: &date_str,
            time_str: &time_str,
            weather_icon: icon,
            weather_str: &weather_str,
            wifi_str: &wifi_str,
            city_str: &city_str,
            battery_percent: battery_percent.unwrap_or(0),
            battery_str: &battery_str,
            is_charging,
        };

        // Flash on the first draw of a boot, when there's no previous
        // frame to reconstruct, and hourly to clear accumulated ghosting.
        let is_full_refresh = (!is_timer_wake && is_first_draw) || minute == Some(0);
        is_first_draw = false;

        if is_full_refresh {
            display.full_clear();
            draw_frame(&mut display, display::FbTarget::Front, &current);
            display.update(display::RefreshMode::Full);
        } else {
            // Reconstruct the previous frame into back_fb so the diff has
            // an accurate baseline, then draw the current one into
            // front_fb - see `RefreshMode` in display.rs.
            let prev_date = cache::load_prev_date();
            let prev_time = cache::load_prev_time();
            let prev_battery_str = cache::load_prev_battery();
            let previous = FrameValues {
                date_str: &prev_date,
                time_str: &prev_time,
                weather_icon: old_icon,
                weather_str: &old_weather_str,
                wifi_str: &old_wifi_str,
                city_str: &old_city_str,
                battery_percent: cache::load_prev_battery_percent(),
                battery_str: &prev_battery_str,
                is_charging: cache::load_prev_charging(),
            };
            display.clear_target(display::FbTarget::Back);
            draw_frame(&mut display, display::FbTarget::Back, &previous);
            display.clear_target(display::FbTarget::Front);
            draw_frame(&mut display, display::FbTarget::Front, &current);
            display.update(display::RefreshMode::Fast);
        }

        cache::store_date(&date_str);
        cache::store_time(&time_str);
        cache::store_battery(&battery_str);
        cache::store_battery_percent(battery_percent.unwrap_or(0));
        cache::store_charging(is_charging);
        watchdog::feed();

        // Wait only long enough to reach the next :00. A flat
        // SLEEP_SECONDS would drift later every iteration, since a
        // WiFi-refresh wake takes several seconds longer than a plain one.
        let mut sec_reg = [0u8; 1];
        let wait_seconds = match i2c.write_read(ADDR_RTC_BM8563, &[0x02], &mut sec_reg, 1000) {
            Ok(()) => {
                let sec = bcd_to_dec(sec_reg[0] & 0x7F) as u64;
                if sec == 0 { SLEEP_SECONDS } else { SLEEP_SECONDS - sec.min(SLEEP_SECONDS - 1) }
            }
            Err(e) => {
                log::warn!("RTC read before waiting failed, using flat {SLEEP_SECONDS}s: {e:?}");
                SLEEP_SECONDS
            }
        };

        loop_iters += 1;
        let next_wake_count = wake_count + loop_iters;
        let next_wake_due_for_refresh = next_wake_count % WEATHER_REFRESH_EVERY_N_WAKES == 0
            || next_wake_count % NETWORK_REFRESH_EVERY_N_WAKES == 0;

        // Deep sleep powers down USB, so a board on a charger would
        // otherwise be unreachable for ~58 of every 60 seconds. Keyed on
        // CHG_STAT rather than USB detection because a false positive here
        // means burning battery instead of sleeping.
        if is_charging && !FORCE_DEEP_SLEEP && !next_wake_due_for_refresh {
            log::info!("Charging - staying awake, redrawing again in {wait_seconds}s");
            FreeRtos::delay_ms((wait_seconds * 1000) as u32);
        } else {
            sleep::enter_deep_sleep_for(wait_seconds);
        }
    }
}
