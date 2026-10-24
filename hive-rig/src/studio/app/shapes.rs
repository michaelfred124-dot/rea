//! Blendshapes in the app: the shape list with a weight slider per shape (and
//! a search box once there are many), corrective shapes made from the test
//! pose, in-between targets, a sculpt brush set (grab, smooth, relax, inflate,
//! erase) that works in that pose, and mirror, split, combine and remove. The
//! shape operations themselves are `hive_rig::shapes`; these keep the
//! document's mesh and its built rig in step and make each change undoable.

use eframe::egui::{self, Align, Layout, PointerButton, Pos2, RichText, Ui};
use glam::{Mat4, Quat, Vec3};

use hive_rig::Rig;
use hive_rig::shapes::{self, PoseReader, SculptBrush, Shape};

use super::Studio;
use crate::theme::{self, ACCENT, TEXT, TEXT_DIM, TEXT_FAINT, WARN};
use crate::view::Projector;

const SHAPE_UNDO_LIMIT: usize = 50;
/// Past this many shapes the card shows a search box.
const SEARCH_FROM: usize = 6;

/// The sculpt brush's settings and what it works on.
pub struct ShapeTools {
    pub brush: SculptBrush,
    /// How much one dab does at the brush's centre (0 to 1).
    pub strength: f32,
    /// The in-between being sculpted (by its weight), or the full shape.
    pub target: Option<f32>,
    /// Shapes whose name contains this are listed.
    pub filter: String,
    /// Frames left in which Properties scrolls to the Blendshapes card (for
    /// screenshots of the sculpt tools).
    pub scroll_to: u8,
}

impl Default for ShapeTools {
    fn default() -> Self {
        ShapeTools {
            brush: SculptBrush::Grab,
            strength: 0.5,
            target: None,
            filter: String::new(),
            scroll_to: 0,
        }
    }
}

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
    Target(Option<f32>),
    AddInbetween,
    RemoveInbetween(f32),
    PoseToTarget,
}

