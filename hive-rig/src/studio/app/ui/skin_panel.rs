//! The weight tools' widgets: the brush modes, the paint preview bend, locks,
//! the vertex selection and its tools in Properties, the weight table panel,
//! and, in the viewport, selecting vertices and showing the one under the
//! pointer. Everything here calls `Studio` operations (`app/skin_edit.rs`).

use eframe::egui::{
    self, Align2, Color32, FontId, Key, Modifiers, PointerButton, Pos2, Rect, RichText, Sense,
    Stroke, Ui, Vec2,
};

use hive_rig::shapes::SculptBrush;
use hive_rig::skeleton::Side;
use hive_rig::skin::BrushMode;

use super::super::{MeshTool, Mode, SelectOp, Studio};
use super::{DOCK_CLEARANCE, EDGE, floating_area, panel_frame, panel_header, room};
use crate::theme::{self, ACCENT, BORDER, TEXT, TEXT_DIM, TEXT_FAINT, WARN};
use crate::view::Projector;

/// Selected vertices, drawn as dots on the mesh.
const SELECTED: Color32 = Color32::from_rgb(110, 200, 255);
/// The bone list's height and the height of one of its rows.
const BONE_LIST_HEIGHT: f32 = 176.0;
const BONE_ROW: f32 = 19.0;
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

/// The sculpt brush's mark and colour.
fn sculpt_glyph(brush: SculptBrush) -> (&'static str, Color32) {
    match brush {
        SculptBrush::Grab => ("\u{2194}", Color32::from_rgb(255, 190, 110)),
        SculptBrush::Smooth => ("~", Color32::from_rgb(190, 160, 255)),
        SculptBrush::Relax => ("\u{2248}", Color32::from_rgb(120, 220, 200)),
        SculptBrush::Inflate => ("\u{25cb}", Color32::from_rgb(255, 150, 120)),
        SculptBrush::Erase => ("\u{2715}", Color32::from_rgb(150, 170, 200)),
    }
}

fn mode_label(mode: BrushMode) -> &'static str {
    MODES
        .iter()
        .flatten()
        .find(|(m, _)| *m == mode)
        .map_or("Add", |(_, l)| l)
}

/// One line on what a drag does with this mesh tool.
fn mesh_tool_tip(tool: MeshTool) -> &'static str {
    match tool {
        MeshTool::View => "Drags orbit; pick a tool to edit the skin or shapes.",
        MeshTool::Paint => "Drag on the mesh to paint the picked bone.",
        MeshTool::Select => "Click or box-select vertices to edit their weights exactly.",
        MeshTool::Sculpt => "Drag on the mesh to sculpt the picked shape.",
    }
}

