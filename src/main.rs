#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui::{
    self, pos2, vec2, Align2, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId,
    Painter, Pos2, Rect, RichText, Sense, Shape, Stroke, StrokeKind, Ui, UiBuilder,
};
use rusqlite::{types::ValueRef, Connection};
use std::collections::{HashMap, VecDeque};
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
use windows::Win32::System::Com::{CoInitializeEx, IBindCtx, COINIT_APARTMENTTHREADED};
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

const LYRIC_LEAD_MS: u64 = 300;
const NOTIF_SHOW: Duration = Duration::from_secs(5);
const UA: &str = "EmelkaNotch/0.3 (personal project)";
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

fn apply_update(url: &str, tag: &str) -> Result<(), Box<dyn std::error::Error>> {
    let exe = std::env::current_exe()?;
    let new = exe.with_extension("new");
    let old = exe.with_extension("old");

    let mut reader = ureq::get(url)
        .set("User-Agent", UA)
        .timeout(Duration::from_secs(120))
        .call()?
        .into_reader();
    let mut f = std::fs::File::create(&new)?;
    std::io::copy(&mut reader, &mut f)?;
    drop(f);

    let _ = std::fs::remove_file(&old);
    std::fs::rename(&exe, &old)?;
    if let Err(e) = std::fs::rename(&new, &exe) {
        let _ = std::fs::rename(&old, &exe); // rollback
        return Err(e.into());
    }
    // The new instance reads this on startup and shows "EmelkaNotch updated".
    let _ = std::fs::write(exe.with_extension("updated"), tag);
    std::process::Command::new(&exe).spawn()?;
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
) {
    {
        let mut s = upd.lock().unwrap();
        if matches!(*s, Upd::Checking | Upd::Updating(_)) {
            return;
        }
        *s = Upd::Checking;
    }
    ctx.request_repaint();

    match check_update() {
        Ok(Some((tag, url))) => {
            *upd.lock().unwrap() = Upd::Updating(tag.clone());
            push_notice(
                &notifs,
                "Updating EmelkaNotch",
                format!("Downloading {tag}…"),
            );
            ctx.request_repaint();
            if apply_update(&url, &tag).is_err() {
                *upd.lock().unwrap() = Upd::Failed;
                push_notice(
                    &notifs,
                    "Update failed",
                    "Couldn't install the update.".into(),
                );
            }
        }
        Ok(None) => {
            *upd.lock().unwrap() = if manual { Upd::UpToDate } else { Upd::Idle };
        }
        Err(()) => {
            *upd.lock().unwrap() = if manual { Upd::Failed } else { Upd::Idle };
        }
    }
    ctx.request_repaint();
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
            run_update(upd.clone(), notifs.clone(), ctx.clone(), false);
            thread::sleep(Duration::from_secs(6 * 3600));
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

fn read_disk() -> Option<DiskInfo> {
    let disks = Disks::new_with_refreshed_list();
    let d = disks
        .list()
        .iter()
        .find(|d| {
            d.mount_point()
                .to_string_lossy()
                .to_uppercase()
                .starts_with("C:")
        })
        .or_else(|| disks.list().first())?;
    let gb = 1_073_741_824.0;
    let total = d.total_space() as f64 / gb;
    let free = d.available_space() as f64 / gb;
    Some(DiskInfo {
        name: d
            .mount_point()
            .to_string_lossy()
            .trim_end_matches('\\')
            .to_string(),
        used_gb: (total - free).max(0.0),
        total_gb: total,
    })
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

fn donut(p: &Painter, c: Pos2, r: f32, frac: f32, color: Color32) {
    p.circle_stroke(c, r, Stroke::new(7.0, TRACK));
    let n = 64;
    let start = -std::f32::consts::FRAC_PI_2;
    let sweep = std::f32::consts::TAU * frac.clamp(0.0, 1.0);
    let pts: Vec<Pos2> = (0..=n)
        .map(|i| {
            let a = start + sweep * i as f32 / n as f32;
            c + vec2(a.cos(), a.sin()) * r
        })
        .collect();
    if frac > 0.0 {
        p.add(Shape::line(pts, Stroke::new(7.0, color)));
    }
}

// =====================================================================
// Helpers
// =====================================================================

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

fn shape_rect(pw: f32, ph: f32, e: f32) -> Rect {
    let w = lerp(pw, PANEL_W, e);
    let h = lerp(ph, PANEL_H, e);
    Rect::from_min_size(pos2((WIN_W - w) / 2.0, TOP), vec2(w, h))
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

fn tab(ui: &mut Ui, label: &str, selected: bool) -> bool {
    let color = if selected { TEXT } else { MUTED };
    let galley = ui
        .painter()
        .layout_no_wrap(label.to_owned(), FontId::proportional(12.0), color);
    let (rect, resp) = ui.allocate_exact_size(galley.size() + vec2(22.0, 10.0), Sense::click());
    let bg = if selected {
        Color32::from_rgb(40, 42, 52)
    } else if resp.hovered() {
        Color32::from_rgb(24, 25, 31)
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, CornerRadius::same(8), bg);
    ui.painter()
        .galley(rect.center() - galley.size() / 2.0, galley, color);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
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

// =====================================================================
// App
// =====================================================================

#[derive(Clone, Copy, PartialEq)]
enum Section {
    Media,
    Alerts,
    System,
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
    disk: Option<DiskInfo>,
    last_disk: Instant,
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

        let mut sys = System::new();
        sys.refresh_cpu_all();
        sys.refresh_memory();

        Self {
            shared,
            notifs,
            tx,
            upd,
            section: Section::Media,
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
            disk: read_disk(),
            last_disk: Instant::now(),
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
            self.disk = read_disk();
            self.last_disk = Instant::now();
        }
    }

    fn update_hover(&mut self, ctx: &egui::Context) {
        let shape = shape_rect(self.pill_w, self.pill_h, ease(self.t));
        let hit = Rect::from_min_max(
            pos2(shape.left() - 4.0, 0.0),
            pos2(shape.right() + 4.0, shape.bottom() + 4.0),
        );
        let inside = cursor_local(ctx).map_or(false, |c| hit.contains(c));

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

    /// Decide what the pill shows (notification > lyric > title > idle) and measure it.
    fn refresh_view(
        &mut self,
        p: &Painter,
        media: &Media,
        lyrics: &Lyrics,
        pos_ms: u64,
        notif: Option<&Notif>,
    ) {
        let lyric_line = if media.playing {
            current_line(lyrics, pos_ms + LYRIC_LEAD_MS)
        } else {
            None
        };

        let (kind, main, sub, app): (u8, String, Option<String>, Option<String>) =
            if let Some(n) = notif {
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
                (0, "EmelkaNotch".to_string(), None, None)
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

        self.refresh_view(&painter, &media, &lyrics, pos_ms, notif.as_ref());

        // Smoothly follow the target pill size.
        let dt = ui.input(|i| i.stable_dt).min(0.05);
        let k = 1.0 - (-dt * 16.0).exp();
        self.pill_w += (self.view.w - self.pill_w) * k;
        self.pill_h += (self.view.h - self.pill_h) * k;
        self.animating = notif.is_some()
            || (self.view.w - self.pill_w).abs() > 0.5
            || (self.view.h - self.pill_h).abs() > 0.5;

        let rect = shape_rect(self.pill_w, self.pill_h, e);
        let radius = CornerRadius::same(lerp((self.pill_h / 2.0).min(22.0), 26.0, e).round() as u8);

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
            } else {
                let time = ui.input(|i| i.time) as f32;
                let eq_color = if media.available { ACCENT } else { DIM };
                for i in 0..3 {
                    let h = if media.playing {
                        4.0 + 9.0 * ((time * 7.0 + i as f32 * 1.7).sin() * 0.5 + 0.5)
                    } else {
                        4.0
                    };
                    let r = Rect::from_center_size(
                        pos2(rect.left() + 20.0 + i as f32 * 5.0, c.y),
                        vec2(3.0, h),
                    );
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
                pos2((WIN_W - PANEL_W) / 2.0 + PAD, TOP + PAD),
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
            ] {
                if tab(ui, label, self.section == s) {
                    self.section = s;
                }
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.update_button(ui);
            });
        });
        ui.add_space(10.0);

        match self.section {
            Section::Media => self.media_section(ui, media, pos_ms, lyrics),
            Section::Alerts => self.alerts_section(ui),
            Section::System => self.system_section(ui),
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

    fn alerts_section(&mut self, ui: &mut Ui) {
        let (items, err) = {
            let n = self.notifs.lock().unwrap();
            (
                n.items.iter().take(3).cloned().collect::<Vec<_>>(),
                n.error.clone(),
            )
        };

        if items.is_empty() {
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

        ui.spacing_mut().item_spacing.y = 2.0;
        for n in &items {
            let (rect, _) =
                ui.allocate_exact_size(vec2(ui.available_width(), 40.0), Sense::hover());
            let tex = self.icon_tex(ui.ctx(), &n.app_id, &n.icon);
            let p = ui.painter_at(rect);
            app_icon(
                &p,
                pos2(rect.left() + 14.0, rect.center().y),
                12.0,
                tex.as_ref(),
                &n.app,
                1.0,
            );

            let max = rect.width() - 36.0 - 48.0;
            let title_src = if n.title.is_empty() { &n.app } else { &n.title };
            let (title, _) = fit(&p, &one_line(title_src), semibold(12.0), max);
            let (body, _) = fit(&p, &one_line(&n.body), FontId::proportional(11.0), max);

            p.text(
                pos2(rect.left() + 36.0, rect.top() + 12.0),
                Align2::LEFT_CENTER,
                title,
                semibold(12.0),
                TEXT,
            );
            p.text(
                pos2(rect.left() + 36.0, rect.top() + 28.0),
                Align2::LEFT_CENTER,
                body,
                FontId::proportional(11.0),
                MUTED,
            );
            p.text(
                pos2(rect.right(), rect.top() + 12.0),
                Align2::RIGHT_CENTER,
                ago(n.at),
                FontId::proportional(10.0),
                DIM,
            );
        }
    }

    fn media_section(&self, ui: &mut Ui, media: &Media, pos_ms: u64, lyrics: &Lyrics) {
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
        Self::draw_lyrics(ui, area, lyrics, pos_ms);
    }

    fn draw_lyrics(ui: &Ui, area: Rect, lyrics: &Lyrics, pos_ms: u64) {
        let painter = ui.painter_at(area);
        let center = area.center();
        match lyrics {
            Lyrics::Synced(lines) => {
                let pos = pos_ms + LYRIC_LEAD_MS;
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

            // Storage
            let (r, p) = card(ui, w, h);
            match &self.disk {
                Some(d) => {
                    let frac = (d.used_gb / d.total_gb.max(0.001)) as f32;
                    card_header(&p, r, "Storage", &d.name);
                    let c = pos2(r.left() + 46.0, r.top() + 83.0);
                    let col = if frac > 0.9 { WARN } else { GREEN };
                    donut(&p, c, 29.0, frac, col);
                    p.text(
                        c,
                        Align2::CENTER_CENTER,
                        format!("{:.0}%", frac * 100.0),
                        semibold(13.0),
                        TEXT,
                    );

                    let x = r.left() + 90.0;
                    let small_f = FontId::proportional(10.0);
                    p.text(
                        pos2(x, c.y - 22.0),
                        Align2::LEFT_CENTER,
                        "Used",
                        small_f.clone(),
                        DIM,
                    );
                    p.text(
                        pos2(x, c.y - 9.0),
                        Align2::LEFT_CENTER,
                        fmt_gb(d.used_gb),
                        semibold(12.0),
                        TEXT,
                    );
                    p.text(
                        pos2(x, c.y + 9.0),
                        Align2::LEFT_CENTER,
                        "Free",
                        small_f,
                        DIM,
                    );
                    p.text(
                        pos2(x, c.y + 22.0),
                        Align2::LEFT_CENTER,
                        fmt_gb(d.total_gb - d.used_gb),
                        semibold(12.0),
                        TEXT,
                    );
                }
                None => {
                    card_header(&p, r, "Storage", "");
                    p.text(
                        r.center(),
                        Align2::CENTER_CENTER,
                        "No disk info",
                        FontId::proportional(11.0),
                        DIM,
                    );
                }
            }
        });
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
