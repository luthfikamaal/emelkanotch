// =====================================================================
// Screenshot Tool (Window / Region / Fullscreen)
// =====================================================================

use crate::notification::{push_notice, NotifState};
use chrono::Local;
use eframe::egui;
use std::fs::{self, File};
use std::io::BufWriter;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Instant;

use windows::core::{HSTRING, PCWSTR};
use windows::Graphics::Imaging::{
    BitmapAlphaMode, BitmapDecoder, BitmapPixelFormat, BitmapTransform, ColorManagementMode,
    ExifOrientationMode,
};
use windows::Storage::{FileAccessMode, StorageFile};
use windows::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
use windows::Win32::Graphics::Dwm::{
    DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS,
};
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDC, GetDIBits,
    GetDeviceCaps, ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DESKTOPHORZRES,
    DESKTOPVERTRES, DIB_RGB_COLORS, SRCCOPY,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, MOD_ALT, MOD_NOREPEAT, MOD_SHIFT,
};
use windows::Win32::UI::Shell::Common::COMDLG_FILTERSPEC;
use windows::Win32::UI::Shell::{
    FileOpenDialog, IFileOpenDialog, FOS_FILEMUSTEXIST, FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST,
    SIGDN_FILESYSPATH,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetMessageW, GetSystemMetrics, GetWindowLongW, GetWindowRect, GetWindowTextW,
    IsIconic, IsWindowVisible, GWL_EXSTYLE, MSG, SM_CXSCREEN, SM_CYSCREEN, WM_HOTKEY,
    WS_EX_TOOLWINDOW,
};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum CaptureMode {
    Window,
    Region,
    Screen,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum BgMode {
    White,
    Image,
    Transparent,
}

#[derive(Clone)]
pub(crate) struct WindowCandidate {
    pub rect: [i32; 4], // [left, top, right, bottom] in physical screen pixels
    pub title: String,
}

#[derive(Clone)]
pub(crate) struct RawDesktop {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<Vec<u8>>,
    pub windows: Vec<WindowCandidate>,
}

#[derive(Clone)]
pub(crate) struct CaptureResult {
    pub preview: Arc<egui::ColorImage>,
    pub rgba: Arc<Vec<u8>>,
    pub width: u32,
    pub height: u32,
    pub saved_path: Option<PathBuf>,
    pub mode: CaptureMode,
    pub at: Instant,
    pub ver: u64,
}

#[derive(Clone, Default)]
pub(crate) enum OverlayPhase {
    #[default]
    Idle,
    /// Waiting briefly while the Notch hides itself so it won't appear in the screenshot.
    Preparing { mode: CaptureMode, started: Instant },
    /// Interactive fullscreen selector for Window or Region mode.
    Active {
        mode: CaptureMode,
        desktop: RawDesktop,
        drag_start: Option<egui::Pos2>,
        drag_cur: Option<egui::Pos2>,
    },
}

pub(crate) struct ScreenshotState {
    pub bg_mode: BgMode,
    pub bg_image_path: Option<String>,
    pub bg_image_cache: Option<(String, u32, u32, Arc<Vec<u8>>)>,
    pub shadow_enabled: bool,
    pub latest: Option<CaptureResult>,
    pub busy: bool,
    pub picking_bg: bool,
    pub status_msg: Option<(String, Instant)>,
    pub hotkey_triggered: Option<CaptureMode>,
    pub next_ver: u64,
}

impl Default for ScreenshotState {
    fn default() -> Self {
        Self {
            bg_mode: BgMode::White,
            bg_image_path: None,
            bg_image_cache: None,
            shadow_enabled: true,
            latest: None,
            busy: false,
            picking_bg: false,
            status_msg: None,
            hotkey_triggered: None,
            next_ver: 1,
        }
    }
}

fn config_path() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("APPDATA")?).join("EmelkaNotch");
    let _ = fs::create_dir_all(&dir);
    Some(dir.join("screenshot.json"))
}

