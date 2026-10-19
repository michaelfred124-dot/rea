//! Blendshapes in the app: the shape list with a weight slider per shape,
//! corrective shapes made from the test pose, a sculpt brush that moves
//! vertices in that pose, and mirror, split, combine and remove. The shape
//! operations themselves are `hive_rig::shapes`; these keep the document's
//! mesh and its built rig in step and make each change undoable.

use eframe::egui::{self, Align, Layout, RichText, Ui};
use glam::{Quat, Vec3};

use hive_rig::Rig;
use hive_rig::shapes::{self, PoseReader, Shape};

use super::Studio;
use crate::theme::{self, TEXT, TEXT_DIM, TEXT_FAINT};

const SHAPE_UNDO_LIMIT: usize = 50;

/// What a click in the Blendshapes card asks for, run after the card is drawn.
enum ShapeAction {
    Pick(String),
    Weight(String, f32),
    Mirror(String),
    Split(String),
    Remove(String),
    Corrective,
    New,
    Combine(Vec<String>),
    Undo,
}

impl Studio {
    /// Local rotations of the test pose, one per bone of the built rig (or the
    /// preview skeleton before a build); every bone is at rest without a pose.
    pub fn pose_locals(&self) -> Vec<Quat> {
        let Some(doc) = &self.doc else {
            return Vec::new();
        };
        let sk = match (&doc.rig, &doc.preview) {
            (Some(rig), _) => &rig.skeleton,
            (None, Ok(built)) => &built.skeleton,
            (None, Err(_)) => return Vec::new(),
        };
        let mut locals = vec![Quat::IDENTITY; sk.bones.len()];
        if let Some(pose) = &self.pose
            && let Some(i) = sk.index_of(&pose.bone)
        {
            let axis = [Vec3::X, Vec3::Y, Vec3::Z][pose.axis.min(2)];
            locals[i] = Quat::from_axis_angle(axis, pose.degrees.to_radians());
        }
        locals
    }

    /// The shapes of the character.
    pub fn shapes(&self) -> &[Shape] {
        self.doc.as_ref().map_or(&[][..], |d| &d.mesh.shapes)
    }

    /// Whether any shape moves the mesh at rest (a weight above zero).
    pub fn shapes_show(&self) -> bool {
        self.shapes()
            .iter()
            .any(|s| s.driver.is_none() && s.weight != 0.0)
    }

    /// The mesh as shown without a test pose: the shapes at their weights.
    pub fn shaped_rest(&self) -> Option<Vec<Vec3>> {
        let doc = self.doc.as_ref()?;
        let weights: Vec<f32> = doc
            .mesh
            .shapes
            .iter()
            .map(|s| if s.driver.is_some() { 0.0 } else { s.weight })
            .collect();
        Some(shapes::apply(
            &doc.mesh.positions,
            &doc.mesh.shapes,
            &weights,
        ))
    }

    fn push_shape_undo(&mut self) {
        let Some(doc) = &self.doc else { return };
        self.shape_undo.push(doc.mesh.shapes.clone());
        if self.shape_undo.len() > SHAPE_UNDO_LIMIT {
            self.shape_undo.remove(0);
        }
    }

    /// Put `shapes` on the document's mesh and on its built rig.
    fn set_shapes(&mut self, list: Vec<Shape>) {
        let Some(doc) = self.doc.as_mut() else { return };
        if let Some(rig) = doc.rig.as_mut() {
            rig.mesh.shapes = list.clone();
        }
        if let Some((rig, _)) = doc.kept.as_mut() {
            rig.mesh.shapes = list.clone();
        }
        doc.mesh.shapes = list;
        doc.exported = false;
    }

