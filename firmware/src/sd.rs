use esp_idf_svc::fs::fatfs::Fatfs;
use esp_idf_svc::hal::gpio::{AnyInputPin, Gpio38, Gpio39, Gpio40};
use esp_idf_svc::hal::sd::spi::SdSpiHostDriver;
use esp_idf_svc::hal::sd::{SdCardConfiguration, SdCardDriver};
use esp_idf_svc::hal::spi::{config::DriverConfig, Dma, SpiDriver, SPI2};
use esp_idf_svc::sys::{f_close, f_open, f_read, FRESULT_FR_OK, FA_READ, FIL};
use serde::Deserialize;
use std::ffi::CString;

#[derive(Deserialize)]
pub struct Config {
    pub ssid: String,
    pub password: String,
    /// Falls back to IP geolocation if either is missing.
    #[serde(default)]
    pub lat: Option<f64>,
    #[serde(default)]
    pub lon: Option<f64>,
    /// Display name for the location line. Only used alongside a manual
    /// lat/lon; the geoip path returns its own city name.
    #[serde(default)]
    pub city: Option<String>,
}

// Deliberately does not print `password`.
impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("ssid", &self.ssid)
            .field("password", &"<redacted>")
            .field("lat", &self.lat)
            .field("lon", &self.lon)
            .field("city", &self.city)
            .finish()
    }
}

const CONFIG_FILE_MAX_BYTES: usize = 4096;

/// Mounts the SD card over SPI and reads `wifi_config.json` from its root.
/// Returns `None` if the card, mount or file isn't there or doesn't parse
/// - the clock still boots and shows the time without a card. Logs the
/// reason, never the file contents.
pub fn read_config(
    spi2: SPI2<'static>,
    sclk: Gpio39<'static>,
    mosi: Gpio38<'static>,
    miso: Gpio40<'static>,
    cs: esp_idf_svc::hal::gpio::Gpio47<'static>,
) -> Option<Config> {
    // The default transfer size is FIFO-sized and too small for SD block
    // reads ("txdata transfer > host maximum"). 4096 matches FIL's buffer.
    let dma_config = DriverConfig::new().dma(Dma::Auto(4096));
    let spi_driver = match SpiDriver::new(spi2, sclk, mosi, Some(miso), &dma_config) {
        Ok(d) => d,
        Err(e) => {
            log::warn!("SD: SPI bus init failed: {e:?}");
            return None;
        }
    };

    let host_driver = match SdSpiHostDriver::new(
        spi_driver,
        Some(cs),
        None::<AnyInputPin>,
        None::<AnyInputPin>,
        None::<AnyInputPin>,
        None,
    ) {
        Ok(d) => d,
        Err(e) => {
            log::warn!("SD: SPI host driver init failed: {e:?}");
            return None;
        }
    };

    let card_driver = match SdCardDriver::new_spi(host_driver, &SdCardConfiguration::new()) {
        Ok(d) => d,
        Err(e) => {
            log::warn!("SD: card init failed (no card inserted?): {e:?}");
            return None;
        }
    };

    let mut fatfs = match Fatfs::new_sdcard(0, card_driver) {
        Ok(f) => f,
        Err(e) => {
            log::warn!("SD: FAT diskio registration failed: {e:?}");
            return None;
        }
    };

    let mounted = match fatfs.mount() {
        Ok(m) => m,
        Err(e) => {
            log::warn!("SD: FAT mount failed: {e:?}");
            return None;
        }
    };
    log::info!("SD card mounted");

    let json = read_file_0drive("wifi_config.json")?;
    drop(mounted); // unmount before we're done with the SD card

    match serde_json::from_str::<Config>(&json) {
        Ok(creds) => {
            log::info!("SD: parsed wifi_config.json: {creds:?}");
            Some(creds)
        }
        Err(e) => {
            // serde_json's error text can echo fragments of the input, and
            // the input contains the password - log position only.
            log::warn!(
                "SD: wifi_config.json found but not valid JSON with \
                 \"ssid\"/\"password\" string fields (line {}, col {})",
                e.line(),
                e.column()
            );
            None
        }
    }
}

/// Reads a file from the mounted "0:" drive via the raw FatFs API. Going
/// through ESP-IDF's VFS layer instead would mean hand-building the
/// version-gated `sdmmc_host_t`; f_open/f_read are stable across versions.
fn read_file_0drive(name: &str) -> Option<String> {
    let path = CString::new(format!("0:/{name}")).ok()?;
    let mut file = Box::new(FIL::default());

    let res = unsafe { f_open(&mut *file, path.as_ptr(), FA_READ as u8) };
    if res != FRESULT_FR_OK {
        log::warn!("SD: f_open(\"{name}\") failed, FRESULT={res} (file missing?)");
        return None;
    }

    let mut buf = vec![0u8; CONFIG_FILE_MAX_BYTES];
    let mut bytes_read: u32 = 0;
    let res = unsafe {
        f_read(
            &mut *file,
            buf.as_mut_ptr() as *mut core::ffi::c_void,
            buf.len() as u32,
            &mut bytes_read,
        )
    };
    unsafe { f_close(&mut *file) };

    if res != FRESULT_FR_OK {
        log::warn!("SD: f_read(\"{name}\") failed, FRESULT={res}");
        return None;
    }

    buf.truncate(bytes_read as usize);
    String::from_utf8(buf).ok()
}
