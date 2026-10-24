//! Posing in the Rig stage: pick controls, move, turn and scale them with a
//! manipulator or the channel box, and watch the skeleton and skin follow.
//! The control rig itself runs in `hive_rig::pose`; this keeps the pose, the
//! control selection, the tool and the pose's own undo history.

use eframe::egui::{self, Color32, Pos2, Rect, Stroke};
use glam::{Mat3, Mat4, Quat, Vec3};

use hive_rig::controls::ControlRig;
use hive_rig::pose::{self, CHANNEL_NAMES, Channels, Pose, Posed};
use hive_rig::skeleton::Skeleton;

use super::{Mode, RigPick, Studio, UNDO_LIMIT};
use crate::view::Projector;

/// The manipulator in use, picked with Q, W, E and R as in Maya.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    Select,
    Move,
    Rotate,
    Scale,
}

impl Tool {
    pub const ALL: [Tool; 4] = [Tool::Select, Tool::Move, Tool::Rotate, Tool::Scale];

    pub fn label(self) -> &'static str {
        match self {
            Tool::Select => "Select",
            Tool::Move => "Move",
            Tool::Rotate => "Rotate",
            Tool::Scale => "Scale",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Tool::Select => "Q",
            Tool::Move => "W",
            Tool::Rotate => "E",
            Tool::Scale => "R",
        }
    }
}

/// One manipulator change, from where the drag started.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum GizmoOp {
    /// Move by this much, in model space.
    Move(Vec3),
    /// Turn about this model-space axis (through each control's pivot), radians.
    Rotate(Vec3, f32),
    /// Scale by this factor along one of the control's own axes, or all three.
    Scale(Option<usize>, f32),
}

/// The pose being edited and how.
pub struct Posing {
    pub pose: Pose,
    /// Picked controls (internal names), the lead (where the manipulator sits) last.
    pub selected: Vec<String>,
    pub tool: Tool,
    /// The manipulator's axes are the model's, not the control's own.
    pub world: bool,
    undo: Vec<Pose>,
    redo: Vec<Pose>,
    /// The pose when the current channel edit began.
    pub(super) drag_start: Option<Pose>,
    /// The manipulator drag in progress.
    pub gizmo: Option<GizmoDrag>,
}

impl Default for Posing {
    fn default() -> Self {
        Posing {
            pose: Pose::default(),
            selected: Vec::new(),
            tool: Tool::Move,
            world: false,
            undo: Vec::new(),
            redo: Vec::new(),
            drag_start: None,
            gizmo: None,
        }
    }
}

impl Posing {
    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }
}

/// Where a control's manipulator sits and how it is turned.
#[derive(Clone, Copy, Debug)]
pub struct Manip {
    /// The control's posed pivot.
    pub at: Vec3,
    /// The control's own axes, posed (columns).
    pub local: Mat3,
}

impl Studio {
    /// The skeleton and controls posing works on: the preview's (internal names).
    fn pose_rig(&self) -> Option<(&Skeleton, &ControlRig)> {
        let built = self.doc.as_ref()?.preview.as_ref().ok()?;
        Some((&built.skeleton, &built.controls))
    }

    /// Whether the posed rig is what shows: the build stages, a pose away from
    /// rest, and no test pose in its place.
    pub fn pose_shows(&self) -> bool {
        self.mode != Mode::Guides && self.pose.is_none() && !self.posing.pose.is_rest()
    }

    /// The rig run in the current pose.
    pub fn posed(&self) -> Option<Posed> {
        let (sk, rig) = self.pose_rig()?;
        Some(pose::evaluate(sk, rig, &self.posing.pose))
    }

    /// The skinning matrices of the shown pose, when the control rig poses it.
    pub fn pose_mats(&self) -> Option<Vec<Mat4>> {
        if self.pose_shows() {
            self.posed().map(|p| p.bones)
        } else {
            None
        }
    }

    /// Record the pose before an edit, for undo.
    pub fn begin_pose_edit(&mut self) {
        let p = &mut self.posing;
        p.undo.push(p.pose.clone());
        if p.undo.len() > UNDO_LIMIT {
            p.undo.remove(0);
        }
        p.redo.clear();
    }

    pub fn undo_pose(&mut self) -> bool {
        let p = &mut self.posing;
        let Some(prev) = p.undo.pop() else {
            return false;
        };
        p.redo.push(std::mem::replace(&mut p.pose, prev));
        self.info("Undid the pose change.");
        true
    }