    /// Run a shape operation on the built rig (or, for operations that don't
    /// need the skin, a stand-in), undoably.
    fn shape_edit(&mut self, f: impl FnOnce(&mut Rig) -> anyhow::Result<String>) -> bool {
        let Some(doc) = self.doc.as_ref() else {
            return false;
        };
        let Some(rig) = doc.rig.as_ref() else {
            self.warning("Build the rig first: shapes are made on the built rig.");
            return false;
        };
        let mut rig = rig.clone();
        rig.mesh.shapes = doc.mesh.shapes.clone();
        match f(&mut rig).and_then(|msg| rig.mesh.validate().map(|_| msg)) {
            Ok(msg) => {
                self.push_shape_undo();
                self.set_shapes(rig.mesh.shapes);
                self.info(msg);
                true
            }
            Err(e) => {
                self.fail(format!("{e:#}"));
                false
            }
        }
    }

    pub fn undo_shapes(&mut self) {
        if let Some(list) = self.shape_undo.pop() {
            self.set_shapes(list);
            self.info("Undid the last shape change.");
        }
    }

    /// Set a shape's weight (the slider). Not an undo step of its own: a drag
    /// makes many.
    pub fn set_shape_weight(&mut self, name: &str, weight: f32) {
        let mut list = self.shapes().to_vec();
        if let Some(s) = list.iter_mut().find(|s| s.name == name) {
            s.weight = weight;
            self.set_shapes(list);
        }
    }

    /// A corrective shape for the test pose: named after the bone, driven by
    /// it (off at rest, on at the pose's angle), picked for sculpting.
    pub fn new_corrective(&mut self) -> bool {
        let Some(pose) = self.pose.clone() else {
            self.warning("Pose a bone first (pick a bone, then Test this bone).");
            return false;
        };
        if pose.degrees.abs() < 1.0 {
            self.warning("Turn the test pose first: a corrective is made for a bent joint.");
            return false;
        }
        let axis = [Vec3::X, Vec3::Y, Vec3::Z][pose.axis.min(2)];
        let (axis, end) = if pose.degrees < 0.0 {
            (-axis, -pose.degrees)
        } else {
            (axis, pose.degrees)
        };
        let mut name = format!("{}_fix", pose.bone);
        let mut k = 2;
        while self.shapes().iter().any(|s| s.name == name) {
            name = format!("{}_fix{k}", pose.bone);
            k += 1;
        }
        let made = name.clone();
        let ok = self.shape_edit(|rig| {
            rig.mesh.shapes.push(Shape::from_dense(&made, &[]));
            shapes::set_driver(
                rig,
                &made,
                Some(PoseReader {
                    bone: pose.bone.clone(),
                    axis,
                    start: 0.0,
                    end,
                }),
            )?;
            Ok(format!(
                "Made '{made}', driven by {}: sculpt it in this pose (Sculpt, then drag on the mesh).",
                pose.bone
            ))
        });
        if ok {
            self.shape_pick = Some(name);
            self.sculpting = true;
            self.painting = false;
        }
        ok
    }

    /// A new empty shape for sculpting at rest.
    pub fn new_shape(&mut self) -> bool {
        let mut name = "shape".to_string();
        let mut k = 2;
        while self.shapes().iter().any(|s| s.name == name) {
            name = format!("shape{k}");
            k += 1;
        }
        let made = name.clone();
        let ok = self.shape_edit(|rig| {
            let mut s = Shape::from_dense(&made, &[]);
            s.weight = 1.0;
            rig.mesh.shapes.push(s);
            Ok(format!("Made '{made}'. Sculpt it by dragging on the mesh."))
        });
        if ok {
            self.shape_pick = Some(name);
            self.sculpting = true;
            self.painting = false;
        }
        ok
    }

    /// Start a sculpt stroke: one undo step for the whole drag.
    pub fn begin_sculpt(&mut self) -> bool {
        let ready = self.sculpting
            && self.shape_pick.is_some()
            && self.doc.as_ref().is_some_and(|d| d.rig.is_some());
        if ready {
            self.push_shape_undo();
        }
        ready
    }

