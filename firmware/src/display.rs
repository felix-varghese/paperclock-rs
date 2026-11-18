use esp_idf_svc::sys::epdiy;
use std::ffi::CString;

#[derive(Clone, Copy)]
pub enum Font {
    /// Inter Bold 132pt - the clock. Inter's numerals are tabular, so the
    /// digits don't shift width as the time changes.
    Large,
    /// Literata Medium 28pt - weather/temperature.
    Small,
    /// Literata Medium 24pt - the date line.
    Date,
    /// Literata Medium 20pt - battery and city text.
    Tiny,
    /// Literata Medium 16pt - the WiFi corner.
    Micro,
    /// Weather Icons 64pt, addressed by Private Use Area codepoint.
    Icon,
    /// Font Awesome 6 Free Solid 22pt - the wifi glyph (U+F1EB).
    WifiIcon,
    /// Font Awesome 6 Free Solid 18pt - the bolt glyph (U+F0E7), drawn
    /// next to the battery percentage while charging.
    ChargeIcon,
}

impl Font {
    fn as_epd_font(self) -> *const epdiy::EpdFont {
        unsafe {
            match self {
                Font::Large => &epdiy::Inter_Bold_132 as *const _,
                Font::Small => &epdiy::Literata_Medium_28 as *const _,
                Font::Date => &epdiy::Literata_Medium_24 as *const _,
                Font::Tiny => &epdiy::Literata_Medium_20 as *const _,
                Font::Micro => &epdiy::Literata_Medium_16 as *const _,
                Font::Icon => &epdiy::WeatherIcons_64 as *const _,
                Font::WifiIcon => &epdiy::FA_Wifi_22 as *const _,
                Font::ChargeIcon => &epdiy::FA_Bolt_18 as *const _,
            }
        }
    }
}

/// Weather Icons glyphs, mapped from Open-Meteo's WMO weather code.
#[derive(Clone, Copy, PartialEq)]
pub enum WeatherIcon {
    Sun,
    MostlyClear,
    PartlyCloudy,
    Overcast,
    Cloud,
    Fog,
    Drizzle,
    Rain,
    RainShowers,
    Sleet,
    Snow,
    SnowWind,
    Thunderstorm,
    ThunderstormShowers,
    Hail,
}

impl WeatherIcon {
    /// WMO weather interpretation codes; see https://open-meteo.com/en/docs
    pub fn from_wmo_code(code: u32) -> Self {
        match code {
            0 => WeatherIcon::Sun,
            1 => WeatherIcon::MostlyClear,
            2 => WeatherIcon::PartlyCloudy,
            3 => WeatherIcon::Overcast,
            45 | 48 => WeatherIcon::Fog,
            51 | 53 | 55 => WeatherIcon::Drizzle,
            56 | 57 => WeatherIcon::Sleet, // freezing drizzle
            61 | 63 | 65 => WeatherIcon::Rain,
            66 | 67 => WeatherIcon::Sleet, // freezing rain
            71 | 73 | 75 | 77 => WeatherIcon::Snow,
            80..=82 => WeatherIcon::RainShowers,
            85 | 86 => WeatherIcon::SnowWind, // snow showers
            95 => WeatherIcon::Thunderstorm,
            96 => WeatherIcon::ThunderstormShowers, // thunderstorm, slight hail
            99 => WeatherIcon::Hail,                // thunderstorm, heavy hail
            _ => WeatherIcon::Cloud,      // anything unmapped
        }
    }

    fn glyph(self) -> char {
        match self {
            WeatherIcon::Sun => '\u{F00D}',                  // wi-day-sunny
            WeatherIcon::MostlyClear => '\u{F00C}',          // wi-day-sunny-overcast
            WeatherIcon::PartlyCloudy => '\u{F002}',         // wi-day-cloudy
            WeatherIcon::Overcast => '\u{F013}',             // wi-cloudy
            WeatherIcon::Cloud => '\u{F041}',                // wi-cloud
            WeatherIcon::Fog => '\u{F014}',                  // wi-fog
            WeatherIcon::Drizzle => '\u{F01C}',              // wi-sprinkle
            WeatherIcon::Rain => '\u{F019}',                 // wi-rain
            WeatherIcon::RainShowers => '\u{F01A}',          // wi-showers
            WeatherIcon::Sleet => '\u{F0B5}',                // wi-sleet
            WeatherIcon::Snow => '\u{F01B}',                 // wi-snow
            WeatherIcon::SnowWind => '\u{F064}',             // wi-snow-wind
            WeatherIcon::Thunderstorm => '\u{F01E}',         // wi-thunderstorm
            WeatherIcon::ThunderstormShowers => '\u{F01D}',  // wi-storm-showers
            WeatherIcon::Hail => '\u{F015}',                 // wi-hail
        }
    }
}