    pub fn redo_pose(&mut self) -> bool {
        let p = &mut self.posing;
        let Some(next) = p.redo.pop() else {
            return false;
        };
        p.undo.push(std::mem::replace(&mut p.pose, next));
        self.info("Redid the pose change.");
        true
    }

    /// Pick a control: alone, or (`add`) toggled in the selection.
    pub fn select_control(&mut self, name: &str, add: bool) {
        let sel = &mut self.posing.selected;
        if add {
            if let Some(i) = sel.iter().position(|n| n == name) {
                sel.remove(i);
            } else {
                sel.push(name.to_string());
            }
        } else {
            sel.clear();
            sel.push(name.to_string());
        }
        self.sync_rig_pick();
    }

    pub fn clear_control_selection(&mut self) {
        self.posing.selected.clear();
        self.sync_rig_pick();
    }

    /// Properties show the lead control.
    fn sync_rig_pick(&mut self) {
        match self.posing.selected.last() {
            Some(lead) => self.rig_pick = Some(RigPick::Control(lead.clone())),
            None => {
                if matches!(self.rig_pick, Some(RigPick::Control(_))) {
                    self.rig_pick = None;
                }
            }
        }
    }

    /// Drop picked controls the rig no longer has (after a rebuild).
    pub fn sync_control_selection(&mut self) {
        let Some((_, rig)) = self.pose_rig() else {
            return;
        };
        let keep: Vec<String> = self
            .posing
            .selected
            .iter()
            .filter(|n| rig.control(n).is_some())
            .cloned()
            .collect();
        if keep.len() != self.posing.selected.len() {
            self.posing.selected = keep;
            self.sync_rig_pick();
        }
    }

    /// Where the lead control's manipulator sits in the current pose.
    pub fn manip(&self) -> Option<Manip> {
        let lead = self.posing.selected.last()?;
        let (sk, rig) = self.pose_rig()?;
        let i = rig.controls.iter().position(|c| &c.name == lead)?;
        let c = &rig.controls[i];
        let posed = self.posed()?;
        let m = posed.controls[i];
        let at = m.transform_point3(pose::pivot(c, sk));
        let (_, rot, _) = m.to_scale_rotation_translation();
        let local = Mat3::from_quat(rot * pose::rest_frame(c, sk));
        Some(Manip { at, local })
    }

    /// Apply a manipulator change to every picked control, measured from
    /// `start` (the pose when the drag began), so a drag never drifts.
    pub fn apply_gizmo(&mut self, start: &Pose, op: GizmoOp) {
        let Some((sk, rig)) = self.pose_rig() else {
            return;
        };
        let posed = pose::evaluate(sk, rig, start);
        let mut out = start.clone();
        for name in &self.posing.selected {
            let Some(i) = rig.controls.iter().position(|c| &c.name == name) else {
                continue;
            };
            let c = &rig.controls[i];
            let locks = pose::locks(c, sk);
            let frame = pose::rest_frame(c, sk);
            let parent = posed.parents[i];
            let mut ch = start.channels(name);
            match op {
                GizmoOp::Move(delta) if !locks.translate => {
                    let m = Mat3::from_mat4(parent) * Mat3::from_quat(frame);
                    if m.determinant().abs() > 1e-12 {
                        ch.translate += m.inverse() * delta;
                    }
                }
                GizmoOp::Rotate(axis, angle) if !locks.rotate => {
                    let (_, prot, _) = parent.to_scale_rotation_translation();
                    let base = (prot * frame).normalize();
                    let turn = Quat::from_axis_angle(axis.normalize_or_zero(), angle);
                    ch.set_rotation(base.inverse() * turn * base * ch.rotation());
                }
                GizmoOp::Scale(axis, factor) if !locks.scale => {
                    let f = factor.max(0.01);
                    match axis {
                        Some(a) => ch.scale[a.min(2)] *= f,
                        None => ch.scale *= f,
                    }
                }
                _ => continue,
            }
            out.set_channels(name, ch);
        }
        self.posing.pose = out;
    }

    /// Set a picked control's channels while a channel edit runs (undo is
    /// recorded when it starts).
    pub fn set_control_channels(&mut self, control: &str, ch: Channels) {
        self.posing.pose.set_channels(control, ch);
    }