pub(crate) fn screenshots_dir() -> PathBuf {
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        let dir = PathBuf::from(profile).join("Pictures").join("Screenshots");
        if fs::create_dir_all(&dir).is_ok() {
            return dir;
        }
    }
    let fallback = PathBuf::from("Screenshots");
    let _ = fs::create_dir_all(&fallback);
    fallback
}

pub(crate) fn load_screenshot_state() -> ScreenshotState {
    let mut st = ScreenshotState::default();
    let Some(p) = config_path() else {
        return st;
    };
    let Ok(raw) = fs::read_to_string(p) else {
        return st;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return st;
    };

    if let Some(m) = v.get("bg_mode").and_then(|x| x.as_str()) {
        st.bg_mode = match m {
            "image" => BgMode::Image,
            "transparent" => BgMode::Transparent,
            _ => BgMode::White,
        };
    }
    if let Some(sh) = v.get("shadow").and_then(|x| x.as_bool()) {
        st.shadow_enabled = sh;
    }
    if let Some(path) = v
        .get("bg_image_path")
        .and_then(|x| x.as_str())
        .filter(|s| !s.is_empty())
    {
        let p_str = path.to_string();
        if Path::new(&p_str).exists() {
            st.bg_image_path = Some(p_str.clone());
            if let Some((w, h, rgba)) = decode_image_file(&p_str) {
                st.bg_image_cache = Some((p_str, w, h, Arc::new(rgba)));
            }
        }
    }
    if st.bg_mode == BgMode::Image && st.bg_image_path.is_none() {
        st.bg_mode = BgMode::White;
    }
    st
}

pub(crate) fn save_screenshot_settings(st: &ScreenshotState) {
    let Some(p) = config_path() else { return };
    let mode_str = match st.bg_mode {
        BgMode::White => "white",
        BgMode::Image => "image",
        BgMode::Transparent => "transparent",
    };
    let json = serde_json::json!({
        "bg_mode": mode_str,
        "bg_image_path": st.bg_image_path.as_deref().unwrap_or(""),
        "shadow": st.shadow_enabled,
    });
    let _ = fs::write(p, json.to_string());
}

