//! The rigging window ("Add a component"): a card for every premade part with
//! its illustration, the options of the picked part, and the art itself.
//! The parts and what adding them does live in `app/library.rs`.

use eframe::egui::{
    self, Align, Align2, Color32, FontId, Layout, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2,
};

use super::super::library::{self, Art, LIBRARY, LibraryPart, Opt};
use super::super::{Category, FingerSides, Studio};
use super::{BONE, SKIN, arc, dialog_frame, dialog_header, hot_chain, joint, paint_art_backdrop};
use crate::theme::{self, ACCENT, BORDER, CARD, CARD_HI, OK, TEXT, TEXT_DIM, TEXT_FAINT};

const CARD_HEIGHT: f32 = 224.0;
const ART_HEIGHT: f32 = 120.0;
const COLUMNS: usize = 5;
const GAP: f32 = 12.0;

/// What a click on a card meant.
enum CardClick {
    /// Show its options.
    Pick,
    /// Add it now with the choices already made for it.
    Add,
}

impl Studio {
    pub(super) fn library_window(&mut self, ctx: &egui::Context) {
        if !self.library_open {
            return;
        }
        let Some(doc) = &self.doc else { return };
        let screen = ctx.content_rect();
        let width = 900f32.min(screen.width() - 60.0);
        let tallies: Vec<usize> = LIBRARY.iter().map(|p| p.tally(&doc.guides)).collect();
        let bones = doc.bone_names();
        let grid_height = (screen.height() - 400.0).clamp(224.0, 700.0);
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::unique("library"))
            .frame(dialog_frame())
            .backdrop_color(Color32::from_black_alpha(140))
            .show(ctx, |ui| {
                ui.set_width(width);
                close |= dialog_header(
                    ui,
                    "Add a component",
                    "Premade parts land on the guides, ready to adjust. Click a card for its options, \
                     or press + to add it as it is. Shift+A opens this window.",
                );
                ui.add_space(4.0);
                let mut cat = self.category;
                ui.horizontal(|ui| {
                    theme::segmented(
                        ui,
                        &mut cat,
                        &[
                            (Category::All, "All"),
                            (Category::Body, "Body"),
                            (Category::Creature, "Wings and creatures"),
                            (Category::Face, "Face"),
                            (Category::Extras, "Extras"),
                            (Category::Templates, "Templates"),
                        ],
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if self.category != Category::Templates {
                            ui.add(
                                egui::TextEdit::singleline(&mut self.lib_search)
                                    .hint_text("Search parts")
                                    .desired_width(150.0),
                            );
                        }
                    });
                });
                self.category = cat;
                ui.add_space(8.0);
                if self.category == Category::Templates {
                    close |= self.template_cards(ui, width);
                    return;
                }
                let query = self.lib_search.trim().to_lowercase();
                let parts: Vec<(usize, &'static LibraryPart)> = LIBRARY
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| {
                        if query.is_empty() {
                            self.category == Category::All || p.category == self.category
                        } else {
                            query.split_whitespace().all(|w| {
                                p.title.to_lowercase().contains(w)
                                    || p.text.to_lowercase().contains(w)
                                    || p.key.contains(w)
                            })
                        }
                    })
                    .collect();
                let card_w = ((width - 16.0 - (COLUMNS - 1) as f32 * GAP) / COLUMNS as f32).floor();
                let mut click: Option<(&'static LibraryPart, CardClick)> = None;
                egui::ScrollArea::vertical()
                    .max_height(grid_height)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing = Vec2::new(GAP, 8.0);
                        if parts.is_empty() {
                            ui.add_space(30.0);
                            ui.label(
                                RichText::new("No part matches that search.")
                                    .color(TEXT_DIM)
                                    .size(14.0),
                            );
                            ui.add_space(30.0);
                        }
                        for row in parts.chunks(COLUMNS) {
                            ui.horizontal(|ui| {
                                for &(i, part) in row {
                                    let selected = self.lib_pick == Some(part.key);
                                    let blocked = part.single && tallies[i] > 0;
                                    if let Some(c) =
                                        part_card(ui, card_w, part, tallies[i], selected, blocked)
                                    {
                                        click = Some((part, c));
                                    }
                                }
                            });
                        }
                    });
                ui.add_space(8.0);
                let mut add = None;
                if let Some((part, CardClick::Pick)) = click {
                    self.pick_part(part);
                }
                if let Some((part, CardClick::Add)) = click {
                    self.pick_part(part);
                    add = Some(part);
                }
                let picked = self.lib_pick.and_then(library::part);
                if let Some(part) = picked {
                    add = add.or(self.library_options(ui, part, &bones, picked_blocked(part, &tallies)));
                } else {
                    options_hint(ui);
                }
                if let Some(part) = add {
                    let before = self.status_serial;
                    self.add_part(part.key);
                    if self.status_serial != before
                        && !self.error
                        && !ctx.input(|i| i.modifiers.shift)
                    {
                        close = true;
                    }
                }
            });
        if close || modal.should_close() {
            self.library_open = false;
        }
    }

    /// The options of the picked part, and its Add button (returned when pressed).
    fn library_options(
        &mut self,
        ui: &mut Ui,
        part: &'static LibraryPart,
        bones: &[String],
        blocked: bool,
    ) -> Option<&'static LibraryPart> {
        let mut o = self.part_options(part).clone();
        let mut pressed = false;
        egui::Frame::new()
            .fill(Color32::from_rgb(17, 20, 26))
            .stroke(Stroke::new(1.0, BORDER))
            .corner_radius(12)
            .inner_margin(12)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    theme::label_caps(ui, "Options");
                    ui.add_space(6.0);
                    ui.label(RichText::new(part.title).color(TEXT).strong().size(14.0));
                    ui.label(RichText::new(part.text).color(TEXT_DIM).size(12.0));
                });
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = Vec2::new(10.0, 8.0);
                    if part.opts.is_empty() {
                        ui.label(
                            RichText::new("Nothing to choose: everything adjusts on the guides.")
                                .color(TEXT_FAINT)
                                .size(12.5),
                        );
                    }
                    let curve = !part.opts.contains(&Opt::Spline) || o.spline;
                    for opt in part.opts {
                        match *opt {
                            Opt::Sides => {
                                caption(ui, "Sides");
                                theme::segmented(
                                    ui,
                                    &mut o.sides,
                                    &[
                                        (FingerSides::Both, "Both"),
                                        (FingerSides::Left, "Left"),
                                        (FingerSides::Right, "Right"),
                                    ],
                                );
                            }
                            Opt::Hand => {
                                caption(ui, "Hand");
                                if o.sides == FingerSides::Both {
                                    o.sides = FingerSides::Right;
                                }
                                theme::segmented(
                                    ui,
                                    &mut o.sides,
                                    &[(FingerSides::Left, "Left"), (FingerSides::Right, "Right")],
                                );
                            }
                            Opt::Joints(label, min, max) => {
                                caption(ui, label);
                                ui.add(egui::DragValue::new(&mut o.joints).range(min..=max));
                            }
                            Opt::Count(label, min, max) => {
                                caption(ui, label);
                                ui.add(egui::DragValue::new(&mut o.count).range(min..=max));
                            }
                            Opt::Rows => {
                                caption(ui, "Rows");
                                ui.add(egui::DragValue::new(&mut o.rows).range(1..=3));
                            }
                            Opt::Controls => {
                                if curve {
                                    caption(ui, "Curve controls");
                                    ui.add(egui::DragValue::new(&mut o.controls).range(2..=16));
                                }
                            }
                            Opt::Parent { required } => {
                                caption(ui, "Hang from");
                                if required && o.parent.is_empty() {
                                    o.parent = "head".into();
                                }
                                let shown = if o.parent.is_empty() {
                                    "Automatic"
                                } else {
                                    o.parent.as_str()
                                };
                                egui::ComboBox::from_id_salt("library_parent")
                                    .selected_text(shown)
                                    .height(320.0)
                                    .show_ui(ui, |ui| {
                                        if !required {
                                            ui.selectable_value(
                                                &mut o.parent,
                                                String::new(),
                                                "Automatic",
                                            );
                                        }
                                        for b in bones {
                                            ui.selectable_value(&mut o.parent, b.clone(), b);
                                        }
                                    });
                            }
                            Opt::Bendy => {
                                ui.checkbox(&mut o.bendy, "Bendy").on_hover_text(
                                    "Bend controls arc every segment, and a pin moves the elbow or knee",
                                );
                            }
                            Opt::Spline => {
                                caption(ui, "Drive");
                                theme::segmented(
                                    ui,
                                    &mut o.spline,
                                    &[(false, "FK"), (true, "Curve")],
                                );
                            }
                            Opt::Id => {
                                caption(ui, "Name");
                                ui.add(
                                    egui::TextEdit::singleline(&mut o.id)
                                        .hint_text("automatic")
                                        .desired_width(110.0),
                                );
                            }
                        }
                    }
                });
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let label = match part.action {
                        library::Action::Spine => "Apply to the spine".to_string(),
                        library::Action::Head => "Select the neck and head".to_string(),
                        library::Action::Preset(_) => format!("Add {}", part.title.to_lowercase()),
                    };
                    pressed = theme::primary(ui, &label, 190.0, !blocked).clicked();
                    ui.add_space(8.0);
                    let note = if blocked {
                        format!("The rig already has {}.", part.title.to_lowercase())
                    } else {
                        "Hold Shift while adding to keep this window open.".to_string()
                    };
                    ui.label(RichText::new(note).color(TEXT_FAINT).size(12.0));
                });
            });
        *self.part_options(part) = o;
        pressed.then_some(part)
    }
}