/// A small padlock, shut when `locked`.
fn paint_lock(painter: &egui::Painter, c: Pos2, locked: bool, color: Color32) {
    let body = Rect::from_center_size(c + Vec2::new(0.0, 2.0), Vec2::new(9.0, 6.5));
    painter.rect_filled(body, 1.5, color);
    let lift = if locked { 0.0 } else { 2.5 };
    let (l, r) = (body.left() + 2.0, body.right() - 2.0);
    let top = body.top() - 4.0 - lift;
    let shackle = vec![
        Pos2::new(l, body.top()),
        Pos2::new(l, top + 1.5),
        Pos2::new(l + 1.0, top),
        Pos2::new(r - 1.0, top),
        Pos2::new(r, top + 1.5),
        Pos2::new(r, body.top() - if locked { 0.0 } else { lift }),
    ];
    painter.add(egui::Shape::line(shackle, Stroke::new(1.4, color)));
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
                self.set_mesh_tool(if on { MeshTool::Select } else { MeshTool::View });
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
        let watch = self.skin_view()
            && (self.weights_bone.is_some()
                || self.skin_tools.color_all
                || self.skin_tools.selecting);
        let moving = ui.input(|i| i.pointer.is_moving());
        // Only a moving pointer looks again (the ray test walks every triangle).
        if !watch {
            self.skin_tools.hover = None;
        } else if moving {
            self.skin_tools.hover = pointer.filter(|_| response.hovered()).and_then(|p| {
                let (o, d) = pr.ray(p);
                self.hit_vertex(o, d)
            });
            if self.skin_tools.hover.is_some() {
                self.skin_tools.inspect = self.skin_tools.hover;
            }
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

    /// The Skin weights card under the rig's Properties: the mesh tool, the
    /// bone list, the brush, mirror and copy, the vertex inspector and the
    /// other weight tools.
    pub(super) fn skin_card(&mut self, ui: &mut Ui, deform: &[String]) {
        if !self.skin_tools.selecting {
            self.scroll_here(ui);
        }
        self.mesh_tool_bar(ui);
        ui.add_space(4.0);
        self.bone_list(ui, deform);
        if !self.skin_tools.color_all {
            super::weight_legend(ui);
        }
        ui.add_space(4.0);
        theme::label_caps(ui, "Brush");
        self.brush_controls(ui);
        ui.add_space(4.0);
        theme::label_caps(ui, "Mirror and transfer");
        self.mirror_row(ui);
        ui.add_space(4.0);
        self.vertex_inspector(ui);
        self.weight_tools(ui);
        ui.checkbox(
            &mut self.keep_weights,
            RichText::new("Keep edited weights on rebuild").size(12.5),
        )
        .on_hover_text(
            "Painted, mirrored and smoothed weights stay when the rig is built again, \
             except where a new bone takes the vertex.",
        );
    }

    /// What a drag on the mesh does, as one row of buttons, and a line on
    /// how to use the tool that is on.
    pub(super) fn mesh_tool_bar(&mut self, ui: &mut Ui) {
        theme::label_caps(ui, "Mesh tool");
        let now = self.mesh_tool();
        let mut tool = now;
        let options: Vec<(MeshTool, &str)> =
            MeshTool::ALL.iter().map(|t| (*t, t.label())).collect();
        theme::segmented(ui, &mut tool, &options);
        if tool != now {
            self.set_mesh_tool(tool);
        }
        ui.label(
            RichText::new(mesh_tool_tip(self.mesh_tool()))
                .size(11.5)
                .color(TEXT_FAINT),
        );
    }

    /// The deform bones, searchable: click one to show (and paint) its
    /// weights. Each row has the bone's colour in the all-bones view, its
    /// weight on the inspected vertex as a bar, and a lock.
    fn bone_list(&mut self, ui: &mut Ui, deform: &[String]) {
        let Some(rig) = self.doc.as_ref().and_then(|d| d.rig.as_ref()) else {
            return;
        };
        let inspected = self.inspected_vertex();
        let rows: Vec<(usize, String, f32)> = deform
            .iter()
            .filter_map(|name| {
                let j = rig.skeleton.index_of(name)?;
                let w = inspected.map_or(0.0, |v| rig.skin.weight(v, j));
                Some((j, name.clone(), w))
            })
            .collect();
        ui.horizontal(|ui| {
            let search = ui.available_width() - 104.0;
            ui.add(
                egui::TextEdit::singleline(&mut self.skin_tools.bone_filter)
                    .hint_text("Search bones")
                    .desired_width(search),
            );
            let before = self.skin_tools.color_all;
            theme::segmented(
                ui,
                &mut self.skin_tools.color_all,
                &[(false, "Bone"), (true, "All")],
            );
            if before != self.skin_tools.color_all {
                self.weight_colors = None;
            }
        });
        let filter = self.skin_tools.bone_filter.to_lowercase();
        let shown: Vec<&(usize, String, f32)> = rows
            .iter()
            .filter(|r| filter.is_empty() || r.1.to_lowercase().contains(&filter))
            .collect();
        let mut pick: Option<Option<String>> = None;
        let mut lock: Option<String> = None;
        egui::ScrollArea::vertical()
            .id_salt("bone_list")
            .max_height(BONE_LIST_HEIGHT)
            .min_scrolled_height(BONE_LIST_HEIGHT)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                for (j, name, w) in shown.iter().map(|r| (r.0, &r.1, r.2)) {
                    let on = self.weights_bone.as_deref() == Some(name.as_str());
                    let locked = self.skin_tools.locked.contains(name);
                    let (rect, r) = ui.allocate_exact_size(
                        Vec2::new(ui.available_width(), BONE_ROW),
                        Sense::click(),
                    );
                    let p = ui.painter();
                    if on {
                        p.rect_filled(rect, 4, ACCENT.gamma_multiply(0.16));
                    } else if r.hovered() {
                        p.rect_filled(rect, 4, Color32::from_white_alpha(10));
                    }
                    let sw = Rect::from_center_size(
                        Pos2::new(rect.left() + 9.0, rect.center().y),
                        Vec2::splat(9.0),
                    );
                    p.rect_filled(sw, 2, crate::view::bone_color(j));
                    if w > 0.0 {
                        // The inspected vertex's weight on this bone, as a bar
                        // in the heat-map colour.
                        let bar = Rect::from_min_size(
                            Pos2::new(rect.left() + 20.0, rect.bottom() - 3.0),
                            Vec2::new((rect.width() - 76.0) * w, 2.0),
                        );
                        p.rect_filled(bar, 1, crate::view::weight_color(w));
                        p.text(
                            Pos2::new(rect.right() - 26.0, rect.center().y),
                            Align2::RIGHT_CENTER,
                            format!("{w:.2}"),
                            FontId::proportional(11.5),
                            TEXT_DIM,
                        );
                    }
                    p.text(
                        Pos2::new(rect.left() + 20.0, rect.center().y),
                        Align2::LEFT_CENTER,
                        name,
                        FontId::proportional(12.5),
                        if on { ACCENT } else { TEXT },
                    );
                    let lock_rect = Rect::from_center_size(
                        Pos2::new(rect.right() - 10.0, rect.center().y),
                        Vec2::splat(BONE_ROW - 2.0),
                    );
                    let lr = ui
                        .interact(
                            lock_rect,
                            egui::Id::unique(("bone_lock", j)),
                            Sense::click(),
                        )
                        .on_hover_text(if locked {
                            "Locked: edits leave this bone's weights alone. Click to unlock."
                        } else {
                            "Lock this bone's weights."
                        });
                    let lock_color = if locked {
                        ACCENT
                    } else if lr.hovered() || r.hovered() {
                        TEXT_DIM
                    } else {
                        Color32::from_white_alpha(28)
                    };
                    paint_lock(ui.painter(), lock_rect.center(), locked, lock_color);
                    if lr.clicked() {
                        lock = Some(name.clone());
                    } else if r.clicked() {
                        pick = Some((!on).then(|| name.clone()));
                    }
                }
            });
        if shown.len() < rows.len() {
            ui.label(
                RichText::new(format!(
                    "{} of {} bones match the search.",
                    shown.len(),
                    rows.len()
                ))
                .size(11.5)
                .color(TEXT_FAINT),
            );
        } else if self.weights_bone.is_none() {
            ui.label(
                RichText::new("Click a bone to show its weights and paint it.")
                    .size(11.5)
                    .color(TEXT_FAINT),
            );
        }
        if let Some(name) = lock {
            self.toggle_lock(&name);
        }
        if let Some(bone) = pick {
            self.weights_bone = bone;
        }
    }

    /// The vertex whose weights the inspector and the bone list show: the
    /// first selected one, else the last one under the pointer.
    pub(super) fn inspected_vertex(&self) -> Option<usize> {
        let count = self
            .doc
            .as_ref()
            .and_then(|d| d.rig.as_ref())
            .map_or(0, |r| r.skin.vertex_count());
        self.skin_tools
            .selected
            .iter()
            .position(|s| *s)
            .or(self.skin_tools.inspect)
            .filter(|v| *v < count)
    }

    /// Mirror either way, smooth, and copy the weights of another rig.
    fn mirror_row(&mut self, ui: &mut Ui) {
        let small = |t: &str| egui::Button::new(RichText::new(t).size(12.5)).corner_radius(8);
        ui.horizontal_wrapped(|ui| {
            if ui
                .add(small("Mirror L to R"))
                .on_hover_text("Copy the left side's weights onto the right side's twin bones.")
                .clicked()
            {
                self.mirror_weights(Side::Left);
            }
            if ui
                .add(small("Mirror R to L"))
                .on_hover_text("Copy the right side's weights onto the left side's twin bones.")
                .clicked()
            {
                self.mirror_weights(Side::Right);
            }
            let what = if self.weights_bone.is_some() {
                "the shown bone"
            } else {
                "every bone"
            };
            if ui
                .add(small("Smooth"))
                .on_hover_text(format!("Blur the weights of {what} with their neighbours."))
                .clicked()
            {
                self.smooth_weights();
            }
            if ui
                .add(small("Copy from rig\u{2026}"))
                .on_hover_text(
                    "Take the weights of another rig (a .hive.json, this character or another \
                     with a similar skeleton), matching bones by name.",
                )
                .clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .set_title("Copy weights from a rig")
                    .add_filter("Hive rig", &["json"])
                    .pick_file()
            {
                self.copy_weights_from(&path);
            }
        });
    }

    /// The inspected vertex's influences, each editable, with their total.
    fn vertex_inspector(&mut self, ui: &mut Ui) {
        theme::label_caps(ui, "Vertex");
        let Some(v) = self.inspected_vertex() else {
            ui.label(
                RichText::new(
                    "Point at the mesh (with a bone shown or Select on) to inspect a vertex.",
                )
                .size(11.5)
                .color(TEXT_FAINT),
            );
            return;
        };
        let Some(rig) = self.doc.as_ref().and_then(|d| d.rig.as_ref()) else {
            return;
        };
        let mut infl: Vec<Influence> = rig
            .skin
            .influences(v)
            .map(|(j, w)| (j, rig.skeleton.bones[j].name.clone(), w))
            .collect();
        infl.sort_by(|a, b| b.2.total_cmp(&a.2));
        let total: f32 = infl.iter().map(|i| i.2).sum();
        let selected = self.skin_tools.selected.get(v).copied().unwrap_or(false);
        let (limit, n) = (rig.guides.skin.max_influences, infl.len());
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!(
                    "{} {v} \u{00b7} {n} of {limit} bones \u{00b7} sum {total:.3}",
                    if selected { "selected" } else { "vertex" }
                ))
                .size(12.0)
                .color(if (total - 1.0).abs() > 1e-3 {
                    WARN
                } else {
                    TEXT_DIM
                }),
            );
            if !selected
                && ui
                    .add(egui::Button::new(RichText::new("Select").size(12.0)).corner_radius(8))
                    .on_hover_text("Select this vertex, so the inspector stays on it.")
                    .clicked()
            {
                self.select_vertices(&[v], SelectOp::Replace);
            }
        });
        for (bone, name, w) in &infl {
            self.table_cell(ui, v, *bone, name, *w);
        }
    }

    /// The weight brush: what it does, its size and strength.
    fn brush_controls(&mut self, ui: &mut Ui) {
        self.brush_modes(ui);
        let height = self
            .doc
            .as_ref()
            .map_or(1.8, |d| d.guides.body.height)
            .max(1e-3);
        let mut cm = self.brush.radius * height * 100.0;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Radius").size(12.0).color(TEXT_DIM));
            let max = 0.25 * height * 100.0;
            if ui
                .add(
                    egui::Slider::new(&mut cm, 0.5..=max)
                        .suffix(" cm")
                        .max_decimals(1),
                )
                .on_hover_text("[ and ] resize the brush in the viewport.")
                .changed()
            {
                self.brush.radius = cm / 100.0 / height;
            }
        });
        ui.horizontal(|ui| {
            ui.label(RichText::new("Strength").size(12.0).color(TEXT_DIM));
            ui.add(egui::Slider::new(&mut self.brush.strength, 0.01..=1.0).max_decimals(2));
        });
        self.brush_extras(ui);
    }

    /// The brush on the mesh, before it touches: its reach, how strongly
    /// each vertex under it would change (dots that fade with the falloff),
    /// its strength as an arc round the ring, and what it works on.
    pub(super) fn paint_brush(&self, painter: &egui::Painter, pr: &Projector) {
        let (Some(at), Some(doc)) = (self.brush_at, &self.doc) else {
            return;
        };
        let sculpt = !self.painting && self.sculpting;
        let (reach, strength) = if sculpt {
            (self.sculpt_radius, self.shape_tools.strength)
        } else {
            (self.brush.radius, self.brush.strength)
        };
        let radius = reach * doc.guides.body.height;
        let right = pr.forward.cross(glam::Vec3::Y).normalize_or(glam::Vec3::X);
        let (Some(c), Some(edge)) = (pr.pos(at), pr.pos(at + right * radius)) else {
            return;
        };
        let r = c.distance(edge).max(2.0);
        let (glyph, color) = if sculpt {
            sculpt_glyph(self.shape_tools.brush)
        } else {
            brush_glyph(self.brush.mode)
        };
        let grab = sculpt && self.shape_tools.brush == SculptBrush::Grab;
        // Grab moves vertices by the whole drag; the others by their strength.
        let strength = if grab { 1.0 } else { strength };
        if let Some(positions) = self.shown_positions() {
            for p in &positions {
                let f = hive_rig::shapes::falloff(p.distance(at), radius) * strength;
                if f > 0.02
                    && let Some(s) = pr.pos(*p)
                {
                    painter.circle_filled(s, 1.2 + 1.6 * f, color.gamma_multiply(0.25 + 0.75 * f));
                }
            }
        }
        painter.circle_filled(c, r, color.gamma_multiply(0.06 + 0.10 * strength));
        // The inner ring shows where the falloff is still above half.
        painter.circle_stroke(c, r * 0.54, Stroke::new(1.0, color.gamma_multiply(0.5)));
        painter.circle_stroke(c, r, Stroke::new(1.4, color.gamma_multiply(0.7)));
        // Strength: an arc from the top, clockwise, a full turn at 1.
        let steps = 48;
        let turn = strength.clamp(0.0, 1.0) * std::f32::consts::TAU;
        let arc: Vec<Pos2> = (0..=steps)
            .map(|i| {
                let a = -std::f32::consts::FRAC_PI_2 + turn * i as f32 / steps as f32;
                c + Vec2::new(a.cos(), a.sin()) * (r + 3.0)
            })
            .collect();
        if !grab {
            painter.add(egui::Shape::line(arc, Stroke::new(3.0, color)));
        }
        painter.circle_filled(c, 2.5, color);
        painter.text(
            c + Vec2::new(r * 0.72 + 6.0, -r * 0.72 - 6.0),
            Align2::CENTER_CENTER,
            glyph,
            FontId::proportional(18.0),
            color,
        );
        let what = if sculpt {
            self.shape_pick
                .clone()
                .map(|s| match self.shape_tools.target {
                    Some(w) => format!("{s} @ {w:.2}"),
                    None => s,
                })
        } else {
            self.weights_bone.clone()
        };
        let size = format!("{:.1} cm", radius * 100.0);
        let chip = match (what, grab) {
            (Some(w), true) => format!("{w}  \u{00b7}  {size}"),
            (Some(w), false) => format!("{w}  \u{00b7}  {size}  \u{00b7}  {strength:.2}"),
            (None, _) => size,
        };
        super::label_chip(painter, c + Vec2::new(r + 10.0, 12.0), &chip);
    }

    /// Top of the viewport: the mesh tool that is on and how to use it, so a
    /// drag never surprises.
    pub(super) fn paint_mode_hint(&self, painter: &egui::Painter, rect: Rect) {
        if !self.skin_view() {
            return;
        }
        let text = match self.mesh_tool() {
            MeshTool::View => return,
            MeshTool::Paint => match &self.weights_bone {
                Some(b) => format!(
                    "Paint {b} \u{00b7} {}: drag on the mesh, [ ] resize, Alt-drag orbits",
                    mode_label(self.brush.mode)
                ),
                None => "Paint: pick a bone in the list first".to_string(),
            },
            MeshTool::Select => {
                "Select: click or drag a box, Shift adds, Ctrl removes, Alt-drag orbits".to_string()
            }
            MeshTool::Sculpt => {
                let shape = self.shape_pick.as_deref().unwrap_or("?");
                let target = self
                    .shape_tools
                    .target
                    .map_or_else(|| "full".to_string(), |w| format!("in-between {w:.2}"));
                format!(
                    "Sculpt {shape} ({target}) \u{00b7} {}: drag on the mesh, [ ] resize{}",
                    self.shape_tools.brush.label(),
                    if self.shape_tools.brush == SculptBrush::Inflate {
                        ", Ctrl deflates"
                    } else {
                        ""
                    }
                )
            }
        };
        let font = FontId::proportional(12.5);
        let galley = painter.layout_no_wrap(text, font, TEXT);
        let size = galley.size() + Vec2::new(20.0, 10.0);
        let at = Pos2::new(rect.center().x - size.x / 2.0, rect.top() + 60.0);
        let r = Rect::from_min_size(at, size);
        painter.rect_filled(r, 8, Color32::from_rgba_unmultiplied(14, 16, 20, 225));
        painter.rect_stroke(
            r,
            8,
            Stroke::new(1.0, ACCENT.gamma_multiply(0.6)),
            egui::StrokeKind::Inside,
        );
        painter.galley(at + Vec2::new(10.0, 5.0), galley, TEXT);
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