/// Decodes an image file (PNG, JPG, WEBP, BMP) via Windows Imaging Component (WIC / WinRT).
pub(crate) fn decode_image_file(path: &str) -> Option<(u32, u32, Vec<u8>)> {
    let abs = fs::canonicalize(path).ok()?;
    // strip \\?\ extended-length prefix if present, as StorageFile prefers standard Win32 paths
    let clean = abs
        .to_string_lossy()
        .strip_prefix(r"\\?\")
        .unwrap_or(&abs.to_string_lossy())
        .to_string();
    let hpath = HSTRING::from(clean);
    let file = StorageFile::GetFileFromPathAsync(&hpath).ok()?.get().ok()?;
    let stream = file.OpenAsync(FileAccessMode::Read).ok()?.get().ok()?;
    let decoder = BitmapDecoder::CreateAsync(&stream).ok()?.get().ok()?;
    let (sw, sh) = (decoder.PixelWidth().ok()?, decoder.PixelHeight().ok()?);
    if sw == 0 || sh == 0 {
        return None;
    }
    let scale = (2560.0_f32 / sw.max(sh) as f32).min(1.0);
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
            ExifOrientationMode::RespectExifOrientation,
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
    Some((w, h, px[..need].to_vec()))
}

/// Opens a native Windows File Open dialog to select a custom background image.
pub(crate) fn spawn_bg_image_picker(ctx: egui::Context, state: Arc<Mutex<ScreenshotState>>) {
    {
        let mut st = state.lock().unwrap();
        if st.picking_bg {
            return;
        }
        st.picking_bg = true;
    }
    thread::spawn(move || {
        let picked = unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            (|| -> Option<String> {
                let dlg: IFileOpenDialog =
                    CoCreateInstance(&FileOpenDialog, None, CLSCTX_ALL).ok()?;
                let name: Vec<u16> = "Image Files (*.png;*.jpg;*.jpeg;*.webp;*.bmp)\0"
                    .encode_utf16()
                    .collect();
                let spec: Vec<u16> = "*.png;*.jpg;*.jpeg;*.webp;*.bmp\0".encode_utf16().collect();
                let filters = [COMDLG_FILTERSPEC {
                    pszName: PCWSTR(name.as_ptr()),
                    pszSpec: PCWSTR(spec.as_ptr()),
                }];
                let _ = dlg.SetFileTypes(&filters);
                let opts = dlg.GetOptions().ok()?;
                let _ = dlg
                    .SetOptions(opts | FOS_FORCEFILESYSTEM | FOS_FILEMUSTEXIST | FOS_PATHMUSTEXIST);
                dlg.Show(None).ok()?;
                let item = dlg.GetResult().ok()?;
                let pwstr = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
                let s = pwstr.to_string().ok();
                windows::Win32::System::Com::CoTaskMemFree(Some(pwstr.0 as *const _));
                s
            })()
        };

        if let Some(path) = picked {
            if let Some((w, h, rgba)) = decode_image_file(&path) {
                let mut st = state.lock().unwrap();
                st.bg_image_path = Some(path.clone());
                st.bg_image_cache = Some((path, w, h, Arc::new(rgba)));
                st.bg_mode = BgMode::Image;
                st.status_msg = Some(("Background image updated".to_string(), Instant::now()));
                st.picking_bg = false;
                save_screenshot_settings(&st);
            } else {
                let mut st = state.lock().unwrap();
                st.status_msg = Some(("Failed to decode image".to_string(), Instant::now()));
                st.picking_bg = false;
            }
        } else {
            state.lock().unwrap().picking_bg = false;
        }
        ctx.request_repaint();
    });
}

/// Spawns a global hotkey listener (`Alt+Shift+S`) to trigger screenshot capture from anywhere.
pub(crate) fn spawn_hotkey_worker(ctx: egui::Context, state: Arc<Mutex<ScreenshotState>>) {
    thread::spawn(move || unsafe {
        if RegisterHotKey(None, 101, MOD_ALT | MOD_SHIFT | MOD_NOREPEAT, b'S' as u32).is_err() {
            return;
        }
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            if msg.message == WM_HOTKEY {
                let mut st = state.lock().unwrap();
                if !st.busy {
                    st.hotkey_triggered = Some(CaptureMode::Region);
                    ctx.request_repaint();
                }
            }
        }
    });
}

// =====================================================================
// Screen & Window Enumeration / Capture
// =====================================================================

struct EnumCtx {
    screen_w: i32,
    screen_h: i32,
    list: Vec<WindowCandidate>,
}

unsafe extern "system" fn enum_win_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = &mut *(lparam.0 as *mut EnumCtx);
    if !IsWindowVisible(hwnd).as_bool() || IsIconic(hwnd).as_bool() {
        return BOOL(1);
    }

    let mut cloaked: u32 = 0;
    let _ = DwmGetWindowAttribute(
        hwnd,
        DWMWA_CLOAKED,
        &mut cloaked as *mut _ as *mut _,
        std::mem::size_of::<u32>() as u32,
    );
    if cloaked != 0 {
        return BOOL(1);
    }

    let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
    if (ex_style & WS_EX_TOOLWINDOW.0) != 0 {
        return BOOL(1);
    }

    let mut title_buf = [0u16; 256];
    let len = GetWindowTextW(hwnd, &mut title_buf);
    let title = if len > 0 {
        String::from_utf16_lossy(&title_buf[..len as usize])
    } else {
        String::new()
    };

    if title == "EmelkaNotch"
        || title == "Program Manager"
        || title == "Windows Input Experience"
        || title == "Microsoft Text Input Application"
    {
        return BOOL(1);
    }

    let mut r = RECT::default();
    if DwmGetWindowAttribute(
        hwnd,
        DWMWA_EXTENDED_FRAME_BOUNDS,
        &mut r as *mut _ as *mut _,
        std::mem::size_of::<RECT>() as u32,
    )
    .is_err()
        && GetWindowRect(hwnd, &mut r).is_err()
    {
        return BOOL(1);
    }

    let left = r.left.clamp(0, ctx.screen_w);
    let top = r.top.clamp(0, ctx.screen_h);
    let right = r.right.clamp(0, ctx.screen_w);
    let bottom = r.bottom.clamp(0, ctx.screen_h);

    let w = right - left;
    let h = bottom - top;
    if w >= 48 && h >= 48 {
        ctx.list.push(WindowCandidate {
            rect: [left, top, right, bottom],
            title: if title.trim().is_empty() {
                "Application Window".to_string()
            } else {
                title
            },
        });
    }
    BOOL(1)
}

