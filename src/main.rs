#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use chrono::{Datelike, Local, NaiveDate, TimeZone, Timelike};
use eframe::egui::{
    self, pos2, vec2, Align2, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId,
    Painter, Pos2, Rect, RichText, Sense, Shape, Stroke, StrokeKind, Ui, UiBuilder,
};
use rusqlite::{types::ValueRef, Connection};
use std::collections::{HashMap, VecDeque};
use std::f32::consts::PI;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use sysinfo::{Disks, System};
use windows::core::PCWSTR;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession, GlobalSystemMediaTransportControlsSessionManager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus,
};
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HGDIOBJ,
};
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, IBindCtx, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Shell::{
    IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY,
};
use windows_sys::Win32::Foundation::POINT;
use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;

// ---------- layout ----------
const PANEL_W: f32 = 500.0;
const PANEL_H: f32 = 212.0;
const PILL_W: f32 = 240.0;
const PILL_H: f32 = 34.0;
const NOTIF_H: f32 = 52.0;
const TOP: f32 = 6.0;
const PAD: f32 = 16.0;
const WIN_W: f32 = PANEL_W + 20.0;
const WIN_H: f32 = PANEL_H + TOP + 12.0;

// [offset] 1/8: now i64 so per-song / per-app offsets can be added (and be negative)
const LYRIC_LEAD_MS: i64 = 300;
const NOTIF_SHOW: Duration = Duration::from_secs(5);
const OSD_SHOW: Duration = Duration::from_millis(1600);
const UA: &str = "EmelkaNotch/0.5 (personal project)";
const HIST: usize = 60;
const VERSION: &str = env!("CARGO_PKG_VERSION");
const REPO: &str = "luthfikamaal/emelkanotch";

// ---------- colors ----------
const BG: Color32 = Color32::from_rgb(9, 9, 12);
const BORDER: Color32 = Color32::from_rgb(42, 44, 52);
const TEXT: Color32 = Color32::from_rgb(232, 234, 240);
const MUTED: Color32 = Color32::from_rgb(140, 146, 160);
const DIM: Color32 = Color32::from_rgb(78, 82, 94);
const ACCENT: Color32 = Color32::from_rgb(120, 170, 255);
const TRACK: Color32 = Color32::from_rgb(45, 47, 56);
const CARD: Color32 = Color32::from_rgb(15, 16, 21);
const PURPLE: Color32 = Color32::from_rgb(170, 140, 255);
const GREEN: Color32 = Color32::from_rgb(130, 220, 170);
const WARN: Color32 = Color32::from_rgb(255, 125, 105);

// ---------- input fields (task row) ----------
const FIELD_H: f32 = 30.0;
const FIELD_R: u8 = 8;
const DUE_W: f32 = 132.0;

// =====================================================================
// Shared media state (filled by background thread)
// =====================================================================

#[derive(Clone, Default)]
struct Media {
    available: bool,
    title: String,
    artist: String,
    app: String,
    playing: bool,
    pos_ms: u64,
    dur_ms: u64,
}

#[derive(Clone, Default)]
enum Lyrics {
    #[default]
    Idle,
    Loading,
    NotFound,
    Synced(Arc<Vec<(u64, String)>>),
}

#[derive(Default)]
struct Shared {
    media: Media,
    snapshot: Option<Instant>,
    lyrics: Lyrics,
    lyrics_key: String,
    cache: HashMap<String, Lyrics>,
}

enum Cmd {
    PlayPause,
    Next,
    Prev,
    Seek(u64),
}

fn read_media(s: &GlobalSystemMediaTransportControlsSession) -> windows::core::Result<Media> {
    let props = s.TryGetMediaPropertiesAsync()?.get()?;
    let title = props.Title()?.to_string();
    let artist = props.Artist()?.to_string();
    let app = s.SourceAppUserModelId()?.to_string();
    let playing = s.GetPlaybackInfo()?.PlaybackStatus()?
        == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing;

    let tl = s.GetTimelineProperties()?;
    let start = tl.StartTime()?.Duration;
    let end = tl.EndTime()?.Duration;
    let dur = (end - start).max(0);
    let mut pos = (tl.Position()?.Duration - start).max(0);

    // Position is a snapshot at LastUpdatedTime; add the time elapsed since then.
    if playing {
        let last = tl.LastUpdatedTime()?.UniversalTime; // 100ns since 1601-01-01
        if last > 0 {
            let now = (SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
                / 100) as i64
                + 116_444_736_000_000_000;
            pos += (now - last).max(0);
        }
    }
    if dur > 0 {
        pos = pos.min(dur);
    }

    Ok(Media {
        available: !title.trim().is_empty(),
        title,
        artist,
        app,
        playing,
        pos_ms: (pos / 10_000) as u64,
        dur_ms: (dur / 10_000) as u64,
    })
}

fn spawn_media_worker(ctx: egui::Context, shared: Arc<Mutex<Shared>>) -> mpsc::Sender<Cmd> {
    let (tx, rx) = mpsc::channel::<Cmd>();
    thread::spawn(move || {
        let mgr = loop {
            match GlobalSystemMediaTransportControlsSessionManager::RequestAsync()
                .and_then(|o| o.get())
            {
                Ok(m) => break m,
                Err(_) => thread::sleep(Duration::from_secs(2)),
            }
        };

        loop {
            let session = mgr.GetCurrentSession().ok();

            match rx.recv_timeout(Duration::from_millis(250)) {
                Ok(cmd) => {
                    if let Some(s) = &session {
                        match cmd {
                            Cmd::PlayPause => {
                                let _ = s.TryTogglePlayPauseAsync().and_then(|o| o.get());
                            }
                            Cmd::Next => {
                                let _ = s.TrySkipNextAsync().and_then(|o| o.get());
                            }
                            Cmd::Prev => {
                                let _ = s.TrySkipPreviousAsync().and_then(|o| o.get());
                            }
                            Cmd::Seek(ms) => {
                                let _ = s
                                    .TryChangePlaybackPositionAsync(ms as i64 * 10_000)
                                    .and_then(|o| o.get());
                            }
                        }
                    }
                    thread::sleep(Duration::from_millis(80));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }

            // A failed read is usually transient (esp. browsers): keep the previous
            // state instead of treating it as "nothing playing".
            let media = match &session {
                Some(s) => match read_media(s) {
                    Ok(m) => m,
                    Err(_) => continue,
                },
                None => Media::default(),
            };
            let key = format!("{}\u{1}{}", media.artist, media.title);

            let mut start_lyrics: Option<(String, String)> = None;
            {
                let mut sh = shared.lock().unwrap();
                sh.media = media.clone();
                sh.snapshot = Some(Instant::now());
                if media.available {
                    if sh.lyrics_key != key {
                        sh.lyrics_key = key.clone();
                        let cached = sh.cache.get(&key).cloned();
                        match cached {
                            Some(c) => sh.lyrics = c,
                            None => {
                                sh.lyrics = Lyrics::Loading;
                                start_lyrics = Some((media.artist.clone(), media.title.clone()));
                            }
                        }
                    }
                } else {
                    sh.lyrics_key.clear();
                    sh.lyrics = Lyrics::Idle;
                }
            }

            if let Some((artist, title)) = start_lyrics {
                let shared = shared.clone();
                let ctx = ctx.clone();
                let key = key.clone();
                thread::spawn(move || {
                    // Let the metadata settle (title/artist/duration often update separately).
                    thread::sleep(Duration::from_millis(600));
                    let dur_s = {
                        let sh = shared.lock().unwrap();
                        if sh.lyrics_key != key {
                            return;
                        }
                        sh.media.dur_ms / 1000
                    };

                    let mut found = None;
                    let mut transient = false;
                    for attempt in 0..4u64 {
                        let (r, tr) = fetch_once(&artist, &title, dur_s);
                        if r.is_some() {
                            found = r;
                            transient = false;
                            break;
                        }
                        transient = tr;
                        if !tr {
                            break;
                        }
                        thread::sleep(Duration::from_secs(2 << attempt));
                        if shared.lock().unwrap().lyrics_key != key {
                            return;
                        }
                    }

                    let lyr = found.unwrap_or(Lyrics::NotFound);
                    {
                        let mut sh = shared.lock().unwrap();
                        if sh.lyrics_key == key {
                            sh.lyrics = lyr.clone();
                            if !transient {
                                sh.cache.insert(key.clone(), lyr);
                            }
                        }
                    }
                    ctx.request_repaint();
                });
            }

            ctx.request_repaint();
        }
    });
    tx
}

// =====================================================================
// Lyrics (LRCLIB)
// =====================================================================

fn clean_title(s: &str) -> String {
    const JUNK: [&str; 6] = [
        "official",
        "video",
        "audio",
        "lyric",
        "visualizer",
        "remaster",
    ];
    let mut t = s.trim().to_string();
    loop {
        let Some(open) = t.rfind(|c: char| c == '(' || c == '[') else {
            break;
        };
        let tail = t[open..].to_lowercase();
        if (t.ends_with(')') || t.ends_with(']')) && JUNK.iter().any(|j| tail.contains(j)) {
            t.truncate(open);
            t = t.trim_end().to_string();
        } else {
            break;
        }
    }
    t
}

fn strip_brackets(s: &str) -> String {
    let mut out = String::new();
    let mut depth = 0i32;
    for c in s.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => {
                if depth > 0 {
                    depth -= 1;
                }
            }
            _ => {
                if depth == 0 {
                    out.push(c);
                }
            }
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn clean_artist(s: &str) -> String {
    let s = s.trim().trim_end_matches(" - Topic");
    s.split(',').next().unwrap_or(s).trim().to_string()
}

fn candidates(artist: &str, title: &str) -> Vec<(String, String)> {
    let a = clean_artist(artist);
    let t1 = clean_title(title);
    let t2 = strip_brackets(&t1);
    let mut v: Vec<(String, String)> = Vec::new();
    let mut push = |a: String, t: String| {
        if !t.is_empty() && !v.iter().any(|(x, y)| *x == a && *y == t) {
            v.push((a, t));
        }
    };
    push(a.clone(), t1.clone());
    push(a.clone(), t2.clone());
    if let Some((l, r)) = t2.split_once(" - ") {
        push(l.trim().to_string(), r.trim().to_string());
    }
    v
}

fn parse_stamp(tag: &str) -> Option<u64> {
    let (m, s) = tag.split_once(':')?;
    let m: u64 = m.parse().ok()?;
    let s: f64 = s.parse().ok()?;
    Some(m * 60_000 + (s * 1000.0).round() as u64)
}

fn parse_lrc(src: &str) -> Vec<(u64, String)> {
    let mut out = Vec::new();
    for line in src.lines() {
        let mut rest = line.trim();
        let mut stamps = Vec::new();
        while let Some(stripped) = rest.strip_prefix('[') {
            let Some(end) = stripped.find(']') else { break };
            if let Some(ms) = parse_stamp(&stripped[..end]) {
                stamps.push(ms);
            }
            rest = stripped[end + 1..].trim_start();
        }
        for ms in stamps {
            out.push((ms, rest.to_string()));
        }
    }
    out.sort_by_key(|(t, _)| *t);
    out
}

fn synced_from_json(v: &serde_json::Value) -> Option<Lyrics> {
    let s = v.get("syncedLyrics")?.as_str()?;
    let lines = parse_lrc(s);
    if lines.is_empty() {
        None
    } else {
        Some(Lyrics::Synced(Arc::new(lines)))
    }
}

/// Err(true) = transient failure (network/5xx/429), Err(false) = definitive miss (404 etc).
fn lrclib(path: &str, params: &[(&str, &str)]) -> Result<serde_json::Value, bool> {
    let mut req = ureq::get(&format!("https://lrclib.net/api/{path}"))
        .set("User-Agent", UA)
        .timeout(Duration::from_secs(8));
    for (k, v) in params {
        req = req.query(k, v);
    }
    match req.call() {
        Ok(r) => r.into_json::<serde_json::Value>().map_err(|_| true),
        Err(ureq::Error::Status(code, _)) => Err(code >= 500 || code == 429),
        Err(_) => Err(true),
    }
}

fn call(path: &str, params: &[(&str, &str)], transient: &mut bool) -> Option<serde_json::Value> {
    match lrclib(path, params) {
        Ok(v) => Some(v),
        Err(t) => {
            *transient |= t;
            None
        }
    }
}

fn best_synced(items: &[serde_json::Value], dur_s: u64) -> Option<Lyrics> {
    let mut best: Option<(f64, &serde_json::Value)> = None;
    for v in items {
        let has = v
            .get("syncedLyrics")
            .and_then(|s| s.as_str())
            .map_or(false, |s| !s.trim().is_empty());
        if !has {
            continue;
        }
        let d = v.get("duration").and_then(|d| d.as_f64()).unwrap_or(0.0);
        let known = dur_s > 0 && d > 0.0;
        let diff = if known { (d - dur_s as f64).abs() } else { 0.0 };
        if known && diff > 15.0 {
            continue;
        }
        if best.map_or(true, |(b, _)| diff < b) {
            best = Some((diff, v));
        }
    }
    best.and_then(|(_, v)| synced_from_json(v))
}

/// Returns (lyrics, had_transient_error).
fn fetch_once(artist: &str, title: &str, dur_s: u64) -> (Option<Lyrics>, bool) {
    let mut tr = false;
    let dur_str = dur_s.to_string();
    let cands = candidates(artist, title);

    for (a, t) in &cands {
        let (a, t) = (a.as_str(), t.as_str());

        if dur_s > 0 {
            if let Some(v) = call(
                "get",
                &[
                    ("artist_name", a),
                    ("track_name", t),
                    ("duration", &dur_str),
                ],
                &mut tr,
            ) {
                if let Some(l) = synced_from_json(&v) {
                    return (Some(l), false);
                }
            }
        }
        if let Some(v) = call("get", &[("artist_name", a), ("track_name", t)], &mut tr) {
            if let Some(l) = synced_from_json(&v) {
                return (Some(l), false);
            }
        }
        if let Some(serde_json::Value::Array(items)) =
            call("search", &[("artist_name", a), ("track_name", t)], &mut tr)
        {
            if let Some(l) = best_synced(&items, dur_s) {
                return (Some(l), false);
            }
        }
    }

    if let Some((a, t)) = cands.first() {
        let q = format!("{a} {t}");
        if let Some(serde_json::Value::Array(items)) = call("search", &[("q", &q)], &mut tr) {
            if let Some(l) = best_synced(&items, dur_s) {
                return (Some(l), false);
            }
        }
        if let Some(serde_json::Value::Array(items)) =
            call("search", &[("track_name", t.as_str())], &mut tr)
        {
            if let Some(l) = best_synced(&items, dur_s) {
                return (Some(l), false);
            }
        }
    }
    (None, tr)
}

fn current_line(l: &Lyrics, pos_ms: u64) -> Option<&str> {
    if let Lyrics::Synced(lines) = l {
        let i = lines.partition_point(|(t, _)| *t <= pos_ms);
        if i > 0 {
            return Some(lines[i - 1].1.as_str());
        }
    }
    None
}

// =====================================================================
// [offset] 2/8: Lyric timing offsets (per song + per app), stored in %APPDATA%
// =====================================================================

#[derive(Default, Clone)]
struct Offsets {
    tracks: HashMap<String, i64>,
    apps: HashMap<String, i64>,
}

fn offsets_path() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("APPDATA")?).join("EmelkaNotch");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join("lyric_offsets.json"))
}

fn load_offsets() -> Offsets {
    let mut o = Offsets::default();
    let Some(p) = offsets_path() else { return o };
    let Ok(s) = std::fs::read_to_string(p) else {
        return o;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) else {
        return o;
    };
    for (field, map) in [("tracks", &mut o.tracks), ("apps", &mut o.apps)] {
        if let Some(obj) = v.get(field).and_then(|x| x.as_object()) {
            for (k, val) in obj {
                if let Some(n) = val.as_i64() {
                    map.insert(k.clone(), n);
                }
            }
        }
    }
    o
}

