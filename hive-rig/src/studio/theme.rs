//! The Hive Rig look: a dark, card-based theme with a honey accent, and the small
//! widgets (cards, pills, segmented controls, stat tiles) the panels are built from.

use eframe::egui::{
    self, Color32, CornerRadius, FontFamily, FontId, Margin, Response, RichText, Sense, Shadow,
    Stroke, TextStyle, Ui, Vec2,
};

pub const BG: Color32 = Color32::from_rgb(13, 15, 19);
pub const PANEL: Color32 = Color32::from_rgb(19, 22, 28);
pub const CARD: Color32 = Color32::from_rgb(26, 30, 38);
pub const CARD_HI: Color32 = Color32::from_rgb(34, 39, 49);
pub const BORDER: Color32 = Color32::from_rgb(40, 46, 57);
pub const TEXT: Color32 = Color32::from_rgb(232, 234, 240);
pub const TEXT_DIM: Color32 = Color32::from_rgb(152, 159, 172);
pub const TEXT_FAINT: Color32 = Color32::from_rgb(98, 105, 118);
pub const ACCENT: Color32 = Color32::from_rgb(247, 185, 64);
pub const ACCENT_TEXT: Color32 = Color32::from_rgb(28, 20, 6);
pub const OK: Color32 = Color32::from_rgb(88, 206, 142);
pub const ERR: Color32 = Color32::from_rgb(242, 98, 98);
/// Warnings: the pre-build checks and the toast for an operation that worked with a caveat.
pub const WARN: Color32 = Color32::from_rgb(255, 168, 48);
pub const CENTER: Color32 = Color32::from_rgb(247, 200, 64);
pub const LEFT: Color32 = Color32::from_rgb(80, 150, 255);
pub const RIGHT: Color32 = Color32::from_rgb(255, 94, 94);

pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    ctx.style_mut_of(egui::Theme::Dark, |s| {
        s.text_styles = [
            (
                TextStyle::Small,
                FontId::new(11.5, FontFamily::Proportional),
            ),
            (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
            (
                TextStyle::Button,
                FontId::new(14.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Heading,
                FontId::new(20.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Monospace,
                FontId::new(13.0, FontFamily::Monospace),
            ),
        ]
        .into();
        let sp = &mut s.spacing;
        sp.item_spacing = Vec2::new(8.0, 8.0);
        sp.button_padding = Vec2::new(12.0, 6.0);
        sp.interact_size = Vec2::new(40.0, 28.0);
        sp.menu_margin = Margin::same(8);
        sp.combo_width = 140.0;

        let v = &mut s.visuals;
        v.dark_mode = true;
        v.panel_fill = PANEL;
        v.window_fill = CARD;
        v.window_stroke = Stroke::new(1.0, BORDER);
        v.window_corner_radius = CornerRadius::same(12);
        v.menu_corner_radius = CornerRadius::same(10);
        v.window_shadow = shadow();
        v.popup_shadow = shadow();
        v.extreme_bg_color = Color32::from_rgb(15, 17, 22);
        v.faint_bg_color = CARD;
        v.text_edit_bg_color = Some(Color32::from_rgb(15, 17, 22));
        v.override_text_color = None;
        v.hyperlink_color = ACCENT;
        v.warn_fg_color = ACCENT;
        v.error_fg_color = ERR;
        v.selection.bg_fill = ACCENT.gamma_multiply(0.28);
        v.selection.stroke = Stroke::new(1.0, ACCENT);

        let w = &mut v.widgets;
        let radius = CornerRadius::same(8);
        w.noninteractive.bg_fill = CARD;
        w.noninteractive.weak_bg_fill = CARD;
        w.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
        w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_DIM);
        w.noninteractive.corner_radius = radius;
        for (state, fill, stroke, text) in [
            (&mut w.inactive, CARD_HI, BORDER, TEXT),
            (
                &mut w.hovered,
                Color32::from_rgb(44, 50, 62),
                Color32::from_rgb(64, 72, 88),
                TEXT,
            ),
            (&mut w.active, Color32::from_rgb(52, 59, 73), ACCENT, TEXT),
            (&mut w.open, Color32::from_rgb(44, 50, 62), BORDER, TEXT),
        ] {
            state.bg_fill = fill;
            state.weak_bg_fill = fill;
            state.bg_stroke = Stroke::new(1.0, stroke);
            state.fg_stroke = Stroke::new(1.5, text);
            state.corner_radius = radius;
            state.expansion = 0.0;
        }
    });
}

fn shadow() -> Shadow {
    Shadow {
        offset: [0, 6],
        blur: 22,
        spread: 0,
        color: Color32::from_black_alpha(110),
    }
}

/// A small, spaced-out, upper-case section label.
pub fn label_caps(ui: &mut Ui, text: &str) {
    ui.label(
        RichText::new(text.to_uppercase())
            .size(11.0)
            .strong()
            .color(TEXT_FAINT)
            .extra_letter_spacing(1.2),
    );
}

/// The main call to action: honey fill, dark text.
pub fn primary(ui: &mut Ui, text: &str, width: f32, enabled: bool) -> Response {
    let button = egui::Button::new(RichText::new(text).size(15.0).strong().color(ACCENT_TEXT))
        .fill(ACCENT)
        .stroke(Stroke::NONE)
        .corner_radius(10)
        .min_size(Vec2::new(width, 40.0));
    ui.add_enabled(enabled, button)
}

pub fn secondary(ui: &mut Ui, text: &str, width: f32) -> Response {
    ui.add(
        egui::Button::new(RichText::new(text).size(14.0))
            .corner_radius(10)
            .min_size(Vec2::new(width, 34.0)),
    )
}

/// A frameless icon button (an emoji glyph) with a tooltip.
pub fn icon(ui: &mut Ui, glyph: &str, tip: &str, enabled: bool) -> Response {
    let b = egui::Button::new(RichText::new(glyph).size(17.0))
        .frame_when_inactive(false)
        .corner_radius(8)
        .min_size(Vec2::new(34.0, 32.0));
    ui.add_enabled(enabled, b).on_hover_text(tip)
}

/// A rounded on/off chip.
pub fn pill(ui: &mut Ui, on: &mut bool, text: &str) -> Response {
    let (fill, stroke, color) = if *on {
        (ACCENT.gamma_multiply(0.18), ACCENT, ACCENT)
    } else {
        (Color32::TRANSPARENT, BORDER, TEXT_DIM)
    };
    let r = ui.add(
        egui::Button::new(RichText::new(text).size(13.0).color(color))
            .fill(fill)
            .stroke(Stroke::new(1.0, stroke))
            .corner_radius(14)
            .min_size(Vec2::new(0.0, 28.0)),
    );
    if r.clicked() {
        *on = !*on;
    }
    r
}

/// A row of mutually exclusive options in one rounded track.
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, options: &[(T, &str)]) -> bool {
    let mut changed = false;
    egui::Frame::new()
        .fill(Color32::from_rgb(15, 17, 22))
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(10)
        .inner_margin(3)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            ui.horizontal(|ui| {
                for (v, text) in options {
                    let on = *value == *v;
                    let b = egui::Button::new(RichText::new(*text).size(13.0).color(if on {
                        ACCENT_TEXT
                    } else {
                        TEXT_DIM
                    }))
                    .fill(if on { ACCENT } else { Color32::TRANSPARENT })
                    .stroke(Stroke::NONE)
                    .corner_radius(8)
                    .min_size(Vec2::new(0.0, 26.0));
                    if ui.add(b).clicked() && !on {
                        *value = *v;
                        changed = true;
                    }
                }
            });
        });
    changed
}

