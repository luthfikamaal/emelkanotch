// =====================================================================
// OSD (volume + brightness), shown in the pill
// =====================================================================

use eframe::egui;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum OsdKind {
    Volume,
    Brightness,
}

#[derive(Clone, Copy)]
pub(crate) struct OsdEvent {
    pub kind: OsdKind,
    pub value: f32, // 0..1
    pub muted: bool,
    pub at: Instant,
}

#[derive(Default)]
pub(crate) struct OsdState {
    pub latest: Option<OsdEvent>,
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
pub(crate) fn spawn_volume_worker(ctx: egui::Context, state: Arc<Mutex<OsdState>>) {
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
pub(crate) fn spawn_brightness_worker(ctx: egui::Context, state: Arc<Mutex<OsdState>>) {
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

use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{eConsole, eRender, IMMDeviceEnumerator, MMDeviceEnumerator};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
};
