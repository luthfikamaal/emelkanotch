// =====================================================================
// Settings (appearance), stored in %APPDATA%\EmelkaNotch\settings.json
// =====================================================================

use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::Command;

const CREATE_NO_WINDOW: u32 = 0x08000000;
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";
const APPROVED_KEY: &str =
    r"HKCU\Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
const APP_NAME: &str = "EmelkaNotch";

pub(crate) fn is_autostart_enabled() -> bool {
    Command::new("reg")
        .args(["query", RUN_KEY, "/v", APP_NAME])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .is_ok_and(|o| o.status.success())
}

pub(crate) fn set_autostart(enabled: bool) -> bool {
    if enabled {
        let Ok(exe) = std::env::current_exe() else {
            return false;
        };
        let val = format!("\"{}\"", exe.display());
        let ok = Command::new("reg")
            .args(["add", RUN_KEY, "/v", APP_NAME, "/t", "REG_SZ", "/d", &val, "/f"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .is_ok_and(|o| o.status.success());
        if ok {
            let _ = Command::new("reg")
                .args(["delete", APPROVED_KEY, "/v", APP_NAME, "/f"])
                .creation_flags(CREATE_NO_WINDOW)
                .output();
        }
        ok
    } else {
        Command::new("reg")
            .args(["delete", RUN_KEY, "/v", APP_NAME, "/f"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .is_ok_and(|o| o.status.success())
    }
}

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

pub(crate) const DEFAULT_REVEAL_HEIGHT: f32 = 12.0;
pub(crate) const MIN_REVEAL_HEIGHT: f32 = 2.0;
pub(crate) const MAX_REVEAL_HEIGHT: f32 = 60.0;

#[derive(Clone, Copy)]
pub(crate) struct AppSettings {
    pub style: NotchStyle,
    pub osd: bool,
    pub auto_hide: bool,
    pub reveal_height: f32,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            style: NotchStyle::Floating,
            osd: true,
            auto_hide: false,
            reveal_height: DEFAULT_REVEAL_HEIGHT,
        }
    }
}

/// Missing / broken file -> defaults.
pub(crate) fn load_settings() -> AppSettings {
    let mut cfg = AppSettings::default();
    let Some(p) = settings_path() else {
        return cfg;
    };
    let Ok(s) = std::fs::read_to_string(p) else {
        return cfg;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) else {
        return cfg;
    };
    if v.get("style").and_then(|x| x.as_str()) == Some("attached") {
        cfg.style = NotchStyle::Attached;
    }
    if let Some(b) = v.get("osd").and_then(|x| x.as_bool()) {
        cfg.osd = b;
    }
    if let Some(b) = v.get("auto_hide").and_then(|x| x.as_bool()) {
        cfg.auto_hide = b;
    }
    if let Some(h) = v.get("reveal_height").and_then(|x| x.as_f64()) {
        cfg.reveal_height = (h as f32).clamp(MIN_REVEAL_HEIGHT, MAX_REVEAL_HEIGHT);
    }
    cfg
}

pub(crate) fn save_settings(cfg: AppSettings) {
    let Some(p) = settings_path() else { return };
    let name = if cfg.style == NotchStyle::Attached {
        "attached"
    } else {
        "floating"
    };
    let _ = std::fs::write(
        p,
        serde_json::json!({
            "style": name,
            "osd": cfg.osd,
            "auto_hide": cfg.auto_hide,
            "reveal_height": cfg.reveal_height.clamp(MIN_REVEAL_HEIGHT, MAX_REVEAL_HEIGHT).round()
        })
        .to_string(),
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
