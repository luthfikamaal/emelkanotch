// =====================================================================
// Prayer times (equran.id) + device location
// =====================================================================

use chrono::{Datelike, Local, NaiveDate, Timelike};
use eframe::egui;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::notification::{push_notice, NotifState};
use crate::theme::UA;

pub(crate) const PRAYER_NAMES: [&str; 8] = [
    "Imsak", "Subuh", "Terbit", "Dhuha", "Dzuhur", "Ashar", "Maghrib", "Isya",
];
pub(crate) const MAIN_PRAYERS: [usize; 5] = [1, 4, 5, 6, 7];

pub(crate) fn prayer_color(i: usize) -> eframe::egui::Color32 {
    use eframe::egui::Color32;
    match i {
        0 => Color32::from_rgb(140, 150, 255), // Imsak
        1 => Color32::from_rgb(96, 170, 255),  // Subuh
        2 => Color32::from_rgb(255, 176, 120), // Terbit
        3 => Color32::from_rgb(255, 222, 110), // Dhuha
        4 => Color32::from_rgb(255, 208, 80),  // Dzuhur
        5 => Color32::from_rgb(255, 146, 80),  // Ashar
        6 => Color32::from_rgb(255, 98, 150),  // Maghrib
        _ => Color32::from_rgb(180, 138, 255), // Isya
    }
}

/// Menit sejak 00:00 untuk: imsak, subuh, terbit, dhuha, dzuhur, ashar, maghrib, isya.
#[derive(Clone, Copy)]
pub(crate) struct PrayerDay {
    pub date: NaiveDate,
    pub t: [u32; 8],
}

#[derive(Clone, Default, PartialEq)]
pub(crate) enum PrayerStatus {
    #[default]
    Locating,
    Loading,
    Ready,
    Error(String),
}

#[derive(Default)]
pub(crate) struct PrayerState {
    pub status: PrayerStatus,
    pub provinsi: String,
    pub kabkota: String,
    pub days: Vec<PrayerDay>,
    pub retry: bool, // dari UI: paksa deteksi ulang lokasi
}

fn prayer_cache_path() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("APPDATA")?).join("EmelkaNotch");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join("prayer_cache.json"))
}

pub(crate) fn load_prayer_cache() -> PrayerState {
    let mut st = PrayerState::default();
    let Some(p) = prayer_cache_path() else {
        return st;
    };
    let Ok(s) = std::fs::read_to_string(p) else {
        return st;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) else {
        return st;
    };
    st.provinsi = v
        .get("provinsi")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    st.kabkota = v
        .get("kabkota")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    if let Some(arr) = v.get("days").and_then(|x| x.as_array()) {
        for d in arr {
            let Some(date) = d
                .get("d")
                .and_then(|x| x.as_str())
                .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
            else {
                continue;
            };
            let Some(ts) = d.get("t").and_then(|x| x.as_array()) else {
                continue;
            };
            if ts.len() != 8 {
                continue;
            }
            let mut t = [0u32; 8];
            for (i, x) in ts.iter().enumerate() {
                t[i] = x.as_u64().unwrap_or(0) as u32;
            }
            st.days.push(PrayerDay { date, t });
        }
    }
    if !st.days.is_empty() {
        st.status = PrayerStatus::Ready;
    }
    st
}

pub(crate) fn save_prayer_cache(s: &PrayerState) {
    let Some(p) = prayer_cache_path() else { return };
    let days: Vec<_> = s
        .days
        .iter()
        .map(|d| serde_json::json!({ "d": d.date.format("%Y-%m-%d").to_string(), "t": d.t }))
        .collect();
    let _ = std::fs::write(
        p,
        serde_json::json!({ "provinsi": s.provinsi, "kabkota": s.kabkota, "days": days })
            .to_string(),
    );
}

pub(crate) fn fmt_hm(m: u32) -> String {
    format!("{:02}:{:02}", (m / 60) % 24, m % 60)
}

/// "1j 23m" kalau >= 1 jam, selain itu "12m 05d".
#[allow(dead_code)]
pub(crate) fn fmt_countdown(secs: u32) -> String {
    let h = secs / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if h > 0 {
        format!("{}j {:02}m", h, m)
    } else {
        format!("{}m {:02}d", m, s)
    }
}

