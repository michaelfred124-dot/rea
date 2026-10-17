//! Animation controls: the DCC-independent description of the control rig.
//!
//! Every exporter turns the same list into native objects: custom-shape bones in
//! Blender, NURBS curves with offset groups in Maya, and Control Rig controls in
//! Unreal. Positions are model space (Y-up, metres). `components` builds it.

use glam::Vec3;
use serde::{Deserialize, Serialize, Serializer};

/// How a control is drawn. The geometry lives here, in [`Shape::strokes`]: the
/// Blender and Maya builders and the app all draw the same lines from it, and
/// Unreal picks the matching shape from its built-in library.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    Circle,
    Square,
    Cube,
    Diamond,
    Cross,
    /// Three rings, one in each plane.
    Sphere,
    /// A flat arrow pointing along the shape's up direction.
    Arrow,
    /// Arrowheads on both ends.
    DoubleArrow,
    /// A move cross with an arrowhead on each arm.
    FourArrows,
    Hexagon,
    Octagon,
    /// Pointing along the shape's up direction.
    Triangle,
    /// An arch over the shape's up side, closed by its diameter.
    HalfCircle,
    /// A square base in the control's plane with its apex along the axis.
    Pyramid,
    /// A four-pointed star.
    Star,
}

/// Line strips in a shape's own frame: `x` across, `y` along the control's axis
/// (the plane normal) and `z` up within the plane. A closed loop repeats its
/// first point at the end.
pub type Strokes = Vec<Vec<Vec3>>;

/// Segments in a full ring of the round shapes.
const RING_SEGMENTS: usize = 32;

impl Shape {
    pub const ALL: [Shape; 15] = [
        Shape::Circle,
        Shape::Square,
        Shape::Cube,
        Shape::Diamond,
        Shape::Cross,
        Shape::Sphere,
        Shape::Arrow,
        Shape::DoubleArrow,
        Shape::FourArrows,
        Shape::Hexagon,
        Shape::Octagon,
        Shape::Triangle,
        Shape::HalfCircle,
        Shape::Pyramid,
        Shape::Star,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Shape::Circle => "Circle",
            Shape::Square => "Square",
            Shape::Cube => "Cube",
            Shape::Diamond => "Diamond",
            Shape::Cross => "Cross",
            Shape::Sphere => "Sphere",
            Shape::Arrow => "Arrow",
            Shape::DoubleArrow => "Double arrow",
            Shape::FourArrows => "Four arrows",
            Shape::Hexagon => "Hexagon",
            Shape::Octagon => "Octagon",
            Shape::Triangle => "Triangle",
            Shape::HalfCircle => "Half circle",
            Shape::Pyramid => "Pyramid",
            Shape::Star => "Star",
        }
    }

    /// The shape's lines at unit size (radius or half-extent 1).
    pub fn strokes(self) -> Strokes {
        use std::f32::consts::{FRAC_PI_2, PI, TAU};
        // A closed loop through points in the control's plane, given as (across, up).
        fn flat(pts: &[(f32, f32)]) -> Vec<Vec3> {
            let mut out: Vec<Vec3> = pts.iter().map(|&(x, z)| Vec3::new(x, 0.0, z)).collect();
            out.push(out[0]);
            out
        }
        fn polygon(sides: usize, start_deg: f32) -> Vec<Vec3> {
            let pts: Vec<(f32, f32)> = (0..sides)
                .map(|i| {
                    let a = (start_deg + i as f32 * 360.0 / sides as f32).to_radians();
                    (a.cos(), a.sin())
                })
                .collect();
            flat(&pts)
        }
        fn arc(a: Vec3, b: Vec3, sweep: f32, segments: usize) -> Vec<Vec3> {
            (0..=segments)
                .map(|i| {
                    let t = i as f32 / segments as f32 * sweep;
                    a * t.cos() + b * t.sin()
                })
                .collect()
        }
        let ring = |a, b| {
            let mut r = arc(a, b, TAU, RING_SEGMENTS);
            r[RING_SEGMENTS] = r[0];
            r
        };
        const SQUARE: [(f32, f32); 4] = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)];
        match self {
            Shape::Circle => vec![ring(Vec3::X, Vec3::Z)],
            Shape::Square => vec![flat(&SQUARE)],
            Shape::Cube => {
                let face = |y: f32| -> Vec<Vec3> {
                    flat(&SQUARE).into_iter().map(|p| p.with_y(y)).collect()
                };
                let mut out = vec![face(-1.0), face(1.0)];
                out.extend(
                    SQUARE
                        .iter()
                        .map(|&(x, z)| vec![Vec3::new(x, -1.0, z), Vec3::new(x, 1.0, z)]),
                );
                out
            }
            Shape::Diamond => {
                let loop4 = |a: Vec3, b: Vec3| vec![a, b, -a, -b, a];
                vec![
                    loop4(Vec3::X, Vec3::Z),
                    loop4(Vec3::X, Vec3::Y),
                    loop4(Vec3::Y, Vec3::Z),
                ]
            }
            Shape::Cross => vec![vec![-Vec3::X, Vec3::X], vec![-Vec3::Z, Vec3::Z]],
            Shape::Sphere => vec![
                ring(Vec3::X, Vec3::Z),
                ring(Vec3::X, Vec3::Y),
                ring(Vec3::Y, Vec3::Z),
            ],
            Shape::Arrow => vec![flat(&[
                (-0.18, -1.0),
                (0.18, -1.0),
                (0.18, 0.15),
                (0.5, 0.15),
                (0.0, 1.0),
                (-0.5, 0.15),
                (-0.18, 0.15),
            ])],
            Shape::DoubleArrow => vec![flat(&[
                (0.0, -1.0),
                (0.5, -0.4),
                (0.18, -0.4),
                (0.18, 0.4),
                (0.5, 0.4),
                (0.0, 1.0),
                (-0.5, 0.4),
                (-0.18, 0.4),
                (-0.18, -0.4),
                (-0.5, -0.4),
            ])],
            Shape::FourArrows => {
                // One arm pointing up, from an inner corner to the next, turned four times.
                const ARM: [(f32, f32); 6] = [
                    (0.13, 0.13),
                    (0.13, 0.6),
                    (0.32, 0.6),
                    (0.0, 1.0),
                    (-0.32, 0.6),
                    (-0.13, 0.6),
                ];
                let pts: Vec<(f32, f32)> = (0..4)
                    .flat_map(|quarter| {
                        let (s, c) = (quarter as f32 * FRAC_PI_2).sin_cos();
                        ARM.iter()
                            .map(move |&(x, z)| (x * c - z * s, x * s + z * c))
                    })
                    .collect();
                vec![flat(&pts)]
            }
            Shape::Hexagon => vec![polygon(6, 90.0)],
            Shape::Octagon => vec![polygon(8, 22.5)],
            Shape::Triangle => vec![polygon(3, 90.0)],
            Shape::HalfCircle => {
                let mut arch = arc(Vec3::X, Vec3::Z, PI, RING_SEGMENTS / 2);
                arch.push(arch[0]);
                vec![arch]
            }
            Shape::Pyramid => {
                let base: Vec<Vec3> = flat(&SQUARE).into_iter().map(|p| p * 0.8).collect();
                let mut out: Strokes = base[..4].iter().map(|&p| vec![p, Vec3::Y]).collect();
                out.insert(0, base);
                out
            }
            Shape::Star => {
                let pts: Vec<(f32, f32)> = (0..8)
                    .map(|i| {
                        let a = (90.0 + i as f32 * 45.0f32).to_radians();
                        let r = if i % 2 == 0 { 1.0 } else { 0.32 };
                        (r * a.cos(), r * a.sin())
                    })
                    .collect();
                vec![flat(&pts)]
            }
        }
    }
}

