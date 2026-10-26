// =====================================================================
// Settings (appearance), stored in %APPDATA%\EmelkaNotch\settings.json
// =====================================================================

use std::path::PathBuf;

/// Floating = pill floating below the top edge. Attached = glued to the top edge of the screen.
#[derive(Clone, Copy, PartialEq)]
pub(crate) enum NotchStyle {
    Floating,
    Attached,
}

fn settings_path() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("APPDATA")?).join("EmelkaNotch");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join("settings.json"))
}

/// (style, osd_enabled). Missing / broken file -> defaults.
pub(crate) fn load_settings() -> (NotchStyle, bool) {
    let mut style = NotchStyle::Floating;
    let mut osd = true;
    let Some(p) = settings_path() else {
        return (style, osd);
    };
    let Ok(s) = std::fs::read_to_string(p) else {
        return (style, osd);
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) else {
        return (style, osd);
    };
    if v.get("style").and_then(|x| x.as_str()) == Some("attached") {
        style = NotchStyle::Attached;
    }
    if let Some(b) = v.get("osd").and_then(|x| x.as_bool()) {
        osd = b;
    }
    (style, osd)
}

pub(crate) fn save_settings(style: NotchStyle, osd: bool) {
    let Some(p) = settings_path() else { return };
    let name = if style == NotchStyle::Attached {
        "attached"
    } else {
        "floating"
    };
    let _ = std::fs::write(
        p,
        serde_json::json!({ "style": name, "osd": osd }).to_string(),
    );
}

pub(crate) const STYLE_ANIM_SECS: f32 = 0.1;

fn ease_in_out_cubic(x: f32) -> f32 {
    if x < 0.5 {
        4.0 * x * x * x
    } else {
        1.0 - (-2.0 * x + 2.0).powi(3) / 2.0
    }
}

pub(crate) fn style_progress(ctx: &eframe::egui::Context, style: NotchStyle) -> f32 {
    let raw = ctx.animate_bool_with_time(
        eframe::egui::Id::new("notch_style_anim"),
        style == NotchStyle::Attached,
        STYLE_ANIM_SECS,
    );
    ease_in_out_cubic(raw)
}

pub(crate) fn style_animating(t: f32) -> bool {
    t > 0.001 && t < 0.999
}
