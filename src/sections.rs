// =====================================================================
// Section rendering (impl block tambahan untuk EmelkaNotch)
// =====================================================================

use crate::antigravity::AuthStatus;
use crate::media::{self, app_key, track_key, Lyrics, Media, LYRIC_LEAD_MS};
use crate::notification::activate_notif;
use crate::prayer::{fmt_hm, prayer_color, prayer_icon, MAIN_PRAYERS, PRAYER_NAMES};
use crate::settings::{save_settings, NotchStyle};
use crate::system::{fmt_due, fmt_gb};
use crate::tasks::{save_todos, Todo};
use crate::theme::*;
use crate::ui::*;
use crate::updater::Upd;
use crate::EmelkaNotch;
use chrono::{Datelike, Local, NaiveDate, TimeZone, Timelike};
use eframe::egui::{
    self, pos2, vec2, Align2, Color32, CornerRadius, FontId, Rect, Sense, Stroke, StrokeKind, Ui,
};
use std::time::Instant;

impl EmelkaNotch {
    pub(crate) fn update_button(&self, ui: &mut Ui) {
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
            std::thread::spawn(move || crate::updater::run_update(upd, notifs, ctx, true));
        }
    }

    pub(crate) fn settings_section(&mut self, ui: &mut Ui) {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.label(
            egui::RichText::new("Appearance")
                .font(semibold(12.0))
                .color(TEXT),
        );
        ui.add_space(6.0);

        let gap = 8.0;
        let w = (ui.available_width() - gap) / 2.0;
        let h = 80.0;
        let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), h), Sense::hover());

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

            if resp.clicked() && self.style != *st {
                self.style = *st;
                crate::settings::save_settings(self.style, self.osd_enabled);
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

    pub(crate) fn alerts_section(&mut self, ui: &mut Ui) {
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
                    egui::RichText::new("No notifications yet")
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
                // ---- dismiss button (x), di kanan: klik ini TIDAK membuka apa pun ----
                let x_rect = Rect::from_min_size(
                    pos2(full.right() - 26.0, full.center().y - 12.0),
                    vec2(24.0, 24.0),
                );
                let xr = btn_rect(ui, x_rect, ("notif_x", i));
                let c = pos2(full.right() - 14.0, full.center().y);
                let st = Stroke::new(1.3, if xr.hovered() { TEXT } else { MUTED });
                p.line_segment([c + vec2(-3.5, -3.5), c + vec2(3.5, 3.5)], st);
                p.line_segment([c + vec2(-3.5, 3.5), c + vec2(3.5, -3.5)], st);
                if xr.clicked() {
                    start_one = Some(n.at);
                }

                // ---- klik di seluruh baris = buka aksi notifikasi (app / link) ----
                // Tidak menghapus; itu hanya lewat tombol x di atas.
                if resp.clicked() {
                    activate_notif(n);
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

    // ---- helpers for the offset control ----
    pub(crate) fn lead_for(&self, media: &Media) -> i64 {
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
            ellipsize(&media::app_label(&media.app), 10)
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
            media::save_offsets(&self.offsets);
        }
    }

    pub(crate) fn media_section(
        &mut self,
        ui: &mut Ui,
        media: &Media,
        pos_ms: u64,
        lyrics: &Lyrics,
    ) {
        if !media.available {
            ui.add_space(26.0);
            ui.vertical_centered(|ui| {
                ui.label(
                    egui::RichText::new("Nothing playing")
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

        // ---- baris atas: cover (kalau ada) + teks di kiri, kontrol di kanan ----
        const ART: f32 = 50.0;
        let width = ui.available_width();
        let app = media::app_label(&media.app);
        let album = media.album.trim();
        // Baris ke-3: "Album · App" (atau hanya App kalau album tidak tersedia).
        let meta = if album.is_empty() {
            app
        } else {
            format!("{album} · {app}")
        };

        // Cover: tekstur di-cache per versi cover; yang lama dibuang supaya tidak menumpuk.
        let (art, art_ver) = {
            let sh = self.shared.lock().unwrap();
            (sh.art.clone(), sh.art_ver)
        };
        let art_tex = art.and_then(|img| {
            let id = format!("art:{art_ver}");
            if !self.textures.contains_key(&id) {
                self.textures.retain(|k, _| !k.starts_with("art:"));
            }
            self.icon_tex(ui.ctx(), &id, &Some(img))
        });
        let has_art = art_tex.is_some();
        let text_w_max = width - 130.0 - if has_art { ART + 10.0 } else { 0.0 };
        let (n_title, n_artist, n_meta) = if has_art { (28, 38, 42) } else { (36, 48, 52) };

        ui.horizontal(|ui| {
            if let Some(tex) = &art_tex {
                let (r, _) = ui.allocate_exact_size(vec2(ART, ART), Sense::hover());
                egui::Image::new(egui::load::SizedTexture::new(tex.id(), r.size()))
                    .corner_radius(CornerRadius::same(8))
                    .paint_at(ui, r);
            }
            ui.vertical(|ui| {
                ui.set_width(text_w_max);
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(ellipsize(&media.title, n_title))
                            .font(semibold(15.0))
                            .color(TEXT),
                    )
                    .wrap_mode(egui::TextWrapMode::Extend),
                );
                if !media.artist.trim().is_empty() {
                    ui.add(
                        egui::Label::new(
                            egui::RichText::new(ellipsize(media.artist.trim(), n_artist))
                                .size(12.0)
                                .color(MUTED),
                        )
                        .wrap_mode(egui::TextWrapMode::Extend),
                    );
                }
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(ellipsize(&meta, n_meta))
                            .size(10.5)
                            .color(DIM),
                    )
                    .wrap_mode(egui::TextWrapMode::Extend),
                );
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icon_button(ui, Icon::Next, 28.0, false).clicked() {
                    let _ = self.tx.send(media::Cmd::Next);
                }
                let icon = if media.playing {
                    Icon::Pause
                } else {
                    Icon::Play
                };
                if icon_button(ui, icon, 34.0, true).clicked() {
                    let _ = self.tx.send(media::Cmd::PlayPause);
                }
                if icon_button(ui, Icon::Prev, 28.0, false).clicked() {
                    let _ = self.tx.send(media::Cmd::Prev);
                }
            });
        });

        // ---- progress ----
        ui.add_space(5.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 8.0;
            ui.label(small(media::fmt_time(pos_ms)));
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
                    let _ = self
                        .tx
                        .send(media::Cmd::Seek((f * media.dur_ms as f32) as u64));
                }
            }
            ui.label(small(media::fmt_time(media.dur_ms)));
        });

        // ---- lirik (area dipadatkan sedikit supaya muat dengan baris album) ----
        ui.add_space(4.0);
        let (area, _) = ui.allocate_exact_size(vec2(ui.available_width(), 46.0), Sense::hover());
        let lead = self.lead_for(media);
        Self::draw_lyrics(ui, area, lyrics, media::shift(pos_ms, lead));
        self.offset_control(ui, area, media);
    }

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

    pub(crate) fn system_section(&self, ui: &mut Ui) {
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
                let row_h = (88.0_f32 / shown as f32).min(30.0);
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

    pub(crate) fn prayer_section(&mut self, ui: &mut Ui) {
        let today = Local::now().date_naive();
        let (status, kab, today_d, tomorrow_d) = {
            let s = self.prayer.lock().unwrap();
            (
                s.status.clone(),
                s.kabkota.clone(),
                s.days.iter().find(|d| d.date == today).copied(),
                today
                    .succ_opt()
                    .and_then(|t| s.days.iter().find(|d| d.date == t).copied()),
            )
        };

        // ---- belum ada data: loading / error ----
        let Some(day) = today_d else {
            let mut retry = false;
            ui.add_space(18.0);
            ui.vertical_centered(|ui| {
                let (title, sub) = match &status {
                    crate::prayer::PrayerStatus::Error(e) => {
                        ("Jadwal belum tersedia".to_string(), ellipsize(e, 70))
                    }
                    crate::prayer::PrayerStatus::Loading => (
                        "Mengambil jadwal sholat…".to_string(),
                        "Sumber: equran.id (Kemenag RI)".to_string(),
                    ),
                    _ => (
                        "Mencari lokasimu…".to_string(),
                        "Memakai lokasi perangkat, fallback ke IP".to_string(),
                    ),
                };
                ui.label(egui::RichText::new(title).font(semibold(14.0)).color(TEXT));
                ui.add_space(2.0);
                ui.label(small(sub));
                if matches!(status, crate::prayer::PrayerStatus::Error(_)) {
                    ui.add_space(8.0);
                    let (r, _) = ui.allocate_exact_size(vec2(92.0, 26.0), Sense::hover());
                    let p = ui.painter().clone();
                    if text_btn(ui, &p, r, "Coba lagi", "pr_retry", true, true) {
                        retry = true;
                    }
                }
            });
            if retry {
                let mut s = self.prayer.lock().unwrap();
                s.retry = true;
                s.status = crate::prayer::PrayerStatus::Locating;
            }
            return;
        };

        // ---- hitung sholat berikutnya ----
        let now = Local::now();
        let secs = now.num_seconds_from_midnight() as i64;
        let times: [i64; 8] = day.t.map(|m| m as i64 * 60);

        let next_idx = MAIN_PRAYERS.iter().copied().find(|&i| times[i] > secs);
        let tomorrow = next_idx.is_none();
        let (next_i, next_at) = match next_idx {
            Some(i) => (i, times[i]),
            None => {
                // lewat Isya -> Subuh besok (kalau data besok belum ada, pakai perkiraan hari ini)
                let t = tomorrow_d.map_or(times[1], |d| d.t[1] as i64 * 60);
                (1usize, 86_400 + t)
            }
        };
        let cur = MAIN_PRAYERS
            .iter()
            .rev()
            .copied()
            .find(|&i| times[i] <= secs);
        let prev_at = cur.map_or(times[7] - 86_400, |i| times[i]);
        let frac = ((secs - prev_at) as f32 / (next_at - prev_at).max(1) as f32).clamp(0.0, 1.0);
        let remain = (next_at - secs).max(0);
        let cd = format!(
            "{:02}:{:02}:{:02}",
            remain / 3600,
            (remain % 3600) / 60,
            remain % 60
        );
        let shown = |i: usize| -> u32 {
            if tomorrow && i == 1 {
                ((next_at - 86_400) / 60) as u32
            } else {
                day.t[i]
            }
        };

        // =============== HERO ===============
        let w = ui.available_width();
        let col = prayer_color(next_i);
        let (hero, _) = ui.allocate_exact_size(vec2(w, 56.0), Sense::hover());
        let p = ui.painter().clone();
        let hero_bg = CARD.lerp_to_gamma(col, 0.10);
        p.rect_filled(hero, CornerRadius::same(12), hero_bg);
        p.rect_stroke(
            hero,
            CornerRadius::same(12),
            Stroke::new(1.0, col.gamma_multiply(0.45)),
            StrokeKind::Inside,
        );

        // ikon + countdown
        let ic = pos2(hero.left() + 30.0, hero.top() + 26.0);
        let ic_bg = hero_bg.lerp_to_gamma(col, 0.22);
        p.circle_filled(ic, 18.0, ic_bg);
        prayer_icon(&p, ic, next_i, col, ic_bg);
        p.text(
            pos2(hero.left() + 58.0, hero.top() + 15.0),
            Align2::LEFT_CENTER,
            format!(
                "Menuju {}{}",
                PRAYER_NAMES[next_i],
                if tomorrow { " (besok)" } else { "" }
            ),
            FontId::proportional(10.5),
            MUTED,
        );
        p.text(
            pos2(hero.left() + 58.0, hero.top() + 34.0),
            Align2::LEFT_CENTER,
            cd,
            semibold(20.0),
            TEXT,
        );

        // Imsak / Terbit / Dhuha
        let x0 = hero.left() + 170.0;
        for (j, idx) in [0usize, 2, 3].into_iter().enumerate() {
            let cx = x0 + j as f32 * 58.0 + 29.0;
            p.text(
                pos2(cx, hero.top() + 19.0),
                Align2::CENTER_CENTER,
                PRAYER_NAMES[idx],
                FontId::proportional(10.0),
                prayer_color(idx),
            );
            p.text(
                pos2(cx, hero.top() + 34.0),
                Align2::CENTER_CENTER,
                fmt_hm(day.t[idx]),
                semibold(12.0),
                TEXT,
            );
        }

        // waktu berikutnya + lokasi (klik = deteksi ulang)
        p.text(
            pos2(hero.right() - 14.0, hero.top() + 19.0),
            Align2::RIGHT_CENTER,
            fmt_hm(shown(next_i)),
            semibold(17.0),
            col,
        );
        let loc_txt = ellipsize(kab.trim_start_matches("Kota "), 18);
        let lf = FontId::proportional(10.5);
        let lw = text_w(&p, &loc_txt, &lf);
        let loc_rect = Rect::from_min_max(
            pos2(hero.right() - lw - 36.0, hero.top() + 28.0),
            pos2(hero.right() - 6.0, hero.top() + 46.0),
        );
        let lr = btn_rect(ui, loc_rect, "pr_loc").on_hover_text("Klik untuk deteksi ulang lokasi");
        let lcol = if lr.hovered() { TEXT } else { MUTED };
        p.text(
            pos2(hero.right() - 14.0, hero.top() + 37.0),
            Align2::RIGHT_CENTER,
            &loc_txt,
            lf,
            lcol,
        );
        let pin = pos2(hero.right() - 14.0 - lw - 9.0, hero.top() + 37.0);
        p.circle_stroke(pin, 2.6, Stroke::new(1.1, lcol));
        p.circle_filled(pin, 0.9, lcol);
        if lr.clicked() {
            self.prayer.lock().unwrap().retry = true;
        }

        // progress bar antar sholat
        let track = Rect::from_min_max(
            pos2(hero.left() + 14.0, hero.bottom() - 7.0),
            pos2(hero.right() - 14.0, hero.bottom() - 5.0),
        );
        p.rect_filled(track, CornerRadius::same(1), TRACK);
        p.rect_filled(
            Rect::from_min_size(track.min, vec2(track.width() * frac, track.height())),
            CornerRadius::same(1),
            col,
        );

        // =============== 5 KARTU ===============
        ui.add_space(8.0);
        let gap = 6.0;
        let cw = (w - gap * 4.0) / 5.0;
        let ch = 60.0;
        let (row, _) = ui.allocate_exact_size(vec2(w, ch), Sense::hover());
        for (n, &i) in MAIN_PRAYERS.iter().enumerate() {
            let r = Rect::from_min_size(
                pos2(row.left() + n as f32 * (cw + gap), row.top()),
                vec2(cw, ch),
            );
            let c = prayer_color(i);
            let is_next = i == next_i;
            let is_cur = cur == Some(i);
            let passed = !is_next && !is_cur && times[i] <= secs;

            let bgc = if is_next {
                CARD.lerp_to_gamma(c, 0.16)
            } else {
                CARD
            };
            p.rect_filled(r, CornerRadius::same(12), bgc);
            p.rect_stroke(
                r,
                CornerRadius::same(12),
                Stroke::new(
                    if is_next { 1.5 } else { 1.0 },
                    if is_next {
                        c
                    } else if is_cur {
                        c.gamma_multiply(0.5)
                    } else {
                        BORDER
                    },
                ),
                StrokeKind::Inside,
            );
            prayer_icon(
                &p,
                pos2(r.center().x, r.top() + 17.0),
                i,
                if passed { DIM } else { c },
                bgc,
            );
            p.text(
                pos2(r.center().x, r.top() + 35.0),
                Align2::CENTER_CENTER,
                PRAYER_NAMES[i],
                FontId::proportional(10.5),
                if passed {
                    DIM
                } else if is_next {
                    TEXT
                } else {
                    MUTED
                },
            );
            p.text(
                pos2(r.center().x, r.top() + 49.0),
                Align2::CENTER_CENTER,
                fmt_hm(shown(i)),
                semibold(13.0),
                if passed {
                    DIM
                } else if is_next {
                    c
                } else {
                    TEXT
                },
            );
            if is_cur {
                // titik kecil = waktu sholat yang sedang berlangsung
                p.circle_filled(pos2(r.right() - 9.0, r.top() + 9.0), 2.5, c);
            }
        }
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
                egui::UiBuilder::new()
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
                            .hint_text(egui::RichText::new("Add a task…").color(DIM)),
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

    pub(crate) fn todo_section(&mut self, ui: &mut Ui) {
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
                    egui::RichText::new("No tasks yet")
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

    pub(crate) fn weather_section(&mut self, ui: &mut Ui) {
        let retry = {
            let st = self.weather.lock().unwrap();
            crate::weather::draw_weather_panel(ui, &st)
        };
        if retry {
            self.weather.lock().unwrap().retry = true;
        }
    }

    pub(crate) fn antigravity_section(&mut self, ui: &mut Ui) {
        ui.spacing_mut().item_spacing.y = 6.0;

        let (status, quota, is_loading) = {
            let st = self.antigravity.lock().unwrap();
            (st.status.clone(), st.quota.clone(), st.is_loading)
        };

        let w = ui.available_width();

        if let Some(q) = quota {
            // 1. Header card
            let h_head = 54.0;
            let (r_head, p_head) = card(ui, w, h_head);

            p_head.text(
                pos2(r_head.left() + 14.0, r_head.top() + 18.0),
                Align2::LEFT_CENTER,
                "Models & Quota",
                semibold(14.0),
                TEXT,
            );

            let acc_text = format!("Account: {}", q.account);
            p_head.text(
                pos2(r_head.left() + 14.0, r_head.top() + 38.0),
                Align2::LEFT_CENTER,
                &acc_text,
                FontId::proportional(11.0),
                MUTED,
            );

            // Actions: Refresh + Sign out
            let btn_w = 58.0;
            let btn_h = 24.0;
            let r_logout = Rect::from_min_size(
                pos2(
                    r_head.right() - btn_w - 12.0,
                    r_head.center().y - btn_h / 2.0,
                ),
                vec2(btn_w, btn_h),
            );
            if text_btn(ui, &p_head, r_logout, "Sign out", "ag_logout", false, true) {
                self.antigravity.lock().unwrap().trigger_logout = true;
            }

            let r_refresh = Rect::from_min_size(
                pos2(r_logout.left() - 32.0, r_head.center().y - btn_h / 2.0),
                vec2(26.0, btn_h),
            );
            let ref_label = if is_loading { "…" } else { "↻" };
            if text_btn(
                ui,
                &p_head,
                r_refresh,
                ref_label,
                "ag_refresh",
                false,
                !is_loading,
            ) {
                self.antigravity.lock().unwrap().trigger_refresh = true;
            }

            // 2. Scrollable Quota Groups Card
            let h_body = (ui.available_height() - 4.0).max(180.0);
            let (r_body, _) = card(ui, w, h_body);

            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r_body.shrink(10.0)));

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(&mut child, |ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;

                    for group in &q.groups {
                        ui.add_space(2.0);
                        ui.label(
                            egui::RichText::new(group.name.to_uppercase())
                                .font(semibold(11.5))
                                .color(TEXT),
                        );

                        if !group.description.is_empty() {
                            ui.label(
                                egui::RichText::new(&group.description)
                                    .font(FontId::proportional(10.0))
                                    .color(MUTED),
                            );
                        }

                        for bucket in &group.buckets {
                            ui.add_space(2.0);
                            let pct = (bucket.remaining_fraction * 100.0).clamp(0.0, 100.0);
                            let pct_str = format!("{:.2}%", pct);

                            // Title row: Bucket name & percentage
                            ui.horizontal(|ui| {
                                ui.label(
                                    egui::RichText::new(&bucket.name)
                                        .font(semibold(11.0))
                                        .color(TEXT),
                                );
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.label(
                                            egui::RichText::new(&pct_str)
                                                .font(semibold(11.5))
                                                .color(TEXT),
                                        );
                                    },
                                );
                            });

                            // Progress bar
                            let bar_h = 7.0;
                            let (bar_rect, _) = ui.allocate_exact_size(
                                vec2(ui.available_width(), bar_h),
                                Sense::hover(),
                            );
                            let p_bar = ui.painter();
                            p_bar.rect_filled(bar_rect, CornerRadius::same(3), TRACK);

                            if bucket.remaining_fraction > 0.0 {
                                let fill_w = bar_rect.width() * bucket.remaining_fraction.min(1.0);
                                let fill_rect =
                                    Rect::from_min_size(bar_rect.min, vec2(fill_w, bar_h));
                                let fill_col = if pct < 20.0 { WARN } else { GREEN };
                                p_bar.rect_filled(fill_rect, CornerRadius::same(3), fill_col);
                            }

                            // Reset subtitle (e.g. "Refreshes in 167h 23m" or "Quota available")
                            let reset_str = crate::antigravity::format_reset_info(
                                bucket.reset_time.as_deref(),
                                bucket.remaining_fraction,
                            );
                            ui.label(
                                egui::RichText::new(reset_str)
                                    .font(FontId::proportional(10.0))
                                    .color(GREEN),
                            );
                        }

                        ui.add_space(6.0);
                    }
                });
        } else {
            match status {
                AuthStatus::Authenticating { port } => {
                    let h_card = 200.0;
                    let (r, p) = card(ui, w, h_card);

                    p.text(
                        pos2(r.center().x, r.top() + 45.0),
                        Align2::CENTER_CENTER,
                        "Browser Authorization",
                        semibold(15.0),
                        TEXT,
                    );

                    let msg = format!(
                        "A browser window was opened for Google OAuth sign in.\nListening for callback on 127.0.0.1:{}...",
                        port
                    );
                    p.text(
                        pos2(r.center().x, r.top() + 85.0),
                        Align2::CENTER_CENTER,
                        &msg,
                        FontId::proportional(11.5),
                        MUTED,
                    );

                    let cancel_w = 90.0;
                    let cancel_h = 28.0;
                    let r_cancel = Rect::from_center_size(
                        pos2(r.center().x, r.top() + 140.0),
                        vec2(cancel_w, cancel_h),
                    );
                    if text_btn(ui, &p, r_cancel, "Cancel", "ag_cancel_oauth", false, true) {
                        let mut st = self.antigravity.lock().unwrap();
                        st.status = AuthStatus::LoggedOut;
                    }
                }
                AuthStatus::LoggedOut | AuthStatus::Error(_) => {
                    let h_card = 220.0;
                    let (r, p) = card(ui, w, h_card);

                    let ic_c = pos2(r.center().x, r.top() + 40.0);
                    let pts = vec![
                        pos2(ic_c.x, ic_c.y - 7.0),
                        pos2(ic_c.x + 5.5, ic_c.y),
                        pos2(ic_c.x, ic_c.y + 7.0),
                        pos2(ic_c.x - 5.5, ic_c.y),
                    ];
                    p.add(egui::Shape::closed_line(pts, Stroke::new(1.5, ACCENT)));
                    p.circle_filled(ic_c, 1.8, ACCENT);

                    p.text(
                        pos2(r.center().x, r.top() + 75.0),
                        Align2::CENTER_CENTER,
                        "Antigravity Models & Quota",
                        semibold(15.0),
                        TEXT,
                    );

                    p.text(
                        pos2(r.center().x, r.top() + 102.0),
                        Align2::CENTER_CENTER,
                        "Sign in with Google OAuth to track your Antigravity models and quota.",
                        FontId::proportional(11.5),
                        MUTED,
                    );

                    if let AuthStatus::Error(ref err) = status {
                        p.text(
                            pos2(r.center().x, r.top() + 125.0),
                            Align2::CENTER_CENTER,
                            err,
                            FontId::proportional(10.5),
                            WARN,
                        );
                    }

                    let btn_w = 200.0;
                    let btn_h = 32.0;
                    let r_login = Rect::from_center_size(
                        pos2(r.center().x, r.top() + 160.0),
                        vec2(btn_w, btn_h),
                    );
                    if text_btn(
                        ui,
                        &p,
                        r_login,
                        "Sign in with Google OAuth",
                        "ag_login_btn",
                        true,
                        true,
                    ) {
                        self.antigravity.lock().unwrap().trigger_login = true;
                    }
                }
                _ => {
                    let h_card = 160.0;
                    let (r, p) = card(ui, w, h_card);
                    p.text(
                        pos2(r.center().x, r.center().y),
                        Align2::CENTER_CENTER,
                        "Loading Models & Quota...",
                        semibold(13.0),
                        MUTED,
                    );
                }
            }
        }
    }
}
