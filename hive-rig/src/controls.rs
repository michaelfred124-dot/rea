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
