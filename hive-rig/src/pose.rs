//! Posing a rig without a DCC: the animator controls' values, and an evaluator
//! that turns them into posed bones.
//!
//! A [`Pose`] holds what an animator has keyed by hand: each control's
//! translate, rotate and scale channels (offsets from where the control sits
//! at rest) and its custom attributes (`ik_fk`, `roll`, `stretch` and the rest,
//! listed by [`attributes`]). [`evaluate`] runs the control rig the way the
//! Blender and Maya builders wire it: FK controls turn their bones, two-bone
//! limbs solve toward their IK control in the plane of their pole and blend
//! with their FK chain, feet roll about the heel, ball and toe, spline chains
//! follow their curve controls, eyes aim, twist helpers take their share and
//! bendy segments arc. The result is a transform per bone that takes the rest
//! model space to the posed one: the skinning matrices.
//!
//! Every control has a rest frame (see [`rest_frame`]) and a pivot (see
//! [`pivot`]); its channels act in that frame about that pivot, under its
//! parent (a control, a bone, or its chosen space). Bones keep their rest
//! orientation in model space (joints carry translation only), so a bone's
//! posed transform is a rotation and scale about its head, then a move.

use std::collections::{BTreeMap, HashMap};

use glam::{EulerRot, Mat3, Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::controls::{Control, ControlKind, ControlParent, ControlRig, FootRoll, soft_reach};
use crate::skeleton::Skeleton;

/// One control's transform channels: offsets from its rest placement, in its
/// rest frame. Rotation is in degrees, applied X, then Y, then Z.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Channels {
    #[serde(default)]
    pub translate: Vec3,
    #[serde(default)]
    pub rotate: Vec3,
    #[serde(default = "one")]
    pub scale: Vec3,
}

fn one() -> Vec3 {
    Vec3::ONE
}

impl Default for Channels {
    fn default() -> Self {
        Channels {
            translate: Vec3::ZERO,
            rotate: Vec3::ZERO,
            scale: Vec3::ONE,
        }
    }
}

impl Channels {
    pub fn is_rest(&self) -> bool {
        *self == Channels::default()
    }

    /// The rotation channels as a quaternion (X first, then Y, then Z).
    pub fn rotation(&self) -> Quat {
        let r = self.rotate * (std::f32::consts::PI / 180.0);
        Quat::from_euler(EulerRot::ZYX, r.z, r.y, r.x)
    }

    /// Set the rotation channels from a quaternion, choosing the angles
    /// nearest the current ones so dragging past 180 degrees keeps counting.
    pub fn set_rotation(&mut self, q: Quat) {
        let (z, y, x) = q.normalize().to_euler(EulerRot::ZYX);
        let near = |a: f32, to: f32| {
            let mut a = a.to_degrees();
            while a - to > 180.0 {
                a -= 360.0;
            }
            while to - a > 180.0 {
                a += 360.0;
            }
            a
        };
        let old = self.rotate;
        self.rotate = Vec3::new(near(x, old.x), near(y, old.y), near(z, old.z));
    }

    /// The value of channel `index` (0..3 translate, 3..6 rotate, 6..9 scale).
    pub fn get(&self, index: usize) -> f32 {
        let v = [self.translate, self.rotate, self.scale][index / 3 % 3];
        v[index % 3]
    }

    pub fn set(&mut self, index: usize, value: f32) {
        let v = match index / 3 % 3 {
            0 => &mut self.translate,
            1 => &mut self.rotate,
            _ => &mut self.scale,
        };
        v[index % 3] = value;
    }
}

/// The channel names an animator types: `tx` .. `sz`, in [`Channels::get`] order.
pub const CHANNEL_NAMES: [&str; 9] = ["tx", "ty", "tz", "rx", "ry", "rz", "sx", "sy", "sz"];

/// A pose of a control rig: the channels and attributes that differ from rest,
/// by control name. Controls the rig doesn't have are ignored, so a pose
/// survives rebuilding the rig from edited guides.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Pose {
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub channels: BTreeMap<String, Channels>,
    /// Control -> attribute -> value.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attributes: BTreeMap<String, BTreeMap<String, f32>>,
}

impl Pose {
    pub fn is_rest(&self) -> bool {
        self.channels.is_empty() && self.attributes.is_empty()
    }

    pub fn channels(&self, control: &str) -> Channels {
        self.channels.get(control).copied().unwrap_or_default()
    }

    pub fn set_channels(&mut self, control: &str, ch: Channels) {
        if ch.is_rest() {
            self.channels.remove(control);
        } else {
            self.channels.insert(control.to_string(), ch);
        }
    }

    /// An attribute's value in this pose, or its default.
    pub fn attribute(&self, rig: &ControlRig, control: &str, name: &str) -> f32 {
        self.attributes
            .get(control)
            .and_then(|a| a.get(name))
            .copied()
            .unwrap_or_else(|| attribute_default(rig, control, name))
    }

    /// Set an attribute, clamped to its range (and whole for a space). Setting
    /// it to its default takes it out of the pose. Returns false when the
    /// control has no such attribute.
    pub fn set_attribute(
        &mut self,
        rig: &ControlRig,
        control: &str,
        name: &str,
        value: f32,
    ) -> bool {
        let Some(def) = attributes(rig)
            .into_iter()
            .find(|a| a.control == control && a.name == name)
        else {
            return false;
        };
        let mut v = value.clamp(def.min, def.max);
        if !def.choices.is_empty() {
            v = v.round();
        }
        let entry = self.attributes.entry(control.to_string()).or_default();
        if v == def.default {
            entry.remove(name);
        } else {
            entry.insert(name.to_string(), v);
        }
        if entry.is_empty() {
            self.attributes.remove(control);
        }
        true
    }

    /// Put these controls back at rest: channels and attributes.
    pub fn reset(&mut self, controls: &[String]) {
        for c in controls {
            self.channels.remove(c);
            self.attributes.remove(c);
        }
    }
}

/// A custom attribute a control carries, as the builders add it.
#[derive(Debug, Clone, PartialEq)]
pub struct Attribute {
    pub control: String,
    pub name: String,
    pub default: f32,
    pub min: f32,
    pub max: f32,
    /// A space switch: the spaces by name; the value is an index into them.
    pub choices: Vec<String>,
}

/// Every custom attribute of the rig: on each IK control `ik_fk` (1 = IK),
/// `stretch`, `soft` (two-bone limbs) and `roll` (feet); on each spline's
/// settings control `stretch` and `volume`; and `space` on every control with
/// space switching.
pub fn attributes(rig: &ControlRig) -> Vec<Attribute> {
    let mut out = Vec::new();
    let mut add = |control: &str, name: &str, default: f32, min: f32, max: f32| {
        out.push(Attribute {
            control: control.to_string(),
            name: name.to_string(),
            default,
            min,
            max,
            choices: Vec::new(),
        })
    };
    for ch in &rig.ik_chains {
        add(&ch.ik_control, "ik_fk", ch.default_blend, 0.0, 1.0);
        add(&ch.ik_control, "stretch", ch.default_stretch, 0.0, 1.0);
        if ch.hock.is_none() {
            add(&ch.ik_control, "soft", ch.default_soft, 0.0, 1.0);
        }
        if ch.foot.is_some() {
            add(&ch.ik_control, "roll", 0.0, -90.0, 120.0);
        }
    }
    for sp in &rig.splines {
        add(&sp.settings, "stretch", sp.default_stretch, 0.0, 1.0);
        add(&sp.settings, "volume", sp.default_volume, 0.0, 1.0);
    }
    for c in &rig.controls {
        if c.spaces.len() > 1 {
            out.push(Attribute {
                control: c.name.clone(),
                name: "space".into(),
                default: 0.0,
                min: 0.0,
                max: (c.spaces.len() - 1) as f32,
                choices: c.spaces.clone(),
            });
        }
    }
    out
}

