//! Animation controls: the DCC-independent description of the control rig.
//!
//! Every exporter turns the same list into native objects: custom-shape bones in
//! Blender, NURBS curves with offset groups in Maya, and Control Rig controls in
//! Unreal. Positions are model space (Y-up, metres).

use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::biped::{self, LimbNames, SPINE_COUNT};
use crate::landmarks::Landmarks;
use crate::skeleton::{Side, Skeleton};

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

const CENTER: [f32; 3] = [1.0, 0.85, 0.1];
const LEFT: [f32; 3] = [0.1, 0.45, 1.0];
const RIGHT: [f32; 3] = [1.0, 0.15, 0.15];
const ROOT: [f32; 3] = [0.85, 0.3, 0.9];

fn side_color(side: Side) -> [f32; 3] {
    match side {
        Side::Center => CENTER,
        Side::Left => LEFT,
        Side::Right => RIGHT,
    }
}

pub fn ctrl_name(bone: &str) -> String {
    format!("{bone}_ctrl")
}

pub fn fk_name(bone: &str) -> String {
    format!("{bone}_fk_ctrl")
}

pub fn build(sk: &Skeleton, lm: &Landmarks) -> ControlRig {
    let h = lm.height;
    let mut rig = ControlRig::default();
    let bone = |n: &str| sk.bone(n).unwrap_or_else(|| panic!("missing bone {n}"));

    let mut add = |name: String,
                   kind: ControlKind,
                   parent: ControlParent,
                   driven: Option<&str>,
                   position: Vec3,
                   axis: Vec3,
                   shape: Shape,
                   size: f32,
                   color: [f32; 3]| {
        rig.controls.push(Control {
            name,
            kind,
            parent,
            bone: driven.map(str::to_string),
            position,
            axis,
            shape,
            size,
            color,
            spaces: Vec::new(),
        });
    };
    let pc = |n: &str| ControlParent::Control(n.to_string());

    let root = bone("root");
    add(
        ctrl_name("root"),
        ControlKind::Root,
        ControlParent::None,
        Some("root"),
        root.head,
        Vec3::Y,
        Shape::Circle,
        0.25 * h,
        ROOT,
    );
    let pelvis = bone("pelvis");
    add(
        "cog_ctrl".into(),
        ControlKind::Cog,
        pc("root_ctrl"),
        None,
        pelvis.head,
        Vec3::Y,
        Shape::Square,
        0.17 * h,
        CENTER,
    );
    add(
        ctrl_name("pelvis"),
        ControlKind::Fk,
        pc("cog_ctrl"),
        Some("pelvis"),
        pelvis.head,
        Vec3::Y,
        Shape::Circle,
        0.13 * h,
        CENTER,
    );

    let mut parent = "cog_ctrl".to_string();
    for i in 0..SPINE_COUNT {
        let n = biped::spine_name(i);
        let b = bone(&n);
        add(
            ctrl_name(&n),
            ControlKind::Fk,
            pc(&parent),
            Some(&n),
            b.head,
            b.direction(),
            Shape::Circle,
            0.12 * h,
            CENTER,
        );
        parent = ctrl_name(&n);
    }
    for n in ["neck_01", "head"] {
        let b = bone(n);
        let size = if n == "head" { 0.08 * h } else { 0.06 * h };
        add(
            ctrl_name(n),
            ControlKind::Fk,
            pc(&parent),
            Some(n),
            b.head,
            b.direction(),
            Shape::Circle,
            size,
            CENTER,
        );
        parent = ctrl_name(n);
    }

    let top_spine = ctrl_name(&biped::spine_name(SPINE_COUNT - 1));
    for side in [Side::Left, Side::Right] {
        let s = side.suffix();
        let color = side_color(side);
        let clav = format!("clavicle{s}");
        let b = bone(&clav);
        add(
            ctrl_name(&clav),
            ControlKind::Fk,
            pc(&top_spine),
            Some(&clav),
            b.tail,
            b.direction(),
            Shape::Diamond,
            0.035 * h,
            color,
        );

        let arm = biped::arm_names(side);
        let leg = biped::leg_names(side);
        let limbs: [(&str, LimbNames, String, f32, Vec3); 2] = [
            ("arm", arm, ctrl_name(&clav), 0.0, -Vec3::Z),
            ("leg", leg, ctrl_name("pelvis"), 1.0, Vec3::Z),
        ];
        for (kind, names, fk_parent, blend, bend_fallback) in limbs {
            let up = bone(&names.upper);
            let lo = bone(&names.lower);
            let end = bone(&names.end);
            let limb_size = if kind == "arm" { 0.045 * h } else { 0.06 * h };

            // FK chain.
            let mut p = fk_parent;
            for b in [up, lo, end] {
                add(
                    fk_name(&b.name),
                    ControlKind::Fk,
                    pc(&p),
                    Some(&b.name),
                    b.head,
                    b.direction(),
                    Shape::Circle,
                    limb_size,
                    color,
                );
                p = fk_name(&b.name);
            }

            // IK goal and pole.
            let ik = format!("{}_ik_ctrl", names.end);
            let ik_axis = if kind == "arm" {
                up.direction()
            } else {
                Vec3::Y
            };
            add(
                ik.clone(),
                ControlKind::IkEffector,
                pc("root_ctrl"),
                Some(&names.end),
                end.head,
                ik_axis,
                Shape::Cube,
                limb_size,
                color,
            );

            let mid = (up.head + end.head) * 0.5;
            let bend = (lo.head - mid).try_normalize().unwrap_or(bend_fallback);
            let bend = if bend.dot(bend_fallback) < 0.0 {
                bend_fallback
            } else {
                bend
            };
            let pole_name = format!("{}_pole_ctrl", names.lower);
            add(
                pole_name.clone(),
                ControlKind::Pole,
                pc("root_ctrl"),
                None,
                lo.head + bend * 0.3 * h,
                bend,
                Shape::Diamond,
                0.025 * h,
                color,
            );

            rig.ik_chains.push(IkChain {
                name: format!("{kind}{s}"),
                upper: names.upper.clone(),
                lower: names.lower.clone(),
                end: names.end.clone(),
                ik_control: ik,
                pole_control: pole_name,
                fk_controls: [
                    fk_name(&names.upper),
                    fk_name(&names.lower),
                    fk_name(&names.end),
                ],
                default_blend: blend,
                default_stretch: 0.0,
                foot: (kind == "leg").then(|| {
                    let ball = bone(&format!("ball{s}"));
                    let heel = if side == Side::Left {
                        lm.heel_l
                    } else {
                        crate::landmarks::mirror(lm.heel_l, lm.center_x)
                    };
                    FootRoll {
                        heel,
                        ball: ball.head,
                        toe_tip: ball.tail,
                        ball_bone: ball.name.clone(),
                        ball_control: ctrl_name(&ball.name),
                        break_angle: 30.0,
                    }
                }),
            });
        }

        // Toes follow the foot in both IK and FK.
        let ball = format!("ball{s}");
        let b = bone(&ball);
        add(
            ctrl_name(&ball),
            ControlKind::Fk,
            ControlParent::Bone(format!("foot{s}")),
            Some(&ball),
            b.head,
            b.direction(),
            Shape::Circle,
            0.035 * h,
            color,
        );
    }

    // Space switching, as animators expect from production rigs: hands can follow
    // the world, body, chest or head; feet the world or body; poles their goal.
    let chest = ctrl_name(&biped::spine_name(SPINE_COUNT - 1));
    let chains = rig.ik_chains.clone();
    let set_spaces = |rig: &mut ControlRig, name: &str, spaces: &[&str]| {
        if let Some(c) = rig.controls.iter_mut().find(|c| c.name == name) {
            c.spaces = spaces.iter().map(|s| s.to_string()).collect();
        }
    };
    for chain in &chains {
        if chain.name.starts_with("arm") {
            set_spaces(
                &mut rig,
                &chain.ik_control,
                &["root_ctrl", "cog_ctrl", &chest, "head_ctrl"],
            );
        } else {
            set_spaces(&mut rig, &chain.ik_control, &["root_ctrl", "cog_ctrl"]);
        }
        set_spaces(
            &mut rig,
            &chain.pole_control,
            &["root_ctrl", &chain.ik_control],
        );
    }
    set_spaces(
        &mut rig,
        "head_ctrl",
        &["neck_01_ctrl", &chest, "cog_ctrl", "root_ctrl"],
    );
    rig
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{demo::demo_humanoid, landmarks::detect};

    #[test]
    fn every_parent_exists_and_precedes_its_children() {
        let lm = detect(&demo_humanoid());
        let sk = biped::build(&lm);
        let rig = build(&sk, &lm);
        for (i, c) in rig.controls.iter().enumerate() {
            match &c.parent {
                ControlParent::Control(p) => {
                    let pi = rig.controls.iter().position(|x| &x.name == p).expect(p);
                    assert!(pi < i, "{} listed before parent {p}", c.name);
                }
                ControlParent::Bone(b) => assert!(sk.bone(b).is_some()),
                ControlParent::None => assert_eq!(c.kind, ControlKind::Root),
            }
            if let Some(b) = &c.bone {
                assert!(sk.bone(b).is_some(), "{b}");
            }
        }
        assert_eq!(rig.ik_chains.len(), 4);
        // Spaces start with the control's parent and only name earlier controls.
        for (i, c) in rig.controls.iter().enumerate() {
            if let Some(first) = c.spaces.first() {
                assert_eq!(
                    &c.parent,
                    &ControlParent::Control(first.clone()),
                    "{}",
                    c.name
                );
            }
            for sp in &c.spaces {
                let si = rig.controls.iter().position(|x| &x.name == sp).expect(sp);
                assert!(si < i, "{} space {sp} comes later", c.name);
            }
        }
        let legs: Vec<_> = rig.ik_chains.iter().filter(|c| c.foot.is_some()).collect();
        assert_eq!(legs.len(), 2);
        let f = legs[0].foot.as_ref().unwrap();
        assert!(f.heel.z < f.ball.z && f.ball.z < f.toe_tip.z);
        // Poles sit in front of knees and behind elbows.
        assert!(
            rig.control("calf_l_pole_ctrl").unwrap().position.z > sk.bone("calf_l").unwrap().head.z
        );
        assert!(
            rig.control("lowerarm_l_pole_ctrl").unwrap().position.z
                < sk.bone("lowerarm_l").unwrap().head.z
        );
    }
}
