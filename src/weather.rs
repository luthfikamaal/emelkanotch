use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use chrono::{Datelike, Local, NaiveDateTime, Timelike};
use eframe::egui;

use crate::prayer::{ip_coords, reverse_geocode, win_coords};

const UA: &str = concat!("EmelkaNotch/", env!("CARGO_PKG_VERSION"));
const REFRESH_SECS: u64 = 900;

// ---------- data ----------

#[derive(Clone, Copy, Default)]
pub(crate) struct WeatherNow {
    pub temp: f32,
    pub feels: f32,
    pub humidity: u8,
    pub wind: f32, // km/j
    pub code: u8,  // WMO weather code
    pub is_day: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct WeatherHour {
    pub time: NaiveDateTime,
    pub temp: f32,
    pub pop: u8, // peluang hujan %
    pub code: u8,
}

#[derive(Clone, Default, PartialEq)]
pub(crate) enum WeatherStatus {
    #[default]
    Locating,
    Loading,
    Ready,
    Error(String),
}

#[derive(Default)]
pub(crate) struct WeatherState {
    pub status: WeatherStatus,
    pub place: String,
    pub lat: f64,
    pub lon: f64,
    pub now: Option<WeatherNow>,
    pub hourly: Vec<WeatherHour>,
    pub updated: Option<NaiveDateTime>,
    pub retry: bool,
}

type Parsed = (WeatherNow, Vec<WeatherHour>);

// ---------- cache ----------

fn weather_cache_path() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("APPDATA")?).join("EmelkaNotch");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join("weather_cache.json"))
}

pub(crate) fn load_weather_cache() -> WeatherState {
    let mut st = WeatherState::default();
    let Some(p) = weather_cache_path() else {
        return st;
    };
    let Ok(s) = std::fs::read_to_string(p) else {
        return st;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) else {
        return st;
    };
    st.place = v
        .get("place")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    st.lat = v.get("lat").and_then(|x| x.as_f64()).unwrap_or(0.0);
    st.lon = v.get("lon").and_then(|x| x.as_f64()).unwrap_or(0.0);
    if let Some(raw) = v.get("raw") {
        if let Ok((n, h)) = parse_forecast(raw) {
            st.now = Some(n);
            st.hourly = h;
            st.status = WeatherStatus::Ready;
        }
    }
    st
}

fn save_weather_cache(place: &str, lat: f64, lon: f64, raw: &serde_json::Value) {
    let Some(p) = weather_cache_path() else {
        return;
    };
    let data = serde_json::json!({
        "place": place,
        "lat": lat,
        "lon": lon,
        "raw": raw,
    });
    if let Ok(s) = serde_json::to_string(&data) {
        let _ = std::fs::write(p, s);
    }
}

// ---------- fetch ----------

fn fetch_forecast(lat: f64, lon: f64) -> Result<serde_json::Value, String> {
    ureq::get("https://api.open-meteo.com/v1/forecast")
        .set("User-Agent", UA)
        .query("latitude", &format!("{lat:.4}"))
        .query("longitude", &format!("{lon:.4}"))
        .query(
            "current",
            "temperature_2m,relative_humidity_2m,apparent_temperature,is_day,weather_code,wind_speed_10m",
        )
        .query(
            "hourly",
            "temperature_2m,precipitation_probability,weather_code",
        )
        .query("timezone", "auto")
        .query("forecast_days", "7")
        .timeout(Duration::from_secs(12))
        .call()
        .map_err(|e| e.to_string())?
        .into_json::<serde_json::Value>()
        .map_err(|e| e.to_string())
}

fn num(v: &serde_json::Value) -> f32 {
    v.as_f64().unwrap_or(0.0) as f32
}

fn parse_forecast(v: &serde_json::Value) -> Result<Parsed, String> {
    let bad = || "Unrecognized weather format".to_string();

    let cur = v.get("current").ok_or_else(bad)?;
    let g = |k: &str| cur.get(k).map_or(0.0, num);
    let now = WeatherNow {
        temp: g("temperature_2m"),
        feels: g("apparent_temperature"),
        humidity: g("relative_humidity_2m").round() as u8,
        wind: g("wind_speed_10m"),
        code: g("weather_code") as u8,
        is_day: g("is_day") > 0.5,
    };

    let h = v.get("hourly").ok_or_else(bad)?;
    let col = |k: &str| {
        h.get(k)
            .and_then(|x| x.as_array())
            .cloned()
            .unwrap_or_default()
    };
    let (times, temps, pops, codes) = (
        col("time"),
        col("temperature_2m"),
        col("precipitation_probability"),
        col("weather_code"),
    );
    let mut hours = Vec::new();
    for (i, t) in times.iter().enumerate() {
        let Some(time) = t
            .as_str()
            .and_then(|s| NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M").ok())
        else {
            continue;
        };
        hours.push(WeatherHour {
            time,
            temp: temps.get(i).map_or(0.0, num),
            pop: pops.get(i).map_or(0.0, num).round() as u8,
            code: codes.get(i).map_or(0.0, num) as u8,
        });
    }
    hours.truncate(72);

    if hours.is_empty() {
        return Err("Empty weather data from server".into());
    }
    Ok((now, hours))
}

// ---------- WMO helpers ----------

fn weather_desc(code: u8) -> &'static str {
    match code {
        0 => "Clear",
        1 => "Mostly Clear",
        2 => "Partly Cloudy",
        3 => "Overcast",
        45 | 48 => "Foggy",
        51 | 53 | 55 => "Drizzle",
        56 | 57 => "Freezing Drizzle",
        61 | 63 | 65 => "Rain",
        66 | 67 => "Freezing Rain",
        71 | 73 | 75 => "Snow",
        77 => "Snow Grains",
        80 | 81 | 82 => "Rain Showers",
        85 | 86 => "Snow Showers",
        95 => "Thunderstorm",
        96 | 99 => "Hailstorm",
        _ => "Cloudy",
    }
}

