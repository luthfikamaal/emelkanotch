// =====================================================================
// Auto-update (GitHub Releases)
// =====================================================================

use crate::notification::{push_notice, NotifState};
use crate::theme::{REPO, UA, VERSION};
use eframe::egui;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

#[derive(Clone, Default, PartialEq)]
pub(crate) enum Upd {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Updating(String),
    Failed,
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

pub(crate) fn take_updated_marker() -> Option<String> {
    let m = std::env::current_exe().ok()?.with_extension("updated");
    let tag = std::fs::read_to_string(&m).ok()?;
    let _ = std::fs::remove_file(&m);
    Some(tag.trim().to_string())
}

pub(crate) fn run_update(
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

pub(crate) fn spawn_updater(
    ctx: egui::Context,
    upd: Arc<Mutex<Upd>>,
    notifs: Arc<Mutex<NotifState>>,
) {
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
