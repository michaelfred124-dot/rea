//! Animation controls: the DCC-independent description of the control rig.
//!
//! Every exporter turns the same list into native objects: custom-shape bones in
//! Blender, NURBS curves with offset groups in Maya, and Control Rig controls in
//! Unreal. Positions are model space (Y-up, metres). `components` builds it.

use glam::Vec3;
use serde::{Deserialize, Serialize, Serializer};

pub use crate::face_board::FaceBoard;

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
    /// A square with rounded corners.
    RoundedSquare,
    /// A triangle with rounded corners, pointing along the shape's up direction.
    RoundedTriangle,
    /// A quarter of a disc, opening toward the shape's up and across directions.
    QuarterCircle,
    /// A toothed ring.
    Gear,
    /// A three-quarter ring with an arrowhead: turn this way.
    CurvedArrow,
    /// A ring on a stick, standing up from the control.
    Lollipop,
    /// Two rings along the axis, joined by four lines.
    Cylinder,
    /// A plus sign outline with wide arms.
    Plus,
}

/// Line strips in a shape's own frame: `x` across, `y` along the control's axis
/// (the plane normal) and `z` up within the plane. A closed loop repeats its
/// first point at the end.
pub type Strokes = Vec<Vec<Vec3>>;

/// Segments in a full ring of the round shapes.
const RING_SEGMENTS: usize = 32;

impl Shape {
    pub const ALL: [Shape; 23] = [
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
        Shape::RoundedSquare,
        Shape::RoundedTriangle,
        Shape::QuarterCircle,
        Shape::Gear,
        Shape::CurvedArrow,
        Shape::Lollipop,
        Shape::Cylinder,
        Shape::Plus,
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
            Shape::RoundedSquare => "Rounded square",
            Shape::RoundedTriangle => "Rounded triangle",
            Shape::QuarterCircle => "Quarter circle",
            Shape::Gear => "Gear",
            Shape::CurvedArrow => "Curved arrow",
            Shape::Lollipop => "Lollipop",
            Shape::Cylinder => "Cylinder",
            Shape::Plus => "Plus",
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
            Shape::RoundedSquare => vec![rounded(
                &[(1.0, 1.0), (-1.0, 1.0), (-1.0, -1.0), (1.0, -1.0)],
                0.35,
            )],
            Shape::RoundedTriangle => {
                let corners: Vec<(f32, f32)> = (0..3)
                    .map(|i| {
                        let a = (90.0 + i as f32 * 120.0f32).to_radians();
                        (a.cos(), a.sin())
                    })
                    .collect();
                // Rounding pulls the tips in; grow it back to full size.
                let mut tri = rounded(&corners, 0.3);
                let reach = tri.iter().map(|p| p.length()).fold(0.0, f32::max);
                tri.iter_mut().for_each(|p| *p /= reach);
                vec![tri]
            }
            Shape::QuarterCircle => {
                // From the centre out along across, round to up, and back.
                let mut out = vec![Vec3::ZERO];
                out.extend(arc(Vec3::X, Vec3::Z, FRAC_PI_2, RING_SEGMENTS / 4));
                out.push(Vec3::ZERO);
                vec![out]
            }
            Shape::Gear => {
                // Eight teeth: each a flat top at full radius and a gap at 0.78.
                const TEETH: usize = 8;
                let step = TAU / TEETH as f32;
                let pts: Vec<(f32, f32)> = (0..TEETH)
                    .flat_map(|t| {
                        let a = t as f32 * step;
                        [(0.78, 0.0), (1.0, 0.12), (1.0, 0.38), (0.78, 0.5)]
                            .into_iter()
                            .map(move |(r, f)| {
                                let (s, c) = (a + f * step).sin_cos();
                                (r * c, r * s)
                            })
                    })
                    .collect();
                vec![flat(&pts)]
            }
            Shape::CurvedArrow => {
                // Three quarters of a ring from the bottom round through across,
                // up and the far side, ending in an arrowhead pointing on round.
                let r = 0.8;
                let (start, sweep) = (-FRAC_PI_2, 1.5 * PI);
                let round = |a: f32| Vec3::new(a.cos(), 0.0, a.sin());
                let body = (0..=24)
                    .map(|i| round(start + sweep * i as f32 / 24.0) * r)
                    .collect();
                let end = start + sweep;
                let tip = round(end) * r + round(end + FRAC_PI_2) * 0.45;
                let head = vec![round(end) * (r + 0.2), tip, round(end) * (r - 0.2)];
                vec![body, head]
            }
            Shape::Lollipop => {
                // A stick up to a ring that starts where the stick meets it.
                let centre = Vec3::Z * 0.75;
                let mut head = arc(-Vec3::Z * 0.25, Vec3::X * 0.25, TAU, RING_SEGMENTS);
                head[RING_SEGMENTS] = head[0];
                let head = head.into_iter().map(|p| p + centre).collect();
                vec![vec![Vec3::ZERO, Vec3::Z * 0.5], head]
            }
            Shape::Cylinder => {
                let r = 0.8;
                let ring_at = |y: f32| -> Vec<Vec3> {
                    ring(Vec3::X * r, Vec3::Z * r)
                        .into_iter()
                        .map(|p| p.with_y(y))
                        .collect()
                };
                let mut out = vec![ring_at(-0.6), ring_at(0.6)];
                out.extend(
                    [(r, 0.0), (0.0, r), (-r, 0.0), (0.0, -r)]
                        .iter()
                        .map(|&(x, z)| vec![Vec3::new(x, -0.6, z), Vec3::new(x, 0.6, z)]),
                );
                out
            }
            Shape::Plus => {
                const W: f32 = 0.3;
                vec![flat(&[
                    (W, W),
                    (W, 1.0),
                    (-W, 1.0),
                    (-W, W),
                    (-1.0, W),
                    (-1.0, -W),
                    (-W, -W),
                    (-W, -1.0),
                    (W, -1.0),
                    (W, -W),
                    (1.0, -W),
                    (1.0, W),
                ])]
            }
        }
    }
}