/// The frame a control's shape is drawn in: across, along `axis`, and up. Up is
/// world up (+Y) flattened into the control's plane, or forward (+Z) when the
/// axis itself points up or down, so arrows point the same way in every program.
pub fn shape_frame(axis: Vec3) -> [Vec3; 3] {
    let along = axis.try_normalize().unwrap_or(Vec3::Y);
    let reference = if along.y.abs() > 0.9 {
        Vec3::Z
    } else {
        Vec3::Y
    };
    let up = (reference - along * along.dot(reference)).normalize();
    [along.cross(up), along, up]
}

/// `shape`'s lines at `position`, facing `axis`, at `size`, in model space.
pub fn place_strokes(shape: Shape, position: Vec3, axis: Vec3, size: f32) -> Strokes {
    let [across, along, up] = shape_frame(axis);
    shape
        .strokes()
        .into_iter()
        .map(|line| {
            line.into_iter()
                .map(|p| position + (across * p.x + along * p.y + up * p.z) * size)
                .collect()
        })
        .collect()
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

/// Written with one extra field, `outline`: the shape's lines in model space at
/// rest, which the Blender and Maya builders draw as they are. It is derived from
/// the other fields, so reading a rig back ignores it.
#[derive(Debug, Clone, Deserialize)]
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
    #[serde(default)]
    pub spaces: Vec<String>,
    /// Id of the component that built this control. Only known for a freshly
    /// built rig; it isn't written to rig files.
    #[serde(skip)]
    pub component: String,
}

impl Control {
    /// The control's shape in model space, drawn at `position` facing `axis`
    /// (its rest placement or a posed one).
    pub fn outline_at(&self, position: Vec3, axis: Vec3) -> Strokes {
        place_strokes(self.shape, position, axis, self.size)
    }

