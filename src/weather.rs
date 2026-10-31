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

// ---------- icons ----------

fn cloud(p: &egui::Painter, c: egui::Pos2, s: f32, col: egui::Color32) {
    use egui::{pos2, vec2, Rect};
    p.circle_filled(c + vec2(-4.0 * s, 1.0 * s), 3.6 * s, col);
    p.circle_filled(c + vec2(0.0, -1.5 * s), 4.6 * s, col);
    p.circle_filled(c + vec2(4.5 * s, 1.0 * s), 3.2 * s, col);
    p.rect_filled(
        Rect::from_min_max(pos2(c.x - 4.0 * s, c.y), pos2(c.x + 4.5 * s, c.y + 4.6 * s)),
        1.5 * s,
        col,
    );
}

fn sun(p: &egui::Painter, c: egui::Pos2, r: f32, col: egui::Color32) {
    use egui::{vec2, Stroke};
    p.circle_filled(c, r, col);
    for k in 0..8 {
        let a = k as f32 * std::f32::consts::TAU / 8.0;
        let co = vec2(a.cos(), a.sin());
        p.line_segment(
            [c + co * (r + 2.4), c + co * (r + 4.6)],
            Stroke::new(1.3, col),
        );
    }
}

/// Ikon cuaca. `bg` = warna latar (untuk "memotong" bulan sabit), `scale` = skala ikon.
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
                sun(p, c, 4.2 * u, col);
            } else {
                p.circle_filled(c + vec2(-1.0, 0.0) * u, 7.0 * u, col);
                p.circle_filled(c + vec2(2.2, -2.0) * u, 6.0 * u, bg);
                p.circle_filled(c + vec2(5.5, -5.0) * u, 1.3 * u, col);
            }
        }
        2 => {
            let sc = c + vec2(3.5, -4.0) * u;
            if is_day {
                p.circle_filled(sc, 3.2 * u, col);
            } else {
                p.circle_filled(sc, 3.4 * u, col);
                p.circle_filled(sc + vec2(1.6, -1.2) * u, 2.8 * u, bg);
            }
            cloud(p, c + vec2(-1.5, 2.0) * u, 0.95 * u, mix(bg, col, 0.8));
        }
        45 | 48 => {
            cloud(p, c + vec2(0.0, -3.0) * u, 0.85 * u, mix(bg, col, 0.8));
            p.line_segment(
                [
                    pos2(c.x - 7.0 * u, c.y + 4.0 * u),
                    pos2(c.x + 7.0 * u, c.y + 4.0 * u),
                ],
                st,
            );
            p.line_segment(
                [
                    pos2(c.x - 5.0 * u, c.y + 7.5 * u),
                    pos2(c.x + 7.0 * u, c.y + 7.5 * u),
                ],
                st,
            );
        }
        51..=57 | 61..=67 | 80..=82 => {
            cloud(p, c + vec2(0.0, -3.0) * u, 0.95 * u, mix(bg, col, 0.8));
            let n = if matches!(code, 51..=57) { 2 } else { 3 };
            for k in 0..n {
                let x = c.x - 4.0 * u + k as f32 * 4.0 * u;
                p.line_segment(
                    [
                        pos2(x + 1.0 * u, c.y + 4.0 * u),
                        pos2(x - 0.5 * u, c.y + 7.5 * u),
                    ],
                    st,
                );
            }
        }
        71..=77 | 85 | 86 => {
            cloud(p, c + vec2(0.0, -3.0) * u, 0.95 * u, mix(bg, col, 0.8));
            for k in 0..3 {
                let x = c.x - 4.0 * u + k as f32 * 4.0 * u;
                p.circle_filled(
                    pos2(x, c.y + 6.0 * u + (k % 2) as f32 * 1.5 * u),
                    1.0 * u,
                    col,
                );
            }
        }
        95..=99 => {
            cloud(p, c + vec2(0.0, -3.0) * u, 0.95 * u, mix(bg, col, 0.7));
            let pts = [
                pos2(c.x + 1.0 * u, c.y + 1.5 * u),
                pos2(c.x - 2.0 * u, c.y + 5.5 * u),
                pos2(c.x + 0.5 * u, c.y + 5.5 * u),
                pos2(c.x - 1.5 * u, c.y + 9.0 * u),
            ];
            for w in pts.windows(2) {
                p.line_segment([w[0], w[1]], Stroke::new((1.6 * u).max(1.0), col));
            }
        }
        _ => cloud(p, c, 1.1 * u, col),
    }
}

fn wind_glyph(p: &egui::Painter, c: egui::Pos2, col: egui::Color32) {
    use egui::{pos2, Stroke};
    let st = Stroke::new(1.2, col);
    for (dy, len) in [(-3.0, 8.0), (0.0, 10.0), (3.0, 7.0)] {
        p.line_segment(
            [pos2(c.x - 5.0, c.y + dy), pos2(c.x - 5.0 + len, c.y + dy)],
            st,
        );
    }
}