/// A closed loop round `corners` (in the plane, counter-clockwise) with each
/// corner rounded off by an arc of `radius`, keeping the loop inside the
/// corners' outline.
fn rounded(corners: &[(f32, f32)], radius: f32) -> Vec<Vec3> {
    const STEPS: usize = 6;
    let n = corners.len();
    let at = |i: usize| {
        let (x, z) = corners[i % n];
        Vec3::new(x, 0.0, z)
    };
    let mut out = Vec::new();
    for i in 0..n {
        let (prev, here, next) = (at(i + n - 1), at(i), at(i + 1));
        let a = (prev - here).normalize();
        let b = (next - here).normalize();
        // The arc touches both edges `cut` from the corner; its centre is on
        // the bisector.
        let half = a.angle_between(b) * 0.5;
        let cut = radius / half.tan();
        let centre = here + (a + b).normalize() * (radius / half.sin());
        let (from, to) = (here + a * cut, here + b * cut);
        let (u, v) = ((from - centre).normalize(), (to - centre).normalize());
        let sweep = u.angle_between(v);
        let w = (v - u * u.dot(v)).normalize();
        for k in 0..=STEPS {
            let t = sweep * k as f32 / STEPS as f32;
            out.push(centre + (u * t.cos() + w * t.sin()) * radius);
        }
    }
    out.push(out[0]);
    out
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
    /// Curve control of a spline chain (tentacle, spline spine): moving and
    /// turning it bends the curve the chain's joints follow.
    Spline,
    /// Bendy limbs: a mid-segment control whose offset arcs that segment's bend
    /// joints (see [`Bendy`]).
    Bend,
    /// Bendy limbs: the elbow or knee control where the two segments' curves meet.
    Pin,
    /// The face board's panel: moving it moves its sliders (see [`FaceBoard`]).
    Board,
    /// A face board slider: its move along its track poses the face.
    Slider,
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
    /// More lines drawn with the shape, fixed to the control (model space at
    /// rest): the face board's panel edge and slider tracks.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extra: Strokes,
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

    /// The control's lines at rest: its shape and any extra lines.
    pub fn rest_lines(&self) -> Strokes {
        let mut out = self.outline();
        out.extend(self.extra.iter().cloned());
        out
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
            #[serde(skip_serializing_if = "<[Vec<Vec3>]>::is_empty")]
            extra: &'a [Vec<Vec3>],
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
            extra: &self.extra,
            outline: self
                .rest_lines()
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
    /// Initial value of the `soft` attribute (0 to 1): how much of the reach is
    /// eased so the middle joint doesn't snap straight. 0 is a plain IK solve.
    #[serde(default)]
    pub default_soft: f32,
    /// The softened zone at `soft` = 1, as a fraction of the solved length
    /// (upper plus lower). See [`soft_reach`].
    #[serde(default = "soft_zone")]
    pub soft_zone: f32,
    /// Bendy limbs: bend controls and the joints they arc.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bendy: Option<Bendy>,
}

/// The fraction of the chain eased at `soft` = 1.
pub const SOFT_ZONE: f32 = 0.1;

fn soft_zone() -> f32 {
    SOFT_ZONE
}

/// Where a softened two-bone chain of length `length` reaches toward a goal
/// `distance` away (the ease every builder implements). With `soft` = 0 it is
/// `min(distance, length)`. Otherwise the last `soft * zone * length` of the reach
/// approaches full extension along an exponential, so the chain never quite
/// straightens and the middle joint doesn't pop. Stretch then blends from this
/// reach to the goal: the builders aim the solve at
/// `reach + stretch * (distance - reach)` and scale the bones by that over `reach`.
pub fn soft_reach(distance: f32, length: f32, soft: f32, zone: f32) -> f32 {
    let ease = soft.clamp(0.0, 1.0) * zone * length;
    if ease <= 0.0 {
        return distance.min(length);
    }
    let start = length - ease;
    if distance <= start {
        distance
    } else {
        start + ease * (1.0 - (-(distance - start) / ease).exp())
    }
}

/// A bendy limb: each segment's skin rides on evenly spaced bend joints that
/// follow a curve, so the limb can arc.
///
/// The upper curve runs from the upper bone's head to the pin control, the lower
/// one from the pin to the lower bone's tail. A point a fraction `t` along a
/// segment sits at `lerp(start, end, t) + 4t(1 - t) * offset`, where `offset` is
/// how far the segment's bend control has been moved from the curve's midpoint
/// (where it sits at rest). Bend joint `i` of `n` starts at `t = i / n` and aims
/// at the point at `(i + 1) / n`, so the ends stay put. The segment bones keep
/// driving the limb (IK, FK and stretch) but no longer deform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bendy {
    /// The elbow or knee control (`ControlKind::Pin`), following the lower bone.
    pub pin: String,
    pub upper: BendSegment,
    pub lower: BendSegment,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BendSegment {
    /// The segment bone (upper or lower).
    pub bone: String,
    /// Its bend control (`ControlKind::Bend`), at the segment's middle.
    pub control: String,
    /// The bend joints from the segment's start to its end.
    pub joints: Vec<String>,
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
    /// Initial value of the `spring` attribute on the IK control (0 to 1). At 0
    /// the hock keeps the angle the hock control gives it as the paw moves; at 1
    /// it swings with the thigh (a spring hock, see [`spring_hock`]), so moving
    /// the paw alone folds the whole leg. The hock control turns it further.
    #[serde(default)]
    pub default_spring: f32,
}

/// The pose a spring hock gives a three-segment leg: the motion every builder
/// implements at `spring` = 1.
///
/// The thigh and the hock turn together, by one rotation `Q`, so the angle
/// between them stays as it is at rest (the way a horse's stifle and hock flex
/// together). Then `hip + Q(thigh) + calf + Q(hock) = ankle`: the leg folds like
/// a two-bone chain from the hip to the ankle whose upper bone is the thigh plus
/// the hock (from the hip to `knee + ankle - hock` at rest) and whose lower bone
/// is the calf. The builders solve that chain with the limb's pole, give the hock
/// the upper bone's turn, and the leg's own two-bone IK then reaches the hock
/// joint exactly, with no loop between the two solves.
///
/// `rest` is the hip, knee, hock and ankle at rest and `rest_pole` the pole
/// control there; `hip`, `goal` and `pole` are posed. The middle joint keeps its
/// rest angle about the reach line from the plane through the pole, as the IK
/// solvers do. Returns the posed hip, knee, hock and ankle (the ankle stops
/// short of a goal out of reach), or `None` for a degenerate leg.
pub fn spring_hock(
    rest: [Vec3; 4],
    rest_pole: Vec3,
    hip: Vec3,
    goal: Vec3,
    pole: Vec3,
) -> Option<[Vec3; 4]> {
    let [hip0, knee0, hock0, ankle0] = rest;
    let thigh = knee0 - hip0;
    let shin = ankle0 - hock0;
    let mid0 = thigh + shin;
    let (upper, lower) = (mid0.length(), (ankle0 - hip0 - mid0).length());
    // The reach line and the plane through the pole: (along, toward the pole, normal).
    let frame = |root: Vec3, end: Vec3, pole: Vec3| -> Option<[Vec3; 3]> {
        let along = (end - root).try_normalize()?;
        let normal = along.cross(pole - root).try_normalize()?;
        Some([along, normal.cross(along), normal])
    };
    let [_, b0, n0] = frame(hip0, ankle0, rest_pole)?;
    let angle = mid0.dot(n0).atan2(mid0.dot(b0));
    let [a, b, n] = frame(hip, goal, pole)?;
    let reach = (goal - hip)
        .length()
        .clamp((upper - lower).abs() + 1e-6, upper + lower);
    let x = (reach * reach + upper * upper - lower * lower) / (2.0 * reach);
    let r = (upper * upper - x * x).max(0.0).sqrt();
    let mid = a * x + (b * angle.cos() + n * angle.sin()) * r;
    // Q turns the upper bone's frame (its direction and its side) from rest to now.
    let basis = |u: Vec3, side: Vec3| -> Option<glam::Mat3> {
        let u = u.try_normalize()?;
        let v = (side - u * u.dot(side)).try_normalize()?;
        Some(glam::Mat3::from_cols(u, v, u.cross(v)))
    };
    let q = basis(mid, n)? * basis(mid0, n0)?.transpose();
    let ankle = hip + a * reach;
    Some([hip, hip + q * thigh, ankle - q * shin, ankle])
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

/// A chain of joints that follows a smooth curve (a tentacle, or a spline spine).
///
/// The curve runs through `points` (the joints' heads) at rest. Each point
/// follows the curve controls by `weights`, the way a skinned curve follows its
/// joints: posed, a point moves by the weighted blend of its controls' moves
/// from rest. All joints but the last are fitted along the curve, stretching to
/// its length or keeping their own; the last joint starts where they end and
/// turns with the last curve control, like a chest or the tip of a tentacle.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SplineChain {
    /// The component id.
    pub name: String,
    /// The deform joints from the base to the tip.
    pub bones: Vec<String>,
    /// Curve controls from the start of the curve to its end. The first can
    /// also drive a bone (a spline spine's hips control drives the pelvis).
    pub controls: Vec<String>,
    /// The FK layer: `fk_controls[k]` carries curve control `k + 1` (and the
    /// FK controls after it), so turning one swings the rest of the chain.
    pub fk_controls: Vec<String>,
    /// The control that carries the `stretch` and `volume` attributes.
    pub settings: String,
    /// Curve points at rest: every joint's head.
    pub points: Vec<Vec3>,
    /// Each point's place along the chain, 0 at the base to 1 at the tip.
    pub params: Vec<f32>,
    /// Per point, the weight of each curve control (they sum to 1).
    pub weights: Vec<Vec<f32>>,
    /// Initial `stretch`: 1 = the joints stretch to the curve's length, 0 = they
    /// keep their own and slide along it.
    pub default_stretch: f32,
    /// Initial `volume`: 0 = no squash, 1 = the joints thin as they stretch and
    /// thicken as they squash, keeping their volume.
    pub default_volume: f32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ControlRig {
    pub controls: Vec<Control>,
    pub ik_chains: Vec<IkChain>,
    /// Curve-driven chains: tentacles and spline spines.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub splines: Vec<SplineChain>,
    /// Left/right control twins as `[left, right]`, for the animator tools' pose
    /// mirroring. Found from the internal names when the rig is built, so the DCC
    /// scripts never guess sides from the exported naming convention.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mirror_pairs: Vec<[String; 2]>,
    /// The face board: sliders beside the head posing the face, and the lip seal.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub face_board: Option<FaceBoard>,
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

    /// The spring hock keeps every segment's length and the thigh-to-hock angle,
    /// lands the ankle on a goal in reach and rests where the leg rests.
    #[test]
    fn spring_hock_folds_thigh_and_hock_together() {
        let g = crate::components::quadruped(0.8);
        let leg = g.component("hind_leg_l").unwrap();
        let p = |n: &str| leg.position(n).unwrap();
        let rest = [p("hip"), p("knee"), p("hock"), p("ankle")];
        let pole = p("knee") + Vec3::Z * 0.3;
        let lengths = |q: &[Vec3; 4]| [0, 1, 2].map(|i| q[i].distance(q[i + 1]));
        let angle = |q: &[Vec3; 4]| (q[1] - q[0]).angle_between(q[3] - q[2]);
        let at_rest = spring_hock(rest, pole, rest[0], rest[3], pole).unwrap();
        for (a, b) in at_rest.iter().zip(&rest) {
            assert!(a.distance(*b) < 1e-5, "{a} {b}");
        }
        for lift in [
            Vec3::new(0.0, 0.1, 0.0),
            Vec3::new(0.0, 0.15, 0.1),
            Vec3::new(0.02, 0.05, -0.08),
        ] {
            let goal = rest[3] + lift;
            let q = spring_hock(rest, pole, rest[0], goal, pole + lift * 0.5).unwrap();
            assert!(q[3].distance(goal) < 1e-5, "the ankle reaches {goal}");
            for (got, want) in lengths(&q).iter().zip(lengths(&rest)) {
                assert!((got - want).abs() < 1e-5, "{got} vs {want}");
            }
            assert!((angle(&q) - angle(&rest)).abs() < 1e-4);
            // Lifting the paw straight up folds the knee forward, toward the pole.
            if lift.z >= 0.0 {
                assert!(q[1].z > rest[1].z + 0.01, "{}", q[1]);
            }
        }
        // Out of reach the leg straightens toward the goal and stops short,
        // still keeping the thigh-to-hock angle.
        let far = rest[3] + (rest[3] - rest[0]) * 2.0;
        let q = spring_hock(rest, pole, rest[0], far, pole).unwrap();
        assert!(q[3].distance(rest[0]) < far.distance(rest[0]));
        assert!((angle(&q) - angle(&rest)).abs() < 1e-4);
    }

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
            extra: Vec::new(),
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
    fn the_newer_shapes_read_as_themselves() {
        let points = |s: Shape| s.strokes().concat();
        for s in [
            Shape::RoundedSquare,
            Shape::RoundedTriangle,
            Shape::QuarterCircle,
            Shape::Gear,
            Shape::CurvedArrow,
            Shape::Lollipop,
            Shape::Plus,
        ] {
            assert!(points(s).iter().all(|p| p.y == 0.0), "{s:?} is flat");
        }
        // Rounded shapes have no sharp corner: no point reaches the corner of
        // the square, and the outline still touches every side.
        let rs = points(Shape::RoundedSquare);
        assert!(rs.iter().all(|p| p.x.abs() + p.z.abs() < 1.9));
        for dir in [Vec3::X, -Vec3::X, Vec3::Z, -Vec3::Z] {
            assert!(rs.iter().any(|p| (p.dot(dir) - 1.0).abs() < 1e-4), "{dir}");
        }
        let rt = points(Shape::RoundedTriangle);
        let top = rt.iter().map(|p| p.z).fold(f32::MIN, f32::max);
        assert!((top - 1.0).abs() < 1e-4, "the rounded triangle points up");
        // The quarter circle stays in one quadrant and starts at its centre.
        let q = points(Shape::QuarterCircle);
        assert!(q.iter().all(|p| p.x >= -1e-6 && p.z >= -1e-6));
        assert_eq!(q[0], Vec3::ZERO);
        // Gear teeth: the outline goes in and out eight times.
        let gear = &Shape::Gear.strokes()[0];
        let outer = gear
            .windows(2)
            .filter(|w| w[0].length() < 0.9 && w[1].length() > 0.9);
        assert_eq!(outer.count(), 8);
        // The curved arrow is an open arc plus a head at its end.
        let ca = Shape::CurvedArrow.strokes();
        assert_eq!(ca.len(), 2);
        assert_ne!(ca[0].first(), ca[0].last());
        let end = *ca[0].last().unwrap();
        assert!(
            ca[1].iter().any(|p| (*p - end).length() > 0.3),
            "the head points on"
        );
        // The lollipop's stick meets its ring.
        let lp = Shape::Lollipop.strokes();
        assert!(lp[1].iter().any(|p| (*p - lp[0][1]).length() < 1e-5));
        // The cylinder has two rings across the axis and four lines along it.
        let cy = Shape::Cylinder.strokes();
        assert_eq!(cy.len(), 6);
        assert!(cy[0].iter().all(|p| p.y == cy[0][0].y) && cy[0][0].y == -cy[1][0].y);
        // The plus is one closed outline of twelve corners.
        assert_eq!(Shape::Plus.strokes()[0].len(), 13);
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
    fn soft_reach_eases_toward_full_extension() {
        let length = 0.8;
        // Off: the plain solve, straight once the goal is out of reach.
        for d in [0.2, 0.79, 0.8, 1.3] {
            assert_eq!(soft_reach(d, length, 0.0, SOFT_ZONE), d.min(length));
        }
        let soft = 0.5;
        let start = length * (1.0 - soft * SOFT_ZONE);
        let mut last = 0.0f32;
        for i in 0..=400 {
            let d = 0.5 + i as f32 * 0.0025;
            let r = soft_reach(d, length, soft, SOFT_ZONE);
            if d <= start {
                assert_eq!(r, d, "below the eased zone the goal is reached");
            } else {
                assert!(r < d && r <= length, "{d}: eased short of the goal");
            }
            assert!(r >= last, "the reach only grows as the goal moves out");
            last = r;
        }
        // Smooth where the ease begins: the slope is 1 on both sides.
        let slope = (soft_reach(start + 1e-4, length, soft, SOFT_ZONE) - start) / 1e-4;
        assert!((slope - 1.0).abs() < 0.01, "{slope}");
        // Far out it settles just short of the full length.
        assert!(length - soft_reach(10.0, length, soft, SOFT_ZONE) < 1e-4);
        // Soft is clamped to 0..1.
        assert_eq!(
            soft_reach(0.79, length, 3.0, SOFT_ZONE),
            soft_reach(0.79, length, 1.0, SOFT_ZONE)
        );
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