    /// Put the picked controls (or every control) back at rest.
    pub fn reset_pose(&mut self, selected_only: bool) {
        if self.posing.pose.is_rest() {
            self.info("The rig is already at rest.");
            return;
        }
        self.begin_pose_edit();
        if selected_only {
            let names = self.posing.selected.clone();
            self.posing.pose.reset(&names);
            self.info(format!("Reset {}.", super::count(names.len(), "control")));
        } else {
            self.posing.pose = Pose::default();
            self.info("Reset the pose: every control is back at rest.");
        }
    }
}

/// A manipulator part: an axis (0..3) or the centre (all axes, or the screen plane).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    Axis(usize),
    Center,
}

/// A manipulator drag in progress.
#[derive(Clone, Debug)]
pub struct GizmoDrag {
    pub handle: Handle,
    pub start_pointer: Pos2,
    pub start: Pose,
    pub manip: Manip,
    /// The manipulator's axes when the drag began (columns).
    pub axes: Mat3,
    /// How long the manipulator's arms are, in model units.
    pub length: f32,
}

/// Screen length of the manipulator's arms.
const GIZMO_PIXELS: f32 = 80.0;
/// How near the pointer has to be to grab a manipulator part.
const GIZMO_REACH: f32 = 9.0;
const RING_STEPS: usize = 48;

/// Where the ray under `s` comes nearest the line through `at` along `dir`,
/// as a distance along it (None when they run side by side).
fn along_line(pr: &Projector, s: Pos2, at: Vec3, dir: Vec3) -> Option<f32> {
    let (o, r) = pr.ray(s);
    let w = at - o;
    let (a, b, c) = (dir.dot(dir), dir.dot(r), r.dot(r));
    let (d, e) = (dir.dot(w), r.dot(w));
    let denom = a * c - b * b;
    if denom.abs() < 1e-8 {
        return None;
    }
    Some((b * e - c * d) / denom)
}

/// Where the ray under `s` meets the plane through `at` facing `normal`.
fn on_plane(pr: &Projector, s: Pos2, at: Vec3, normal: Vec3) -> Option<Vec3> {
    let (o, r) = pr.ray(s);
    let denom = r.dot(normal);
    if denom.abs() < 1e-6 {
        return None;
    }
    Some(o + r * ((at - o).dot(normal) / denom))
}

fn distance_to_segment(p: Pos2, a: Pos2, b: Pos2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_sq().max(1e-6)).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

/// A circle of `radius` about `axis` through `at`, closed.
fn ring(at: Vec3, axis: Vec3, radius: f32) -> Vec<Vec3> {
    let axis = axis.normalize_or_zero();
    let u = axis.any_orthonormal_vector();
    let v = axis.cross(u);
    (0..=RING_STEPS)
        .map(|i| {
            let t = i as f32 / RING_STEPS as f32 * std::f32::consts::TAU;
            at + (u * t.cos() + v * t.sin()) * radius
        })
        .collect()
}

impl Studio {
    /// The manipulator's axes as drawn: the model's for Move and Rotate in
    /// world mode, the control's own otherwise (Scale is always local).
    fn gizmo_axes(&self, m: &Manip) -> Mat3 {
        if self.posing.world && self.posing.tool != Tool::Scale {
            Mat3::IDENTITY
        } else {
            m.local
        }
    }

    /// The manipulator's arm length in model units, so it shows a fixed size.
    fn gizmo_length(pr: &Projector, at: Vec3) -> f32 {
        let side = pr.forward.any_orthonormal_vector();
        match (pr.pos(at), pr.pos(at + side * 0.1)) {
            (Some(a), Some(b)) if a.distance(b) > 1e-3 => GIZMO_PIXELS * 0.1 / a.distance(b),
            _ => 0.1,
        }
    }

    /// The manipulator shows: the Rig stage, no test pose, a tool other than Select.
    pub fn gizmo_on(&self) -> bool {
        self.mode == Mode::Rig && self.posing.tool != Tool::Select && self.pose.is_none()
    }

