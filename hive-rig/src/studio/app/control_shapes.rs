//! Custom control shapes in the app: draw one on a grid (click points, open or
//! closed lines, mirrored across the up axis), import one (.svg or .json),
//! keep it in the user's shape library and the guides, and put it on any
//! control. Also each control's line width. The shapes themselves are
//! `hive_rig::custom_shapes`; these keep the guides and the library in step
//! and make each change undoable.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2};

use hive_rig::components::{self, StyleChange, StyleScope};
use hive_rig::controls::Control;
use hive_rig::custom_shapes::{self, CustomShape, ShapeLibrary, ShapeLine};

use super::Studio;
use crate::theme::{self, ACCENT, BORDER, TEXT, TEXT_DIM, TEXT_FAINT};

/// Grid steps across half the drawing area (the grid runs -1 to 1).
const GRID_STEPS: f32 = 8.0;

/// A shape being drawn in the shape editor.
#[derive(Debug, Clone)]
pub struct ShapeDraft {
    pub name: String,
    /// The lines so far; the last one is the one points are added to.
    pub lines: Vec<ShapeLine>,
    pub mirror_x: bool,
    /// Clicked points land on the grid.
    pub snap: bool,
    /// The control the shape goes on when it's saved.
    pub control: Option<String>,
}

impl ShapeDraft {
    pub fn new(control: Option<String>) -> ShapeDraft {
        ShapeDraft {
            name: String::new(),
            lines: vec![ShapeLine {
                points: Vec::new(),
                closed: false,
            }],
            mirror_x: true,
            snap: true,
            control,
        }
    }

    /// Add a point to the current line, on the grid when snapping.
    pub fn add_point(&mut self, p: [f32; 2]) {
        let p = if self.snap {
            p.map(|v| (v * GRID_STEPS).round() / GRID_STEPS)
        } else {
            p
        };
        let p = p.map(|v| v.clamp(-1.0, 1.0));
        let line = self.lines.last_mut().unwrap();
        if line.points.last() != Some(&p) {
            line.points.push(p);
        }
    }

    pub fn undo_point(&mut self) {
        if let Some(line) = self.lines.last_mut()
            && line.points.pop().is_none()
            && self.lines.len() > 1
        {
            self.lines.pop();
            self.lines.last_mut().unwrap().points.pop();
        }
    }

    /// Start another line (the current one needs two points first).
    pub fn new_line(&mut self) {
        if self.lines.last().is_some_and(|l| l.points.len() >= 2) {
            self.lines.push(ShapeLine {
                points: Vec::new(),
                closed: false,
            });
        }
    }

    /// The shape drawn so far: its lines with two points or more.
    pub fn shape(&self) -> CustomShape {
        CustomShape {
            lines: self
                .lines
                .iter()
                .filter(|l| l.points.len() >= 2)
                .cloned()
                .collect(),
            mirror_x: self.mirror_x,
        }
    }
}

impl Studio {
    /// The user's shape library, read from its file the first time.
    pub fn shape_library(&mut self) -> &ShapeLibrary {
        if self.shape_lib.is_none() {
            let lib = match &self.shape_lib_path {
                Some(p) => ShapeLibrary::load(p).unwrap_or_else(|e| {
                    eprintln!("hive-rig: {e:#}");
                    ShapeLibrary::default()
                }),
                None => ShapeLibrary::default(),
            };
            self.shape_lib = Some(lib);
        }
        self.shape_lib.as_ref().unwrap()
    }

    /// Custom shapes to offer: the guides' first, then the library's others.
    /// Each comes with whether the guides already have it.
    pub fn custom_shape_choices(&mut self) -> Vec<(String, CustomShape, bool)> {
        let mut out: Vec<(String, CustomShape, bool)> = self
            .doc
            .as_ref()
            .map(|d| {
                d.guides
                    .custom_shapes
                    .iter()
                    .map(|(n, s)| (n.clone(), s.clone(), true))
                    .collect()
            })
            .unwrap_or_default();
        let lib: Vec<(String, CustomShape)> = self
            .shape_library()
            .shapes
            .iter()
            .map(|(n, s)| (n.clone(), s.clone()))
            .collect();
        for (n, s) in lib {
            if !out.iter().any(|(o, _, _)| *o == n) {
                out.push((n, s, false));
            }
        }
        out
    }