/// Which epdiy update mode to push a frame with.
///
/// `Fast` diffs `front_fb` (what was just drawn) against `back_fb`
/// (epdiy's record of what's on the panel) and pushes only the pixels that
/// differ. Both buffers are in PSRAM, which deep sleep does not retain, so
/// after a wake `back_fb` is blank while the panel - being bistable -
/// still shows the last frame. Diffing against a blank record skips pixels
/// that still have ink on them, leaving the old glyph behind.
///
/// The fix is to redraw the known previous frame into `back_fb` (from the
/// values in cache.rs) before drawing the new one into `front_fb`, so the
/// diff runs against reality. `main.rs` does this via `FbTarget::Back`.
///
/// Both draws must be preceded by `Display::clear_target`: drawing only
/// sets the pixels a glyph covers and never clears around it, so a buffer
/// drawn into twice keeps the older shape underneath.
#[derive(Clone, Copy)]
pub enum RefreshMode {
    /// MODE_GC16: flashing full-panel refresh. Used on the first draw of a
    /// boot, and hourly to clear the ghosting `Fast` leaves behind.
    Full,
    /// MODE_GL16: non-flashing. Requires a reconstructed `back_fb`.
    Fast,
}

impl RefreshMode {
    fn as_epd_mode(self) -> epdiy::EpdDrawMode {
        match self {
            RefreshMode::Full => epdiy::EpdDrawMode_MODE_GC16,
            RefreshMode::Fast => epdiy::EpdDrawMode_MODE_GL16,
        }
    }
}

/// Which framebuffer a draw call targets. `Front` is what to draw next;
/// `Back` is epdiy's record of what's on the panel - see `RefreshMode`.
#[derive(Clone, Copy)]
pub enum FbTarget {
    Front,
    Back,
}

const BLACK: u8 = 0x00;

/// A measured element's pixel width, for centering and side-by-side layout.
pub struct TextBounds {
    pub width: i32,
}

pub struct Display {
    state: epdiy::EpdiyHighlevelState,
}

impl Display {
    /// Battery icon footprint in px, for laying out adjacent text.
    pub const BATTERY_ICON_WIDTH: i32 = 40; // 36px body + 4px terminal nub
    pub const BATTERY_ICON_HEIGHT: i32 = 20;

    /// Brings up the panel driver and allocates the framebuffers in PSRAM.
    /// Doesn't touch the panel itself - that happens on `update`.
    pub fn init() -> Self {
        unsafe {
            epdiy::epd_init(
                &epdiy::epd_board_m5papers3 as *const _,
                &epdiy::ED047TC1 as *const _,
                epdiy::EpdInitOptions_EPD_OPTIONS_DEFAULT,
            );
            log::info!("epd_init done");

            // EPD_BUILTIN_WAVEFORM is `#define EPD_BUILTIN_WAVEFORM NULL` in C.
            let state = epdiy::epd_hl_init(std::ptr::null());
            log::info!("epd_hl_init done (framebuffers allocated in PSRAM)");

            Display { state }
        }
    }

    /// Flashing full-panel clear. Wipes ghosting off the panel and resets
    /// both framebuffers to white.
    pub fn full_clear(&mut self) {
        unsafe {
            epdiy::epd_poweron();
            epdiy::epd_fullclear(&mut self.state, ROOM_TEMPERATURE_C);
            epdiy::epd_poweroff();
        }
        log::info!("full_clear done");
    }

    /// Wipes `target` to white. Nothing else resets a framebuffer within a
    /// boot, so without this a buffer drawn into repeatedly accumulates
    /// every frame's ink - see `RefreshMode`.
    pub fn clear_target(&mut self, target: FbTarget) {
        unsafe {
            let fb = self.fb_ptr(target);
            let fb_size = (epdiy::epd_width() / 2 * epdiy::epd_height()) as usize;
            std::ptr::write_bytes(fb, 0xFF, fb_size);
        }
    }

    fn fb_ptr(&mut self, target: FbTarget) -> *mut u8 {
        unsafe {
            match target {
                FbTarget::Front => epdiy::epd_hl_get_framebuffer(&mut self.state),
                FbTarget::Back => self.state.back_fb,
            }
        }
    }