fn weather_color(code: u8, is_day: bool) -> egui::Color32 {
    if !is_day {
        return match code {
            0 | 1 => egui::Color32::from_rgb(205, 220, 255),
            2 => egui::Color32::from_rgb(150, 160, 180),
            3 => egui::Color32::from_rgb(150, 160, 180),
            45 | 48 => egui::Color32::from_rgb(150, 160, 180),
            51..=57 | 61..=67 | 80..=82 => egui::Color32::from_rgb(96, 170, 255),
            71..=77 | 85 | 86 => egui::Color32::from_rgb(200, 230, 255),
            95..=99 => egui::Color32::from_rgb(255, 196, 70),
            _ => egui::Color32::from_rgb(170, 178, 196),
        };
    }
    match code {
        0 | 1 => egui::Color32::from_rgb(255, 196, 70),
        2 => egui::Color32::from_rgb(170, 178, 196),
        3 => egui::Color32::from_rgb(170, 178, 196),
        45 | 48 => egui::Color32::from_rgb(160, 170, 180),
        51..=57 | 61..=67 | 80..=82 => egui::Color32::from_rgb(96, 170, 255),
        71..=77 | 85 | 86 => egui::Color32::from_rgb(200, 230, 255),
        95..=99 => egui::Color32::from_rgb(255, 196, 70),
        _ => egui::Color32::from_rgb(170, 178, 196),
    }
}

fn month_short(m: u32) -> &'static str {
    match m {
        1 => "Jan",
        2 => "Feb",
        3 => "Mar",
        4 => "Apr",
        5 => "May",
        6 => "Jun",
        7 => "Jul",
        8 => "Aug",
        9 => "Sep",
        10 => "Oct",
        11 => "Nov",
        _ => "Dec",
    }
}

fn mix(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    egui::Color32::from_rgb(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()))
}

// ---------- sky palette & atmospheric helpers ----------

struct SkyTheme {
    top: egui::Color32,
    bot: egui::Color32,
    glow: egui::Color32,
    accent: egui::Color32,
    border: egui::Color32,
    stars: bool,
    rain_streaks: bool,
}

fn sky_theme(code: u8, is_day: bool) -> SkyTheme {
    use egui::Color32;
    if !is_day {
        match code {
            0 | 1 => SkyTheme {
                top: Color32::from_rgb(13, 22, 44),
                bot: Color32::from_rgb(24, 38, 70),
                glow: Color32::from_rgb(125, 168, 255),
                accent: Color32::from_rgb(154, 194, 255),
                border: Color32::from_rgb(56, 76, 118),
                stars: true,
                rain_streaks: false,
            },
            51..=67 | 80..=82 => SkyTheme {
                top: Color32::from_rgb(13, 24, 38),
                bot: Color32::from_rgb(22, 42, 62),
                glow: Color32::from_rgb(90, 166, 240),
                accent: Color32::from_rgb(108, 184, 255),
                border: Color32::from_rgb(48, 76, 104),
                stars: false,
                rain_streaks: true,
            },
            95..=99 => SkyTheme {
                top: Color32::from_rgb(22, 18, 38),
                bot: Color32::from_rgb(38, 30, 62),
                glow: Color32::from_rgb(240, 182, 76),
                accent: Color32::from_rgb(248, 198, 92),
                border: Color32::from_rgb(80, 68, 112),
                stars: false,
                rain_streaks: true,
            },
            _ => SkyTheme {
                top: Color32::from_rgb(18, 24, 38),
                bot: Color32::from_rgb(30, 40, 60),
                glow: Color32::from_rgb(122, 152, 206),
                accent: Color32::from_rgb(138, 182, 250),
                border: Color32::from_rgb(56, 68, 94),
                stars: false,
                rain_streaks: false,
            },
        }
    } else {
        match code {
            0 | 1 => SkyTheme {
                top: Color32::from_rgb(24, 58, 98),
                bot: Color32::from_rgb(44, 88, 132),
                glow: Color32::from_rgb(255, 198, 92),
                accent: Color32::from_rgb(255, 204, 96),
                border: Color32::from_rgb(76, 118, 158),
                stars: false,
                rain_streaks: false,
            },
            2 => SkyTheme {
                top: Color32::from_rgb(24, 48, 78),
                bot: Color32::from_rgb(40, 70, 102),
                glow: Color32::from_rgb(220, 205, 155),
                accent: Color32::from_rgb(138, 194, 255),
                border: Color32::from_rgb(68, 96, 128),
                stars: false,
                rain_streaks: false,
            },
            51..=67 | 80..=82 => SkyTheme {
                top: Color32::from_rgb(18, 34, 52),
                bot: Color32::from_rgb(32, 56, 80),
                glow: Color32::from_rgb(104, 178, 248),
                accent: Color32::from_rgb(108, 184, 255),
                border: Color32::from_rgb(56, 86, 114),
                stars: false,
                rain_streaks: true,
            },
            95..=99 => SkyTheme {
                top: Color32::from_rgb(26, 24, 44),
                bot: Color32::from_rgb(44, 38, 68),
                glow: Color32::from_rgb(248, 190, 82),
                accent: Color32::from_rgb(250, 200, 96),
                border: Color32::from_rgb(84, 76, 118),
                stars: false,
                rain_streaks: true,
            },
            _ => SkyTheme {
                top: Color32::from_rgb(26, 34, 48),
                bot: Color32::from_rgb(42, 52, 70),
                glow: Color32::from_rgb(162, 182, 214),
                accent: Color32::from_rgb(142, 188, 252),
                border: Color32::from_rgb(66, 78, 100),
                stars: false,
                rain_streaks: false,
            },
        }
    }
}