fn picked_blocked(part: &LibraryPart, tallies: &[usize]) -> bool {
    LIBRARY
        .iter()
        .position(|p| p.key == part.key)
        .is_some_and(|i| part.single && tallies[i] > 0)
}

fn caption(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).color(TEXT_DIM));
}

fn options_hint(ui: &mut Ui) {
    egui::Frame::new()
        .fill(Color32::from_rgb(17, 20, 26))
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(12)
        .inner_margin(12)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                theme::label_caps(ui, "Options");
                ui.add_space(6.0);
                ui.label(
                    RichText::new(
                        "Pick a card to choose sides, joints and what it hangs from. \
                         The + on a card adds it with the defaults.",
                    )
                    .color(TEXT_FAINT)
                    .size(12.5),
                );
            });
        });
}

fn part_card(
    ui: &mut Ui,
    width: f32,
    part: &LibraryPart,
    tally: usize,
    selected: bool,
    blocked: bool,
) -> Option<CardClick> {
    let sense = if blocked {
        Sense::hover()
    } else {
        Sense::click()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, CARD_HEIGHT), sense);
    let hovered = response.hovered() && !blocked;
    let lit = hovered || selected;
    let painter = ui.painter();
    painter.rect_filled(rect, 14, if lit { CARD_HI } else { CARD });
    painter.rect_stroke(
        rect,
        14,
        Stroke::new(
            if selected {
                2.0
            } else if hovered {
                1.5
            } else {
                1.0
            },
            if selected {
                ACCENT
            } else if hovered {
                ACCENT.gamma_multiply(0.8)
            } else {
                BORDER
            },
        ),
        egui::StrokeKind::Inside,
    );
    let art = Rect::from_min_size(
        rect.min + Vec2::splat(6.0),
        Vec2::new(width - 12.0, ART_HEIGHT),
    );
    paint_art_backdrop(painter, art, lit);
    illustration(&painter.with_clip_rect(art), art, part.art, lit);
    // The state chip: what the rig has of this part.
    let state = match (part.edits(), tally) {
        (true, _) => Some("In rig".to_string()),
        (false, 0) => None,
        (false, _) if part.single => Some("Added".to_string()),
        (false, n) => Some(format!("{n} added")),
    };
    if let Some(text) = &state {
        let w = 14.0 + text.len() as f32 * 6.4;
        let chip = Rect::from_min_size(
            art.right_top() + Vec2::new(-w - 8.0, 8.0),
            Vec2::new(w, 20.0),
        );
        painter.rect_filled(chip, 10, Color32::from_rgb(24, 50, 40));
        painter.text(
            chip.center(),
            Align2::CENTER_CENTER,
            text,
            FontId::proportional(11.0),
            OK,
        );
    }
    let mut click = (response.clicked() && !blocked).then_some(CardClick::Pick);
    // The + adds the part as it is. Parts that change the rig's own spine or
    // head, and parts already in, don't have one.
    if !blocked && !part.edits() {
        let plus = art.left_top() + Vec2::new(16.0, 16.0);
        let hit = Rect::from_center_size(plus, Vec2::splat(26.0));
        let r = ui.interact(hit, response.id.with("plus"), Sense::click());
        let on = r.hovered();
        let painter = ui.painter();
        painter.circle_filled(
            plus,
            11.0,
            if on {
                ACCENT
            } else {
                Color32::from_black_alpha(110)
            },
        );
        let ink = if on { theme::ACCENT_TEXT } else { TEXT_DIM };
        painter.line_segment(
            [plus - Vec2::new(4.5, 0.0), plus + Vec2::new(4.5, 0.0)],
            Stroke::new(1.8, ink),
        );
        painter.line_segment(
            [plus - Vec2::new(0.0, 4.5), plus + Vec2::new(0.0, 4.5)],
            Stroke::new(1.8, ink),
        );
        if r.clicked() {
            click = Some(CardClick::Add);
        }
        r.on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(format!(
                "Add {} with the defaults",
                part.title.to_lowercase()
            ));
    }
    let painter = ui.painter();
    painter.text(
        Pos2::new(rect.left() + 14.0, art.bottom() + 10.0),
        Align2::LEFT_TOP,
        part.title,
        FontId::proportional(15.5),
        if blocked { TEXT_DIM } else { TEXT },
    );
    let galley = painter.layout(
        part.text.to_string(),
        FontId::proportional(12.0),
        TEXT_DIM,
        width - 28.0,
    );
    painter.galley(
        Pos2::new(rect.left() + 14.0, art.bottom() + 32.0),
        galley,
        TEXT_DIM,
    );
    if blocked {
        response.on_hover_text(format!("The rig already has {}", part.title.to_lowercase()));
    } else {
        response.on_hover_cursor(egui::CursorIcon::PointingHand);
    }
    click
}