fn save_offsets(o: &Offsets) {
    let Some(p) = offsets_path() else { return };
    let v = serde_json::json!({ "tracks": o.tracks, "apps": o.apps });
    let _ = std::fs::write(p, v.to_string());
}

/// Cleaned so "(Official Video)" etc. doesn't create a separate entry.
fn track_key(m: &Media) -> String {
    format!(
        "{}\u{1}{}",
        clean_artist(&m.artist).to_lowercase(),
        clean_title(&m.title).to_lowercase()
    )
}

fn app_key(m: &Media) -> String {
    app_label(&m.app).to_lowercase()
}

fn shift(pos_ms: u64, lead: i64) -> u64 {
    (pos_ms as i64 + lead).max(0) as u64
}

// =====================================================================
// Settings (appearance), stored in %APPDATA%\EmelkaNotch\settings.json
// =====================================================================

/// Floating = pill floating below the top edge. Attached = glued to the top edge of the screen.
#[derive(Clone, Copy, PartialEq)]
enum NotchStyle {
    Floating,
    Attached,
}

fn settings_path() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("APPDATA")?).join("EmelkaNotch");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join("settings.json"))
}

/// (style, osd_enabled). Missing / broken file -> defaults.
fn load_settings() -> (NotchStyle, bool) {
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

fn save_settings(style: NotchStyle, osd: bool) {
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

// =====================================================================
// OSD: volume + brightness shown in the pill
// =====================================================================

#[derive(Clone, Copy, PartialEq)]
enum OsdKind {
    Volume,
    Brightness,
}

#[derive(Clone, Copy)]
struct OsdEvent {
    kind: OsdKind,
    value: f32, // 0..1
    muted: bool,
    at: Instant,
}

#[derive(Default)]
struct OsdState {
    latest: Option<OsdEvent>,
}

fn push_osd(state: &Arc<Mutex<OsdState>>, kind: OsdKind, value: f32, muted: bool) {
    state.lock().unwrap().latest = Some(OsdEvent {
        kind,
        value,
        muted,
        at: Instant::now(),
    });
}

/// Polls the default output device's master volume + mute state.
fn spawn_volume_worker(ctx: egui::Context, state: Arc<Mutex<OsdState>>) {
    thread::spawn(move || unsafe {
        let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        let enumerator: IMMDeviceEnumerator =
            match CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL) {
                Ok(e) => e,
                Err(_) => return,
            };

        let mut last: Option<(f32, bool)> = None;
        loop {
            thread::sleep(Duration::from_millis(120));

            // Re-fetch the default endpoint every time so switching output devices just works.
            let cur = (|| -> Option<(f32, bool)> {
                let dev = enumerator.GetDefaultAudioEndpoint(eRender, eConsole).ok()?;
                let vol: IAudioEndpointVolume = dev.Activate(CLSCTX_ALL, None).ok()?;
                let level = vol.GetMasterVolumeLevelScalar().ok()?;
                let muted = vol.GetMute().ok()?.as_bool();
                Some((level, muted))
            })();
            let Some(cur) = cur else { continue };

            // The first reading is only a baseline (no OSD on startup).
            if let Some(prev) = last {
                if (prev.0 - cur.0).abs() >= 0.004 || prev.1 != cur.1 {
                    push_osd(&state, OsdKind::Volume, cur.0, cur.1);
                    ctx.request_repaint();
                }
            }
            last = Some(cur);
        }
    });
}

/// Laptop panels: reads WmiMonitorBrightness through one long-lived hidden PowerShell.
/// On PCs / external monitors it simply prints nothing and the OSD never fires.
fn spawn_brightness_worker(ctx: egui::Context, state: Arc<Mutex<OsdState>>) {
    thread::spawn(move || {
        use std::io::{BufRead, BufReader};
        use std::os::windows::process::CommandExt;
        use std::process::{Command, Stdio};

        // Exits by itself when EmelkaNotch is gone, so it never lingers after a crash/update.
        let script = format!(
            "$p={}; while($true){{ \
               if(-not (Get-Process -Id $p -ErrorAction SilentlyContinue)){{ exit }}; \
               try{{ \
                 $b=(Get-CimInstance -Namespace root/WMI -ClassName WmiMonitorBrightness -ErrorAction Stop | Select-Object -First 1).CurrentBrightness; \
                 [Console]::Out.WriteLine($b); [Console]::Out.Flush() \
               }}catch{{}}; \
               Start-Sleep -Milliseconds 400 \
             }}",
            std::process::id()
        );

        let Ok(mut child) = Command::new("powershell")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-WindowStyle",
                "Hidden",
                "-Command",
                &script,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .spawn()
        else {
            return;
        };
        let Some(out) = child.stdout.take() else {
            return;
        };

        let mut last: Option<u32> = None;
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            let Ok(v) = line.trim().parse::<u32>() else {
                continue;
            };
            if let Some(prev) = last {
                if prev != v {
                    push_osd(&state, OsdKind::Brightness, v as f32 / 100.0, false);
                    ctx.request_repaint();
                }
            }
            last = Some(v);
        }
    });
}

// =====================================================================
// Notifications (reads the Windows notification database)
// =====================================================================
//
// UserNotificationListener needs package identity, which a plain .exe doesn't
// have. wpndatabase.db is where Windows stores every toast, so we read a copy.

#[derive(Clone)]
struct Notif {
    app: String,
    app_id: String,
    icon: Option<Arc<egui::ColorImage>>,
    title: String,
    body: String,
    at: Instant,
}

#[derive(Default)]
struct NotifState {
    items: VecDeque<Notif>,
    latest: Option<Notif>,
    error: Option<String>,
}

/// Asks the Windows Shell for the app's icon via shell:AppsFolder\<AUMID>.
fn load_app_icon(aumid: &str) -> Option<Arc<egui::ColorImage>> {
    if aumid.is_empty() {
        return None;
    }
    unsafe {
        let path: Vec<u16> = format!("shell:AppsFolder\\{aumid}")
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let factory: IShellItemImageFactory =
            SHCreateItemFromParsingName(PCWSTR(path.as_ptr()), None::<&IBindCtx>).ok()?;
        let hbmp = factory
            .GetImage(
                SIZE { cx: 64, cy: 64 },
                SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK,
            )
            .ok()?;
        let hobj = HGDIOBJ(hbmp.0 as _);

        let mut bm = BITMAP::default();
        GetObjectW(
            hobj,
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bm as *mut _ as *mut _),
        );
        let (w, h) = (bm.bmWidth, bm.bmHeight);
        if w <= 0 || h <= 0 {
            let _ = DeleteObject(hobj);
            return None;
        }

        let mut bi = BITMAPINFO::default();
        bi.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h, // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };
        let mut buf = vec![0u8; (w * h * 4) as usize];
        let hdc = CreateCompatibleDC(None);
        let lines = GetDIBits(
            hdc,
            hbmp,
            0,
            h as u32,
            Some(buf.as_mut_ptr() as *mut _),
            &mut bi,
            DIB_RGB_COLORS,
        );
        let _ = DeleteDC(hdc);
        let _ = DeleteObject(hobj);
        if lines == 0 {
            return None;
        }

        // BGRA -> RGBA
        let mut any_alpha = false;
        for px in buf.chunks_exact_mut(4) {
            px.swap(0, 2);
            any_alpha |= px[3] != 0;
        }
        if !any_alpha {
            for px in buf.chunks_exact_mut(4) {
                px[3] = 255;
            }
        }
        Some(Arc::new(egui::ColorImage::from_rgba_unmultiplied(
            [w as usize, h as usize],
            &buf,
        )))
    }
}

fn decode_entities(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

fn parse_toast(xml: &str) -> Option<(String, String)> {
    let mut texts: Vec<String> = Vec::new();
    let mut rest = xml;
    while let Some(i) = rest.find("<text") {
        rest = &rest[i + 5..];
        let Some(gt) = rest.find('>') else { break };
        let attrs = &rest[..gt];
        if !matches!(
            attrs.chars().next(),
            None | Some(' ') | Some('/') | Some('\t') | Some('\n') | Some('\r')
        ) {
            continue; // some other tag starting with "text"
        }
        if attrs.trim_end().ends_with('/') {
            rest = &rest[gt + 1..];
            continue;
        }
        let body = &rest[gt + 1..];
        let Some(end) = body.find("</text>") else {
            break;
        };
        let content = decode_entities(&body[..end]).trim().to_string();
        rest = &body[end + 7..];
        if attrs.contains("attribution") || content.is_empty() {
            continue;
        }
        texts.push(content);
    }
    if texts.is_empty() {
        return None;
    }
    let title = texts.remove(0);
    Some((title, texts.join(" ")))
}

fn pretty_app(id: &str) -> String {
    let l = id.to_lowercase();
    for (k, v) in [
        ("whatsapp", "WhatsApp"),
        ("telegram", "Telegram"),
        ("discord", "Discord"),
        ("slack", "Slack"),
        ("msedge", "Edge"),
        ("chrome", "Chrome"),
        ("firefox", "Firefox"),
        ("brave", "Brave"),
        ("outlook", "Outlook"),
        ("teams", "Teams"),
        ("spotify", "Spotify"),
        ("instagram", "Instagram"),
        ("securityhealth", "Security"),
        ("systemtoast", "Windows"),
    ] {
        if l.contains(k) {
            return v.to_string();
        }
    }
    let base = id.split('!').next().unwrap_or(id);
    let base = base.split('_').next().unwrap_or(base);
    let base = base.rsplit('.').next().unwrap_or(base);
    if base.is_empty() {
        "Windows".to_string()
    } else {
        base.to_string()
    }
}

fn payload_to_string(bytes: &[u8]) -> String {
    if bytes.len() >= 2 && bytes[1] == 0 {
        let u: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        String::from_utf16_lossy(&u).trim_matches('\0').to_string()
    } else {
        String::from_utf8_lossy(bytes)
            .trim_matches('\0')
            .to_string()
    }
}

fn file_stamp(p: &Path) -> Option<(u64, SystemTime)> {
    let m = std::fs::metadata(p).ok()?;
    Some((m.len(), m.modified().ok()?))
}

fn read_new(
    path: &Path,
    last_id: &mut Option<i64>,
    icons: &mut HashMap<String, Option<Arc<egui::ColorImage>>>,
) -> rusqlite::Result<Vec<Notif>> {
    let conn = Connection::open(path)?;
    let Some(prev) = *last_id else {
        // First run: remember where we are, don't replay old notifications.
        let max: i64 = conn.query_row("SELECT IFNULL(MAX(Id),0) FROM Notification", [], |r| {
            r.get(0)
        })?;
        *last_id = Some(max);
        return Ok(Vec::new());
    };

    let mut stmt = conn.prepare(
        "SELECT n.Id, n.Type, h.PrimaryId, n.Payload \
         FROM Notification n LEFT JOIN NotificationHandler h ON h.RecordId = n.HandlerId \
         WHERE n.Id > ?1 ORDER BY n.Id",
    )?;
    let rows = stmt.query_map([prev], |r| {
        let id: i64 = r.get(0)?;
        let ty = match r.get_ref(1)? {
            ValueRef::Text(t) => String::from_utf8_lossy(t).to_lowercase(),
            _ => String::new(),
        };
        let app = match r.get_ref(2)? {
            ValueRef::Text(t) => String::from_utf8_lossy(t).to_string(),
            _ => String::new(),
        };
        let payload = match r.get_ref(3)? {
            ValueRef::Blob(b) | ValueRef::Text(b) => b.to_vec(),
            _ => Vec::new(),
        };
        Ok((id, ty, app, payload))
    })?;

    let mut out = Vec::new();
    let mut max_id = prev;
    for row in rows {
        let (id, ty, app, payload) = row?;
        max_id = max_id.max(id);
        if !(ty.is_empty() || ty == "toast") {
            continue;
        }
        let xml = payload_to_string(&payload);
        if let Some((title, body)) = parse_toast(&xml) {
            let icon = icons
                .entry(app.clone())
                .or_insert_with(|| load_app_icon(&app))
                .clone();
            out.push(Notif {
                app: pretty_app(&app),
                app_id: app.clone(),
                icon,
                title,
                body,
                at: Instant::now(),
            });
        }
    }
    *last_id = Some(max_id);
    Ok(out)
}

fn spawn_notif_worker(ctx: egui::Context, state: Arc<Mutex<NotifState>>) {
    thread::spawn(move || {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
        let mut icons: HashMap<String, Option<Arc<egui::ColorImage>>> = HashMap::new();

        let set_err = |m: Option<String>| {
            state.lock().unwrap().error = m;
        };

        let Some(base) = std::env::var_os("LOCALAPPDATA").map(|p| {
            PathBuf::from(p)
                .join("Microsoft")
                .join("Windows")
                .join("Notifications")
        }) else {
            set_err(Some("LOCALAPPDATA not found.".into()));
            return;
        };
        let src_db = base.join("wpndatabase.db");
        let src_wal = base.join("wpndatabase.db-wal");

        let tmp = std::env::temp_dir().join("EmelkaNotch_wpn");
        let _ = std::fs::create_dir_all(&tmp);
        let dst_db = tmp.join("w.db");
        let dst_wal = tmp.join("w.db-wal");
        let dst_shm = tmp.join("w.db-shm");

        let mut last_id: Option<i64> = None;
        let mut last_stamp = None;

        loop {
            thread::sleep(Duration::from_millis(500));

            let stamp = (file_stamp(&src_db), file_stamp(&src_wal));
            if last_stamp == Some(stamp) {
                continue;
            }

            let _ = std::fs::remove_file(&dst_wal);
            let _ = std::fs::remove_file(&dst_shm);
            if let Err(e) = std::fs::copy(&src_db, &dst_db) {
                set_err(Some(format!("Can't read the notification database ({e})")));
                continue;
            }
            let _ = std::fs::copy(&src_wal, &dst_wal);

            match read_new(&dst_db, &mut last_id, &mut icons) {
                Ok(new) => {
                    last_stamp = Some(stamp);
                    set_err(None);
                    if !new.is_empty() {
                        let mut st = state.lock().unwrap();
                        for n in new {
                            st.items.push_front(n.clone());
                            st.latest = Some(n);
                        }
                        st.items.truncate(30);
                        drop(st);
                        ctx.request_repaint();
                    }
                }
                Err(e) => set_err(Some(format!("Notification DB error: {e}"))),
            }
        }
    });
}

// =====================================================================
// Auto-update (GitHub Releases)
// =====================================================================

#[derive(Clone, Default, PartialEq)]
enum Upd {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Updating(String),
    Failed,
}

/// Shows a message in the pill / Notification tab using the normal notification path.
fn push_notice(state: &Arc<Mutex<NotifState>>, title: &str, body: String) {
    let n = Notif {
        app: "EmelkaNotch".into(),
        app_id: String::new(),
        icon: None,
        title: title.into(),
        body,
        at: Instant::now(),
    };
    let mut st = state.lock().unwrap();
    st.items.push_front(n.clone());
    st.latest = Some(n);
    st.items.truncate(30);
}

fn parse_ver(s: &str) -> Vec<u32> {
    s.trim_start_matches('v')
        .split('.')
        .filter_map(|p| p.parse().ok())
        .collect()
}

/// Ok(Some((tag, url))) = newer release, Ok(None) = up to date, Err = couldn't check.
fn check_update() -> Result<Option<(String, String)>, ()> {
    let v: serde_json::Value = ureq::get(&format!(
        "https://api.github.com/repos/{REPO}/releases/latest"
    ))
    .set("User-Agent", UA)
    .timeout(Duration::from_secs(10))
    .call()
    .map_err(|_| ())?
    .into_json()
    .map_err(|_| ())?;
    let Some(tag) = v.get("tag_name").and_then(|t| t.as_str()) else {
        return Err(());
    };
    if parse_ver(tag) <= parse_ver(VERSION) {
        return Ok(None);
    }
    let url = v
        .get("assets")
        .and_then(|a| a.as_array())
        .and_then(|a| {
            a.iter()
                .find(|x| x.get("name").and_then(|n| n.as_str()) == Some("EmelkaNotch.exe"))
        })
        .and_then(|a| a.get("browser_download_url"))
        .and_then(|u| u.as_str())
        .map(String::from);
    match url {
        Some(u) => Ok(Some((tag.to_string(), u))),
        None => Err(()),
    }
}

/// Best-effort log so update failures aren't silent (%APPDATA%\EmelkaNotch\update.log).
fn log_update_error(msg: &str) {
    let dir = std::env::var("APPDATA")
        .map(|d| std::path::PathBuf::from(d).join("EmelkaNotch"))
        .unwrap_or_else(|_| std::env::temp_dir());
    let _ = std::fs::create_dir_all(&dir);
    let line = format!(
        "[{}] {}\n",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
        msg
    );
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("update.log"))
    {
        let _ = f.write_all(line.as_bytes());
    }
}