/// A number with a caption, for the rig summary.
pub fn stat(ui: &mut Ui, value: usize, caption: &str, width: f32) {
    egui::Frame::new()
        .fill(Color32::from_rgb(20, 23, 30))
        .corner_radius(10)
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(width - 20.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(
                    RichText::new(value.to_string())
                        .size(22.0)
                        .strong()
                        .color(TEXT),
                );
                ui.label(RichText::new(caption).size(11.5).color(TEXT_DIM));
            });
        });
}

/// A small coloured badge.
pub fn badge(ui: &mut Ui, text: &str, color: Color32) -> Response {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.16))
        .corner_radius(6)
        .inner_margin(Margin::symmetric(7, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(11.5).color(color));
        })
        .response
}

/// A filled dot the height of a line of text.
pub fn dot(ui: &mut Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(10.0, 14.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
}

/// The Hive Rig mark: a honeycomb hexagon with a joint inside.
pub fn logo(painter: &egui::Painter, center: egui::Pos2, radius: f32) {
    let hex: Vec<egui::Pos2> = (0..6)
        .map(|i| {
            let a = std::f32::consts::FRAC_PI_3 * i as f32 + std::f32::consts::FRAC_PI_6;
            center + Vec2::angled(a) * radius
        })
        .collect();
    painter.add(egui::Shape::convex_polygon(hex, ACCENT, Stroke::NONE));
    let s = radius * 0.42;
    let a = center + Vec2::new(-s, s * 0.9);
    let b = center + Vec2::new(0.0, -s * 0.6);
    let c = center + Vec2::new(s, s * 0.9);
    let stroke = Stroke::new((radius * 0.16).max(1.5), ACCENT_TEXT);
    painter.line_segment([a, b], stroke);
    painter.line_segment([b, c], stroke);
    for p in [a, b, c] {
        painter.circle_filled(p, radius * 0.15, ACCENT_TEXT);
    }
}
