// =====================================================================
// UI components (widgets, icons, drawing primitives)
// =====================================================================

use crate::osd::OsdKind;
use crate::theme::{lerp, semibold, BG, CARD, DIM, FIELD_R, HIST, MUTED, TEXT, TRACK};
use eframe::egui::{
    self, pos2, vec2, Align2, Color32, CornerRadius, FontId, Painter, Pos2, Rect, Sense, Shape,
    Stroke, StrokeKind, Ui, UiBuilder,
};
use std::collections::VecDeque;
use std::f32::consts::PI;

#[derive(Clone, Copy, PartialEq)]
pub(crate) enum Section {
    Media,
    Alerts,
    System,
    Weather,
    Todo,
    Prayer,
    Antigravity,
    Settings,
}

pub(crate) fn card(ui: &mut Ui, w: f32, h: f32) -> (Rect, Painter) {
    let (rect, _) = ui.allocate_exact_size(vec2(w, h), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, CornerRadius::same(12), CARD);
    p.rect_stroke(
        rect,
        CornerRadius::same(12),
        Stroke::new(1.0, crate::theme::BORDER),
        StrokeKind::Inside,
    );
    (rect, p)
}

pub(crate) fn card_header(p: &Painter, r: Rect, title: &str, value: &str) {
    p.text(
        pos2(r.left() + 12.0, r.top() + 16.0),
        Align2::LEFT_CENTER,
        title,
        FontId::proportional(11.0),
        crate::theme::MUTED,
    );
    p.text(
        pos2(r.right() - 12.0, r.top() + 16.0),
        Align2::RIGHT_CENTER,
        value,
        semibold(13.0),
        TEXT,
    );
}