    /// The manipulator part under the pointer.
    pub fn gizmo_handle(&self, pr: &Projector, at: Pos2) -> Option<Handle> {
        if !self.gizmo_on() {
            return None;
        }
        let m = self.manip()?;
        let axes = self.gizmo_axes(&m);
        let len = Self::gizmo_length(pr, m.at);
        let center = pr.pos(m.at)?;
        let mut best: Option<(f32, Handle)> = None;
        let mut offer = |d: f32, h: Handle| {
            if d <= GIZMO_REACH && best.is_none_or(|b| d < b.0) {
                best = Some((d, h));
            }
        };
        match self.posing.tool {
            Tool::Rotate => {
                for a in 0..3 {
                    for w in ring(m.at, axes.col(a), len).windows(2) {
                        if let (Some(p), Some(q)) = (pr.pos(w[0]), pr.pos(w[1])) {
                            offer(distance_to_segment(at, p, q), Handle::Axis(a));
                        }
                    }
                }
            }
            _ => {
                offer(center.distance(at) * 0.8, Handle::Center);
                for a in 0..3 {
                    if let Some(tip) = pr.pos(m.at + axes.col(a) * len) {
                        offer(distance_to_segment(at, center, tip), Handle::Axis(a));
                    }
                }
            }
        }
        best.map(|b| b.1)
    }

    /// Start dragging a manipulator part (records undo).
    pub fn begin_gizmo(&mut self, pr: &Projector, handle: Handle, pointer: Pos2) -> bool {
        let Some(manip) = self.manip() else {
            return false;
        };
        self.begin_pose_edit();
        self.posing.gizmo = Some(GizmoDrag {
            handle,
            start_pointer: pointer,
            start: self.posing.pose.clone(),
            axes: self.gizmo_axes(&manip),
            length: Self::gizmo_length(pr, manip.at),
            manip,
        });
        true
    }

    /// Follow the pointer in a manipulator drag.
    pub fn drag_gizmo(&mut self, pr: &Projector, pointer: Pos2) {
        let Some(g) = self.posing.gizmo.clone() else {
            return;
        };
        let at = g.manip.at;
        let line = |a: usize| {
            let d = g.axes.col(a);
            along_line(pr, g.start_pointer, at, d)
                .zip(along_line(pr, pointer, at, d))
                .map(|(t0, t1)| (d, t1 - t0))
        };
        let op = match (self.posing.tool, g.handle) {
            (Tool::Move, Handle::Center) => Some(GizmoOp::Move(
                pr.drag_point(pointer, at) - pr.drag_point(g.start_pointer, at),
            )),
            (Tool::Move, Handle::Axis(a)) => line(a).map(|(d, t)| GizmoOp::Move(d * t)),
            (Tool::Rotate, Handle::Axis(a)) => {
                let n = g.axes.col(a);
                on_plane(pr, g.start_pointer, at, n)
                    .zip(on_plane(pr, pointer, at, n))
                    .map(|(p0, p1)| {
                        let (v0, v1) = (p0 - at, p1 - at);
                        GizmoOp::Rotate(n, n.dot(v0.cross(v1)).atan2(v0.dot(v1)))
                    })
            }
            (Tool::Scale, Handle::Center) => {
                let dx = pointer.x - g.start_pointer.x;
                Some(GizmoOp::Scale(None, (dx / 150.0).exp()))
            }
            (Tool::Scale, Handle::Axis(a)) => {
                line(a).map(|(_, t)| GizmoOp::Scale(Some(a), (g.length + t) / g.length))
            }
            _ => None,
        };
        if let Some(op) = op {
            self.apply_gizmo(&g.start, op);
        }
    }

    /// End a manipulator drag; one that changed nothing leaves no undo step.
    pub fn end_gizmo(&mut self) {
        if let Some(g) = self.posing.gizmo.take()
            && g.start == self.posing.pose
        {
            self.posing.undo.pop();
        }
    }

