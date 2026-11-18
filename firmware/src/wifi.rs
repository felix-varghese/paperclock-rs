use crate::sd::Config as SdConfig;
use esp_idf_svc::eventloop::EspSystemEventLoop;
use esp_idf_svc::hal::modem::Modem;
use esp_idf_svc::nvs::EspDefaultNvsPartition;
use esp_idf_svc::wifi::{AuthMethod, BlockingWifi, ClientConfiguration, Configuration, EspWifi};

pub fn init(
    modem: Modem<'static>,
    sys_loop: EspSystemEventLoop,
    nvs: EspDefaultNvsPartition,
) -> anyhow::Result<BlockingWifi<EspWifi<'static>>> {
    let mut wifi = BlockingWifi::wrap(
        EspWifi::new(modem, sys_loop.clone(), Some(nvs))?,
        sys_loop,
    )?;
    // Without an explicit Client config before start(), EspWifi comes up
    // in softAP mode and scan() fails with ESP_FAIL.
    wifi.set_configuration(&Configuration::Client(ClientConfiguration::default()))?;
    wifi.start()?;
    log::info!("WiFi radio started in STA mode");
    Ok(wifi)
}

/// Scans and logs every visible network. Doesn't require credentials.
pub fn scan_and_log(wifi: &mut BlockingWifi<EspWifi<'static>>) -> anyhow::Result<()> {
    log::info!("Scanning for WiFi networks...");
    let networks = wifi.scan()?;
    if networks.is_empty() {
        log::warn!("No WiFi networks found in scan");
    }
    for ap in &networks {
        log::info!(
            "  SSID={:<32} RSSI={:>4} channel={:<3} auth={:?}",
            ap.ssid.as_str(),
            ap.signal_strength,
            ap.channel,
            ap.auth_method
        );
    }
    Ok(())
}

/// Outlives the WiFi driver itself, which is dropped after the refresh to
/// save power, so the display can still show the last known status.
pub struct WifiStatus {
    pub connected: bool,
    /// Signal strength in dBm at connect time; `None` if not connected.
    pub rssi: Option<i32>,
}

/// Connects using credentials from the SD card. Returns
/// `connected: false` without trying if there aren't any.
pub fn connect_if_configured(
    wifi: &mut BlockingWifi<EspWifi<'static>>,
    creds: Option<&SdConfig>,
) -> anyhow::Result<WifiStatus> {
    let Some(creds) = creds else {
        log::warn!("No WiFi credentials from SD card - skipping WiFi connect (scan-only)");
        return Ok(WifiStatus { connected: false, rssi: None });
    };

    // Never log creds.password.
    log::info!("Connecting to SSID '{}'...", creds.ssid);
    let config = Configuration::Client(ClientConfiguration {
        ssid: creds
            .ssid
            .as_str()
            .try_into()
            .map_err(|_| anyhow::anyhow!("SSID too long for heapless string"))?,
        bssid: None,
        auth_method: AuthMethod::WPA2Personal,
        password: creds
            .password
            .as_str()
            .try_into()
            .map_err(|_| anyhow::anyhow!("password too long for heapless string"))?,
        channel: None,
        ..Default::default()
    });
    wifi.set_configuration(&config)?;
    wifi.connect()?;
    log::info!("WiFi connected");
    wifi.wait_netif_up()?;

    let ip_info = wifi.wifi().sta_netif().get_ip_info()?;
    log::info!("WiFi DHCP info: {ip_info:?}");

    let rssi = wifi.wifi().get_rssi().ok();
    log::info!("WiFi RSSI: {rssi:?} dBm");

    Ok(WifiStatus { connected: true, rssi })
}