/// Draw a smoothly shaded rounded-rectangle sky canvas with vertical gradient + radial light.
fn paint_sky_card(
    p: &egui::Painter,
    rect: egui::Rect,
    radius: f32,
    theme: &SkyTheme,
    light_pos: egui::Pos2,
) {
    use egui::{pos2, Mesh, Shape};
    use std::f32::consts::FRAC_PI_2;

    let r = radius.min(rect.width() * 0.5).min(rect.height() * 0.5);
    let segs = 8usize;
    let corners = [
        (pos2(rect.right() - r, rect.top() + r), -FRAC_PI_2, 0.0),
        (pos2(rect.right() - r, rect.bottom() - r), 0.0, FRAC_PI_2),
        (
            pos2(rect.left() + r, rect.bottom() - r),
            FRAC_PI_2,
            std::f32::consts::PI,
        ),
        (
            pos2(rect.left() + r, rect.top() + r),
            std::f32::consts::PI,
            std::f32::consts::PI + FRAC_PI_2,
        ),
    ];

    let sample_color = |pt: egui::Pos2| {
        let ty = ((pt.y - rect.top()) / rect.height().max(1.0)).clamp(0.0, 1.0);
        let base = mix(theme.top, theme.bot, ty);
        let dist = (pt - light_pos).length() / 185.0;
        let glow = (1.0 - dist).clamp(0.0, 1.0).powi(2) * 0.34;
        mix(base, theme.glow, glow)
    };

    let mut perim = Vec::with_capacity(corners.len() * (segs + 1));
    for (center, a0, a1) in corners {
        for s in 0..=segs {
            let a = a0 + (a1 - a0) * (s as f32 / segs as f32);
            perim.push(pos2(center.x + r * a.cos(), center.y + r * a.sin()));
        }
    }

    let mut mesh = Mesh::default();
    let c = rect.center();
    mesh.colored_vertex(c, sample_color(c));
    for &pt in &perim {
        mesh.colored_vertex(pt, sample_color(pt));
    }
    let n = perim.len() as u32;
    for i in 0..n {
        let next = if i + 1 == n { 1 } else { i + 2 };
        mesh.add_triangle(0, i + 1, next);
    }
    p.add(Shape::mesh(mesh));

    // Subtle atmospheric details inside the sky canvas
    if theme.stars {
        for (fx, fy, sr, a) in [
            (0.62, 0.22, 1.1, 180),
            (0.71, 0.16, 0.9, 130),
            (0.84, 0.18, 1.2, 200),
            (0.91, 0.34, 0.9, 140),
            (0.66, 0.42, 0.8, 110),
            (0.55, 0.30, 1.0, 150),
        ] {
            p.circle_filled(
                pos2(rect.left() + rect.width() * fx, rect.top() + rect.height() * fy),
                sr,
                egui::Color32::from_rgba_unmultiplied(235, 242, 255, a),
            );
        }
    }
    if theme.rain_streaks {
        let st = egui::Stroke::new(
            1.0,
            egui::Color32::from_rgba_unmultiplied(180, 218, 255, 28),
        );
        for (fx, fy, len) in [
            (0.58, 0.18, 14.0),
            (0.66, 0.36, 16.0),
            (0.74, 0.20, 12.0),
            (0.88, 0.26, 15.0),
            (0.62, 0.62, 13.0),
            (0.79, 0.68, 14.0),
        ] {
            let x = rect.left() + rect.width() * fx;
            let y = rect.top() + rect.height() * fy;
            p.line_segment([pos2(x, y), pos2(x - len * 0.32, y + len)], st);
        }
    }
}

// ---------- icons ----------

fn cloud_layered(p: &egui::Painter, c: egui::Pos2, s: f32, col: egui::Color32, bg: egui::Color32) {
    use egui::{pos2, vec2, Rect};
    let back = mix(bg, col, 0.52);
    let front = mix(col, egui::Color32::from_rgb(246, 249, 255), 0.32);

    // Back puff for dimensional depth
    p.circle_filled(c + vec2(3.6 * s, -2.0 * s), 3.4 * s, back);
    p.circle_filled(c + vec2(-2.2 * s, -2.2 * s), 3.2 * s, back);

    // Main luminous cloud silhouette
    p.circle_filled(c + vec2(-4.2 * s, 1.2 * s), 3.6 * s, front);
    p.circle_filled(c + vec2(-0.3 * s, -1.4 * s), 4.8 * s, front);
    p.circle_filled(c + vec2(4.4 * s, 1.2 * s), 3.3 * s, front);
    p.rect_filled(
        Rect::from_min_max(
            pos2(c.x - 4.2 * s, c.y + 0.2 * s),
            pos2(c.x + 4.4 * s, c.y + 4.8 * s),
        ),
        1.6 * s,
        front,
    );

    // Soft underbelly tint for sculpted volume
    let under = mix(bg, col, 0.72);
    p.rect_filled(
        Rect::from_min_max(
            pos2(c.x - 4.6 * s, c.y + 3.2 * s),
            pos2(c.x + 4.6 * s, c.y + 4.8 * s),
        ),
        1.2 * s,
        under,
    );
}

fn sun(p: &egui::Painter, c: egui::Pos2, r: f32, col: egui::Color32) {
    use egui::{vec2, Color32, Stroke};
    p.circle_filled(
        c,
        r * 1.45,
        Color32::from_rgba_unmultiplied(col.r(), col.g(), col.b(), 36),
    );
    p.circle_filled(c, r, col);
    p.circle_filled(
        c + vec2(-r * 0.22, -r * 0.22),
        r * 0.45,
        mix(col, Color32::WHITE, 0.45),
    );
    for k in 0..8 {
        let a = k as f32 * std::f32::consts::TAU / 8.0;
        let co = vec2(a.cos(), a.sin());
        p.line_segment(
            [c + co * (r + 2.4), c + co * (r + 4.8)],
            Stroke::new((r * 0.28).clamp(1.1, 2.0), col),
        );
    }
}