    fn style_scope(&self) -> StyleScope {
        if self.symmetry {
            StyleScope::WithTwin
        } else {
            StyleScope::One
        }
    }

    /// Put the custom shape `name` on `control` (and its twin with symmetry),
    /// bringing it into the guides from the library if needed: one undo step.
    pub fn use_custom_shape(&mut self, control: &str, name: &str) -> bool {
        let from_library = self.shape_library().shapes.get(name).cloned();
        let scope = self.style_scope();
        let (control, name) = (control.to_string(), name.to_string());
        self.edit("use the shape", |g| {
            if !g.custom_shapes.contains_key(&name) {
                let shape = from_library
                    .ok_or_else(|| anyhow::anyhow!("no shape named '{name}' in the library"))?;
                components::add_custom_shape(g, &name, shape)?;
            }
            let change = StyleChange {
                custom: Some(name.clone()),
                ..Default::default()
            };
            let n = components::restyle(g, &control, change, scope)?.len();
            Ok(format!(
                "{control}{} now drawn as {name}.",
                if n > 1 { " and its twin" } else { "" }
            ))
        })
    }

    /// Keep a shape in the user's library (saved at once) and in the guides,
    /// and put it on `control` if given: one undo step.
    pub fn keep_custom_shape(
        &mut self,
        name: &str,
        shape: CustomShape,
        control: Option<&str>,
    ) -> bool {
        let name = name.trim().to_string();
        if let Err(e) = custom_shapes::check_name(&name).and_then(|_| shape.validate()) {
            self.fail(format!("Can't keep the shape: {e:#}"));
            return false;
        }
        self.shape_library();
        let lib = self.shape_lib.as_mut().unwrap();
        lib.shapes.insert(name.clone(), shape.clone());
        let saved = match &self.shape_lib_path {
            Some(p) => lib.save(p).map(|_| Some(p.clone())),
            None => Ok(None),
        };
        let scope = self.style_scope();
        let target = control.map(str::to_string);
        let ok = self.edit("keep the shape", |g| {
            components::add_custom_shape(g, &name, shape)?;
            if let Some(c) = &target {
                let change = StyleChange {
                    custom: Some(name.clone()),
                    ..Default::default()
                };
                components::restyle(g, c, change, scope)?;
            }
            Ok(match &target {
                Some(c) => format!("Saved the shape {name} and put it on {c}."),
                None => format!("Saved the shape {name}."),
            })
        });
        match saved {
            Err(e) => self.warning(format!(
                "Kept {name} in the guides, but couldn't save the shape library: {e:#}"
            )),
            Ok(Some(p)) if ok => {
                let msg = format!(
                    "{} It's in your shape library ({}).",
                    self.status,
                    p.display()
                );
                self.info(msg);
            }
            _ => {}
        }
        ok
    }

    /// Import a shape from an .svg or .json file, keep it and put it on `control`.
    pub fn import_custom_shape(&mut self, path: &Path, control: Option<&str>) -> bool {
        match CustomShape::import(path) {
            Ok((name, shape)) => self.keep_custom_shape(&name, shape, control),
            Err(e) => {
                self.fail(format!("Couldn't import the shape: {e:#}"));
                false
            }
        }
    }

    /// Set a control's line width (and its twin's with symmetry).
    pub fn set_line_width(&mut self, control: &str, width: f32) {
        let change = StyleChange {
            line_width: Some(width),
            ..Default::default()
        };
        self.restyle_controls(control, change, self.style_scope());
    }