    /// Draw the manipulator on the lead control.
    pub fn paint_gizmo(&self, painter: &egui::Painter, pr: &Projector, hover: Option<Pos2>) {
        if !self.gizmo_on() {
            return;
        }
        let Some(m) = self.manip() else {
            return;
        };
        let axes = self.gizmo_axes(&m);
        let len = Self::gizmo_length(pr, m.at);
        let active = self
            .posing
            .gizmo
            .as_ref()
            .map(|g| g.handle)
            .or_else(|| hover.and_then(|p| self.gizmo_handle(pr, p)));
        let colors = [
            Color32::from_rgb(235, 70, 70),
            Color32::from_rgb(90, 210, 90),
            Color32::from_rgb(80, 130, 245),
        ];
        let hot = Color32::from_rgb(255, 225, 60);
        let Some(center) = pr.pos(m.at) else {
            return;
        };
        for (a, base) in colors.iter().enumerate() {
            let color = if active == Some(Handle::Axis(a)) {
                hot
            } else {
                *base
            };
            let stroke = Stroke::new(2.5, color);
            match self.posing.tool {
                Tool::Rotate => {
                    let pts: Vec<Pos2> = ring(m.at, axes.col(a), len)
                        .iter()
                        .filter_map(|p| pr.pos(*p))
                        .collect();
                    painter.add(egui::Shape::line(pts, stroke));
                }
                tool => {
                    let Some(tip) = pr.pos(m.at + axes.col(a) * len) else {
                        continue;
                    };
                    painter.line_segment([center, tip], stroke);
                    if tool == Tool::Scale {
                        let r = Rect::from_center_size(tip, egui::vec2(9.0, 9.0));
                        painter.rect_filled(r, 1.0, color);
                    } else {
                        painter.circle_filled(tip, 4.5, color);
                    }
                }
            }
        }
        if self.posing.tool != Tool::Rotate {
            let color = if active == Some(Handle::Center) {
                hot
            } else {
                Color32::from_gray(230)
            };
            painter.circle_stroke(center, 6.0, Stroke::new(2.0, color));
        }
    }
}