/// Ikon cuaca berlapis. `bg` = warna latar (untuk potongan bulan sabit & kedalaman awan).
pub(crate) fn weather_icon(
    p: &egui::Painter,
    c: egui::Pos2,
    code: u8,
    is_day: bool,
    col: egui::Color32,
    bg: egui::Color32,
    scale: f32,
) {
    use egui::{pos2, vec2, Stroke};
    let u = scale.max(0.1);
    let st = Stroke::new((1.4 * u).max(1.0), col);
    match code {
        0 | 1 => {
            if is_day {
                sun(p, c, 4.4 * u, col);
            } else {
                p.circle_filled(c + vec2(-1.0, 0.0) * u, 6.8 * u, col);
                p.circle_filled(c + vec2(2.3, -2.0) * u, 5.8 * u, bg);
                p.circle_filled(c + vec2(5.6, -5.0) * u, 1.3 * u, col);
                p.circle_filled(c + vec2(7.2, -1.2) * u, 0.9 * u, col.gamma_multiply(0.75));
            }
        }
        2 => {
            let sc = c + vec2(3.8, -4.0) * u;
            if is_day {
                let sun_col = egui::Color32::from_rgb(255, 204, 92);
                p.circle_filled(sc, 3.5 * u, sun_col);
                p.circle_filled(
                    sc + vec2(-0.8, -0.8) * u,
                    1.5 * u,
                    mix(sun_col, egui::Color32::WHITE, 0.4),
                );
            } else {
                let moon_col = egui::Color32::from_rgb(206, 224, 255);
                p.circle_filled(sc, 3.6 * u, moon_col);
                p.circle_filled(sc + vec2(1.7, -1.3) * u, 3.0 * u, bg);
            }
            cloud_layered(p, c + vec2(-1.6, 2.0) * u, 0.96 * u, col, bg);
        }
        45 | 48 => {
            cloud_layered(p, c + vec2(0.0, -3.0) * u, 0.88 * u, col, bg);
            p.line_segment(
                [
                    pos2(c.x - 6.8 * u, c.y + 4.2 * u),
                    pos2(c.x + 6.8 * u, c.y + 4.2 * u),
                ],
                st,
            );
            p.line_segment(
                [
                    pos2(c.x - 4.6 * u, c.y + 7.4 * u),
                    pos2(c.x + 5.8 * u, c.y + 7.4 * u),
                ],
                Stroke::new((1.2 * u).max(1.0), col.gamma_multiply(0.7)),
            );
        }
        51..=57 | 61..=67 | 80..=82 => {
            let cloud_col = egui::Color32::from_rgb(192, 204, 222);
            cloud_layered(p, c + vec2(0.0, -2.8) * u, 0.95 * u, cloud_col, bg);
            let n = if matches!(code, 51..=57) { 2 } else { 3 };
            let drop_st = Stroke::new((1.5 * u).max(1.1), egui::Color32::from_rgb(108, 186, 255));
            for k in 0..n {
                let x = c.x - (n as f32 - 1.0) * 2.1 * u + k as f32 * 4.2 * u;
                let dy = if k % 2 == 1 { 1.2 * u } else { 0.0 };
                p.line_segment(
                    [
                        pos2(x + 0.8 * u, c.y + 4.2 * u + dy),
                        pos2(x - 0.8 * u, c.y + 7.8 * u + dy),
                    ],
                    drop_st,
                );
            }
        }
        71..=77 | 85 | 86 => {
            cloud_layered(p, c + vec2(0.0, -2.8) * u, 0.95 * u, col, bg);
            for k in 0..3 {
                let x = c.x - 4.0 * u + k as f32 * 4.0 * u;
                p.circle_filled(
                    pos2(x, c.y + 6.2 * u + (k % 2) as f32 * 1.6 * u),
                    1.1 * u,
                    col,
                );
            }
        }
        95..=99 => {
            let storm_col = egui::Color32::from_rgb(164, 174, 198);
            cloud_layered(p, c + vec2(0.0, -3.0) * u, 0.95 * u, storm_col, bg);
            let bolt = egui::Color32::from_rgb(255, 204, 84);
            let pts = [
                pos2(c.x + 1.0 * u, c.y + 1.4 * u),
                pos2(c.x - 2.2 * u, c.y + 5.6 * u),
                pos2(c.x + 0.5 * u, c.y + 5.6 * u),
                pos2(c.x - 1.6 * u, c.y + 9.2 * u),
            ];
            for w in pts.windows(2) {
                p.line_segment([w[0], w[1]], Stroke::new((1.7 * u).max(1.1), bolt));
            }
        }
        _ => cloud_layered(p, c, 1.12 * u, col, bg),
    }
}

fn wind_glyph(p: &egui::Painter, c: egui::Pos2, col: egui::Color32) {
    use egui::{pos2, Stroke};
    let st = Stroke::new(1.25, col);
    for (dy, len) in [(-2.8, 7.5), (0.0, 9.5), (2.8, 6.5)] {
        p.line_segment(
            [pos2(c.x - 4.5, c.y + dy), pos2(c.x - 4.5 + len, c.y + dy)],
            st,
        );
    }
}

fn drop_glyph(p: &egui::Painter, c: egui::Pos2, col: egui::Color32) {
    use egui::{pos2, Stroke};
    p.circle_filled(pos2(c.x, c.y + 1.6), 2.3, col);
    let st = Stroke::new(1.25, col);
    p.line_segment([pos2(c.x, c.y - 4.2), pos2(c.x - 2.1, c.y + 1.1)], st);
    p.line_segment([pos2(c.x, c.y - 4.2), pos2(c.x + 2.1, c.y + 1.1)], st);
}