fn attribute_default(rig: &ControlRig, control: &str, name: &str) -> f32 {
    for ch in &rig.ik_chains {
        if ch.ik_control == control {
            match name {
                "ik_fk" => return ch.default_blend,
                "stretch" => return ch.default_stretch,
                "soft" => return ch.default_soft,
                _ => {}
            }
        }
    }
    for sp in &rig.splines {
        if sp.settings == control {
            match name {
                "stretch" => return sp.default_stretch,
                "volume" => return sp.default_volume,
                _ => {}
            }
        }
    }
    0.0
}

/// Which channels of a control are locked, as the builders lock them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Locks {
    pub translate: bool,
    pub rotate: bool,
    pub scale: bool,
}

/// FK controls that only turn their bone (and the FK layer of a spline, and
/// the hock control) can't move; bend and pin controls can't turn; only the
/// root and FK controls with a bone scale.
pub fn locks(c: &Control, sk: &Skeleton) -> Locks {
    let bone = c.bone.as_deref().and_then(|b| sk.bone(b));
    let moves_bone = bone.is_some_and(|b| b.translate || b.parent.is_none());
    Locks {
        translate: match c.kind {
            ControlKind::Fk => !moves_bone,
            ControlKind::Hock => true,
            _ => false,
        },
        rotate: matches!(c.kind, ControlKind::Bend | ControlKind::Pin),
        scale: !(c.kind == ControlKind::Root || (c.kind == ControlKind::Fk && bone.is_some())),
    }
}

/// The frame a control's channels act in at rest: along its bone (X down the
/// bone, as joints are oriented in Maya) for an FK control that turns a bone,
/// along its axis for curve, hock and FK-layer controls, and the model axes
/// for everything that is mostly moved (root, body, IK, pole, aim, pin, bend
/// and face controls).
pub fn rest_frame(c: &Control, sk: &Skeleton) -> Quat {
    let bone = c.bone.as_deref().and_then(|b| sk.bone(b));
    let axis = match (c.kind, bone) {
        (ControlKind::Fk, Some(b)) if !(b.translate || b.parent.is_none()) => b.direction(),
        (ControlKind::Fk, None) | (ControlKind::Spline | ControlKind::Hock, _) => c.axis,
        _ => return Quat::IDENTITY,
    };
    axis_frame(axis)
}

/// A rotation whose X axis points along `axis`, Y as near world up as it can
/// (forward when the axis is upright).
fn axis_frame(axis: Vec3) -> Quat {
    let x = axis.try_normalize().unwrap_or(Vec3::X);
    let up = if x.y.abs() < 0.9 { Vec3::Y } else { Vec3::Z };
    let z = x.cross(up).normalize();
    let y = z.cross(x).normalize();
    Quat::from_mat3(&Mat3::from_cols(x, y, z)).normalize()
}

/// Where a control turns and scales about: the head of the bone it drives
/// (so a clavicle control swings the shoulder, an eyelid control pivots on
/// the eye), or its own position.
pub fn pivot(c: &Control, sk: &Skeleton) -> Vec3 {
    match (c.kind, c.bone.as_deref().and_then(|b| sk.bone(b))) {
        (ControlKind::Fk | ControlKind::Root, Some(b)) => b.head,
        _ => c.position,
    }
}

/// A posed rig: for every bone and control, the transform from rest model
/// space to posed model space.
#[derive(Debug, Clone)]
pub struct Posed {
    /// The skinning matrices, one per bone.
    pub bones: Vec<Mat4>,
    /// One per control: its outline at rest, moved by this, is where it shows.
    pub controls: Vec<Mat4>,
    /// Each control's parent transform (what its channels act under).
    pub parents: Vec<Mat4>,
    rotations: Vec<Quat>,
    parent_of: Vec<Option<usize>>,
}

impl Posed {
    /// The rig at rest.
    pub fn rest(sk: &Skeleton, rig: &ControlRig) -> Posed {
        evaluate(sk, rig, &Pose::default())
    }

    pub fn head(&self, sk: &Skeleton, bone: usize) -> Vec3 {
        self.bones[bone].transform_point3(sk.bones[bone].head)
    }

    pub fn tail(&self, sk: &Skeleton, bone: usize) -> Vec3 {
        self.bones[bone].transform_point3(sk.bones[bone].tail)
    }

    /// Each bone's model-space rotation from rest.
    pub fn rotation(&self, bone: usize) -> Quat {
        self.rotations[bone]
    }

    /// Each bone's rotation relative to its parent's: what shape readers
    /// (correctives) measure.
    pub fn locals(&self) -> Vec<Quat> {
        self.rotations
            .iter()
            .zip(&self.parent_of)
            .map(|(r, p)| match p {
                Some(p) => (self.rotations[*p].inverse() * *r).normalize(),
                None => *r,
            })
            .collect()
    }
}

/// Run the control rig in `pose` on skeleton `sk`.
pub fn evaluate(sk: &Skeleton, rig: &ControlRig, pose: &Pose) -> Posed {
    let mut e = Eval::new(sk, rig, pose);
    let states: Vec<BoneState> = (0..sk.bones.len()).map(|i| e.bone(i)).collect();
    let controls: Vec<Mat4> = (0..rig.controls.len()).map(|i| e.control(i)).collect();
    let parents: Vec<Mat4> = (0..rig.controls.len()).map(|i| e.parent_frame(i)).collect();
    Posed {
        bones: states
            .iter()
            .zip(&sk.bones)
            .map(|(s, b)| s.matrix(b.head))
            .collect(),
        rotations: states.iter().map(|s| s.rot).collect(),
        parent_of: sk.bones.iter().map(|b| b.parent).collect(),
        controls,
        parents,
    }
}

/// A posed bone: where its head went, how it turned, and its scale in rest
/// axes (stretch, squash, or a scaled control).
#[derive(Debug, Clone, Copy)]
struct BoneState {
    head: Vec3,
    rot: Quat,
    k: Mat3,
}

impl BoneState {
    fn rest(head: Vec3) -> BoneState {
        BoneState {
            head,
            rot: Quat::IDENTITY,
            k: Mat3::IDENTITY,
        }
    }

    fn matrix(&self, rest_head: Vec3) -> Mat4 {
        Mat4::from_translation(self.head)
            * Mat4::from_mat3(Mat3::from_quat(self.rot) * self.k)
            * Mat4::from_translation(-rest_head)
    }

    /// Without scale: what controls hung from the bone follow.
    fn rigid(&self, rest_head: Vec3) -> Mat4 {
        Mat4::from_translation(self.head)
            * Mat4::from_quat(self.rot)
            * Mat4::from_translation(-rest_head)
    }

    /// Where a rest point fixed to the bone goes.
    fn carry(&self, rest_head: Vec3, point: Vec3) -> Vec3 {
        self.head + self.rot * (self.k * (point - rest_head))
    }
}

/// What moves a bone.
#[derive(Debug, Clone, Copy)]
enum Driver {
    /// Slot `1` of IK chain `0` (upper, lower, hock, end).
    Chain(usize, usize),
    /// Bend joint `2` of segment `1` (upper 0, lower 1) of chain `0`.
    Bend(usize, usize, usize),
    /// Joint `1` of spline `0`.
    Spline(usize, usize),
    /// Turned (and, for a moving bone, moved) by a control.
    Fk(usize),
    /// Aims at a control.
    Aim(usize),
    Twist,
    /// Rides on its parent.
    Follow,
}

