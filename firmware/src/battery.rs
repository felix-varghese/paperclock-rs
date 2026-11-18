use esp_idf_svc::hal::adc::attenuation;
use esp_idf_svc::hal::adc::oneshot::config::{AdcChannelConfig, Calibration};
use esp_idf_svc::hal::adc::oneshot::{AdcChannelDriver, AdcDriver};
use esp_idf_svc::hal::adc::ADC1;
use esp_idf_svc::hal::gpio::Gpio3;

const DIVIDER_RATIO: f32 = 2.0;

// Matches M5Unified's Power_Class.cpp verbatim: `(mv - 3300) * 100 /
// (float)(4150 - 3350)`. The asymmetric 3300/3350 is an upstream quirk,
// kept so the percentage agrees with what M5's own firmware shows.
const VOLTAGE_EMPTY_MV: f32 = 3300.0;
const VOLTAGE_RANGE_MV: f32 = 4150.0 - 3350.0;

pub struct Battery {
    pub voltage_mv: u32,
    pub percent: u8,
}

// RTC memory survives deep sleep and resets only on power-on. 0 doubles
// as "no reading yet" - the board can't be running at 0mV.
#[link_section = ".rtc.data"]
static mut LAST_BATTERY_MV: u32 = 0;

/// Change in battery voltage since the last call, across deep sleep.
/// `None` on the first reading after power-on. Positive = charged.
pub fn track_usage(mv: u32) -> Option<i32> {
    unsafe {
        let ptr = core::ptr::addr_of_mut!(LAST_BATTERY_MV);
        let prev = *ptr;
        *ptr = mv;
        if prev == 0 {
            None
        } else {
            Some(mv as i32 - prev as i32)
        }
    }
}

/// Sets up the ADC once and returns a closure that reads it. `impl FnMut`
/// avoids spelling out AdcChannelDriver's generics at the call site.
pub fn setup(
    adc1: ADC1<'static>,
    pin: Gpio3<'static>,
) -> anyhow::Result<impl FnMut() -> anyhow::Result<Battery>> {
    let adc = AdcDriver::new(adc1)?;
    let config = AdcChannelConfig {
        // 4.2V battery / 2.0 divider = 2.1V at the pin; max attenuation
        // covers the full 0-3.3V input range.
        attenuation: attenuation::DB_12,
        // Defaults to Calibration::None, which converts counts to mV with
        // a fixed formula and no correction for this chip's own ADC
        // nonlinearity - readings come out hundreds of mV off. Curve uses
        // the factory eFuse calibration data.
        calibration: Calibration::Curve,
        ..Default::default()
    };
    let mut channel = AdcChannelDriver::new(adc, pin, &config)?;

    Ok(move || {
        let pin_mv = channel.read()? as f32;
        let battery_mv = pin_mv * DIVIDER_RATIO;
        let percent = (((battery_mv - VOLTAGE_EMPTY_MV) / VOLTAGE_RANGE_MV) * 100.0)
            .clamp(0.0, 100.0) as u8;

        log::info!("Battery: {battery_mv:.0}mV ({percent}%)");

        Ok(Battery {
            voltage_mv: battery_mv as u32,
            percent,
        })
    })
}