/// Rename with retries — on Windows, AV/Defender often holds the just-downloaded
/// exe briefly (real-time scan), causing a sharing violation on the first try.
fn rename_retry(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
    let mut last = None;
    for i in 0..10 {
        match std::fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) => {
                last = Some(e);
                thread::sleep(Duration::from_millis(300 * (i as u64 + 1)));
            }
        }
    }
    Err(last.unwrap())
}

fn apply_update(url: &str, tag: &str) -> Result<(), String> {
    let fail = |e: String| {
        log_update_error(&format!("apply_update({tag}): {e}"));
        Err(e)
    };
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => return fail(format!("current_exe: {e}")),
    };
    let new = exe.with_extension("new");
    let old = exe.with_extension("old");

    let resp = match ureq::get(url)
        .set("User-Agent", UA)
        .timeout(Duration::from_secs(300))
        .call()
    {
        Ok(r) => r,
        Err(e) => return fail(format!("download: {e}")),
    };
    let expected = resp
        .header("Content-Length")
        .and_then(|l| l.parse::<u64>().ok());
    let mut reader = resp.into_reader();
    let mut f = match std::fs::File::create(&new) {
        Ok(f) => f,
        Err(e) => return fail(format!("create {}: {e}", new.display())),
    };
    if let Err(e) = std::io::copy(&mut reader, &mut f) {
        let _ = std::fs::remove_file(&new);
        return fail(format!("write: {e}"));
    }
    drop(f);
    // Sanity check: must be a PE (MZ) and match Content-Length.
    let size = match std::fs::metadata(&new) {
        Ok(m) => m.len(),
        Err(e) => return fail(format!("metadata: {e}")),
    };
    if let Some(exp) = expected {
        if size != exp {
            let _ = std::fs::remove_file(&new);
            return fail(format!("size mismatch: got {size}, expected {exp}"));
        }
    }
    match std::fs::read(&new) {
        Ok(b) if b.len() >= 2 && &b[..2] == b"MZ" => {}
        _ => {
            let _ = std::fs::remove_file(&new);
            return fail("downloaded file is not a valid exe (missing MZ header)".into());
        }
    }

    let _ = std::fs::remove_file(&old);
    if let Err(e) = rename_retry(&exe, &old) {
        return fail(format!("rename exe->old: {e}"));
    }
    if let Err(e) = rename_retry(&new, &exe) {
        let _ = std::fs::rename(&old, &exe); // rollback
        return fail(format!("rename new->exe: {e}"));
    }
    // The new instance reads this on startup and shows "EmelkaNotch updated".
    let _ = std::fs::write(exe.with_extension("updated"), tag);
    if let Err(e) = std::process::Command::new(&exe).spawn() {
        return fail(format!("restart: {e}"));
    }
    std::process::exit(0);
}

fn take_updated_marker() -> Option<String> {
    let m = std::env::current_exe().ok()?.with_extension("updated");
    let tag = std::fs::read_to_string(&m).ok()?;
    let _ = std::fs::remove_file(&m);
    Some(tag.trim().to_string())
}

fn run_update(
    upd: Arc<Mutex<Upd>>,
    notifs: Arc<Mutex<NotifState>>,
    ctx: egui::Context,
    manual: bool,
) -> bool {
    {
        let mut s = upd.lock().unwrap();
        if matches!(*s, Upd::Checking | Upd::Updating(_)) {
            return false;
        }
        *s = Upd::Checking;
    }
    ctx.request_repaint();
    let mut check_failed = false;

    match check_update() {
        Ok(Some((tag, url))) => {
            *upd.lock().unwrap() = Upd::Updating(tag.clone());
            push_notice(
                &notifs,
                "Updating EmelkaNotch",
                format!("Downloading {tag}…"),
            );
            ctx.request_repaint();
            if let Err(e) = apply_update(&url, &tag) {
                *upd.lock().unwrap() = Upd::Failed;
                push_notice(
                    &notifs,
                    "Update failed",
                    format!("Couldn't install the update. {e}"),
                );
            }
        }
        Ok(None) => {
            *upd.lock().unwrap() = if manual { Upd::UpToDate } else { Upd::Idle };
        }
        Err(()) => {
            check_failed = true;
            *upd.lock().unwrap() = if manual { Upd::Failed } else { Upd::Idle };
        }
    }
    ctx.request_repaint();
    check_failed
}

fn spawn_updater(ctx: egui::Context, upd: Arc<Mutex<Upd>>, notifs: Arc<Mutex<NotifState>>) {
    thread::spawn(move || {
        // Leftover from the previous update (the old process needs a moment to exit).
        if let Ok(exe) = std::env::current_exe() {
            for _ in 0..10 {
                thread::sleep(Duration::from_secs(3));
                if std::fs::remove_file(exe.with_extension("old")).is_ok()
                    || !exe.with_extension("old").exists()
                {
                    break;
                }
            }
        }
        loop {
            let failed = run_update(upd.clone(), notifs.clone(), ctx.clone(), false);
            thread::sleep(Duration::from_secs(if failed { 5 * 60 } else { 6 * 3600 }));
        }
    });
}

// =====================================================================
// Todo (stored in %APPDATA%\EmelkaNotch\todos.db)
// =====================================================================

#[derive(Clone)]
struct Todo {
    id: u64,
    text: String,
    done: bool,
    due: Option<i64>, // unix seconds
    notified: bool,
}

#[derive(Default)]
struct TodoState {
    items: Vec<Todo>,
    next_id: u64,
}

fn todo_db_path() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("APPDATA")?).join("EmelkaNotch");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join("todos.db"))
}

fn open_todo_db() -> rusqlite::Result<Connection> {
    let p = todo_db_path().ok_or(rusqlite::Error::InvalidPath(PathBuf::new()))?;
    let conn = Connection::open(p)?;
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS todos (
            id       INTEGER PRIMARY KEY,
            text     TEXT NOT NULL,
            done     INTEGER NOT NULL DEFAULT 0,
            due      INTEGER,
            notified INTEGER NOT NULL DEFAULT 0
        );",
    )?;
    Ok(conn)
}

fn load_todos() -> TodoState {
    let mut st = TodoState::default();
    let Ok(conn) = open_todo_db() else { return st };
    let Ok(mut stmt) = conn.prepare("SELECT id, text, done, due, notified FROM todos ORDER BY id")
    else {
        return st;
    };
    let Ok(rows) = stmt.query_map([], |r| {
        Ok(Todo {
            id: r.get::<_, i64>(0)? as u64,
            text: r.get(1)?,
            done: r.get::<_, i64>(2)? != 0,
            due: r.get(3)?,
            notified: r.get::<_, i64>(4)? != 0,
        })
    }) else {
        return st;
    };
    for t in rows.flatten() {
        st.next_id = st.next_id.max(t.id);
        st.items.push(t);
    }
    st
}

fn save_todos(items: &[Todo]) {
    let Ok(mut conn) = open_todo_db() else { return };
    let Ok(tx) = conn.transaction() else { return };
    let _ = tx.execute("DELETE FROM todos", []);
    for t in items {
        let _ = tx.execute(
            "INSERT INTO todos (id, text, done, due, notified) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params![t.id as i64, t.text, t.done as i64, t.due, t.notified as i64],
        );
    }
    let _ = tx.commit();
}

fn fmt_due(ts: i64) -> String {
    let Some(dt) = Local.timestamp_opt(ts, 0).single() else {
        return String::new();
    };
    let today = Local::now().date_naive();
    let d = dt.date_naive();
    let day = if d == today {
        "Today".to_string()
    } else if Some(d) == today.succ_opt() {
        "Tomorrow".to_string()
    } else {
        dt.format("%d %b").to_string()
    };
    format!("{day} {}", dt.format("%H:%M"))
}

/// Checks every second; when a todo is due, sends a notification to the pill.
fn spawn_todo_worker(
    ctx: egui::Context,
    todos: Arc<Mutex<TodoState>>,
    notifs: Arc<Mutex<NotifState>>,
) {
    thread::spawn(move || loop {
        thread::sleep(Duration::from_secs(1));
        let now = Local::now().timestamp();
        let mut fired: Vec<String> = Vec::new();
        {
            let mut st = todos.lock().unwrap();
            for t in st.items.iter_mut() {
                if !t.done && !t.notified && t.due.map_or(false, |d| d <= now) {
                    t.notified = true;
                    fired.push(t.text.clone());
                }
            }
            if !fired.is_empty() {
                save_todos(&st.items);
            }
        }
        if !fired.is_empty() {
            for text in fired {
                push_notice(&notifs, "Reminder", text);
            }
            ctx.request_repaint();
        }
    });
}

// =====================================================================
// System info helpers
// =====================================================================

struct DiskInfo {
    name: String,
    used_gb: f64,
    total_gb: f64,
}

fn read_disks() -> Vec<DiskInfo> {
    let disks = Disks::new_with_refreshed_list();
    let gb = 1_073_741_824.0;
    let mut out: Vec<DiskInfo> = Vec::new();
    for d in disks.list() {
        let total = d.total_space() as f64 / gb;
        if total <= 0.0 {
            continue; // empty CD-ROM etc.
        }
        let name = d
            .mount_point()
            .to_string_lossy()
            .trim_end_matches('\\')
            .to_string();
        if out.iter().any(|x| x.name == name) {
            continue;
        }
        let free = d.available_space() as f64 / gb;
        out.push(DiskInfo {
            name,
            used_gb: (total - free).max(0.0),
            total_gb: total,
        });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn fmt_gb(g: f64) -> String {
    if g >= 1000.0 {
        format!("{:.1} TB", g / 1024.0)
    } else {
        format!("{:.0} GB", g)
    }
}

fn push_hist(q: &mut VecDeque<f32>, v: f32) {
    if q.len() >= HIST {
        q.pop_front();
    }
    q.push_back(v);
}

fn card(ui: &mut Ui, w: f32, h: f32) -> (Rect, Painter) {
    let (rect, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, CornerRadius::same(12), CARD);
    p.rect_stroke(
        rect,
        CornerRadius::same(12),
        Stroke::new(1.0, BORDER),
        StrokeKind::Inside,
    );
    (rect, p)
}

fn card_header(p: &Painter, r: Rect, title: &str, value: &str) {
    p.text(
        pos2(r.left() + 12.0, r.top() + 16.0),
        Align2::LEFT_CENTER,
        title,
        FontId::proportional(11.0),
        MUTED,
    );
    p.text(
        pos2(r.right() - 12.0, r.top() + 16.0),
        Align2::RIGHT_CENTER,
        value,
        semibold(13.0),
        TEXT,
    );
}

fn sparkline(p: &Painter, r: Rect, data: &VecDeque<f32>, color: Color32) {
    for f in [0.25, 0.5, 0.75] {
        let y = r.bottom() - r.height() * f;
        p.line_segment(
            [pos2(r.left(), y), pos2(r.right(), y)],
            Stroke::new(1.0, TRACK.gamma_multiply(0.5)),
        );
    }
    if data.len() < 2 {
        return;
    }
    let step = r.width() / (HIST - 1) as f32;
    let n = data.len();
    let pts: Vec<Pos2> = data
        .iter()
        .enumerate()
        .map(|(i, v)| {
            pos2(
                r.right() - (n - 1 - i) as f32 * step,
                r.bottom() - v.clamp(0.0, 1.0) * r.height(),
            )
        })
        .collect();

    // gradient fill under the line
    let mut mesh = egui::Mesh::default();
    for pt in &pts {
        mesh.colored_vertex(*pt, color.gamma_multiply(0.30));
        mesh.colored_vertex(pos2(pt.x, r.bottom()), Color32::TRANSPARENT);
    }
    for i in 0..pts.len() - 1 {
        let k = (i * 2) as u32;
        mesh.add_triangle(k, k + 1, k + 2);
        mesh.add_triangle(k + 1, k + 2, k + 3);
    }
    p.add(Shape::mesh(mesh));
    p.add(Shape::line(pts, Stroke::new(1.6, color)));
}

// =====================================================================
// Helpers
// =====================================================================

fn greeting() -> String {
    match Local::now().hour() {
        5..=11 => "Good morning",
        12..=16 => "Good afternoon",
        17..=21 => "Good evening",
        _ => "Hello, night owl",
    }
    .to_string()
}

fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

fn ease(t: f32) -> f32 {
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("Inter-SemiBold".into()))
}

fn small(s: String) -> RichText {
    RichText::new(s).size(11.0).color(MUTED)
}

fn ellipsize(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        s.chars().take(n.saturating_sub(1)).collect::<String>() + "…"
    }
}

fn text_w(p: &Painter, s: &str, f: &FontId) -> f32 {
    p.layout_no_wrap(s.to_owned(), f.clone(), Color32::WHITE)
        .size()
        .x
}