fn humidity_glyph(p: &egui::Painter, c: egui::Pos2, col: egui::Color32) {
    use egui::{pos2, Stroke};
    let st = Stroke::new(1.2, col);
    p.circle_stroke(pos2(c.x, c.y + 1.4), 2.5, st);
    p.line_segment([pos2(c.x, c.y - 4.2), pos2(c.x - 2.2, c.y + 0.6)], st);
    p.line_segment([pos2(c.x, c.y - 4.2), pos2(c.x + 2.2, c.y + 0.6)], st);
    p.circle_filled(pos2(c.x, c.y + 2.1), 1.1, col);
}

fn smooth_curve(points: &[egui::Pos2]) -> Vec<egui::Pos2> {
    if points.len() < 3 {
        return points.to_vec();
    }
    let mut out = Vec::with_capacity((points.len() - 1) * 8 + 1);
    for i in 0..points.len() - 1 {
        let p0 = points[i.saturating_sub(1)];
        let p1 = points[i];
        let p2 = points[i + 1];
        let p3 = points[(i + 2).min(points.len() - 1)];
        for s in 0..8 {
            let t = s as f32 / 8.0;
            let t2 = t * t;
            let t3 = t2 * t;
            let x = 0.5
                * (2.0 * p1.x
                    + (-p0.x + p2.x) * t
                    + (2.0 * p0.x - 5.0 * p1.x + 4.0 * p2.x - p3.x) * t2
                    + (-p0.x + 3.0 * p1.x - 3.0 * p2.x + p3.x) * t3);
            let y = 0.5
                * (2.0 * p1.y
                    + (-p0.y + p2.y) * t
                    + (2.0 * p0.y - 5.0 * p1.y + 4.0 * p2.y - p3.y) * t2
                    + (-p0.y + 3.0 * p1.y - 3.0 * p2.y + p3.y) * t3);
            out.push(egui::pos2(x, y));
        }
    }
    if let Some(last) = points.last() {
        out.push(*last);
    }
    out
}

// ---------- layout ----------

const PAD: f32 = 16.0;
const COL_INSET: f32 = 12.0;

