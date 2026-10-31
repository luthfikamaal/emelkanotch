#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod antigravity;
mod media;
mod notification;
mod osd;
mod prayer;
mod sections;
mod settings;
mod system;
mod tasks;
mod theme;
mod ui;
mod updater;
mod weather;

use chrono::{Datelike, Local, NaiveDate};
use eframe::egui::{
    self, pos2, vec2, Align2, CornerRadius, FontId, Rect, Stroke, StrokeKind, Ui, UiBuilder,
};
use std::collections::{HashMap, VecDeque};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};
use sysinfo::System;

use antigravity::{spawn_antigravity_worker, AntigravityState};
use media::{spawn_media_worker, Cmd, Lyrics, Media, Shared};
use notification::{push_notice, spawn_notif_worker, Notif, NotifState};
use osd::{spawn_brightness_worker, spawn_volume_worker, OsdEvent, OsdKind, OsdState};
use prayer::{load_prayer_cache, spawn_prayer_notifier, spawn_prayer_worker, PrayerState};
use settings::{load_settings, NotchStyle};
use system::{read_disks, DiskInfo};
use tasks::{load_todos, spawn_todo_worker, TodoState};
use theme::*;
use ui::{cursor_local, push_hist, shape_rect, tab_icon, Section};
use updater::{spawn_updater, take_updated_marker, Upd};
use weather::{load_weather_cache, spawn_weather_worker, WeatherState};