    /// Move the vertices near `at` (on the mesh as shown) by `offset`, in the
    /// test pose, into the picked shape. Returns how many moved.
    pub fn sculpt_at(&mut self, at: Vec3, offset: Vec3) -> usize {
        let (Some(name), Some(doc)) = (self.shape_pick.clone(), self.doc.as_ref()) else {
            return 0;
        };
        let Some(rig) = doc.rig.as_ref() else {
            return 0;
        };
        let locals = self.pose_locals();
        let mut rig = rig.clone();
        rig.mesh.shapes = doc.mesh.shapes.clone();
        let posed = shapes::posed_mesh(&rig, &locals);
        let radius = self.sculpt_radius * doc.guides.body.height;
        let moves = shapes::brush_moves(&posed, at, radius, offset);
        match shapes::sculpt(&mut rig, &name, &locals, &moves) {
            Ok(n) => {
                self.set_shapes(rig.mesh.shapes);
                n
            }
            Err(e) => {
                self.fail(format!("{e:#}"));
                0
            }
        }
    }

    pub fn mirror_shape(&mut self, name: &str) -> bool {
        let name = name.to_string();
        self.shape_edit(|rig| {
            let made = shapes::mirror_on_rig(rig, &name, None)?;
            Ok(format!("Mirrored '{name}' as '{made}'."))
        })
    }

    pub fn split_shape(&mut self, name: &str) -> bool {
        let name = name.to_string();
        self.shape_edit(|rig| {
            let [l, r] = shapes::split_on_rig(rig, &name, None, None)?;
            Ok(format!("Split '{name}' into '{l}' and '{r}'."))
        })
    }

    pub fn combine_shapes(&mut self, names: &[String]) -> bool {
        let names = names.to_vec();
        let mut new = names.join("_");
        let mut k = 2;
        while self.shapes().iter().any(|s| s.name == new) {
            new = format!("{}_{k}", names.join("_"));
            k += 1;
        }
        self.shape_edit(|rig| {
            shapes::combine_on_rig(rig, &names, &new)?;
            Ok(format!("Combined {} into '{new}'.", names.join(" and ")))
        })
    }

    pub fn remove_shape(&mut self, name: &str) -> bool {
        let name = name.to_string();
        let ok = self.shape_edit(|rig| {
            shapes::remove(rig, &name)?;
            Ok(format!("Removed '{name}'."))
        });
        if ok && self.shape_pick.as_deref() == Some(name.as_str()) {
            self.shape_pick = None;
            self.sculpting = false;
        }
        ok
    }