impl Studio {
    /// Local rotations of the test pose, one per bone of the built rig (or the
    /// preview skeleton before a build); every bone is at rest without a pose.
    pub fn pose_locals(&self) -> Vec<Quat> {
        let Some(doc) = &self.doc else {
            return Vec::new();
        };
        // The control-rig pose, when it is what shows.
        if self.pose_shows()
            && let Some(posed) = self.posed()
        {
            return posed.locals();
        }
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

    /// A shape's weight as it shows now: its slider, or what its reader gives
    /// in the test pose.
    pub fn shape_weight_now(&self, s: &Shape) -> f32 {
        let Some(d) = &s.driver else { return s.weight };
        let Some(rig) = self.doc.as_ref().and_then(|d| d.rig.as_ref()) else {
            return 0.0;
        };
        let locals = self.pose_locals();
        rig.skeleton.index_of(&d.bone).map_or(0.0, |i| {
            d.weight(locals.get(i).copied().unwrap_or(Quat::IDENTITY))
        })
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

    /// The built rig with the document's shapes, for a shape operation.
    fn shape_rig(&self) -> Option<Rig> {
        let doc = self.doc.as_ref()?;
        let mut rig = doc.rig.as_ref()?.clone();
        rig.mesh.shapes = doc.mesh.shapes.clone();
        Some(rig)
    }

    /// Run a shape operation on the built rig (or, for operations that don't
    /// need the skin, a stand-in), undoably.
    fn shape_edit(&mut self, f: impl FnOnce(&mut Rig) -> anyhow::Result<String>) -> bool {
        let Some(mut rig) = self.shape_rig() else {
            self.warning("Build the rig first: shapes are made on the built rig.");
            return false;
        };
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
            self.check_target();
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

    /// Pick the shape the sculpt brush edits (its full target).
    pub fn pick_shape(&mut self, name: &str) {
        if self.shape_pick.as_deref() != Some(name) {
            self.shape_tools.target = None;
        }
        self.shape_pick = Some(name.to_string());
    }

    /// The picked shape.
    fn picked_shape(&self) -> Option<&Shape> {
        let name = self.shape_pick.as_deref()?;
        self.shapes().iter().find(|s| s.name == name)
    }

    /// Forget a target in-between the picked shape no longer has.
    fn check_target(&mut self) {
        let keep = match (self.shape_tools.target, self.picked_shape()) {
            (Some(w), Some(s)) => s.inbetween_at(w).is_some(),
            (Some(_), None) => false,
            (None, _) => true,
        };
        if !keep {
            self.shape_tools.target = None;
        }
    }

    /// Sculpt one of the picked shape's targets: an in-between (by its weight)
    /// or the full shape. An undriven shape's slider moves to that target so
    /// it shows as it is sculpted.
    pub fn pick_target(&mut self, target: Option<f32>) {
        let Some(s) = self.picked_shape() else { return };
        let (name, driven) = (s.name.clone(), s.driver.is_some());
        self.shape_tools.target = target;
        if !driven {
            self.set_shape_weight(&name, target.unwrap_or(1.0));
        }
    }

    /// Give the picked shape an in-between where it is now (its slider, or
    /// what the test pose gives a corrective), or half way when that is at an
    /// end, and sculpt it next.
    pub fn add_inbetween_here(&mut self) -> bool {
        let Some(s) = self.picked_shape() else {
            self.warning("Pick a shape first.");
            return false;
        };
        let now = self.shape_weight_now(s);
        let weight = if (0.05..=0.95).contains(&now) {
            now
        } else {
            0.5
        };
        self.add_inbetween_here_at(weight)
    }

    /// Give the picked shape an in-between at `weight` and sculpt it next.
    pub fn add_inbetween_here_at(&mut self, weight: f32) -> bool {
        let Some(s) = self.picked_shape() else {
            self.warning("Pick a shape first.");
            return false;
        };
        let name = s.name.clone();
        let weight = (weight * 100.0).round() / 100.0;
        let ok = self.shape_edit(|rig| {
            shapes::add_inbetween(rig, &name, weight)?;
            Ok(format!(
                "'{name}' has an in-between at {weight:.2}: sculpt it now, it blends in on the way to the full shape."
            ))
        });
        if ok {
            self.pick_target(Some(weight));
            self.set_mesh_tool(super::MeshTool::Sculpt);
        }
        ok
    }

    pub fn remove_inbetween(&mut self, weight: f32) -> bool {
        let Some(name) = self.shape_pick.clone() else {
            return false;
        };
        let ok = self.shape_edit(|rig| {
            shapes::remove_inbetween(rig, &name, weight)?;
            Ok(format!("Took the in-between at {weight:.2} off '{name}'."))
        });
        self.check_target();
        ok
    }

    /// Turn the test pose so the picked corrective's reader reaches the
    /// target being sculpted (when the reader turns about X, Y or Z).
    pub fn pose_to_target(&mut self) -> bool {
        let Some(s) = self.picked_shape() else {
            return false;
        };
        let Some(d) = s.driver.clone() else {
            return false;
        };
        let w = self.shape_tools.target.unwrap_or(1.0);
        let a = d.axis.normalize_or_zero();
        let Some((axis, sign)) = (0..3)
            .map(|i| (i, a[i]))
            .find(|(_, x)| x.abs() > 0.99)
            .map(|(i, x)| (i, x.signum()))
        else {
            self.warning("This reader turns about a tilted axis; pose the bone by hand.");
            return false;
        };
        let degrees = (d.start + w * (d.end - d.start)) * sign;
        self.pose = Some(super::Pose {
            bone: d.bone.clone(),
            axis,
            degrees,
        });
        self.info(format!(
            "Posed {} at {degrees:.0}° about {}: the target shows fully here.",
            d.bone,
            ["X", "Y", "Z"][axis]
        ));
        true
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
                "Made '{made}', driven by {}: sculpt it in this pose (drag on the mesh).",
                pose.bone
            ))
        });
        if ok {
            self.pick_shape(&name);
            self.set_mesh_tool(super::MeshTool::Sculpt);
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
            self.pick_shape(&name);
            self.set_mesh_tool(super::MeshTool::Sculpt);
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

    /// The sculpt brush's radius in metres.
    fn sculpt_reach(&self) -> f32 {
        self.sculpt_radius * self.doc.as_ref().map_or(1.8, |d| d.guides.body.height)
    }

    /// Grab: move the vertices near `at` (on the mesh as shown) by `offset`,
    /// in the test pose, into the picked target. Returns how many moved.
    pub fn sculpt_at(&mut self, at: Vec3, offset: Vec3) -> usize {
        let (Some(name), Some(mut rig)) = (self.shape_pick.clone(), self.shape_rig()) else {
            return 0;
        };
        let target = self.shape_tools.target;
        let locals = self.pose_locals();
        let mats = self.sculpt_mats(&rig, &locals);
        let posed = shapes::posed_target(&rig, &name, target, &locals, &mats);
        let moves = shapes::brush_moves(&posed, at, self.sculpt_reach(), offset);
        self.apply_sculpt(&mut rig, &name, target, (&locals, &mats), &moves)
    }

    /// The skinning matrices of the pose that shows: the control rig's when
    /// it is posed, else the test pose's.
    fn sculpt_mats(&self, rig: &Rig, locals: &[Quat]) -> Vec<Mat4> {
        self.pose_mats()
            .unwrap_or_else(|| shapes::skin_matrices(&rig.skeleton, locals))
    }

    fn apply_sculpt(
        &mut self,
        rig: &mut Rig,
        name: &str,
        target: Option<f32>,
        (locals, mats): (&[Quat], &[Mat4]),
        moves: &[(usize, Vec3)],
    ) -> usize {
        match shapes::sculpt_target(rig, name, target, locals, mats, moves) {
            Ok(n) => {
                self.set_shapes(std::mem::take(&mut rig.mesh.shapes));
                n
            }
            Err(e) => {
                self.fail(format!("{e:#}"));
                0
            }
        }
    }

    /// One dab of the smooth, relax, inflate or erase brush at `at`, in the
    /// test pose, on the picked target. `invert` deflates with Inflate.
    pub fn sculpt_dab(&mut self, at: Vec3, invert: bool) -> usize {
        let (Some(name), Some(mut rig)) = (self.shape_pick.clone(), self.shape_rig()) else {
            return 0;
        };
        let target = self.shape_tools.target;
        let locals = self.pose_locals();
        let mats = self.sculpt_mats(&rig, &locals);
        let posed = shapes::posed_target(&rig, &name, target, &locals, &mats);
        let (radius, strength) = (self.sculpt_reach(), self.shape_tools.strength);
        let moves = match self.shape_tools.brush {
            SculptBrush::Grab => return 0,
            SculptBrush::Smooth => {
                let adj = self.neighbours();
                shapes::smooth_moves(&posed, &adj, at, radius, strength)
            }
            SculptBrush::Relax | SculptBrush::Inflate => {
                let weld = rig.mesh.welded();
                let normals = shapes::welded_normals(&posed, &rig.mesh.indices, &weld);
                if self.shape_tools.brush == SculptBrush::Relax {
                    let adj = self.neighbours();
                    shapes::relax_moves(&posed, &normals, &adj, at, radius, strength)
                } else {
                    // A full-strength dab pushes a tenth of the radius.
                    let amount = 0.1 * radius * strength * if invert { -1.0 } else { 1.0 };
                    shapes::inflate_moves(&posed, &normals, at, radius, amount)
                }
            }
            SculptBrush::Erase => {
                return match shapes::erase(&mut rig, &name, target, &posed, at, radius, strength) {
                    Ok(n) => {
                        self.set_shapes(rig.mesh.shapes);
                        n
                    }
                    Err(e) => {
                        self.fail(format!("{e:#}"));
                        0
                    }
                };
            }
        };
        self.apply_sculpt(&mut rig, &name, target, (&locals, &mats), &moves)
    }

    /// The sculpt brush in the viewport: Grab drags the vertices under it
    /// across the screen (in the plane facing the camera through the grab
    /// point); the other brushes dab along the stroke, a quarter of the brush
    /// apart. Ctrl inverts Inflate.
    pub(super) fn sculpt_input(
        &mut self,
        ui: &Ui,
        response: &egui::Response,
        pr: &Projector,
        pointer: Option<Pos2>,
        press: Option<Pos2>,
    ) {
        let hit_at = |s: &Studio, p: Pos2| {
            let (o, d) = pr.ray(p);
            s.hit_mesh(o, d)
        };
        let grab = self.shape_tools.brush == SculptBrush::Grab;
        if (self.stroke.is_none() || !grab)
            && response.hovered()
            && let Some(p) = pointer
            && (!grab || self.stroke.is_none())
        {
            if let Some(hit) = hit_at(self, p) {
                self.brush_at = Some(hit);
            } else if self.stroke.is_none() {
                self.brush_at = None;
            }
        }
        if response.drag_started_by(PointerButton::Primary)
            && let Some(p) = press.or(pointer)
            && let Some(hit) = hit_at(self, p)
            && self.begin_sculpt()
        {
            self.stroke = Some(hit);
            if !grab {
                let invert = ui.input(|i| i.modifiers.command);
                self.sculpt_dab(hit, invert);
            }
        }
        let Some(last) = self.stroke else { return };
        if !response.dragged_by(PointerButton::Primary) {
            return;
        }
        if grab {
            let Some(p) = pointer else { return };
            let (o, d) = pr.ray(p);
            let across = d.dot(pr.forward);
            if across.abs() > 1e-6 {
                let to = o + d * ((last - o).dot(pr.forward) / across);
                if to.distance(last) > 1e-5 {
                    self.sculpt_at(last, to - last);
                    self.stroke = Some(to);
                    self.brush_at = Some(to);
                }
            }
        } else if let Some(hit) = self.brush_at
            && hit.distance(last) >= 0.25 * self.sculpt_reach()
        {
            let invert = ui.input(|i| i.modifiers.command);
            self.sculpt_dab(hit, invert);
            self.stroke = Some(hit);
        }
    }

    /// Put the sculpt brush where the picked target moves the mesh most, so
    /// it shows without a pointer (for the `--sculpt` screenshot).
    pub fn show_sculpt_brush(&mut self) {
        let (Some(s), Some(shown)) = (self.picked_shape(), self.shown_positions()) else {
            return;
        };
        let (vertices, offsets) = match self.shape_tools.target.and_then(|w| s.inbetween_at(w)) {
            Some(k) => (&s.inbetweens[k].vertices, &s.inbetweens[k].offsets),
            None => (&s.vertices, &s.offsets),
        };
        self.brush_at = vertices
            .iter()
            .zip(offsets)
            .max_by(|a, b| a.1.length().total_cmp(&b.1.length()))
            .and_then(|(v, _)| shown.get(*v as usize).copied());
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
            self.shape_tools.target = None;
        }
        ok
    }