pub(crate) fn sparkline(p: &Painter, r: Rect, data: &VecDeque<f32>, color: Color32) {
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

pub(crate) fn push_hist(q: &mut VecDeque<f32>, v: f32) {
    if q.len() >= HIST {
        q.pop_front();
    }
    q.push_back(v);
}

/// Cursor position in the window's local point space (works even when unfocused).
pub(crate) fn cursor_local(ctx: &egui::Context) -> Option<egui::Pos2> {
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

pub(crate) fn badge(p: &Painter, c: Pos2, r: f32, app: &str, alpha: f32) {
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
pub(crate) fn app_icon(
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

/// Speaker (with waves, or an X when muted) / sun icon for the OSD.
pub(crate) fn osd_icon(p: &Painter, c: Pos2, kind: OsdKind, muted: bool, col: Color32) {
    let st = Stroke::new(1.4, col);
    match kind {
        OsdKind::Volume => {
            p.rect_filled(
                Rect::from_min_max(pos2(c.x - 7.0, c.y - 2.2), pos2(c.x - 3.5, c.y + 2.2)),
                CornerRadius::same(1),
                col,
            );
            p.add(Shape::convex_polygon(
                vec![
                    pos2(c.x - 3.5, c.y - 2.2),
                    pos2(c.x + 0.5, c.y - 5.5),
                    pos2(c.x + 0.5, c.y + 5.5),
                    pos2(c.x - 3.5, c.y + 2.2),
                ],
                col,
                Stroke::NONE,
            ));
            if muted {
                p.line_segment([c + vec2(3.0, -3.0), c + vec2(8.0, 3.0)], st);
                p.line_segment([c + vec2(3.0, 3.0), c + vec2(8.0, -3.0)], st);
            } else {
                for r in [3.5f32, 6.5] {
                    let pts: Vec<Pos2> = (0..=8)
                        .map(|i| {
                            let a = -0.75 + 1.5 * i as f32 / 8.0;
                            pos2(c.x + 0.5 + r * a.cos(), c.y + r * a.sin())
                        })
                        .collect();
                    p.add(Shape::line(pts, st));
                }
            }
        }
        OsdKind::Brightness => {
            p.circle_stroke(c, 3.2, st);
            for k in 0..8 {
                let a = k as f32 * PI / 4.0;
                let (s, co) = a.sin_cos();
                p.line_segment(
                    [c + vec2(co * 5.6, s * 5.6), c + vec2(co * 8.0, s * 8.0)],
                    Stroke::new(1.3, col),
                );
            }
        }
    }
}

/// Tab berbentuk ikon; nama tampil sebagai tooltip saat di-hover.
pub(crate) fn tab_icon(ui: &mut Ui, sec: Section, label: &str, selected: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(32.0, 28.0), Sense::click());
    let hov = resp.hovered();
    let bg = if selected {
        Color32::from_rgb(40, 42, 52)
    } else if hov {
        Color32::from_rgb(24, 25, 31)
    } else {
        Color32::TRANSPARENT
    };
    let col = if selected {
        TEXT
    } else if hov {
        Color32::from_rgb(190, 194, 205)
    } else {
        MUTED
    };
    let p = ui.painter();
    p.rect_filled(rect, CornerRadius::same(8), bg);
    let c = rect.center();
    let st = Stroke::new(1.4, col);

    match sec {
        Section::Media => {
            // not musik
            p.circle_filled(c + vec2(-2.5, 4.0), 2.8, col);
            p.line_segment([c + vec2(0.2, 3.5), c + vec2(0.2, -5.5)], st);
            p.line_segment([c + vec2(0.2, -5.5), c + vec2(4.5, -3.0)], st);
        }
        Section::Alerts => {
            // lonceng
            let r = 4.5;
            let mut pts: Vec<Pos2> = (0..=12)
                .map(|i| {
                    let a = PI + PI * i as f32 / 12.0;
                    pos2(c.x + r * a.cos(), c.y - 1.0 + r * a.sin())
                })
                .collect();
            pts.push(pos2(c.x + 5.5, c.y + 3.5));
            pts.push(pos2(c.x - 5.5, c.y + 3.5));
            p.add(Shape::closed_line(pts, st));
            p.circle_filled(c + vec2(0.0, 6.0), 1.3, col);
        }
        Section::System => {
            // chip
            p.rect_stroke(
                Rect::from_center_size(c, vec2(11.0, 11.0)),
                CornerRadius::same(2),
                st,
                StrokeKind::Inside,
            );
            p.rect_stroke(
                Rect::from_center_size(c, vec2(5.0, 5.0)),
                CornerRadius::same(1),
                Stroke::new(1.2, col),
                StrokeKind::Inside,
            );
            for o in [-2.5f32, 2.5] {
                p.line_segment([pos2(c.x + o, c.y - 5.5), pos2(c.x + o, c.y - 8.0)], st);
                p.line_segment([pos2(c.x + o, c.y + 5.5), pos2(c.x + o, c.y + 8.0)], st);
                p.line_segment([pos2(c.x - 5.5, c.y + o), pos2(c.x - 8.0, c.y + o)], st);
                p.line_segment([pos2(c.x + 5.5, c.y + o), pos2(c.x + 8.0, c.y + o)], st);
            }
        }
        Section::Weather => {
            // matahari + awan kecil
            let sc = c + vec2(-2.5, -3.0);
            p.circle_stroke(sc, 2.8, st);
            for k in 0..8 {
                let a = k as f32 * PI / 4.0;
                let (s, co) = a.sin_cos();
                p.line_segment(
                    [sc + vec2(co * 4.2, s * 4.2), sc + vec2(co * 5.7, s * 5.7)],
                    Stroke::new(1.1, col),
                );
            }
            let cc = c + vec2(2.0, 3.5);
            p.circle_filled(cc + vec2(-2.5, 0.5), 2.1, col);
            p.circle_filled(cc + vec2(0.5, -0.5), 2.7, col);
            p.circle_filled(cc + vec2(3.0, 0.5), 1.9, col);
            p.rect_filled(
                Rect::from_min_max(pos2(cc.x - 2.5, cc.y + 0.5), pos2(cc.x + 3.0, cc.y + 2.8)),
                CornerRadius::same(1),
                col,
            );
        }
        Section::Todo => {
            // lingkaran + centang
            p.circle_stroke(c, 6.5, st);
            p.line_segment([c + vec2(-3.0, 0.2), c + vec2(-0.8, 2.6)], st);
            p.line_segment([c + vec2(-0.8, 2.6), c + vec2(3.2, -2.4)], st);
        }
        Section::Prayer => {
            // kubah masjid
            let base = c.y + 1.5;
            let pts: Vec<Pos2> = (0..=14)
                .map(|i| {
                    let a = PI + PI * i as f32 / 14.0;
                    pos2(c.x + 5.5 * a.cos(), base + 5.5 * a.sin())
                })
                .collect();
            p.add(Shape::line(pts, st));
            p.line_segment([pos2(c.x - 5.5, base), pos2(c.x - 5.5, base + 5.5)], st);
            p.line_segment([pos2(c.x + 5.5, base), pos2(c.x + 5.5, base + 5.5)], st);
            p.line_segment(
                [pos2(c.x - 5.5, base + 5.5), pos2(c.x + 5.5, base + 5.5)],
                st,
            );
            p.line_segment([pos2(c.x, base - 5.5), pos2(c.x, base - 8.0)], st);
        }
        Section::Antigravity => {
            // Antigravity AI Spark / Diamond core with levitating rings
            let pts = vec![
                pos2(c.x, c.y - 5.5),
                pos2(c.x + 4.2, c.y),
                pos2(c.x, c.y + 5.5),
                pos2(c.x - 4.2, c.y),
            ];
            p.add(Shape::closed_line(pts, st));
            p.circle_filled(c, 1.4, col);
            p.line_segment(
                [pos2(c.x - 6.5, c.y), pos2(c.x - 5.0, c.y)],
                Stroke::new(1.2, col),
            );
            p.line_segment(
                [pos2(c.x + 5.0, c.y), pos2(c.x + 6.5, c.y)],
                Stroke::new(1.2, col),
            );
        }
        Section::Settings => {
            // gear
            p.circle_stroke(c, 4.0, st);
            p.circle_filled(c, 1.4, col);
            for k in 0..8 {
                let a = k as f32 * PI / 4.0;
                let (s, co) = a.sin_cos();
                p.line_segment(
                    [c + vec2(co * 4.6, s * 4.6), c + vec2(co * 6.8, s * 6.8)],
                    Stroke::new(1.8, col),
                );
            }
        }
    }

    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(label)
        .clicked()
}

#[derive(Clone, Copy)]
pub(crate) enum Icon {
    Prev,
    Next,
    Play,
    Pause,
}

pub(crate) fn icon_button(ui: &mut Ui, icon: Icon, size: f32, primary: bool) -> egui::Response {
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
pub(crate) fn bar(ui: &mut Ui, width: f32, frac: f32, fill: Color32) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 14.0), Sense::click());
    let track = Rect::from_center_size(rect.center(), vec2(rect.width(), 4.0));
    ui.painter()
        .rect_filled(track, CornerRadius::same(2), TRACK);
    let filled = Rect::from_min_size(track.min, vec2(track.width() * frac.clamp(0.0, 1.0), 4.0));
    ui.painter()
        .rect_filled(filled, CornerRadius::same(2), fill);
    resp
}

/// A child Ui placed at `rect` shifted by `dx`, clipped to `rect`, with its own opacity.
pub(crate) fn slide_child(
    ui: &mut Ui,
    rect: Rect,
    dx: f32,
    alpha: f32,
    salt: impl std::hash::Hash,
) -> Ui {
    let mut c = ui.new_child(
        UiBuilder::new()
            .id_salt(salt)
            .max_rect(rect.translate(vec2(dx, 0.0)))
            .layout(egui::Layout::top_down(egui::Align::Min)),
    );
    c.set_clip_rect(rect.intersect(ui.clip_rect()));
    c.set_opacity(alpha.clamp(0.0, 1.0));
    c
}

pub(crate) fn field_fill(hovered: bool, focused: bool) -> Color32 {
    if focused {
        Color32::from_rgb(30, 32, 40)
    } else if hovered {
        Color32::from_rgb(25, 26, 33)
    } else {
        Color32::from_rgb(20, 21, 27)
    }
}

pub(crate) fn btn_rect(ui: &mut Ui, rect: Rect, id: impl std::hash::Hash) -> egui::Response {
    ui.interact(rect, egui::Id::new(id), Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Rounded text button. Returns true when clicked (never when disabled).
pub(crate) fn text_btn(
    ui: &mut Ui,
    p: &Painter,
    rect: Rect,
    label: &str,
    id: &str,
    primary: bool,
    enabled: bool,
) -> bool {
    let r = btn_rect(ui, rect, ("pk_btn", id));
    let hov = r.hovered() && enabled;
    let (fill, col) = if !enabled {
        (TRACK, DIM)
    } else if primary {
        (if hov { Color32::WHITE } else { TEXT }, BG)
    } else {
        (field_fill(hov, false), if hov { TEXT } else { MUTED })
    };
    p.rect_filled(rect, CornerRadius::same(FIELD_R), fill);
    p.text(
        rect.center(),
        Align2::CENTER_CENTER,
        label,
        semibold(11.5),
        col,
    );
    r.clicked() && enabled
}

pub(crate) fn arrow_btn(ui: &mut Ui, p: &Painter, rect: Rect, dir: f32, id: &str) -> bool {
    let r = btn_rect(ui, rect, ("pk_arrow", id));
    if r.hovered() {
        p.rect_filled(rect, CornerRadius::same(6), Color32::from_rgb(32, 34, 42));
    }
    let c = rect.center();
    let s = 4.0;
    p.add(Shape::convex_polygon(
        vec![
            pos2(c.x - dir * s * 0.6, c.y - s),
            pos2(c.x - dir * s * 0.6, c.y + s),
            pos2(c.x + dir * s * 0.9, c.y),
        ],
        if r.hovered() { TEXT } else { MUTED },
        Stroke::NONE,
    ));
    r.clicked()
}

/// [-]  09  [+]  stepper. Wraps around; scroll wheel works too.
pub(crate) fn stepper(
    ui: &mut Ui,
    p: &Painter,
    origin: Pos2,
    val: &mut u32,
    modulo: u32,
    step: u32,
    id: &str,
) {
    let full = Rect::from_min_size(origin, vec2(96.0, 28.0));
    p.rect_filled(
        full,
        CornerRadius::same(FIELD_R),
        field_fill(ui.rect_contains_pointer(full), false),
    );
    let minus = Rect::from_min_size(origin, vec2(26.0, 28.0));
    let plus = Rect::from_min_size(pos2(origin.x + 70.0, origin.y), vec2(26.0, 28.0));
    let mut delta = 0i32;
    if btn_rect(ui, minus, ("pk_minus", id)).clicked() {
        delta = -1;
    }
    if btn_rect(ui, plus, ("pk_plus", id)).clicked() {
        delta = 1;
    }
    if ui.rect_contains_pointer(full) {
        let dy = ui.input(|i| i.raw_scroll_delta.y);
        if dy > 0.0 {
            delta = 1;
        } else if dy < 0.0 {
            delta = -1;
        }
    }
    if delta != 0 {
        *val = (*val as i32 + delta * step as i32).rem_euclid(modulo as i32) as u32;
    }
    let st = Stroke::new(1.4, MUTED);
    let mc = minus.center();
    p.line_segment([mc - vec2(4.0, 0.0), mc + vec2(4.0, 0.0)], st);
    let pc = plus.center();
    p.line_segment([pc - vec2(4.0, 0.0), pc + vec2(4.0, 0.0)], st);
    p.line_segment([pc - vec2(0.0, 4.0), pc + vec2(0.0, 4.0)], st);
    p.text(
        pos2(origin.x + 48.0, origin.y + 14.0),
        Align2::CENTER_CENTER,
        format!("{:02}", *val),
        semibold(13.0),
        TEXT,
    );
}

pub(crate) fn shape_rect(pw: f32, ph: f32, e: f32, top: f32, panel_h: f32) -> Rect {
    let w = lerp(pw, crate::theme::PANEL_W, e);
    let h = lerp(ph, panel_h, e);
    Rect::from_min_size(pos2((crate::theme::WIN_W - w) / 2.0, top), vec2(w, h))
}

use windows_sys::Win32::Foundation::POINT;
use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