/// Illustrations for the rigging window: a small mannequin, or a bust for face
/// parts, with the part the component adds picked out in honey.
fn illustration(painter: &egui::Painter, r: Rect, art: Art, hot: bool) {
    match art {
        Art::Face
        | Art::Eyes
        | Art::Jaw
        | Art::Ears
        | Art::Antlers
        | Art::Horns
        | Art::Ponytail
        | Art::FaceBoard
        | Art::NeckHead => bust(painter, r, art, hot),
        _ => figure(painter, r, art, hot),
    }
}

fn figure(painter: &egui::Painter, r: Rect, art: Art, hot: bool) {
    // A 100-unit-tall mannequin facing the viewer: its left is on screen right.
    let k = r.height() * 0.84 / 100.0;
    let origin = Pos2::new(r.center().x, r.top() + r.height() * 0.06);
    let pt = |x: f32, y: f32| origin + Vec2::new(x * k, y * k);
    let limb = Stroke::new(5.5 * k, SKIN);
    // Body.
    painter.add(egui::Shape::convex_polygon(
        vec![
            pt(-12.0, 20.0),
            pt(12.0, 20.0),
            pt(9.5, 38.0),
            pt(8.0, 53.0),
            pt(-8.0, 53.0),
            pt(-9.5, 38.0),
        ],
        SKIN,
        Stroke::NONE,
    ));
    painter.circle_filled(pt(0.0, 9.0), 7.5 * k, SKIN);
    painter.line_segment([pt(0.0, 14.0), pt(0.0, 21.0)], Stroke::new(4.0 * k, SKIN));
    let arm = |s: f32| [pt(12.0 * s, 21.0), pt(23.0 * s, 34.0), pt(32.0 * s, 46.0)];
    let leg = |s: f32| {
        [
            pt(5.5 * s, 52.0),
            pt(6.5 * s, 74.0),
            pt(7.0 * s, 94.0),
            pt(9.5 * s, 98.0),
        ]
    };
    for s in [-1.0f32, 1.0] {
        let a = arm(s);
        painter.add(egui::Shape::line(a.to_vec(), limb));
        painter.circle_filled(a[2], 3.2 * k, SKIN);
        let l = leg(s);
        painter.add(egui::Shape::line(l.to_vec(), Stroke::new(6.5 * k, SKIN)));
    }
    // Skeleton.
    let bone = Stroke::new(1.2, BONE);
    let spine = [
        pt(0.0, 52.0),
        pt(0.0, 37.0),
        pt(0.0, 22.0),
        pt(0.0, 16.0),
        pt(0.0, 9.0),
    ];
    painter.add(egui::Shape::line(spine.to_vec(), bone));
    for s in [-1.0f32, 1.0] {
        let a = arm(s);
        painter.add(egui::Shape::line(
            vec![pt(0.0, 22.0), a[0], a[1], a[2]],
            bone,
        ));
        let l = leg(s);
        painter.add(egui::Shape::line(
            vec![pt(0.0, 52.0), l[0], l[1], l[2], l[3]],
            bone,
        ));
    }
    let dot = 2.4;
    for (x, y) in [
        (0.0, 52.0),
        (0.0, 37.0),
        (0.0, 22.0),
        (0.0, 16.0),
        (0.0, 9.0),
    ] {
        joint(painter, pt(x, y), x, dot);
    }
    for s in [-1.0f32, 1.0] {
        for (x, y) in [
            (12.0, 21.0),
            (23.0, 34.0),
            (32.0, 46.0),
            (5.5, 52.0),
            (6.5, 74.0),
            (7.0, 94.0),
        ] {
            joint(painter, pt(x * s, y), x * s, dot);
        }
    }

    match art {
        Art::Fingers => {
            for s in [-1.0f32, 1.0] {
                let wrist = pt(32.0 * s, 46.0);
                painter.circle_filled(
                    wrist + Vec2::new(2.0 * s * k, 5.0 * k),
                    9.0 * k,
                    ACCENT.gamma_multiply(0.16),
                );
                for (dx, dy) in [(-1.5, 6.5), (1.0, 8.5), (3.2, 8.8), (5.2, 8.0), (6.8, 6.2)] {
                    let base = wrist + Vec2::new(1.5 * s * k, 2.5 * k);
                    let tip = wrist + Vec2::new(dx * s * k, dy * k);
                    let mid = base.lerp(tip, 0.5);
                    hot_chain(painter, &[base, mid, tip], 1.0, hot);
                }
            }
        }
        Art::Toes => {
            // Five toes fanning out from each foot's ball.
            for s in [-1.0f32, 1.0] {
                let ball = pt(9.5 * s, 97.0);
                painter.circle_filled(ball, 3.0 * k, ACCENT.gamma_multiply(0.16));
                for (dx, dy) in [
                    (3.2, 3.5),
                    (2.5, 1.2),
                    (1.2, 0.4),
                    (-0.2, -0.2),
                    (-1.5, -1.0),
                ] {
                    let tip = ball + Vec2::new(dx * s * k * 2.0, dy * k * 2.0);
                    let base = ball;
                    let mid = base.lerp(tip, 0.5);
                    hot_chain(painter, &[base, mid, tip], 1.0, hot);
                }
            }
        }
        Art::HindLegs => {
            // A zigzag hind leg on each side: thigh, calf, hock and paw.
            for s in [-1.0f32, 1.0] {
                let pts: Vec<Pos2> = [
                    (5.5, 52.0),
                    (9.5, 67.0),
                    (4.5, 82.0),
                    (7.0, 93.0),
                    (11.0, 97.0),
                ]
                .into_iter()
                .map(|(x, y)| pt(x * s, y))
                .collect();
                hot_chain(painter, &pts, 2.0, hot);
            }
        }
        Art::Tail => {
            let pts: Vec<Pos2> = [
                (3.0, 55.0),
                (9.0, 63.0),
                (16.0, 68.0),
                (23.0, 68.0),
                (28.0, 64.0),
                (31.0, 58.0),
            ]
            .into_iter()
            .map(|(x, y)| pt(x, y))
            .collect();
            hot_chain(painter, &pts, 2.2, hot);
        }
        Art::Tentacle => {
            // A wavy chain from the pelvis with square curve controls along it.
            let pts: Vec<Pos2> = (0..9)
                .map(|i| {
                    let t = i as f32 / 8.0;
                    let x = 4.0 + 40.0 * t;
                    let y = 58.0 + 16.0 * t - 7.0 * (t * std::f32::consts::TAU).sin();
                    pt(x, y)
                })
                .collect();
            hot_chain(painter, &pts, 1.4, hot);
            let honey = if hot {
                Color32::from_rgb(255, 205, 100)
            } else {
                ACCENT
            };
            for i in [0, 3, 5, 8] {
                painter.rect_stroke(
                    Rect::from_center_size(pts[i], Vec2::splat(5.5 * k)),
                    1,
                    Stroke::new(1.6, honey),
                    egui::StrokeKind::Middle,
                );
            }
        }
        Art::Prop => {
            let hand = pt(-32.0, 46.0);
            let tip = pt(-46.0, 14.0);
            let dir = (tip - hand).normalized();
            let n = Vec2::new(-dir.y, dir.x);
            let guard = hand + dir * 5.0 * k;
            painter.add(egui::Shape::line(
                vec![hand, tip],
                Stroke::new(8.0, ACCENT.gamma_multiply(0.18)),
            ));
            painter.add(egui::Shape::convex_polygon(
                vec![guard + n * 2.2 * k, tip, guard - n * 2.2 * k],
                if hot {
                    Color32::from_rgb(255, 205, 100)
                } else {
                    ACCENT
                },
                Stroke::NONE,
            ));
            painter.line_segment(
                [guard + n * 5.0 * k, guard - n * 5.0 * k],
                Stroke::new(2.5, ACCENT),
            );
            painter.circle_stroke(hand, 4.5, Stroke::new(1.8, ACCENT));
        }
        Art::Chain => {
            let pts: Vec<Pos2> = [
                (13.0, 22.0),
                (17.0, 29.0),
                (19.5, 36.5),
                (20.5, 44.0),
                (20.0, 51.0),
            ]
            .into_iter()
            .map(|(x, y)| pt(x + 4.0, y))
            .collect();
            hot_chain(painter, &pts, 1.8, hot);
        }
        Art::Arms | Art::ExtraArms => {
            for s in [-1.0f32, 1.0] {
                let extra = art == Art::ExtraArms;
                let pts: Vec<Pos2> = if extra {
                    [(9.0, 31.0), (19.0, 44.0), (27.0, 57.0), (29.0, 62.0)]
                } else {
                    [(12.0, 21.0), (23.0, 34.0), (32.0, 46.0), (34.0, 51.0)]
                }
                .into_iter()
                .map(|(x, y)| pt(x * s, y))
                .collect();
                hot_chain(painter, &pts, 2.0, hot);
            }
        }
        Art::Legs => {
            for s in [-1.0f32, 1.0] {
                let pts: Vec<Pos2> = [
                    (5.5, 52.0),
                    (6.5, 74.0),
                    (7.0, 94.0),
                    (9.5, 98.0),
                    (13.0, 98.5),
                ]
                .into_iter()
                .map(|(x, y)| pt(x * s, y))
                .collect();
                hot_chain(painter, &pts, 2.0, hot);
            }
        }
        Art::Spine => {
            let pts: Vec<Pos2> = [52.0, 45.0, 37.0, 29.0, 22.0]
                .into_iter()
                .map(|y| pt(0.0, y))
                .collect();
            hot_chain(painter, &pts, 2.6, hot);
        }
        Art::SplineSpine => {
            // An S-curve with square curve controls at the hips, middle and chest.
            let pts: Vec<Pos2> = (0..9)
                .map(|i| {
                    let t = i as f32 / 8.0;
                    pt(5.0 * (t * std::f32::consts::TAU).sin(), 52.0 - 31.0 * t)
                })
                .collect();
            hot_chain(painter, &pts, 1.8, hot);
            let honey = if hot {
                Color32::from_rgb(255, 205, 100)
            } else {
                ACCENT
            };
            for i in [0, 4, 8] {
                painter.rect_stroke(
                    Rect::from_center_size(pts[i], Vec2::splat(6.0 * k)),
                    1,
                    Stroke::new(1.6, honey),
                    egui::StrokeKind::Middle,
                );
            }
        }
        Art::Wings => {
            // A wing spread on each side: arm bones with feathers fanning from the forearm.
            let honey = if hot {
                Color32::from_rgb(255, 205, 100)
            } else {
                ACCENT
            };
            for s in [-1.0f32, 1.0] {
                let bones = [(11.0, 23.0), (24.0, 14.0), (37.0, 10.0), (46.0, 14.0)];
                let pts: Vec<Pos2> = bones.iter().map(|&(x, y)| pt(x * s, y)).collect();
                let (elbow, wrist) = (pts[1], pts[3]);
                let mut outline = vec![pts[0]];
                for f in 0..7 {
                    let t = f as f32 / 6.0;
                    let base = elbow.lerp(wrist, t);
                    let dir = Vec2::new(s * (0.35 + 0.35 * t), 1.0 - 0.35 * t).normalized();
                    let tip = base + dir * (19.0 - 5.0 * t) * k;
                    painter.line_segment([base, tip], Stroke::new(1.5, honey.gamma_multiply(0.7)));
                    outline.push(tip);
                }
                outline.push(wrist);
                painter.add(egui::Shape::convex_polygon(
                    outline,
                    ACCENT.gamma_multiply(0.1),
                    Stroke::NONE,
                ));
                hot_chain(painter, &pts, 2.0, hot);
            }
        }
        Art::ExtraLegs => {
            // Spider-style legs: the knee folds up, the foot comes down far out.
            for s in [-1.0f32, 1.0] {
                for (knee, foot) in [
                    ((20.0, 36.0), (26.0, 97.0)),
                    ((30.0, 41.0), (37.0, 97.0)),
                    ((40.0, 46.0), (47.0, 97.0)),
                ] {
                    let pts = [
                        pt(6.0 * s, 51.0),
                        pt(knee.0 * s, knee.1),
                        pt(foot.0 * s, foot.1),
                    ];
                    hot_chain(painter, &pts, 1.6, hot);
                }
            }
        }
        Art::Cape => {
            // Strips hanging from the shoulders, fanning out as they fall.
            let honey = if hot {
                Color32::from_rgb(255, 205, 100)
            } else {
                ACCENT
            };
            let mut hem = Vec::new();
            for x in [-11.0f32, -4.0, 4.0, 11.0] {
                let pts = [pt(x * 0.7, 21.0), pt(x * 1.25, 42.0), pt(x * 1.7, 66.0)];
                hot_chain(painter, &pts, 1.4, hot);
                hem.push(pts[2]);
            }
            painter.add(egui::Shape::line(
                hem,
                Stroke::new(1.2, honey.gamma_multiply(0.55)),
            ));
        }
        Art::Skirt => {
            let honey = if hot {
                Color32::from_rgb(255, 205, 100)
            } else {
                ACCENT
            };
            let mut hem = Vec::new();
            for x in [-12.0f32, -7.2, -2.4, 2.4, 7.2, 12.0] {
                let pts = [pt(x * 0.6, 52.0), pt(x * 1.25, 66.0), pt(x * 1.75, 80.0)];
                hot_chain(painter, &pts, 1.3, hot);
                hem.push(pts[2]);
            }
            painter.add(egui::Shape::line(
                hem,
                Stroke::new(1.2, honey.gamma_multiply(0.55)),
            ));
        }
        _ => {}
    }
}