impl Studio {
    /// The channel box for a picked control in the Rig stage: the tools, the
    /// nine channels (locked ones greyed), its custom attributes and resets.
    /// A drag or typed edit on any value is one undo step.
    pub fn channel_box(&mut self, ui: &mut egui::Ui, control: &str) {
        let Some((sk, rig)) = self.pose_rig() else {
            return;
        };
        let Some(c) = rig.control(control) else {
            return;
        };
        let locks = pose::locks(c, sk);
        let attrs: Vec<pose::Attribute> = pose::attributes(rig)
            .into_iter()
            .filter(|a| a.control == control)
            .collect();
        let values: Vec<f32> = attrs
            .iter()
            .map(|a| self.posing.pose.attribute(rig, control, &a.name))
            .collect();

        ui.horizontal_wrapped(|ui| {
            for tool in Tool::ALL {
                let on = self.posing.tool == tool;
                if ui
                    .selectable_label(on, tool.label())
                    .on_hover_text(format!("{} ({})", tool.label(), tool.key()))
                    .clicked()
                {
                    self.posing.tool = tool;
                }
            }
            ui.separator();
            let label = if self.posing.world { "World" } else { "Local" };
            if ui
                .selectable_label(self.posing.world, label)
                .on_hover_text("Move and rotate along the model's axes, or the control's own")
                .clicked()
            {
                self.posing.world = !self.posing.world;
            }
        });
        ui.add_space(4.0);

        let mut ch = self.posing.pose.channels(control);
        let mut editing = false;
        let mut changed = false;
        egui::Grid::new(("channels", control))
            .num_columns(4)
            .spacing([6.0, 3.0])
            .show(ui, |ui| {
                for (row, (title, locked)) in [
                    ("Translate", locks.translate),
                    ("Rotate", locks.rotate),
                    ("Scale", locks.scale),
                ]
                .into_iter()
                .enumerate()
                {
                    ui.label(title);
                    for k in 0..3 {
                        let i = row * 3 + k;
                        let mut v = ch.get(i);
                        let speed = match row {
                            0 => 0.002,
                            1 => 0.5,
                            _ => 0.005,
                        };
                        let r = ui
                            .add_enabled(
                                !locked,
                                egui::DragValue::new(&mut v).speed(speed).max_decimals(3),
                            )
                            .on_hover_text(CHANNEL_NAMES[i]);
                        editing |= r.dragged() || r.has_focus();
                        if r.changed() {
                            ch.set(i, v);
                            changed = true;
                        }
                    }
                    ui.end_row();
                }
            });
        if changed {
            self.start_channel_edit();
            self.set_control_channels(control, ch);
        }

        if !attrs.is_empty() {
            ui.add_space(4.0);
            for (a, value) in attrs.iter().zip(values) {
                let mut v = value;
                let r = if a.choices.is_empty() {
                    ui.add(egui::Slider::new(&mut v, a.min..=a.max).text(&a.name))
                } else {
                    let mut pick = v.round() as usize;
                    let r = egui::ComboBox::from_label(&a.name)
                        .selected_text(a.choices.get(pick).cloned().unwrap_or_default())
                        .show_index(ui, &mut pick, a.choices.len(), |i| a.choices[i].clone());
                    v = pick as f32;
                    r
                };
                editing |= r.dragged() || r.has_focus();
                if r.changed() {
                    self.start_channel_edit();
                    if let Some(Ok(built)) = self.doc.as_ref().map(|d| &d.preview) {
                        self.posing
                            .pose
                            .set_attribute(&built.controls, control, &a.name, v);
                    }
                }
            }
        }
        if !editing {
            self.posing.drag_start = None;
        }

        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    !self.posing.selected.is_empty(),
                    egui::Button::new("Reset picked"),
                )
                .clicked()
            {
                self.reset_pose(true);
            }
            if ui
                .add_enabled(!self.posing.pose.is_rest(), egui::Button::new("Reset pose"))
                .clicked()
            {
                self.reset_pose(false);
            }
        });
    }

    /// Record undo once at the start of a channel-box edit.
    fn start_channel_edit(&mut self) {
        if self.posing.drag_start.is_none() {
            self.posing.drag_start = Some(self.posing.pose.clone());
            self.begin_pose_edit();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hive_rig::controls::ControlKind;

    fn built() -> Studio {
        let mut s = Studio::default();
        s.load_demo();
        assert!(s.build(), "{}", s.status);
        s.mode = Mode::Rig;
        s
    }

    /// The FK control that turns `bone`.
    fn fk_on(s: &Studio, bone: &str) -> String {
        let (_, rig) = s.pose_rig().unwrap();
        rig.controls
            .iter()
            .find(|c| c.kind == ControlKind::Fk && c.bone.as_deref() == Some(bone))
            .map(|c| c.name.clone())
            .unwrap()
    }

    #[test]
    fn turning_an_fk_control_poses_the_skin_and_undoes() {
        let mut s = built();
        let rest = s.frame().unwrap();
        let name = fk_on(&s, "lowerarm_l");
        s.select_control(&name, false);
        assert_eq!(s.rig_pick, Some(RigPick::Control(name.clone())));
        let start = s.posing.pose.clone();
        s.begin_pose_edit();
        s.apply_gizmo(&start, GizmoOp::Rotate(Vec3::Y, 1.2));
        assert!(!s.posing.pose.is_rest());
        assert!(s.pose_shows());
        let posed = s.frame().unwrap();
        let sk = &s.pose_rig().unwrap().0;
        let hand = sk.index_of("hand_l").unwrap();
        assert!(posed.bones[hand].0.distance(rest.bones[hand].0) > 0.05);
        let moved = posed.mesh.as_ref().unwrap();
        let base = &s.doc.as_ref().unwrap().mesh.positions;
        assert!(moved.iter().zip(base).any(|(a, b)| a.distance(*b) > 0.05));
        // The turn is measured from the drag's start, so applying it again
        // gives the same pose, not twice the turn.
        let once = s.posing.pose.clone();
        s.apply_gizmo(&start, GizmoOp::Rotate(Vec3::Y, 1.2));
        assert_eq!(s.posing.pose, once);
        // Ctrl+Z in the Rig stage takes the pose back first.
        assert!(s.can_undo());
        s.undo();
        assert!(s.posing.pose.is_rest());
        s.redo();
        assert_eq!(s.posing.pose, once);
        s.reset_pose(false);
        assert!(s.posing.pose.is_rest());
    }

    #[test]
    fn locked_channels_ignore_the_manipulator() {
        let mut s = built();
        let name = fk_on(&s, "lowerarm_l");
        let (sk, rig) = s.pose_rig().unwrap();
        assert!(pose::locks(rig.control(&name).unwrap(), sk).translate);
        s.select_control(&name, false);
        let start = s.posing.pose.clone();
        s.apply_gizmo(&start, GizmoOp::Move(Vec3::X * 0.3));
        assert!(s.posing.pose.is_rest());
    }

    #[test]
    fn shift_adds_and_removes_controls() {
        let mut s = built();
        let a = fk_on(&s, "lowerarm_l");
        let b = fk_on(&s, "lowerarm_r");
        s.select_control(&a, false);
        s.select_control(&b, true);
        assert_eq!(s.posing.selected, vec![a.clone(), b.clone()]);
        assert_eq!(s.rig_pick, Some(RigPick::Control(b.clone())));
        s.select_control(&b, true);
        assert_eq!(s.posing.selected, vec![a.clone()]);
        assert!(s.manip().is_some());
        s.clear_control_selection();
        assert!(s.manip().is_none());
        assert_eq!(s.rig_pick, None);
    }
}