fn parse_hm(s: &str) -> Option<u32> {
    let (h, m) = s.split_once(':')?;
    Some(h.trim().parse::<u32>().ok()? * 60 + m.trim().parse::<u32>().ok()?)
}

// ---------- equran.id ----------

fn equran_get(path: &str) -> Result<serde_json::Value, String> {
    ureq::get(&format!("https://equran.id/api/v2/{path}"))
        .set("User-Agent", UA)
        .timeout(Duration::from_secs(10))
        .call()
        .map_err(|e| e.to_string())?
        .into_json::<serde_json::Value>()
        .map_err(|e| e.to_string())
}

fn equran_post(path: &str, body: serde_json::Value) -> Result<serde_json::Value, String> {
    ureq::post(&format!("https://equran.id/api/v2/{path}"))
        .set("User-Agent", UA)
        .timeout(Duration::from_secs(15))
        .send_json(body)
        .map_err(|e| e.to_string())?
        .into_json::<serde_json::Value>()
        .map_err(|e| e.to_string())
}

fn json_strings(v: &serde_json::Value) -> Vec<String> {
    v.get("data")
        .and_then(|d| d.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

fn fetch_month(prov: &str, kab: &str, year: i32, month: u32) -> Result<Vec<PrayerDay>, String> {
    let v = equran_post(
        "shalat",
        serde_json::json!({ "provinsi": prov, "kabkota": kab, "bulan": month, "tahun": year }),
    )?;
    let arr = v
        .get("data")
        .and_then(|d| d.get("jadwal"))
        .and_then(|j| j.as_array())
        .ok_or("Format jadwal tidak dikenali")?;
    let keys = [
        "imsak", "subuh", "terbit", "dhuha", "dzuhur", "ashar", "maghrib", "isya",
    ];
    let mut out = Vec::new();
    for it in arr {
        let Some(date) = it
            .get("tanggal_lengkap")
            .and_then(|x| x.as_str())
            .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok())
        else {
            continue;
        };
        let mut t = [0u32; 8];
        let mut ok = true;
        for (i, k) in keys.iter().enumerate() {
            match it.get(*k).and_then(|x| x.as_str()).and_then(parse_hm) {
                Some(m) => t[i] = m,
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            out.push(PrayerDay { date, t });
        }
    }
    if out.is_empty() {
        Err("Jadwal kosong dari server".into())
    } else {
        Ok(out)
    }
}

// ---------- location ----------

fn win_coords() -> Option<(f64, f64)> {
    use windows::Devices::Geolocation::Geolocator;
    use windows::Foundation::TimeSpan;
    let g = Geolocator::new().ok()?;
    let pos = g
        .GetGeopositionAsyncWithAgeAndTimeout(
            TimeSpan {
                Duration: 30 * 60 * 10_000_000,
            }, // cache boleh sampai 30 menit
            TimeSpan {
                Duration: 10 * 10_000_000,
            }, // timeout 10 detik
        )
        .ok()?
        .get()
        .ok()?;
    let p = pos.Coordinate().ok()?.Point().ok()?.Position().ok()?;
    Some((p.Latitude, p.Longitude))
}

fn ip_coords() -> Option<(f64, f64)> {
    let v: serde_json::Value = ureq::get("https://ipwho.is/")
        .set("User-Agent", UA)
        .timeout(Duration::from_secs(8))
        .call()
        .ok()?
        .into_json()
        .ok()?;
    if v.get("success").and_then(|x| x.as_bool()) == Some(false) {
        return None;
    }
    Some((v.get("latitude")?.as_f64()?, v.get("longitude")?.as_f64()?))
}

/// (provinsi, kandidat kab/kota + apakah "kota"(true) / "kabupaten"(false) / tidak diketahui)
fn reverse_geocode(lat: f64, lon: f64) -> Result<(String, Vec<(String, Option<bool>)>), String> {
    let v: serde_json::Value = ureq::get("https://nominatim.openstreetmap.org/reverse")
        .set("User-Agent", UA)
        .set("Accept-Language", "id")
        .query("format", "jsonv2")
        .query("lat", &lat.to_string())
        .query("lon", &lon.to_string())
        .query("zoom", "10")
        .query("addressdetails", "1")
        .timeout(Duration::from_secs(10))
        .call()
        .map_err(|e| e.to_string())?
        .into_json()
        .map_err(|e| e.to_string())?;
    let a = v.get("address").ok_or("Alamat tidak ditemukan")?;
    let state = a
        .get("state")
        .or_else(|| a.get("province"))
        .and_then(|x| x.as_str())
        .ok_or("Provinsi tidak terdeteksi (di luar Indonesia?)")?
        .to_string();
    let mut c = Vec::new();
    for (key, default_kind) in [
        ("city", Some(true)),
        ("town", Some(true)),
        ("county", Some(false)),
        ("regency", Some(false)),
        ("municipality", None),
        ("state_district", None),
        ("city_district", None),
    ] {
        if let Some(s) = a.get(key).and_then(|x| x.as_str()) {
            let l = s.to_lowercase();
            let kind = if l.contains("kabupaten") || l.contains("regency") || l.starts_with("kab") {
                Some(false)
            } else if l.starts_with("kota") || l.contains("city") {
                Some(true)
            } else {
                default_kind
            };
            c.push((s.to_string(), kind));
        }
    }
    Ok((state, c))
}

fn norm_place(s: &str) -> String {
    const STOP: [&str; 20] = [
        "kab",
        "kabupaten",
        "kota",
        "city",
        "regency",
        "administrasi",
        "provinsi",
        "province",
        "daerah",
        "khusus",
        "ibukota",
        "istimewa",
        "dki",
        "di",
        "d",
        "i",
        "special",
        "region",
        "capital",
        "of",
    ];
    let cleaned: String = s
        .to_lowercase()
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();
    cleaned
        .split_whitespace()
        .filter(|w| !STOP.contains(w))
        .collect::<Vec<_>>()
        .join(" ")
}

fn score_name(a: &str, b: &str) -> i32 {
    if a.is_empty() || b.is_empty() {
        return 0;
    }
    if a == b {
        return 100;
    }
    if a.contains(b) || b.contains(a) {
        return 50;
    }
    a.split(' ')
        .filter(|w| b.split(' ').any(|x| x == *w))
        .count() as i32
        * 10
}

fn match_province(list: &[String], state: &str) -> Option<String> {
    let t = norm_place(state);
    list.iter()
        .map(|p| (score_name(&norm_place(p), &t), p))
        .filter(|(s, _)| *s > 0)
        .max_by_key(|(s, _)| *s)
        .map(|(_, p)| p.clone())
}

fn match_kabkota(list: &[String], cands: &[(String, Option<bool>)]) -> Option<String> {
    let mut best: (i32, Option<&String>) = (0, None);
    for k in list {
        let kn = norm_place(k);
        let k_city = k.to_lowercase().starts_with("kota");
        for (i, (c, kind)) in cands.iter().enumerate() {
            let mut s = score_name(&kn, &norm_place(c));
            if s == 0 {
                continue;
            }
            if *kind == Some(k_city) {
                s += 10; // "Kota Bandung" vs "Kab. Bandung"
            }
            s -= i as i32; // kandidat yang lebih spesifik menang saat seri
            if s > best.0 {
                best = (s, Some(k));
            }
        }
    }
    best.1.cloned()
}

fn resolve_location() -> Result<(String, String), String> {
    let (lat, lon) = win_coords()
        .or_else(ip_coords)
        .ok_or("Lokasi perangkat tidak tersedia")?;
    let (state, cands) = reverse_geocode(lat, lon)?;
    let provs = json_strings(&equran_get("shalat/provinsi")?);
    let prov = match_province(&provs, &state).ok_or("Provinsi tidak ada di data equran.id")?;
    let kabs = json_strings(&equran_post(
        "shalat/kabkota",
        serde_json::json!({ "provinsi": prov }),
    )?);
    let kab = match_kabkota(&kabs, &cands).ok_or("Kab/kota tidak cocok dengan data equran.id")?;
    Ok((prov, kab))
}

// =====================================================================
// UI
// =====================================================================

/// Ikon kecil per waktu sholat (semua 8 waktu sekarang punya ikon).
/// `bg` = warna latar (dipakai untuk "memotong" bulan sabit).
pub(crate) fn prayer_icon(
    p: &eframe::egui::Painter,
    c: eframe::egui::Pos2,
    idx: usize,
    col: eframe::egui::Color32,
    bg: eframe::egui::Color32,
) {
    use eframe::egui::{pos2, vec2, Rect, Stroke};
    use std::f32::consts::PI;
    let st = Stroke::new(1.4, col);
    match idx {
        // Imsak: bulan sabit kecil + garis fajar tipis
        0 => {
            p.circle_filled(c + vec2(-1.0, -1.0), 5.6, col);
            p.circle_filled(c + vec2(1.4, -2.6), 4.8, bg);
            p.circle_filled(c + vec2(5.0, -5.5), 1.0, col);
            p.line_segment([pos2(c.x - 8.0, c.y + 7.0), pos2(c.x + 8.0, c.y + 7.0)], st);
        }
        // Subuh (sunrise) / Maghrib (sunset)
        1 | 6 => {
            let hz = c.y + 2.0;
            let sc = pos2(c.x, hz);
            let pc = p.with_clip_rect(Rect::from_min_max(
                pos2(c.x - 14.0, c.y - 14.0),
                pos2(c.x + 14.0, hz),
            ));
            pc.circle_filled(sc, 5.0, col);
            for k in 1..6 {
                let a = PI + k as f32 * PI / 6.0;
                let (s, co) = a.sin_cos();
                pc.line_segment(
                    [sc + vec2(co * 7.0, s * 7.0), sc + vec2(co * 9.5, s * 9.5)],
                    Stroke::new(1.3, col),
                );
            }
            p.line_segment([pos2(c.x - 9.0, hz + 1.0), pos2(c.x + 9.0, hz + 1.0)], st);
            let d = if idx == 1 { -1.0 } else { 1.0 };
            let y0 = hz + 5.5;
            p.line_segment([pos2(c.x - 2.5, y0 - d * 1.5), pos2(c.x, y0 + d * 1.5)], st);
            p.line_segment([pos2(c.x + 2.5, y0 - d * 1.5), pos2(c.x, y0 + d * 1.5)], st);
        }
        // Terbit: setengah matahari di cakrawala + panah ke atas
        2 => {
            let hz = c.y + 4.0;
            let sc = pos2(c.x, hz);
            let pc = p.with_clip_rect(Rect::from_min_max(
                pos2(c.x - 14.0, c.y - 14.0),
                pos2(c.x + 14.0, hz),
            ));
            pc.circle_filled(sc, 4.4, col);
            for k in 1..4 {
                let a = PI + k as f32 * PI / 4.0;
                let (s, co) = a.sin_cos();
                pc.line_segment(
                    [sc + vec2(co * 6.4, s * 6.4), sc + vec2(co * 8.8, s * 8.8)],
                    Stroke::new(1.3, col),
                );
            }
            p.line_segment([pos2(c.x - 9.0, hz + 1.0), pos2(c.x + 9.0, hz + 1.0)], st);
            // panah naik
            p.line_segment([pos2(c.x, hz + 7.0), pos2(c.x, hz + 3.0)], st);
            p.line_segment([pos2(c.x - 2.2, hz + 5.0), pos2(c.x, hz + 3.0)], st);
            p.line_segment([pos2(c.x + 2.2, hz + 5.0), pos2(c.x, hz + 3.0)], st);
        }
        // Dhuha: matahari kecil di kiri-atas + sinar pendek (pagi cerah)
        3 => {
            let sc = c + vec2(-1.5, -1.5);
            p.circle_filled(sc, 3.6, col);
            for k in 0..6 {
                let a = k as f32 * PI / 3.0 + PI / 6.0;
                let (s, co) = a.sin_cos();
                p.line_segment(
                    [sc + vec2(co * 5.6, s * 5.6), sc + vec2(co * 7.6, s * 7.6)],
                    Stroke::new(1.3, col),
                );
            }
            // garis tanah
            p.line_segment([pos2(c.x - 8.0, c.y + 8.0), pos2(c.x + 8.0, c.y + 8.0)], st);
        }
        // Dzuhur: matahari penuh
        4 => {
            p.circle_filled(c, 4.2, col);
            for k in 0..8 {
                let a = k as f32 * PI / 4.0;
                let (s, co) = a.sin_cos();
                p.line_segment(
                    [c + vec2(co * 6.6, s * 6.6), c + vec2(co * 9.0, s * 9.0)],
                    Stroke::new(1.4, col),
                );
            }
        }
        // Ashar: matahari outline
        5 => {
            p.circle_stroke(c, 4.0, st);
            p.circle_filled(c, 1.5, col);
            for k in 0..8 {
                let a = k as f32 * PI / 4.0 + PI / 8.0;
                let (s, co) = a.sin_cos();
                p.line_segment(
                    [c + vec2(co * 6.6, s * 6.6), c + vec2(co * 8.6, s * 8.6)],
                    Stroke::new(1.3, col),
                );
            }
        }
        // Isya: bulan sabit + bintang
        7 => {
            p.circle_filled(c + vec2(-1.0, 0.0), 7.0, col);
            p.circle_filled(c + vec2(2.2, -2.0), 6.0, bg);
            p.circle_filled(c + vec2(5.5, -5.0), 1.3, col);
            p.circle_filled(c + vec2(3.0, 4.8), 0.9, col);
        }
        _ => {}
    }
}

/// Waktu sholat utama berikutnya: (index, sisa detik). Lewat Isya -> Subuh besok.
#[allow(dead_code)]
pub(crate) fn next_prayer(s: &PrayerState) -> Option<(usize, u32)> {
    let now = Local::now();
    let today = now.date_naive();
    let secs = now.num_seconds_from_midnight();
    if let Some(d) = s.days.iter().find(|d| d.date == today) {
        for i in MAIN_PRAYERS {
            let at = d.t[i] * 60;
            if at > secs {
                return Some((i, at - secs));
            }
        }
    }
    let tm = today.succ_opt()?;
    let d = s.days.iter().find(|d| d.date == tm)?;
    Some((1, 86_400 - secs + d.t[1] * 60))
}

fn mix(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    egui::Color32::from_rgb(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()))
}

/// Panel jadwal siap pakai: header lokasi + hitung mundur, lalu 8 baris waktu.
/// Waktu utama tampil terang, waktu sunnah/penanda lebih redup,
/// yang berikutnya di-highlight dengan warna waktunya.
/// Pemakaian: `draw_prayer_panel(ui, &state.lock().unwrap());`
#[allow(dead_code)]
pub(crate) fn draw_prayer_panel(ui: &mut egui::Ui, s: &PrayerState) {
    use egui::{vec2, Align2, Color32, FontId, Sense};

    let dim = Color32::from_rgb(140, 142, 154);
    match &s.status {
        PrayerStatus::Locating => {
            ui.colored_label(dim, "Mendeteksi lokasi…");
            return;
        }
        PrayerStatus::Loading => {
            ui.colored_label(dim, "Memuat jadwal…");
            return;
        }
        PrayerStatus::Error(e) => {
            ui.colored_label(Color32::from_rgb(255, 120, 120), e.as_str());
            return;
        }
        PrayerStatus::Ready => {}
    }

    let now = Local::now();
    let today = now.date_naive();
    let secs = now.num_seconds_from_midnight();
    let Some(day) = s.days.iter().find(|d| d.date == today) else {
        ui.colored_label(dim, "Jadwal hari ini belum tersedia");
        return;
    };
    let next = next_prayer(s);

    let base_bg = Color32::from_rgb(24, 24, 28);
    let w = ui.available_width();

    // ----- header -----
    let (hr, _) = ui.allocate_exact_size(vec2(w, 38.0), Sense::hover());
    let p = ui.painter();
    p.text(
        hr.left_top() + vec2(4.0, 3.0),
        Align2::LEFT_TOP,
        s.kabkota.trim_start_matches("Kota "),
        FontId::proportional(13.0),
        Color32::from_rgb(235, 236, 242),
    );
    p.text(
        hr.left_top() + vec2(4.0, 21.0),
        Align2::LEFT_TOP,
        &s.provinsi,
        FontId::proportional(10.5),
        dim,
    );
    if let Some((i, left)) = next {
        let col = prayer_color(i);
        p.text(
            hr.right_top() + vec2(-4.0, 3.0),
            Align2::RIGHT_TOP,
            fmt_countdown(left),
            FontId::monospace(15.0),
            col,
        );
        p.text(
            hr.right_top() + vec2(-4.0, 21.0),
            Align2::RIGHT_TOP,
            format!("menuju {}", PRAYER_NAMES[i]),
            FontId::proportional(10.5),
            dim,
        );
    }
    ui.add_space(4.0);

    // ----- rows -----
    for i in 0..8 {
        let (rect, _) = ui.allocate_exact_size(vec2(w, 30.0), Sense::hover());
        let p = ui.painter();
        let col = prayer_color(i);
        let main = MAIN_PRAYERS.contains(&i);
        let is_next = next.map_or(false, |(n, _)| n == i);
        let passed = day.t[i] * 60 <= secs;

        let row_bg = if is_next {
            mix(base_bg, col, 0.16)
        } else {
            base_bg
        };
        p.rect_filled(rect.shrink2(vec2(0.0, 1.0)), 8.0, row_bg);
        if is_next {
            // aksen vertikal di kiri
            let bar = egui::Rect::from_min_size(
                rect.left_top() + vec2(0.0, 7.0),
                vec2(3.0, rect.height() - 14.0),
            );
            p.rect_filled(bar, 1.5, col);
        }

        // intensitas: utama terang, lainnya redup, yang sudah lewat makin redup
        let strength = match (main, passed && !is_next) {
            (true, false) => 1.0,
            (true, true) => 0.55,
            (false, false) => 0.7,
            (false, true) => 0.4,
        };
        let icon_col = mix(row_bg, col, strength);
        let text_col = mix(row_bg, Color32::from_rgb(238, 239, 245), strength);

        prayer_icon(p, rect.left_center() + vec2(20.0, 0.0), i, icon_col, row_bg);
        p.text(
            rect.left_center() + vec2(42.0, 0.0),
            Align2::LEFT_CENTER,
            PRAYER_NAMES[i],
            FontId::proportional(if main { 13.5 } else { 12.0 }),
            text_col,
        );
        p.text(
            rect.right_center() + vec2(-12.0, 0.0),
            Align2::RIGHT_CENTER,
            fmt_hm(day.t[i]),
            FontId::monospace(if main { 14.0 } else { 12.0 }),
            if is_next { col } else { text_col },
        );
    }
}

// ---------- workers ----------

fn merge_days(s: &mut PrayerState, new: Vec<PrayerDay>, today: NaiveDate) {
    s.days.retain(|d| !new.iter().any(|n| n.date == d.date));
    s.days.extend(new);
    s.days.sort_by_key(|d| d.date);
    let keep = today - chrono::Duration::days(1);
    s.days.retain(|d| d.date >= keep);
}

fn wait_retry(st: &Arc<Mutex<PrayerState>>, secs: u32) {
    for _ in 0..secs {
        thread::sleep(Duration::from_secs(1));
        if st.lock().unwrap().retry {
            break;
        }
    }
}

pub(crate) fn spawn_prayer_worker(ctx: egui::Context, st: Arc<Mutex<PrayerState>>) {
    thread::spawn(move || {
        let mut loc: Option<(String, String)> = {
            let s = st.lock().unwrap();
            if s.kabkota.is_empty() {
                None
            } else {
                Some((s.provinsi.clone(), s.kabkota.clone()))
            }
        };
        let mut loc_checked: Option<Instant> = None;
        let mut next_tried: Option<NaiveDate> = None;

        loop {
            let force = std::mem::take(&mut st.lock().unwrap().retry);
            let due = loc.is_none()
                || force
                || loc_checked.map_or(true, |t| t.elapsed() > Duration::from_secs(1800));

            if due {
                if loc.is_none() || force {
                    st.lock().unwrap().status = PrayerStatus::Locating;
                    ctx.request_repaint();
                }
                match resolve_location() {
                    Ok(l) => {
                        if loc.as_ref() != Some(&l) {
                            next_tried = None;
                        }
                        loc = Some(l);
                        loc_checked = Some(Instant::now());
                    }
                    Err(e) => {
                        if loc.is_none() {
                            st.lock().unwrap().status = PrayerStatus::Error(e);
                            ctx.request_repaint();
                            wait_retry(&st, 30);
                            continue;
                        }
                        // pakai lokasi lama, coba lagi ~5 menit lagi
                        loc_checked = Instant::now().checked_sub(Duration::from_secs(1500));
                    }
                }
            }

            let (prov, kab) = loc.clone().unwrap();
            let today = Local::now().date_naive();
            let (changed, has_today) = {
                let s = st.lock().unwrap();
                (
                    s.provinsi != prov || s.kabkota != kab,
                    s.days.iter().any(|d| d.date == today),
                )
            };

            if changed || !has_today {
                {
                    let mut s = st.lock().unwrap();
                    if changed {
                        s.days.clear();
                        s.provinsi = prov.clone();
                        s.kabkota = kab.clone();
                    }
                    if s.days.is_empty() {
                        s.status = PrayerStatus::Loading;
                    }
                }
                ctx.request_repaint();
                match fetch_month(&prov, &kab, today.year(), today.month()) {
                    Ok(days) => {
                        let mut s = st.lock().unwrap();
                        merge_days(&mut s, days, today);
                        s.status = PrayerStatus::Ready;
                        save_prayer_cache(&s);
                    }
                    Err(e) => {
                        {
                            let mut s = st.lock().unwrap();
                            if s.days.is_empty() {
                                s.status = PrayerStatus::Error(e);
                            }
                        }
                        ctx.request_repaint();
                        wait_retry(&st, 30);
                        continue;
                    }
                }
                ctx.request_repaint();
            }

            // Akhir bulan: ambil bulan depan supaya Subuh besok tetap akurat.
            let tm = today.succ_opt().unwrap_or(today);
            let need_next =
                !st.lock().unwrap().days.iter().any(|d| d.date == tm) && next_tried != Some(today);
            if need_next {
                next_tried = Some(today);
                if let Ok(days) = fetch_month(&prov, &kab, tm.year(), tm.month()) {
                    let mut s = st.lock().unwrap();
                    merge_days(&mut s, days, today);
                    save_prayer_cache(&s);
                }
            }

            wait_retry(&st, 20);
        }
    });
}

/// Kirim notifikasi ke pill saat waktu sholat masuk.
pub(crate) fn spawn_prayer_notifier(
    ctx: egui::Context,
    st: Arc<Mutex<PrayerState>>,
    notifs: Arc<Mutex<NotifState>>,
) {
    thread::spawn(move || {
        let mut fired: HashSet<(NaiveDate, usize)> = HashSet::new();
        loop {
            thread::sleep(Duration::from_secs(1));
            let now = Local::now();
            let today = now.date_naive();
            let secs = now.num_seconds_from_midnight();
            fired.retain(|(d, _)| *d >= today);

            let hit = {
                let s = st.lock().unwrap();
                s.days
                    .iter()
                    .find(|d| d.date == today)
                    .map(|d| (d.t, s.kabkota.clone()))
            };
            let Some((t, kab)) = hit else { continue };
            for i in MAIN_PRAYERS {
                let at = t[i] * 60;
                // jendela 90 detik, jadi tidak spam kalau app baru dibuka lewat dari waktunya
                if secs >= at && secs < at + 90 && fired.insert((today, i)) {
                    push_notice(
                        &notifs,
                        &format!("Waktu {}", PRAYER_NAMES[i]),
                        format!("{} · {}", fmt_hm(t[i]), kab.trim_start_matches("Kota ")),
                    );
                    ctx.request_repaint();
                }
            }
        }
    });
}