struct ChainOut {
    bones: Vec<BoneState>,
    blend: f32,
}

struct Eval<'a> {
    sk: &'a Skeleton,
    rig: &'a ControlRig,
    pose: &'a Pose,
    control_index: HashMap<&'a str, usize>,
    drivers: Vec<Driver>,
    bones: Vec<Option<BoneState>>,
    controls: Vec<Option<Mat4>>,
    chains: Vec<Option<ChainOut>>,
    goals: Vec<Option<(Mat4, Mat4)>>,
    bends: Vec<Option<[Vec<BoneState>; 2]>>,
    splines: Vec<Option<Vec<BoneState>>>,
    busy_bones: Vec<bool>,
    busy_controls: Vec<bool>,
}

fn rot_of(m: Mat4) -> Quat {
    let (_, r, _) = m.to_scale_rotation_translation();
    r.normalize()
}

/// The rotation turning `from` onto `to` (both any length).
fn arc(from: Vec3, to: Vec3) -> Quat {
    match (from.try_normalize(), to.try_normalize()) {
        (Some(a), Some(b)) => Quat::from_rotation_arc(a, b),
        _ => Quat::IDENTITY,
    }
}

fn lerp3(a: Mat3, b: Mat3, t: f32) -> Mat3 {
    a * (1.0 - t) + b * t
}

/// Rigid blend of two transforms (rotation and translation; scale dropped).
fn blend_rigid(a: Mat4, b: Mat4, t: f32) -> Mat4 {
    if t <= 0.0 {
        return a;
    }
    if t >= 1.0 {
        return b;
    }
    let (_, ra, ta) = a.to_scale_rotation_translation();
    let (_, rb, tb) = b.to_scale_rotation_translation();
    Mat4::from_rotation_translation(ra.slerp(rb, t), ta.lerp(tb, t))
}

/// Scale `along` in direction `dir` and `across` perpendicular to it.
fn stretch_matrix(dir: Vec3, along: f32, across: f32) -> Mat3 {
    let d = dir.normalize_or_zero();
    let outer = Mat3::from_cols(d * d.x, d * d.y, d * d.z);
    Mat3::IDENTITY * across + outer * (along - across)
}

/// The turn of `q` about `axis` (the twist of a swing-twist split), in radians.
fn twist_about(q: Quat, axis: Vec3) -> f32 {
    let axis = axis.normalize_or_zero();
    let along = Vec3::new(q.x, q.y, q.z).dot(axis);
    let mut a = 2.0 * along.atan2(q.w);
    let pi = std::f32::consts::PI;
    while a > pi {
        a -= 2.0 * pi;
    }
    while a < -pi {
        a += 2.0 * pi;
    }
    a
}

/// Model-space rotation with X along `dir` and Z along the bend plane's
/// normal (the frame a limb bone keeps in its IK plane).
fn plane_frame(dir: Vec3, normal: Vec3) -> Mat3 {
    let x = dir.try_normalize().unwrap_or(Vec3::X);
    let z = (normal - x * normal.dot(x))
        .try_normalize()
        .unwrap_or_else(|| {
            let up = if x.y.abs() < 0.9 { Vec3::Y } else { Vec3::Z };
            x.cross(up).normalize()
        });
    Mat3::from_cols(x, z.cross(x), z)
}

/// A two-bone solve: from `root` toward `goal`, bending in the plane holding
/// `pole`, turned `twist` radians about the line to the goal. Returns the
/// middle joint, the chain's end and the bend plane's normal.
fn solve_two_bone(
    root: Vec3,
    goal: Vec3,
    pole: Vec3,
    upper: f32,
    lower: f32,
    reach: f32,
    twist: f32,
) -> (Vec3, Vec3, Vec3) {
    let to_goal = goal - root;
    let dir = to_goal.try_normalize().unwrap_or(Vec3::NEG_Y);
    let side = pole - root;
    let bend = (side - dir * side.dot(dir))
        .try_normalize()
        .unwrap_or_else(|| dir.any_orthonormal_vector());
    let bend = Quat::from_axis_angle(dir, twist) * bend;
    let reach = reach.clamp((upper - lower).abs() + 1e-6, upper + lower);
    let x = (upper * upper - lower * lower + reach * reach) / (2.0 * reach);
    let y = (upper * upper - x * x).max(0.0).sqrt();
    let mid = root + dir * x + bend * y;
    let end = root + dir * reach;
    (mid, end, dir.cross(bend))
}

/// The reverse-foot transforms (in rest model space) for a roll: the whole
/// foot's, and the planted toes' (heel and toe-tip pivots only).
fn foot_roll(foot: &FootRoll, roll: f32) -> (Mat4, Mat4) {
    let flat = (foot.toe_tip - foot.heel).with_y(0.0);
    let forward = flat.try_normalize().unwrap_or(Vec3::Z);
    let axis = Vec3::Y.cross(forward).normalize();
    let about = |at: Vec3, degrees: f32| {
        Mat4::from_translation(at)
            * Mat4::from_axis_angle(axis, degrees.to_radians())
            * Mat4::from_translation(-at)
    };
    let brk = foot.break_angle.max(0.0);
    let toes = about(foot.heel, roll.min(0.0)) * about(foot.toe_tip, (roll - brk).max(0.0));
    let whole = toes * about(foot.ball, roll.clamp(0.0, brk));
    (whole, toes)
}