fn drop_glyph(p: &egui::Painter, c: egui::Pos2, col: egui::Color32) {
    use egui::{pos2, Stroke};
    p.circle_filled(pos2(c.x, c.y + 1.8), 2.3, col);
    let st = Stroke::new(1.2, col);
    p.line_segment([pos2(c.x, c.y - 4.5), pos2(c.x - 2.1, c.y + 1.2)], st);
    p.line_segment([pos2(c.x, c.y - 4.5), pos2(c.x + 2.1, c.y + 1.2)], st);
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

// Satu kartu: hero 112 + header 46 + grafik 54 + hourly 64 = 276px.
const PAD: f32 = 20.0; // padding kiri/kanan kartu
const COL_INSET: f32 = 20.0; // inset kolom grafik/hourly
const R: f32 = 18.0; // radius kartu
const H_CARD: f32 = 276.0; // tinggi total kartu cuaca
const HEAD_Y: f32 = 120.0; // y judul 24-hour forecast
const CH_TOP: f32 = 158.0; // batas atas area grafik
const CH_BOT: f32 = 212.0; // batas bawah area grafik
const CH_FILL_BOT: f32 = 216.0; // dasar area fill
const Y_ICON: f32 = 238.0; // pusat ikon hourly
const Y_TIME: f32 = 260.0; // baseline jam hourly

/// Panel cuaca: hero kondisi sekarang, lalu kartu forecast 24 jam
/// (grafik suhu, ikon, dan jam berbagi kolom yang sama).
/// Mengembalikan `true` bila user minta deteksi ulang lokasi + refresh.
pub(crate) fn draw_weather_panel(ui: &mut egui::Ui, s: &WeatherState) -> bool {
    use eframe::egui::{self, pos2, vec2, Align2, Color32, FontId, Rect, Sense};

    let dim = Color32::from_rgb(140, 142, 154);
    let bright = Color32::from_rgb(238, 239, 245);
    let card_bg = Color32::from_rgb(23, 27, 36);
    let rain = Color32::from_rgb(96, 170, 255);
    let accent_soft = Color32::from_rgb(145, 185, 240);
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
    let cutoff = now
        .date_naive()
        .and_hms_opt(now.hour(), 0, 0)
        .unwrap_or_default();
    let hours: Vec<&WeatherHour> = s
        .hourly
        .iter()
        .filter(|h| h.time >= cutoff)
        .take(7)
        .collect();
    let col = weather_color(now_w.code, now_w.is_day);

    // ================= single card: hero + forecast =================
    ui.add_space(4.0);
    let (card, _) = ui.allocate_exact_size(vec2(w, H_CARD), Sense::hover());
    let p = ui.painter();
    p.rect_filled(card, R, card_bg);
    p.rect_stroke(
        card,
        R,
        egui::Stroke::new(1.0, Color32::from_rgb(38, 44, 58)),
        egui::StrokeKind::Inside,
    );

    let left = card.left() + PAD;
    let top = card.top();
    let right = card.right() - PAD;

    // --- hero left: temp / condition / location ---
    let temp_txt = format!("{:.0}°", now_w.temp);
    let temp_font = FontId::proportional(48.0);
    p.text(
        pos2(left, top + 38.0),
        Align2::LEFT_CENTER,
        &temp_txt,
        temp_font.clone(),
        bright,
    );
    let tw = crate::theme::text_w(p, &temp_txt, &temp_font);
    p.text(
        pos2(left + tw + 2.0, top + 44.0),
        Align2::LEFT_CENTER,
        "C",
        FontId::proportional(17.0),
        bright,
    );
    p.text(
        pos2(left, top + 72.0),
        Align2::LEFT_CENTER,
        format!(
            "{} · Feels like {:.0}°",
            weather_desc(now_w.code),
            now_w.feels
        ),
        FontId::proportional(13.0),
        bright,
    );

    let is_updating = matches!(s.status, WeatherStatus::Locating | WeatherStatus::Loading);
    let raw_place = if is_updating {
        "Locating…"
    } else {
        s.place.trim_start_matches("Kota ")
    };
    let loc_font = FontId::proportional(11.0);
    let (place, place_w) = crate::theme::fit(p, raw_place, loc_font.clone(), 240.0);
    let row_y = top + 96.0;
    p.circle_filled(pos2(left + 2.5, row_y), 2.4, rain);
    p.text(
        pos2(left + 10.0, row_y),
        Align2::LEFT_CENTER,
        &place,
        loc_font,
        if is_updating { rain } else { bright },
    );
    if !is_updating {
        if let Some(u) = s.updated {
            p.text(
                pos2(left + 10.0 + place_w + 6.0, row_y),
                Align2::LEFT_CENTER,
                format!("· {:02}:{:02}", u.hour(), u.minute()),
                FontId::proportional(10.0),
                dim,
            );
        }
    }

    // --- hero right: large icon + wind/humidity ---
    let stat_font = FontId::proportional(10.5);
    let hum_s = format!("{}%", now_w.humidity);
    let wind_s = format!("{:.0} km/h", now_w.wind);
    let hw = crate::theme::text_w(p, &hum_s, &stat_font);
    let ww = crate::theme::text_w(p, &wind_s, &stat_font);
    let hum_tx = right - hw;
    let wind_tx = hum_tx - 8.0 - 18.0 - ww;
    let stats_left = wind_tx - 10.0;
    let icon_c = pos2((stats_left + right) * 0.5, top + 50.0);
    for (r, a) in [(44.0, 6), (36.0, 8), (28.0, 12)] {
        p.circle_filled(
            icon_c,
            r,
            Color32::from_rgba_unmultiplied(col.r(), col.g(), col.b(), a),
        );
    }
    let icon_bg = mix(card_bg, col, 0.10);
    weather_icon(p, icon_c, now_w.code, now_w.is_day, col, icon_bg, 2.7);

    drop_glyph(p, pos2(hum_tx - 8.0, row_y), accent_soft);
    p.text(
        pos2(hum_tx, row_y),
        Align2::LEFT_CENTER,
        &hum_s,
        stat_font.clone(),
        accent_soft,
    );
    wind_glyph(p, pos2(wind_tx - 10.0, row_y), accent_soft);
    p.text(
        pos2(wind_tx, row_y),
        Align2::LEFT_CENTER,
        &wind_s,
        stat_font,
        accent_soft,
    );

    let place_rect = Rect::from_min_max(
        pos2(left, row_y - 10.0),
        pos2(left + 10.0 + place_w + 4.0, row_y + 10.0),
    );
    let resp = ui
        .interact(place_rect, ui.id().with("wx_place"), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Click to re-detect location & refresh");
    if resp.clicked() {
        retry = true;
    }

    // --- divider + forecast header ---
    p.line_segment(
        [
            pos2(card.left() + PAD, top + HEAD_Y - 6.0),
            pos2(card.right() - PAD, top + HEAD_Y - 6.0),
        ],
        egui::Stroke::new(1.0, Color32::from_rgb(36, 42, 54)),
    );
    p.text(
        pos2(card.left() + PAD, top + HEAD_Y + 8.0),
        Align2::LEFT_CENTER,
        "24-Hour Forecast",
        crate::theme::semibold(13.5),
        bright,
    );
    p.text(
        pos2(card.right() - PAD, top + HEAD_Y + 8.0),
        Align2::RIGHT_CENTER,
        format!("Today · {} {}", now.day(), month_short(now.month())),
        FontId::proportional(10.0),
        dim,
    );

    // --- grafik suhu + hourly dalam kolom yang sama ---
    if !hours.is_empty() {
        let n = hours.len();
        let cw = (card.width() - 2.0 * COL_INSET) / n as f32;
        let col_x = |i: usize| card.left() + COL_INSET + cw * (i as f32 + 0.5);

        let chart_top = top + CH_TOP;
        let chart_bot = top + CH_BOT;
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
        let mut fill = curve.clone();
        fill.push(pos2(pts[n - 1].x, top + CH_FILL_BOT));
        fill.push(pos2(pts[0].x, top + CH_FILL_BOT));
        p.add(egui::Shape::Path(egui::epaint::PathShape {
            points: fill,
            closed: true,
            fill: Color32::from_rgba_unmultiplied(96, 170, 255, 18),
            stroke: egui::epaint::PathStroke::NONE,
        }));
        p.line_segment(
            [
                pos2(card.left() + PAD, chart_bot),
                pos2(card.right() - PAD, chart_bot),
            ],
            egui::Stroke::new(1.0, Color32::from_rgb(34, 40, 52)),
        );
        p.add(egui::Shape::line(curve, egui::Stroke::new(2.5, rain)));

        for (i, (h, pt)) in hours.iter().zip(pts.iter()).enumerate() {
            let cx = col_x(i);
            p.text(
                pos2(cx, pt.y - 9.0),
                Align2::CENTER_BOTTOM,
                format!("{:.0}°", h.temp),
                FontId::proportional(10.0),
                bright,
            );
            p.circle_filled(*pt, 3.2, rain);
            p.circle_filled(*pt, 1.4, card_bg);

            let is_day = (6..18).contains(&h.time.hour());
            let c = weather_color(h.code, is_day);
            weather_icon(p, pos2(cx, top + Y_ICON), h.code, is_day, c, card_bg, 1.2);

            let time_col = if i == 0 { rain } else { dim };
            let label = if i == 0 {
                "Now".to_string()
            } else {
                h.time.format("%H:%M").to_string()
            };
            p.text(
                pos2(cx, top + Y_TIME),
                Align2::CENTER_CENTER,
                label,
                FontId::proportional(9.5),
                time_col,
            );
            if h.pop >= 50 {
                p.rect_filled(
                    Rect::from_min_size(pos2(cx - 4.0, top + Y_TIME + 7.0), vec2(8.0, 1.5)),
                    1,
                    rain,
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
