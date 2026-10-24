//! The weight tools' widgets: the brush modes, the paint preview bend, locks,
//! the vertex selection and its tools in Properties, the weight table panel,
//! and, in the viewport, selecting vertices and showing the one under the
//! pointer. Everything here calls `Studio` operations (`app/skin_edit.rs`).

use eframe::egui::{
    self, Align2, Color32, FontId, Key, Modifiers, PointerButton, Pos2, Rect, RichText, Sense,
    Stroke, Ui, Vec2,
};

use hive_rig::skin::BrushMode;

use super::super::{Mode, SelectOp, Studio};
use super::{DOCK_CLEARANCE, EDGE, floating_area, panel_frame, panel_header, room};
use crate::theme::{self, ACCENT, BORDER, TEXT, TEXT_DIM, TEXT_FAINT};
use crate::view::Projector;

/// Selected vertices, drawn as dots on the mesh.
const SELECTED: Color32 = Color32::from_rgb(110, 200, 255);
const TABLE_WIDTH: f32 = 270.0;
/// Vertices listed in the weight table; the rest are counted.
const TABLE_ROWS: usize = 12;

/// One row of the weight table: bone index, bone name and weight.
type Influence = (usize, String, f32);

/// The brush modes in two rows of three, so they fit the panel.
const MODES: [[(BrushMode, &str); 3]; 2] = [
    [
        (BrushMode::Add, "Add"),
        (BrushMode::Subtract, "Subtract"),
        (BrushMode::Replace, "Replace"),
    ],
    [
        (BrushMode::Scale, "Scale"),
        (BrushMode::Smooth, "Smooth"),
        (BrushMode::Mend, "Mend"),
    ],
];

/// The brush's mark, and its colour, for each mode.
pub(super) fn brush_glyph(mode: BrushMode) -> (&'static str, Color32) {
    match mode {
        BrushMode::Add => ("+", ACCENT),
        BrushMode::Subtract => ("\u{2212}", Color32::from_rgb(110, 200, 255)),
        BrushMode::Replace => ("=", Color32::from_rgb(120, 220, 160)),
        BrushMode::Scale => ("\u{00d7}", Color32::from_rgb(255, 140, 90)),
        BrushMode::Smooth => ("~", Color32::from_rgb(190, 160, 255)),
        BrushMode::Mend => ("^", Color32::from_rgb(255, 120, 200)),
    }
}

impl Studio {
    /// Scroll Properties to put this point at the top, while a screenshot
    /// asks for the weight tools, or a picked control's look (`--tools`).
    pub(in crate::app) fn scroll_here(&mut self, ui: &mut Ui) {
        if self.skin_tools.scroll_to_tools > 0 {
            self.skin_tools.scroll_to_tools -= 1;
            ui.scroll_to_cursor(Some(egui::Align::Min));
        }
    }

    /// The vertex tools apply: a built rig, viewed past the guides stage.
    fn skin_view(&self) -> bool {
        self.mode != Mode::Guides && self.doc.as_ref().is_some_and(|d| d.rig.is_some())
    }

    /// Clicks and drags on the mesh select vertices.
    pub(super) fn select_tool_on(&self) -> bool {
        self.skin_tools.selecting && !self.painting && self.skin_view()
    }

    /// The brush mode buttons.
    pub(super) fn brush_modes(&mut self, ui: &mut Ui) {
        if !self.skin_tools.selecting {
            self.scroll_here(ui);
        }
        for row in &MODES {
            theme::segmented(ui, &mut self.brush.mode, row);
        }
    }

