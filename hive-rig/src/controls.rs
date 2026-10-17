//! Animation controls: the DCC-independent description of the control rig.
//!
//! Every exporter turns the same list into native objects: custom-shape bones in
//! Blender, NURBS curves with offset groups in Maya, and Control Rig controls in
//! Unreal. Positions are model space (Y-up, metres). `components` builds it.

use glam::Vec3;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    Circle,
    Square,
    Cube,
    Diamond,
    Cross,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlKind {
    /// World placement of the whole character.
    Root,
    /// Centre of gravity: moves the body without the feet.
    Cog,
    /// Forward kinematics: the control's rotation drives `bone`.
    Fk,
    /// IK goal for a two-bone chain.
    IkEffector,
    /// Pole vector aiming the middle joint of a two-bone chain.
    Pole,
    /// Look-at target: `bone`, if set, aims at the control (eyes). Without a bone it
    /// is the master that moves its aim targets together.
    Aim,
    /// Hock control of a three-segment leg: sits at the ankle under the IK goal
    /// and swings the hock about it, which sets where the two-bone IK aims.
    Hock,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "name", rename_all = "snake_case")]
pub enum ControlParent {
    None,
    Control(String),
    /// Follows a deform bone whatever drives it (e.g. the ball control follows the
    /// foot in both IK and FK).
    Bone(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Control {
    pub name: String,
    pub kind: ControlKind,
    pub parent: ControlParent,
    /// Bone this control drives (FK) or the end bone of its chain (IK effector).
    pub bone: Option<String>,
    pub position: Vec3,
    /// Normal of the shape's main plane (circle facing down the bone, etc.).
    pub axis: Vec3,
    pub shape: Shape,
    /// Radius / half-extent in metres.
    pub size: f32,
    pub color: [f32; 3],
    /// Space-switch targets. When non-empty, the first entry is the default
    /// parent and the animator can switch the control to follow any of the others.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub spaces: Vec<String>,
}

/// A two-bone limb with IK and FK controls and a blend between them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IkChain {
    pub name: String,
    pub upper: String,
    pub lower: String,
    pub end: String,
    pub ik_control: String,
    pub pole_control: String,
    pub fk_controls: [String; 3],
    /// Initial value of the `ik_fk` attribute: 1 = IK, 0 = FK.
    pub default_blend: f32,
    /// Initial value of the `stretch` attribute: 0 = rigid, 1 = the limb stretches
    /// to reach an IK goal beyond its length.
    pub default_stretch: f32,
    /// Reverse-foot pivots for legs, driven by a `roll` attribute on the IK control.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foot: Option<FootRoll>,
    /// Three-segment (digitigrade) legs: the extra segment between `lower` and
    /// `end`. The two-bone IK on `upper` and `lower` then reaches the hock point
    /// instead of the goal, `hock.bone` follows the hock control and `end`
    /// follows the IK goal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hock: Option<HockSegment>,
}

/// The extra segment of a three-segment leg (thigh, calf, hock, foot).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HockSegment {
    /// The bone from the hock joint down to the ankle (the metatarsus).
    pub bone: String,
    /// Its FK control, between the lower and end FK controls.
    pub fk_control: String,
    /// The IK hock control (`ControlKind::Hock`), placed at the ankle. Its
    /// parent in the data is the IK control; builders hang it under the
    /// reverse-foot goal so it follows the foot roll.
    pub control: String,
}

impl IkChain {
    /// Bones of the chain from the hip or shoulder down, including the hock.
    pub fn bones(&self) -> Vec<&str> {
        let mut out = vec![self.upper.as_str(), self.lower.as_str()];
        if let Some(h) = &self.hock {
            out.push(&h.bone);
        }
        out.push(&self.end);
        out
    }
}

/// Reverse-foot setup. A negative `roll` rocks back on the heel; a positive roll
/// lifts the heel around the ball, then (past `break_angle`) around the toe tip.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FootRoll {
    pub heel: Vec3,
    pub ball: Vec3,
    pub toe_tip: Vec3,
    pub ball_bone: String,
    pub ball_control: String,
    /// Degrees of ball roll before the toe pivot takes over.
    pub break_angle: f32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ControlRig {
    pub controls: Vec<Control>,
    pub ik_chains: Vec<IkChain>,
}

impl ControlRig {
    pub fn control(&self, name: &str) -> Option<&Control> {
        self.controls.iter().find(|c| c.name == name)
    }
}

pub fn ctrl_name(bone: &str) -> String {
    format!("{bone}_ctrl")
}

pub fn fk_name(bone: &str) -> String {
    format!("{bone}_fk_ctrl")
}