    pub fn outline(&self) -> Strokes {
        self.outline_at(self.position, self.axis)
    }
}

impl Serialize for Control {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Written<'a> {
            name: &'a str,
            kind: ControlKind,
            parent: &'a ControlParent,
            bone: &'a Option<String>,
            position: Vec3,
            axis: Vec3,
            shape: Shape,
            size: f32,
            color: [f32; 3],
            #[serde(skip_serializing_if = "<[String]>::is_empty")]
            spaces: &'a [String],
            outline: Vec<Vec<[f32; 3]>>,
        }
        // A tenth of a millimetre is plenty for a drawn line and keeps files small.
        let round = |v: Vec3| (v * 1e4).round().to_array().map(|x| x / 1e4);
        Written {
            name: &self.name,
            kind: self.kind,
            parent: &self.parent,
            bone: &self.bone,
            position: self.position,
            axis: self.axis,
            shape: self.shape,
            size: self.size,
            color: self.color,
            spaces: &self.spaces,
            outline: self
                .outline()
                .into_iter()
                .map(|line| line.into_iter().map(round).collect())
                .collect(),
        }
        .serialize(serializer)
    }
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
    /// Left/right control twins as `[left, right]`, for the animator tools' pose
    /// mirroring. Found from the internal names when the rig is built, so the DCC
    /// scripts never guess sides from the exported naming convention.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mirror_pairs: Vec<[String; 2]>,
}

impl ControlRig {
    pub fn control(&self, name: &str) -> Option<&Control> {
        self.controls.iter().find(|c| c.name == name)
    }

    /// Pair every left control with its right twin. Must run on internal names
    /// (`hand_l_ik_ctrl`), before a naming convention renames them.
    pub fn find_mirror_pairs(&mut self) {
        let names: std::collections::HashSet<&str> =
            self.controls.iter().map(|c| c.name.as_str()).collect();
        self.mirror_pairs = self
            .controls
            .iter()
            .filter_map(|c| {
                let right = right_twin(&c.name)?;
                names
                    .contains(right.as_str())
                    .then(|| [c.name.clone(), right])
            })
            .collect();
    }
}

/// The right-side twin of a left internal name, or `None` when the name isn't on
/// the left. The side is the last `l`/`r` token after the first, as `naming` reads it.
fn right_twin(internal: &str) -> Option<String> {
    let mut tokens: Vec<&str> = internal.split('_').collect();
    let at = tokens.iter().rposition(|t| *t == "l" || *t == "r")?;
    if at == 0 || tokens[at] != "l" {
        return None;
    }
    tokens[at] = "r";
    Some(tokens.join("_"))
}

pub fn ctrl_name(bone: &str) -> String {
    format!("{bone}_ctrl")
}