    /// The Properties section for a picked control's custom shape and line width.
    pub(super) fn custom_shape_section(&mut self, ui: &mut Ui, c: &Control) {
        theme::label_caps(ui, "Custom shapes");
        let choices = self.custom_shape_choices();
        let current = c.custom.as_ref().map(|s| s.name.clone());
        let mut chosen = None;
        if choices.is_empty() {
            ui.label(
                RichText::new("Draw your own, or import an .svg or .json of points.")
                    .size(11.5)
                    .color(TEXT_FAINT),
            );
        } else {
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing = Vec2::splat(4.0);
                for (name, shape, in_guides) in &choices {
                    let on = current.as_deref() == Some(name.as_str());
                    let tip = if *in_guides {
                        name.clone()
                    } else {
                        format!("{name} (from your shape library)")
                    };
                    if custom_tile(ui, shape, on).on_hover_text(tip).clicked() && !on {
                        chosen = Some(name.clone());
                    }
                }
            });
        }
        if let Some(name) = &current {
            ui.label(
                RichText::new(format!("{name}; Unreal draws it as {}", c.shape.label()))
                    .size(12.0)
                    .color(TEXT_FAINT),
            );
        }
        ui.horizontal(|ui| {
            if ui
                .button("Draw…")
                .on_hover_text("Click points on a grid to draw a new control shape")
                .clicked()
            {
                self.shape_draft = Some(ShapeDraft::new(Some(c.name.clone())));
            }
            if ui
                .button("Import…")
                .on_hover_text("An .svg (polylines, polygons, straight paths) or a .json of points")
                .clicked()
                && let Some(p) = rfd::FileDialog::new()
                    .set_title("Import a control shape")
                    .add_filter("Shapes", &["svg", "json"])
                    .pick_file()
            {
                self.import_custom_shape(&p, Some(&c.name));
            }
        });
        if let Some(name) = chosen {
            self.use_custom_shape(&c.name, &name);
        }
        ui.add_space(6.0);
        theme::label_caps(ui, "Line width");
        let mut width = c.width();
        let (lo, hi) = custom_shapes::LINE_WIDTH_RANGE;
        if ui
            .add(
                egui::Slider::new(&mut width, lo..=hi)
                    .step_by(0.25)
                    .fixed_decimals(2),
            )
            .on_hover_text(
                "1 is normal. Blender (4.2+) and Maya draw it this thick; \
                 Unreal uses its thin shape below 1",
            )
            .changed()
        {
            self.set_line_width(&c.name, width);
        }
    }

    /// The shape editor window, while a shape is being drawn.
    pub(super) fn shape_editor(&mut self, ctx: &egui::Context) {
        let Some(mut draft) = self.shape_draft.take() else {
            return;
        };
        let mut open = true;
        let mut save = false;
        let mut cancel = false;
        egui::Window::new("Draw a control shape")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .default_pos(Pos2::new(700.0, 120.0))
            .show(ctx, |ui| {
                ui.label(
                    RichText::new(
                        "Click to add points, right-click to take the last one back. \
                         The centre is the control; up is up the control's plane.",
                    )
                    .size(12.0)
                    .color(TEXT_DIM),
                );
                ui.add_space(4.0);
                draw_canvas(ui, &mut draft, 280.0);
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    if let Some(line) = draft.lines.last_mut() {
                        ui.checkbox(&mut line.closed, "Closed");
                    }
                    ui.checkbox(&mut draft.mirror_x, "Mirror X");
                    ui.checkbox(&mut draft.snap, "Snap");
                });
                ui.horizontal(|ui| {
                    if ui.button("New line").clicked() {
                        draft.new_line();
                    }
                    if ui.button("Undo point").clicked() {
                        draft.undo_point();
                    }
                    if ui.button("Clear").clicked() {
                        draft.lines = ShapeDraft::new(None).lines;
                    }
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Name").color(TEXT_DIM));
                    ui.add(egui::TextEdit::singleline(&mut draft.name).desired_width(170.0));
                });
                let shape = draft.shape();
                let ready = !draft.name.trim().is_empty() && shape.validate().is_ok();
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let label = if draft.control.is_some() {
                        "Save and use"
                    } else {
                        "Save"
                    };
                    if theme::primary(ui, label, 120.0, ready).clicked() {
                        save = true;
                    }
                    if theme::secondary(ui, "Cancel", 80.0).clicked() {
                        cancel = true;
                    }
                });
            });
        if save {
            let shape = draft.shape();
            if self.keep_custom_shape(&draft.name.clone(), shape, draft.control.as_deref()) {
                return;
            }
        }
        if open && !cancel {
            self.shape_draft = Some(draft);
        }
    }
}

