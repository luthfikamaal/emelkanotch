// =====================================================================
// Section rendering (impl block tambahan untuk EmelkaNotch)
// =====================================================================

use crate::antigravity::AuthStatus;
use crate::media::{self, app_key, track_key, Lyrics, Media, LYRIC_LEAD_MS};
use crate::notification::activate_notif;
use crate::prayer::{fmt_hm, prayer_color, prayer_icon, MAIN_PRAYERS, PRAYER_NAMES};
use crate::screenshot::{
    capture_raw_desktop, copy_rgba_to_clipboard, crop_desktop, process_capture_async,
    rerender_latest_async, save_screenshot_settings, screenshots_dir, spawn_bg_image_picker,
    BgMode, CaptureMode, OverlayPhase, DEFAULT_PADDING, DEFAULT_ROUNDING, DEFAULT_SHADOW,
};
use crate::settings::{
    save_settings, AppSettings, NotchStyle, MAX_REVEAL_HEIGHT, MIN_REVEAL_HEIGHT,
};
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
            Upd::Idle => (format!("v{VERSION}"), false),
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
                save_settings(AppSettings {
                    style: self.style,
                    osd: self.osd_enabled,
                    auto_hide: self.auto_hide,
                    reveal_height: self.reveal_height,
                });
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
            save_settings(AppSettings {
                style: self.style,
                osd: self.osd_enabled,
                auto_hide: self.auto_hide,
                reveal_height: self.reveal_height,
            });
        }

        // ---- Auto-hide notch toggle ----
        ui.add_space(6.0);
        let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
        let resp = btn_rect(ui, row, "autohide_toggle");
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
            "Auto-hide notch (reveal from top on hover)",
            semibold(11.5),
            TEXT,
        );
        let k = ui
            .ctx()
            .animate_bool_with_time(egui::Id::new("autohide_sw"), self.auto_hide, 0.15);
        let sw = Rect::from_center_size(pos2(row.right() - 27.0, row.center().y), vec2(30.0, 16.0));
        p.rect_filled(sw, CornerRadius::same(8), TRACK.lerp_to_gamma(ACCENT, k));
        p.circle_filled(
            pos2(lerp(sw.left() + 8.0, sw.right() - 8.0, k), sw.center().y),
            5.5,
            TEXT,
        );
        if resp.clicked() {
            self.auto_hide = !self.auto_hide;
            save_settings(AppSettings {
                style: self.style,
                osd: self.osd_enabled,
                auto_hide: self.auto_hide,
                reveal_height: self.reveal_height,
            });
        }

        // ---- Reveal trigger height ----
        ui.add_space(6.0);
        let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
        let p = ui.painter().clone();
        p.rect_filled(row, CornerRadius::same(10), CARD);
        p.rect_stroke(
            row,
            CornerRadius::same(10),
            Stroke::new(1.0, BORDER),
            StrokeKind::Inside,
        );
        let enabled = self.auto_hide;
        p.text(
            pos2(row.left() + 12.0, row.center().y),
            Align2::LEFT_CENTER,
            "Top hover trigger height",
            semibold(11.5),
            if enabled { TEXT } else { DIM },
        );

        let ctrl = Rect::from_center_size(
            pos2(row.right() - 56.0, row.center().y),
            vec2(96.0, 22.0),
        );
        p.rect_filled(
            ctrl,
            CornerRadius::same(6),
            if enabled {
                field_fill(ui.rect_contains_pointer(ctrl), false)
            } else {
                Color32::from_rgb(14, 15, 19)
            },
        );
        let minus = Rect::from_min_size(ctrl.min, vec2(26.0, ctrl.height()));
        let plus = Rect::from_min_size(
            pos2(ctrl.right() - 26.0, ctrl.top()),
            vec2(26.0, ctrl.height()),
        );
        let mut delta = 0.0f32;
        if enabled {
            if btn_rect(ui, minus, "rh_minus").clicked() {
                delta = -2.0;
            }
            if btn_rect(ui, plus, "rh_plus").clicked() {
                delta = 2.0;
            }
            if ui.rect_contains_pointer(ctrl) {
                let dy = ui.input(|i| i.raw_scroll_delta.y);
                if dy > 0.0 {
                    delta = 2.0;
                } else if dy < 0.0 {
                    delta = -2.0;
                }
            }
        }
        if delta != 0.0 {
            let next = (self.reveal_height + delta).clamp(MIN_REVEAL_HEIGHT, MAX_REVEAL_HEIGHT);
            if (next - self.reveal_height).abs() > 0.01 {
                self.reveal_height = next;
                save_settings(AppSettings {
                    style: self.style,
                    osd: self.osd_enabled,
                    auto_hide: self.auto_hide,
                    reveal_height: self.reveal_height,
                });
            }
        }
        let st = Stroke::new(1.3, if enabled { MUTED } else { DIM });
        let mc = minus.center();
        p.line_segment([mc - vec2(3.5, 0.0), mc + vec2(3.5, 0.0)], st);
        let pc = plus.center();
        p.line_segment([pc - vec2(3.5, 0.0), pc + vec2(3.5, 0.0)], st);
        p.line_segment([pc - vec2(0.0, 3.5), pc + vec2(0.0, 3.5)], st);
        p.text(
            ctrl.center(),
            Align2::CENTER_CENTER,
            format!("{} px", self.reveal_height.round() as i32),
            semibold(11.0),
            if enabled { TEXT } else { DIM },
        );

        // ---- Launch at Windows startup toggle ----
        ui.add_space(6.0);
        let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::hover());
        let resp = btn_rect(ui, row, "autostart_toggle");
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
            "Launch at Windows startup",
            semibold(11.5),
            TEXT,
        );
        let k = ui
            .ctx()
            .animate_bool_with_time(egui::Id::new("autostart_sw"), self.autostart, 0.15);
        let sw = Rect::from_center_size(pos2(row.right() - 27.0, row.center().y), vec2(30.0, 16.0));
        p.rect_filled(sw, CornerRadius::same(8), TRACK.lerp_to_gamma(ACCENT, k));
        p.circle_filled(
            pos2(lerp(sw.left() + 8.0, sw.right() - 8.0, k), sw.center().y),
            5.5,
            TEXT,
        );
        if resp.clicked() {
            let next = !self.autostart;
            if crate::settings::set_autostart(next) {
                self.autostart = next;
            }
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
            let w = ui.available_width();
            let h = ui.available_height().max(120.0);
            let (empty_r, p) = card(ui, w, h);
            let cx = empty_r.center().x;
            let cy = empty_r.center().y;

            let badge_c = pos2(cx, cy - 18.0);
            p.circle_filled(badge_c, 13.0, ACCENT.gamma_multiply(0.14));
            p.circle_stroke(badge_c, 13.0, Stroke::new(1.0, ACCENT.gamma_multiply(0.35)));
            let s = Stroke::new(1.7, ACCENT);
            p.line_segment([badge_c + vec2(-4.5, 0.5), badge_c + vec2(-1.2, 3.8)], s);
            p.line_segment([badge_c + vec2(-1.2, 3.8), badge_c + vec2(5.0, -3.5)], s);

            p.text(
                pos2(cx, cy + 6.0),
                Align2::CENTER_CENTER,
                "All caught up — no notifications",
                semibold(12.5),
                TEXT,
            );
            let sub = err.unwrap_or_else(|| "New Windows notifications will appear here".into());
            p.text(
                pos2(cx, cy + 23.0),
                Align2::CENTER_CENTER,
                sub,
                FontId::proportional(11.0),
                MUTED,
            );
            return;
        }

        const DUR: f32 = 0.25; // slide-out duration
        const STAGGER: f32 = 0.06; // delay between rows on "Clear all"
        let mut start_all = false;
        let mut start_one: Option<Instant> = None;

        // ---- header: count + "Clear all" ----
        let (head, _) = ui.allocate_exact_size(vec2(ui.available_width(), 20.0), Sense::hover());
        ui.painter()
            .circle_filled(pos2(head.left() + 6.0, head.center().y), 3.0, ACCENT);
        ui.painter().text(
            pos2(head.left() + 14.0, head.center().y),
            Align2::LEFT_CENTER,
            format!("{total} notification{}", if total == 1 { "" } else { "s" }),
            FontId::proportional(10.5),
            MUTED,
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

        ui.add_space(2.0);

        // ---- items: hover shows an x; removed rows slide right, fade, then collapse ----
        let anim_all = self.notif_clear.map(|t| t.elapsed().as_secs_f32());
        let mut animating = anim_all.is_some() || !self.notif_out.is_empty();
        ui.spacing_mut().item_spacing.y = 4.0;
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
            let row_bg = if hovered {
                Color32::from_rgb(21, 23, 30)
            } else {
                CARD
            };
            p.rect_filled(full, CornerRadius::same(8), row_bg);
            p.rect_stroke(
                full,
                CornerRadius::same(8),
                Stroke::new(
                    1.0,
                    if hovered {
                        BORDER
                    } else {
                        Color32::from_rgb(26, 28, 36)
                    },
                ),
                StrokeKind::Inside,
            );
            app_icon(
                &p,
                pos2(full.left() + 18.0, full.center().y),
                11.0,
                tex.as_ref(),
                &n.app,
                1.0,
            );

            let max = w - 42.0 - 52.0;
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
                pos2(full.left() + 38.0, full.top() + 25.5),
                Align2::LEFT_CENTER,
                body,
                FontId::proportional(11.0),
                MUTED,
            );
            if hovered {
                // ---- dismiss button (x), di kanan: klik ini TIDAK membuka apa pun ----
                let x_rect = Rect::from_center_size(
                    pos2(full.right() - 16.0, full.center().y),
                    vec2(22.0, 22.0),
                );
                let xr = btn_rect(ui, x_rect, ("notif_x", i));
                if xr.hovered() {
                    p.rect_filled(x_rect, CornerRadius::same(5), WARN.gamma_multiply(0.18));
                }
                let c = x_rect.center();
                let st = Stroke::new(1.3, if xr.hovered() { WARN } else { MUTED });
                p.line_segment([c + vec2(-3.2, -3.2), c + vec2(3.2, 3.2)], st);
                p.line_segment([c + vec2(-3.2, 3.2), c + vec2(3.2, -3.2)], st);
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
                    pos2(full.right() - 10.0, full.center().y),
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

    fn media_accent(&mut self, art: &Option<std::sync::Arc<egui::ColorImage>>, art_ver: u64) -> Color32 {
        if art_ver == self.art_accent.0 && art_ver != 0 {
            return self.art_accent.1;
        }
        let Some(img) = art else {
            self.art_accent = (0, ACCENT);
            return ACCENT;
        };
        let mut r_sum = 0.0f32;
        let mut g_sum = 0.0f32;
        let mut b_sum = 0.0f32;
        let mut w_sum = 0.0f32;
        for px in img.pixels.iter().step_by(4) {
            let r = px.r() as f32 / 255.0;
            let g = px.g() as f32 / 255.0;
            let b = px.b() as f32 / 255.0;
            let maxc = r.max(g).max(b);
            let minc = r.min(g).min(b);
            let sat = if maxc > 0.001 { (maxc - minc) / maxc } else { 0.0 };
            if maxc < 0.14 || maxc > 0.96 {
                continue;
            }
            let weight = 0.25 + sat * sat * 2.5;
            r_sum += r * weight;
            g_sum += g * weight;
            b_sum += b * weight;
            w_sum += weight;
        }
        let col = if w_sum > 0.01 {
            let mut r = r_sum / w_sum;
            let mut g = g_sum / w_sum;
            let mut b = b_sum / w_sum;
            // Lift dark accents so they stay crisp & readable on dark OLED surfaces
            let maxc = r.max(g).max(b).max(0.01);
            if maxc < 0.58 {
                let scale = 0.68 / maxc;
                r = (r * scale).min(1.0);
                g = (g * scale).min(1.0);
                b = (b * scale).min(1.0);
            }
            // Gently boost vibrancy without neon clipping
            let avg = (r + g + b) / 3.0;
            r = (avg + (r - avg) * 1.25).clamp(0.18, 0.95);
            g = (avg + (g - avg) * 1.25).clamp(0.18, 0.95);
            b = (avg + (b - avg) * 1.25).clamp(0.18, 0.95);
            Color32::from_rgb(
                (r * 255.0).round() as u8,
                (g * 255.0).round() as u8,
                (b * 255.0).round() as u8,
            )
        } else {
            ACCENT
        };
        self.art_accent = (art_ver, col);
        col
    }

    fn paint_media_hero(
        p: &egui::Painter,
        rect: Rect,
        radius: f32,
        accent: Color32,
        light_pos: egui::Pos2,
    ) {
        use std::f32::consts::{FRAC_PI_2, PI};
        let r = radius.min(rect.width() * 0.5).min(rect.height() * 0.5);
        let segs = 8usize;
        let corners = [
            (pos2(rect.right() - r, rect.top() + r), -FRAC_PI_2, 0.0),
            (pos2(rect.right() - r, rect.bottom() - r), 0.0, FRAC_PI_2),
            (pos2(rect.left() + r, rect.bottom() - r), FRAC_PI_2, PI),
            (pos2(rect.left() + r, rect.top() + r), PI, PI + FRAC_PI_2),
        ];

        let base_top = Color32::from_rgb(16, 18, 26);
        let base_bot = Color32::from_rgb(10, 11, 16);
        let sample = |pt: egui::Pos2| {
            let ty = ((pt.y - rect.top()) / rect.height().max(1.0)).clamp(0.0, 1.0);
            let base = base_top.lerp_to_gamma(base_bot, ty);
            let dist = (pt - light_pos).length() / 230.0;
            let glow = (1.0 - dist).clamp(0.0, 1.0).powi(2) * 0.34;
            base.lerp_to_gamma(accent, glow)
        };

        let mut perim = Vec::with_capacity(corners.len() * (segs + 1));
        for (center, a0, a1) in corners {
            for s in 0..=segs {
                let a = a0 + (a1 - a0) * (s as f32 / segs as f32);
                perim.push(pos2(center.x + r * a.cos(), center.y + r * a.sin()));
            }
        }
        let mut mesh = egui::Mesh::default();
        let c = rect.center();
        mesh.colored_vertex(c, sample(c));
        for &pt in &perim {
            mesh.colored_vertex(pt, sample(pt));
        }
        let n = perim.len() as u32;
        for i in 0..n {
            let next = if i + 1 == n { 1 } else { i + 2 };
            mesh.add_triangle(0, i + 1, next);
        }
        p.add(egui::Shape::mesh(mesh));
    }

    fn cover_uv(tex_size: egui::Vec2, target_size: egui::Vec2) -> Rect {
        if tex_size.x <= 0.0 || tex_size.y <= 0.0 || target_size.x <= 0.0 || target_size.y <= 0.0 {
            return Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));
        }
        let tex_ar = tex_size.x / tex_size.y;
        let dst_ar = target_size.x / target_size.y;
        if tex_ar > dst_ar {
            let keep = dst_ar / tex_ar;
            let m = (1.0 - keep) * 0.5;
            Rect::from_min_max(pos2(m, 0.0), pos2(1.0 - m, 1.0))
        } else {
            let keep = tex_ar / dst_ar;
            let m = (1.0 - keep) * 0.5;
            Rect::from_min_max(pos2(0.0, m), pos2(1.0, 1.0 - m))
        }
    }

    pub(crate) fn media_section(
        &mut self,
        ui: &mut Ui,
        media: &Media,
        pos_ms: u64,
        lyrics: &Lyrics,
    ) {
        ui.spacing_mut().item_spacing.y = 0.0;
        let w = ui.available_width();

        if !media.available {
            let (card, _) = ui.allocate_exact_size(vec2(w, 178.0), Sense::hover());
            let p = ui.painter().clone();
            p.rect_filled(card, CornerRadius::same(16), Color32::from_rgb(11, 13, 19));
            p.rect_stroke(
                card,
                CornerRadius::same(16),
                Stroke::new(1.0, Color32::from_rgb(30, 34, 46)),
                StrokeKind::Inside,
            );
            let vc = pos2(card.center().x, card.center().y - 24.0);
            p.circle_filled(vc, 22.0, Color32::from_rgb(18, 21, 30));
            p.circle_stroke(vc, 22.0, Stroke::new(1.0, Color32::from_rgb(42, 46, 62)));
            p.circle_stroke(vc, 14.0, Stroke::new(0.8, Color32::from_rgb(32, 36, 48)));
            p.circle_filled(vc, 5.5, Color32::from_rgb(36, 42, 58));
            p.circle_filled(vc, 2.0, Color32::from_rgb(11, 13, 19));
            p.text(
                pos2(card.center().x, card.center().y + 14.0),
                Align2::CENTER_CENTER,
                "Nothing playing",
                semibold(14.0),
                TEXT,
            );
            p.text(
                pos2(card.center().x, card.center().y + 33.0),
                Align2::CENTER_CENTER,
                "Play music in Spotify, YouTube Music, or Chrome",
                FontId::proportional(11.0),
                MUTED,
            );
            return;
        }

        let app = media::app_label(&media.app);
        let artist = media.artist.trim();
        let album = media.album.trim();
        let sub_line = match (artist.is_empty(), album.is_empty()) {
            (false, false) => format!("{artist} · {album}"),
            (false, true) => artist.to_string(),
            (true, false) => album.to_string(),
            (true, true) => app.clone(),
        };

        // Cover art texture + cached dominant accent color
        let (art, art_ver) = {
            let sh = self.shared.lock().unwrap();
            (sh.art.clone(), sh.art_ver)
        };
        let accent = self.media_accent(&art, art_ver);
        let art_tex = art.and_then(|img| {
            let id = format!("art:{art_ver}");
            if !self.textures.contains_key(&id) {
                self.textures.retain(|k, _| !k.starts_with("art:"));
            }
            self.icon_tex(ui.ctx(), &id, &Some(img))
        });

        // ================= 1. AMBIENT NOW PLAYING CARD =================
        let hero_h = 124.0;
        let (hero, _) = ui.allocate_exact_size(vec2(w, hero_h), Sense::hover());
        let p = ui.painter().clone();
        let art_size = 64.0;
        let art_rect = Rect::from_min_size(
            pos2(hero.left() + 14.0, hero.top() + 14.0),
            vec2(art_size, art_size),
        );
        Self::paint_media_hero(&p, hero, 16.0, accent, art_rect.center());
        p.rect_stroke(
            hero,
            CornerRadius::same(16),
            Stroke::new(1.0, accent.gamma_multiply(0.28).lerp_to_gamma(BORDER, 0.45)),
            StrokeKind::Inside,
        );

        // Cover art with center-crop UV (never stretched) or bespoke vinyl fallback
        if let Some(tex) = &art_tex {
            let uv = Self::cover_uv(tex.size_vec2(), art_rect.size());
            egui::Image::new(egui::load::SizedTexture::new(tex.id(), art_rect.size()))
                .uv(uv)
                .corner_radius(CornerRadius::same(11))
                .paint_at(ui, art_rect);
            p.rect_stroke(
                art_rect,
                CornerRadius::same(11),
                Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 38)),
                StrokeKind::Inside,
            );
        } else {
            p.rect_filled(art_rect, CornerRadius::same(11), Color32::from_rgb(16, 18, 26));
            p.rect_stroke(
                art_rect,
                CornerRadius::same(11),
                Stroke::new(1.0, Color32::from_rgb(42, 46, 60)),
                StrokeKind::Inside,
            );
            let vc = art_rect.center();
            p.circle_filled(vc, 22.0, Color32::from_rgb(11, 13, 18));
            p.circle_stroke(vc, 17.0, Stroke::new(0.9, Color32::from_rgb(36, 40, 54)));
            p.circle_stroke(vc, 12.0, Stroke::new(0.8, Color32::from_rgb(30, 34, 46)));
            p.circle_filled(vc, 7.0, accent.gamma_multiply(0.85));
            p.circle_filled(vc, 2.2, Color32::from_rgb(11, 13, 18));
        }

        // Source chip (top-right) with live mini equalizer
        let time = ui.input(|i| i.time) as f32;
        let app_font = semibold(10.0);
        let (app_short, app_w) = fit(&p, &app, app_font.clone(), 86.0);
        let chip_w = 18.0 + app_w + 10.0;
        let chip_r = Rect::from_min_size(
            pos2(hero.right() - 14.0 - chip_w, hero.top() + 14.0),
            vec2(chip_w, 20.0),
        );
        p.rect_filled(
            chip_r,
            CornerRadius::same(10),
            Color32::from_rgba_unmultiplied(8, 10, 16, 150),
        );
        p.rect_stroke(
            chip_r,
            CornerRadius::same(10),
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 24)),
            StrokeKind::Inside,
        );
        for i in 0..3 {
            let bh = if media.playing {
                3.0 + 6.0 * ((time * 7.2 + i as f32 * 1.9).sin() * 0.5 + 0.5)
            } else {
                3.0
            };
            let bx = chip_r.left() + 7.5 + i as f32 * 3.0;
            let bar_r = Rect::from_center_size(pos2(bx, chip_r.center().y), vec2(1.8, bh));
            p.rect_filled(
                bar_r,
                CornerRadius::same(1),
                if media.playing { accent } else { DIM },
            );
        }
        p.text(
            pos2(chip_r.left() + 18.0, chip_r.center().y),
            Align2::LEFT_CENTER,
            &app_short,
            app_font,
            Color32::from_rgba_unmultiplied(226, 232, 246, 210),
        );

        // Track title + artist
        let tx = art_rect.right() + 13.0;
        let title_max_w = (chip_r.left() - 10.0 - tx).max(120.0);
        let title_font = semibold(15.5);
        let (title_fit, _) = fit(&p, &media.title, title_font.clone(), title_max_w);
        p.text(
            pos2(tx, hero.top() + 24.0),
            Align2::LEFT_CENTER,
            title_fit,
            title_font,
            TEXT,
        );

        // Transport controls (bottom-right of upper block)
        let ctrl_y = hero.top() + 58.0;
        let next_r = Rect::from_center_size(pos2(hero.right() - 28.0, ctrl_y), vec2(28.0, 28.0));
        let play_r = Rect::from_center_size(pos2(hero.right() - 64.0, ctrl_y), vec2(34.0, 34.0));
        let prev_r = Rect::from_center_size(pos2(hero.right() - 100.0, ctrl_y), vec2(28.0, 28.0));

        let sub_max_w = (prev_r.left() - 12.0 - tx).max(100.0);
        let sub_font = FontId::proportional(11.5);
        let (sub_fit, _) = fit(&p, &sub_line, sub_font.clone(), sub_max_w);
        p.text(
            pos2(tx, hero.top() + 46.0),
            Align2::LEFT_CENTER,
            sub_fit,
            sub_font,
            Color32::from_rgba_unmultiplied(212, 220, 238, 195),
        );
        p.text(
            pos2(tx, hero.top() + 65.0),
            Align2::LEFT_CENTER,
            if media.playing { "Now Playing" } else { "Paused" },
            FontId::proportional(10.0),
            if media.playing {
                accent.gamma_multiply(0.9)
            } else {
                DIM
            },
        );

        // Draw transport buttons inside hero
        let draw_ctrl = |ui: &mut Ui, r: Rect, icon: Icon, primary: bool, id: &str| -> bool {
            let resp = btn_rect(ui, r, id);
            let c = r.center();
            let sz = r.width();
            let p = ui.painter();
            if primary {
                let fill = if resp.hovered() {
                    Color32::WHITE
                } else {
                    Color32::from_rgb(236, 239, 246)
                };
                p.circle_filled(c, sz * 0.5, fill);
            } else if resp.hovered() {
                p.circle_filled(
                    c,
                    sz * 0.5,
                    Color32::from_rgba_unmultiplied(255, 255, 255, 24),
                );
            }
            let col = if primary {
                Color32::from_rgb(10, 11, 16)
            } else if resp.hovered() {
                Color32::WHITE
            } else {
                Color32::from_rgb(210, 216, 232)
            };
            let s = sz * 0.2;
            let tri = |dir: f32, cx: f32| {
                egui::Shape::convex_polygon(
                    vec![
                        pos2(cx - dir * s * 0.9, c.y - s),
                        pos2(cx - dir * s * 0.9, c.y + s),
                        pos2(cx + dir * s * 0.9, c.y),
                    ],
                    col,
                    Stroke::NONE,
                )
            };
            let bar_fn = |cx: f32, bw: f32| {
                p.rect_filled(
                    Rect::from_center_size(pos2(cx, c.y), vec2(bw, s * 2.0)),
                    CornerRadius::same(1),
                    col,
                );
            };
            match icon {
                Icon::Play => {
                    p.add(tri(1.0, c.x + s * 0.15));
                }
                Icon::Pause => {
                    bar_fn(c.x - s * 0.55, s * 0.58);
                    bar_fn(c.x + s * 0.55, s * 0.58);
                }
                Icon::Next => {
                    p.add(tri(1.0, c.x - s * 0.35));
                    bar_fn(c.x + s * 0.95, s * 0.35);
                }
                Icon::Prev => {
                    p.add(tri(-1.0, c.x + s * 0.35));
                    bar_fn(c.x - s * 0.95, s * 0.35);
                }
            }
            resp.clicked()
        };

        if draw_ctrl(ui, prev_r, Icon::Prev, false, "med_prev") {
            let _ = self.tx.send(media::Cmd::Prev);
        }
        let play_icon = if media.playing {
            Icon::Pause
        } else {
            Icon::Play
        };
        if draw_ctrl(ui, play_r, play_icon, true, "med_play") {
            let _ = self.tx.send(media::Cmd::PlayPause);
        }
        if draw_ctrl(ui, next_r, Icon::Next, false, "med_next") {
            let _ = self.tx.send(media::Cmd::Next);
        }

        // ---- Interactive Scrubber Bar inside bottom of hero ----
        let scrub_rect = Rect::from_min_max(
            pos2(hero.left() + 14.0, hero.top() + 87.0),
            pos2(hero.right() - 14.0, hero.top() + 103.0),
        );
        let scrub_resp = ui
            .interact(scrub_rect, ui.id().with("med_scrub"), Sense::click_and_drag())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        let frac = if media.dur_ms > 0 {
            (pos_ms as f32 / media.dur_ms as f32).clamp(0.0, 1.0)
        } else {
            0.0
        };
        let scrubbing = scrub_resp.hovered() || scrub_resp.dragged();
        let track_h = if scrubbing { 5.5 } else { 4.0 };
        let track = Rect::from_center_size(scrub_rect.center(), vec2(scrub_rect.width(), track_h));
        let p = ui.painter().clone();
        p.rect_filled(
            track,
            CornerRadius::same(3),
            Color32::from_rgba_unmultiplied(255, 255, 255, 26),
        );
        let fill_w = track.width() * frac;
        if fill_w > 0.5 {
            p.rect_filled(
                Rect::from_min_size(track.min, vec2(fill_w, track_h)),
                CornerRadius::same(3),
                accent.lerp_to_gamma(Color32::WHITE, 0.35),
            );
        }
        if scrubbing {
            let knob_x = track.left() + fill_w;
            p.circle_filled(pos2(knob_x, track.center().y), 5.2, Color32::WHITE);
            if media.dur_ms > 0 {
                if let Some(hp) = scrub_resp.hover_pos() {
                    let hf = ((hp.x - track.left()) / track.width()).clamp(0.0, 1.0);
                    let h_ms = (hf * media.dur_ms as f32) as u64;
                    let tip = media::fmt_time(h_ms);
                    let tip_x = hp.x.clamp(track.left() + 18.0, track.right() - 18.0);
                    let tip_r = Rect::from_center_size(pos2(tip_x, track.top() - 12.0), vec2(36.0, 16.0));
                    p.rect_filled(
                        tip_r,
                        CornerRadius::same(5),
                        Color32::from_rgba_unmultiplied(10, 12, 18, 225),
                    );
                    p.text(
                        tip_r.center(),
                        Align2::CENTER_CENTER,
                        tip,
                        semibold(9.5),
                        TEXT,
                    );
                }
            }
        }
        if (scrub_resp.clicked() || scrub_resp.drag_stopped()) && media.dur_ms > 0 {
            if let Some(pt) = scrub_resp.interact_pointer_pos() {
                let f = ((pt.x - track.left()) / track.width()).clamp(0.0, 1.0);
                let _ = self
                    .tx
                    .send(media::Cmd::Seek((f * media.dur_ms as f32) as u64));
            }
        }

        // Elapsed & total time
        p.text(
            pos2(track.left(), hero.bottom() - 11.5),
            Align2::LEFT_CENTER,
            media::fmt_time(pos_ms),
            FontId::proportional(10.0),
            Color32::from_rgba_unmultiplied(200, 208, 226, 185),
        );
        p.text(
            pos2(track.right(), hero.bottom() - 11.5),
            Align2::RIGHT_CENTER,
            media::fmt_time(media.dur_ms),
            FontId::proportional(10.0),
            Color32::from_rgba_unmultiplied(166, 174, 194, 175),
        );

        // ================= 2. LYRICS / AMBIENT STAGE =================
        ui.add_space(8.0);
        let (area, _) = ui.allocate_exact_size(vec2(w, 54.0), Sense::hover());
        let p = ui.painter().clone();
        p.rect_filled(area, CornerRadius::same(12), Color32::from_rgb(10, 12, 18));
        p.rect_stroke(
            area,
            CornerRadius::same(12),
            Stroke::new(1.0, Color32::from_rgb(28, 32, 44)),
            StrokeKind::Inside,
        );
        let lead = self.lead_for(media);
        Self::draw_lyrics(
            ui,
            area.shrink2(vec2(10.0, 4.0)),
            lyrics,
            media::shift(pos_ms, lead),
            media.playing,
            accent,
        );
        self.offset_control(ui, area.shrink2(vec2(8.0, 4.0)), media);
    }

    fn draw_lyrics(
        ui: &Ui,
        area: Rect,
        lyrics: &Lyrics,
        pos_ms: u64,
        playing: bool,
        accent: Color32,
    ) {
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
                        ellipsize(&lines[i as usize].1, 60),
                        if focus > 0.6 {
                            semibold(13.0)
                        } else {
                            FontId::proportional(12.0)
                        },
                        color,
                    );
                }
            }
            Lyrics::Loading => {
                painter.text(
                    center,
                    Align2::CENTER_CENTER,
                    "Looking for synced lyrics…",
                    FontId::proportional(11.5),
                    MUTED,
                );
            }
            Lyrics::NotFound | Lyrics::Idle => {
                // Subtle ambient waveform bars + quiet label so the stage feels alive
                let time = ui.input(|i| i.time) as f32;
                let bars = 17usize;
                let gap = 5.5;
                let total_w = (bars as f32 - 1.0) * gap;
                let x0 = center.x - total_w * 0.5;
                let wy = center.y - 7.0;
                for i in 0..bars {
                    let norm = (i as f32 / (bars - 1) as f32 - 0.5).abs() * 2.0; // 0 at center, 1 at edge
                    let env = (1.0 - norm * norm).max(0.2);
                    let h = if playing {
                        3.0 + 13.0
                            * env
                            * ((time * 5.2 + i as f32 * 0.85).sin() * 0.35
                                + (time * 3.1 - i as f32 * 1.3).cos() * 0.25
                                + 0.5)
                    } else {
                        3.0
                    };
                    let bar_r = Rect::from_center_size(pos2(x0 + i as f32 * gap, wy), vec2(2.2, h));
                    painter.rect_filled(
                        bar_r,
                        CornerRadius::same(1),
                        accent.gamma_multiply(if playing { 0.38 + 0.38 * env } else { 0.22 }),
                    );
                }
                painter.text(
                    pos2(center.x, center.y + 12.0),
                    Align2::CENTER_CENTER,
                    "No synced lyrics available",
                    FontId::proportional(10.0),
                    DIM,
                );
            }
        }
    }

    fn paint_sys_card(p: &egui::Painter, rect: Rect, radius: f32, tint: Color32) {
        use std::f32::consts::{FRAC_PI_2, PI};
        let r = radius.min(rect.width() * 0.5).min(rect.height() * 0.5);
        let segs = 7usize;
        let corners = [
            (pos2(rect.right() - r, rect.top() + r), -FRAC_PI_2, 0.0),
            (pos2(rect.right() - r, rect.bottom() - r), 0.0, FRAC_PI_2),
            (pos2(rect.left() + r, rect.bottom() - r), FRAC_PI_2, PI),
            (pos2(rect.left() + r, rect.top() + r), PI, PI + FRAC_PI_2),
        ];
        let light = pos2(rect.right() - 28.0, rect.top() + 18.0);
        let top_c = Color32::from_rgb(15, 18, 26);
        let bot_c = Color32::from_rgb(10, 12, 18);
        let sample = |pt: egui::Pos2| {
            let ty = ((pt.y - rect.top()) / rect.height().max(1.0)).clamp(0.0, 1.0);
            let base = top_c.lerp_to_gamma(bot_c, ty);
            let dist = (pt - light).length() / 155.0;
            let glow = (1.0 - dist).clamp(0.0, 1.0).powi(2) * 0.22;
            base.lerp_to_gamma(tint, glow)
        };
        let mut perim = Vec::with_capacity(corners.len() * (segs + 1));
        for (center, a0, a1) in corners {
            for s in 0..=segs {
                let a = a0 + (a1 - a0) * (s as f32 / segs as f32);
                perim.push(pos2(center.x + r * a.cos(), center.y + r * a.sin()));
            }
        }
        let mut mesh = egui::Mesh::default();
        let c = rect.center();
        mesh.colored_vertex(c, sample(c));
        for &pt in &perim {
            mesh.colored_vertex(pt, sample(pt));
        }
        let n = perim.len() as u32;
        for i in 0..n {
            let next = if i + 1 == n { 1 } else { i + 2 };
            mesh.add_triangle(0, i + 1, next);
        }
        p.add(egui::Shape::mesh(mesh));
    }

    fn draw_sys_sparkline(
        p: &egui::Painter,
        r: Rect,
        data: &std::collections::VecDeque<f32>,
        color: Color32,
        hover_x: Option<f32>,
    ) -> Option<f32> {
        // Subtle 50% baseline
        let mid_y = r.center().y;
        p.line_segment(
            [pos2(r.left(), mid_y), pos2(r.right(), mid_y)],
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 12)),
        );
        p.line_segment(
            [pos2(r.left(), r.bottom()), pos2(r.right(), r.bottom())],
            Stroke::new(1.0, Color32::from_rgba_unmultiplied(255, 255, 255, 16)),
        );
        if data.len() < 2 {
            return None;
        }
        let n = data.len();
        let step = r.width() / (HIST - 1) as f32;
        let pts: Vec<egui::Pos2> = data
            .iter()
            .enumerate()
            .map(|(i, v)| {
                pos2(
                    r.right() - (n - 1 - i) as f32 * step,
                    r.bottom() - v.clamp(0.0, 1.0) * r.height(),
                )
            })
            .collect();

        let mut mesh = egui::Mesh::default();
        for pt in &pts {
            mesh.colored_vertex(*pt, color.gamma_multiply(0.28));
            mesh.colored_vertex(pos2(pt.x, r.bottom()), Color32::TRANSPARENT);
        }
        for i in 0..pts.len() - 1 {
            let k = (i * 2) as u32;
            mesh.add_triangle(k, k + 1, k + 2);
            mesh.add_triangle(k + 1, k + 2, k + 3);
        }
        p.add(egui::Shape::mesh(mesh));
        p.add(egui::Shape::line(
            pts.clone(),
            Stroke::new(3.6, color.gamma_multiply(0.14)),
        ));
        p.add(egui::Shape::line(pts.clone(), Stroke::new(1.7, color)));

        if let Some(last) = pts.last() {
            p.circle_filled(*last, 2.6, color);
            p.circle_filled(*last, 1.1, Color32::WHITE);
        }

        // Optional interactive hover readout
        if let Some(hx) = hover_x {
            if let Some((idx, best_pt)) = pts
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| (a.x - hx).abs().total_cmp(&(b.x - hx).abs()))
            {
                if (best_pt.x - hx).abs() <= step.max(10.0) {
                    p.line_segment(
                        [pos2(best_pt.x, r.top()), pos2(best_pt.x, r.bottom())],
                        Stroke::new(1.0, color.gamma_multiply(0.45)),
                    );
                    p.circle_filled(*best_pt, 3.2, Color32::WHITE);
                    return data.get(idx).copied();
                }
            }
        }
        None
    }

    pub(crate) fn system_section(&self, ui: &mut Ui) {
        ui.spacing_mut().item_spacing.y = 0.0;
        let w = ui.available_width();
        let gb = 1_073_741_824.0;
        let cpu = self.cpu_hist.back().copied().unwrap_or(0.0);
        let mem = self.mem_hist.back().copied().unwrap_or(0.0);
        let used = self.sys.used_memory() as f64 / gb;
        let total = self.sys.total_memory() as f64 / gb;
        let free = (total - used).max(0.0);
        let cores = self.sys.cpus().len();
        let cpu_peak = self
            .cpu_hist
            .iter()
            .copied()
            .fold(cpu, f32::max);

        let up_s = sysinfo::System::uptime();
        let up_txt = {
            let d = up_s / 86_400;
            let h = (up_s % 86_400) / 3600;
            let m = (up_s % 3600) / 60;
            if d > 0 {
                format!("Up {d}d {h}h")
            } else if h > 0 {
                format!("Up {h}h {m:02}m")
            } else {
                format!("Up {m}m")
            }
        };

        let cpu_col = if cpu > 0.85 {
            WARN
        } else if cpu > 0.65 {
            Color32::from_rgb(248, 190, 92)
        } else {
            Color32::from_rgb(108, 182, 255)
        };
        let mem_col = if mem > 0.88 {
            WARN
        } else if mem > 0.78 {
            Color32::from_rgb(245, 178, 98)
        } else {
            Color32::from_rgb(162, 138, 255)
        };

        let ptr = ui.input(|i| i.pointer.hover_pos());

        // ================= 1. TOP TELEMETRY PAIR: CPU + MEMORY =================
        let gap = 8.0;
        let cw = (w - gap) * 0.5;
        let top_h = 118.0;
        let (top_row, _) = ui.allocate_exact_size(vec2(w, top_h), Sense::hover());
        let p = ui.painter().clone();

        // ---- CPU CARD ----
        let cpu_r = Rect::from_min_size(top_row.min, vec2(cw, top_h));
        Self::paint_sys_card(&p, cpu_r, 14.0, cpu_col);
        p.rect_stroke(
            cpu_r,
            CornerRadius::same(14),
            Stroke::new(1.0, Color32::from_rgb(34, 40, 54)),
            StrokeKind::Inside,
        );

        // Mini CPU chip glyph
        let ic = pos2(cpu_r.left() + 18.0, cpu_r.top() + 20.0);
        p.rect_stroke(
            Rect::from_center_size(ic, vec2(8.0, 8.0)),
            CornerRadius::same(2),
            Stroke::new(1.2, cpu_col),
            StrokeKind::Inside,
        );
        p.rect_filled(
            Rect::from_center_size(ic, vec2(3.0, 3.0)),
            CornerRadius::same(1),
            cpu_col,
        );
        p.text(
            pos2(cpu_r.left() + 28.0, cpu_r.top() + 17.0),
            Align2::LEFT_CENTER,
            "CPU",
            semibold(12.0),
            TEXT,
        );
        p.text(
            pos2(cpu_r.left() + 28.0, cpu_r.top() + 30.0),
            Align2::LEFT_CENTER,
            format!("{cores} threads"),
            FontId::proportional(9.5),
            DIM,
        );

        let cpu_chart = Rect::from_min_max(
            pos2(cpu_r.left() + 12.0, cpu_r.top() + 44.0),
            pos2(cpu_r.right() - 12.0, cpu_r.bottom() - 25.0),
        );
        let cpu_hx = ptr.filter(|pos| cpu_chart.contains(*pos)).map(|pos| pos.x);
        let cpu_hov = Self::draw_sys_sparkline(&p, cpu_chart, &self.cpu_hist, cpu_col, cpu_hx);
        let cpu_shown = cpu_hov.unwrap_or(cpu);

        p.text(
            pos2(cpu_r.right() - 14.0, cpu_r.top() + 23.0),
            Align2::RIGHT_CENTER,
            format!("{:.0}%", cpu_shown * 100.0),
            semibold(21.0),
            if cpu_hov.is_some() { cpu_col } else { TEXT },
        );
        p.text(
            pos2(cpu_r.left() + 13.0, cpu_r.bottom() - 12.5),
            Align2::LEFT_CENTER,
            format!("Peak {:.0}%", cpu_peak * 100.0),
            FontId::proportional(10.0),
            MUTED,
        );
        p.text(
            pos2(cpu_r.right() - 13.0, cpu_r.bottom() - 12.5),
            Align2::RIGHT_CENTER,
            up_txt,
            FontId::proportional(10.0),
            DIM,
        );

        // ---- MEMORY CARD ----
        let mem_r = Rect::from_min_size(pos2(cpu_r.right() + gap, top_row.top()), vec2(cw, top_h));
        Self::paint_sys_card(&p, mem_r, 14.0, mem_col);
        p.rect_stroke(
            mem_r,
            CornerRadius::same(14),
            Stroke::new(1.0, Color32::from_rgb(34, 40, 54)),
            StrokeKind::Inside,
        );

        // Mini RAM module glyph
        let mc = pos2(mem_r.left() + 18.0, mem_r.top() + 20.0);
        p.rect_stroke(
            Rect::from_center_size(mc, vec2(10.0, 6.5)),
            CornerRadius::same(1),
            Stroke::new(1.2, mem_col),
            StrokeKind::Inside,
        );
        for dx in [-2.5f32, 0.0, 2.5] {
            p.line_segment(
                [pos2(mc.x + dx, mc.y + 3.2), pos2(mc.x + dx, mc.y + 5.2)],
                Stroke::new(1.1, mem_col),
            );
        }
        p.text(
            pos2(mem_r.left() + 28.0, mem_r.top() + 17.0),
            Align2::LEFT_CENTER,
            "Memory",
            semibold(12.0),
            TEXT,
        );
        p.text(
            pos2(mem_r.left() + 28.0, mem_r.top() + 30.0),
            Align2::LEFT_CENTER,
            format!("{free:.1} GB free"),
            FontId::proportional(9.5),
            DIM,
        );

        // Utilization gauge bar + compact trend sparkline
        let bar_r = Rect::from_min_max(
            pos2(mem_r.left() + 13.0, mem_r.top() + 45.0),
            pos2(mem_r.right() - 13.0, mem_r.top() + 51.0),
        );
        p.rect_filled(bar_r, CornerRadius::same(3), Color32::from_rgb(26, 30, 42));
        let fill_w = (bar_r.width() * mem.clamp(0.0, 1.0)).max(3.0);
        p.rect_filled(
            Rect::from_min_size(bar_r.min, vec2(fill_w, bar_r.height())),
            CornerRadius::same(3),
            mem_col,
        );

        let mem_chart = Rect::from_min_max(
            pos2(mem_r.left() + 12.0, mem_r.top() + 60.0),
            pos2(mem_r.right() - 12.0, mem_r.bottom() - 25.0),
        );
        let mem_hx = ptr.filter(|pos| mem_chart.contains(*pos)).map(|pos| pos.x);
        let mem_hov = Self::draw_sys_sparkline(&p, mem_chart, &self.mem_hist, mem_col, mem_hx);
        let mem_shown = mem_hov.unwrap_or(mem);

        p.text(
            pos2(mem_r.right() - 14.0, mem_r.top() + 23.0),
            Align2::RIGHT_CENTER,
            format!("{:.0}%", mem_shown * 100.0),
            semibold(21.0),
            if mem_hov.is_some() { mem_col } else { TEXT },
        );
        p.text(
            pos2(mem_r.left() + 13.0, mem_r.bottom() - 12.5),
            Align2::LEFT_CENTER,
            format!("{used:.1} / {total:.1} GB"),
            FontId::proportional(10.0),
            MUTED,
        );
        p.text(
            pos2(mem_r.right() - 13.0, mem_r.bottom() - 12.5),
            Align2::RIGHT_CENTER,
            "60s trend",
            FontId::proportional(10.0),
            DIM,
        );

        // ================= 2. FULL-WIDTH STORAGE DECK =================
        ui.add_space(8.0);
        let st_h = 70.0;
        let (st_r, _) = ui.allocate_exact_size(vec2(w, st_h), Sense::hover());
        let p = ui.painter().clone();
        p.rect_filled(st_r, CornerRadius::same(14), Color32::from_rgb(10, 12, 18));
        p.rect_stroke(
            st_r,
            CornerRadius::same(14),
            Stroke::new(1.0, Color32::from_rgb(28, 33, 45)),
            StrokeKind::Inside,
        );

        // Drive glyph + header
        let dc = pos2(st_r.left() + 17.0, st_r.top() + 16.0);
        p.rect_stroke(
            Rect::from_center_size(dc, vec2(10.0, 7.0)),
            CornerRadius::same(2),
            Stroke::new(1.2, MUTED),
            StrokeKind::Inside,
        );
        p.circle_filled(pos2(dc.x + 2.5, dc.y), 1.0, GREEN);
        p.text(
            pos2(st_r.left() + 27.0, st_r.top() + 16.0),
            Align2::LEFT_CENTER,
            "Storage",
            semibold(11.5),
            TEXT,
        );

        if self.disks.is_empty() {
            p.text(
                pos2(st_r.center().x, st_r.bottom() - 22.0),
                Align2::CENTER_CENTER,
                "No disk information available",
                FontId::proportional(11.0),
                DIM,
            );
            return;
        }

        let tot_used: f64 = self.disks.iter().map(|d| d.used_gb).sum();
        let tot_cap: f64 = self.disks.iter().map(|d| d.total_gb).sum();
        let n = self.disks.len();
        p.text(
            pos2(st_r.right() - 14.0, st_r.top() + 16.0),
            Align2::RIGHT_CENTER,
            format!(
                "{n} drive{} · {} / {}",
                if n == 1 { "" } else { "s" },
                fmt_gb(tot_used),
                fmt_gb(tot_cap)
            ),
            FontId::proportional(10.0),
            MUTED,
        );

        let shown = n.min(3);
        let pad_x = 14.0;
        let inner_w = st_r.width() - pad_x * 2.0;
        let seg_gap = 16.0;
        let seg_w = (inner_w - seg_gap * (shown as f32 - 1.0)) / shown as f32;
        for (i, d) in self.disks.iter().take(shown).enumerate() {
            let x0 = st_r.left() + pad_x + i as f32 * (seg_w + seg_gap);
            let x1 = x0 + seg_w;
            if i > 0 {
                let dx = x0 - seg_gap * 0.5;
                p.line_segment(
                    [pos2(dx, st_r.top() + 31.0), pos2(dx, st_r.bottom() - 11.0)],
                    Stroke::new(1.0, Color32::from_rgb(28, 32, 44)),
                );
            }
            let frac = (d.used_gb / d.total_gb.max(0.001)).clamp(0.0, 1.0) as f32;
            let free_gb = (d.total_gb - d.used_gb).max(0.0);
            let col = if frac > 0.88 {
                WARN
            } else if frac > 0.75 {
                Color32::from_rgb(245, 186, 92)
            } else {
                GREEN
            };

            // Drive badge + capacity
            let badge_r = Rect::from_min_size(pos2(x0, st_r.top() + 30.0), vec2(24.0, 16.0));
            p.rect_filled(badge_r, CornerRadius::same(4), Color32::from_rgb(22, 26, 37));
            p.text(
                badge_r.center(),
                Align2::CENTER_CENTER,
                &d.name,
                semibold(10.0),
                TEXT,
            );
            p.text(
                pos2(badge_r.right() + 7.0, badge_r.center().y),
                Align2::LEFT_CENTER,
                format!("{} / {}", fmt_gb(d.used_gb), fmt_gb(d.total_gb)),
                FontId::proportional(10.0),
                MUTED,
            );
            p.text(
                pos2(x1, badge_r.center().y),
                Align2::RIGHT_CENTER,
                format!("{} free", fmt_gb(free_gb)),
                semibold(10.0),
                if frac > 0.88 { WARN } else { col },
            );

            // Drive bar
            let track = Rect::from_min_size(pos2(x0, st_r.bottom() - 15.0), vec2(seg_w, 5.0));
            p.rect_filled(track, CornerRadius::same(3), Color32::from_rgb(24, 28, 39));
            p.rect_filled(
                Rect::from_min_size(track.min, vec2((track.width() * frac).max(3.0), 5.0)),
                CornerRadius::same(3),
                col,
            );
        }
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

        // ---- no data yet: loading / error ----
        let Some(day) = today_d else {
            let mut retry = false;
            ui.add_space(18.0);
            ui.vertical_centered(|ui| {
                let (title, sub) = match &status {
                    crate::prayer::PrayerStatus::Error(e) => {
                        ("Schedule unavailable".to_string(), ellipsize(e, 70))
                    }
                    crate::prayer::PrayerStatus::Loading => (
                        "Loading prayer times…".to_string(),
                        "Source: equran.id (Kemenag RI)".to_string(),
                    ),
                    _ => (
                        "Detecting your location…".to_string(),
                        "Using device location, IP fallback".to_string(),
                    ),
                };
                ui.label(egui::RichText::new(title).font(semibold(14.0)).color(TEXT));
                ui.add_space(2.0);
                ui.label(small(sub));
                if matches!(status, crate::prayer::PrayerStatus::Error(_)) {
                    ui.add_space(8.0);
                    let (r, _) = ui.allocate_exact_size(vec2(92.0, 26.0), Sense::hover());
                    let p = ui.painter().clone();
                    if text_btn(ui, &p, r, "Retry", "pr_retry", true, true) {
                        retry = true;
                    }
                }
            });
            if retry {
                {
                    let mut s = self.prayer.lock().unwrap();
                    s.retry = true;
                    s.status = crate::prayer::PrayerStatus::Locating;
                }
                {
                    let mut w = self.weather.lock().unwrap();
                    w.retry = true;
                    w.status = crate::weather::WeatherStatus::Locating;
                }
            }
            return;
        };

        // ---- calculate next prayer ----
        let now = Local::now();
        let secs = now.num_seconds_from_midnight() as i64;
        let times: [i64; 8] = day.t.map(|m| m as i64 * 60);

        let next_idx = MAIN_PRAYERS.iter().copied().find(|&i| times[i] > secs);
        let tomorrow = next_idx.is_none();
        let (next_i, next_at) = match next_idx {
            Some(i) => (i, times[i]),
            None => {
                // past Isha -> tomorrow's Fajr
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

        // =============== HEADER: LOCATION + DATE ===============
        ui.spacing_mut().item_spacing.y = 0.0;
        let w = ui.available_width();
        let col = prayer_color(next_i);
        let surface = Color32::from_rgb(12, 13, 18);
        let surface_dim = Color32::from_rgb(7, 8, 11);
        let border_soft = Color32::from_rgb(34, 37, 47);

        let (hdr, _) = ui.allocate_exact_size(vec2(w, 18.0), Sense::hover());
        let p = ui.painter().clone();
        let is_updating = matches!(
            status,
            crate::prayer::PrayerStatus::Locating | crate::prayer::PrayerStatus::Loading
        );
        let loc_txt = if is_updating {
            "Locating…".to_string()
        } else {
            ellipsize(kab.trim_start_matches("Kota "), 26)
        };
        let lf = FontId::proportional(11.0);
        let lw = text_w(&p, &loc_txt, &lf);
        let loc_rect = Rect::from_min_max(
            pos2(hdr.left(), hdr.top() - 2.0),
            pos2(hdr.left() + 18.0 + lw + 6.0, hdr.bottom() + 2.0),
        );
        let lr =
            btn_rect(ui, loc_rect, "pr_loc").on_hover_text("Click to re-detect location & refresh");
        let lcol = if is_updating {
            col
        } else if lr.hovered() {
            TEXT
        } else {
            MUTED
        };
        let pin = pos2(hdr.left() + 6.0, hdr.center().y);
        p.circle_stroke(pin, 2.8, Stroke::new(1.2, lcol));
        p.circle_filled(pin, 1.0, lcol);
        p.text(
            pos2(hdr.left() + 15.0, hdr.center().y),
            Align2::LEFT_CENTER,
            &loc_txt,
            lf,
            lcol,
        );
        p.text(
            pos2(hdr.right(), hdr.center().y),
            Align2::RIGHT_CENTER,
            now.format("%a, %d %b").to_string(),
            FontId::proportional(10.5),
            MUTED,
        );
        if lr.clicked() {
            {
                let mut s = self.prayer.lock().unwrap();
                s.retry = true;
                s.status = crate::prayer::PrayerStatus::Locating;
            }
            {
                let mut w = self.weather.lock().unwrap();
                w.retry = true;
                w.status = crate::weather::WeatherStatus::Locating;
            }
        }

        // =============== HERO: NEXT PRAYER FOCUS ===============
        ui.add_space(8.0);
        let hero_h = 78.0;
        let (hero, _) = ui.allocate_exact_size(vec2(w, hero_h), Sense::hover());
        let p = ui.painter().clone();
        p.rect_filled(hero, CornerRadius::same(14), surface);
        p.rect_stroke(
            hero,
            CornerRadius::same(14),
            Stroke::new(1.0, col.gamma_multiply(0.38)),
            StrokeKind::Inside,
        );

        let ic = pos2(hero.left() + 28.0, hero.top() + 34.0);
        let ic_bg = surface.lerp_to_gamma(col, 0.16);
        p.circle_filled(ic, 17.0, ic_bg);
        prayer_icon(&p, ic, next_i, col, ic_bg);

        p.text(
            pos2(hero.left() + 56.0, hero.top() + 20.0),
            Align2::LEFT_CENTER,
            format!(
                "Next · {}{}",
                PRAYER_NAMES[next_i],
                if tomorrow { " (tomorrow)" } else { "" }
            ),
            FontId::proportional(11.0),
            MUTED,
        );
        p.text(
            pos2(hero.left() + 56.0, hero.top() + 43.0),
            Align2::LEFT_CENTER,
            cd,
            semibold(24.0),
            TEXT,
        );

        p.text(
            pos2(hero.right() - 16.0, hero.top() + 26.0),
            Align2::RIGHT_CENTER,
            fmt_hm(shown(next_i)),
            semibold(21.0),
            col,
        );
        let cur_label = match cur {
            Some(c_idx) => format!("Current · {}", PRAYER_NAMES[c_idx]),
            None => "After Isha".to_string(),
        };
        p.text(
            pos2(hero.right() - 16.0, hero.top() + 46.5),
            Align2::RIGHT_CENTER,
            cur_label,
            FontId::proportional(10.5),
            MUTED,
        );

        let track = Rect::from_min_max(
            pos2(hero.left() + 16.0, hero.bottom() - 11.0),
            pos2(hero.right() - 16.0, hero.bottom() - 8.0),
        );
        p.rect_filled(track, CornerRadius::same(2), TRACK);
        let filled_w = (track.width() * frac).max(3.0);
        p.rect_filled(
            Rect::from_min_size(track.min, vec2(filled_w, track.height())),
            CornerRadius::same(2),
            col,
        );
        p.circle_filled(
            pos2(track.left() + filled_w, track.center().y),
            2.6,
            TEXT,
        );

        // =============== 5 WAJIB PRAYERS ===============
        ui.add_space(8.0);
        let gap = 6.0;
        let cw = (w - gap * 4.0) / 5.0;
        let ch = 62.0;
        let (row, _) = ui.allocate_exact_size(vec2(w, ch), Sense::hover());
        for (n, &i) in MAIN_PRAYERS.iter().enumerate() {
            let r = Rect::from_min_size(
                pos2(row.left() + n as f32 * (cw + gap), row.top()),
                vec2(cw, ch),
            );
            let is_next = i == next_i;
            let is_cur = cur == Some(i);
            let passed = !is_next && !is_cur && times[i] <= secs;

            let bgc = if is_next {
                surface.lerp_to_gamma(col, 0.14)
            } else if is_cur {
                Color32::from_rgb(18, 20, 28)
            } else if passed {
                surface_dim
            } else {
                surface
            };
            let stroke_col = if is_next {
                col.gamma_multiply(0.85)
            } else if is_cur {
                Color32::from_rgb(62, 66, 82)
            } else if passed {
                Color32::from_rgb(26, 28, 36)
            } else {
                border_soft
            };
            p.rect_filled(r, CornerRadius::same(11), bgc);
            p.rect_stroke(
                r,
                CornerRadius::same(11),
                Stroke::new(if is_next { 1.3 } else { 1.0 }, stroke_col),
                StrokeKind::Inside,
            );
            let ic_col = if is_next {
                col
            } else if is_cur {
                TEXT
            } else if passed {
                DIM
            } else {
                MUTED
            };
            prayer_icon(&p, pos2(r.center().x, r.top() + 16.0), i, ic_col, bgc);
            p.text(
                pos2(r.center().x, r.top() + 34.0),
                Align2::CENTER_CENTER,
                PRAYER_NAMES[i],
                FontId::proportional(10.0),
                if passed {
                    DIM
                } else if is_next || is_cur {
                    TEXT
                } else {
                    MUTED
                },
            );
            p.text(
                pos2(r.center().x, r.top() + 49.0),
                Align2::CENTER_CENTER,
                fmt_hm(shown(i)),
                semibold(12.5),
                if passed {
                    DIM
                } else if is_next {
                    col
                } else {
                    TEXT
                },
            );
            if is_cur {
                p.circle_filled(pos2(r.right() - 8.5, r.top() + 8.5), 2.3, col);
            }
        }

        // =============== SECONDARY TIMES: IMSAK · SUNRISE · DHUHA ===============
        ui.add_space(8.0);
        let (bar, _) = ui.allocate_exact_size(vec2(w, 24.0), Sense::hover());
        p.rect_filled(bar, CornerRadius::same(8), surface_dim);
        p.rect_stroke(
            bar,
            CornerRadius::same(8),
            Stroke::new(1.0, Color32::from_rgb(28, 31, 40)),
            StrokeKind::Inside,
        );
        let seg_w = w / 3.0;
        for (j, idx) in [0usize, 2, 3].into_iter().enumerate() {
            let x_l = bar.left() + j as f32 * seg_w;
            if j > 0 {
                p.line_segment(
                    [pos2(x_l, bar.top() + 5.0), pos2(x_l, bar.bottom() - 5.0)],
                    Stroke::new(1.0, Color32::from_rgb(30, 33, 42)),
                );
            }
            let cx = x_l + seg_w * 0.5;
            let name = PRAYER_NAMES[idx];
            let time_s = fmt_hm(day.t[idx]);
            let nf = FontId::proportional(10.5);
            let tf = semibold(11.0);
            let nw = text_w(&p, name, &nf);
            let tw = text_w(&p, &time_s, &tf);
            let start_x = cx - (nw + 6.0 + tw) * 0.5;
            p.text(
                pos2(start_x, bar.center().y),
                Align2::LEFT_CENTER,
                name,
                nf,
                MUTED,
            );
            p.text(
                pos2(start_x + nw + 6.0, bar.center().y),
                Align2::LEFT_CENTER,
                time_s,
                tf,
                TEXT,
            );
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
        ui.painter().rect_stroke(
            task,
            CornerRadius::same(FIELD_R),
            Stroke::new(
                1.0,
                if focused {
                    ACCENT.gamma_multiply(0.65)
                } else if hovered {
                    BORDER
                } else {
                    Color32::from_rgb(30, 32, 40)
                },
            ),
            StrokeKind::Inside,
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
                            .hint_text(egui::RichText::new("Add a new task…").color(DIM)),
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
        let has_due = self.todo_due.is_some();
        ui.painter().rect_filled(
            when,
            CornerRadius::same(FIELD_R),
            field_fill(main.hovered(), has_due),
        );
        ui.painter().rect_stroke(
            when,
            CornerRadius::same(FIELD_R),
            Stroke::new(
                1.0,
                if has_due {
                    ACCENT.gamma_multiply(0.5)
                } else if main.hovered() {
                    BORDER
                } else {
                    Color32::from_rgb(30, 32, 40)
                },
            ),
            StrokeKind::Inside,
        );
        // calendar icon
        let now_ts = Local::now().timestamp();
        let (label, col, icon_col) = match self.todo_due {
            Some(ts) => {
                let c = if ts <= now_ts { WARN } else { ACCENT };
                (fmt_due(ts), c, c)
            }
            None => ("Date & time".to_string(), DIM, MUTED),
        };
        {
            let p = ui.painter();
            let ic = pos2(when.left() + 16.0, when.center().y);
            let box_r = Rect::from_center_size(ic, vec2(11.0, 10.0));
            p.rect_stroke(
                box_r,
                CornerRadius::same(2),
                Stroke::new(1.2, icon_col),
                StrokeKind::Inside,
            );
            p.line_segment(
                [
                    pos2(box_r.left(), box_r.top() + 3.0),
                    pos2(box_r.right(), box_r.top() + 3.0),
                ],
                Stroke::new(1.2, icon_col),
            );
        }
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
        p.rect_filled(area, CornerRadius::same(12), CARD);
        p.rect_stroke(
            area,
            CornerRadius::same(12),
            Stroke::new(1.0, BORDER),
            StrokeKind::Inside,
        );

        let inner = area.shrink2(vec2(10.0, 4.0));
        let (cw, ch) = (25.0, 15.5);
        let cal_w = cw * 7.0;
        let (year, month) = self.pk_view;
        let Some(first) = NaiveDate::from_ymd_opt(year, month, 1) else {
            self.picker_open = false;
            return;
        };

        // Vertical divider between calendar and time controls
        let div_x = inner.left() + cal_w + 12.0;
        p.line_segment(
            [
                pos2(div_x, inner.top() + 4.0),
                pos2(div_x, inner.bottom() - 4.0),
            ],
            Stroke::new(1.0, Color32::from_rgb(28, 30, 38)),
        );

        // ---- calendar header ----
        let hy = inner.top() + 9.0;
        if arrow_btn(
            ui,
            &p,
            Rect::from_center_size(pos2(inner.left() + 10.0, hy), vec2(20.0, 20.0)),
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
            Rect::from_center_size(pos2(inner.left() + cal_w - 10.0, hy), vec2(20.0, 20.0)),
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
            pos2(inner.left() + cal_w / 2.0, hy),
            Align2::CENTER_CENTER,
            first.format("%B %Y").to_string(),
            semibold(11.5),
            TEXT,
        );
        for (i, w) in ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"]
            .iter()
            .enumerate()
        {
            p.text(
                pos2(inner.left() + cw * (i as f32 + 0.5), inner.top() + 27.0),
                Align2::CENTER_CENTER,
                *w,
                FontId::proportional(9.5),
                DIM,
            );
        }

        // ---- day grid ----
        let grid_top = inner.top() + 36.0;
        let offset = first.weekday().num_days_from_monday() as usize;
        for d in 1..=31u32 {
            let Some(date) = NaiveDate::from_ymd_opt(year, month, d) else {
                break;
            };
            let idx = offset + d as usize - 1;
            let cell = Rect::from_min_size(
                pos2(
                    inner.left() + (idx % 7) as f32 * cw,
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
                FontId::proportional(10.5),
                col,
            );
            if resp.map_or(false, |r| r.clicked()) {
                self.pk_date = date;
            }
        }

        // ---- right side: time, quick dates, actions ----
        let rx = div_x + 14.0;
        p.text(
            pos2(rx, inner.top() + 7.0),
            Align2::LEFT_CENTER,
            "Reminder Time",
            FontId::proportional(10.5),
            MUTED,
        );
        stepper(
            ui,
            &p,
            pos2(rx, inner.top() + 18.0),
            &mut self.pk_h,
            24,
            1,
            "h",
        );
        p.text(
            pos2(rx + 96.0 + 8.0, inner.top() + 32.0),
            Align2::CENTER_CENTER,
            ":",
            semibold(13.0),
            DIM,
        );
        stepper(
            ui,
            &p,
            pos2(rx + 96.0 + 16.0, inner.top() + 18.0),
            &mut self.pk_m,
            60,
            5,
            "m",
        );

        let mut quick: Option<NaiveDate> = None;
        let mut x = rx;
        for (label, w, days) in [
            ("Today", 56.0, 0i64),
            ("Tomorrow", 74.0, 1),
            ("Next week", 82.0, 7),
        ] {
            let r = Rect::from_min_size(pos2(x, inner.top() + 56.0), vec2(w, 24.0));
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
        let by = inner.bottom() - 30.0;
        let ts = self.pk_ts();
        let future = ts.map_or(false, |t| t > now.timestamp());
        let right = inner.right();
        let done = Rect::from_min_size(pos2(right - 68.0, by), vec2(68.0, 26.0));
        let clear = Rect::from_min_size(pos2(right - 68.0 - 6.0 - 60.0, by), vec2(60.0, 26.0));
        let (sum, sum_col) = match ts {
            Some(t) if future => (fmt_due(t), ACCENT),
            Some(_) => ("Time has passed".to_string(), WARN),
            None => (String::new(), TEXT),
        };
        p.text(
            pos2(rx, by + 13.0),
            Align2::LEFT_CENTER,
            sum,
            FontId::proportional(11.0),
            sum_col,
        );
        if text_btn(ui, &p, clear, "Clear", "clear", false, true) {
            self.todo_due = None;
            self.picker_open = false;
        }
        if text_btn(ui, &p, done, "Set", "done", true, future) {
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
            let w = ui.available_width();
            let h = ui.available_height().max(96.0);
            let (empty_r, p) = card(ui, w, h);
            let cx = empty_r.center().x;
            let cy = empty_r.center().y;

            let badge_c = pos2(cx, cy - 18.0);
            p.circle_filled(badge_c, 13.0, GREEN.gamma_multiply(0.14));
            p.circle_stroke(badge_c, 13.0, Stroke::new(1.0, GREEN.gamma_multiply(0.35)));
            let s = Stroke::new(1.7, GREEN);
            p.line_segment([badge_c + vec2(-4.5, 0.5), badge_c + vec2(-1.2, 3.8)], s);
            p.line_segment([badge_c + vec2(-1.2, 3.8), badge_c + vec2(5.0, -3.5)], s);

            p.text(
                pos2(cx, cy + 6.0),
                Align2::CENTER_CENTER,
                "All clear — no tasks yet",
                semibold(12.5),
                TEXT,
            );
            p.text(
                pos2(cx, cy + 23.0),
                Align2::CENTER_CENTER,
                "Type a task above or pick a reminder time",
                FontId::proportional(11.0),
                MUTED,
            );
            return;
        }

        let now = Local::now().timestamp();
        let total = items.len();
        let done_count = items.iter().filter(|t| t.done).count();
        let active_count = total.saturating_sub(done_count);

        // ---- Status bar + Clear completed ----
        let mut clear_done = false;
        let (head, _) = ui.allocate_exact_size(vec2(ui.available_width(), 18.0), Sense::hover());
        let dot_col = if active_count == 0 { GREEN } else { ACCENT };
        ui.painter()
            .circle_filled(pos2(head.left() + 6.0, head.center().y), 3.0, dot_col);
        let status_label = if active_count == 0 {
            format!("All {total} completed")
        } else if done_count == 0 {
            format!(
                "{active_count} pending task{}",
                if active_count == 1 { "" } else { "s" }
            )
        } else {
            format!("{active_count} pending · {done_count} done")
        };
        ui.painter().text(
            pos2(head.left() + 14.0, head.center().y),
            Align2::LEFT_CENTER,
            status_label,
            FontId::proportional(10.5),
            MUTED,
        );

        if done_count > 0 {
            let label = "Clear done";
            let gw = text_w(ui.painter(), label, &FontId::proportional(10.5));
            let btn = Rect::from_min_size(
                pos2(head.right() - gw - 14.0, head.top() - 1.0),
                vec2(gw + 14.0, 20.0),
            );
            let r = btn_rect(ui, btn, "todo_clear_done");
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
            if r.clicked() {
                clear_done = true;
            }
        }

        ui.add_space(3.0);

        let added = self.added;
        let mut toggle: Option<u64> = None;
        let mut delete: Option<u64> = None;

        egui::ScrollArea::vertical()
            .max_height(80.0)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                for t in &items {
                    let row_h = 32.0;
                    let (alloc, resp) =
                        ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click());
                    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);

                    // New item: slides in from the right, fades in, and flashes green.
                    let anim = added
                        .filter(|(id, at)| *id == t.id && at.elapsed().as_secs_f32() < 0.45)
                        .map(|(_, at)| at.elapsed().as_secs_f32() / 0.45);
                    let mut p = ui.painter_at(alloc);
                    let rect = match anim {
                        Some(k) => {
                            let e = ease_out(k);
                            p.set_opacity(e);
                            ui.scroll_to_rect(alloc, None);
                            alloc.translate(vec2((1.0 - e) * 60.0, 0.0))
                        }
                        None => alloc,
                    };

                    let overdue = !t.done && t.due.map_or(false, |d| d <= now);
                    let hovered = resp.hovered();
                    let row_bg = if hovered {
                        Color32::from_rgb(21, 23, 30)
                    } else {
                        CARD
                    };
                    p.rect_filled(rect, CornerRadius::same(8), row_bg);

                    if let Some(k) = anim {
                        p.rect_filled(
                            rect,
                            CornerRadius::same(8),
                            GREEN.gamma_multiply(0.18 * (1.0 - k)),
                        );
                    }

                    let border_col = if overdue {
                        WARN.gamma_multiply(if hovered { 0.55 } else { 0.35 })
                    } else if hovered {
                        BORDER
                    } else {
                        Color32::from_rgb(26, 28, 36)
                    };
                    p.rect_stroke(
                        rect,
                        CornerRadius::same(8),
                        Stroke::new(1.0, border_col),
                        StrokeKind::Inside,
                    );

                    let cy = rect.center().y;
                    let cc = pos2(rect.left() + 15.0, cy);
                    if t.done {
                        p.circle_filled(cc, 7.5, GREEN);
                        let s = Stroke::new(1.6, BG);
                        p.line_segment([cc + vec2(-3.0, 0.0), cc + vec2(-0.8, 2.4)], s);
                        p.line_segment([cc + vec2(-0.8, 2.4), cc + vec2(3.5, -2.5)], s);
                    } else {
                        let ring_col = if hovered { ACCENT } else { MUTED };
                        p.circle_filled(cc, 7.5, Color32::from_rgb(10, 11, 15));
                        p.circle_stroke(cc, 7.5, Stroke::new(1.5, ring_col));
                    }

                    let right_pad = if hovered { 30.0 } else { 10.0 };
                    let mut right_edge = rect.right() - right_pad;
                    let x = rect.left() + 32.0;

                    if let Some(d) = t.due {
                        let label = fmt_due(d);
                        let f = FontId::proportional(10.0);
                        let tw = text_w(&p, &label, &f);
                        let pill_w = tw + 12.0;
                        let pill_h = 18.0;
                        let pill_r = Rect::from_min_size(
                            pos2(right_edge - pill_w, cy - pill_h / 2.0),
                            vec2(pill_w, pill_h),
                        );
                        let (bg_c, fg_c) = if t.done {
                            (Color32::from_rgb(24, 26, 33), DIM)
                        } else if overdue {
                            (WARN.gamma_multiply(0.16), WARN)
                        } else {
                            (ACCENT.gamma_multiply(0.14), ACCENT)
                        };
                        p.rect_filled(pill_r, CornerRadius::same(5), bg_c);
                        p.text(pill_r.center(), Align2::CENTER_CENTER, label, f, fg_c);
                        right_edge = pill_r.left() - 8.0;
                    }

                    let text_max = (right_edge - x).max(30.0);
                    let f = if t.done {
                        FontId::proportional(12.0)
                    } else {
                        semibold(12.0)
                    };
                    let (txt, tw) = fit(&p, &one_line(&t.text), f.clone(), text_max);
                    let col = if t.done { DIM } else { TEXT };
                    p.text(pos2(x, cy), Align2::LEFT_CENTER, txt, f, col);
                    if t.done {
                        p.line_segment(
                            [pos2(x, cy), pos2(x + tw, cy)],
                            Stroke::new(1.0, MUTED.gamma_multiply(0.6)),
                        );
                    }

                    if hovered {
                        let del_rect =
                            Rect::from_center_size(pos2(rect.right() - 15.0, cy), vec2(20.0, 20.0));
                        let ptr_in_del = resp
                            .interact_pointer_pos()
                            .or_else(|| ui.input(|i| i.pointer.hover_pos()))
                            .map_or(false, |pt| del_rect.contains(pt));
                        if ptr_in_del {
                            p.rect_filled(
                                del_rect,
                                CornerRadius::same(5),
                                WARN.gamma_multiply(0.18),
                            );
                        }
                        let c = del_rect.center();
                        let s = Stroke::new(1.3, if ptr_in_del { WARN } else { MUTED });
                        p.line_segment([c + vec2(-3.2, -3.2), c + vec2(3.2, 3.2)], s);
                        p.line_segment([c + vec2(-3.2, 3.2), c + vec2(3.2, -3.2)], s);
                    }

                    if resp.clicked() {
                        if let Some(pos) = resp.interact_pointer_pos() {
                            if pos.x > rect.right() - 28.0 {
                                delete = Some(t.id);
                            } else {
                                toggle = Some(t.id);
                            }
                        }
                    }
                }
            });

        if toggle.is_some() || delete.is_some() || clear_done {
            let mut st = self.todos.lock().unwrap();
            if clear_done {
                st.items.retain(|t| !t.done);
            }
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
            {
                let mut w = self.weather.lock().unwrap();
                w.retry = true;
                w.status = crate::weather::WeatherStatus::Locating;
            }
            {
                let mut p = self.prayer.lock().unwrap();
                p.retry = true;
                p.status = crate::prayer::PrayerStatus::Locating;
            }
        }
    }

    pub(crate) fn antigravity_section(&mut self, ui: &mut Ui) {
        ui.spacing_mut().item_spacing.y = 8.0;

        let (status, quota, is_loading) = {
            let st = self.antigravity.lock().unwrap();
            (st.status.clone(), st.quota.clone(), st.is_loading)
        };

        let w = ui.available_width();

        if let Some(q) = quota {
            // 1. Compact Header Card (42px)
            let h_head = 42.0;
            let (r_head, p_head) = card(ui, w, h_head);

            // Sparkle diamond badge on left
            let ic = pos2(r_head.left() + 18.0, r_head.center().y);
            p_head.circle_filled(ic, 10.0, ACCENT.gamma_multiply(0.14));
            let pts = vec![
                pos2(ic.x, ic.y - 4.8),
                pos2(ic.x + 3.8, ic.y),
                pos2(ic.x, ic.y + 4.8),
                pos2(ic.x - 3.8, ic.y),
            ];
            p_head.add(egui::Shape::closed_line(pts, Stroke::new(1.3, ACCENT)));

            let title_font = semibold(13.0);
            let title_x = r_head.left() + 34.0;
            p_head.text(
                pos2(title_x, r_head.center().y),
                Align2::LEFT_CENTER,
                "Models & Quota",
                title_font.clone(),
                TEXT,
            );
            let title_w = text_w(&p_head, "Models & Quota", &title_font);

            let acc_text = format!("·  {}", q.account);
            p_head.text(
                pos2(title_x + title_w + 8.0, r_head.center().y),
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
                    r_head.right() - btn_w - 10.0,
                    r_head.center().y - btn_h / 2.0,
                ),
                vec2(btn_w, btn_h),
            );
            if text_btn(ui, &p_head, r_logout, "Sign out", "ag_logout", false, true) {
                self.antigravity.lock().unwrap().trigger_logout = true;
            }

            let r_refresh = Rect::from_min_size(
                pos2(r_logout.left() - 30.0, r_head.center().y - btn_h / 2.0),
                vec2(24.0, btn_h),
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

            // Helper to clean up verbose descriptions and bucket labels
            let clean_desc = |d: &str| -> String {
                d.trim_start_matches("Models within this group: ")
                    .trim()
                    .to_string()
            };
            let clean_bucket = |name: &str| -> String {
                if name.contains("Five Hour") {
                    "5-Hour Limit".to_string()
                } else if name.contains("Argon Weekly") {
                    "Weekly Limit".to_string()
                } else if name.contains("Weekly Limit") {
                    "Weekly Limit".to_string()
                } else {
                    name.to_string()
                }
            };

            // 2. Top Row: 2 Side-by-Side Bento Cards (Gemini Models & Claude/GPT Models)
            let gap = 8.0;
            let col_w = (w - gap) / 2.0;
            let top_h = 152.0;
            let (row_rect, _) = ui.allocate_exact_size(vec2(w, top_h), Sense::hover());

            for idx in 0..2 {
                let Some(group) = q.groups.get(idx) else {
                    continue;
                };
                let c_rect = Rect::from_min_size(
                    pos2(row_rect.left() + idx as f32 * (col_w + gap), row_rect.top()),
                    vec2(col_w, top_h),
                );
                let p = ui.painter_at(c_rect);
                p.rect_filled(c_rect, CornerRadius::same(12), CARD);
                p.rect_stroke(
                    c_rect,
                    CornerRadius::same(12),
                    Stroke::new(1.0, BORDER),
                    StrokeKind::Inside,
                );

                let pad = 12.0;
                let left = c_rect.left() + pad;
                let right = c_rect.right() - pad;
                let top = c_rect.top();

                let dot_col = if idx == 0 { ACCENT } else { PURPLE };
                p.circle_filled(pos2(left + 3.5, top + 16.0), 3.0, dot_col);
                p.text(
                    pos2(left + 11.0, top + 16.0),
                    Align2::LEFT_CENTER,
                    group.name.to_uppercase(),
                    semibold(11.0),
                    TEXT,
                );

                let desc = clean_desc(&group.description);
                let (desc_fit, _) = fit(&p, &desc, FontId::proportional(10.0), right - left);
                p.text(
                    pos2(left, top + 31.0),
                    Align2::LEFT_CENTER,
                    desc_fit,
                    FontId::proportional(10.0),
                    MUTED,
                );

                p.line_segment(
                    [pos2(left, top + 43.0), pos2(right, top + 43.0)],
                    Stroke::new(1.0, Color32::from_rgb(26, 28, 36)),
                );

                for (b_idx, bucket) in group.buckets.iter().take(2).enumerate() {
                    let by = top + 55.0 + b_idx as f32 * 49.0;
                    let pct = (bucket.remaining_fraction * 100.0).clamp(0.0, 100.0);
                    let pct_str = format!("{:.2}%", pct);
                    let bar_col = if pct < 20.0 { WARN } else { GREEN };

                    p.text(
                        pos2(left, by),
                        Align2::LEFT_CENTER,
                        clean_bucket(&bucket.name),
                        semibold(11.0),
                        TEXT,
                    );
                    p.text(
                        pos2(right, by),
                        Align2::RIGHT_CENTER,
                        &pct_str,
                        semibold(11.0),
                        bar_col,
                    );

                    let bar_rect =
                        Rect::from_min_size(pos2(left, by + 9.0), vec2(right - left, 6.0));
                    p.rect_filled(bar_rect, CornerRadius::same(3), TRACK);
                    if bucket.remaining_fraction > 0.0 {
                        let fw = bar_rect.width() * bucket.remaining_fraction.clamp(0.0, 1.0);
                        p.rect_filled(
                            Rect::from_min_size(bar_rect.min, vec2(fw, 6.0)),
                            CornerRadius::same(3),
                            bar_col,
                        );
                    }

                    let reset_str = crate::antigravity::format_reset_info(
                        bucket.reset_time.as_deref(),
                        bucket.remaining_fraction,
                    );
                    let reset_col = if reset_str == "Quota available" {
                        GREEN.gamma_multiply(0.85)
                    } else {
                        MUTED
                    };
                    let (r_fit, _) = fit(&p, &reset_str, FontId::proportional(9.5), right - left);
                    p.text(
                        pos2(left, by + 24.0),
                        Align2::LEFT_CENTER,
                        r_fit,
                        FontId::proportional(9.5),
                        reset_col,
                    );
                }
            }

            // 3. Bottom Row: Full-Width Card for 3rd Group (Gemini 4 Argon)
            if let Some(group) = q.groups.get(2) {
                let bot_h = ui.available_height().max(74.0);
                let (b_rect, p) = card(ui, w, bot_h);
                let pad = 12.0;
                let left = b_rect.left() + pad;
                let right = b_rect.right() - pad;
                let top = b_rect.top();

                p.circle_filled(pos2(left + 3.5, top + 17.0), 3.0, GREEN);
                let g_title = group.name.to_uppercase();
                let g_font = semibold(11.0);
                p.text(
                    pos2(left + 11.0, top + 17.0),
                    Align2::LEFT_CENTER,
                    &g_title,
                    g_font.clone(),
                    TEXT,
                );
                let gw = text_w(&p, &g_title, &g_font);

                p.text(
                    pos2(left + 11.0 + gw + 8.0, top + 17.0),
                    Align2::LEFT_CENTER,
                    "·  Separate Pro / Ultra plan quota",
                    FontId::proportional(10.0),
                    MUTED,
                );

                if let Some(bucket) = group.buckets.first() {
                    let pct = (bucket.remaining_fraction * 100.0).clamp(0.0, 100.0);
                    let pct_str = format!("{:.2}%", pct);
                    let bar_col = if pct < 20.0 { WARN } else { GREEN };

                    p.text(
                        pos2(right, top + 17.0),
                        Align2::RIGHT_CENTER,
                        &pct_str,
                        semibold(11.5),
                        bar_col,
                    );

                    let bar_y = b_rect.center().y - 2.0;
                    let bar_rect = Rect::from_min_size(pos2(left, bar_y), vec2(right - left, 6.5));
                    p.rect_filled(bar_rect, CornerRadius::same(3), TRACK);
                    if bucket.remaining_fraction > 0.0 {
                        let fw = bar_rect.width() * bucket.remaining_fraction.clamp(0.0, 1.0);
                        p.rect_filled(
                            Rect::from_min_size(bar_rect.min, vec2(fw, 6.5)),
                            CornerRadius::same(3),
                            bar_col,
                        );
                    }

                    let reset_str = crate::antigravity::format_reset_info(
                        bucket.reset_time.as_deref(),
                        bucket.remaining_fraction,
                    );
                    let bot_y = b_rect.bottom() - 16.0;
                    p.text(
                        pos2(left, bot_y),
                        Align2::LEFT_CENTER,
                        &bucket.name,
                        FontId::proportional(10.5),
                        MUTED,
                    );
                    p.text(
                        pos2(right, bot_y),
                        Align2::RIGHT_CENTER,
                        reset_str,
                        FontId::proportional(10.0),
                        if pct >= 99.95 {
                            GREEN.gamma_multiply(0.85)
                        } else {
                            MUTED
                        },
                    );
                }
            }
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

    // =====================================================================
    // Screenshot Tool Section & Interactive Overlay
    // =====================================================================

    pub(crate) fn start_capture(&mut self, ctx: &egui::Context, mode: CaptureMode) {
        self.open = false;
        self.t = 0.0;
        self.hover_since = None;
        self.overlay = OverlayPhase::Preparing {
            mode,
            started: Instant::now(),
        };
        ctx.request_repaint();
    }

    pub(crate) fn screenshot_section(&mut self, ui: &mut Ui) {
        let w = ui.available_width();
        ui.spacing_mut().item_spacing.y = 0.0;

        // 1. Top Row: 3 Capture Mode Cards (Window, Region, Fullscreen)
        let gap = 8.0;
        let card_w = (w - gap * 2.0) / 3.0;
        let card_h = 54.0;
        let (row_rect, _) = ui.allocate_exact_size(vec2(w, card_h), Sense::hover());

        let modes = [
            (CaptureMode::Window, "Window", "Click a window"),
            (CaptureMode::Region, "Region", "Alt + Shift + S"),
            (CaptureMode::Screen, "Fullscreen", "Entire display"),
        ];

        let busy = self.screenshot.lock().unwrap().busy;

        for (i, (mode, title, sub)) in modes.iter().enumerate() {
            let x = row_rect.left() + i as f32 * (card_w + gap);
            let r = Rect::from_min_size(pos2(x, row_rect.top()), vec2(card_w, card_h));
            let resp = btn_rect(ui, r, ("ss_mode_card", i));
            let hov = resp.hovered() && !busy;

            let p = ui.painter_at(r);
            let fill = if hov {
                Color32::from_rgb(24, 26, 35)
            } else {
                CARD
            };
            let border_col = if hov {
                ACCENT.gamma_multiply(0.75)
            } else {
                BORDER
            };
            p.rect_filled(r, CornerRadius::same(12), fill);
            p.rect_stroke(
                r,
                CornerRadius::same(12),
                Stroke::new(1.0, border_col),
                StrokeKind::Inside,
            );

            // Icon badge on the left
            let ic = pos2(r.left() + 24.0, r.center().y);
            p.circle_filled(
                ic,
                14.0,
                if hov {
                    ACCENT.gamma_multiply(0.20)
                } else {
                    Color32::from_rgb(24, 26, 34)
                },
            );
            let ic_col = if hov { ACCENT } else { TEXT };
            let st = Stroke::new(1.4, ic_col);

            match mode {
                CaptureMode::Window => {
                    let wr = Rect::from_center_size(ic, vec2(13.0, 10.0));
                    p.rect_stroke(wr, CornerRadius::same(2), st, StrokeKind::Inside);
                    p.line_segment(
                        [
                            pos2(wr.left(), wr.top() + 3.2),
                            pos2(wr.right(), wr.top() + 3.2),
                        ],
                        Stroke::new(1.0, ic_col),
                    );
                }
                CaptureMode::Region => {
                    let cr = Rect::from_center_size(ic, vec2(12.0, 10.0));
                    let len = 3.5;
                    // 4 viewfinder corners
                    p.line_segment([cr.left_top(), cr.left_top() + vec2(len, 0.0)], st);
                    p.line_segment([cr.left_top(), cr.left_top() + vec2(0.0, len)], st);
                    p.line_segment([cr.right_top(), cr.right_top() + vec2(-len, 0.0)], st);
                    p.line_segment([cr.right_top(), cr.right_top() + vec2(0.0, len)], st);
                    p.line_segment([cr.left_bottom(), cr.left_bottom() + vec2(len, 0.0)], st);
                    p.line_segment([cr.left_bottom(), cr.left_bottom() + vec2(0.0, -len)], st);
                    p.line_segment([cr.right_bottom(), cr.right_bottom() + vec2(-len, 0.0)], st);
                    p.line_segment([cr.right_bottom(), cr.right_bottom() + vec2(0.0, -len)], st);
                }
                CaptureMode::Screen => {
                    let sr = Rect::from_center_size(ic + vec2(0.0, -1.0), vec2(13.0, 9.0));
                    p.rect_stroke(sr, CornerRadius::same(2), st, StrokeKind::Inside);
                    p.line_segment(
                        [
                            pos2(ic.x - 3.0, sr.bottom() + 2.5),
                            pos2(ic.x + 3.0, sr.bottom() + 2.5),
                        ],
                        st,
                    );
                }
            }

            p.text(
                pos2(r.left() + 45.0, r.center().y - 7.0),
                Align2::LEFT_CENTER,
                *title,
                semibold(12.0),
                TEXT,
            );
            p.text(
                pos2(r.left() + 45.0, r.center().y + 8.0),
                Align2::LEFT_CENTER,
                *sub,
                FontId::proportional(10.0),
                if *mode == CaptureMode::Region {
                    ACCENT.gamma_multiply(0.9)
                } else {
                    MUTED
                },
            );

            if resp.clicked() && !busy {
                let ctx = ui.ctx().clone();
                self.start_capture(&ctx, *mode);
            }
        }

        ui.add_space(6.0);

        // 2. Middle Bar: Background Mode (White / Image / Clear) + Set Background Image + Reset
        let (bar_rect, p) = card(ui, w, 34.0);
        let (bg_mode, bg_path, mut padding, mut rounding, mut shadow, picking_bg) = {
            let st = self.screenshot.lock().unwrap();
            (
                st.bg_mode,
                st.bg_image_path.clone(),
                st.padding,
                st.rounding,
                st.shadow,
                st.picking_bg,
            )
        };

        p.text(
            pos2(bar_rect.left() + 12.0, bar_rect.center().y),
            Align2::LEFT_CENTER,
            "BG:",
            semibold(11.0),
            MUTED,
        );

        let mut x_cur = bar_rect.left() + 38.0;
        for (m, label) in [
            (BgMode::White, "White"),
            (BgMode::Image, "Image"),
            (BgMode::Transparent, "Clear"),
        ] {
            let pill_w = 50.0;
            let pill_r =
                Rect::from_min_size(pos2(x_cur, bar_rect.center().y - 11.0), vec2(pill_w, 22.0));
            let resp = btn_rect(ui, pill_r, ("ss_bg_mode", label));
            let active = bg_mode == m;
            let fill = if active {
                ACCENT.gamma_multiply(0.22)
            } else if resp.hovered() {
                Color32::from_rgb(28, 30, 38)
            } else {
                Color32::from_rgb(20, 21, 27)
            };
            p.rect_filled(pill_r, CornerRadius::same(6), fill);
            if active {
                p.rect_stroke(
                    pill_r,
                    CornerRadius::same(6),
                    Stroke::new(1.0, ACCENT.gamma_multiply(0.8)),
                    StrokeKind::Inside,
                );
            }
            p.text(
                pill_r.center(),
                Align2::CENTER_CENTER,
                label,
                semibold(10.5),
                if active { TEXT } else { MUTED },
            );
            if resp.clicked() {
                if m == BgMode::Image && bg_path.is_none() {
                    spawn_bg_image_picker(ui.ctx().clone(), self.screenshot.clone());
                } else {
                    {
                        let mut st = self.screenshot.lock().unwrap();
                        st.bg_mode = m;
                        save_screenshot_settings(&st);
                    }
                    rerender_latest_async(ui.ctx().clone(), self.screenshot.clone());
                }
            }
            x_cur += pill_w + 4.0;
        }

        // "Set BG Image..." button
        let bg_btn_label = if picking_bg {
            "Selecting…".to_string()
        } else if let Some(ref path) = bg_path {
            let fname = std::path::Path::new(path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "Custom BG".to_string());
            format!("🖼 {}", ellipsize(&fname, 15))
        } else {
            "Set BG Image…".to_string()
        };
        let bg_btn_w = 122.0;
        let bg_btn_r = Rect::from_min_size(
            pos2(x_cur + 4.0, bar_rect.center().y - 11.0),
            vec2(bg_btn_w, 22.0),
        );
        let bg_resp = btn_rect(ui, bg_btn_r, "ss_pick_bg_btn");
        p.rect_filled(
            bg_btn_r,
            CornerRadius::same(6),
            field_fill(bg_resp.hovered(), false),
        );
        p.rect_stroke(
            bg_btn_r,
            CornerRadius::same(6),
            Stroke::new(1.0, BORDER),
            StrokeKind::Inside,
        );
        let (fit_lbl, _) = fit(
            &p,
            &bg_btn_label,
            FontId::proportional(10.5),
            bg_btn_w - 12.0,
        );
        p.text(
            bg_btn_r.center(),
            Align2::CENTER_CENTER,
            fit_lbl,
            FontId::proportional(10.5),
            if bg_resp.hovered() { TEXT } else { MUTED },
        );
        if bg_resp
            .on_hover_text("Choose background image (PNG/JPG/WEBP/BMP)")
            .clicked()
            && !picking_bg
        {
            spawn_bg_image_picker(ui.ctx().clone(), self.screenshot.clone());
        }

        // Reset button on the right of the bar (resets Padding=54, Rounded=12, Shadow=19)
        let is_default = (padding - DEFAULT_PADDING).abs() < 0.1
            && (rounding - DEFAULT_ROUNDING).abs() < 0.1
            && (shadow - DEFAULT_SHADOW).abs() < 0.1;
        let rst_w = 58.0;
        let rst_r = Rect::from_min_size(
            pos2(bar_rect.right() - rst_w - 10.0, bar_rect.center().y - 11.0),
            vec2(rst_w, 22.0),
        );
        if text_btn(ui, &p, rst_r, "Reset", "ss_reset_style", false, !is_default) {
            padding = DEFAULT_PADDING;
            rounding = DEFAULT_ROUNDING;
            shadow = DEFAULT_SHADOW;
            {
                let mut st = self.screenshot.lock().unwrap();
                st.padding = DEFAULT_PADDING;
                st.rounding = DEFAULT_ROUNDING;
                st.shadow = DEFAULT_SHADOW;
                save_screenshot_settings(&st);
            }
            rerender_latest_async(ui.ctx().clone(), self.screenshot.clone());
        }

        ui.add_space(6.0);

        // 3. Sliders Row: Padding (default 54), Rounded (default 12), Shadow (default 19)
        let (sl_rect, p) = card(ui, w, 38.0);
        let col_w = (w - 24.0) / 3.0;
        let mut style_changed = false;
        let mut style_released = false;

        let mut draw_slider =
            |idx: usize, label: &str, val: &mut f32, min_v: f32, max_v: f32, id: &str| {
                let x0 = sl_rect.left() + 12.0 + idx as f32 * col_w;
                let x1 = x0 + col_w - if idx < 2 { 12.0 } else { 0.0 };
                let top_y = sl_rect.top() + 11.0;
                let bar_y = sl_rect.top() + 27.0;

                p.text(
                    pos2(x0, top_y),
                    Align2::LEFT_CENTER,
                    label,
                    FontId::proportional(10.5),
                    MUTED,
                );
                p.text(
                    pos2(x1, top_y),
                    Align2::RIGHT_CENTER,
                    format!("{:.0}", *val),
                    semibold(10.5),
                    TEXT,
                );

                let hit_r = Rect::from_min_max(pos2(x0, bar_y - 7.0), pos2(x1, bar_y + 7.0));
                let resp = ui
                    .interact(
                        hit_r,
                        egui::Id::new(("ss_slider", id)),
                        Sense::click_and_drag(),
                    )
                    .on_hover_cursor(egui::CursorIcon::PointingHand);

                if (resp.dragged() || resp.clicked()) && !busy {
                    if let Some(pos) = resp.interact_pointer_pos() {
                        let t = ((pos.x - x0) / (x1 - x0).max(1.0)).clamp(0.0, 1.0);
                        let next = (min_v + t * (max_v - min_v)).round();
                        if (next - *val).abs() >= 0.5 {
                            *val = next;
                            style_changed = true;
                        }
                    }
                }
                if resp.drag_stopped() || resp.clicked() {
                    style_released = true;
                }

                let frac = ((*val - min_v) / (max_v - min_v).max(1.0)).clamp(0.0, 1.0);
                let track_r = Rect::from_min_max(pos2(x0, bar_y - 2.0), pos2(x1, bar_y + 2.0));
                p.rect_filled(track_r, CornerRadius::same(2), TRACK);
                p.rect_filled(
                    Rect::from_min_size(track_r.min, vec2(track_r.width() * frac, 4.0)),
                    CornerRadius::same(2),
                    ACCENT,
                );
                let knob_x = x0 + track_r.width() * frac;
                p.circle_filled(
                    pos2(knob_x, bar_y),
                    if resp.hovered() || resp.dragged() {
                        5.0
                    } else {
                        4.0
                    },
                    TEXT,
                );
            };

        draw_slider(0, "Padding", &mut padding, 0.0, 120.0, "pad");
        draw_slider(1, "Rounded", &mut rounding, 0.0, 36.0, "rad");
        draw_slider(2, "Shadow", &mut shadow, 0.0, 40.0, "shd");

        if style_changed {
            let mut st = self.screenshot.lock().unwrap();
            st.padding = padding;
            st.rounding = rounding;
            st.shadow = shadow;
            save_screenshot_settings(&st);
        }
        if style_released {
            rerender_latest_async(ui.ctx().clone(), self.screenshot.clone());
        }

        ui.add_space(6.0);

        // 4. Bottom Row: Preview & Quick Actions Card
        let prev_h = ui.available_height().max(132.0);
        let (card_r, p) = card(ui, w, prev_h);

        let (latest, status_msg) = {
            let st = self.screenshot.lock().unwrap();
            (st.latest.clone(), st.status_msg.clone())
        };

        let thumb_rect = Rect::from_min_size(
            pos2(card_r.left() + 10.0, card_r.top() + 10.0),
            vec2(186.0, prev_h - 20.0),
        );
        p.rect_filled(
            thumb_rect,
            CornerRadius::same(8),
            Color32::from_rgb(10, 11, 15),
        );
        p.rect_stroke(
            thumb_rect,
            CornerRadius::same(8),
            Stroke::new(1.0, BORDER),
            StrokeKind::Inside,
        );

        if let Some(ref cap) = latest {
            let need_load = self
                .ss_preview_tex
                .as_ref()
                .map_or(true, |(v, _)| *v != cap.ver);
            if need_load {
                let tex = ui.ctx().load_texture(
                    "ss_latest_preview",
                    (*cap.preview).clone(),
                    egui::TextureOptions::LINEAR,
                );
                self.ss_preview_tex = Some((cap.ver, tex));
            }
            if let Some((_, ref tex)) = self.ss_preview_tex {
                let avail = thumb_rect.shrink(6.0);
                let aspect = cap.width as f32 / cap.height.max(1) as f32;
                let box_aspect = avail.width() / avail.height().max(1.0);
                let img_size = if aspect > box_aspect {
                    vec2(avail.width(), avail.width() / aspect)
                } else {
                    vec2(avail.height() * aspect, avail.height())
                };
                let img_rect = Rect::from_center_size(avail.center(), img_size);
                p.image(
                    tex.id(),
                    img_rect,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    Color32::WHITE,
                );
            }
        } else {
            // Live style mockup reflecting Padding, Rounded, and Shadow sliders
            let mock_bg = thumb_rect.shrink(8.0);
            let bg_col = match bg_mode {
                BgMode::White => Color32::from_rgb(245, 246, 250),
                BgMode::Image => Color32::from_rgb(55, 75, 120),
                BgMode::Transparent => Color32::from_rgb(22, 24, 30),
            };
            p.rect_filled(mock_bg, CornerRadius::same(6), bg_col);
            let pad_t = (padding / 120.0).clamp(0.0, 1.0);
            let win_w = lerp(154.0, 88.0, pad_t);
            let win_h = lerp(86.0, 48.0, pad_t);
            let win_r = Rect::from_center_size(mock_bg.center(), vec2(win_w, win_h));
            let mock_rad = ((rounding * 0.5).round() as u8).min(18);
            if shadow > 0.5 {
                let sh_t = (shadow / 40.0).clamp(0.0, 1.0);
                let sh_alpha = (sh_t * 110.0).round() as u8;
                p.rect_filled(
                    win_r
                        .translate(vec2(0.0, 2.0 + sh_t * 4.0))
                        .expand(1.5 + sh_t * 4.5),
                    CornerRadius::same(mock_rad.saturating_add(2)),
                    Color32::from_black_alpha(sh_alpha),
                );
            }
            p.rect_filled(
                win_r,
                CornerRadius::same(mock_rad),
                Color32::from_rgb(24, 26, 34),
            );
            // 3 window header dots in mockup
            p.circle_filled(
                pos2(win_r.left() + 10.0, win_r.top() + 9.0),
                2.5,
                Color32::from_rgb(255, 95, 86),
            );
            p.circle_filled(
                pos2(win_r.left() + 18.0, win_r.top() + 9.0),
                2.5,
                Color32::from_rgb(255, 189, 46),
            );
            p.circle_filled(
                pos2(win_r.left() + 26.0, win_r.top() + 9.0),
                2.5,
                Color32::from_rgb(39, 201, 63),
            );
            p.text(
                win_r.center() + vec2(0.0, 4.0),
                Align2::CENTER_CENTER,
                "Preview",
                FontId::proportional(10.0),
                MUTED,
            );
        }

        // Right column details & buttons
        let rx = thumb_rect.right() + 14.0;
        let r_max = card_r.right() - 12.0 - rx;

        if let Some(ref cap) = latest {
            let fname = cap
                .saved_path
                .as_ref()
                .and_then(|p| p.file_name())
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "Screenshot.png".to_string());
            let (fit_name, _) = fit(&p, &fname, semibold(12.5), r_max - 42.0);
            p.text(
                pos2(rx, card_r.top() + 22.0),
                Align2::LEFT_CENTER,
                fit_name,
                semibold(12.5),
                TEXT,
            );
            p.text(
                pos2(card_r.right() - 12.0, card_r.top() + 22.0),
                Align2::RIGHT_CENTER,
                ago(cap.at),
                FontId::proportional(10.5),
                MUTED,
            );

            let mode_name = match cap.mode {
                CaptureMode::Window => "Window",
                CaptureMode::Region => "Region",
                CaptureMode::Screen => "Fullscreen",
            };
            p.text(
                pos2(rx, card_r.top() + 42.0),
                Align2::LEFT_CENTER,
                format!(
                    "{} × {} px  ·  {} capture",
                    cap.width, cap.height, mode_name
                ),
                FontId::proportional(11.0),
                MUTED,
            );
        } else {
            p.text(
                pos2(rx, card_r.top() + 24.0),
                Align2::LEFT_CENTER,
                "Ready to Capture",
                semibold(13.0),
                TEXT,
            );
            p.text(
                pos2(rx, card_r.top() + 44.0),
                Align2::LEFT_CENTER,
                "Press Alt+Shift+S or select a mode above.",
                FontId::proportional(11.0),
                MUTED,
            );
            p.text(
                pos2(rx, card_r.top() + 62.0),
                Align2::LEFT_CENTER,
                "Auto-copies to Clipboard & saves PNG.",
                FontId::proportional(10.5),
                DIM,
            );
        }

        if let Some((msg, at)) = status_msg {
            if at.elapsed().as_secs() < 5 {
                let (fit_msg, _) = fit(&p, &msg, FontId::proportional(10.5), r_max);
                p.text(
                    pos2(rx, card_r.top() + 66.0),
                    Align2::LEFT_CENTER,
                    fit_msg,
                    FontId::proportional(10.5),
                    GREEN,
                );
            }
        }

        // Action buttons at bottom right
        let btn_y = card_r.bottom() - 38.0;
        let btn_h = 28.0;
        let copy_r = Rect::from_min_size(pos2(rx, btn_y), vec2(115.0, btn_h));
        if text_btn(
            ui,
            &p,
            copy_r,
            "Copy to Clipboard",
            "ss_copy_latest",
            true,
            latest.is_some(),
        ) {
            if let Some(ref cap) = latest {
                if copy_rgba_to_clipboard(cap.width, cap.height, &cap.rgba) {
                    self.screenshot.lock().unwrap().status_msg =
                        Some(("Copied to clipboard!".to_string(), Instant::now()));
                }
            }
        }

        let dir_r = Rect::from_min_size(pos2(rx + 123.0, btn_y), vec2(115.0, btn_h));
        if text_btn(ui, &p, dir_r, "Open Folder", "ss_open_folder", false, true) {
            let folder = latest
                .as_ref()
                .and_then(|c| c.saved_path.as_ref())
                .and_then(|p| p.parent())
                .map(|p| p.to_path_buf())
                .unwrap_or_else(screenshots_dir);
            let _ = std::process::Command::new("explorer.exe")
                .arg(folder)
                .spawn();
        }
    }

    pub(crate) fn update_capture_overlay(&mut self, ctx: &egui::Context) {
        // Check if global hotkey Alt+Shift+S fired
        let hk = self.screenshot.lock().unwrap().hotkey_triggered.take();
        if let Some(mode) = hk {
            if matches!(self.overlay, OverlayPhase::Idle) {
                self.start_capture(ctx, mode);
            }
        }

        match self.overlay.clone() {
            OverlayPhase::Idle => {}
            OverlayPhase::Preparing { mode, started } => {
                if started.elapsed() < std::time::Duration::from_millis(90) {
                    ctx.request_repaint();
                    return;
                }
                let Some(desktop) = capture_raw_desktop() else {
                    self.overlay = OverlayPhase::Idle;
                    return;
                };
                if mode == CaptureMode::Screen {
                    self.overlay = OverlayPhase::Idle;
                    process_capture_async(
                        ctx.clone(),
                        self.screenshot.clone(),
                        self.notifs.clone(),
                        CaptureMode::Screen,
                        desktop.width,
                        desktop.height,
                        (*desktop.rgba).clone(),
                    );
                } else {
                    self.ss_desktop_ver = self.ss_desktop_ver.wrapping_add(1);
                    let img = egui::ColorImage::from_rgba_unmultiplied(
                        [desktop.width as usize, desktop.height as usize],
                        &desktop.rgba,
                    );
                    let tex =
                        ctx.load_texture("ss_desktop_freeze", img, egui::TextureOptions::LINEAR);
                    self.ss_desktop_tex = Some((self.ss_desktop_ver, tex));
                    self.overlay = OverlayPhase::Active {
                        mode,
                        desktop,
                        drag_start: None,
                        drag_cur: None,
                    };
                    ctx.request_repaint();
                }
            }
            OverlayPhase::Active {
                mut mode,
                desktop,
                mut drag_start,
                mut drag_cur,
            } => {
                let ppp = ctx.pixels_per_point();
                let screen_w = desktop.width as f32 / ppp;
                let screen_h = desktop.height as f32 / ppp;
                let mut next_overlay: Option<OverlayPhase> = None;
                let mut finish_crop: Option<(CaptureMode, [i32; 4])> = None;
                let tex_opt = self.ss_desktop_tex.as_ref().map(|(_, t)| t.clone());

                let vp_builder = egui::ViewportBuilder::default()
                    .with_title("EmelkaNotch Capture")
                    .with_position([0.0, 0.0])
                    .with_inner_size([screen_w, screen_h])
                    .with_decorations(false)
                    .with_always_on_top()
                    .with_resizable(false)
                    .with_taskbar(false);

                ctx.show_viewport_immediate(
                    egui::ViewportId::from_hash_of("emelka_capture_overlay"),
                    vp_builder,
                    |vp_ctx, _| {
                        if vp_ctx.input(|i| {
                            i.key_pressed(egui::Key::Escape) || i.pointer.secondary_clicked()
                        }) {
                            next_overlay = Some(OverlayPhase::Idle);
                            return;
                        }
                        if vp_ctx.input(|i| i.key_pressed(egui::Key::Space)) {
                            mode = if mode == CaptureMode::Region {
                                CaptureMode::Window
                            } else {
                                CaptureMode::Region
                            };
                            drag_start = None;
                            drag_cur = None;
                        }

                        egui::CentralPanel::default()
                            .frame(egui::Frame::NONE)
                            .show(vp_ctx, |ui| {
                                let full_rect =
                                    Rect::from_min_size(pos2(0.0, 0.0), vec2(screen_w, screen_h));
                                let p = ui.painter().clone();

                                // 1. Draw frozen desktop
                                if let Some(ref tex) = tex_opt {
                                    p.image(
                                        tex.id(),
                                        full_rect,
                                        Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                                        Color32::WHITE,
                                    );
                                }
                                // 2. Dim backdrop
                                p.rect_filled(
                                    full_rect,
                                    CornerRadius::ZERO,
                                    Color32::from_black_alpha(115),
                                );

                                // Floating bottom toolbar rect (so clicks on it don't trigger capture)
                                let bar_r = Rect::from_center_size(
                                    pos2(screen_w * 0.5, (screen_h - 54.0).max(40.0)),
                                    vec2(430.0, 40.0),
                                );
                                let ptr_pos = ui.input(|i| i.pointer.interact_pos());
                                let over_bar = ptr_pos.map_or(false, |pos| bar_r.contains(pos));

                                match mode {
                                    CaptureMode::Region => {
                                        if !over_bar {
                                            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
                                            if ui.input(|i| i.pointer.primary_pressed()) {
                                                drag_start = ptr_pos;
                                                drag_cur = ptr_pos;
                                            } else if ui.input(|i| i.pointer.primary_down())
                                                && drag_start.is_some()
                                            {
                                                drag_cur = ptr_pos;
                                            }
                                        }

                                        if let (Some(s), Some(c)) = (drag_start, drag_cur) {
                                            let sel = Rect::from_two_pos(s, c);
                                            if let Some(ref tex) = tex_opt {
                                                let uv = Rect::from_min_max(
                                                    pos2(
                                                        (sel.left() / screen_w).clamp(0.0, 1.0),
                                                        (sel.top() / screen_h).clamp(0.0, 1.0),
                                                    ),
                                                    pos2(
                                                        (sel.right() / screen_w).clamp(0.0, 1.0),
                                                        (sel.bottom() / screen_h).clamp(0.0, 1.0),
                                                    ),
                                                );
                                                p.image(tex.id(), sel, uv, Color32::WHITE);
                                            }
                                            p.rect_stroke(
                                                sel,
                                                CornerRadius::same(4),
                                                Stroke::new(1.5, Color32::WHITE),
                                                StrokeKind::Outside,
                                            );

                                            let x0 = (sel.left() * ppp).round() as i32;
                                            let y0 = (sel.top() * ppp).round() as i32;
                                            let x1 = (sel.right() * ppp).round() as i32;
                                            let y1 = (sel.bottom() * ppp).round() as i32;
                                            let pw = (x1 - x0).max(0);
                                            let ph = (y1 - y0).max(0);

                                            // Dimension badge
                                            let badge_r = Rect::from_center_size(
                                                pos2(sel.center().x, (sel.top() - 18.0).max(20.0)),
                                                vec2(104.0, 24.0),
                                            );
                                            p.rect_filled(
                                                badge_r,
                                                CornerRadius::same(12),
                                                Color32::from_black_alpha(210),
                                            );
                                            p.text(
                                                badge_r.center(),
                                                Align2::CENTER_CENTER,
                                                format!("{pw} × {ph}"),
                                                semibold(11.5),
                                                TEXT,
                                            );

                                            if ui.input(|i| i.pointer.primary_released()) {
                                                if pw >= 12 && ph >= 12 {
                                                    finish_crop = Some((
                                                        CaptureMode::Region,
                                                        [x0, y0, x1, y1],
                                                    ));
                                                } else {
                                                    drag_start = None;
                                                    drag_cur = None;
                                                }
                                            }
                                        }
                                    }
                                    CaptureMode::Window | CaptureMode::Screen => {
                                        if !over_bar {
                                            ui.ctx()
                                                .set_cursor_icon(egui::CursorIcon::PointingHand);
                                        }
                                        let hovered_win = ptr_pos.and_then(|pos| {
                                            if over_bar {
                                                return None;
                                            }
                                            let px = (pos.x * ppp).round() as i32;
                                            let py = (pos.y * ppp).round() as i32;
                                            desktop.windows.iter().find(|w| {
                                                px >= w.rect[0]
                                                    && px <= w.rect[2]
                                                    && py >= w.rect[1]
                                                    && py <= w.rect[3]
                                            })
                                        });

                                        if let Some(win) = hovered_win {
                                            let wr = Rect::from_min_max(
                                                pos2(
                                                    win.rect[0] as f32 / ppp,
                                                    win.rect[1] as f32 / ppp,
                                                ),
                                                pos2(
                                                    win.rect[2] as f32 / ppp,
                                                    win.rect[3] as f32 / ppp,
                                                ),
                                            );
                                            if let Some(ref tex) = tex_opt {
                                                let uv = Rect::from_min_max(
                                                    pos2(
                                                        (wr.left() / screen_w).clamp(0.0, 1.0),
                                                        (wr.top() / screen_h).clamp(0.0, 1.0),
                                                    ),
                                                    pos2(
                                                        (wr.right() / screen_w).clamp(0.0, 1.0),
                                                        (wr.bottom() / screen_h).clamp(0.0, 1.0),
                                                    ),
                                                );
                                                p.image(tex.id(), wr, uv, Color32::WHITE);
                                            }
                                            p.rect_stroke(
                                                wr,
                                                CornerRadius::same(10),
                                                Stroke::new(2.5, ACCENT),
                                                StrokeKind::Outside,
                                            );

                                            let pw = win.rect[2] - win.rect[0];
                                            let ph = win.rect[3] - win.rect[1];
                                            let lbl = format!(
                                                "{}  ·  {} × {}",
                                                ellipsize(&win.title, 34),
                                                pw,
                                                ph
                                            );
                                            let bw = (text_w(&p, &lbl, &semibold(12.0)) + 28.0)
                                                .clamp(120.0, 380.0);
                                            let badge_r = Rect::from_center_size(
                                                pos2(
                                                    wr.center().x,
                                                    (wr.top() + 24.0).min(wr.center().y),
                                                ),
                                                vec2(bw, 28.0),
                                            );
                                            p.rect_filled(
                                                badge_r,
                                                CornerRadius::same(14),
                                                Color32::from_black_alpha(215),
                                            );
                                            p.rect_stroke(
                                                badge_r,
                                                CornerRadius::same(14),
                                                Stroke::new(1.0, ACCENT.gamma_multiply(0.6)),
                                                StrokeKind::Inside,
                                            );
                                            p.text(
                                                badge_r.center(),
                                                Align2::CENTER_CENTER,
                                                lbl,
                                                semibold(12.0),
                                                TEXT,
                                            );

                                            if ui.input(|i| i.pointer.primary_clicked())
                                                && !over_bar
                                            {
                                                finish_crop = Some((CaptureMode::Window, win.rect));
                                            }
                                        }
                                    }
                                }

                                // 3. Bottom Floating Control Pill
                                p.rect_filled(
                                    bar_r,
                                    CornerRadius::same(20),
                                    Color32::from_rgba_unmultiplied(12, 13, 18, 235),
                                );
                                p.rect_stroke(
                                    bar_r,
                                    CornerRadius::same(20),
                                    Stroke::new(1.0, BORDER),
                                    StrokeKind::Inside,
                                );

                                let reg_btn = Rect::from_min_size(
                                    pos2(bar_r.left() + 8.0, bar_r.top() + 6.0),
                                    vec2(76.0, 28.0),
                                );
                                let win_btn = Rect::from_min_size(
                                    pos2(bar_r.left() + 88.0, bar_r.top() + 6.0),
                                    vec2(76.0, 28.0),
                                );

                                for (b_rect, m_val, m_lbl) in [
                                    (reg_btn, CaptureMode::Region, "Region"),
                                    (win_btn, CaptureMode::Window, "Window"),
                                ] {
                                    let active = mode == m_val;
                                    let r_resp = btn_rect(ui, b_rect, ("ov_mode_btn", m_lbl));
                                    p.rect_filled(
                                        b_rect,
                                        CornerRadius::same(14),
                                        if active {
                                            ACCENT.gamma_multiply(0.26)
                                        } else if r_resp.hovered() {
                                            Color32::from_rgb(28, 30, 38)
                                        } else {
                                            Color32::TRANSPARENT
                                        },
                                    );
                                    p.text(
                                        b_rect.center(),
                                        Align2::CENTER_CENTER,
                                        m_lbl,
                                        semibold(11.5),
                                        if active { TEXT } else { MUTED },
                                    );
                                    if r_resp.clicked() {
                                        mode = m_val;
                                        drag_start = None;
                                        drag_cur = None;
                                    }
                                }

                                p.text(
                                    pos2(bar_r.right() - 16.0, bar_r.center().y),
                                    Align2::RIGHT_CENTER,
                                    "Space: Switch Mode  ·  Esc: Cancel",
                                    FontId::proportional(11.0),
                                    MUTED,
                                );
                            });
                    },
                );

                if let Some((cap_mode, rect)) = finish_crop {
                    self.overlay = OverlayPhase::Idle;
                    if let Some((cw, ch, cropped)) = crop_desktop(&desktop, rect) {
                        process_capture_async(
                            ctx.clone(),
                            self.screenshot.clone(),
                            self.notifs.clone(),
                            cap_mode,
                            cw,
                            ch,
                            cropped,
                        );
                    }
                } else if let Some(next) = next_overlay {
                    self.overlay = next;
                } else {
                    self.overlay = OverlayPhase::Active {
                        mode,
                        desktop,
                        drag_start,
                        drag_cur,
                    };
                }
            }
        }
    }
}