/// Truncate with "…" so the text fits `max_w`. Returns (text, width).
fn fit(p: &Painter, s: &str, f: FontId, max_w: f32) -> (String, f32) {
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

fn one_line(s: &str) -> String {
    s.replace(['\n', '\r'], " ")
}

fn ago(t: Instant) -> String {
    let s = t.elapsed().as_secs();
    if s < 60 {
        "now".into()
    } else if s < 3600 {
        format!("{}m", s / 60)
    } else {
        format!("{}h", s / 3600)
    }
}

fn fmt_time(ms: u64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}

fn app_label(id: &str) -> String {
    let l = id.to_lowercase();
    let name = if l.contains("spotify") {
        "Spotify"
    } else if l.contains("chrome") {
        "Chrome"
    } else if l.contains("msedge") || l.contains("edge") {
        "Edge"
    } else if l.contains("firefox") {
        "Firefox"
    } else if l.contains("brave") {
        "Brave"
    } else if l.contains("applemusic") || l.contains("itunes") {
        "Apple Music"
    } else {
        return id.trim_end_matches(".exe").to_string();
    };
    name.to_string()
}

fn setup_fonts(ctx: &egui::Context) {
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

fn shape_rect(pw: f32, ph: f32, e: f32, top: f32) -> Rect {
    let w = lerp(pw, PANEL_W, e);
    let h = lerp(ph, PANEL_H, e);
    Rect::from_min_size(pos2((WIN_W - w) / 2.0, top), vec2(w, h))
}

/// Cursor position in the window's local point space (works even when unfocused).
fn cursor_local(ctx: &egui::Context) -> Option<egui::Pos2> {
    let mut p = POINT { x: 0, y: 0 };
    if unsafe { GetCursorPos(&mut p) } == 0 {
        return None;
    }
    let rect = ctx.input(|i| i.viewport().inner_rect)?;
    let ppp = ctx.pixels_per_point();
    Some(pos2(
        p.x as f32 / ppp - rect.min.x,
        p.y as f32 / ppp - rect.min.y,
    ))
}

// ---------- small widgets ----------

fn badge(p: &Painter, c: Pos2, r: f32, app: &str, alpha: f32) {
    const PAL: [Color32; 6] = [
        Color32::from_rgb(120, 170, 255),
        Color32::from_rgb(255, 150, 120),
        Color32::from_rgb(130, 220, 170),
        Color32::from_rgb(230, 190, 100),
        Color32::from_rgb(200, 150, 255),
        Color32::from_rgb(120, 210, 220),
    ];
    let h = app
        .bytes()
        .fold(0usize, |a, b| a.wrapping_mul(31).wrapping_add(b as usize));
    p.circle_filled(c, r, PAL[h % PAL.len()].gamma_multiply(alpha));
    let initial = app
        .chars()
        .next()
        .map(|ch| ch.to_uppercase().to_string())
        .unwrap_or_else(|| "?".to_string());
    p.text(
        c,
        Align2::CENTER_CENTER,
        initial,
        semibold(r * 0.95),
        Color32::from_rgb(10, 10, 12).gamma_multiply(alpha),
    );
}

/// Real app icon if we have one, otherwise the letter badge.
fn app_icon(
    p: &Painter,
    c: Pos2,
    r: f32,
    tex: Option<&egui::TextureHandle>,
    app: &str,
    alpha: f32,
) {
    if let Some(t) = tex {
        p.image(
            t.id(),
            Rect::from_center_size(c, vec2(r * 2.0, r * 2.0)),
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE.gamma_multiply(alpha),
        );
    } else {
        badge(p, c, r, app, alpha);
    }
}

/// Speaker (with waves, or an X when muted) / sun icon for the OSD.
fn osd_icon(p: &Painter, c: Pos2, kind: OsdKind, muted: bool, col: Color32) {
    let st = Stroke::new(1.4, col);
    match kind {
        OsdKind::Volume => {
            p.rect_filled(
                Rect::from_min_max(pos2(c.x - 7.0, c.y - 2.2), pos2(c.x - 3.5, c.y + 2.2)),
                CornerRadius::same(1),
                col,
            );
            p.add(Shape::convex_polygon(
                vec![
                    pos2(c.x - 3.5, c.y - 2.2),
                    pos2(c.x + 0.5, c.y - 5.5),
                    pos2(c.x + 0.5, c.y + 5.5),
                    pos2(c.x - 3.5, c.y + 2.2),
                ],
                col,
                Stroke::NONE,
            ));
            if muted {
                p.line_segment([c + vec2(3.0, -3.0), c + vec2(8.0, 3.0)], st);
                p.line_segment([c + vec2(3.0, 3.0), c + vec2(8.0, -3.0)], st);
            } else {
                for r in [3.5f32, 6.5] {
                    let pts: Vec<Pos2> = (0..=8)
                        .map(|i| {
                            let a = -0.75 + 1.5 * i as f32 / 8.0;
                            pos2(c.x + 0.5 + r * a.cos(), c.y + r * a.sin())
                        })
                        .collect();
                    p.add(Shape::line(pts, st));
                }
            }
        }
        OsdKind::Brightness => {
            p.circle_stroke(c, 3.2, st);
            for k in 0..8 {
                let a = k as f32 * PI / 4.0;
                let (s, co) = a.sin_cos();
                p.line_segment(
                    [c + vec2(co * 5.6, s * 5.6), c + vec2(co * 8.0, s * 8.0)],
                    Stroke::new(1.3, col),
                );
            }
        }
    }
}

/// Tab berbentuk ikon; nama tampil sebagai tooltip saat di-hover.
fn tab_icon(ui: &mut Ui, sec: Section, label: &str, selected: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(32.0, 28.0), Sense::click());
    let hov = resp.hovered();
    let bg = if selected {
        Color32::from_rgb(40, 42, 52)
    } else if hov {
        Color32::from_rgb(24, 25, 31)
    } else {
        Color32::TRANSPARENT
    };
    let col = if selected {
        TEXT
    } else if hov {
        Color32::from_rgb(190, 194, 205)
    } else {
        MUTED
    };
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(8), bg);
    let c = rect.center();
    let st = Stroke::new(1.4, col);

    match sec {
        Section::Media => {
            // not musik
            p.circle_filled(c + vec2(-2.5, 4.0), 2.8, col);
            p.line_segment([c + vec2(0.2, 3.5), c + vec2(0.2, -5.5)], st);
            p.line_segment([c + vec2(0.2, -5.5), c + vec2(4.5, -3.0)], st);
        }
        Section::Alerts => {
            // lonceng
            let r = 4.5;
            let mut pts: Vec<Pos2> = (0..=12)
                .map(|i| {
                    let a = PI + PI * i as f32 / 12.0;
                    pos2(c.x + r * a.cos(), c.y - 1.0 + r * a.sin())
                })
                .collect();
            pts.push(pos2(c.x + 5.5, c.y + 3.5));
            pts.push(pos2(c.x - 5.5, c.y + 3.5));
            p.add(Shape::closed_line(pts, st));
            p.circle_filled(c + vec2(0.0, 6.0), 1.3, col);
        }
        Section::System => {
            // chip
            p.rect_stroke(
                Rect::from_center_size(c, vec2(11.0, 11.0)),
                CornerRadius::same(2),
                st,
                StrokeKind::Inside,
            );
            p.rect_stroke(
                Rect::from_center_size(c, vec2(5.0, 5.0)),
                CornerRadius::same(1),
                Stroke::new(1.2, col),
                StrokeKind::Inside,
            );
            for o in [-2.5f32, 2.5] {
                p.line_segment([pos2(c.x + o, c.y - 5.5), pos2(c.x + o, c.y - 8.0)], st);
                p.line_segment([pos2(c.x + o, c.y + 5.5), pos2(c.x + o, c.y + 8.0)], st);
                p.line_segment([pos2(c.x - 5.5, c.y + o), pos2(c.x - 8.0, c.y + o)], st);
                p.line_segment([pos2(c.x + 5.5, c.y + o), pos2(c.x + 8.0, c.y + o)], st);
            }
        }
        Section::Todo => {
            // lingkaran + centang
            p.circle_stroke(c, 6.5, st);
            p.line_segment([c + vec2(-3.0, 0.2), c + vec2(-0.8, 2.6)], st);
            p.line_segment([c + vec2(-0.8, 2.6), c + vec2(3.2, -2.4)], st);
        }
        Section::Settings => {
            // gear
            p.circle_stroke(c, 4.0, st);
            p.circle_filled(c, 1.4, col);
            for k in 0..8 {
                let a = k as f32 * PI / 4.0;
                let (s, co) = a.sin_cos();
                p.line_segment(
                    [c + vec2(co * 4.6, s * 4.6), c + vec2(co * 6.8, s * 6.8)],
                    Stroke::new(1.8, col),
                );
            }
        }
    }

    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(label)
        .clicked()
}

#[derive(Clone, Copy)]
enum Icon {
    Prev,
    Next,
    Play,
    Pause,
}

fn icon_button(ui: &mut Ui, icon: Icon, size: f32, primary: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let p = ui.painter();
    let c = rect.center();
    if primary {
        let fill = if resp.hovered() {
            Color32::WHITE
        } else {
            Color32::from_rgb(232, 233, 238)
        };
        p.circle_filled(c, size / 2.0, fill);
    } else if resp.hovered() {
        p.circle_filled(c, size / 2.0, Color32::from_rgb(32, 34, 42));
    }
    let col = if primary {
        Color32::from_rgb(10, 10, 12)
    } else {
        TEXT
    };
    let s = size * 0.2;

    let tri = |dir: f32, cx: f32| {
        Shape::convex_polygon(
            vec![
                pos2(cx - dir * s * 0.9, c.y - s),
                pos2(cx - dir * s * 0.9, c.y + s),
                pos2(cx + dir * s * 0.9, c.y),
            ],
            col,
            Stroke::NONE,
        )
    };
    let bar = |cx: f32, w: f32| {
        p.rect_filled(
            Rect::from_center_size(pos2(cx, c.y), vec2(w, s * 2.0)),
            CornerRadius::same(1),
            col,
        );
    };

    match icon {
        Icon::Play => {
            p.add(tri(1.0, c.x + s * 0.15));
        }
        Icon::Pause => {
            bar(c.x - s * 0.55, s * 0.6);
            bar(c.x + s * 0.55, s * 0.6);
        }
        Icon::Next => {
            p.add(tri(1.0, c.x - s * 0.35));
            bar(c.x + s * 0.95, s * 0.35);
        }
        Icon::Prev => {
            p.add(tri(-1.0, c.x + s * 0.35));
            bar(c.x - s * 0.95, s * 0.35);
        }
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Thin horizontal bar. Returns the response so callers can handle clicks.
fn bar(ui: &mut Ui, width: f32, frac: f32, fill: Color32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 14.0), Sense::click());
    let track = Rect::from_center_size(rect.center(), vec2(rect.width(), 4.0));
    ui.painter()
        .rect_filled(track, CornerRadius::same(2), TRACK);
    let filled = Rect::from_min_size(track.min, vec2(track.width() * frac.clamp(0.0, 1.0), 4.0));
    ui.painter()
        .rect_filled(filled, CornerRadius::same(2), fill);
    resp
}

fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