/// Panel cuaca: kanvas langit atmosferik sesuai kondisi + 3 kartu telemetri bergauge +
/// tray forecast per jam interaktif dengan kurva gradien.
pub(crate) fn draw_weather_panel(ui: &mut egui::Ui, s: &WeatherState) -> bool {
    use eframe::egui::{self, pos2, vec2, Align2, Color32, CornerRadius, FontId, Rect, Sense, Stroke};

    let dim = Color32::from_rgb(138, 145, 162);
    let bright = Color32::from_rgb(242, 245, 252);
    let rain_col = Color32::from_rgb(104, 182, 255);
    let humid_col = Color32::from_rgb(98, 208, 210);
    let wind_col = Color32::from_rgb(186, 198, 222);
    let mut retry = false;

    let Some(now_w) = s.now else {
        ui.add_space(16.0);
        ui.vertical_centered(|ui| {
            let (title, sub, failed) = match &s.status {
                WeatherStatus::Locating => (
                    "Detecting location…".to_string(),
                    "Using device location, IP fallback".to_string(),
                    false,
                ),
                WeatherStatus::Error(e) => (
                    "Weather unavailable".to_string(),
                    crate::theme::ellipsize(e, 70),
                    true,
                ),
                _ => (
                    "Loading weather…".to_string(),
                    "Source: open-meteo.com".to_string(),
                    false,
                ),
            };
            ui.label(
                egui::RichText::new(title)
                    .font(crate::theme::semibold(14.0))
                    .color(bright),
            );
            ui.add_space(2.0);
            ui.label(egui::RichText::new(sub).size(11.0).color(dim));
            if failed {
                ui.add_space(8.0);
                let (r, _) = ui.allocate_exact_size(vec2(92.0, 26.0), Sense::hover());
                let p = ui.painter().clone();
                if crate::ui::text_btn(ui, &p, r, "Retry", "wx_retry", true, true) {
                    retry = true;
                }
            }
        });
        return retry;
    };

    let old_spacing = ui.spacing().item_spacing;
    ui.spacing_mut().item_spacing.y = 0.0;

    let w = ui.available_width();
    let now = Local::now();
    let today = now.date_naive();
    let cutoff = today.and_hms_opt(now.hour(), 0, 0).unwrap_or_default();
    let hours: Vec<&WeatherHour> = s
        .hourly
        .iter()
        .filter(|h| h.time >= cutoff)
        .take(7)
        .collect();
    let col = weather_color(now_w.code, now_w.is_day);
    let sky = sky_theme(now_w.code, now_w.is_day);

    // Real High / Low temperature from today's hourly data
    let mut day_lo = f32::MAX;
    let mut day_hi = f32::MIN;
    for h in s.hourly.iter().filter(|h| h.time.date() == today) {
        day_lo = day_lo.min(h.temp);
        day_hi = day_hi.max(h.temp);
    }
    let has_hl = day_lo <= day_hi;

    // ================= 1. ATMOSPHERIC HERO CANVAS =================
    let hero_h = 132.0;
    let (hero, _) = ui.allocate_exact_size(vec2(w, hero_h), Sense::hover());
    let p = ui.painter().clone();
    let icon_c = pos2(hero.right() - 56.0, hero.top() + 74.0);
    paint_sky_card(&p, hero, 16.0, &sky, icon_c);
    p.rect_stroke(
        hero,
        CornerRadius::same(16),
        Stroke::new(1.0, sky.border.gamma_multiply(0.65)),
        egui::StrokeKind::Inside,
    );

    let left = hero.left() + PAD;
    let top = hero.top();
    let right = hero.right() - PAD;

    // Top-left frosted location chip (click to refresh)
    let is_updating = matches!(s.status, WeatherStatus::Locating | WeatherStatus::Loading);
    let raw_place = if is_updating {
        "Locating…"
    } else {
        s.place.trim_start_matches("Kota ")
    };
    let loc_font = crate::theme::semibold(10.5);
    let (place, place_w) = crate::theme::fit(&p, raw_place, loc_font.clone(), 165.0);
    let time_s = if !is_updating {
        s.updated
            .map(|u| format!("· {:02}:{:02}", u.hour(), u.minute()))
            .unwrap_or_default()
    } else {
        String::new()
    };
    let time_w = if time_s.is_empty() {
        0.0
    } else {
        crate::theme::text_w(&p, &time_s, &FontId::proportional(10.0)) + 5.0
    };
    let chip_w = 22.0 + place_w + time_w + 10.0;
    let chip_r = Rect::from_min_size(pos2(left, top + 12.0), vec2(chip_w, 22.0));
    let resp = ui
        .interact(chip_r, ui.id().with("wx_place"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Click to re-detect location & refresh");
    if resp.clicked() {
        retry = true;
    }
    let chip_fill = if resp.hovered() {
        Color32::from_rgba_unmultiplied(12, 16, 26, 170)
    } else {
        Color32::from_rgba_unmultiplied(8, 12, 20, 125)
    };
    p.rect_filled(chip_r, CornerRadius::same(11), chip_fill);
    p.rect_stroke(
        chip_r,
        CornerRadius::same(11),
        Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 28)),
        egui::StrokeKind::Inside,
    );
    let pin = pos2(chip_r.left() + 11.0, chip_r.center().y);
    let pin_col = if is_updating { sky.accent } else { bright };
    p.circle_stroke(pin, 2.6, Stroke::new(1.2, pin_col));
    p.circle_filled(pin, 0.9, pin_col);
    p.text(
        pos2(chip_r.left() + 19.0, chip_r.center().y),
        Align2::LEFT_CENTER,
        &place,
        loc_font,
        if is_updating { sky.accent } else { bright },
    );
    if !time_s.is_empty() {
        p.text(
            pos2(chip_r.left() + 19.0 + place_w + 5.0, chip_r.center().y),
            Align2::LEFT_CENTER,
            &time_s,
            FontId::proportional(10.0),
            Color32::from_rgba_unmultiplied(210, 222, 240, 165),
        );
    }

    // Top-right date + H/L chip
    let hl_txt = if has_hl {
        format!(
            "{} {}  ·  H {:.0}°  L {:.0}°",
            now.day(),
            month_short(now.month()),
            day_hi,
            day_lo
        )
    } else {
        format!("{} {}", now.day(), month_short(now.month()))
    };
    let hl_font = FontId::proportional(10.5);
    let hl_w = crate::theme::text_w(&p, &hl_txt, &hl_font);
    let hl_rect = Rect::from_min_size(pos2(right - hl_w - 16.0, top + 12.0), vec2(hl_w + 16.0, 22.0));
    p.rect_filled(
        hl_rect,
        CornerRadius::same(11),
        Color32::from_rgba_unmultiplied(8, 12, 20, 105),
    );
    p.text(
        hl_rect.center(),
        Align2::CENTER_CENTER,
        hl_txt,
        hl_font,
        Color32::from_rgba_unmultiplied(230, 238, 250, 215),
    );

    // Editorial temperature + condition block
    let temp_txt = format!("{:.0}°", now_w.temp);
    let temp_font = crate::theme::semibold(48.0);
    p.text(
        pos2(left, top + 82.0),
        Align2::LEFT_CENTER,
        &temp_txt,
        temp_font.clone(),
        bright,
    );
    let tw = crate::theme::text_w(&p, &temp_txt, &temp_font);
    p.text(
        pos2(left + tw + 1.0, top + 66.0),
        Align2::LEFT_CENTER,
        "C",
        crate::theme::semibold(14.0),
        Color32::from_rgba_unmultiplied(220, 232, 248, 175),
    );

    let cond_x = left + tw + 24.0;
    p.text(
        pos2(cond_x, top + 72.0),
        Align2::LEFT_CENTER,
        weather_desc(now_w.code),
        crate::theme::semibold(15.0),
        bright,
    );
    p.text(
        pos2(cond_x, top + 93.0),
        Align2::LEFT_CENTER,
        format!("Feels like {:.0}°", now_w.feels),
        FontId::proportional(11.5),
        Color32::from_rgba_unmultiplied(214, 226, 244, 185),
    );

    // Hero layered weather artwork on the right
    let icon_bg = mix(sky.top, sky.bot, 0.55);
    weather_icon(&p, icon_c, now_w.code, now_w.is_day, col, icon_bg, 3.1);

    // ================= 2. TELEMETRY MICRO-CARDS WITH GAUGES =================
    ui.add_space(8.0);
    let tel_h = 42.0;
    let gap = 7.0;
    let cw = (w - gap * 2.0) / 3.0;
    let (tel_row, _) = ui.allocate_exact_size(vec2(w, tel_h), Sense::hover());
    let p = ui.painter().clone();
    let cur_pop = hours.first().map_or(0, |h| h.pop);
    let cards = [
        (
            "Rain",
            format!("{cur_pop}%"),
            (cur_pop as f32 / 100.0).clamp(0.0, 1.0),
            rain_col,
            0u8,
        ),
        (
            "Humidity",
            format!("{}%", now_w.humidity),
            (now_w.humidity as f32 / 100.0).clamp(0.0, 1.0),
            humid_col,
            1u8,
        ),
        (
            "Wind",
            format!("{:.0} km/h", now_w.wind),
            (now_w.wind / 40.0).clamp(0.04, 1.0),
            wind_col,
            2u8,
        ),
    ];

    for (i, (label, val, frac, accent, kind)) in cards.iter().enumerate() {
        let r = Rect::from_min_size(
            pos2(tel_row.left() + i as f32 * (cw + gap), tel_row.top()),
            vec2(cw, tel_h),
        );
        p.rect_filled(r, CornerRadius::same(11), Color32::from_rgb(13, 16, 23));
        p.rect_stroke(
            r,
            CornerRadius::same(11),
            Stroke::new(1.0, Color32::from_rgb(31, 36, 49)),
            egui::StrokeKind::Inside,
        );

        let ic_pos = pos2(r.left() + 15.0, r.top() + 17.5);
        match *kind {
            0 => drop_glyph(&p, ic_pos, *accent),
            1 => humidity_glyph(&p, ic_pos, *accent),
            _ => wind_glyph(&p, ic_pos, *accent),
        }
        p.text(
            pos2(r.left() + 26.0, r.top() + 17.5),
            Align2::LEFT_CENTER,
            *label,
            FontId::proportional(10.5),
            dim,
        );
        p.text(
            pos2(r.right() - 12.0, r.top() + 17.5),
            Align2::RIGHT_CENTER,
            val,
            crate::theme::semibold(12.5),
            bright,
        );

        // Micro gauge track at bottom of each telemetry card
        let tr = Rect::from_min_max(
            pos2(r.left() + 11.0, r.bottom() - 8.0),
            pos2(r.right() - 11.0, r.bottom() - 5.5),
        );
        p.rect_filled(tr, CornerRadius::same(2), Color32::from_rgb(26, 30, 42));
        if *frac > 0.01 {
            p.rect_filled(
                Rect::from_min_size(tr.min, vec2((tr.width() * *frac).max(3.0), tr.height())),
                CornerRadius::same(2),
                *accent,
            );
        }
    }

    // ================= 3. HOURLY FORECAST TRAY =================
    ui.add_space(8.0);
    let fc_h = 114.0;
    let (fc, _) = ui.allocate_exact_size(vec2(w, fc_h), Sense::hover());
    let p = ui.painter().clone();
    let tray_bg = Color32::from_rgb(10, 12, 18);
    p.rect_filled(fc, CornerRadius::same(14), tray_bg);
    p.rect_stroke(
        fc,
        CornerRadius::same(14),
        Stroke::new(1.0, Color32::from_rgb(28, 33, 45)),
        egui::StrokeKind::Inside,
    );

    let f_top = fc.top();
    let ptr = ui.input(|i| i.pointer.hover_pos());
    let mut hovered_idx: Option<usize> = None;

    if !hours.is_empty() {
        let n = hours.len();
        let col_w = (fc.width() - 2.0 * COL_INSET) / n as f32;
        let col_x = |i: usize| fc.left() + COL_INSET + col_w * (i as f32 + 0.5);

        if let Some(pos) = ptr {
            if fc.contains(pos) && pos.y >= f_top + 22.0 {
                let rel = ((pos.x - (fc.left() + COL_INSET)) / col_w).floor() as isize;
                if rel >= 0 && (rel as usize) < n {
                    hovered_idx = Some(rel as usize);
                }
            }
        }

        // Header
        p.text(
            pos2(fc.left() + 14.0, f_top + 14.0),
            Align2::LEFT_CENTER,
            "Hourly Forecast",
            crate::theme::semibold(11.5),
            bright,
        );
        let right_lbl = if let Some(hi) = hovered_idx {
            let h = hours[hi];
            let t_s = if hi == 0 {
                "Now".to_string()
            } else {
                h.time.format("%H:%M").to_string()
            };
            format!("{t_s} · {} · {}% rain", weather_desc(h.code), h.pop)
        } else {
            format!("Next {} hours", n)
        };
        p.text(
            pos2(fc.right() - 14.0, f_top + 14.0),
            Align2::RIGHT_CENTER,
            right_lbl,
            FontId::proportional(10.0),
            if hovered_idx.is_some() { sky.accent } else { dim },
        );

        // Subtle column highlight (hovered or "Now")
        let focus_idx = hovered_idx.unwrap_or(0);
        let hl = Rect::from_min_max(
            pos2(fc.left() + COL_INSET + focus_idx as f32 * col_w + 2.0, f_top + 24.0),
            pos2(
                fc.left() + COL_INSET + (focus_idx + 1) as f32 * col_w - 2.0,
                fc.bottom() - 6.0,
            ),
        );
        p.rect_filled(
            hl,
            CornerRadius::same(9),
            if hovered_idx.is_some() {
                Color32::from_rgb(20, 25, 37)
            } else {
                Color32::from_rgb(15, 19, 28)
            },
        );

        // Temperature curve with true vertical gradient fade to transparent
        let chart_top = f_top + 38.0;
        let chart_bot = f_top + 58.0;
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for h in &hours {
            lo = lo.min(h.temp);
            hi = hi.max(h.temp);
        }
        let span = (hi - lo + 1.0).max(3.0);
        let mid = (hi + lo) * 0.5;
        let lo = mid - span * 0.5;
        let pts: Vec<egui::Pos2> = hours
            .iter()
            .enumerate()
            .map(|(i, h)| {
                let y = chart_bot - (h.temp - lo) / span * (chart_bot - chart_top);
                pos2(col_x(i), y)
            })
            .collect();

        let curve = smooth_curve(&pts);
        let mut mesh = egui::Mesh::default();
        for pt in &curve {
            mesh.colored_vertex(*pt, sky.accent.gamma_multiply(0.24));
            mesh.colored_vertex(pos2(pt.x, chart_bot + 7.0), Color32::TRANSPARENT);
        }
        for i in 0..curve.len().saturating_sub(1) {
            let k = (i * 2) as u32;
            mesh.add_triangle(k, k + 1, k + 2);
            mesh.add_triangle(k + 1, k + 2, k + 3);
        }
        p.add(egui::Shape::mesh(mesh));
        p.add(egui::Shape::line(
            curve.clone(),
            Stroke::new(4.2, sky.accent.gamma_multiply(0.14)),
        ));
        p.add(egui::Shape::line(curve, Stroke::new(2.0, sky.accent)));

        for (i, (h, pt)) in hours.iter().zip(pts.iter()).enumerate() {
            let cx = col_x(i);
            let active = focus_idx == i;
            p.text(
                pos2(cx, pt.y - 5.5),
                Align2::CENTER_BOTTOM,
                format!("{:.0}°", h.temp),
                if active {
                    crate::theme::semibold(10.5)
                } else {
                    FontId::proportional(9.5)
                },
                if active { bright } else { Color32::from_rgb(198, 204, 218) },
            );
            p.circle_filled(*pt, if active { 3.3 } else { 2.4 }, sky.accent);
            p.circle_filled(*pt, 1.2, tray_bg);

            let is_day = (6..18).contains(&h.time.hour());
            let c = weather_color(h.code, is_day);
            let icon_bg = if active {
                if hovered_idx.is_some() {
                    Color32::from_rgb(20, 25, 37)
                } else {
                    Color32::from_rgb(15, 19, 28)
                }
            } else {
                tray_bg
            };
            weather_icon(&p, pos2(cx, f_top + 74.0), h.code, is_day, c, icon_bg, 1.05);

            let label = if i == 0 {
                "Now".to_string()
            } else {
                h.time.format("%H:%M").to_string()
            };
            p.text(
                pos2(cx, f_top + 91.0),
                Align2::CENTER_CENTER,
                label,
                if active {
                    crate::theme::semibold(9.5)
                } else {
                    FontId::proportional(9.0)
                },
                if active { sky.accent } else { dim },
            );
            if h.pop >= 20 {
                p.text(
                    pos2(cx, f_top + 102.5),
                    Align2::CENTER_CENTER,
                    format!("{}%", h.pop),
                    FontId::proportional(8.5),
                    rain_col.gamma_multiply(if active { 1.0 } else { 0.8 }),
                );
            }
        }
    }

    ui.spacing_mut().item_spacing = old_spacing;
    retry
}

