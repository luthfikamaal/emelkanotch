use eframe::egui;
use rusqlite::{types::ValueRef, Connection};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime};
use windows::core::PCWSTR;
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{
    CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetObjectW, BITMAP, BITMAPINFO,
    BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, HGDIOBJ,
};
use windows::Win32::System::Com::{CoInitializeEx, IBindCtx, COINIT_APARTMENTTHREADED};
use windows::Win32::UI::Shell::{
    IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK, SIIGBF_ICONONLY,
};

#[derive(Clone)]
pub(crate) struct Notif {
    pub app: String,
    pub app_id: String,
    pub icon: Option<Arc<egui::ColorImage>>,
    pub title: String,
    pub body: String,
    pub launch: Option<String>,
    pub at: Instant,
}

#[derive(Default)]
pub(crate) struct NotifState {
    pub items: VecDeque<Notif>,
    pub latest: Option<Notif>,
    pub error: Option<String>,
}

pub(crate) fn activate_notif(n: &Notif) {
    let target = if let Some(u) = &n.launch {
        u.clone()
    } else if !n.app_id.is_empty() && n.icon.is_some() {
        format!("shell:AppsFolder\\{}", n.app_id)
    } else {
        return;
    };
    let _ = std::process::Command::new("explorer.exe")
        .arg(target)
        .spawn();
}

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

fn xml_attr(tag: &str, name: &str) -> Option<String> {
    let mut from = 0;
    while let Some(i) = tag[from..].find(name) {
        let s = from + i;
        let before_ok = s == 0 || tag.as_bytes()[s - 1].is_ascii_whitespace();
        let after = &tag[s + name.len()..];
        let after_t = after.trim_start();
        if before_ok && after_t.starts_with('=') {
            let v = after_t[1..].trim_start();
            let q = v.chars().next()?;
            if q == '"' || q == '\'' {
                let end = v[1..].find(q)?;
                return Some(decode_entities(&v[1..1 + end]));
            }
        }
        from = s + name.len();
    }
    None
}

fn parse_toast_launch(xml: &str) -> Option<String> {
    let i = xml.find("<toast")?;
    let rest = &xml[i + 6..];
    let gt = rest.find('>')?;
    let tag = &rest[..gt];
    let launch = xml_attr(tag, "launch")?;
    let launch = launch.trim().to_string();
    if launch.is_empty() {
        return None;
    }
    let is_protocol =
        xml_attr(tag, "activationType").map_or(false, |t| t.eq_ignore_ascii_case("protocol"));
    let l = launch.to_lowercase();
    if is_protocol || l.starts_with("http://") || l.starts_with("https://") || l.starts_with("ms-")
    {
        Some(launch)
    } else {
        None
    }
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
            let launch = parse_toast_launch(&xml);
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
                launch,
                at: Instant::now(),
            });
        }
    }
    *last_id = Some(max_id);
    Ok(out)
}

pub(crate) fn spawn_notif_worker(ctx: egui::Context, state: Arc<Mutex<NotifState>>) {
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

pub(crate) fn push_notice(state: &Arc<Mutex<NotifState>>, title: &str, body: String) {
    let n = Notif {
        app: "EmelkaNotch".into(),
        app_id: String::new(),
        icon: None,
        title: title.into(),
        body,
        launch: None,
        at: Instant::now(),
    };
    let mut st = state.lock().unwrap();
    st.items.push_front(n.clone());
    st.latest = Some(n);
    st.items.truncate(30);
}