/// Captures the entire primary screen in physical pixels and snapshots all visible window bounds.
pub(crate) fn capture_raw_desktop() -> Option<RawDesktop> {
    unsafe {
        let hdc_screen = GetDC(None);
        if hdc_screen.is_invalid() {
            return None;
        }
        let mut w = GetDeviceCaps(hdc_screen, DESKTOPHORZRES);
        let mut h = GetDeviceCaps(hdc_screen, DESKTOPVERTRES);
        if w <= 0 || h <= 0 {
            w = GetSystemMetrics(SM_CXSCREEN);
            h = GetSystemMetrics(SM_CYSCREEN);
        }
        if w <= 0 || h <= 0 {
            let _ = ReleaseDC(None, hdc_screen);
            return None;
        }

        let hdc_mem = CreateCompatibleDC(hdc_screen);
        let hbmp = CreateCompatibleBitmap(hdc_screen, w, h);
        let old = SelectObject(hdc_mem, hbmp);
        let ok = BitBlt(hdc_mem, 0, 0, w, h, hdc_screen, 0, 0, SRCCOPY).is_ok();
        let _ = SelectObject(hdc_mem, old);

        let mut rgba = vec![0u8; (w as usize) * (h as usize) * 4];
        let mut lines = 0;
        if ok {
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
            lines = GetDIBits(
                hdc_mem,
                hbmp,
                0,
                h as u32,
                Some(rgba.as_mut_ptr() as *mut _),
                &mut bi,
                DIB_RGB_COLORS,
            );
        }

        let _ = DeleteObject(hbmp);
        let _ = DeleteDC(hdc_mem);
        let _ = ReleaseDC(None, hdc_screen);

        if lines == 0 {
            return None;
        }

        // Convert BGRA -> RGBA with opaque alpha
        for px in rgba.chunks_exact_mut(4) {
            px.swap(0, 2);
            px[3] = 255;
        }

        let mut enum_ctx = EnumCtx {
            screen_w: w,
            screen_h: h,
            list: Vec::new(),
        };
        let _ = EnumWindows(
            Some(enum_win_proc),
            LPARAM(&mut enum_ctx as *mut EnumCtx as isize),
        );

        Some(RawDesktop {
            width: w as u32,
            height: h as u32,
            rgba: Arc::new(rgba),
            windows: enum_ctx.list,
        })
    }
}

/// Crops a sub-rectangle `[left, top, right, bottom]` from a `RawDesktop`.
pub(crate) fn crop_desktop(desktop: &RawDesktop, rect: [i32; 4]) -> Option<(u32, u32, Vec<u8>)> {
    let x0 = rect[0].clamp(0, desktop.width as i32) as u32;
    let y0 = rect[1].clamp(0, desktop.height as i32) as u32;
    let x1 = rect[2].clamp(0, desktop.width as i32) as u32;
    let y1 = rect[3].clamp(0, desktop.height as i32) as u32;
    if x1 <= x0 + 4 || y1 <= y0 + 4 {
        return None;
    }
    let cw = x1 - x0;
    let ch = y1 - y0;
    let mut out = vec![0u8; (cw * ch * 4) as usize];
    let src_stride = (desktop.width * 4) as usize;
    let dst_stride = (cw * 4) as usize;
    for row in 0..ch as usize {
        let sy = (y0 as usize + row) * src_stride + (x0 as usize * 4);
        let dy = row * dst_stride;
        out[dy..dy + dst_stride].copy_from_slice(&desktop.rgba[sy..sy + dst_stride]);
    }
    Some((cw, ch, out))
}