/// A child Ui placed at `rect` shifted by `dx`, clipped to `rect`, with its own opacity.
fn slide_child(ui: &mut Ui, rect: Rect, dx: f32, alpha: f32, salt: impl std::hash::Hash) -> Ui {
    let mut c = ui.new_child(
        UiBuilder::new()
            .id_salt(salt)
            .max_rect(rect.translate(vec2(dx, 0.0)))
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    c.set_clip_rect(rect.intersect(ui.clip_rect()));
    c.set_opacity(alpha.clamp(0.0, 1.0));
    c
}

fn field_fill(hovered: bool, focused: bool) -> Color32 {
    if focused {
        Color32::from_rgb(30, 32, 40)
    } else if hovered {
        Color32::from_rgb(25, 26, 33)
    } else {
        Color32::from_rgb(20, 21, 27)
    }
}

fn btn_rect(ui: &mut Ui, rect: Rect, id: impl std::hash::Hash) -> egui::Response {
    ui.interact(rect, egui::Id::new(id), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Rounded text button. Returns true when clicked (never when disabled).
fn text_btn(
    ui: &mut Ui,
    p: &Painter,
    rect: Rect,
    label: &str,
    id: &str,
    primary: bool,
    enabled: bool,
) -> bool {
    let r = btn_rect(ui, rect, ("pk_btn", id));
    let hov = r.hovered() && enabled;
    let (fill, col) = if !enabled {
        (TRACK, DIM)
    } else if primary {
        (if hov { Color32::WHITE } else { TEXT }, BG)
    } else {
        (field_fill(hov, false), if hov { TEXT } else { MUTED })
    };
    p.rect_filled(rect, CornerRadius::same(FIELD_R), fill);
    p.text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        semibold(11.5),
        col,
    );
    r.clicked() && enabled
}

fn arrow_btn(ui: &mut Ui, p: &Painter, rect: Rect, dir: f32, id: &str) -> bool {
    let r = btn_rect(ui, rect, ("pk_arrow", id));
    if r.hovered() {
        p.rect_filled(rect, CornerRadius::same(6), Color32::from_rgb(32, 34, 42));
    }
    let c = rect.center();
    let s = 4.0;
    p.add(Shape::convex_polygon(
        vec![
            pos2(c.x - dir * s * 0.6, c.y - s),
            pos2(c.x - dir * s * 0.6, c.y + s),
            pos2(c.x + dir * s * 0.9, c.y),
        ],
        if r.hovered() { TEXT } else { MUTED },
        Stroke::NONE,
    ));
    r.clicked()
}

/// [-]  09  [+]  stepper. Wraps around; scroll wheel works too.
fn stepper(
    ui: &mut Ui,
    p: &Painter,
    origin: Pos2,
    val: &mut u32,
    modulo: u32,
    step: u32,
    id: &str,
) {
    let full = Rect::from_min_size(origin, vec2(96.0, 28.0));
    p.rect_filled(
        full,
        CornerRadius::same(FIELD_R),
        field_fill(ui.rect_contains_pointer(full), false),
    );
    let minus = Rect::from_min_size(origin, vec2(26.0, 28.0));
    let plus = Rect::from_min_size(pos2(origin.x + 70.0, origin.y), vec2(26.0, 28.0));
    let mut delta = 0i32;
    if btn_rect(ui, minus, ("pk_minus", id)).clicked() {
        delta = -1;
    }
    if btn_rect(ui, plus, ("pk_plus", id)).clicked() {
        delta = 1;
    }
    if ui.rect_contains_pointer(full) {
        let dy = ui.input(|i| i.raw_scroll_delta.y);
        if dy > 0.0 {
            delta = 1;
        } else if dy < 0.0 {
            delta = -1;
        }
    }
    if delta != 0 {
        *val = (*val as i32 + delta * step as i32).rem_euclid(modulo as i32) as u32;
    }
    let st = Stroke::new(1.4, MUTED);
    let mc = minus.center();
    p.line_segment([mc - vec2(4.0, 0.0), mc + vec2(4.0, 0.0)], st);
    let pc = plus.center();
    p.line_segment([pc - vec2(4.0, 0.0), pc + vec2(4.0, 0.0)], st);
    p.line_segment([pc - vec2(0.0, 4.0), pc + vec2(0.0, 4.0)], st);
    p.text(
        pos2(origin.x + 48.0, origin.y + 14.0),
        Align2::CENTER_CENTER,
        format!("{:02}", *val),
        semibold(13.0),
        TEXT,
    );
}

// =====================================================================
// App
// =====================================================================

#[derive(Clone, Copy, PartialEq)]
enum Section {
    Media,
    Alerts,
    System,
    Todo,
    Settings,
}

/// What the collapsed pill is currently showing (already truncated + measured).
#[derive(Clone, Default)]
struct PillView {
    key: String,
    kind: u8, // 0 idle, 1 title, 2 lyric, 3 notification
    main: String,
    sub: Option<String>,
    app: Option<String>,
    app_id: String,
    icon: Option<Arc<egui::ColorImage>>,
    w: f32,
    h: f32,
}

struct EmelkaNotch {
    shared: Arc<Mutex<Shared>>,
    notifs: Arc<Mutex<NotifState>>,
    tx: mpsc::Sender<Cmd>,
    upd: Arc<Mutex<Upd>>,
    section: Section,

    // appearance
    style: NotchStyle,
    osd_enabled: bool,
    osd: Arc<Mutex<OsdState>>,

    // animation / hover
    t: f32,
    open: bool,
    hover_since: Option<Instant>,
    last_inside: Instant,
    passthrough: bool,
    positioned: bool,

    // pill
    pill_w: f32,
    pill_h: f32,
    view: PillView,
    view_changed: Instant,
    animating: bool,

    // app icon textures (keyed by AUMID)
    textures: HashMap<String, Option<egui::TextureHandle>>,

    // system panel
    sys: System,
    last_refresh: Instant,
    cpu_hist: VecDeque<f32>,
    mem_hist: VecDeque<f32>,
    disks: Vec<DiskInfo>,
    last_disk: Instant,

    // tasks
    todos: Arc<Mutex<TodoState>>,
    todo_text: String,
    todo_due: Option<i64>,
    picker_open: bool,
    pk_date: NaiveDate,
    pk_h: u32,
    pk_m: u32,
    pk_view: (i32, u32), // year, month shown in the calendar

    // transitions
    prev_section: Option<Section>,
    tab_t: f32,
    tab_dir: f32,
    pk_anim: f32, // 0 = task list, 1 = date/time picker
    cur_alpha: f32,
    added: Option<(u64, Instant)>,
    add_flash: Option<Instant>,
    notif_clear: Option<Instant>,       // "Clear all" animation start
    notif_out: Vec<(Instant, Instant)>, // (notification.at, dismiss start)

    // [offset] 3/8: lyric offsets
    offsets: Offsets,
    off_app: bool, // false = adjust this song, true = adjust this app
    off_hold: Option<(u8, Instant, Instant)>, // (button, pressed at, last step)
}

impl EmelkaNotch {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup_fonts(&cc.egui_ctx);
        cc.egui_ctx.set_theme(egui::Theme::Dark);
        cc.egui_ctx
            .style_mut(|s| s.interaction.selectable_labels = false);

        let shared = Arc::new(Mutex::new(Shared::default()));
        let tx = spawn_media_worker(cc.egui_ctx.clone(), shared.clone());

        let notifs = Arc::new(Mutex::new(NotifState::default()));
        spawn_notif_worker(cc.egui_ctx.clone(), notifs.clone());

        let upd = Arc::new(Mutex::new(Upd::default()));
        if take_updated_marker().is_some() {
            push_notice(
                &notifs,
                "EmelkaNotch updated",
                format!("Now running v{VERSION}"),
            );
        }
        spawn_updater(cc.egui_ctx.clone(), upd.clone(), notifs.clone());

        let todos = Arc::new(Mutex::new(load_todos()));
        spawn_todo_worker(cc.egui_ctx.clone(), todos.clone(), notifs.clone());

        let (style, osd_enabled) = load_settings();
        let osd = Arc::new(Mutex::new(OsdState::default()));
        spawn_volume_worker(cc.egui_ctx.clone(), osd.clone());
        spawn_brightness_worker(cc.egui_ctx.clone(), osd.clone());

        let mut sys = System::new();
        sys.refresh_cpu_all();
        sys.refresh_memory();

        Self {
            shared,
            notifs,
            tx,
            upd,
            section: Section::Media,
            style,
            osd_enabled,
            osd,
            t: 0.0,
            open: false,
            hover_since: None,
            last_inside: Instant::now(),
            passthrough: true,
            positioned: false,
            pill_w: PILL_W,
            pill_h: PILL_H,
            view: PillView {
                w: PILL_W,
                h: PILL_H,
                ..Default::default()
            },
            view_changed: Instant::now(),
            animating: false,
            textures: HashMap::new(),
            sys,
            last_refresh: Instant::now(),
            cpu_hist: VecDeque::new(),
            mem_hist: VecDeque::new(),
            disks: read_disks(),
            last_disk: Instant::now(),
            todos,
            todo_text: String::new(),
            todo_due: None,
            picker_open: false,
            pk_date: Local::now().date_naive(),
            pk_h: 9,
            pk_m: 0,
            pk_view: (Local::now().year(), Local::now().month()),
            prev_section: None,
            tab_t: 1.0,
            tab_dir: 1.0,
            pk_anim: 0.0,
            cur_alpha: 1.0,
            added: None,
            add_flash: None,
            notif_clear: None,
            notif_out: Vec::new(),
            // [offset] 3/8
            offsets: load_offsets(),
            off_app: false,
            off_hold: None,
        }
    }

    /// Jarak dari tepi atas jendela ke bentuk notch.
    fn top(&self) -> f32 {
        match self.style {
            NotchStyle::Floating => TOP,
            NotchStyle::Attached => 0.0,
        }
    }

    fn corner_radius(&self, e: f32) -> CornerRadius {
        match self.style {
            NotchStyle::Floating => {
                CornerRadius::same(lerp((self.pill_h / 2.0).min(22.0), 26.0, e).round() as u8)
            }
            NotchStyle::Attached => {
                let r = lerp(10.0, 24.0, e).round() as u8;
                CornerRadius {
                    nw: 0,
                    ne: 0,
                    sw: r,
                    se: r,
                }
            }
        }
    }

    fn icon_tex(
        &mut self,
        ctx: &egui::Context,
        id: &str,
        img: &Option<Arc<egui::ColorImage>>,
    ) -> Option<egui::TextureHandle> {
        if let Some(t) = self.textures.get(id) {
            return t.clone();
        }
        let t = img.as_ref().map(|i| {
            ctx.load_texture(
                format!("app-{id}"),
                (**i).clone(),
                egui::TextureOptions::LINEAR,
            )
        });
        self.textures.insert(id.to_string(), t.clone());
        t
    }

    /// Sample CPU/RAM once per second so the charts are already filled when the panel opens.
    fn sample_system(&mut self) {
        if self.last_refresh.elapsed() < Duration::from_secs(1) {
            return;
        }
        self.last_refresh = Instant::now();
        self.sys.refresh_cpu_all();
        self.sys.refresh_memory();

        let total = self.sys.total_memory() as f32;
        let mem = if total > 0.0 {
            self.sys.used_memory() as f32 / total
        } else {
            0.0
        };
        push_hist(&mut self.cpu_hist, self.sys.global_cpu_usage() / 100.0);
        push_hist(&mut self.mem_hist, mem);

        if self.last_disk.elapsed() > Duration::from_secs(15) {
            self.disks = read_disks();
            self.last_disk = Instant::now();
        }
    }

    fn update_hover(&mut self, ctx: &egui::Context) {
        let shape = shape_rect(self.pill_w, self.pill_h, ease(self.t), self.top());
        let hit = Rect::from_min_max(
            pos2(shape.left() - 4.0, 0.0),
            pos2(shape.right() + 4.0, shape.bottom() + 4.0),
        );
        let over = cursor_local(ctx).map_or(false, |c| hit.contains(c));
        // Keep the panel open while a text field has keyboard focus.
        let typing = ctx.input(|i| i.focused) && ctx.memory(|m| m.focused().is_some());
        let inside = over || (typing && self.open);

        if inside {
            self.last_inside = Instant::now();
            self.hover_since.get_or_insert_with(Instant::now);
        } else {
            self.hover_since = None;
        }

        self.open = match self.hover_since {
            Some(s) if s.elapsed() >= Duration::from_millis(100) => true,
            _ => self.open && self.last_inside.elapsed() < Duration::from_millis(450),
        };

        // Animate
        let dt = ctx.input(|i| i.stable_dt).min(0.05);
        let step = dt / if self.open { 0.40 } else { 0.30 };
        self.t = if self.open {
            (self.t + step).min(1.0)
        } else {
            (self.t - step).max(0.0)
        };

        // Let clicks fall through the transparent area when collapsed and not hovered.
        let want_passthrough = !(inside || self.t > 0.0);
        if want_passthrough != self.passthrough {
            self.passthrough = want_passthrough;
            ctx.send_viewport_cmd(egui::ViewportCommand::MousePassthrough(want_passthrough));
        }
    }

    /// Decide what the pill shows (osd > notification > lyric > title > idle) and measure it.
    // [offset] 4/8: new `lead` parameter
    #[allow(clippy::too_many_arguments)]
    fn refresh_view(
        &mut self,
        p: &Painter,
        media: &Media,
        lyrics: &Lyrics,
        pos_ms: u64,
        lead: i64,
        notif: Option<&Notif>,
        osd: Option<OsdEvent>,
    ) {
        let lyric_line = if media.playing {
            current_line(lyrics, shift(pos_ms, lead))
        } else {
            None
        };

        let (kind, main, sub, app): (u8, String, Option<String>, Option<String>) =
            if let Some(o) = osd {
                let name = match o.kind {
                    OsdKind::Volume => "Volume",
                    OsdKind::Brightness => "Brightness",
                };
                (4, name.to_string(), None, None)
            } else if let Some(n) = notif {
                let title = if n.title.is_empty() {
                    n.app.clone()
                } else {
                    n.title.clone()
                };
                (
                    3,
                    one_line(&title),
                    Some(one_line(&n.body)),
                    Some(n.app.clone()),
                )
            } else if let Some(line) = lyric_line {
                let t = if line.trim().is_empty() {
                    "•••".to_string()
                } else {
                    line.to_string()
                };
                (2, t, None, None)
            } else if media.available {
                (1, media.title.clone(), None, None)
            } else {
                (0, greeting(), None, None)
            };

        let (app_id, icon) = notif
            .map(|n| (n.app_id.clone(), n.icon.clone()))
            .unwrap_or_default();

        let key = format!("{kind}|{main}|{}", sub.as_deref().unwrap_or(""));
        if key == self.view.key {
            return;
        }
        self.view_changed = Instant::now();

        if kind == 3 {
            let max = PANEL_W - 66.0;
            let (m, mw) = fit(p, &main, semibold(12.0), max);
            let (s, sw) = match &sub {
                Some(b) if !b.is_empty() => {
                    let (t, w) = fit(p, b, FontId::proportional(11.0), max);
                    (Some(t), w)
                }
                _ => (None, 0.0),
            };
            self.view = PillView {
                key,
                kind,
                main: m,
                sub: s,
                app,
                app_id,
                icon,
                w: (44.0 + mw.max(sw) + 22.0).clamp(PILL_W, PANEL_W),
                h: NOTIF_H,
            };
        } else {
            let (m, mw) = fit(p, &main, semibold(12.0), PANEL_W - 62.0);
            self.view = PillView {
                key,
                kind,
                main: m,
                sub: None,
                app: None,
                app_id: String::new(),
                icon: None,
                w: (mw + 62.0).clamp(PILL_W, PANEL_W),
                h: PILL_H,
            };
        }
    }

    fn draw(&mut self, ui: &mut Ui, e: f32) {
        let painter = ui.painter().clone();

        let (media, snapshot, lyrics) = {
            let sh = self.shared.lock().unwrap();
            (sh.media.clone(), sh.snapshot, sh.lyrics.clone())
        };
        let mut pos_ms = media.pos_ms;
        if media.playing {
            pos_ms += snapshot.map_or(0, |s| s.elapsed().as_millis() as u64);
        }
        if media.dur_ms > 0 {
            pos_ms = pos_ms.min(media.dur_ms);
        }

        let notif = {
            let n = self.notifs.lock().unwrap();
            n.latest.clone().filter(|x| x.at.elapsed() < NOTIF_SHOW)
        };
        let osd = if self.osd_enabled {
            self.osd
                .lock()
                .unwrap()
                .latest
                .filter(|o| o.at.elapsed() < OSD_SHOW)
        } else {
            None
        };

        // [offset] 5/8
        let lead = self.lead_for(&media);
        self.refresh_view(&painter, &media, &lyrics, pos_ms, lead, notif.as_ref(), osd);

        // Smoothly follow the target pill size.
        let dt = ui.input(|i| i.stable_dt).min(0.05);
        let k = 1.0 - (-dt * 16.0).exp();
        self.pill_w += (self.view.w - self.pill_w) * k;
        self.pill_h += (self.view.h - self.pill_h) * k;
        self.animating = notif.is_some()
            || osd.is_some()
            || (self.view.w - self.pill_w).abs() > 0.5
            || (self.view.h - self.pill_h).abs() > 0.5;

        let top = self.top();
        let rect = shape_rect(self.pill_w, self.pill_h, e, top);
        let radius = self.corner_radius(e);

        painter.rect_filled(rect, radius, BG);
        if e > 0.0 {
            painter.rect_stroke(
                rect,
                radius,
                Stroke::new(1.0, BORDER.gamma_multiply(e)),
                StrokeKind::Inside,
            );
        }

        // Collapsed pill content
        let pill_alpha = (1.0 - e * 3.0).clamp(0.0, 1.0);
        if pill_alpha > 0.0 {
            let v = self.view.clone();
            let fade = (self.view_changed.elapsed().as_secs_f32() / 0.2).clamp(0.0, 1.0);
            let a = pill_alpha * fade;
            let p = painter.with_clip_rect(rect);
            let c = rect.center();

            if v.kind == 3 {
                let tex = self.icon_tex(ui.ctx(), &v.app_id, &v.icon);
                app_icon(
                    &p,
                    pos2(rect.left() + 24.0, c.y),
                    11.0,
                    tex.as_ref(),
                    v.app.as_deref().unwrap_or("?"),
                    pill_alpha,
                );
                p.text(
                    pos2(rect.left() + 44.0, c.y - 8.0),
                    Align2::LEFT_CENTER,
                    &v.main,
                    semibold(12.0),
                    TEXT.gamma_multiply(a),
                );
                if let Some(s) = &v.sub {
                    p.text(
                        pos2(rect.left() + 44.0, c.y + 8.0),
                        Align2::LEFT_CENTER,
                        s,
                        FontId::proportional(11.0),
                        MUTED.gamma_multiply(a),
                    );
                }
            } else if v.kind == 4 {
                // Volume / brightness: icon + progress bar + percentage (value is read live,
                // so dragging the slider doesn't re-trigger the fade).
                if let Some(o) = osd {
                    let muted = o.kind == OsdKind::Volume && o.muted;
                    let val = if muted { 0.0 } else { o.value.clamp(0.0, 1.0) };
                    osd_icon(
                        &p,
                        pos2(rect.left() + 24.0, c.y),
                        o.kind,
                        muted,
                        TEXT.gamma_multiply(pill_alpha),
                    );
                    let bar_l = rect.left() + 42.0;
                    let bar_r = (rect.right() - 54.0).max(bar_l + 10.0);
                    let track = Rect::from_min_max(pos2(bar_l, c.y - 2.0), pos2(bar_r, c.y + 2.0));
                    p.rect_filled(track, CornerRadius::same(2), TRACK.gamma_multiply(a));
                    p.rect_filled(
                        Rect::from_min_size(track.min, vec2(track.width() * val, 4.0)),
                        CornerRadius::same(2),
                        (if muted { DIM } else { TEXT }).gamma_multiply(a),
                    );
                    let label = if muted {
                        "Muted".to_string()
                    } else {
                        format!("{:.0}%", val * 100.0)
                    };
                    p.text(
                        pos2(rect.right() - 16.0, c.y),
                        Align2::RIGHT_CENTER,
                        label,
                        semibold(12.0),
                        (if muted { MUTED } else { TEXT }).gamma_multiply(a),
                    );
                }
            } else {
                let time = ui.input(|i| i.time) as f32;
                let eq_color = if media.available { ACCENT } else { DIM };
                let ppp = ui.ctx().pixels_per_point();
                let bw = (3.0 * ppp).round().max(1.0);
                let gp = (2.0 * ppp).round().max(1.0);
                let x0 = ((rect.left() + 20.0) * ppp).round();
                for i in 0..3 {
                    let h = if media.playing {
                        4.0 + 9.0 * ((time * 7.0 + i as f32 * 1.7).sin() * 0.5 + 0.5)
                    } else {
                        4.0
                    };
                    let hp = ((h * ppp / 2.0).round() * 2.0).max(2.0);
                    let left = (x0 + i as f32 * (bw + gp)) / ppp;
                    let top = (c.y * ppp - hp / 2.0).round() / ppp;
                    let r = Rect::from_min_size(pos2(left, top), vec2(bw / ppp, hp / ppp));
                    p.rect_filled(
                        r,
                        CornerRadius::same(1),
                        eq_color.gamma_multiply(pill_alpha),
                    );
                }
                p.text(
                    pos2(rect.left() + 42.0, c.y),
                    Align2::LEFT_CENTER,
                    &v.main,
                    semibold(12.0),
                    TEXT.gamma_multiply(a),
                );
            }
        }

        // Expanded panel content
        let panel_alpha = ((e - 0.5) / 0.5).clamp(0.0, 1.0);
        if panel_alpha > 0.0 {
            let inner = Rect::from_min_size(
                pos2((WIN_W - PANEL_W) / 2.0 + PAD, top + PAD),
                vec2(PANEL_W - PAD * 2.0, PANEL_H - PAD * 2.0),
            );
            let mut child = ui.new_child(
                UiBuilder::new()
                    .id_salt("panel")
                    .max_rect(inner)
                    .layout(egui::Layout::top_down(egui::Align::Min)),
            );
            child.set_clip_rect(child.clip_rect().intersect(rect));
            child.set_opacity(panel_alpha);
            self.cur_alpha = panel_alpha;
            self.draw_panel(&mut child, &media, pos_ms, &lyrics);
        }
    }

    fn draw_panel(&mut self, ui: &mut Ui, media: &Media, pos_ms: u64, lyrics: &Lyrics) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for (s, label) in [
                (Section::Media, "Media"),
                (Section::Alerts, "Notification"),
                (Section::System, "System"),
                (Section::Todo, "Tasks"),
                (Section::Settings, "Settings"),
            ] {
                if tab_icon(ui, s, label, self.section == s) && self.section != s {
                    self.prev_section = Some(self.section);
                    self.tab_dir = if (s as usize) > (self.section as usize) {
                        1.0
                    } else {
                        -1.0
                    };
                    self.tab_t = 0.0;
                    self.section = s;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.update_button(ui);
            });
        });
        ui.add_space(10.0);

        let dt = ui.input(|i| i.stable_dt).min(0.05);
        if self.prev_section.is_some() {
            self.tab_t = (self.tab_t + dt / 0.30).min(1.0);
            if self.tab_t >= 1.0 {
                self.prev_section = None;
            }
        }
        let content = ui.available_rect_before_wrap();
        match self.prev_section {
            Some(prev) => {
                // Slide: old content leaves, new content enters from the side of the clicked tab.
                let e = ease_out(self.tab_t);
                let w = content.width();
                let a = self.cur_alpha;
                let mut old = slide_child(
                    ui,
                    content,
                    -self.tab_dir * e * w,
                    a * (1.0 - e),
                    ("sec", prev as u8),
                );
                self.draw_section(&mut old, prev, media, pos_ms, lyrics);
                let cur = self.section;
                let mut new = slide_child(
                    ui,
                    content,
                    self.tab_dir * (1.0 - e) * w,
                    a * e,
                    ("sec", cur as u8),
                );
                self.draw_section(&mut new, cur, media, pos_ms, lyrics);
                ui.ctx().request_repaint();
            }
            None => self.draw_section(ui, self.section, media, pos_ms, lyrics),
        }
    }

    fn draw_section(
        &mut self,
        ui: &mut Ui,
        sec: Section,
        media: &Media,
        pos_ms: u64,
        lyrics: &Lyrics,
    ) {
        match sec {
            Section::Media => self.media_section(ui, media, pos_ms, lyrics),
            Section::Alerts => self.alerts_section(ui),
            Section::System => self.system_section(ui),
            Section::Todo => self.todo_section(ui),
            Section::Settings => self.settings_section(ui),
        }
    }

    fn update_button(&self, ui: &mut Ui) {
        let status = self.upd.lock().unwrap().clone();
        let (label, busy) = match &status {
            Upd::Idle => (format!("v{VERSION} · Check for updates"), false),
            Upd::Checking => ("Checking…".to_string(), true),
            Upd::UpToDate => (format!("v{VERSION} · Up to date"), false),
            Upd::Updating(t) => (format!("Updating to {t}…"), true),
            Upd::Failed => ("Update failed · Retry".to_string(), false),
        };
        let color = if busy { DIM } else { MUTED };
        let galley = ui
            .painter()
            .layout_no_wrap(label, FontId::proportional(10.5), color);
        let (rect, resp) = ui.allocate_exact_size(galley.size() + vec2(14.0, 10.0), Sense::click());
        if resp.hovered() && !busy {
            ui.painter()
                .rect_filled(rect, CornerRadius::same(8), Color32::from_rgb(24, 25, 31));
        }
        ui.painter()
            .galley(rect.center() - galley.size() / 2.0, galley, color);

        if resp
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked()
            && !busy
        {
            let (upd, notifs, ctx) = (self.upd.clone(), self.notifs.clone(), ui.ctx().clone());
            thread::spawn(move || run_update(upd, notifs, ctx, true));
        }
    }

    fn settings_section(&mut self, ui: &mut Ui) {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.label(RichText::new("Appearance").font(semibold(12.0)).color(TEXT));
        ui.add_space(6.0);

        let gap = 8.0;
        let w = (ui.available_width() - gap) / 2.0;
        let h = 80.0;
        let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::hover());
        let mut chosen: Option<NotchStyle> = None;

        let options = [
            (
                NotchStyle::Floating,
                "Floating",
                "Floating pill below the top edge",
            ),
            (
                NotchStyle::Attached,
                "Attached",
                "Attached to the top edge of the screen",
            ),
        ];
        for (i, (st, name, desc)) in options.iter().enumerate() {
            let r = Rect::from_min_size(
                pos2(row.left() + i as f32 * (w + gap), row.top()),
                vec2(w, h),
            );
            let resp = btn_rect(ui, r, ("style_opt", i));
            let sel = self.style == *st;
            let p = ui.painter().clone();

            p.rect_filled(
                r,
                CornerRadius::same(12),
                if resp.hovered() {
                    field_fill(true, false)
                } else {
                    CARD
                },
            );
            p.rect_stroke(
                r,
                CornerRadius::same(12),
                Stroke::new(
                    if sel { 1.5 } else { 1.0 },
                    if sel { ACCENT } else { BORDER },
                ),
                StrokeKind::Inside,
            );

            // mini preview
            let pv =
                Rect::from_min_size(pos2(r.left() + 12.0, r.top() + 10.0), vec2(w - 24.0, 30.0));
            p.rect_filled(pv, CornerRadius::same(6), Color32::from_rgb(28, 30, 38));
            let (pw, ph) = (56.0, 12.0);
            let x = pv.center().x - pw / 2.0;
            let (prect, rad) = match st {
                NotchStyle::Floating => (
                    Rect::from_min_size(pos2(x, pv.top() + 5.0), vec2(pw, ph)),
                    CornerRadius::same(6),
                ),
                NotchStyle::Attached => (
                    Rect::from_min_size(pos2(x, pv.top()), vec2(pw, ph)),
                    CornerRadius {
                        nw: 0,
                        ne: 0,
                        sw: 6,
                        se: 6,
                    },
                ),
            };
            p.rect_filled(prect, rad, BG);

            p.text(
                pos2(r.left() + 12.0, r.top() + 54.0),
                Align2::LEFT_CENTER,
                *name,
                semibold(12.0),
                if sel { TEXT } else { MUTED },
            );
            p.text(
                pos2(r.left() + 12.0, r.top() + 68.0),
                Align2::LEFT_CENTER,
                *desc,
                FontId::proportional(10.0),
                DIM,
            );

            if resp.clicked() {
                chosen = Some(*st);
            }
        }

        if let Some(s) = chosen {
            if s != self.style {
                self.style = s;
                save_settings(self.style, self.osd_enabled);
            }
        }

        // ---- OSD toggle ----
        ui.add_space(8.0);
        let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
        let resp = btn_rect(ui, row, "osd_toggle");
        let p = ui.painter().clone();
        p.rect_filled(
            row,
            CornerRadius::same(10),
            if resp.hovered() {
                field_fill(true, false)
            } else {
                CARD
            },
        );
        p.rect_stroke(
            row,
            CornerRadius::same(10),
            Stroke::new(1.0, BORDER),
            StrokeKind::Inside,
        );
        p.text(
            pos2(row.left() + 12.0, row.center().y),
            Align2::LEFT_CENTER,
            "Show volume & brightness in the pill",
            semibold(11.5),
            TEXT,
        );
        let k = ui
            .ctx()
            .animate_bool_with_time(egui::Id::new("osd_sw"), self.osd_enabled, 0.15);
        let sw = Rect::from_center_size(pos2(row.right() - 27.0, row.center().y), vec2(30.0, 16.0));
        p.rect_filled(sw, CornerRadius::same(8), TRACK.lerp_to_gamma(ACCENT, k));
        p.circle_filled(
            pos2(lerp(sw.left() + 8.0, sw.right() - 8.0, k), sw.center().y),
            5.5,
            TEXT,
        );
        if resp.clicked() {
            self.osd_enabled = !self.osd_enabled;
            save_settings(self.style, self.osd_enabled);
        }
    }

    fn alerts_section(&mut self, ui: &mut Ui) {
        let (items, total, err) = {
            let n = self.notifs.lock().unwrap();
            (
                n.items.iter().take(3).cloned().collect::<Vec<_>>(),
                n.items.len(),
                n.error.clone(),
            )
        };

        if items.is_empty() {
            self.notif_clear = None;
            self.notif_out.clear();
            ui.add_space(26.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new("No notifications yet")
                        .font(semibold(14.0))
                        .color(TEXT),
                );
                ui.add_space(2.0);
                ui.label(small(err.unwrap_or_else(|| {
                    "New Windows notifications will show up here.".into()
                })));
            });
            return;
        }

        const DUR: f32 = 0.25; // slide-out duration
        const STAGGER: f32 = 0.06; // delay between rows on "Clear all"
        let mut start_all = false;
        let mut start_one: Option<Instant> = None;

        // ---- header: count + "Clear all" ----
        let (head, _) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
        ui.painter().text(
            pos2(head.left() + 4.0, head.center().y),
            Align2::LEFT_CENTER,
            format!("{total} notification{}", if total == 1 { "" } else { "s" }),
            FontId::proportional(10.5),
            DIM,
        );
        let label = "Clear all";
        let gw = text_w(ui.painter(), label, &FontId::proportional(10.5));
        let btn = Rect::from_min_size(
            pos2(head.right() - gw - 16.0, head.top()),
            vec2(gw + 16.0, 20.0),
        );
        let r = btn_rect(ui, btn, "notif_clear_all");
        if r.hovered() {
            ui.painter()
                .rect_filled(btn, CornerRadius::same(6), Color32::from_rgb(24, 25, 31));
        }
        ui.painter().text(
            btn.center(),
            Align2::CENTER_CENTER,
            label,
            FontId::proportional(10.5),
            if r.hovered() { TEXT } else { MUTED },
        );
        if r.clicked() && self.notif_clear.is_none() {
            start_all = true;
        }

        // ---- items: hover shows an x; removed rows slide right, fade, then collapse ----
        let anim_all = self.notif_clear.map(|t| t.elapsed().as_secs_f32());
        let mut animating = anim_all.is_some() || !self.notif_out.is_empty();
        ui.spacing_mut().item_spacing.y = 2.0;
        for (i, n) in items.iter().enumerate() {
            let k = if let Some(el) = anim_all {
                Some(((el - i as f32 * STAGGER) / DUR).clamp(0.0, 1.0))
            } else {
                self.notif_out
                    .iter()
                    .find(|(at, _)| *at == n.at)
                    .map(|(_, st)| (st.elapsed().as_secs_f32() / DUR).clamp(0.0, 1.0))
            };
            let exiting = k.is_some();
            let k = k.unwrap_or(0.0);
            let collapse = ease(((k - 0.6) / 0.4).clamp(0.0, 1.0));
            let h = 36.0 * (1.0 - collapse);
            if h < 0.5 {
                continue;
            }
            let w = ui.available_width();
            let (rect, resp) = ui.allocate_exact_size(
                vec2(w, h),
                if exiting {
                    Sense::hover()
                } else {
                    Sense::click()
                },
            );
            let full = Rect::from_min_size(rect.min, vec2(w, 36.0))
                .translate(vec2(ease_out(k) * w * 0.6, 0.0));
            let tex = self.icon_tex(ui.ctx(), &n.app_id, &n.icon);
            let mut p = ui.painter_at(rect);
            if exiting {
                p.set_opacity(1.0 - k);
            }
            let hovered = resp.hovered() && !exiting;
            if hovered {
                p.rect_filled(full, CornerRadius::same(8), Color32::from_rgb(16, 17, 22));
            }
            app_icon(
                &p,
                pos2(full.left() + 16.0, full.center().y),
                11.0,
                tex.as_ref(),
                &n.app,
                1.0,
            );

            let max = w - 40.0 - 48.0;
            let title_src = if n.title.is_empty() { &n.app } else { &n.title };
            let (title, _) = fit(&p, &one_line(title_src), semibold(12.0), max);
            let (body, _) = fit(&p, &one_line(&n.body), FontId::proportional(11.0), max);

            p.text(
                pos2(full.left() + 38.0, full.top() + 11.0),
                Align2::LEFT_CENTER,
                title,
                semibold(12.0),
                TEXT,
            );
            p.text(
                pos2(full.left() + 38.0, full.top() + 26.0),
                Align2::LEFT_CENTER,
                body,
                FontId::proportional(11.0),
                MUTED,
            );
            if hovered {
                let c = pos2(full.right() - 14.0, full.center().y);
                let st = Stroke::new(1.3, MUTED);
                p.line_segment([c + vec2(-3.5, -3.5), c + vec2(3.5, 3.5)], st);
                p.line_segment([c + vec2(-3.5, 3.5), c + vec2(3.5, -3.5)], st);
                if resp.clicked() {
                    start_one = Some(n.at);
                }
                resp.on_hover_cursor(egui::CursorIcon::PointingHand);
            } else {
                p.text(
                    pos2(full.right() - 8.0, full.center().y),
                    Align2::RIGHT_CENTER,
                    ago(n.at),
                    FontId::proportional(10.0),
                    DIM,
                );
            }
        }

        if start_all {
            self.notif_clear = Some(Instant::now());
        }
        if let Some(at) = start_one {
            if !self.notif_out.iter().any(|(a, _)| *a == at) {
                self.notif_out.push((at, Instant::now()));
            }
        }

        // ---- finish: actually remove once the animation is done ----
        let all_done = anim_all.map_or(false, |el| {
            el >= DUR + STAGGER * items.len().saturating_sub(1) as f32 + 0.02
        });
        let done: Vec<Instant> = self
            .notif_out
            .iter()
            .filter(|(_, st)| st.elapsed().as_secs_f32() >= DUR)
            .map(|(a, _)| *a)
            .collect();
        if all_done || !done.is_empty() {
            let mut st = self.notifs.lock().unwrap();
            if all_done {
                st.items.clear();
                st.latest = None;
                self.notif_clear = None;
                self.notif_out.clear();
            } else {
                st.items.retain(|n| !done.contains(&n.at));
                self.notif_out.retain(|(a, _)| !done.contains(a));
            }
        }
        if animating || start_all || start_one.is_some() {
            animating = true;
        }
        if animating {
            ui.ctx().request_repaint();
        }
    }

    // [offset] 6/8: helpers for the offset control
    fn lead_for(&self, media: &Media) -> i64 {
        let t = self
            .offsets
            .tracks
            .get(&track_key(media))
            .copied()
            .unwrap_or(0);
        let a = self.offsets.apps.get(&app_key(media)).copied().unwrap_or(0);
        LYRIC_LEAD_MS + a + t
    }

    fn scope_offset(&self, media: &Media) -> i64 {
        if self.off_app {
            self.offsets.apps.get(&app_key(media)).copied().unwrap_or(0)
        } else {
            self.offsets
                .tracks
                .get(&track_key(media))
                .copied()
                .unwrap_or(0)
        }
    }

    /// Appears over the lyrics area while hovering it.
    fn offset_control(&mut self, ui: &mut Ui, area: Rect, media: &Media) {
        let hover = ui.rect_contains_pointer(area);
        let a = ui
            .ctx()
            .animate_bool_with_time(egui::Id::new("off_ctl"), hover, 0.15);
        if a <= 0.01 {
            return;
        }
        let mut p = ui.painter_at(area);
        p.set_opacity(a);

        let scope_label = if self.off_app {
            ellipsize(&app_label(&media.app), 10)
        } else {
            "Song".to_string()
        };
        let (pad, gap, btn_w, val_w, h) = (4.0, 2.0, 22.0, 46.0, 24.0);
        let chip_w = text_w(&p, &scope_label, &semibold(10.5)) + 16.0;
        let total = pad * 2.0 + chip_w + gap * 3.0 + btn_w * 2.0 + val_w;
        let r = Rect::from_min_size(
            pos2(area.right() - total, area.center().y - h / 2.0),
            vec2(total, h),
        );
        p.rect_filled(r, CornerRadius::same(8), CARD);
        p.rect_stroke(
            r,
            CornerRadius::same(8),
            Stroke::new(1.0, BORDER),
            StrokeKind::Inside,
        );

        let (y, hh) = (r.top() + 2.0, h - 4.0);
        let chip = Rect::from_min_size(pos2(r.left() + pad, y), vec2(chip_w, hh));
        let minus = Rect::from_min_size(pos2(chip.right() + gap, y), vec2(btn_w, hh));
        let val = Rect::from_min_size(pos2(minus.right() + gap, y), vec2(val_w, hh));
        let plus = Rect::from_min_size(pos2(val.right() + gap, y), vec2(btn_w, hh));

        let r_chip = btn_rect(ui, chip, "off_scope");
        let r_minus = btn_rect(ui, minus, "off_minus");
        let r_val = btn_rect(ui, val, "off_val");
        let r_plus = btn_rect(ui, plus, "off_plus");

        let hl = Color32::from_rgb(32, 34, 42);
        for (rc, rr) in [
            (chip, &r_chip),
            (minus, &r_minus),
            (val, &r_val),
            (plus, &r_plus),
        ] {
            if rr.hovered() {
                p.rect_filled(rc, CornerRadius::same(6), hl);
            }
        }

        p.text(
            chip.center(),
            Align2::CENTER_CENTER,
            &scope_label,
            semibold(10.5),
            if self.off_app { ACCENT } else { TEXT },
        );

        let st = Stroke::new(1.4, if r_minus.hovered() { TEXT } else { MUTED });
        let mc = minus.center();
        p.line_segment([mc - vec2(4.0, 0.0), mc + vec2(4.0, 0.0)], st);

        let st = Stroke::new(1.4, if r_plus.hovered() { TEXT } else { MUTED });
        let pc = plus.center();
        p.line_segment([pc - vec2(4.0, 0.0), pc + vec2(4.0, 0.0)], st);
        p.line_segment([pc - vec2(0.0, 4.0), pc + vec2(0.0, 4.0)], st);

        let cur = self.scope_offset(media);
        let vlabel = if cur == 0 {
            "0.0s".to_string()
        } else {
            format!("{:+.1}s", cur as f32 / 1000.0)
        };
        p.text(
            val.center(),
            Align2::CENTER_CENTER,
            vlabel,
            FontId::proportional(11.0),
            if cur != 0 { TEXT } else { MUTED },
        );

        let mut delta = 0i64;
        let mut reset = false;
        if r_chip.clicked() {
            self.off_app = !self.off_app;
        }
        if r_val.clicked() && cur != 0 {
            reset = true;
        }

        // Press-and-hold on - / + (accelerates the longer it's held)
        let held: Option<(u8, i64)> = if r_minus.is_pointer_button_down_on() {
            Some((1, -1))
        } else if r_plus.is_pointer_button_down_on() {
            Some((2, 1))
        } else {
            None
        };
        match held {
            Some((id, sign)) => {
                let now = Instant::now();
                match self.off_hold {
                    Some((hid, start, last)) if hid == id => {
                        let t = start.elapsed().as_secs_f32();
                        if t > 0.4 {
                            let (every, step) = if t > 1.5 { (0.10, 500) } else { (0.06, 100) };
                            if last.elapsed().as_secs_f32() >= every {
                                delta = sign * step;
                                self.off_hold = Some((id, start, now));
                            }
                        }
                    }
                    _ => {
                        delta = sign * 100; // first press
                        self.off_hold = Some((id, now, now));
                    }
                }
                ui.ctx().request_repaint();
            }
            None => self.off_hold = None,
        }

        if delta != 0 || reset {
            let key = if self.off_app {
                app_key(media)
            } else {
                track_key(media)
            };
            let map = if self.off_app {
                &mut self.offsets.apps
            } else {
                &mut self.offsets.tracks
            };
            let e = map.entry(key).or_insert(0);
            *e = if reset {
                0
            } else {
                (*e + delta).clamp(-15_000, 15_000)
            };
            self.offsets.apps.retain(|_, v| *v != 0);
            self.offsets.tracks.retain(|_, v| *v != 0);
            save_offsets(&self.offsets);
        }
    }

    // [offset] 7/8: now `&mut self`
    fn media_section(&mut self, ui: &mut Ui, media: &Media, pos_ms: u64, lyrics: &Lyrics) {
        if !media.available {
            ui.add_space(26.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new("Nothing playing")
                        .font(semibold(14.0))
                        .color(TEXT),
                );
                ui.add_space(2.0);
                ui.label(small(
                    "Play something in Spotify, YouTube Music (Chrome), etc.".into(),
                ));
            });
            return;
        }

        let width = ui.available_width();
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.set_width(width - 130.0);
                ui.add(
                    egui::Label::new(
                        RichText::new(ellipsize(&media.title, 36))
                            .font(semibold(15.0))
                            .color(TEXT),
                    )
                    .wrap_mode(egui::TextWrapMode::Extend),
                );
                ui.add(
                    egui::Label::new(
                        RichText::new(ellipsize(
                            &format!("{} · {}", media.artist, app_label(&media.app)),
                            48,
                        ))
                        .size(12.0)
                        .color(MUTED),
                    )
                    .wrap_mode(egui::TextWrapMode::Extend),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icon_button(ui, Icon::Next, 28.0, false).clicked() {
                    let _ = self.tx.send(Cmd::Next);
                }
                let icon = if media.playing {
                    Icon::Pause
                } else {
                    Icon::Play
                };
                if icon_button(ui, icon, 34.0, true).clicked() {
                    let _ = self.tx.send(Cmd::PlayPause);
                }
                if icon_button(ui, Icon::Prev, 28.0, false).clicked() {
                    let _ = self.tx.send(Cmd::Prev);
                }
            });
        });

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.label(small(fmt_time(pos_ms)));
            let bar_w = (ui.available_width() - 40.0).max(40.0);
            let frac = if media.dur_ms > 0 {
                pos_ms as f32 / media.dur_ms as f32
            } else {
                0.0
            };
            let resp = bar(ui, bar_w, frac, TEXT);
            if resp.clicked() && media.dur_ms > 0 {
                if let Some(p) = resp.interact_pointer_pos() {
                    let f = ((p.x - resp.rect.left()) / resp.rect.width()).clamp(0.0, 1.0);
                    let _ = self.tx.send(Cmd::Seek((f * media.dur_ms as f32) as u64));
                }
            }
            ui.label(small(fmt_time(media.dur_ms)));
        });

        ui.add_space(6.0);
        let (area, _) = ui.allocate_exact_size(vec2(ui.available_width(), 54.0), Sense::hover());
        let lead = self.lead_for(media);
        Self::draw_lyrics(ui, area, lyrics, shift(pos_ms, lead));
        self.offset_control(ui, area, media);
    }

    // [offset] 8/8: `pos_ms` already includes the lead/offset
    fn draw_lyrics(ui: &Ui, area: Rect, lyrics: &Lyrics, pos_ms: u64) {
        let painter = ui.painter_at(area);
        let center = area.center();
        match lyrics {
            Lyrics::Synced(lines) => {
                let pos = pos_ms;
                let idx = lines.partition_point(|(t, _)| *t <= pos) as f32 - 1.0;
                let anim = ui
                    .ctx()
                    .animate_value_with_time(egui::Id::new("lyric_idx"), idx, 0.28);
                let lh = 19.0;
                let base = anim.floor() as isize;
                for i in (base - 1)..=(base + 2) {
                    if i < 0 || i as usize >= lines.len() {
                        continue;
                    }
                    let dy = (i as f32 - anim) * lh;
                    let near = (1.0 - dy.abs() / (lh * 1.9)).clamp(0.0, 1.0);
                    let focus = (1.0 - dy.abs() / lh).clamp(0.0, 1.0);
                    let color = DIM.lerp_to_gamma(TEXT, focus).gamma_multiply(near);
                    painter.text(
                        pos2(center.x, center.y + dy),
                        Align2::CENTER_CENTER,
                        ellipsize(&lines[i as usize].1, 62),
                        FontId::proportional(13.0),
                        color,
                    );
                }
            }
            Lyrics::Loading => {
                painter.text(
                    center,
                    Align2::CENTER_CENTER,
                    "Looking for lyrics…",
                    FontId::proportional(12.0),
                    MUTED,
                );
            }
            Lyrics::NotFound | Lyrics::Idle => {
                painter.text(
                    center,
                    Align2::CENTER_CENTER,
                    "No synced lyrics found",
                    FontId::proportional(12.0),
                    DIM,
                );
            }
        }
    }

    fn system_section(&self, ui: &mut Ui) {
        let gb = 1_073_741_824.0;
        let cpu = self.cpu_hist.back().copied().unwrap_or(0.0);
        let mem = self.mem_hist.back().copied().unwrap_or(0.0);
        let used = self.sys.used_memory() as f64 / gb;
        let total = self.sys.total_memory() as f64 / gb;

        let gap = 8.0;
        let w = (ui.available_width() - gap * 2.0) / 3.0;
        let h = 132.0;
        let chart = |r: Rect| {
            Rect::from_min_max(
                pos2(r.left() + 12.0, r.top() + 34.0),
                pos2(r.right() - 12.0, r.bottom() - 28.0),
            )
        };

        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;

            // CPU
            let (r, p) = card(ui, w, h);
            card_header(&p, r, "CPU", &format!("{:.0}%", cpu * 100.0));
            sparkline(&p, chart(r), &self.cpu_hist, ACCENT);
            p.text(
                pos2(r.left() + 12.0, r.bottom() - 14.0),
                Align2::LEFT_CENTER,
                "Last 60s",
                FontId::proportional(10.0),
                DIM,
            );

            // RAM
            let (r, p) = card(ui, w, h);
            card_header(&p, r, "RAM", &format!("{:.0}%", mem * 100.0));
            sparkline(&p, chart(r), &self.mem_hist, PURPLE);
            p.text(
                pos2(r.left() + 12.0, r.bottom() - 14.0),
                Align2::LEFT_CENTER,
                format!("{used:.1} / {total:.1} GB"),
                FontId::proportional(10.0),
                MUTED,
            );

            // Storage (all drives)
            let (r, p) = card(ui, w, h);
            if self.disks.is_empty() {
                card_header(&p, r, "Storage", "");
                p.text(
                    r.center(),
                    Align2::CENTER_CENTER,
                    "No disk info",
                    FontId::proportional(11.0),
                    DIM,
                );
            } else {
                let n = self.disks.len();
                card_header(
                    &p,
                    r,
                    "Storage",
                    &format!("{n} drive{}", if n == 1 { "" } else { "s" }),
                );
                let shown = n.min(4);
                let row_h = (88.0 / shown as f32).min(30.0);
                let (x0, x1) = (r.left() + 12.0, r.right() - 12.0);
                for (i, d) in self.disks.iter().take(shown).enumerate() {
                    let top = r.top() + 36.0 + i as f32 * row_h;
                    let frac = (d.used_gb / d.total_gb.max(0.001)) as f32;
                    let col = if frac > 0.9 { WARN } else { GREEN };
                    p.text(
                        pos2(x0, top + 6.0),
                        Align2::LEFT_CENTER,
                        &d.name,
                        semibold(11.0),
                        TEXT,
                    );
                    p.text(
                        pos2(x1, top + 6.0),
                        Align2::RIGHT_CENTER,
                        format!("{} / {}", fmt_gb(d.used_gb), fmt_gb(d.total_gb)),
                        FontId::proportional(10.0),
                        MUTED,
                    );
                    let track = Rect::from_min_size(pos2(x0, top + 15.0), vec2(x1 - x0, 4.0));
                    p.rect_filled(track, CornerRadius::same(2), TRACK);
                    p.rect_filled(
                        Rect::from_min_size(
                            track.min,
                            vec2(track.width() * frac.clamp(0.0, 1.0), 4.0),
                        ),
                        CornerRadius::same(2),
                        col,
                    );
                }
            }
        });
    }

    fn add_todo(&mut self) -> bool {
        let text = self.todo_text.trim().to_string();
        if text.is_empty() {
            return false;
        }
        let due = self.todo_due;
        {
            let mut st = self.todos.lock().unwrap();
            st.next_id += 1;
            let id = st.next_id;
            st.items.push(Todo {
                id,
                text,
                done: false,
                due,
                notified: false,
            });
            save_todos(&st.items);
            self.added = Some((id, Instant::now()));
        }
        self.add_flash = Some(Instant::now());
        self.todo_text.clear();
        self.todo_due = None;
        true
    }

    fn open_picker(&mut self) {
        let base = self
            .todo_due
            .and_then(|t| Local.timestamp_opt(t, 0).single())
            .unwrap_or_else(|| Local::now() + chrono::Duration::hours(1));
        self.pk_date = base.date_naive();
        self.pk_h = base.hour();
        self.pk_m = if self.todo_due.is_some() {
            base.minute()
        } else {
            base.minute() / 5 * 5
        };
        self.pk_view = (base.year(), base.month());
        self.picker_open = true;
    }

    fn pk_ts(&self) -> Option<i64> {
        let dt = self.pk_date.and_hms_opt(self.pk_h, self.pk_m, 0)?;
        Local
            .from_local_datetime(&dt)
            .earliest()
            .map(|d| d.timestamp())
    }

    /// [ task text ........ ] [ date & time ] [ + ]  - same height and radius.
    fn todo_input_row(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), FIELD_H), Sense::hover());
        let gap = 6.0;
        let btn = Rect::from_min_size(
            pos2(row.right() - FIELD_H, row.top()),
            vec2(FIELD_H, FIELD_H),
        );
        let when = Rect::from_min_max(
            pos2(btn.left() - gap - DUE_W, row.top()),
            pos2(btn.left() - gap, row.bottom()),
        );
        let task = Rect::from_min_max(row.min, pos2(when.left() - gap, row.bottom()));

        // ---- task text ----
        let task_id = egui::Id::new("todo_text");
        let focused = ctx.memory(|m| m.has_focus(task_id));
        let hovered = ui.rect_contains_pointer(task);
        ui.painter().rect_filled(
            task,
            CornerRadius::same(FIELD_R),
            field_fill(hovered, focused),
        );
        if ui
            .interact(task, egui::Id::new("todo_text_box"), Sense::click())
            .clicked()
        {
            ctx.memory_mut(|m| m.request_focus(task_id));
        }
        let inner = task.shrink2(vec2(12.0, 0.0));
        let r1 = ui
            .scope_builder(
                UiBuilder::new()
                    .max_rect(inner)
                    .layout(egui::Layout::left_to_right(egui::Align::Center)),
                |ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.todo_text)
                            .id(task_id)
                            .frame(false)
                            .margin(egui::Margin::ZERO)
                            .desired_width(inner.width())
                            .font(FontId::proportional(12.5))
                            .text_color(TEXT)
                            .hint_text(RichText::new("Add a task…").color(DIM)),
                    )
                },
            )
            .inner;

        // ---- date & time button ----
        let main = ui.interact(when, egui::Id::new("todo_when"), Sense::click());
        let x_rect =
            Rect::from_min_size(pos2(when.right() - 26.0, when.top()), vec2(26.0, FIELD_H));
        let x_resp = if self.todo_due.is_some() {
            Some(btn_rect(ui, x_rect, "todo_when_clear"))
        } else {
            None
        };
        let main = main.on_hover_cursor(egui::CursorIcon::PointingHand);
        ui.painter().rect_filled(
            when,
            CornerRadius::same(FIELD_R),
            field_fill(main.hovered(), false),
        );
        // calendar icon
        {
            let p = ui.painter();
            let ic = pos2(when.left() + 16.0, when.center().y);
            let box_r = Rect::from_center_size(ic, vec2(11.0, 10.0));
            p.rect_stroke(
                box_r,
                CornerRadius::same(2),
                Stroke::new(1.2, MUTED),
                StrokeKind::Inside,
            );
            p.line_segment(
                [
                    pos2(box_r.left(), box_r.top() + 3.0),
                    pos2(box_r.right(), box_r.top() + 3.0),
                ],
                Stroke::new(1.2, MUTED),
            );
        }
        let now_ts = Local::now().timestamp();
        let (label, col) = match self.todo_due {
            Some(ts) => (fmt_due(ts), if ts <= now_ts { WARN } else { TEXT }),
            None => ("Date & time".to_string(), DIM),
        };
        ui.painter().text(
            pos2(when.left() + 30.0, when.center().y),
            Align2::LEFT_CENTER,
            label,
            FontId::proportional(11.5),
            col,
        );
        if let Some(xr) = &x_resp {
            let c = x_rect.center();
            let sc = if xr.hovered() { TEXT } else { MUTED };
            let st = Stroke::new(1.3, sc);
            let p = ui.painter();
            p.line_segment([c + vec2(-3.5, -3.5), c + vec2(3.5, 3.5)], st);
            p.line_segment([c + vec2(-3.5, 3.5), c + vec2(3.5, -3.5)], st);
        }
        if x_resp.as_ref().map_or(false, |r| r.clicked()) {
            self.todo_due = None;
        } else if main.clicked() {
            self.open_picker();
        }

        // ---- add button (flashes green with a check after a successful add) ----
        let resp = ui.interact(btn, egui::Id::new("todo_add"), Sense::click());
        let flash = self
            .add_flash
            .map(|t| t.elapsed().as_secs_f32())
            .filter(|e| *e < 0.7);
        let k = flash.map_or(0.0, |e| 1.0 - e / 0.7);
        let base = if resp.hovered() { Color32::WHITE } else { TEXT };
        ui.painter().rect_filled(
            btn,
            CornerRadius::same(FIELD_R),
            base.lerp_to_gamma(GREEN, k),
        );
        let c = btn.center();
        let s = Stroke::new(1.8, BG);
        if flash.map_or(false, |e| e < 0.45) {
            ui.painter()
                .line_segment([c + vec2(-5.0, 0.5), c + vec2(-1.5, 4.0)], s);
            ui.painter()
                .line_segment([c + vec2(-1.5, 4.0), c + vec2(5.5, -4.0)], s);
        } else {
            ui.painter()
                .line_segment([c - vec2(5.0, 0.0), c + vec2(5.0, 0.0)], s);
            ui.painter()
                .line_segment([c - vec2(0.0, 5.0), c + vec2(0.0, 5.0)], s);
        }
        let clicked = resp
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .clicked();

        let enter = r1.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if clicked || enter {
            if self.add_todo() {
                ctx.memory_mut(|m| m.request_focus(task_id));
            }
        }
    }

    /// Inline date + time picker (replaces the task list while open).
    fn picker_ui(&mut self, ui: &mut Ui) {
        let now = Local::now();
        let today = now.date_naive();
        let (area, _) = ui.allocate_exact_size(vec2(ui.available_width(), 138.0), Sense::hover());
        let p = ui.painter_at(area);
        let (cw, ch) = (26.0, 16.0);
        let cal_w = cw * 7.0;
        let (year, month) = self.pk_view;
        let Some(first) = NaiveDate::from_ymd_opt(year, month, 1) else {
            self.picker_open = false;
            return;
        };

        // ---- calendar header ----
        let hy = area.top() + 10.0;
        if arrow_btn(
            ui,
            &p,
            Rect::from_center_size(pos2(area.left() + 10.0, hy), vec2(20.0, 20.0)),
            -1.0,
            "prev",
        ) {
            self.pk_view = if month == 1 {
                (year - 1, 12)
            } else {
                (year, month - 1)
            };
        }
        if arrow_btn(
            ui,
            &p,
            Rect::from_center_size(pos2(area.left() + cal_w - 10.0, hy), vec2(20.0, 20.0)),
            1.0,
            "next",
        ) {
            self.pk_view = if month == 12 {
                (year + 1, 1)
            } else {
                (year, month + 1)
            };
        }
        p.text(
            pos2(area.left() + cal_w / 2.0, hy),
            Align2::CENTER_CENTER,
            first.format("%B %Y").to_string(),
            semibold(12.0),
            TEXT,
        );
        for (i, w) in ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"]
            .iter()
            .enumerate()
        {
            p.text(
                pos2(area.left() + cw * (i as f32 + 0.5), area.top() + 30.0),
                Align2::CENTER_CENTER,
                *w,
                FontId::proportional(10.0),
                DIM,
            );
        }

        // ---- day grid ----
        let grid_top = area.top() + 40.0;
        let offset = first.weekday().num_days_from_monday() as usize;
        for d in 1..=31u32 {
            let Some(date) = NaiveDate::from_ymd_opt(year, month, d) else {
                break;
            };
            let idx = offset + d as usize - 1;
            let cell = Rect::from_min_size(
                pos2(
                    area.left() + (idx % 7) as f32 * cw,
                    grid_top + (idx / 7) as f32 * ch,
                ),
                vec2(cw, ch),
            )
            .shrink2(vec2(1.0, 0.5));
            let past = date < today;
            let resp = if past {
                None
            } else {
                Some(btn_rect(ui, cell, ("pk_day", d)))
            };
            let hovered = resp.as_ref().map_or(false, |r| r.hovered());
            let selected = date == self.pk_date;
            if selected {
                p.rect_filled(cell, CornerRadius::same(5), TEXT);
            } else if hovered {
                p.rect_filled(cell, CornerRadius::same(5), Color32::from_rgb(32, 34, 42));
            }
            if date == today && !selected {
                p.rect_stroke(
                    cell,
                    CornerRadius::same(5),
                    Stroke::new(1.0, ACCENT),
                    StrokeKind::Inside,
                );
            }
            let col = if selected {
                BG
            } else if past {
                DIM
            } else {
                TEXT
            };
            p.text(
                cell.center(),
                Align2::CENTER_CENTER,
                d.to_string(),
                FontId::proportional(11.0),
                col,
            );
            if resp.map_or(false, |r| r.clicked()) {
                self.pk_date = date;
            }
        }

        // ---- right side: time, quick dates, actions ----
        let rx = area.left() + cal_w + 28.0;
        p.text(
            pos2(rx, area.top() + 8.0),
            Align2::LEFT_CENTER,
            "Time",
            FontId::proportional(11.0),
            MUTED,
        );
        stepper(
            ui,
            &p,
            pos2(rx, area.top() + 20.0),
            &mut self.pk_h,
            24,
            1,
            "h",
        );
        p.text(
            pos2(rx + 96.0 + 8.0, area.top() + 34.0),
            Align2::CENTER_CENTER,
            ":",
            semibold(13.0),
            DIM,
        );
        stepper(
            ui,
            &p,
            pos2(rx + 96.0 + 16.0, area.top() + 20.0),
            &mut self.pk_m,
            60,
            5,
            "m",
        );

        let mut quick: Option<NaiveDate> = None;
        let mut x = rx;
        for (label, w, days) in [
            ("Today", 56.0, 0i64),
            ("Tomorrow", 76.0, 1),
            ("Next week", 84.0, 7),
        ] {
            let r = Rect::from_min_size(pos2(x, area.top() + 58.0), vec2(w, 24.0));
            if text_btn(ui, &p, r, label, label, false, true) {
                quick = today.checked_add_signed(chrono::Duration::days(days));
            }
            x += w + 6.0;
        }
        if let Some(d) = quick {
            self.pk_date = d;
            self.pk_view = (d.year(), d.month());
        }

        // actions
        let by = area.top() + 104.0;
        let ts = self.pk_ts();
        let future = ts.map_or(false, |t| t > now.timestamp());
        let right = area.right();
        let done = Rect::from_min_size(pos2(right - 72.0, by), vec2(72.0, 28.0));
        let clear = Rect::from_min_size(pos2(right - 72.0 - 6.0 - 64.0, by), vec2(64.0, 28.0));
        let (sum, sum_col) = match ts {
            Some(t) if future => (fmt_due(t), TEXT),
            Some(_) => ("Time has passed".to_string(), WARN),
            None => (String::new(), TEXT),
        };
        p.text(
            pos2(rx, by + 14.0),
            Align2::LEFT_CENTER,
            sum,
            FontId::proportional(11.5),
            sum_col,
        );
        if text_btn(ui, &p, clear, "Clear", "clear", false, true) {
            self.todo_due = None;
            self.picker_open = false;
        }
        if text_btn(ui, &p, done, "Done", "done", true, future) {
            self.todo_due = ts;
            self.picker_open = false;
        }
    }

    fn todo_section(&mut self, ui: &mut Ui) {
        let dt = ui.input(|i| i.stable_dt).min(0.05);
        let target = if self.picker_open { 1.0 } else { 0.0 };
        let step = dt / 0.30;
        if self.pk_anim < target {
            self.pk_anim = (self.pk_anim + step).min(1.0);
        } else if self.pk_anim > target {
            self.pk_anim = (self.pk_anim - step).max(0.0);
        }

        if self.pk_anim <= 0.0 {
            self.todo_main(ui);
        } else if self.pk_anim >= 1.0 {
            self.picker_ui(ui);
        } else {
            // Task list slides out to the left, picker slides in from the right (and back).
            let rect = ui.available_rect_before_wrap();
            let e = ease(self.pk_anim);
            let w = rect.width();
            let a = self.cur_alpha;
            let mut c1 = slide_child(ui, rect, -e * w, a * (1.0 - e), "tk_main");
            self.todo_main(&mut c1);
            let mut c2 = slide_child(ui, rect, (1.0 - e) * w, a * e, "tk_picker");
            self.picker_ui(&mut c2);
            ui.ctx().request_repaint();
        }
    }

    fn todo_main(&mut self, ui: &mut Ui) {
        self.todo_input_row(ui);
        ui.add_space(6.0);

        // ---- list ----
        let mut items = self.todos.lock().unwrap().items.clone();
        items.sort_by_key(|t| (t.done, t.due.unwrap_or(i64::MAX), t.id));

        if items.is_empty() {
            ui.add_space(14.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new("No tasks yet")
                        .font(semibold(14.0))
                        .color(TEXT),
                );
                ui.add_space(2.0);
                ui.label(small(
                    "Pick a date & time to get a reminder (optional)".into(),
                ));
            });
            return;
        }

        let now = Local::now().timestamp();
        let added = self.added;
        let mut toggle: Option<u64> = None;
        let mut delete: Option<u64> = None;

        egui::ScrollArea::vertical()
            .max_height(100.0)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                for t in &items {
                    let (alloc, resp) =
                        ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
                    // New item: slides in from the right, fades in, and flashes green.
                    let anim = added
                        .filter(|(id, at)| *id == t.id && at.elapsed().as_secs_f32() < 0.45)
                        .map(|(_, at)| at.elapsed().as_secs_f32() / 0.45);
                    let mut p = ui.painter_at(alloc);
                    let rect = match anim {
                        Some(k) => {
                            let e = ease_out(k);
                            p.set_opacity(e);
                            p.rect_filled(
                                alloc,
                                CornerRadius::same(FIELD_R),
                                GREEN.gamma_multiply(0.16 * (1.0 - k)),
                            );
                            ui.scroll_to_rect(alloc, None);
                            alloc.translate(vec2((1.0 - e) * 60.0, 0.0))
                        }
                        None => alloc,
                    };
                    if resp.hovered() {
                        p.rect_filled(
                            rect,
                            CornerRadius::same(FIELD_R),
                            Color32::from_rgb(20, 21, 27),
                        );
                    }
                    let cy = rect.center().y;
                    let cc = pos2(rect.left() + 12.0, cy);
                    if t.done {
                        p.circle_filled(cc, 7.0, GREEN);
                        let s = Stroke::new(1.6, BG);
                        p.line_segment([cc + vec2(-3.0, 0.0), cc + vec2(-1.0, 2.5)], s);
                        p.line_segment([cc + vec2(-1.0, 2.5), cc + vec2(3.5, -2.5)], s);
                    } else {
                        p.circle_stroke(cc, 7.0, Stroke::new(1.5, DIM));
                    }

                    let right = rect.right() - 24.0;
                    let x = rect.left() + 28.0;
                    let mut text_max = right - x;
                    if let Some(d) = t.due {
                        let overdue = !t.done && d <= now;
                        let label = fmt_due(d);
                        let f = FontId::proportional(10.5);
                        let w = text_w(&p, &label, &f);
                        p.text(
                            pos2(right, cy),
                            Align2::RIGHT_CENTER,
                            label,
                            f,
                            if overdue { WARN } else { MUTED },
                        );
                        text_max -= w + 10.0;
                    }
                    let f = FontId::proportional(12.0);
                    let (txt, tw) = fit(&p, &one_line(&t.text), f.clone(), text_max.max(30.0));
                    let col = if t.done { DIM } else { TEXT };
                    p.text(pos2(x, cy), Align2::LEFT_CENTER, txt, f, col);
                    if t.done {
                        p.line_segment([pos2(x, cy), pos2(x + tw, cy)], Stroke::new(1.0, DIM));
                    }

                    if resp.hovered() {
                        let c = pos2(rect.right() - 11.0, cy);
                        let s = Stroke::new(1.3, MUTED);
                        p.line_segment([c + vec2(-3.5, -3.5), c + vec2(3.5, 3.5)], s);
                        p.line_segment([c + vec2(-3.5, 3.5), c + vec2(3.5, -3.5)], s);
                    }
                    if resp.clicked() {
                        if let Some(pos) = resp.interact_pointer_pos() {
                            if pos.x > rect.right() - 24.0 {
                                delete = Some(t.id);
                            } else {
                                toggle = Some(t.id);
                            }
                        }
                    }
                }
            });

        if toggle.is_some() || delete.is_some() {
            let mut st = self.todos.lock().unwrap();
            if let Some(id) = toggle {
                if let Some(t) = st.items.iter_mut().find(|t| t.id == id) {
                    t.done = !t.done;
                    // re-opened -> reminder is active again if the time is still ahead
                    t.notified = t.due.map_or(false, |d| d <= now);
                }
            }
            if let Some(id) = delete {
                st.items.retain(|t| t.id != id);
            }
            save_todos(&st.items);
        }
    }
}

