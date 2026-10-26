// =====================================================================
// System info helpers (disks) + shared date formatting
// =====================================================================

use chrono::{Local, TimeZone};
use std::time::Duration;
use sysinfo::Disks;

pub(crate) struct DiskInfo {
    pub name: String,
    pub used_gb: f64,
    pub total_gb: f64,
}

pub(crate) fn read_disks() -> Vec<DiskInfo> {
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

pub(crate) fn fmt_gb(g: f64) -> String {
    if g >= 1000.0 {
        format!("{:.1} TB", g / 1024.0)
    } else {
        format!("{:.0} GB", g)
    }
}

pub(crate) fn fmt_due(ts: i64) -> String {
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

#[allow(unused)]
fn _keep_duration_import(_d: Duration) {}