// ---------- worker ----------

fn wait_retry(st: &Arc<Mutex<WeatherState>>, secs: u32) {
    for _ in 0..secs {
        thread::sleep(Duration::from_secs(1));
        if st.lock().unwrap().retry {
            break;
        }
    }
}

pub(crate) fn spawn_weather_worker(ctx: egui::Context, st: Arc<Mutex<WeatherState>>) {
    thread::spawn(move || {
        let mut last_fetch: Option<Instant> = None;
        let mut last_loc_check: Option<Instant> = None;

        loop {
            let force = std::mem::take(&mut st.lock().unwrap().retry);
            let (cached_lat, cached_lon, cached_place) = {
                let s = st.lock().unwrap();
                (s.lat, s.lon, s.place.clone())
            };
            let have_loc = cached_lat.abs() > 0.001 || cached_lon.abs() > 0.001;
            let need_loc = force
                || !have_loc
                || last_loc_check.map_or(true, |t| t.elapsed() > Duration::from_secs(1800));

            let (lat, lon) = if need_loc {
                if !have_loc || force {
                    st.lock().unwrap().status = WeatherStatus::Locating;
                    ctx.request_repaint();
                }
                match win_coords().or_else(ip_coords) {
                    Some((lat, lon)) => {
                        last_loc_check = Some(Instant::now());
                        let moved =
                            (lat - cached_lat).abs() > 0.01 || (lon - cached_lon).abs() > 0.01;
                        let place = if force || cached_place.is_empty() || moved {
                            reverse_geocode(lat, lon)
                                .ok()
                                .and_then(|(state, cands)| {
                                    cands.first().map(|(name, _)| name.clone()).or(Some(state))
                                })
                                .unwrap_or_else(|| {
                                    if cached_place.is_empty() {
                                        "Current location".to_string()
                                    } else {
                                        cached_place.clone()
                                    }
                                })
                        } else {
                            cached_place.clone()
                        };
                        {
                            let mut s = st.lock().unwrap();
                            s.lat = lat;
                            s.lon = lon;
                            if !place.is_empty() {
                                s.place = place;
                            }
                        }
                        (lat, lon)
                    }
                    None if have_loc => {
                        last_loc_check = Some(Instant::now());
                        st.lock().unwrap().status = WeatherStatus::Ready;
                        (cached_lat, cached_lon)
                    }
                    None => {
                        {
                            let mut s = st.lock().unwrap();
                            s.status = WeatherStatus::Error("Device location unavailable".into());
                        }
                        ctx.request_repaint();
                        wait_retry(&st, 30);
                        continue;
                    }
                }
            } else {
                (cached_lat, cached_lon)
            };

            let stale =
                last_fetch.map_or(true, |t| t.elapsed() > Duration::from_secs(REFRESH_SECS));
            if force || stale {
                {
                    let mut s = st.lock().unwrap();
                    if s.now.is_none() {
                        s.status = WeatherStatus::Loading;
                    }
                }
                ctx.request_repaint();
                match fetch_forecast(lat, lon)
                    .and_then(|raw| parse_forecast(&raw).map(|p| (raw, p)))
                {
                    Ok((raw, (n, h))) => {
                        let place = {
                            let mut s = st.lock().unwrap();
                            s.now = Some(n);
                            s.hourly = h;
                            s.updated = Some(Local::now().naive_local());
                            s.status = WeatherStatus::Ready;
                            s.place.clone()
                        };
                        save_weather_cache(&place, lat, lon, &raw);
                        last_fetch = Some(Instant::now());
                        ctx.request_repaint();
                    }
                    Err(e) => {
                        {
                            let mut s = st.lock().unwrap();
                            s.status = WeatherStatus::Error(e);
                        }
                        ctx.request_repaint();
                        wait_retry(&st, 20);
                    }
                }
            }

            wait_retry(&st, 30);
        }
    });
}
