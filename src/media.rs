use crate::theme::UA;
use eframe::egui;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession, GlobalSystemMediaTransportControlsSessionManager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus,
};

pub(crate) const LYRIC_LEAD_MS: i64 = 300;

#[derive(Clone, Default)]
pub(crate) struct Media {
    pub available: bool,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub app: String,
    pub playing: bool,
    pub pos_ms: u64,
    pub dur_ms: u64,
}

#[derive(Clone, Default)]
pub(crate) enum Lyrics {
    #[default]
    Idle,
    Loading,
    NotFound,
    Synced(Arc<Vec<(u64, String)>>),
}

#[derive(Default)]
pub(crate) struct Shared {
    pub media: Media,
    pub snapshot: Option<Instant>,
    pub lyrics: Lyrics,
    pub lyrics_key: String,
    pub cache: HashMap<String, Lyrics>,
    pub art: Option<Arc<egui::ColorImage>>,
    pub art_ver: u64,
}

pub(crate) enum Cmd {
    PlayPause,
    Next,
    Prev,
    Seek(u64),
}

fn read_media(s: &GlobalSystemMediaTransportControlsSession) -> windows::core::Result<Media> {
    let props = s.TryGetMediaPropertiesAsync()?.get()?;
    let title = props.Title()?.to_string();
    let artist = props.Artist()?.to_string();
    let album = props
        .AlbumTitle()
        .map(|h| h.to_string())
        .unwrap_or_default()
        .trim()
        .to_string();
    let app = s.SourceAppUserModelId()?.to_string();
    let playing = s.GetPlaybackInfo()?.PlaybackStatus()?
        == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing;

    let tl = s.GetTimelineProperties()?;
    let start = tl.StartTime()?.Duration;
    let end = tl.EndTime()?.Duration;
    let dur = (end - start).max(0);
    let mut pos = (tl.Position()?.Duration - start).max(0);

    if playing {
        let last = tl.LastUpdatedTime()?.UniversalTime;
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
        album,
        app,
        playing,
        pos_ms: (pos / 10_000) as u64,
        dur_ms: (dur / 10_000) as u64,
    })
}

fn read_art(s: &GlobalSystemMediaTransportControlsSession) -> Option<Arc<egui::ColorImage>> {
    use windows::Graphics::Imaging::{
        BitmapAlphaMode, BitmapDecoder, BitmapPixelFormat, BitmapTransform, ColorManagementMode,
        ExifOrientationMode,
    };
    let props = s.TryGetMediaPropertiesAsync().ok()?.get().ok()?;
    let thumb = props.Thumbnail().ok()?;
    let stream = thumb.OpenReadAsync().ok()?.get().ok()?;
    let decoder = BitmapDecoder::CreateAsync(&stream).ok()?.get().ok()?;
    let (sw, sh) = (decoder.PixelWidth().ok()?, decoder.PixelHeight().ok()?);
    if sw == 0 || sh == 0 {
        return None;
    }
    let scale = (160.0_f32 / sw.max(sh) as f32).min(1.0);
    let w = ((sw as f32 * scale).round() as u32).max(1);
    let h = ((sh as f32 * scale).round() as u32).max(1);

    let tf = BitmapTransform::new().ok()?;
    tf.SetScaledWidth(w).ok()?;
    tf.SetScaledHeight(h).ok()?;
    let data = decoder
        .GetPixelDataTransformedAsync(
            BitmapPixelFormat::Rgba8,
            BitmapAlphaMode::Straight,
            &tf,
            ExifOrientationMode::IgnoreExifOrientation,
            ColorManagementMode::DoNotColorManage,
        )
        .ok()?
        .get()
        .ok()?;
    let px = data.DetachPixelData().ok()?;
    let need = (w * h * 4) as usize;
    if px.len() < need {
        return None;
    }
    Some(Arc::new(egui::ColorImage::from_rgba_unmultiplied(
        [w as usize, h as usize],
        &px[..need],
    )))
}

fn art_hash(img: &egui::ColorImage) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    img.size.hash(&mut h);
    img.pixels.hash(&mut h);
    h.finish()
}

pub(crate) fn spawn_media_worker(
    ctx: egui::Context,
    shared: Arc<Mutex<Shared>>,
) -> mpsc::Sender<Cmd> {
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

        let mut art_key = String::new();
        let mut art_tries = 0u8;
        let mut art_last = Instant::now();
        let mut art_sig: u64 = 0;

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

            let media = match &session {
                Some(s) => match read_media(s) {
                    Ok(m) => m,
                    Err(_) => continue,
                },
                None => Media::default(),
            };
            let key = format!("{}\u{1}{}", media.artist, media.title);

            if media.available {
                if art_key != key {
                    art_key = key.clone();
                    art_tries = 0;
                    art_last = Instant::now();
                    art_sig = 0;
                    shared.lock().unwrap().art = None;
                }
                let wait_ms = [900u64, 1600, 2500][(art_tries as usize).min(2)];
                if art_tries < 3 && art_last.elapsed() >= Duration::from_millis(wait_ms) {
                    art_tries += 1;
                    art_last = Instant::now();
                    if let Some(img) = session.as_ref().and_then(read_art) {
                        let sig = art_hash(&img);
                        if sig != art_sig {
                            art_sig = sig;
                            let mut sh = shared.lock().unwrap();
                            sh.art = Some(img);
                            sh.art_ver += 1;
                        }
                    }
                }
            } else if !art_key.is_empty() {
                art_key.clear();
                shared.lock().unwrap().art = None;
            }

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

pub(crate) fn clean_title(s: &str) -> String {
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

pub(crate) fn strip_brackets(s: &str) -> String {
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

pub(crate) fn clean_artist(s: &str) -> String {
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

pub(crate) fn current_line(l: &Lyrics, pos_ms: u64) -> Option<&str> {
    if let Lyrics::Synced(lines) = l {
        let i = lines.partition_point(|(t, _)| *t <= pos_ms);
        if i > 0 {
            return Some(lines[i - 1].1.as_str());
        }
    }
    None
}

// =====================================================================
// Lyric timing offsets (per song + per app), stored in %APPDATA%
// =====================================================================

#[derive(Default, Clone)]
pub(crate) struct Offsets {
    pub tracks: HashMap<String, i64>,
    pub apps: HashMap<String, i64>,
}

fn offsets_path() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("APPDATA")?).join("EmelkaNotch");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir.join("lyric_offsets.json"))
}

pub(crate) fn load_offsets() -> Offsets {
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

pub(crate) fn save_offsets(o: &Offsets) {
    let Some(p) = offsets_path() else { return };
    let v = serde_json::json!({ "tracks": o.tracks, "apps": o.apps });
    let _ = std::fs::write(p, v.to_string());
}

/// Cleaned so "(Official Video)" etc. doesn't create a separate entry.
pub(crate) fn track_key(m: &Media) -> String {
    format!(
        "{}\u{1}{}",
        clean_artist(&m.artist).to_lowercase(),
        clean_title(&m.title).to_lowercase()
    )
}

pub(crate) fn app_key(m: &Media) -> String {
    app_label(&m.app).to_lowercase()
}

pub(crate) fn shift(pos_ms: u64, lead: i64) -> u64 {
    (pos_ms as i64 + lead).max(0) as u64
}

pub(crate) fn app_label(id: &str) -> String {
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

pub(crate) fn fmt_time(ms: u64) -> String {
    let s = ms / 1000;
    format!("{}:{:02}", s / 60, s % 60)
}