// =====================================================================
// App
// =====================================================================

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
    style_t: f32, // 0 = Floating, 1 = Attached (animated)
    osd_enabled: bool,
    osd: Arc<Mutex<OsdState>>,

    // animation / hover
    t: f32,
    panel_cur_h: f32,
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

    prayer: Arc<Mutex<PrayerState>>,
    weather: Arc<Mutex<WeatherState>>,
    pub(crate) antigravity: Arc<Mutex<AntigravityState>>,

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

    // lyric offsets
    offsets: media::Offsets,
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
        let prayer = Arc::new(Mutex::new(load_prayer_cache()));
        spawn_prayer_worker(cc.egui_ctx.clone(), prayer.clone());
        spawn_prayer_notifier(cc.egui_ctx.clone(), prayer.clone(), notifs.clone());
        let weather = Arc::new(Mutex::new(load_weather_cache()));
        spawn_weather_worker(cc.egui_ctx.clone(), weather.clone());
        let antigravity = Arc::new(Mutex::new(AntigravityState::default()));
        spawn_antigravity_worker(cc.egui_ctx.clone(), antigravity.clone());
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
            style_t: if style == NotchStyle::Attached {
                1.0
            } else {
                0.0
            },
            osd_enabled,
            osd,
            t: 0.0,
            panel_cur_h: PANEL_H,
            open: false,
            hover_since: None,
            last_inside: Instant::now(),
            passthrough: false,
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
            prayer,
            weather,
            antigravity,
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
            offsets: media::load_offsets(),
            off_app: false,
            off_hold: None,
        }
    }

    fn top(&self) -> f32 {
        lerp(TOP, 0.0, self.style_t)
    }

    /// Weather and Antigravity get taller panels so cards have room to breathe.
    /// Keep it tall through the slide transition so the outgoing content isn't clipped.
    fn panel_target_h(&self) -> f32 {
        if self.section == Section::Weather || self.prev_section == Some(Section::Weather) {
            WEATHER_PANEL_H
        } else if self.section == Section::Antigravity
            || self.prev_section == Some(Section::Antigravity)
        {
            ANTIGRAVITY_PANEL_H
        } else if self.section == Section::Prayer || self.prev_section == Some(Section::Prayer) {
            PRAYER_PANEL_H
        } else {
            PANEL_H
        }
    }

    fn corner_radius(&self, e: f32) -> CornerRadius {
        let s = self.style_t;
        let r_float = lerp((self.pill_h / 2.0).min(22.0), 26.0, e);
        let r_attach = lerp(10.0, 24.0, e);

        let top_r = lerp(r_float, 0.0, s);
        let bot_r = lerp(r_float, r_attach, s);
        CornerRadius {
            nw: top_r.round() as u8,
            ne: top_r.round() as u8,
            sw: bot_r.round() as u8,
            se: bot_r.round() as u8,
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
        let dt = ctx.input(|i| i.stable_dt).min(0.05);
        let target_h = self.panel_target_h();
        let kh = 1.0 - (-dt * 14.0).exp();
        self.panel_cur_h += (target_h - self.panel_cur_h) * kh;
        if (target_h - self.panel_cur_h).abs() < 0.5 {
            self.panel_cur_h = target_h;
        } else {
            ctx.request_repaint();
        }

        let shape = shape_rect(
            self.pill_w,
            self.pill_h,
            ease(self.t),
            self.top(),
            self.panel_cur_h,
        );
        let slack = if self.open { 6.0 } else { 28.0 };
        let half = (shape.width() / 2.0 + slack).max(if self.open { 0.0 } else { 120.0 });
        let cx = shape.center().x;
        let hit = Rect::from_min_max(
            pos2(cx - half, -10.0),
            pos2(cx + half, shape.bottom() + slack),
        );
        let over = cursor_local(ctx).map_or(false, |c| hit.contains(c));
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
        let step = dt / if self.open { 0.40 } else { 0.30 };
        self.t = if self.open {
            (self.t + step).min(1.0)
        } else {
            (self.t - step).max(0.0)
        };

        // Let clicks fall through the transparent area below the panel, but keep
        // the window interactive while dragging a control inside it.
        let dragging = ctx.input(|i| i.pointer.any_down());
        let want_passthrough = !(inside || dragging);
        if want_passthrough != self.passthrough {
            self.passthrough = want_passthrough;
            ctx.send_viewport_cmd(egui::ViewportCommand::MousePassthrough(want_passthrough));
        }
    }

    /// Decide what the pill shows (osd > notification > lyric > title > idle) and measure it.
    #[allow(clippy::too_many_arguments)]
    fn refresh_view(
        &mut self,
        p: &egui::Painter,
        media: &Media,
        lyrics: &Lyrics,
        pos_ms: u64,
        lead: i64,
        notif: Option<&Notif>,
        osd: Option<OsdEvent>,
    ) {
        let lyric_line = if media.playing {
            media::current_line(lyrics, media::shift(pos_ms, lead))
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
        let panel_h = self.panel_cur_h;
        let rect = shape_rect(self.pill_w, self.pill_h, e, top, panel_h);
        let radius = self.corner_radius(e);

        painter.rect_filled(rect, radius, BG);
        painter.rect_stroke(
            rect,
            radius,
            Stroke::new(1.0, BORDER.gamma_multiply(0.45 + 0.55 * e)),
            StrokeKind::Inside,
        );

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
                ui::app_icon(
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
                    ui::osd_icon(
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
                vec2(PANEL_W - PAD * 2.0, panel_h - PAD * 2.0),
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
                (Section::Weather, "Weather"),
                (Section::Todo, "Tasks"),
                (Section::Prayer, "Prayer"),
                (Section::Antigravity, "Antigravity"),
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
                let mut old = ui::slide_child(
                    ui,
                    content,
                    -self.tab_dir * e * w,
                    a * (1.0 - e),
                    ("sec", prev as u8),
                );
                self.draw_section(&mut old, prev, media, pos_ms, lyrics);
                let cur = self.section;
                let mut new = ui::slide_child(
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
            Section::Weather => self.weather_section(ui),
            Section::Todo => self.todo_section(ui),
            Section::Prayer => self.prayer_section(ui),
            Section::Antigravity => self.antigravity_section(ui),
            Section::Settings => self.settings_section(ui),
        }
    }
}

impl eframe::App for EmelkaNotch {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Center horizontally on the monitor (in logical points, DPI-safe).
        if !self.positioned {
            let ppp = ctx.pixels_per_point();
            let screen_w = ctx
                .input(|i| i.viewport().monitor_size)
                .map(|m| m.x)
                .unwrap_or_else(|| unsafe {
                    windows_sys::Win32::UI::WindowsAndMessaging::GetSystemMetrics(0) as f32 / ppp
                });
            let x = ((screen_w - WIN_W) / 2.0).max(0.0);
            ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(pos2(x, 0.0)));
            self.positioned = true;
        }

        self.sample_system();
        self.style_t = settings::style_progress(ctx, self.style);
        self.update_hover(ctx);
        let e = ease(self.t);

        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |ui| self.draw(ui, e));

        let playing = self.shared.lock().unwrap().media.playing;
        if self.t > 0.0 || self.animating || settings::style_animating(self.style_t) {
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
        .with_active(false);

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