impl eframe::App for EmelkaNotch {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Center horizontally on the monitor (in logical points, DPI-safe).
        if !self.positioned {
            if let Some(m) = ctx.input(|i| i.viewport().monitor_size) {
                let x = ((m.x - WIN_W) / 2.0).max(0.0);
                ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos2(x, 0.0)));
                self.positioned = true;
            }
        }

        self.sample_system();
        self.update_hover(ctx);
        let e = ease(self.t);

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| self.draw(ui, e));

        // Repaint scheduling: full speed while animating/open, slower when idle.
        let playing = self.shared.lock().unwrap().media.playing;
        if self.t > 0.0 || self.animating {
            ctx.request_repaint_after(Duration::from_millis(16));
        } else if playing {
            ctx.request_repaint_after(Duration::from_millis(33));
        } else {
            ctx.request_repaint_after(Duration::from_millis(60));
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }
}

fn main() -> eframe::Result<()> {
    let viewport = egui::ViewportBuilder::default()
        .with_title("EmelkaNotch")
        .with_inner_size([WIN_W, WIN_H])
        .with_position([0.0, 0.0])
        .with_decorations(false)
        .with_transparent(true)
        .with_always_on_top()
        .with_resizable(false)
        .with_taskbar(false)
        .with_active(false)
        .with_mouse_passthrough(true);

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "EmelkaNotch",
        options,
        Box::new(|cc| Ok(Box::new(EmelkaNotch::new(cc)))),
    )
}