    /// Draws `text` at (x, y) into `target`. Doesn't touch the panel; call
    /// `update` once the frame is complete.
    pub fn draw_text_to(&mut self, target: FbTarget, font: Font, text: &str, x: i32, y: i32) {
        let c_text = CString::new(text).expect("text had a NUL byte");
        unsafe {
            let fb = self.fb_ptr(target);
            let mut cursor_x = x;
            let mut cursor_y = y;
            let err = epdiy::epd_write_default(
                font.as_epd_font(),
                c_text.as_ptr(),
                &mut cursor_x,
                &mut cursor_y,
                fb,
            );
            if err != epdiy::EpdDrawError_EPD_DRAW_SUCCESS {
                log::warn!("epd_write_default(\"{text}\") returned error flags: 0x{err:x}");
            }
        }
    }

    /// Measures `text` without drawing it, from a baseline-left origin.
    pub fn measure_text(&self, font: Font, text: &str) -> TextBounds {
        let c_text = CString::new(text).expect("text had a NUL byte");
        let (x, y) = (0i32, 0i32);
        let (mut x1, mut y1, mut w, mut h) = (0i32, 0i32, 0i32, 0i32);
        unsafe {
            // The header says properties may be NULL, but the
            // implementation asserts non-NULL.
            let props = epdiy::epd_font_properties_default();
            epdiy::epd_get_text_bounds(
                font.as_epd_font(),
                c_text.as_ptr(),
                &x,
                &y,
                &mut x1,
                &mut y1,
                &mut w,
                &mut h,
                &props,
            );
        }
        TextBounds { width: w }
    }

    /// Draws a battery outline with a fill proportional to `percent`.
    /// (x, y) is the top-left corner; see `BATTERY_ICON_WIDTH`/`_HEIGHT`.
    pub fn draw_battery_icon_to(&mut self, target: FbTarget, x: i32, y: i32, percent: u8) {
        const NUB_WIDTH: i32 = 4;
        const WIDTH: i32 = Display::BATTERY_ICON_WIDTH - NUB_WIDTH; // body only, excludes the nub
        const HEIGHT: i32 = Display::BATTERY_ICON_HEIGHT;
        const PAD: i32 = 3;

        unsafe {
            let fb = self.fb_ptr(target);

            epdiy::epd_draw_rect(epdiy::EpdRect { x, y, width: WIDTH, height: HEIGHT }, BLACK, fb);
            epdiy::epd_fill_rect(
                epdiy::EpdRect { x: x + WIDTH, y: y + HEIGHT / 2 - 3, width: NUB_WIDTH, height: 6 },
                BLACK,
                fb,
            );

            let inner_w = WIDTH - PAD * 2;
            let fill_w = (inner_w as f32 * (percent.min(100) as f32 / 100.0)).round() as i32;
            if fill_w > 0 {
                epdiy::epd_fill_rect(
                    epdiy::EpdRect {
                        x: x + PAD,
                        y: y + PAD,
                        width: fill_w,
                        height: HEIGHT - PAD * 2,
                    },
                    BLACK,
                    fb,
                );
            }
        }
    }

    /// Measures a weather icon glyph without drawing it.
    pub fn measure_weather_icon(&self, icon: WeatherIcon) -> TextBounds {
        let mut buf = [0u8; 4];
        let s = icon.glyph().encode_utf8(&mut buf);
        self.measure_text(Font::Icon, s)
    }

    /// Draws a weather icon glyph, baseline-left at (x, y).
    pub fn draw_weather_icon_to(&mut self, target: FbTarget, icon: WeatherIcon, x: i32, y: i32) {
        let mut buf = [0u8; 4];
        let s = icon.glyph().encode_utf8(&mut buf);
        self.draw_text_to(target, Font::Icon, s, x, y);
    }

    /// Pushes the framebuffer to the panel. `Full` writes everything;
    /// `Fast` writes only what differs from `back_fb` - see `RefreshMode`.
    pub fn update(&mut self, mode: RefreshMode) {
        unsafe {
            epdiy::epd_poweron();
            let err = epdiy::epd_hl_update_screen(&mut self.state, mode.as_epd_mode(), ROOM_TEMPERATURE_C);
            epdiy::epd_poweroff();
            if err != epdiy::EpdDrawError_EPD_DRAW_SUCCESS {
                log::warn!("epd_hl_update_screen returned error flags: 0x{err:x}");
            }
        }
        log::info!("update done");
    }
}

const ROOM_TEMPERATURE_C: i32 = 25;