// =====================================================================
// Post-Processing (Rounded Corners + SDF Gaussian Shadow)
// =====================================================================

/// Fast, accurate approximation of complementary error function erfc(x) in [0, 2].
#[inline]
fn fast_erfc(x: f32) -> f32 {
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.5 * z);
    let r = t
        * (-z * z - 1.2655122
            + t * (1.0000237
                + t * (0.37409196
                    + t * (0.09678418
                        + t * (-0.18628806
                            + t * (0.27886808
                                + t * (-1.13520398
                                    + t * (1.4885159 + t * (-0.82215223 + t * 0.17087278)))))))))
            .exp();
    if x >= 0.0 {
        r
    } else {
        2.0 - r
    }
}

/// Signed distance from point `(px, py)` to a rounded rectangle centered at `(cx, cy)`
/// with half-size `(hx, hy)` and corner radius `r`.
#[inline]
fn rounded_box_sdf(px: f32, py: f32, cx: f32, cy: f32, hx: f32, hy: f32, r: f32) -> f32 {
    let qx = (px - cx).abs() - (hx - r);
    let qy = (py - cy).abs() - (hy - r);
    let out_d = (qx.max(0.0).powi(2) + qy.max(0.0).powi(2)).sqrt();
    let in_d = qx.max(qy).min(0.0);
    out_d + in_d - r
}

/// Samples an RGBA image at normalized UV `(u, v)` with bilinear interpolation.
#[inline]
fn sample_bilinear(rgba: &[u8], w: u32, h: u32, u: f32, v: f32) -> [u8; 4] {
    let fx = (u.clamp(0.0, 1.0) * (w.saturating_sub(1)) as f32).max(0.0);
    let fy = (v.clamp(0.0, 1.0) * (h.saturating_sub(1)) as f32).max(0.0);
    let x0 = (fx.floor() as u32).min(w - 1);
    let y0 = (fy.floor() as u32).min(h - 1);
    let x1 = (x0 + 1).min(w - 1);
    let y1 = (y0 + 1).min(h - 1);
    let tx = fx - x0 as f32;
    let ty = fy - y0 as f32;

    let idx = |x: u32, y: u32| ((y * w + x) * 4) as usize;
    let p00 = idx(x0, y0);
    let p10 = idx(x1, y0);
    let p01 = idx(x0, y1);
    let p11 = idx(x1, y1);

    let mut out = [0u8; 4];
    for c in 0..3 {
        let top = rgba[p00 + c] as f32 * (1.0 - tx) + rgba[p10 + c] as f32 * tx;
        let bot = rgba[p01 + c] as f32 * (1.0 - tx) + rgba[p11 + c] as f32 * tx;
        out[c] = (top * (1.0 - ty) + bot * ty).round().clamp(0.0, 255.0) as u8;
    }
    out[3] = 255;
    out
}