/// The drawing grid: the shape so far (and its mirror image), clickable.
fn draw_canvas(ui: &mut Ui, draft: &mut ShapeDraft, size: f32) {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 8.0, Color32::from_rgb(16, 18, 24));
    let half = size * 0.5 - 12.0;
    let c = rect.center();
    let to_screen = |p: [f32; 2]| c + Vec2::new(p[0], -p[1]) * half;
    let from_screen = |s: Pos2| [(s.x - c.x) / half, (c.y - s.y) / half];
    let steps = GRID_STEPS as i32;
    for i in -steps..=steps {
        let t = i as f32 / GRID_STEPS;
        let color = if i == 0 {
            Color32::from_rgb(70, 78, 94)
        } else if i % 4 == 0 {
            Color32::from_rgb(40, 45, 56)
        } else {
            Color32::from_rgb(28, 32, 40)
        };
        let s = Stroke::new(1.0, color);
        painter.line_segment([to_screen([t, -1.0]), to_screen([t, 1.0])], s);
        painter.line_segment([to_screen([-1.0, t]), to_screen([1.0, t])], s);
    }
    painter.rect_stroke(
        rect,
        8.0,
        Stroke::new(1.0, BORDER),
        egui::StrokeKind::Inside,
    );
    // The mirror image first, faint, then what's drawn.
    let shape = draft.shape();
    if draft.mirror_x && !shape.lines.is_empty() {
        for (pts, closed) in shape.drawn_lines() {
            let mut line: Vec<Pos2> = pts.iter().map(|p| to_screen(p.to_array())).collect();
            if closed {
                line.push(line[0]);
            }
            painter.add(egui::Shape::line(
                line,
                Stroke::new(1.5, ACCENT.gamma_multiply(0.45)),
            ));
        }
    }
    for (k, l) in draft.lines.iter().enumerate() {
        let mut line: Vec<Pos2> = l.points.iter().map(|p| to_screen(*p)).collect();
        if l.closed && line.len() > 2 {
            line.push(line[0]);
        }
        let current = k + 1 == draft.lines.len();
        painter.add(egui::Shape::line(
            line,
            Stroke::new(2.2, if current { ACCENT } else { TEXT }),
        ));
        for p in &l.points {
            painter.circle_filled(to_screen(*p), 3.5, if current { ACCENT } else { TEXT });
        }
    }
    if let Some(pos) = response.hover_pos() {
        let mut p = from_screen(pos);
        if draft.snap {
            p = p.map(|v| (v * GRID_STEPS).round() / GRID_STEPS);
        }
        painter.circle_stroke(to_screen(p), 5.0, Stroke::new(1.0, TEXT_DIM));
    }
    if response.clicked()
        && let Some(pos) = response.interact_pointer_pos()
    {
        draft.add_point(from_screen(pos));
    }
    if response.secondary_clicked() {
        draft.undo_point();
    }
}

/// A tile showing a custom shape face on, ringed when it's the control's.
fn custom_tile(ui: &mut Ui, shape: &CustomShape, on: bool) -> egui::Response {
    const TILE: f32 = 40.0;
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(TILE), Sense::click());
    let hot = response.hovered();
    let painter = ui.painter_at(rect);
    painter.rect(
        rect,
        8.0,
        if on {
            ACCENT.gamma_multiply(0.25)
        } else if hot {
            Color32::from_rgb(34, 38, 48)
        } else {
            Color32::from_rgb(20, 23, 30)
        },
        Stroke::new(1.0, if on { ACCENT } else { BORDER }),
        egui::StrokeKind::Inside,
    );
    let color = if on || hot { TEXT } else { TEXT_DIM };
    custom_preview(&painter, rect.shrink(8.0), shape, Stroke::new(1.4, color));
    response
}

/// A custom shape's lines face on, fitted to `rect`.
pub fn custom_preview(painter: &egui::Painter, rect: Rect, shape: &CustomShape, stroke: Stroke) {
    let half = rect.width().min(rect.height()) * 0.5;
    let at = rect.center();
    for line in shape.strokes() {
        let pts: Vec<Pos2> = line
            .iter()
            .map(|p| at + Vec2::new(p.x, -p.z) * half)
            .collect();
        painter.add(egui::Shape::line(pts, stroke));
    }
}

/// The user's shape library file, unless tests or `HIVE_RIG_SHAPES` say otherwise.
pub fn default_library_path() -> Option<PathBuf> {
    custom_shapes::library_path()
}