    /// The Blendshapes card in the rig's Properties.
    pub(super) fn shapes_card(&mut self, ui: &mut Ui) {
        let Some(doc) = &self.doc else { return };
        let built = doc.rig.is_some();
        let list: Vec<Shape> = doc.mesh.shapes.clone();
        let locals = self.pose_locals();
        let sk = doc.rig.as_ref().map(|r| r.skeleton.clone());
        theme::label_caps(ui, "Blendshapes");
        if list.is_empty() {
            ui.label(
                RichText::new(if built {
                    "No shapes yet. Pose a bone and make a corrective, or add a shape to sculpt."
                } else {
                    "Build the rig to make shapes; shapes in the file show here."
                })
                .size(12.5)
                .color(TEXT_FAINT),
            );
        }
        let mut action: Option<ShapeAction> = None;
        for s in &list {
            ui.horizontal(|ui| {
                let picked = self.shape_pick.as_deref() == Some(s.name.as_str());
                if ui
                    .selectable_label(picked, RichText::new(&s.name).size(12.5).color(TEXT))
                    .on_hover_text(format!(
                        "{} vertices. Click to sculpt this shape.",
                        s.vertices.len()
                    ))
                    .clicked()
                {
                    action = Some(ShapeAction::Pick(s.name.clone()));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.menu_button(RichText::new("...").size(12.5), |ui| {
                        let n = s.name.clone();
                        let items = [
                            ("Mirror to the other side", ShapeAction::Mirror(n.clone())),
                            ("Split into left and right", ShapeAction::Split(n.clone())),
                            ("Remove", ShapeAction::Remove(n)),
                        ];
                        for (label, chosen) in items {
                            if ui.button(label).clicked() {
                                action = Some(chosen);
                                ui.close();
                            }
                        }
                    });
                });
            });
            match &s.driver {
                Some(d) => {
                    let value = sk
                        .as_ref()
                        .and_then(|sk| sk.index_of(&d.bone))
                        .map_or(0.0, |i| {
                            d.weight(locals.get(i).copied().unwrap_or(Quat::IDENTITY))
                        });
                    let mut shown = value;
                    ui.add_enabled(
                        false,
                        egui::Slider::new(&mut shown, 0.0..=1.0).show_value(true),
                    );
                    ui.label(
                        RichText::new(format!(
                            "Driven by {}: off at {:.0}°, on at {:.0}°",
                            d.bone, d.start, d.end
                        ))
                        .size(11.5)
                        .color(TEXT_DIM),
                    );
                }
                None => {
                    let mut w = s.weight;
                    if ui
                        .add(egui::Slider::new(&mut w, 0.0..=1.0).show_value(true))
                        .changed()
                    {
                        action = Some(ShapeAction::Weight(s.name.clone(), w));
                    }
                }
            }
        }
        if built {
            ui.horizontal_wrapped(|ui| {
                if ui
                    .add(
                        egui::Button::new(RichText::new("Corrective from pose").size(12.5))
                            .corner_radius(8),
                    )
                    .on_hover_text(
                        "A shape the test-posed bone turns on as it bends, to sculpt in this pose.",
                    )
                    .clicked()
                {
                    action = Some(ShapeAction::Corrective);
                }
                if ui
                    .add(egui::Button::new(RichText::new("New shape").size(12.5)).corner_radius(8))
                    .clicked()
                {
                    action = Some(ShapeAction::New);
                }
                let free: Vec<String> = list
                    .iter()
                    .filter(|s| s.driver.is_none() && s.weight > 0.0)
                    .map(|s| s.name.clone())
                    .collect();
                if free.len() >= 2
                    && ui
                        .add(
                            egui::Button::new(RichText::new("Combine shown").size(12.5))
                                .corner_radius(8),
                        )
                        .on_hover_text(
                            "Add the shapes with a weight above zero together as a new shape.",
                        )
                        .clicked()
                {
                    action = Some(ShapeAction::Combine(free));
                }
                if !self.shape_undo.is_empty()
                    && ui
                        .add(
                            egui::Button::new(RichText::new("Undo shape").size(12.5))
                                .corner_radius(8),
                        )
                        .clicked()
                {
                    action = Some(ShapeAction::Undo);
                }
            });
            if self.shape_pick.is_some() {
                let mut on = self.sculpting;
                ui.checkbox(
                    &mut on,
                    RichText::new("Sculpt: drag on the mesh").size(12.5),
                )
                .on_hover_text(
                    "Moves the vertices under the brush in the current pose into the picked shape.",
                );
                if on != self.sculpting {
                    self.sculpting = on;
                    if on {
                        self.painting = false;
                    }
                }
                ui.add(egui::Slider::new(&mut self.sculpt_radius, 0.005..=0.2).text("radius"));
            }
        }
        match action {
            Some(ShapeAction::Pick(n)) => self.shape_pick = Some(n),
            Some(ShapeAction::Weight(n, w)) => self.set_shape_weight(&n, w),
            Some(ShapeAction::Mirror(n)) => {
                self.mirror_shape(&n);
            }
            Some(ShapeAction::Split(n)) => {
                self.split_shape(&n);
            }
            Some(ShapeAction::Remove(n)) => {
                self.remove_shape(&n);
            }
            Some(ShapeAction::Corrective) => {
                self.new_corrective();
            }
            Some(ShapeAction::New) => {
                self.new_shape();
            }
            Some(ShapeAction::Combine(names)) => {
                self.combine_shapes(&names);
            }
            Some(ShapeAction::Undo) => self.undo_shapes(),
            None => {}
        }
    }
}