pub fn fk_name(bone: &str) -> String {
    format!("{bone}_fk_ctrl")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn control(shape: Shape, axis: Vec3) -> Control {
        Control {
            name: "test_ctrl".into(),
            kind: ControlKind::Fk,
            parent: ControlParent::None,
            bone: Some("hand_l".into()),
            position: Vec3::new(0.5, 1.2, 0.1),
            axis,
            shape,
            size: 0.05,
            color: [0.1, 0.4, 1.0],
            spaces: Vec::new(),
            component: String::new(),
        }
    }

    #[test]
    fn every_shape_is_unit_sized_lines() {
        for shape in Shape::ALL {
            let strokes = shape.strokes();
            assert!(!strokes.is_empty(), "{shape:?}");
            let mut reach = 0.0f32;
            for line in &strokes {
                assert!(line.len() >= 2, "{shape:?}");
                for p in line {
                    assert!(p.abs().max_element() <= 1.0 + 1e-5, "{shape:?} {p}");
                    reach = reach.max(p.length());
                }
            }
            // Each shape fills its size, so a style's size means the same for all.
            assert!(reach > 0.99, "{shape:?} reaches {reach}");
        }
    }

    #[test]
    fn shapes_read_as_themselves() {
        let points = |s: Shape| s.strokes().concat();
        for s in [
            Shape::Arrow,
            Shape::DoubleArrow,
            Shape::FourArrows,
            Shape::Star,
        ] {
            assert!(points(s).iter().all(|p| p.y == 0.0), "{s:?} is flat");
        }
        // Three rings, one per plane.
        assert_eq!(Shape::Sphere.strokes().len(), 3);
        // The arrow points up its plane, with a single tip.
        let arrow = points(Shape::Arrow);
        assert_eq!(arrow.iter().filter(|p| p.z == 1.0).count(), 1);
        assert!(arrow.iter().all(|p| p.z >= -1.0));
        // The double arrow has a tip at each end.
        let double = points(Shape::DoubleArrow);
        assert!(double.contains(&Vec3::Z) && double.contains(&-Vec3::Z));
        // Four arrowheads, one on each side.
        let fa = points(Shape::FourArrows);
        for dir in [Vec3::X, -Vec3::X, Vec3::Z, -Vec3::Z] {
            assert!(fa.iter().any(|p| (*p - dir).length() < 1e-5), "{dir}");
        }
        // Polygons have the right number of corners.
        for (s, corners) in [
            (Shape::Triangle, 3),
            (Shape::Hexagon, 6),
            (Shape::Octagon, 8),
        ] {
            assert_eq!(s.strokes()[0].len(), corners + 1, "{s:?}");
        }
        // The pyramid's apex is out along the control's axis.
        assert!(points(Shape::Pyramid).contains(&Vec3::Y));
        // The half circle only covers the up side.
        assert!(points(Shape::HalfCircle).iter().all(|p| p.z >= -1e-6));
        // Closed outlines end where they start.
        for s in [
            Shape::Circle,
            Shape::Square,
            Shape::Arrow,
            Shape::Star,
            Shape::HalfCircle,
        ] {
            for line in s.strokes() {
                assert_eq!(line.first(), line.last(), "{s:?}");
            }
        }
    }

    #[test]
    fn shape_names_are_stable_snake_case() {
        for shape in Shape::ALL {
            let name = serde_json::to_string(&shape).unwrap();
            assert_eq!(serde_json::from_str::<Shape>(&name).unwrap(), shape);
        }
        // Names already in guides files keep working.
        for old in ["circle", "square", "cube", "diamond", "cross"] {
            serde_json::from_str::<Shape>(&format!("\"{old}\"")).unwrap();
        }
        assert_eq!(
            serde_json::to_string(&[Shape::DoubleArrow, Shape::FourArrows, Shape::HalfCircle])
                .unwrap(),
            r#"["double_arrow","four_arrows","half_circle"]"#
        );
        let mut labels: Vec<_> = Shape::ALL.iter().map(|s| s.label()).collect();
        labels.sort();
        labels.dedup();
        assert_eq!(labels.len(), Shape::ALL.len());
    }

    #[test]
    fn frame_faces_the_axis_with_up_kept_up() {
        for axis in [
            Vec3::X,
            -Vec3::X,
            Vec3::Y,
            -Vec3::Y,
            Vec3::Z,
            Vec3::new(0.3, -0.8, 0.2),
        ] {
            let [across, along, up] = shape_frame(axis);
            assert!((along - axis.normalize()).length() < 1e-6);
            for (a, b) in [(across, along), (along, up), (up, across)] {
                assert!(a.dot(b).abs() < 1e-5);
            }
            assert!((across.cross(along) - up).length() < 1e-5, "right-handed");
            if axis.y.abs() < 0.9 {
                assert!(up.y > 0.0, "up stays up for {axis}");
            } else {
                assert!(up.z > 0.99, "a vertical control's up points forward");
            }
        }
    }

    #[test]
    fn outline_is_written_and_ignored_on_read() {
        let c = control(Shape::Arrow, Vec3::X);
        let json = serde_json::to_value(&c).unwrap();
        let outline = json["outline"].as_array().unwrap();
        let expected = c.outline();
        assert_eq!(outline.len(), expected.len());
        for (written, line) in outline.iter().zip(&expected) {
            let written = written.as_array().unwrap();
            assert_eq!(written.len(), line.len());
            for (w, p) in written.iter().zip(line) {
                let w: [f32; 3] = serde_json::from_value(w.clone()).unwrap();
                assert!((Vec3::from(w) - *p).length() < 1e-4);
            }
        }
        // The arrow tip sits one size up from the control, in model space.
        let tip = c.position + Vec3::Y * c.size;
        assert!(expected[0].iter().any(|p| (*p - tip).length() < 1e-6));
        assert!(json.get("spaces").is_none(), "no spaces, no field");
        let back: Control = serde_json::from_value(json).unwrap();
        assert_eq!(
            (back.shape, back.size, back.position),
            (c.shape, c.size, c.position)
        );
        // The outline follows the shape, so it is never stale.
        let mut restyled = back;
        restyled.shape = Shape::Sphere;
        assert_eq!(restyled.outline().len(), 3);
    }

    #[test]
    fn finds_right_twins() {
        assert_eq!(
            right_twin("hand_l_ik_ctrl").as_deref(),
            Some("hand_r_ik_ctrl")
        );
        assert_eq!(
            right_twin("index_02_l_ctrl").as_deref(),
            Some("index_02_r_ctrl")
        );
        assert_eq!(right_twin("hand_r_ik_ctrl"), None);
        assert_eq!(right_twin("cog_ctrl"), None);
        assert_eq!(right_twin("l_ctrl"), None);
    }
}