/// Renders the final styled screenshot:
/// - Background (Solid White default, Center-Cropped Custom Image, or Transparent)
/// - Dual-layer Gaussian Drop Shadow (ambient + downward directional shadow)
/// - Anti-aliased rounded corners (12px scaled) with subtle inner/outer border
pub(crate) fn render_screenshot(
    src_rgba: &[u8],
    src_w: u32,
    src_h: u32,
    bg_mode: BgMode,
    bg_image: Option<(u32, u32, &[u8])>,
    shadow_enabled: bool,
) -> (u32, u32, Vec<u8>) {
    let scale = ((src_w.min(src_h) as f32) / 550.0).clamp(0.75, 2.0);
    let radius = (12.0 * scale).round().min((src_w.min(src_h) as f32) * 0.25);

    let (pad_x, pad_top, pad_bot) = if shadow_enabled || bg_mode != BgMode::Transparent {
        (
            (54.0 * scale).round() as u32,
            (44.0 * scale).round() as u32,
            (66.0 * scale).round() as u32,
        )
    } else {
        (0, 0, 0)
    };

    let out_w = src_w + pad_x * 2;
    let out_h = src_h + pad_top + pad_bot;
    let mut out = vec![0u8; (out_w * out_h * 4) as usize];

    // Precompute cover crop parameters if using custom background image
    let cover_params = bg_image.map(|(bw, bh, _)| {
        let out_aspect = out_w as f32 / out_h.max(1) as f32;
        let bg_aspect = bw as f32 / bh.max(1) as f32;
        if out_aspect > bg_aspect {
            let vis_h = bg_aspect / out_aspect;
            (0.0f32, (1.0 - vis_h) * 0.5, 1.0f32, vis_h)
        } else {
            let vis_w = out_aspect / bg_aspect;
            ((1.0 - vis_w) * 0.5, 0.0f32, vis_w, 1.0f32)
        }
    });

    let hx = src_w as f32 * 0.5;
    let hy = src_h as f32 * 0.5;
    let cx = pad_x as f32 + hx;
    let cy = pad_top as f32 + hy;

    let key_off_y = 15.0 * scale;
    let key_sigma = 19.0 * scale * 1.4142135;
    let amb_off_y = 2.0 * scale;
    let amb_sigma = 5.5 * scale * 1.4142135;

    for y in 0..out_h {
        let py = y as f32 + 0.5;
        let v_norm = y as f32 / out_h.saturating_sub(1).max(1) as f32;

        for x in 0..out_w {
            let px = x as f32 + 0.5;
            let idx = ((y * out_w + x) * 4) as usize;

            // 1. Base background pixel
            let mut base = match bg_mode {
                BgMode::White => [255u8, 255, 255, 255],
                BgMode::Image => {
                    if let (Some((bw, bh, bpx)), Some((u0, v0, uw, vh))) = (bg_image, cover_params)
                    {
                        let u_norm = x as f32 / out_w.saturating_sub(1).max(1) as f32;
                        sample_bilinear(bpx, bw, bh, u0 + u_norm * uw, v0 + v_norm * vh)
                    } else {
                        [255u8, 255, 255, 255]
                    }
                }
                BgMode::Transparent => [0u8, 0, 0, 0],
            };

            let sdf = rounded_box_sdf(px, py, cx, cy, hx, hy, radius);

            // 2. Dual-Layer Drop Shadow (evaluated outside and near the border)
            if shadow_enabled && sdf > -1.5 {
                let sdf_key = rounded_box_sdf(px, py - key_off_y, cx, cy, hx, hy, radius);
                let sdf_amb = rounded_box_sdf(px, py - amb_off_y, cx, cy, hx, hy, radius);

                let a_key = 0.165 * fast_erfc(sdf_key / key_sigma);
                let a_amb = 0.075 * fast_erfc(sdf_amb / amb_sigma);
                let sh_a = (a_key + a_amb * (1.0 - a_key)).clamp(0.0, 1.0);

                if base[3] == 255 {
                    let inv = 1.0 - sh_a;
                    base[0] = (base[0] as f32 * inv).round() as u8;
                    base[1] = (base[1] as f32 * inv).round() as u8;
                    base[2] = (base[2] as f32 * inv).round() as u8;
                } else {
                    base[3] = (sh_a * 255.0).round() as u8;
                }
            }

            // 3. Screenshot Pixel with Anti-Aliased Rounded Corners + Subtle Border
            if sdf < 0.75 {
                let cov = (0.5 - sdf).clamp(0.0, 1.0);
                let sx = (x.saturating_sub(pad_x)).min(src_w - 1);
                let sy = (y.saturating_sub(pad_top)).min(src_h - 1);
                let s_idx = ((sy * src_w + sx) * 4) as usize;

                let mut sr = src_rgba[s_idx] as f32;
                let mut sg = src_rgba[s_idx + 1] as f32;
                let mut sb = src_rgba[s_idx + 2] as f32;

                // Subtle inner highlight & outer border definition
                if sdf > -1.4 {
                    let rim = (1.0 - (sdf + 0.5).abs() * 1.1).clamp(0.0, 1.0) * 0.08;
                    sr = sr * (1.0 - rim) + 120.0 * rim;
                    sg = sg * (1.0 - rim) + 124.0 * rim;
                    sb = sb * (1.0 - rim) + 132.0 * rim;
                }

                if cov >= 0.999 {
                    out[idx] = sr.round() as u8;
                    out[idx + 1] = sg.round() as u8;
                    out[idx + 2] = sb.round() as u8;
                    out[idx + 3] = 255;
                } else {
                    let bg_a = base[3] as f32 / 255.0;
                    let out_a = cov + bg_a * (1.0 - cov);
                    if out_a > 0.001 {
                        out[idx] = ((sr * cov + base[0] as f32 * bg_a * (1.0 - cov)) / out_a)
                            .round()
                            .clamp(0.0, 255.0) as u8;
                        out[idx + 1] = ((sg * cov + base[1] as f32 * bg_a * (1.0 - cov)) / out_a)
                            .round()
                            .clamp(0.0, 255.0) as u8;
                        out[idx + 2] = ((sb * cov + base[2] as f32 * bg_a * (1.0 - cov)) / out_a)
                            .round()
                            .clamp(0.0, 255.0) as u8;
                        out[idx + 3] = (out_a * 255.0).round().clamp(0.0, 255.0) as u8;
                    }
                }
            } else {
                out[idx..idx + 4].copy_from_slice(&base);
            }
        }
    }

    (out_w, out_h, out)
}