fn bust(painter: &egui::Painter, r: Rect, art: Art, hot: bool) {
    // Horns and antlers rise high, so their head is smaller and sits lower.
    let tall = matches!(art, Art::Horns | Art::Antlers);
    let rad = r.height() * if tall { 0.2 } else { 0.26 };
    let shift = if art == Art::FaceBoard {
        rad * 0.95
    } else {
        0.0
    };
    let hc = r.center() - Vec2::new(shift, r.height() * if tall { -0.12 } else { 0.1 });
    let p = |x: f32, y: f32| hc + Vec2::new(x * rad, y * rad);
    // Shoulders, neck and head.
    painter.add(egui::Shape::convex_polygon(
        vec![p(-1.7, 1.55), p(1.7, 1.55), p(2.6, 2.6), p(-2.6, 2.6)],
        SKIN,
        Stroke::NONE,
    ));
    painter.rect_filled(Rect::from_min_max(p(-0.38, 0.6), p(0.38, 1.7)), 4, SKIN);
    painter.circle_filled(hc, rad, SKIN);
    // Neck and head bones.
    let bone = Stroke::new(1.2, BONE);
    painter.line_segment([p(0.0, 1.6), p(0.0, 0.25)], bone);
    painter.line_segment([p(0.0, 0.25), p(0.0, -0.95)], bone);
    joint(painter, p(0.0, 1.6), 0.0, 2.6);
    joint(painter, p(0.0, 0.25), 0.0, 2.6);

    match art {
        Art::Face => {
            // A joint on each brow, lid, cheek and lip, like the face guides.
            for sx in [-1.0f32, 1.0] {
                let brow = [p(0.18 * sx, -0.42), p(0.4 * sx, -0.5), p(0.62 * sx, -0.42)];
                hot_chain(painter, &brow, 1.4, hot);
                painter.circle_stroke(
                    p(0.4 * sx, -0.14),
                    0.15 * rad,
                    Stroke::new(1.0, Color32::from_rgb(28, 30, 36)),
                );
                for y in [-0.3, 0.02] {
                    joint(painter, p(0.4 * sx, y), sx, 2.4);
                }
                joint(painter, p(0.55 * sx, 0.3), sx, 2.6);
            }
            joint(painter, p(0.0, 0.18), 0.0, 2.6);
            let mouth = [p(-0.3, 0.6), p(0.0, 0.53), p(0.3, 0.6)];
            painter.add(egui::Shape::line(
                mouth.to_vec(),
                Stroke::new(1.4, Color32::from_rgb(28, 30, 36)),
            ));
            hot_chain(painter, &mouth, 1.4, hot);
            hot_chain(painter, &[p(0.0, 0.68)], 1.4, hot);
        }
        Art::Eyes => {
            let target = p(1.6, -1.15);
            for sx in [-1.0f32, 1.0] {
                let eye = p(0.36 * sx, -0.08);
                painter.extend(egui::Shape::dashed_line(
                    &[eye, target],
                    Stroke::new(1.2, ACCENT.gamma_multiply(0.75)),
                    4.0,
                    3.0,
                ));
                painter.circle_filled(eye, 0.19 * rad, Color32::from_rgb(222, 226, 235));
                painter.circle_filled(
                    eye + (target - eye).normalized() * 0.07 * rad,
                    0.09 * rad,
                    Color32::from_rgb(28, 30, 36),
                );
                painter.circle_stroke(
                    eye,
                    0.19 * rad + 2.0,
                    Stroke::new(
                        1.6,
                        if hot {
                            Color32::from_rgb(255, 205, 100)
                        } else {
                            ACCENT
                        },
                    ),
                );
            }
            let d = 6.0;
            painter.add(egui::Shape::convex_polygon(
                vec![
                    target + Vec2::new(0.0, -d),
                    target + Vec2::new(d, 0.0),
                    target + Vec2::new(0.0, d),
                    target + Vec2::new(-d, 0.0),
                ],
                ACCENT,
                Stroke::new(1.0, theme::ACCENT_TEXT),
            ));
        }
        Art::Jaw => {
            let outline: Vec<Pos2> = [
                (-0.86, 0.05),
                (-0.62, 0.72),
                (0.0, 1.05),
                (0.62, 0.72),
                (0.86, 0.05),
            ]
            .into_iter()
            .map(|(x, y)| p(x, y + 0.08))
            .collect();
            painter.add(egui::Shape::line(
                outline.clone(),
                Stroke::new(7.0, ACCENT.gamma_multiply(0.16)),
            ));
            painter.add(egui::Shape::line(outline, Stroke::new(2.0, ACCENT)));
            painter.add(egui::Shape::line(
                arc(p(0.0, 0.55), 0.32 * rad, -2.6, -0.54),
                Stroke::new(1.6, Color32::from_rgb(28, 30, 36)),
            ));
            hot_chain(painter, &[p(0.0, 0.12), p(0.0, 1.05)], 2.0, hot);
        }
        Art::Ears => {
            for sx in [-1.0f32, 1.0] {
                let shape = vec![p(0.88 * sx, 0.2), p(1.55 * sx, -0.95), p(1.0 * sx, -0.45)];
                painter.add(egui::Shape::convex_polygon(
                    shape,
                    SKIN,
                    Stroke::new(1.0, ACCENT.gamma_multiply(0.6)),
                ));
                hot_chain(
                    painter,
                    &[
                        p(0.92 * sx, -0.05),
                        p(1.18 * sx, -0.48),
                        p(1.45 * sx, -0.85),
                    ],
                    1.6,
                    hot,
                );
            }
        }
        Art::Antlers => {
            // A beam rising from each side of the head, a tine forking off it.
            for sx in [-1.0f32, 1.0] {
                let beam = [
                    p(0.45 * sx, -0.9),
                    p(0.8 * sx, -1.35),
                    p(1.0 * sx, -1.8),
                    p(1.05 * sx, -2.25),
                ];
                hot_chain(painter, &beam, 1.6, hot);
                hot_chain(painter, &[beam[1], p(0.5 * sx, -1.75)], 1.6, hot);
                hot_chain(painter, &[beam[2], p(1.45 * sx, -2.1)], 1.6, hot);
            }
        }
        Art::Ponytail => {
            // A hair cap, then the tail swinging down one side.
            painter.add(egui::Shape::line(
                arc(hc, rad * 1.02, 0.2, std::f32::consts::PI - 0.2),
                Stroke::new(4.0, Color32::from_rgb(86, 94, 112)),
            ));
            let pts = [
                p(0.35, -1.0),
                p(0.95, -0.85),
                p(1.3, -0.3),
                p(1.42, 0.4),
                p(1.36, 1.1),
                p(1.5, 1.75),
            ];
            hot_chain(painter, &pts, 2.0, hot);
        }
        Art::NeckHead => {
            hot_chain(
                painter,
                &[p(0.0, 1.6), p(0.0, 0.25), p(0.0, -0.95)],
                2.6,
                hot,
            );
            painter.circle_stroke(hc, rad + 3.0, Stroke::new(1.0, ACCENT.gamma_multiply(0.45)));
        }
        Art::Horns => {
            // A curved horn rising from each side of the head.
            for sx in [-1.0f32, 1.0] {
                let horn = [
                    p(0.55 * sx, -0.82),
                    p(0.95 * sx, -1.3),
                    p(1.2 * sx, -1.85),
                    p(1.05 * sx, -2.4),
                ];
                hot_chain(painter, &horn, 2.0, hot);
            }
        }
        Art::FaceBoard => {
            // The head, and the board of sliders that stands beside it.
            for sx in [-1.0f32, 1.0] {
                joint(painter, p(0.4 * sx, -0.14), sx, 2.4);
            }
            painter.line_segment(
                [p(-0.3, 0.55), p(0.3, 0.55)],
                Stroke::new(1.4, Color32::from_rgb(28, 30, 36)),
            );
            let board = Rect::from_min_max(p(1.75, -1.0), p(3.15, 1.75));
            painter.rect_filled(board, 8, Color32::from_rgb(20, 23, 30));
            painter.rect_stroke(
                board,
                8,
                Stroke::new(
                    1.4,
                    if hot {
                        Color32::from_rgb(255, 205, 100)
                    } else {
                        ACCENT
                    },
                ),
                egui::StrokeKind::Inside,
            );
            for (i, knob) in [0.3f32, 0.7, 0.45, 0.8, 0.55].into_iter().enumerate() {
                let y = board.top() + 12.0 + i as f32 * (board.height() - 24.0) / 4.0;
                let (l, rr) = (board.left() + 9.0, board.right() - 9.0);
                painter.line_segment([Pos2::new(l, y), Pos2::new(rr, y)], Stroke::new(1.2, BONE));
                painter.circle_filled(
                    Pos2::new(l + (rr - l) * knob, y),
                    3.2,
                    if hot {
                        Color32::from_rgb(255, 205, 100)
                    } else {
                        ACCENT
                    },
                );
            }
            painter.extend(egui::Shape::dashed_line(
                &[p(0.95, 0.1), Pos2::new(board.left(), board.center().y)],
                Stroke::new(1.0, ACCENT.gamma_multiply(0.6)),
                3.0,
                3.0,
            ));
        }
        _ => {}
    }
}
