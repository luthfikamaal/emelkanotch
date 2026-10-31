// =====================================================================
// Layout, colors, shared constants, text helpers, fonts
// =====================================================================

use chrono::{Local, Timelike};
use eframe::egui::{
    self, Color32, FontData, FontDefinitions, FontFamily, FontId, Painter, RichText,
};
use std::sync::Arc;
use std::time::Duration;

// ---------- layout ----------
pub(crate) const PANEL_W: f32 = 500.0;
pub(crate) const PANEL_H: f32 = 212.0;
pub(crate) const PRAYER_PANEL_H: f32 = 228.0;
pub(crate) const WEATHER_PANEL_H: f32 = 358.0;
pub(crate) const ANTIGRAVITY_PANEL_H: f32 = 358.0;
pub(crate) const PILL_W: f32 = 240.0;
pub(crate) const PILL_H: f32 = 34.0;
pub(crate) const NOTIF_H: f32 = 52.0;
pub(crate) const TOP: f32 = 6.0;
pub(crate) const PAD: f32 = 16.0;
pub(crate) const WIN_W: f32 = PANEL_W + 20.0;
pub(crate) const WIN_H: f32 = WEATHER_PANEL_H + TOP + 12.0;

pub(crate) const NOTIF_SHOW: Duration = Duration::from_secs(5);
pub(crate) const OSD_SHOW: Duration = Duration::from_millis(1600);
pub(crate) const UA: &str = concat!(
    "EmelkaNotch/",
    env!("CARGO_PKG_VERSION"),
    " (personal project)"
);
pub(crate) const HIST: usize = 60;
pub(crate) const VERSION: &str = env!("CARGO_PKG_VERSION");
pub(crate) const REPO: &str = "luthfikamaal/emelkanotch";

// ---------- colors ----------
pub(crate) const BG: Color32 = Color32::from_rgb(0, 0, 0);
pub(crate) const BORDER: Color32 = Color32::from_rgb(42, 44, 52);
pub(crate) const TEXT: Color32 = Color32::from_rgb(232, 234, 240);
pub(crate) const MUTED: Color32 = Color32::from_rgb(140, 146, 160);
pub(crate) const DIM: Color32 = Color32::from_rgb(78, 82, 94);
pub(crate) const ACCENT: Color32 = Color32::from_rgb(120, 170, 255);
pub(crate) const TRACK: Color32 = Color32::from_rgb(45, 47, 56);
pub(crate) const CARD: Color32 = Color32::from_rgb(15, 16, 21);
pub(crate) const PURPLE: Color32 = Color32::from_rgb(170, 140, 255);
pub(crate) const GREEN: Color32 = Color32::from_rgb(130, 220, 170);
pub(crate) const WARN: Color32 = Color32::from_rgb(255, 125, 105);

// ---------- input fields ----------
pub(crate) const FIELD_R: u8 = 8;
pub(crate) const FIELD_H: f32 = 30.0;
pub(crate) const DUE_W: f32 = 132.0;

pub(crate) fn greeting() -> String {
    match Local::now().hour() {
        5..=11 => "Good morning",
        12..=16 => "Good afternoon",
        17..=21 => "Good evening",
        _ => "Hello, night owl",
    }
    .to_string()
}

pub(crate) fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

pub(crate) fn ease(t: f32) -> f32 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

pub(crate) fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

pub(crate) fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("Inter-SemiBold".into()))
}

pub(crate) fn small(s: String) -> RichText {
    RichText::new(s).size(11.0).color(MUTED)
}

pub(crate) fn ellipsize(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n.saturating_sub(1)).collect::<String>() + "…"
    }
}

pub(crate) fn text_w(p: &Painter, s: &str, f: &FontId) -> f32 {
    p.layout_no_wrap(s.to_owned(), f.clone(), Color32::WHITE)
        .size()
        .x
}

/// Truncate with "…" so the text fits `max_w`. Returns (text, width).
pub(crate) fn fit(p: &Painter, s: &str, f: FontId, max_w: f32) -> (String, f32) {
    let w = text_w(p, s, &f);
    if w <= max_w {
        return (s.to_string(), w);
    }
    let chars: Vec<char> = s.chars().collect();
    let (mut lo, mut hi) = (1usize, chars.len());
    while lo < hi {
        let mid = (lo + hi + 1) / 2;
        let cand = chars[..mid].iter().collect::<String>() + "…";
        if text_w(p, &cand, &f) <= max_w {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    let out = chars[..lo]
        .iter()
        .collect::<String>()
        .trim_end()
        .to_string()
        + "…";
    let w = text_w(p, &out, &f);
    (out, w)
}

pub(crate) fn one_line(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
}

pub(crate) fn ago(t: Instant) -> String {
    let s = t.elapsed().as_secs();
    if s < 60 {
        "now".into()
    } else if s < 3600 {
        format!("{}m", s / 60)
    } else {
        format!("{}h", s / 3600)
    }
}

use std::time::Instant;

pub(crate) fn setup_fonts(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    fonts.font_data.insert(
        "Inter".to_owned(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/Inter-Regular.ttf"
        ))),
    );
    fonts.font_data.insert(
        "Inter-SemiBold".to_owned(),
        Arc::new(FontData::from_static(include_bytes!(
            "../assets/Inter-SemiBold.ttf"
        ))),
    );
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .insert(0, "Inter".to_owned());
    fonts.families.insert(
        FontFamily::Name("Inter-SemiBold".into()),
        vec!["Inter-SemiBold".to_owned(), "Inter".to_owned()],
    );
    ctx.set_fonts(fonts);
}