fn downscale_preview(rgba: &[u8], w: u32, h: u32, max_dim: u32) -> egui::ColorImage {
    let scale = (max_dim as f32 / w.max(h).max(1) as f32).min(1.0);
    let pw = ((w as f32 * scale).round() as u32).max(1);
    let ph = ((h as f32 * scale).round() as u32).max(1);
    let mut buf = vec![0u8; (pw * ph * 4) as usize];
    for y in 0..ph {
        let v = y as f32 / ph.saturating_sub(1).max(1) as f32;
        for x in 0..pw {
            let u = x as f32 / pw.saturating_sub(1).max(1) as f32;
            let px = sample_bilinear(rgba, w, h, u, v);
            let idx = ((y * pw + x) * 4) as usize;
            // preserve alpha for transparent mode
            let sx = ((u * (w - 1) as f32).round() as u32).min(w - 1);
            let sy = ((v * (h - 1) as f32).round() as u32).min(h - 1);
            let a = rgba[((sy * w + sx) * 4 + 3) as usize];
            buf[idx] = px[0];
            buf[idx + 1] = px[1];
            buf[idx + 2] = px[2];
            buf[idx + 3] = a;
        }
    }
    egui::ColorImage::from_rgba_unmultiplied([pw as usize, ph as usize], &buf)
}

pub(crate) fn copy_rgba_to_clipboard(w: u32, h: u32, rgba: &[u8]) -> bool {
    let Ok(mut cb) = arboard::Clipboard::new() else {
        return false;
    };
    cb.set_image(arboard::ImageData {
        width: w as usize,
        height: h as usize,
        bytes: std::borrow::Cow::Borrowed(rgba),
    })
    .is_ok()
}