impl<'a> Eval<'a> {
    fn new(sk: &'a Skeleton, rig: &'a ControlRig, pose: &'a Pose) -> Eval<'a> {
        let control_index: HashMap<&str, usize> = rig
            .controls
            .iter()
            .enumerate()
            .map(|(i, c)| (c.name.as_str(), i))
            .collect();
        let mut drivers: Vec<Option<Driver>> = vec![None; sk.bones.len()];
        let mut set = |name: &str, d: Driver| {
            if let Some(i) = sk.index_of(name)
                && drivers[i].is_none()
            {
                drivers[i] = Some(d);
            }
        };
        for (k, ch) in rig.ik_chains.iter().enumerate() {
            for (slot, b) in ch.bones().into_iter().enumerate() {
                set(b, Driver::Chain(k, slot));
            }
            if let Some(bendy) = &ch.bendy {
                for (seg, s) in [&bendy.upper, &bendy.lower].into_iter().enumerate() {
                    for (j, b) in s.joints.iter().enumerate() {
                        set(b, Driver::Bend(k, seg, j));
                    }
                }
            }
        }
        for (k, sp) in rig.splines.iter().enumerate() {
            for (j, b) in sp.bones.iter().enumerate() {
                set(b, Driver::Spline(k, j));
            }
        }
        for (i, c) in rig.controls.iter().enumerate() {
            match (c.kind, c.bone.as_deref()) {
                (ControlKind::Fk | ControlKind::Root, Some(b)) => set(b, Driver::Fk(i)),
                (ControlKind::Aim, Some(b)) => set(b, Driver::Aim(i)),
                _ => {}
            }
        }
        let drivers = drivers
            .into_iter()
            .zip(&sk.bones)
            .map(|(d, b)| {
                d.unwrap_or(if b.twist.is_some() {
                    Driver::Twist
                } else {
                    Driver::Follow
                })
            })
            .collect();
        Eval {
            sk,
            rig,
            pose,
            control_index,
            drivers,
            bones: vec![None; sk.bones.len()],
            controls: vec![None; rig.controls.len()],
            chains: (0..rig.ik_chains.len()).map(|_| None).collect(),
            goals: vec![None; rig.ik_chains.len()],
            bends: (0..rig.ik_chains.len()).map(|_| None).collect(),
            splines: (0..rig.splines.len()).map(|_| None).collect(),
            busy_bones: vec![false; sk.bones.len()],
            busy_controls: vec![false; rig.controls.len()],
        }
    }

    fn attr(&self, control: &str, name: &str) -> f32 {
        self.pose.attribute(self.rig, control, name)
    }

    fn control_named(&mut self, name: &str) -> Option<Mat4> {
        let i = *self.control_index.get(name)?;
        Some(self.control(i))
    }

    /// A control's posed point, for a rest point fixed to it.
    fn control_point(&mut self, name: &str, fallback: Vec3) -> Vec3 {
        let Some(&i) = self.control_index.get(name) else {
            return fallback;
        };
        let p = self.rig.controls[i].position;
        self.control(i).transform_point3(p)
    }

    fn control(&mut self, i: usize) -> Mat4 {
        if let Some(m) = self.controls[i] {
            return m;
        }
        if self.busy_controls[i] {
            return Mat4::IDENTITY;
        }
        self.busy_controls[i] = true;
        let parent = self.parent_frame(i);
        let c = &self.rig.controls[i];
        let ch = self.pose.channels(&c.name);
        let frame = Mat4::from_quat(rest_frame(c, self.sk));
        let at = pivot(c, self.sk);
        let local = Mat4::from_translation(at)
            * frame
            * Mat4::from_scale_rotation_translation(ch.scale, ch.rotation(), ch.translate)
            * frame.inverse()
            * Mat4::from_translation(-at);
        let m = parent * local;
        self.busy_controls[i] = false;
        self.controls[i] = Some(m);
        m
    }

    /// What a control's channels act under.
    fn parent_frame(&mut self, i: usize) -> Mat4 {
        let rig = self.rig;
        let c = &rig.controls[i];
        // The hock control rides on its foot's goal (after the roll).
        if c.kind == ControlKind::Hock
            && let Some(k) = rig
                .ik_chains
                .iter()
                .position(|ch| ch.hock.as_ref().is_some_and(|h| h.control == c.name))
        {
            return self.goal(k).0;
        }
        // A bend control rides on its segment's curve midpoint, turned with the segment.
        if c.kind == ControlKind::Bend
            && let Some((k, seg)) = rig.ik_chains.iter().enumerate().find_map(|(k, ch)| {
                let b = ch.bendy.as_ref()?;
                [&b.upper, &b.lower]
                    .iter()
                    .position(|s| s.control == c.name)
                    .map(|seg| (k, seg))
            })
        {
            let (start, end, rot) = self.bend_curve(k, seg);
            let mid = start.lerp(end, 0.5);
            return Mat4::from_translation(mid)
                * Mat4::from_quat(rot)
                * Mat4::from_translation(-c.position);
        }
        // The toes stay planted on the toe pivot in IK and follow the foot in FK.
        if let Some(k) = rig
            .ik_chains
            .iter()
            .position(|ch| ch.foot.as_ref().is_some_and(|f| f.ball_control == c.name))
            && let Some(end) = self.sk.index_of(&rig.ik_chains[k].end)
        {
            let foot = self.bone(end).rigid(self.sk.bones[end].head);
            let toes = self.goal(k).1;
            let blend = self.chain(k).blend;
            return blend_rigid(foot, toes, blend);
        }
        if c.spaces.len() > 1 {
            let pick = self.attr(&c.name, "space").round().max(0.0) as usize;
            let space = c.spaces[pick.min(c.spaces.len() - 1)].clone();
            if let Some(m) = self.control_named(&space) {
                return m;
            }
        }
        match &c.parent {
            ControlParent::None => Mat4::IDENTITY,
            ControlParent::Control(name) => self.control_named(name).unwrap_or(Mat4::IDENTITY),
            ControlParent::Bone(name) => match self.sk.index_of(name) {
                Some(b) => self.bone(b).rigid(self.sk.bones[b].head),
                None => Mat4::IDENTITY,
            },
        }
    }

    fn parent_state(&mut self, bone: usize) -> Option<(BoneState, Vec3)> {
        let p = self.sk.bones[bone].parent?;
        Some((self.bone(p), self.sk.bones[p].head))
    }

    /// Where a bone's head goes when it rides on its parent.
    fn carried_head(&mut self, bone: usize) -> Vec3 {
        let head = self.sk.bones[bone].head;
        match self.parent_state(bone) {
            Some((p, ph)) => p.carry(ph, head),
            None => head,
        }
    }

    fn bone(&mut self, i: usize) -> BoneState {
        if let Some(s) = self.bones[i] {
            return s;
        }
        let rest = BoneState::rest(self.sk.bones[i].head);
        if self.busy_bones[i] {
            return rest;
        }
        self.busy_bones[i] = true;
        let s = self.solve_bone(i).unwrap_or(rest);
        self.busy_bones[i] = false;
        self.bones[i] = Some(s);
        s
    }

    fn solve_bone(&mut self, i: usize) -> Option<BoneState> {
        let sk = self.sk;
        let b = &sk.bones[i];
        Some(match self.drivers[i] {
            Driver::Chain(k, slot) => *self.chain(k).bones.get(slot)?,
            Driver::Bend(k, seg, j) => self.bend(k)[seg].get(j).copied()?,
            Driver::Spline(k, j) => *self.spline(k).get(j)?,
            Driver::Fk(c) => {
                let m = self.control(c);
                let rot = rot_of(m);
                let k = Mat3::from_quat(rot).transpose() * Mat3::from_mat4(m);
                let head = if b.translate || b.parent.is_none() {
                    m.transform_point3(b.head)
                } else {
                    self.carried_head(i)
                };
                BoneState { head, rot, k }
            }
            Driver::Aim(c) => {
                let target = {
                    let p = self.rig.controls[c].position;
                    self.control(c).transform_point3(p)
                };
                let (prot, pk) = self
                    .parent_state(i)
                    .map_or((Quat::IDENTITY, Mat3::IDENTITY), |(p, _)| (p.rot, p.k));
                let head = self.carried_head(i);
                let turn = arc(prot * b.direction(), target - head);
                BoneState {
                    head,
                    rot: (turn * prot).normalize(),
                    k: pk,
                }
            }
            Driver::Twist => {
                let t = b.twist.as_ref()?;
                let angle = sk.index_of(&t.source).map_or(0.0, |s| self.twist_of(s));
                let (prot, pk) = self
                    .parent_state(i)
                    .map_or((Quat::IDENTITY, Mat3::IDENTITY), |(p, _)| (p.rot, p.k));
                BoneState {
                    head: self.carried_head(i),
                    rot: (prot * Quat::from_axis_angle(b.direction(), t.factor * angle))
                        .normalize(),
                    k: pk,
                }
            }
            Driver::Follow => {
                let (prot, pk) = self
                    .parent_state(i)
                    .map_or((Quat::IDENTITY, Mat3::IDENTITY), |(p, _)| (p.rot, p.k));
                BoneState {
                    head: self.carried_head(i),
                    rot: prot,
                    k: pk,
                }
            }
        })
    }

    /// How far a bone has turned about its own length relative to its parent.
    fn twist_of(&mut self, bone: usize) -> f32 {
        let rot = self.bone(bone).rot;
        let prot = self
            .parent_state(bone)
            .map_or(Quat::IDENTITY, |(p, _)| p.rot);
        twist_about(prot.inverse() * rot, self.sk.bones[bone].direction())
    }

    /// An IK control's goal after the foot roll, and the planted toes' frame.
    fn goal(&mut self, k: usize) -> (Mat4, Mat4) {
        if let Some(g) = self.goals[k] {
            return g;
        }
        let rig = self.rig;
        let ch = &rig.ik_chains[k];
        let ik = self.control_named(&ch.ik_control).unwrap_or(Mat4::IDENTITY);
        let g = match &ch.foot {
            Some(foot) => {
                let (whole, toes) = foot_roll(foot, self.attr(&ch.ik_control, "roll"));
                (ik * whole, ik * toes)
            }
            None => (ik, ik),
        };
        self.goals[k] = Some(g);
        g
    }

    fn chain(&mut self, k: usize) -> &ChainOut {
        if self.chains[k].is_none() {
            let out = self.solve_chain(k);
            self.chains[k] = Some(out);
        }
        self.chains[k].as_ref().unwrap()
    }

    fn solve_chain(&mut self, k: usize) -> ChainOut {
        let (sk, rig) = (self.sk, self.rig);
        let ch = &rig.ik_chains[k];
        let idx: Option<Vec<usize>> = ch.bones().iter().map(|n| sk.index_of(n)).collect();
        let Some(idx) = idx else {
            return ChainOut {
                bones: Vec::new(),
                blend: 0.0,
            };
        };
        let rest: Vec<Vec3> = idx.iter().map(|&i| sk.bones[i].head).collect();
        let last = idx.len() - 1;
        let (root, scale) = match self.parent_state(idx[0]) {
            Some((p, ph)) => (p.carry(ph, rest[0]), p.k.determinant().abs().cbrt()),
            None => (rest[0], 1.0),
        };
        let blend = self.attr(&ch.ik_control, "ik_fk").clamp(0.0, 1.0);
        let stretch = self.attr(&ch.ik_control, "stretch").clamp(0.0, 1.0);
        let soft = if ch.hock.is_some() {
            0.0
        } else {
            self.attr(&ch.ik_control, "soft")
        };

        // FK: each bone turns with its FK control.
        let mut fk_names: Vec<&str> = vec![&ch.fk_controls[0], &ch.fk_controls[1]];
        if let Some(h) = &ch.hock {
            fk_names.push(&h.fk_control);
        }
        fk_names.push(&ch.fk_controls[2]);
        let fk: Vec<(Quat, Mat3)> = fk_names
            .iter()
            .map(|n| {
                let m = self.control_named(n).unwrap_or(Mat4::IDENTITY);
                let r = rot_of(m);
                (r, Mat3::from_quat(r).transpose() * Mat3::from_mat4(m))
            })
            .collect();

        // IK: the two-bone solve toward the goal (or the hock point), in the
        // plane of the pole.
        let (goal, _) = self.goal(k);
        let hock = ch
            .hock
            .as_ref()
            .and_then(|h| self.control_named(&h.control));
        let solve_rest = rest[2];
        let target = match hock {
            Some(m) => m.transform_point3(rest[2]),
            None => goal.transform_point3(rest[last]),
        };
        let pole_rest = rig
            .control(&ch.pole_control)
            .map_or(rest[1] + Vec3::Z, |c| c.position);
        let pole = self.control_point(&ch.pole_control, pole_rest);
        let upper = rest[0].distance(rest[1]);
        let lower = rest[1].distance(solve_rest);
        // At rest the solve's plane may sit a little off the bones' own: turn
        // the solve by that much so the rest pose solves to itself.
        let rest_twist = {
            let (mid, _, _) = solve_two_bone(
                rest[0],
                solve_rest,
                pole_rest,
                upper,
                lower,
                rest[0].distance(solve_rest),
                0.0,
            );
            let axis = (solve_rest - rest[0]).normalize_or_zero();
            let flat = |v: Vec3| v - axis * v.dot(axis);
            let (a, b) = (flat(mid - rest[0]), flat(rest[1] - rest[0]));
            if a.length() > 1e-6 * upper && b.length() > 1e-6 * upper {
                a.cross(b).dot(axis).atan2(a.dot(b))
            } else {
                0.0
            }
        };
        let (_, _, rest_normal) = solve_two_bone(
            rest[0],
            solve_rest,
            pole_rest,
            upper,
            lower,
            rest[0].distance(solve_rest),
            rest_twist,
        );
        let length = (upper + lower) * scale;
        let distance = root.distance(target);
        let reach = soft_reach(distance, length, soft, ch.soft_zone);
        let aim = reach + stretch * (distance - reach);
        let amount = if reach > 1e-6 { aim / reach } else { 1.0 };
        let (mid, end, normal) = solve_two_bone(
            root,
            target,
            pole,
            upper * scale * amount,
            lower * scale * amount,
            aim,
            rest_twist,
        );
        let turn = |from: Vec3, to: Vec3| {
            Quat::from_mat3(&(plane_frame(to, normal) * plane_frame(from, rest_normal).transpose()))
                .normalize()
        };
        let mut ik: Vec<Quat> = vec![
            turn(rest[1] - rest[0], mid - root),
            turn(solve_rest - rest[1], end - mid),
        ];
        if let Some(m) = hock {
            ik.push(rot_of(m));
        }
        ik.push(rot_of(goal));

        // Blend, then lay the bones head to tail.
        let mut bones = Vec::with_capacity(idx.len());
        let mut head = root;
        for (slot, &(fk_rot, fk_k)) in fk.iter().enumerate() {
            let rot = fk_rot.slerp(ik[slot], blend).normalize();
            let k = lerp3(fk_k, Mat3::IDENTITY * scale, blend);
            bones.push(BoneState { head, rot, k });
            if slot < last {
                let ik_len = if slot < 2 { scale * amount } else { scale };
                let offset = lerp3(fk_k, Mat3::IDENTITY * ik_len, blend);
                head += rot * (offset * (rest[slot + 1] - rest[slot]));
            }
        }
        ChainOut { bones, blend }
    }

    /// A bendy segment's curve: start, end and the segment bone's turn.
    fn bend_curve(&mut self, k: usize, seg: usize) -> (Vec3, Vec3, Quat) {
        let rig = self.rig;
        let Some(bendy) = &rig.ik_chains[k].bendy else {
            return (Vec3::ZERO, Vec3::ZERO, Quat::IDENTITY);
        };
        let pin_rest = rig.control(&bendy.pin).map_or(Vec3::ZERO, |c| c.position);
        let pin = self.control_point(&bendy.pin, pin_rest);
        let bones = &self.chain(k).bones;
        if bones.len() < 3 {
            return (pin, pin, Quat::IDENTITY);
        }
        if seg == 0 {
            (bones[0].head, pin, bones[0].rot)
        } else {
            (pin, bones[2].head, bones[1].rot)
        }
    }

    fn bend(&mut self, k: usize) -> &[Vec<BoneState>; 2] {
        if self.bends[k].is_none() {
            let out = self.solve_bend(k);
            self.bends[k] = Some(out);
        }
        self.bends[k].as_ref().unwrap()
    }

    fn solve_bend(&mut self, k: usize) -> [Vec<BoneState>; 2] {
        let rig = self.rig;
        let sk = self.sk;
        let Some(bendy) = &rig.ik_chains[k].bendy else {
            return [Vec::new(), Vec::new()];
        };
        let scale = self.chain(k).bones.first().map_or(Mat3::IDENTITY, |b| b.k);
        let mut out: [Vec<BoneState>; 2] = [Vec::new(), Vec::new()];
        for (seg, s) in [&bendy.upper, &bendy.lower].into_iter().enumerate() {
            let (start, end, rot) = self.bend_curve(k, seg);
            let rest_mid = rig.control(&s.control).map_or(Vec3::ZERO, |c| c.position);
            let offset = self.control_point(&s.control, rest_mid) - start.lerp(end, 0.5);
            let at = |t: f32| start.lerp(end, t) + offset * (4.0 * t * (1.0 - t));
            let n = s.joints.len().max(1) as f32;
            for (j, name) in s.joints.iter().enumerate() {
                let Some(bi) = sk.index_of(name) else {
                    out[seg].push(BoneState::rest(Vec3::ZERO));
                    continue;
                };
                let b = &sk.bones[bi];
                let (head, next) = (at(j as f32 / n), at((j + 1) as f32 / n));
                let mut r = arc(rot * b.direction(), next - head) * rot;
                if let Some(t) = &b.twist {
                    let angle = sk.index_of(&t.source).map_or(0.0, |src| self.twist_of(src));
                    r *= Quat::from_axis_angle(b.direction(), t.factor * angle);
                }
                out[seg].push(BoneState {
                    head,
                    rot: r.normalize(),
                    k: scale,
                });
            }
        }
        out
    }

    fn spline(&mut self, k: usize) -> &[BoneState] {
        if self.splines[k].is_none() {
            let out = self.solve_spline(k);
            self.splines[k] = Some(out);
        }
        self.splines[k].as_ref().unwrap()
    }

    fn solve_spline(&mut self, k: usize) -> Vec<BoneState> {
        let rig = self.rig;
        let sp = &rig.splines[k];
        let mats: Vec<Mat4> = sp
            .controls
            .iter()
            .map(|n| self.control_named(n).unwrap_or(Mat4::IDENTITY))
            .collect();
        let rots: Vec<Quat> = mats.iter().map(|m| rot_of(*m)).collect();
        let stretch = self.attr(&sp.settings, "stretch").clamp(0.0, 1.0);
        let volume = self.attr(&sp.settings, "volume").clamp(0.0, 1.0);
        let n = sp.points.len().min(sp.bones.len());
        // The posed curve points: each rest point moved by its controls' blend.
        let curve: Vec<Vec3> = (0..n)
            .map(|j| {
                let w = sp.weights.get(j).map_or(&[][..], |w| &w[..]);
                let mut p = Vec3::ZERO;
                let mut total = 0.0;
                for (m, &wk) in mats.iter().zip(w) {
                    p += m.transform_point3(sp.points[j]) * wk;
                    total += wk;
                }
                if total > 0.0 { p / total } else { sp.points[j] }
            })
            .collect();
        // Without stretch the joints keep their lengths and slide along the curve.
        let mut sliding = vec![curve.first().copied().unwrap_or(Vec3::ZERO)];
        let mut seg = 0;
        let mut travelled = 0.0;
        let mut wanted = 0.0;
        for j in 1..n {
            wanted += sp.points[j - 1].distance(sp.points[j]);
            loop {
                let (a, b) = if seg + 1 < n {
                    (curve[seg], curve[seg + 1])
                } else {
                    (curve[n.saturating_sub(2)], curve[n - 1])
                };
                let len = a.distance(b);
                if seg + 2 >= n || travelled + len >= wanted {
                    let dir = (b - a).try_normalize().unwrap_or(Vec3::Y);
                    sliding.push(a + dir * (wanted - travelled));
                    break;
                }
                travelled += len;
                seg += 1;
            }
        }
        let heads: Vec<Vec3> = (0..n).map(|j| sliding[j].lerp(curve[j], stretch)).collect();
        let blend = |w: &[f32]| {
            let mut q = Quat::from_xyzw(0.0, 0.0, 0.0, 0.0);
            for (r, &wk) in rots.iter().zip(w) {
                let r = if q.dot(*r) < 0.0 { -*r } else { *r };
                q += r * wk;
            }
            if q.length_squared() > 1e-12 {
                q.normalize()
            } else {
                Quat::IDENTITY
            }
        };
        let mut out = Vec::with_capacity(n);
        for j in 0..n {
            if j + 1 == n {
                let r = rots.last().copied().unwrap_or(Quat::IDENTITY);
                out.push(BoneState {
                    head: heads[j],
                    rot: r,
                    k: Mat3::IDENTITY,
                });
                break;
            }
            let rest_dir = sp.points[j + 1] - sp.points[j];
            let posed = heads[j + 1] - heads[j];
            let base = blend(sp.weights.get(j).map_or(&[][..], |w| &w[..]));
            let rot = (arc(base * rest_dir, posed) * base).normalize();
            let ratio = posed.length() / rest_dir.length().max(1e-9);
            let across = ratio.max(1e-3).powf(-0.5 * volume);
            out.push(BoneState {
                head: heads[j],
                rot,
                k: stretch_matrix(rest_dir, ratio, across),
            });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{self, AddOptions, Built, Guides, Preset};
    use crate::rig::detect_guides;

    fn biped() -> (Guides, Built) {
        let (g, _) = detect_guides(&crate::demo::demo_humanoid());
        let built = components::build(&g).unwrap();
        (g, built)
    }

    fn head(p: &Posed, b: &Built, bone: &str) -> Vec3 {
        p.head(&b.skeleton, b.skeleton.index_of(bone).unwrap())
    }

    fn tail(p: &Posed, b: &Built, bone: &str) -> Vec3 {
        p.tail(&b.skeleton, b.skeleton.index_of(bone).unwrap())
    }

    fn moved(ch: Vec3) -> Channels {
        Channels {
            translate: ch,
            ..Channels::default()
        }
    }

    fn turned(degrees: Vec3) -> Channels {
        Channels {
            rotate: degrees,
            ..Channels::default()
        }
    }

    fn ik(pose: &mut Pose, b: &Built, chain: &str, blend: f32) {
        let ctrl = b
            .controls
            .ik_chains
            .iter()
            .find(|c| c.name == chain)
            .unwrap()
            .ik_control
            .clone();
        assert!(pose.set_attribute(&b.controls, &ctrl, "ik_fk", blend));
    }

    fn assert_near(a: Vec3, b: Vec3, tol: f32, what: &str) {
        assert!(a.distance(b) < tol, "{what}: {a} vs {b}");
    }

    #[test]
    fn rest_pose_is_the_identity() {
        let (mut g, _) = biped();
        for preset in [Preset::Fingers, Preset::Eyes, Preset::Tentacle] {
            components::add_preset(&mut g, preset, &AddOptions::default()).unwrap();
        }
        for c in g.components.iter_mut() {
            if c.kind == components::Kind::Arm {
                c.options.bendy = true;
            }
        }
        let b = components::build(&g).unwrap();
        let p = Posed::rest(&b.skeleton, &b.controls);
        for (m, bone) in p.bones.iter().zip(&b.skeleton.bones) {
            assert!(
                m.abs_diff_eq(Mat4::IDENTITY, 1e-4),
                "{} moves at rest: {m}",
                bone.name
            );
        }
        for (m, c) in p.controls.iter().zip(&b.controls.controls) {
            assert!(
                m.abs_diff_eq(Mat4::IDENTITY, 1e-4),
                "{} moves at rest",
                c.name
            );
        }
        assert!(
            p.locals()
                .iter()
                .all(|q| q.abs_diff_eq(Quat::IDENTITY, 1e-4))
        );
    }

    #[test]
    fn fk_child_follows_its_parent() {
        let (_, b) = biped();
        let mut pose = Pose::default();
        ik(&mut pose, &b, "arm_l", 0.0);
        pose.set_channels("upperarm_l_fk_ctrl", turned(Vec3::new(0.0, 0.0, -60.0)));
        let p = evaluate(&b.skeleton, &b.controls, &pose);
        let sk = &b.skeleton;
        let shoulder = sk.bone("upperarm_l").unwrap().head;
        // The shoulder stays, the elbow and wrist swing about it by the same turn.
        assert_near(head(&p, &b, "upperarm_l"), shoulder, 1e-4, "shoulder");
        let turn = p.rotation(sk.index_of("upperarm_l").unwrap());
        for bone in ["lowerarm_l", "hand_l"] {
            let rest = sk.bone(bone).unwrap().head;
            assert_near(
                head(&p, &b, bone),
                shoulder + turn * (rest - shoulder),
                1e-4,
                bone,
            );
            assert!(head(&p, &b, bone).distance(rest) > 0.05, "{bone} moved");
        }
        assert!(turn.angle_between(Quat::IDENTITY).to_degrees() > 59.0);
        // The hand control rides along on the FK chain.
        let hc = b.controls.control("hand_l_fk_ctrl").unwrap();
        let i = b
            .controls
            .controls
            .iter()
            .position(|c| c.name == hc.name)
            .unwrap();
        assert_near(
            p.controls[i].transform_point3(hc.position),
            head(&p, &b, "hand_l"),
            1e-4,
            "hand control",
        );
        // Fingers hang from the hand bone and so does the twist helper.
        let local = p.locals()[sk.index_of("lowerarm_l").unwrap()];
        assert!(
            local.abs_diff_eq(Quat::IDENTITY, 1e-4),
            "elbow stays straight"
        );
    }

    #[test]
    fn ik_reaches_the_target_and_the_pole_sets_the_knee_plane() {
        let (_, b) = biped();
        let sk = &b.skeleton;
        let leg = b
            .controls
            .ik_chains
            .iter()
            .find(|c| c.name == "leg_l")
            .unwrap();
        assert_eq!(leg.default_blend, 1.0);
        let hip = sk.bone("thigh_l").unwrap().head;
        let ankle = sk.bone("foot_l").unwrap().head;
        let lengths = [
            hip.distance(sk.bone("calf_l").unwrap().head),
            sk.bone("calf_l").unwrap().head.distance(ankle),
        ];
        // Lift the foot forward and up: the leg bends to reach it.
        let lift = Vec3::new(0.05, 0.25, 0.2);
        let mut pose = Pose::default();
        pose.set_channels(&leg.ik_control, moved(lift));
        let p = evaluate(sk, &b.controls, &pose);
        let goal = ankle + lift;
        assert_near(head(&p, &b, "foot_l"), goal, 1e-3, "ankle reaches the goal");
        assert_near(head(&p, &b, "thigh_l"), hip, 1e-4, "hip stays");
        let knee = head(&p, &b, "calf_l");
        assert!(
            (knee.distance(hip) - lengths[0]).abs() < 1e-3,
            "thigh keeps its length"
        );
        assert!(
            (knee.distance(goal) - lengths[1]).abs() < 1e-3,
            "calf keeps its length"
        );
        // The knee bends toward the pole (forward), in the hip-goal-pole plane.
        let pole = b.controls.control(&leg.pole_control).unwrap().position;
        let n = (goal - hip).cross(pole - hip).normalize();
        assert!((knee - hip).dot(n).abs() < 1e-3, "knee in the pole plane");
        assert!(knee.z > hip.z.max(goal.z), "knee points forward: {knee}");
        // The foot keeps the IK control's orientation: unturned.
        let foot = p.rotation(sk.index_of("foot_l").unwrap());
        assert!(foot.abs_diff_eq(Quat::IDENTITY, 1e-4));

        // Swing the pole out to the side: the knee follows it into the new plane.
        let side = Vec3::new(0.6, 0.0, -0.3);
        pose.set_channels(&leg.pole_control, moved(side));
        let p = evaluate(sk, &b.controls, &pose);
        let knee2 = head(&p, &b, "calf_l");
        let n = (goal - hip).cross(pole + side - hip).normalize();
        assert!(
            (knee2 - hip).dot(n).abs() < 1e-3,
            "knee in the new pole plane"
        );
        assert!(knee2.x > knee.x + 0.02, "knee swung out: {knee2} vs {knee}");
        assert_near(head(&p, &b, "foot_l"), goal, 1e-3, "still reaches");

        // Out of reach: the leg straightens toward it, and stretch reaches it.
        let far = Vec3::new(0.0, -0.4, 0.0);
        let mut pose = Pose::default();
        pose.set_channels(&leg.ik_control, moved(far));
        let p = evaluate(sk, &b.controls, &pose);
        let reach = head(&p, &b, "foot_l").distance(hip);
        assert!((reach - (lengths[0] + lengths[1])).abs() < 1e-3);
        assert!(head(&p, &b, "foot_l").distance(ankle + far) > 0.1);
        assert!(pose.set_attribute(&b.controls, &leg.ik_control, "stretch", 1.0));
        let p = evaluate(sk, &b.controls, &pose);
        assert_near(
            head(&p, &b, "foot_l"),
            ankle + far,
            1e-3,
            "stretched to the goal",
        );
    }

    #[test]
    fn ik_fk_blend_mixes_the_two_chains() {
        let (_, b) = biped();
        let sk = &b.skeleton;
        let mut pose = Pose::default();
        // FK bends the knee back; IK keeps the foot planted.
        pose.set_channels("calf_l_fk_ctrl", turned(Vec3::new(0.0, 0.0, 70.0)));
        let eval = |pose: &Pose| evaluate(sk, &b.controls, pose);
        let ik_pose = eval(&pose);
        let ankle = sk.bone("foot_l").unwrap().head;
        assert_near(head(&ik_pose, &b, "foot_l"), ankle, 1e-4, "IK ignores FK");
        ik(&mut pose, &b, "leg_l", 0.0);
        let fk_pose = eval(&pose);
        let fk_ankle = head(&fk_pose, &b, "foot_l");
        assert!(fk_ankle.distance(ankle) > 0.1, "FK swings the foot");
        ik(&mut pose, &b, "leg_l", 0.5);
        let half = eval(&pose);
        let calf = sk.index_of("calf_l").unwrap();
        let (r_ik, r_fk, r_half) = (
            ik_pose.rotation(calf),
            fk_pose.rotation(calf),
            half.rotation(calf),
        );
        let total = r_ik.angle_between(r_fk);
        assert!((r_half.angle_between(r_ik) - total * 0.5).abs() < 1e-3);
        assert!((r_half.angle_between(r_fk) - total * 0.5).abs() < 1e-3);
        // The chain stays joined: the blended calf starts where the thigh ends.
        assert_near(
            head(&half, &b, "calf_l"),
            tail(&half, &b, "thigh_l"),
            1e-3,
            "joined",
        );
    }

    #[test]
    fn body_controls_carry_the_spine_and_leave_the_feet() {
        let (_, b) = biped();
        let sk = &b.skeleton;
        let mut pose = Pose::default();
        let down = Vec3::new(0.0, -0.15, 0.0);
        pose.set_channels("cog_ctrl", moved(down));
        let p = evaluate(sk, &b.controls, &pose);
        for bone in ["pelvis", "spine_01", "head", "hand_l"] {
            let rest = sk.bone(bone).unwrap().head;
            assert_near(head(&p, &b, bone), rest + down, 1e-3, bone);
        }
        // IK feet stay planted; the knees bend.
        let ankle = sk.bone("foot_l").unwrap().head;
        assert_near(head(&p, &b, "foot_l"), ankle, 1e-3, "foot planted");
        assert!(p.locals()[sk.index_of("calf_l").unwrap()].angle_between(Quat::IDENTITY) > 0.1);

        // The root moves everything, feet included.
        let mut pose = Pose::default();
        let over = Vec3::new(0.5, 0.0, 0.2);
        pose.set_channels("root_ctrl", moved(over));
        let p = evaluate(sk, &b.controls, &pose);
        for bone in ["pelvis", "foot_l", "hand_r", "head"] {
            let rest = sk.bone(bone).unwrap().head;
            assert_near(head(&p, &b, bone), rest + over, 1e-3, bone);
        }
        // Turning the head control turns the head about its own base.
        let mut pose = Pose::default();
        pose.set_channels("head_ctrl", turned(Vec3::new(0.0, 40.0, 0.0)));
        let p = evaluate(sk, &b.controls, &pose);
        let h = sk.bone("head").unwrap();
        assert_near(head(&p, &b, "head"), h.head, 1e-4, "head base stays");
        assert!(
            tail(&p, &b, "head").distance(h.tail) > 0.01,
            "the head turned"
        );
        assert_near(
            head(&p, &b, "neck_01"),
            sk.bone("neck_01").unwrap().head,
            1e-4,
            "neck",
        );
    }

    #[test]
    fn foot_roll_lifts_the_heel_then_the_toes() {
        let (_, b) = biped();
        let sk = &b.skeleton;
        let leg = b
            .controls
            .ik_chains
            .iter()
            .find(|c| c.name == "leg_l")
            .unwrap();
        let foot = leg.foot.clone().unwrap();
        let ankle = sk.bone("foot_l").unwrap().head;
        let mut pose = Pose::default();
        pose.set_attribute(&b.controls, &leg.ik_control, "roll", foot.break_angle);
        let p = evaluate(sk, &b.controls, &pose);
        // Up on the ball: the ankle rises, the toes stay on the floor.
        assert!(head(&p, &b, "foot_l").y > ankle.y + 0.01);
        assert_near(head(&p, &b, "ball_l"), foot.ball, 1e-3, "ball stays");
        assert_near(
            tail(&p, &b, "ball_l"),
            foot.toe_tip,
            1e-3,
            "toes stay planted",
        );
        // Past the break the toes lift too, about the toe tip.
        pose.set_attribute(
            &b.controls,
            &leg.ik_control,
            "roll",
            foot.break_angle + 30.0,
        );
        let p = evaluate(sk, &b.controls, &pose);
        assert!(head(&p, &b, "ball_l").y > foot.ball.y + 0.005);
        assert_near(tail(&p, &b, "ball_l"), foot.toe_tip, 1e-3, "on the toe tip");
        // Back on the heel: the toes come up.
        pose.set_attribute(&b.controls, &leg.ik_control, "roll", -20.0);
        let p = evaluate(sk, &b.controls, &pose);
        assert!(tail(&p, &b, "ball_l").y > foot.toe_tip.y + 0.01);
        assert!(head(&p, &b, "foot_l").y > ankle.y - 1e-4);
        // Attributes clamp to their range.
        pose.set_attribute(&b.controls, &leg.ik_control, "roll", 500.0);
        assert_eq!(pose.attribute(&b.controls, &leg.ik_control, "roll"), 120.0);
        assert!(!pose.set_attribute(&b.controls, &leg.ik_control, "nope", 1.0));
    }

    #[test]
    fn eyes_aim_and_spaces_switch() {
        let (mut g, _) = biped();
        components::add_preset(&mut g, Preset::Eyes, &AddOptions::default()).unwrap();
        let b = components::build(&g).unwrap();
        let sk = &b.skeleton;
        let mut pose = Pose::default();
        let master = b
            .controls
            .controls
            .iter()
            .find(|c| c.kind == ControlKind::Aim && c.bone.is_none())
            .unwrap();
        pose.set_channels(&master.name, moved(Vec3::new(0.3, 0.0, 0.0)));
        let p = evaluate(sk, &b.controls, &pose);
        let eye = sk.index_of("eye_l").unwrap();
        let dir = (p.tail(sk, eye) - p.head(sk, eye)).normalize();
        assert!(dir.x > 0.1, "the eye looks toward the moved target: {dir}");

        // The hand IK control can follow the chest instead of the root.
        let hand = b.controls.control("hand_l_ik_ctrl").unwrap();
        assert!(hand.spaces.len() > 2, "{:?}", hand.spaces);
        let attrs = attributes(&b.controls);
        let space = attrs
            .iter()
            .find(|a| a.control == hand.name && a.name == "space")
            .unwrap();
        let chest = space
            .choices
            .iter()
            .position(|s| s.starts_with("spine"))
            .unwrap();
        let mut pose = Pose::default();
        ik(&mut pose, &b, "arm_l", 1.0);
        pose.set_attribute(&b.controls, &hand.name, "space", chest as f32);
        pose.set_channels("cog_ctrl", moved(Vec3::new(0.0, -0.2, 0.0)));
        let p = evaluate(sk, &b.controls, &pose);
        let wrist = sk.bone("hand_l").unwrap().head;
        assert_near(
            head(&p, &b, "hand_l"),
            wrist - Vec3::Y * 0.2,
            1e-3,
            "hand follows the chest",
        );
    }

    #[test]
    fn spline_chains_follow_their_curve_controls() {
        let (mut g, _) = biped();
        components::add_preset(&mut g, Preset::Tentacle, &AddOptions::default()).unwrap();
        let b = components::build(&g).unwrap();
        let sk = &b.skeleton;
        let sp = &b.controls.splines[0];
        let tip = sp.controls.last().unwrap().clone();
        let mut pose = Pose::default();
        let shift = Vec3::new(0.0, 0.2, 0.0);
        pose.set_channels(&tip, moved(shift));
        let p = evaluate(sk, &b.controls, &pose);
        let first = sk.index_of(&sp.bones[0]).unwrap();
        let last = sk.index_of(sp.bones.last().unwrap()).unwrap();
        assert_near(p.head(sk, first), sk.bones[first].head, 1e-3, "base stays");
        assert!(
            p.head(sk, last).y > sk.bones[last].head.y + 0.1,
            "tip rises"
        );
        // The joints stay joined along the curve.
        for w in sp.bones.windows(2) {
            let (a, c) = (sk.index_of(&w[0]).unwrap(), sk.index_of(&w[1]).unwrap());
            assert_near(p.tail(sk, a), p.head(sk, c), 1e-3, &w[0]);
        }
    }

    #[test]
    fn channels_round_trip_rotations() {
        let mut ch = Channels::default();
        let q = Quat::from_euler(EulerRot::ZYX, 0.3, -0.4, 1.1);
        ch.set_rotation(q);
        assert!(ch.rotation().abs_diff_eq(q, 1e-5) || ch.rotation().abs_diff_eq(-q, 1e-5));
        assert!((ch.rotate.x - 1.1f32.to_degrees()).abs() < 1e-3);
        // Single-axis turns read as that channel alone.
        let mut ch = Channels::default();
        ch.set_rotation(Quat::from_rotation_y(0.5));
        assert!((ch.rotate - Vec3::new(0.0, 0.5f32.to_degrees(), 0.0)).length() < 1e-3);
        for (i, name) in CHANNEL_NAMES.iter().enumerate() {
            let mut c = Channels::default();
            c.set(i, 2.0);
            assert_eq!(c.get(i), 2.0, "{name}");
        }
        let mut pose = Pose::default();
        pose.set_channels("a", ch);
        assert!(!pose.is_rest());
        pose.reset(&["a".into()]);
        assert!(pose.is_rest());
    }

    #[test]
    fn locks_follow_what_the_builders_allow() {
        let (_, b) = biped();
        let lock = |n: &str| locks(b.controls.control(n).unwrap(), &b.skeleton);
        assert!(lock("upperarm_l_fk_ctrl").translate);
        assert!(!lock("upperarm_l_fk_ctrl").rotate);
        assert!(!lock("upperarm_l_fk_ctrl").scale);
        assert!(!lock("hand_l_ik_ctrl").translate && lock("hand_l_ik_ctrl").scale);
        assert!(!lock("pelvis_ctrl").translate, "the pelvis moves");
        assert!(!lock("root_ctrl").scale);
    }
}