    /// The Blendshapes card in the rig's Properties.
    pub(super) fn shapes_card(&mut self, ui: &mut Ui) {
        let Some(doc) = &self.doc else { return };
        let built = doc.rig.is_some();
        let list: Vec<Shape> = doc.mesh.shapes.clone();
        if self.shape_tools.scroll_to > 0 {
            self.shape_tools.scroll_to -= 1;
            ui.scroll_to_cursor(Some(Align::Min));
        }
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
        if list.len() >= SEARCH_FROM {
            ui.add(
                egui::TextEdit::singleline(&mut self.shape_tools.filter)
                    .hint_text("Search shapes")
                    .desired_width(f32::INFINITY),
            );
        }
        let filter = self.shape_tools.filter.to_lowercase();
        let shown: Vec<&Shape> = list
            .iter()
            .filter(|s| filter.is_empty() || s.name.to_lowercase().contains(&filter))
            .collect();
        egui::ScrollArea::vertical()
            .id_salt("shape_list")
            .max_height(240.0)
            .min_scrolled_height(160.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                for s in &shown {
                    self.shape_row(ui, s, &mut action);
                }
            });
        if shown.len() < list.len() {
            ui.label(
                RichText::new(format!(
                    "{} more hidden by the search.",
                    list.len() - shown.len()
                ))
                .size(11.5)
                .color(TEXT_FAINT),
            );
        }
        if built {
            ui.horizontal_wrapped(|ui| {
                let small =
                    |t: &str| egui::Button::new(RichText::new(t).size(12.5)).corner_radius(8);
                if ui
                    .add(small("Corrective from pose"))
                    .on_hover_text(
                        "A shape the test-posed bone turns on as it bends, to sculpt in this pose.",
                    )
                    .clicked()
                {
                    action = Some(ShapeAction::Corrective);
                }
                if ui.add(small("New shape")).clicked() {
                    action = Some(ShapeAction::New);
                }
                let free: Vec<String> = list
                    .iter()
                    .filter(|s| s.driver.is_none() && s.weight > 0.0)
                    .map(|s| s.name.clone())
                    .collect();
                if free.len() >= 2
                    && ui
                        .add(small("Combine shown"))
                        .on_hover_text(
                            "Add the shapes with a weight above zero together as a new shape.",
                        )
                        .clicked()
                {
                    action = Some(ShapeAction::Combine(free));
                }
                if !self.shape_undo.is_empty() && ui.add(small("Undo shape")).clicked() {
                    action = Some(ShapeAction::Undo);
                }
            });
            if let Some(s) = self.picked_shape().cloned() {
                self.sculpt_section(ui, &s, &mut action);
            }
        }
        match action {
            Some(ShapeAction::Pick(n)) => self.pick_shape(&n),
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
            Some(ShapeAction::Target(t)) => self.pick_target(t),
            Some(ShapeAction::AddInbetween) => {
                self.add_inbetween_here();
            }
            Some(ShapeAction::RemoveInbetween(w)) => {
                self.remove_inbetween(w);
            }
            Some(ShapeAction::PoseToTarget) => {
                self.pose_to_target();
            }
            None => {}
        }
    }

    /// One shape: its name (click to pick it), its weight slider (or, for a
    /// corrective, the weight its reader gives in this pose) and its menu.
    fn shape_row(&self, ui: &mut Ui, s: &Shape, action: &mut Option<ShapeAction>) {
        let picked = self.shape_pick.as_deref() == Some(s.name.as_str());
        ui.horizontal(|ui| {
            let label = if s.inbetweens.is_empty() {
                s.name.clone()
            } else {
                format!("{}  ·{}", s.name, s.inbetweens.len())
            };
            let name = RichText::new(label)
                .size(12.5)
                .color(if picked { ACCENT } else { TEXT });
            let tip = match &s.driver {
                Some(d) => format!(
                    "{} vertices, {} in-betweens. Driven by {}: off at {:.0}°, on at {:.0}°. Click to sculpt it.",
                    s.vertices.len(),
                    s.inbetweens.len(),
                    d.bone,
                    d.start,
                    d.end
                ),
                None => format!(
                    "{} vertices, {} in-betweens. Click to sculpt it.",
                    s.vertices.len(),
                    s.inbetweens.len()
                ),
            };
            let wide = (ui.available_width() - 168.0).max(90.0);
            if ui
                .add_sized([wide, 20.0], egui::Button::selectable(picked, name).truncate())
                .on_hover_text(tip)
                .clicked()
            {
                *action = Some(ShapeAction::Pick(s.name.clone()));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().slider_width = 44.0;
                ui.menu_button(RichText::new("...").size(12.5), |ui| {
                    let n = s.name.clone();
                    let items = [
                        ("Mirror to the other side", ShapeAction::Mirror(n.clone())),
                        ("Split into left and right", ShapeAction::Split(n.clone())),
                        ("Remove", ShapeAction::Remove(n)),
                    ];
                    for (label, chosen) in items {
                        if ui.button(label).clicked() {
                            *action = Some(chosen);
                            ui.close();
                        }
                    }
                });
                match &s.driver {
                    Some(_) => {
                        let mut shown = self.shape_weight_now(s);
                        ui.add_enabled(
                            false,
                            egui::Slider::new(&mut shown, 0.0..=1.0)
                                .show_value(true)
                                .max_decimals(2),
                        )
                        .on_disabled_hover_text("Its bone sets it: bend the test pose to see it.");
                        ui.label(RichText::new("\u{21bb}").color(TEXT_DIM))
                            .on_hover_text("Driven by a pose reader");
                    }
                    None => {
                        let mut w = s.weight;
                        if ui
                            .add(
                                egui::Slider::new(&mut w, 0.0..=1.0)
                                    .show_value(true)
                                    .max_decimals(2),
                            )
                            .changed()
                        {
                            *action = Some(ShapeAction::Weight(s.name.clone(), w));
                        }
                    }
                }
            });
        });
    }

    /// Under the picked shape: which of its targets to sculpt (in-betweens
    /// included), the sculpt brushes, and their size and strength.
    fn sculpt_section(&mut self, ui: &mut Ui, s: &Shape, action: &mut Option<ShapeAction>) {
        ui.add_space(4.0);
        theme::label_caps(ui, &format!("Sculpt {}", s.name));
        ui.label(RichText::new("Target").size(11.5).color(TEXT_FAINT));
        ui.horizontal_wrapped(|ui| {
            let mut target = self.shape_tools.target;
            for b in &s.inbetweens {
                let on = target.is_some_and(|t| (t - b.weight).abs() < 0.005);
                let r = ui
                    .add(egui::Button::selectable(
                        on,
                        RichText::new(format!("{:.2}", b.weight)).size(12.5),
                    ))
                    .on_hover_text(
                        "An in-between: where the vertices are at this weight. Right-click to remove it.",
                    );
                if r.clicked() {
                    target = Some(b.weight);
                }
                r.context_menu(|ui| {
                    if ui.button("Remove this in-between").clicked() {
                        *action = Some(ShapeAction::RemoveInbetween(b.weight));
                        ui.close();
                    }
                });
            }
            if ui
                .add(egui::Button::selectable(
                    target.is_none(),
                    RichText::new("Full 1.00").size(12.5),
                ))
                .clicked()
            {
                target = None;
            }
            if ui
                .add(egui::Button::new(RichText::new("+ In-between").size(12.5)).corner_radius(8))
                .on_hover_text(
                    "Add a target at the shape's weight now (or half way), so the shape passes \
                     through it on the way to the full shape.",
                )
                .clicked()
            {
                *action = Some(ShapeAction::AddInbetween);
            }
            if target != self.shape_tools.target && action.is_none() {
                *action = Some(ShapeAction::Target(target));
            }
        });
        // A corrective sculpted away from its target's pose lands the edit
        // where the target is fully on, which isn't what shows.
        let want = self.shape_tools.target.unwrap_or(1.0);
        let now = self.shape_weight_now(s);
        if s.driver.is_some() && (now - want).abs() > 0.02 {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(format!(
                        "This pose shows it at {now:.2}; the target is fully on at {want:.2}."
                    ))
                    .size(11.5)
                    .color(WARN),
                );
                if ui
                    .add(egui::Button::new(RichText::new("Pose there").size(12.0)).corner_radius(8))
                    .clicked()
                {
                    *action = Some(ShapeAction::PoseToTarget);
                }
            });
        }
        ui.horizontal(|ui| {
            let mut on = self.sculpting;
            theme::pill(ui, &mut on, "Sculpt").on_hover_text(
                "Drag on the mesh to sculpt the picked target in the current pose \
                 ([ and ] resize, Ctrl deflates with Inflate).",
            );
            if on != self.sculpting {
                self.set_mesh_tool(if on {
                    super::MeshTool::Sculpt
                } else {
                    super::MeshTool::View
                });
            }
        });
        let brushes: Vec<(SculptBrush, &str)> =
            SculptBrush::ALL.iter().map(|b| (*b, b.label())).collect();
        theme::segmented(ui, &mut self.shape_tools.brush, &brushes);
        let tip = match self.shape_tools.brush {
            SculptBrush::Grab => "Drag the vertices under the brush with the pointer.",
            SculptBrush::Smooth => "Even out bumps: each vertex moves toward its neighbours.",
            SculptBrush::Relax => {
                "Even out the vertices along the surface, keeping its shape and volume."
            }
            SculptBrush::Inflate => "Push the surface out along its normals (Ctrl: in).",
            SculptBrush::Erase => "Take this target's change back off under the brush.",
        };
        ui.label(RichText::new(tip).size(11.5).color(TEXT_FAINT));
        let height = self
            .doc
            .as_ref()
            .map_or(1.8, |d| d.guides.body.height)
            .max(1e-3);
        let mut cm = self.sculpt_radius * height * 100.0;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Radius").size(12.0).color(TEXT_DIM));
            if ui
                .add(
                    egui::Slider::new(&mut cm, 0.5..=0.2 * height * 100.0)
                        .suffix(" cm")
                        .max_decimals(1),
                )
                .changed()
            {
                self.sculpt_radius = cm / 100.0 / height;
            }
        });
        if self.shape_tools.brush != SculptBrush::Grab {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Strength").size(12.0).color(TEXT_DIM));
                ui.add(
                    egui::Slider::new(&mut self.shape_tools.strength, 0.01..=1.0).max_decimals(2),
                );
            });
        }
    }
}