fn save_rgba_as_png(w: u32, h: u32, rgba: &[u8]) -> Option<PathBuf> {
    let dir = screenshots_dir();
    let stamp = Local::now().format("%Y-%m-%d_%H-%M-%S-%3f");
    let path = dir.join(format!("EmelkaSnap_{stamp}.png"));
    let file = File::create(&path).ok()?;
    let writer = BufWriter::new(file);
    let mut encoder = png::Encoder::new(writer, w, h);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder.set_compression(png::Compression::Fast);
    let mut png_writer = encoder.write_header().ok()?;
    png_writer.write_image_data(rgba).ok()?;
    Some(path)
}

/// Processes a raw cropped region on a background thread, copies to Clipboard, saves PNG,
/// updates preview state, and pushes a Notch notification.
pub(crate) fn process_capture_async(
    ctx: egui::Context,
    state: Arc<Mutex<ScreenshotState>>,
    notifs: Arc<Mutex<NotifState>>,
    mode: CaptureMode,
    crop_w: u32,
    crop_h: u32,
    crop_rgba: Vec<u8>,
) {
    let (bg_mode, shadow_enabled, bg_cache) = {
        let mut st = state.lock().unwrap();
        st.busy = true;
        (st.bg_mode, st.shadow_enabled, st.bg_image_cache.clone())
    };

    thread::spawn(move || {
        let bg_ref = bg_cache
            .as_ref()
            .map(|(_, bw, bh, bpx)| (*bw, *bh, bpx.as_slice()));

        let (out_w, out_h, out_rgba) =
            render_screenshot(&crop_rgba, crop_w, crop_h, bg_mode, bg_ref, shadow_enabled);

        let copied = copy_rgba_to_clipboard(out_w, out_h, &out_rgba);
        let saved_path = save_rgba_as_png(out_w, out_h, &out_rgba);
        let preview = Arc::new(downscale_preview(&out_rgba, out_w, out_h, 420));

        {
            let mut st = state.lock().unwrap();
            let ver = st.next_ver;
            st.next_ver += 1;
            st.latest = Some(CaptureResult {
                preview,
                rgba: Arc::new(out_rgba),
                width: out_w,
                height: out_h,
                saved_path: saved_path.clone(),
                mode,
                at: Instant::now(),
                ver,
            });
            st.busy = false;
            let msg = match (copied, saved_path.is_some()) {
                (true, true) => "Copied to clipboard & saved to Pictures/Screenshots",
                (true, false) => "Copied to clipboard",
                (false, true) => "Saved to Pictures/Screenshots",
                (false, false) => "Capture failed to save",
            };
            st.status_msg = Some((msg.to_string(), Instant::now()));
        }

        let body = if let Some(p) = &saved_path {
            let fname = p
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "Screenshot.png".to_string());
            format!("Copied & saved as {fname}")
        } else {
            "Copied to clipboard".to_string()
        };
        push_notice(&notifs, "Screenshot captured", body);
        ctx.request_repaint();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_render_white_bg_and_shadow() {
        let w = 200u32;
        let h = 120u32;
        let src = vec![40u8; (w * h * 4) as usize];
        let (out_w, out_h, out) = render_screenshot(&src, w, h, BgMode::White, None, true);
        assert!(out_w > w && out_h > h);
        assert_eq!(out.len(), (out_w * out_h * 4) as usize);
        // Top-left corner of canvas should be white background (#FFFFFF)
        assert_eq!(&out[0..4], &[255, 255, 255, 255]);
    }

    #[test]
    fn test_render_custom_image_bg() {
        let w = 160u32;
        let h = 100u32;
        let src = vec![200u8; (w * h * 4) as usize];
        let bg = vec![80u8; 64 * 64 * 4];
        let (out_w, out_h, out) =
            render_screenshot(&src, w, h, BgMode::Image, Some((64, 64, &bg)), true);
        assert!(out_w > w && out_h > h);
        assert_eq!(out[0], 80);
        assert_eq!(out[3], 255);
    }
}