    /// Below the radius and strength: the value Replace and Scale use, and the
    /// bend of the painted bone for watching the skin while painting.
    pub(super) fn brush_extras(&mut self, ui: &mut Ui) {
        let label = match self.brush.mode {
            BrushMode::Replace => Some(("Value", "The weight the brush sets.")),
            BrushMode::Scale => Some((
                "Factor",
                "What one full-strength dab multiplies the weight by.",
            )),
            _ => None,
        };
        if let Some((label, tip)) = label {
            ui.horizontal(|ui| {
                ui.label(RichText::new(label).size(12.0).color(TEXT_DIM));
                ui.add(egui::Slider::new(&mut self.brush.value, 0.0..=1.0).max_decimals(2))
                    .on_hover_text(tip);
            });
        }
        if let Some(axis) = self.paint_bend_axis() {
            let (mut bend, mut new_axis) = (self.paint_bend(), axis);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Bend").size(12.0).color(TEXT_DIM));
                let r = ui
                    .add(
                        egui::Slider::new(&mut bend, 0.0..=90.0)
                            .suffix("°")
                            .max_decimals(0),
                    )
                    .on_hover_text(
                        "Bend the painted bone at its joint to watch the skin deform while you \
                         paint. Nothing is saved.",
                    );
                if r.changed() {
                    self.bend_paint_bone(bend, None);
                }
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("about").size(12.0).color(TEXT_DIM));
                if theme::segmented(ui, &mut new_axis, &[(0, "X"), (1, "Y"), (2, "Z")]) {
                    self.bend_paint_bone(bend, Some(new_axis));
                }
            });
        }
    }

    /// Locks, whole-mesh fixes, re-running the automatic weights, and the
    /// vertex selection with its tools.
    pub(super) fn weight_tools(&mut self, ui: &mut Ui) {
        let small = |t: &str| egui::Button::new(RichText::new(t).size(12.5)).corner_radius(8);
        let shown = self.weights_bone.clone();
        ui.add_space(4.0);
        theme::label_caps(ui, "Locks");
        ui.horizontal(|ui| {
            match &shown {
                Some(bone) => {
                    let locked = self.skin_tools.locked.contains(bone);
                    let mut on = locked;
                    theme::pill(ui, &mut on, if locked { "Locked" } else { "Lock" }).on_hover_text(
                        "Keep the shown bone's weights as they are: painting other bones, \
                         smoothing and the tools below leave them alone.",
                    );
                    if on != locked {
                        self.toggle_lock(bone);
                    }
                }
                None => {
                    ui.label(
                        RichText::new("Pick a bone to lock it.")
                            .size(12.0)
                            .color(TEXT_FAINT),
                    );
                }
            }
            let n = self.skin_tools.locked.len();
            if n > 0 && ui.add(small(&format!("Unlock all ({n})"))).clicked() {
                self.unlock_all();
            }
        });
        if !self.skin_tools.locked.is_empty() {
            let names: Vec<&str> = self.skin_tools.locked.iter().map(String::as_str).collect();
            ui.label(RichText::new(names.join(", ")).size(11.5).color(TEXT_FAINT));
        }

        ui.add_space(4.0);
        theme::label_caps(ui, "Fix");
        ui.horizontal(|ui| {
            if ui
                .add(small("Normalize"))
                .on_hover_text("Make every vertex add up to 1 within the influence limits.")
                .clicked()
            {
                self.normalize_weights();
            }
            if ui
                .add(small("Fix spikes"))
                .on_hover_text(
                    "Every vertex whose weights stand out from its neighbours' takes their \
                     average instead.",
                )
                .clicked()
            {
                self.fix_spikes();
            }
            let auto = ui.add_enabled(shown.is_some(), small("Auto weights"));
            if auto
                .on_hover_text(
                    "Put the automatic weights back wherever the shown bone has weight. \
                     Auto under Selection does it on the selected vertices.",
                )
                .clicked()
            {
                self.reauto_bone();
            }
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new("Spike above").size(12.0).color(TEXT_DIM));
            ui.add(egui::Slider::new(&mut self.skin_tools.spike, 0.2..=0.95).max_decimals(2))
                .on_hover_text(
                    "How far a vertex must differ from its neighbours' average to count as a \
                     spike: 0.5 is a hard edge between two bones, 1 a vertex sharing no bone \
                     with them.",
                );
        });

        ui.add_space(4.0);
        if self.skin_tools.selecting {
            self.scroll_here(ui);
        }
        theme::label_caps(ui, "Selection");
        ui.horizontal(|ui| {
            let mut on = self.skin_tools.selecting;
            theme::pill(ui, &mut on, "Select").on_hover_text(
                "Click vertices on the mesh, or drag a box (Shift adds, Ctrl removes, \
                 Alt-drag orbits). Ctrl+C and Ctrl+V copy and paste weights.",
            );
            if on != self.skin_tools.selecting {
                self.skin_tools.selecting = on;
                if on {
                    self.painting = false;
                    self.brush_at = None;
                }
            }
            let n = self.skin_tools.selected_count();
            ui.label(
                RichText::new(format!("{n} selected"))
                    .size(12.0)
                    .color(TEXT_DIM),
            );
        });
        let any = self.skin_tools.selected_count() > 0;
        ui.horizontal(|ui| {
            if ui.add_enabled(any, small("Grow")).clicked() {
                self.grow_selection();
            }
            if ui.add_enabled(any, small("Shrink")).clicked() {
                self.shrink_selection();
            }
            if ui.add_enabled(any, small("None")).clicked() {
                self.select_none();
            }
            if ui
                .add_enabled(shown.is_some(), small("Its skin"))
                .on_hover_text("Select every vertex the shown bone moves.")
                .clicked()
            {
                self.select_bone_skin();
            }
        });
        ui.horizontal(|ui| {
            if ui.add_enabled(any, small("Copy")).clicked() {
                self.copy_vertex_weights();
            }
            let can_paste = any && self.skin_tools.clipboard.is_some();
            if ui.add_enabled(can_paste, small("Paste")).clicked() {
                self.paste_vertex_weights();
            }
            if ui
                .add_enabled(any, small("Auto"))
                .on_hover_text("Put the automatic weights back on the selected vertices.")
                .clicked()
            {
                self.reauto_selection();
            }
        });
        ui.horizontal(|ui| {
            let flood = ui
                .add_enabled(any && shown.is_some(), small("Flood"))
                .on_hover_text("Set the shown bone to this weight on every selected vertex.");
            if flood.clicked() {
                self.flood_weight();
            }
            ui.add(egui::Slider::new(&mut self.skin_tools.flood, 0.0..=1.0).max_decimals(2));
        });
    }

    /// Selecting vertices and the vertex under the pointer, in the viewport.
    /// Returns whether a box selection holds the drag (so it doesn't orbit).
    pub(super) fn skin_input(
        &mut self,
        ui: &Ui,
        response: &egui::Response,
        pr: &Projector,
        pointer: Option<Pos2>,
        press: Option<Pos2>,
        blocked: bool,
    ) -> bool {
        let watch = self.skin_view() && (self.weights_bone.is_some() || self.skin_tools.selecting);
        let moving = ui.input(|i| i.pointer.is_moving());
        // Only a moving pointer looks again (the ray test walks every triangle).
        if !watch {
            self.skin_tools.hover = None;
        } else if moving {
            self.skin_tools.hover = pointer.filter(|_| response.hovered()).and_then(|p| {
                let (o, d) = pr.ray(p);
                self.hit_vertex(o, d)
            });
        }
        if !self.select_tool_on() {
            self.skin_tools.box_from = None;
            return false;
        }
        let (alt, shift, command) =
            ui.input(|i| (i.modifiers.alt, i.modifiers.shift, i.modifiers.command));
        let op = if command {
            SelectOp::Remove
        } else if shift {
            SelectOp::Add
        } else {
            SelectOp::Replace
        };
        if !blocked {
            if ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::C)) {
                self.copy_vertex_weights();
            }
            if ui.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::V)) {
                self.paste_vertex_weights();
            }
        }
        if response.drag_started_by(PointerButton::Primary) && !alt {
            self.skin_tools.box_from = press.or(pointer);
        }
        if self.skin_tools.box_from.is_some() && pointer.is_some() {
            self.skin_tools.box_to = pointer;
        }
        if self.skin_tools.box_from.is_some() && response.drag_stopped() {
            if let (Some(a), Some(b)) = (self.skin_tools.box_from, self.skin_tools.box_to) {
                self.box_select(pr, Rect::from_two_pos(a, b), op);
            }
            self.skin_tools.box_from = None;
            return true;
        }
        if response.clicked()
            && let Some(p) = pointer
        {
            let (o, d) = pr.ray(p);
            match self.hit_vertex(o, d) {
                Some(v) => self.select_vertices(&[v], op),
                None if op == SelectOp::Replace => self.select_none(),
                None => {}
            }
        }
        self.skin_tools.box_from.is_some()
    }

    /// The selection, the box being dragged and the vertex under the pointer.
    pub(super) fn paint_skin_tools(&self, painter: &egui::Painter, pr: &Projector) {
        if !self.skin_view() {
            return;
        }
        let Some(positions) = self.shown_positions() else {
            return;
        };
        let sel = &self.skin_tools.selected;
        if sel.iter().any(|s| *s) {
            for (v, p) in positions.iter().enumerate() {
                if sel.get(v).copied().unwrap_or(false)
                    && let Some(s) = pr.pos(*p)
                {
                    painter.circle(s, 3.2, SELECTED, Stroke::new(1.0, Color32::BLACK));
                }
            }
        }
        if let (Some(a), Some(b)) = (self.skin_tools.box_from, self.skin_tools.box_to) {
            let r = Rect::from_two_pos(a, b);
            painter.rect_filled(r, 2, SELECTED.gamma_multiply(0.08));
            painter.rect_stroke(r, 2, Stroke::new(1.2, SELECTED), egui::StrokeKind::Inside);
        }
        if let Some(v) = self.skin_tools.hover
            && let Some(s) = positions.get(v).and_then(|p| pr.pos(*p))
        {
            painter.circle_stroke(s, 5.0, Stroke::new(1.6, Color32::WHITE));
            self.vertex_card(painter, s + Vec2::new(14.0, 14.0), v);
        }
    }

    /// A vertex's influences, strongest first, in a small card at `at`.
    fn vertex_card(&self, painter: &egui::Painter, at: Pos2, v: usize) {
        let Some(rig) = self.doc.as_ref().and_then(|d| d.rig.as_ref()) else {
            return;
        };
        let font = FontId::proportional(12.5);
        let mut infl: Vec<(usize, f32)> = rig.skin.influences(v).collect();
        infl.sort_by(|a, b| b.1.total_cmp(&a.1));
        let rows: Vec<(String, String, bool)> = infl
            .into_iter()
            .map(|(j, w)| {
                let name = rig.skeleton.bones[j].name.clone();
                let shown = self.weights_bone.as_deref() == Some(name.as_str());
                let name = if self.skin_tools.locked.contains(&name) {
                    format!("{name} (locked)")
                } else {
                    name
                };
                (name, format!("{w:.3}"), shown)
            })
            .collect();
        let title = format!("vertex {v}");
        let line = 17.0;
        let name_w = rows
            .iter()
            .map(|r| {
                painter
                    .layout_no_wrap(r.0.clone(), font.clone(), TEXT)
                    .size()
                    .x
            })
            .fold(70.0f32, f32::max);
        let size = Vec2::new(name_w + 76.0, line * (rows.len() as f32 + 1.0) + 12.0);
        // Right of the pointer, or left of it on the right half of the view,
        // so the card stays clear of the Properties panel.
        let clip = painter.clip_rect();
        let at = if at.x > clip.center().x {
            at - Vec2::new(size.x + 28.0, 0.0)
        } else {
            at
        };
        let rect = Rect::from_min_size(at, size);
        painter.rect_filled(rect, 8, Color32::from_rgba_unmultiplied(14, 16, 20, 240));
        painter.rect_stroke(rect, 8, Stroke::new(1.0, BORDER), egui::StrokeKind::Inside);
        let left = rect.left() + 8.0;
        let mut y = rect.top() + 6.0;
        painter.text(
            Pos2::new(left, y),
            Align2::LEFT_TOP,
            title,
            font.clone(),
            TEXT_DIM,
        );
        for (name, w, shown) in rows {
            y += line;
            let color = if shown { ACCENT } else { TEXT };
            painter.text(
                Pos2::new(left, y),
                Align2::LEFT_TOP,
                name,
                font.clone(),
                color,
            );
            painter.text(
                Pos2::new(rect.right() - 8.0, y),
                Align2::RIGHT_TOP,
                w,
                font.clone(),
                color,
            );
        }
    }

    /// The weight table: each selected vertex's influences, editable.
    pub(super) fn weight_table(&mut self, ctx: &egui::Context) {
        if !self.skin_view() || self.skin_tools.selected_count() == 0 {
            return;
        }
        let sel = self.skin_tools.selection();
        let rows: Vec<(usize, Vec<Influence>)> = {
            let rig = self.doc.as_ref().unwrap().rig.as_ref().unwrap();
            sel.iter()
                .take(TABLE_ROWS)
                .map(|&v| {
                    let mut infl: Vec<(usize, String, f32)> = rig
                        .skin
                        .influences(v)
                        .map(|(j, w)| (j, rig.skeleton.bones[j].name.clone(), w))
                        .collect();
                    infl.sort_by(|a, b| b.2.total_cmp(&a.2));
                    (v, infl)
                })
                .collect()
        };
        // The room between the outliner and the dock (the header and the note
        // under the list take about 90 points).
        let outliner_bottom = ctx
            .memory(|m| m.area_rect(egui::Id::unique("outliner")))
            .map_or(super::PANEL_TOP, |r| r.bottom());
        let bottom = ctx.content_rect().bottom() - DOCK_CLEARANCE;
        let max_h = (bottom - outliner_bottom - 12.0 - 90.0).max(120.0);
        floating_area(
            "weight_table",
            Align2::LEFT_BOTTOM,
            Vec2::new(EDGE, -DOCK_CLEARANCE),
        )
        .show(ctx, |ui| {
            room(ui, TABLE_WIDTH, max_h + 60.0);
            panel_frame().show(ui, |ui| {
                ui.set_width(TABLE_WIDTH);
                let mut open = true;
                panel_header(
                    ui,
                    "Weight table",
                    &format!("{} vertices", sel.len()),
                    &mut open,
                );
                if !open {
                    self.select_none();
                    return;
                }
                egui::ScrollArea::vertical()
                    .max_height(max_h)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        for (v, infl) in &rows {
                            ui.label(
                                RichText::new(format!("vertex {v}"))
                                    .size(11.5)
                                    .color(TEXT_FAINT),
                            );
                            for (bone, name, w) in infl {
                                self.table_cell(ui, *v, *bone, name, *w);
                            }
                            ui.add_space(4.0);
                        }
                        if sel.len() > TABLE_ROWS {
                            ui.label(
                                RichText::new(format!(
                                    "and {} more; Flood sets a bone on all of them.",
                                    sel.len() - TABLE_ROWS
                                ))
                                .size(11.5)
                                .color(TEXT_FAINT),
                            );
                        }
                    });
                ui.label(
                    RichText::new(
                        "Drag or type a weight; the vertex's other unlocked bones share the rest.",
                    )
                    .size(11.0)
                    .color(TEXT_FAINT),
                );
            });
        });
    }

    /// One editable influence: the bone, a bar and its weight.
    fn table_cell(&mut self, ui: &mut Ui, v: usize, bone: usize, name: &str, w: f32) {
        let locked = self.skin_tools.locked.contains(name);
        ui.horizontal(|ui| {
            let shown = self.weights_bone.as_deref() == Some(name);
            let color = if shown { ACCENT } else { TEXT };
            let label = if locked {
                format!("{name} (locked)")
            } else {
                name.to_string()
            };
            let (rect, _) = ui.allocate_exact_size(Vec2::new(140.0, 18.0), Sense::hover());
            let bar = Rect::from_min_size(
                rect.left_bottom() - Vec2::new(0.0, 3.0),
                Vec2::new(rect.width() * w, 2.0),
            );
            ui.painter().rect_filled(bar, 1, color.gamma_multiply(0.6));
            ui.painter().text(
                rect.left_center(),
                Align2::LEFT_CENTER,
                label,
                FontId::proportional(12.5),
                color,
            );
            let mut value = w;
            let r = ui.add_enabled(
                !locked,
                egui::DragValue::new(&mut value)
                    .range(0.0..=1.0)
                    .speed(0.005)
                    .max_decimals(3),
            );
            if r.drag_started() || r.gained_focus() {
                self.skin_tools.table_step = true;
            }
            if r.changed() && (value - w).abs() > 1e-6 {
                let new_step = std::mem::replace(&mut self.skin_tools.table_step, false)
                    || !(r.dragged() || r.has_focus());
                self.set_weight_on(&[v], bone, value, new_step);
            }
        });
    }
}
