//! Which way a character stands in its file, and the turn that stands it up in
//! hive-rig's frame (Y up, facing +Z, its left on +X).
//!
//! Files come Z-up, facing -Z or +X, or with a root node turned; the rest of
//! hive-rig (landmarks, guides, components, the builders) assumes its own frame.
//! [`detect`] reads the up axis and the facing from the body itself:
//!
//! * the **left-right axis** is the one the mesh (and the skeleton) is the
//!   mirror image of itself across;
//! * **up** is the end the body stands on: a cut just above the floor shows two
//!   (or four) feet in mirrored pairs, a cut a quarter of the way up two (or
//!   four) separate legs, and a cut at the top one head on the mid-line; leaf
//!   joints of the skeleton at the floor in mirrored pairs count the same way;
//! * the **facing** is where the toes point (the foot's sole reaches forward of
//!   the shin; a skeleton's toe joints lie forward of the ankle), or for an
//!   animal on four legs, the end its head is at.
//!
//! Only the 24 axis-aligned turns are considered: files are authored in some
//! axis convention, never at an arbitrary angle. When the cues don't clearly beat
//! the file's own axes, the file's own axes are kept.

use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

use anyhow::{Result, bail};
use glam::{Mat3, Vec2, Vec3};
use serde::{Deserialize, Serialize};

use crate::joints::Joint;
use crate::mesh::Mesh;

/// A signed axis of a file's frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Axis {
    #[serde(rename = "+x")]
    PosX,
    #[serde(rename = "-x")]
    NegX,
    #[serde(rename = "+y")]
    PosY,
    #[serde(rename = "-y")]
    NegY,
    #[serde(rename = "+z")]
    PosZ,
    #[serde(rename = "-z")]
    NegZ,
}

impl Axis {
    pub const ALL: [Axis; 6] = [
        Axis::PosX,
        Axis::NegX,
        Axis::PosY,
        Axis::NegY,
        Axis::PosZ,
        Axis::NegZ,
    ];

    pub fn vec(self) -> Vec3 {
        match self {
            Axis::PosX => Vec3::X,
            Axis::NegX => Vec3::NEG_X,
            Axis::PosY => Vec3::Y,
            Axis::NegY => Vec3::NEG_Y,
            Axis::PosZ => Vec3::Z,
            Axis::NegZ => Vec3::NEG_Z,
        }
    }

    /// 0, 1 or 2 for X, Y or Z.
    pub fn index(self) -> usize {
        match self {
            Axis::PosX | Axis::NegX => 0,
            Axis::PosY | Axis::NegY => 1,
            Axis::PosZ | Axis::NegZ => 2,
        }
    }

    pub fn negated(self) -> Axis {
        match self {
            Axis::PosX => Axis::NegX,
            Axis::NegX => Axis::PosX,
            Axis::PosY => Axis::NegY,
            Axis::NegY => Axis::PosY,
            Axis::PosZ => Axis::NegZ,
            Axis::NegZ => Axis::PosZ,
        }
    }

    /// The axis along `index` (0, 1, 2), positive or negative.
    pub fn from_index(index: usize, positive: bool) -> Axis {
        let a = [Axis::PosX, Axis::PosY, Axis::PosZ][index];
        if positive { a } else { a.negated() }
    }

    pub fn label(self) -> &'static str {
        match self {
            Axis::PosX => "+X",
            Axis::NegX => "-X",
            Axis::PosY => "+Y",
            Axis::NegY => "-Y",
            Axis::PosZ => "+Z",
            Axis::NegZ => "-Z",
        }
    }
}

impl fmt::Display for Axis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

impl FromStr for Axis {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Axis> {
        Ok(match s.trim().to_ascii_lowercase().as_str() {
            "+x" | "x" => Axis::PosX,
            "-x" => Axis::NegX,
            "+y" | "y" => Axis::PosY,
            "-y" => Axis::NegY,
            "+z" | "z" => Axis::PosZ,
            "-z" => Axis::NegZ,
            other => bail!("unknown axis '{other}' (use +x, -x, +y, -y, +z or -z)"),
        })
    }
}

/// Which way a character stands in its source file: the file's axis that points
/// up through the character and the one it faces along. hive-rig's own frame is
/// up +Y, facing +Z.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Orientation {
    pub up: Axis,
    pub forward: Axis,
}

impl Default for Orientation {
    fn default() -> Self {
        Orientation::STANDARD
    }
}

impl Orientation {
    /// hive-rig's frame (and glTF's convention): Y up, facing +Z.
    pub const STANDARD: Orientation = Orientation {
        up: Axis::PosY,
        forward: Axis::PosZ,
    };

    pub fn new(up: Axis, forward: Axis) -> Result<Orientation> {
        if up.index() == forward.index() {
            bail!("up ({up}) and forward ({forward}) must be different axes");
        }
        Ok(Orientation { up, forward })
    }

    pub fn is_standard(&self) -> bool {
        *self == Orientation::STANDARD
    }

    /// The turn from hive-rig's frame to the source file's: hive's +Y goes to
    /// `up`, +Z to `forward` and +X (the character's left) to `up × forward`.
    /// Always a rotation (never a mirror).
    pub fn to_source(&self) -> Mat3 {
        let (u, f) = (self.up.vec(), self.forward.vec());
        Mat3::from_cols(u.cross(f), u, f)
    }

    /// The turn from the source file's frame to hive-rig's.
    pub fn to_standard(&self) -> Mat3 {
        self.to_source().transpose()
    }

    /// Every valid orientation (24).
    pub fn all() -> impl Iterator<Item = Orientation> {
        Axis::ALL.into_iter().flat_map(|u| {
            Axis::ALL
                .into_iter()
                .filter(move |f| f.index() != u.index())
                .map(move |f| Orientation { up: u, forward: f })
        })
    }
}

impl fmt::Display for Orientation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "up {}, facing {}", self.up, self.forward)
    }
}

/// The kind of body, which picks the guide layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BodyKind {
    /// Two legs: the biped layout.
    #[default]
    Biped,
    /// Four legs: the dog layout.
    Quadruped,
}

impl BodyKind {
    /// The layout (template) it gets.
    pub fn template(self) -> &'static str {
        match self {
            BodyKind::Biped => "biped",
            BodyKind::Quadruped => "dog",
        }
    }
}

/// The kind of layout a set of guides is: "dog" (hind legs and front legs),
/// "biped" (legs and arms), or "creature" for anything else.
pub fn layout_name(g: &crate::components::Guides) -> &'static str {
    use crate::components::Kind;
    let has = |k: Kind| g.components.iter().any(|c| c.kind == k);
    if has(Kind::QuadLeg) && g.components.iter().any(|c| c.id.starts_with("front_leg")) {
        "dog"
    } else if has(Kind::Leg) && has(Kind::Arm) {
        "biped"
    } else {
        "creature"
    }
}

/// Advice when a layout doesn't suit the body it's on: biped guides on four
/// legs, or the dog's on two.
pub fn template_advice(g: &crate::components::Guides, body: BodyKind) -> Option<String> {
    match (layout_name(g), body) {
        ("biped", BodyKind::Quadruped) => Some(
            "the body stands on four legs but the guides are a biped's; load the dog template (Templates > dog, or `hive-rig template apply <guides> dog`)".into(),
        ),
        ("dog", BodyKind::Biped) => Some(
            "the body stands on two legs but the guides are the dog's; load the biped template"
                .into(),
        ),
        _ => None,
    }
}

/// What the user asked for; unset parts are detected.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Choice {
    pub up: Option<Axis>,
    pub forward: Option<Axis>,
}

/// The orientation and body kind found for a character, and why.
#[derive(Debug, Clone, PartialEq)]
pub struct Detection {
    pub orientation: Orientation,
    pub body: BodyKind,
    /// Plain-language reasons, such as "feet in a mirrored pair at the bottom".
    pub cues: Vec<String>,
    /// Up and forward as asked for rather than detected (each one).
    pub up_given: bool,
    pub forward_given: bool,
    /// The cues didn't clearly decide; the file's own axes were kept where they
    /// didn't.
    pub unsure: bool,
}

impl Detection {
    /// One line for reports: "up -Z, facing +Y (turned to Y up, facing +Z): feet ...".
    pub fn summary(&self) -> String {
        let how = if self.up_given && self.forward_given {
            "as given".to_string()
        } else if self.unsure {
            "unsure, kept the file's axes where unclear".to_string()
        } else if self.cues.is_empty() {
            "detected".to_string()
        } else {
            format!("detected from {}", self.cues.join(", "))
        };
        format!("{} ({how})", self.orientation)
    }
}

/// Turn a mesh in place by `m` (a rotation): positions, normals, tangents and
/// blendshape offsets.
pub fn turn_mesh(mesh: &mut Mesh, m: Mat3) {
    for p in &mut mesh.positions {
        *p = m * *p;
    }
    for n in &mut mesh.normals {
        *n = m * *n;
    }
    for t in &mut mesh.tangents {
        let d = m * t.truncate();
        *t = d.extend(t.w);
    }
    for s in &mut mesh.shapes {
        for o in &mut s.offsets {
            *o = m * *o;
        }
        for o in &mut s.normal_offsets {
            *o = m * *o;
        }
        for o in s.inbetweens.iter_mut().flat_map(|b| &mut b.offsets) {
            *o = m * *o;
        }
        if let Some(d) = &mut s.driver {
            d.axis = m * d.axis;
        }
    }
    crate::readers::turn(&mut mesh.readers, m);
}

/// Turn joints in place by `m`.
pub fn turn_joints(joints: &mut [Joint], m: Mat3) {
    for j in joints {
        j.position = m * j.position;
    }
}

/// Find the orientation of a character (mesh and, if any, skeleton, both in the
/// file's frame) and its body kind, honouring the parts of `choice` that are set.
pub fn detect(mesh: &Mesh, joints: &[Joint], choice: Choice) -> Detection {
    let frame = Frame::new(mesh, joints);
    let mut cues = Vec::new();
    let mut unsure = false;

    // Up: given, or the best-scoring signed axis.
    let up = match choice.up {
        Some(u) => u,
        None => {
            let lateral_given = choice.forward.map(|f| f.index());
            let mut best: Option<(f32, Axis, Vec<String>)> = None;
            let mut standard_score = f32::NEG_INFINITY;
            for u in Axis::ALL {
                if Some(u.index()) == lateral_given {
                    continue;
                }
                let (score, why) = frame.up_score(u, choice.forward);
                if u == Axis::PosY {
                    standard_score = score;
                }
                if best.as_ref().is_none_or(|b| score > b.0) {
                    best = Some((score, u, why));
                }
            }
            let (score, u, why) = best.expect("six axes");
            if u != Axis::PosY && score < standard_score + 1.0 {
                // Not clearly better than the file's own up.
                unsure = true;
                Axis::PosY
            } else {
                cues.extend(why);
                u
            }
        }
    };

    let body = frame.body_kind(up);
    // Forward: given, or the facing cue along the axis left over by up and the
    // left-right axis.
    let forward = match choice.forward {
        Some(f) if f.index() != up.index() => f,
        _ => {
            let lateral = frame.lateral_axis(up);
            let fwd_index = 3 - up.index() - lateral;
            let (vote, why) = frame.facing_vote(up, fwd_index, body);
            let positive_default = default_forward(up, fwd_index, frame.lateral_clear(up));
            if vote.abs() < 0.5 {
                unsure = true;
                positive_default
            } else {
                cues.extend(why);
                Axis::from_index(fwd_index, vote > 0.0)
            }
        }
    };
    let orientation = Orientation { up, forward };
    if body == BodyKind::Quadruped {
        cues.push("four legs".into());
    }
    Detection {
        orientation,
        body,
        cues,
        up_given: choice.up.is_some(),
        forward_given: choice.forward.is_some(),
        unsure,
    }
}

/// The forward to keep along `index` when nothing says which way: the one the
/// standard frame would have (+Z for Y up), else positive.
/// The usual facing for a file: +Z (glTF), or -Y in a Z-up file (Blender's
/// front view). When the left-right axis is clear it fixes the forward axis,
/// and only the sign is the usual one.
fn default_forward(up: Axis, index: usize, lateral_clear: bool) -> Axis {
    let usual = if up.index() == 2 {
        Axis::NegY
    } else {
        Axis::PosZ
    };
    if !lateral_clear || usual.index() == index {
        usual
    } else {
        Axis::from_index(index, true)
    }
}

/// A cut through the mesh projected onto (left-right, forward-back) and gathered
/// into connected pieces.
#[derive(Debug, Clone)]
struct Piece {
    /// Length of outline in the piece (a weight).
    size: f32,
    center: Vec2,
    min: Vec2,
    max: Vec2,
}

/// The mesh and skeleton measured once.
struct Frame<'a> {
    mesh: &'a Mesh,
    joints: &'a [Joint],
    min: Vec3,
    max: Vec3,
    size: f32,
    /// Mirror symmetry error per axis (lower is more symmetric).
    mirror: [f32; 3],
    /// Median vertex position per axis.
    median: Vec3,
    children: Vec<usize>,
}

impl<'a> Frame<'a> {
    fn new(mesh: &'a Mesh, joints: &'a [Joint]) -> Frame<'a> {
        let (min, max) = mesh.bounds();
        let size = (max - min).max_element().max(1e-6);
        let median = Vec3::from_array([0, 1, 2].map(|a| median(&mesh.positions, a)));
        let mut mirror = [0.0; 3];
        for (a, m) in mirror.iter_mut().enumerate() {
            *m = mirror_error(&mesh.positions, a, median[a], size);
            if joints.len() >= 6 {
                let pts: Vec<Vec3> = joints.iter().map(|j| j.position).collect();
                *m += mirror_error(&pts, a, median[a], size);
            }
        }
        let mut children = vec![0; joints.len()];
        for j in joints {
            if let Some(p) = j.parent
                && p < children.len()
            {
                children[p] += 1;
            }
        }
        Frame {
            mesh,
            joints,
            min,
            max,
            size,
            mirror,
            median,
            children,
        }
    }

    /// Whether one of the two axes across `up` is clearly the more symmetric.
    fn lateral_clear(&self, up: Axis) -> bool {
        let o: Vec<f32> = (0..3)
            .filter(|&a| a != up.index())
            .map(|a| self.mirror[a])
            .collect();
        o[0].min(o[1]) < 0.7 * o[0].max(o[1])
    }

    /// The left-right axis given up: the more symmetric of the other two.
    fn lateral_axis(&self, up: Axis) -> usize {
        let others: Vec<usize> = (0..3).filter(|&a| a != up.index()).collect();
        if self.mirror[others[0]] <= self.mirror[others[1]] {
            others[0]
        } else {
            others[1]
        }
    }

    fn extent(&self, a: usize) -> f32 {
        (self.max - self.min)[a].max(1e-6)
    }

    /// Mid-plane coordinate along axis `a`: the median vertex, so one raised
    /// or longer arm doesn't move it the way it moves the bounds.
    fn mid(&self, a: usize) -> f32 {
        self.median[a]
    }

    /// The cut at height `t` (0 floor, 1 top) along `up`, as pieces in
    /// (left-right from the mid-plane, forward) coordinates.
    fn cut(&self, up: Axis, lateral: usize, t: f32) -> Vec<Piece> {
        let u = up.vec();
        let ua = up.index();
        let fa = 3 - ua - lateral;
        let lo = if u[ua] > 0.0 {
            self.min[ua]
        } else {
            -self.max[ua]
        };
        let level = lo + t * self.extent(ua);
        let mid = self.mid(lateral);
        let cell = 0.01 * self.size;
        let mut cells: HashMap<(i32, i32), f32> = HashMap::new();
        let pos = &self.mesh.positions;
        for tri in self.mesh.indices.as_chunks::<3>().0 {
            let p = [0, 1, 2].map(|k| pos[tri[k] as usize]);
            let h = p.map(|q| q.dot(u) - level);
            let mut hits: Vec<Vec2> = Vec::with_capacity(2);
            for (i, j) in [(0, 1), (1, 2), (2, 0)] {
                if (h[i] <= 0.0) != (h[j] <= 0.0) {
                    let s = h[i] / (h[i] - h[j]);
                    let q = p[i].lerp(p[j], s);
                    hits.push(Vec2::new(q[lateral] - mid, q[fa]));
                }
            }
            if hits.len() == 2 {
                let (a, b) = (hits[0], hits[1]);
                let len = a.distance(b);
                let steps = ((len / (0.5 * cell)).ceil() as usize).clamp(1, 64);
                for k in 0..=steps {
                    let q = a.lerp(b, k as f32 / steps as f32);
                    let key = ((q.x / cell).floor() as i32, (q.y / cell).floor() as i32);
                    *cells.entry(key).or_default() += len / (steps + 1) as f32;
                }
            }
        }
        pieces(&cells, cell)
    }

    /// How well `up` explains the body: feet (or paws) in mirrored pairs at the
    /// bottom, separate legs a quarter of the way up, a head at the top, leaf
    /// joints in pairs at the floor; a little for being the longer axis.
    fn up_score(&self, up: Axis, forward: Option<Axis>) -> (f32, Vec<String>) {
        let lateral = match forward {
            Some(f) => 3 - up.index() - f.index(),
            None => self.lateral_axis(up),
        };
        let mut why = Vec::new();
        let mut score = 0.0;
        let tol = 0.04 * self.size;
        let bottom = self.cut(up, lateral, 0.02);
        let (feet, centred) = pairs(&bottom, tol);
        score += 2.0 * feet.min(2) as f32 - centred.min(2) as f32;
        if feet > 0 && centred == 0 {
            why.push(if feet >= 2 {
                "four feet on the floor".to_string()
            } else {
                "feet on the floor".to_string()
            });
        }
        // Legs: low cuts show only mirrored pieces, nothing across the mid-line
        // (lying down, every cut also crosses the torso).
        let legs = self.leg_levels(up, lateral);
        score += 0.75 * legs as f32;
        if legs >= 4 {
            why.push("legs".into());
        }
        let top = self.cut(up, lateral, 0.96);
        let (top_pairs, top_mid) = pairs(&top, tol);
        if top_mid == 1 && top_pairs == 0 {
            score += 1.0;
            why.push("head at the top".into());
        }
        let skel = self.skeleton_legs(up, lateral);
        if skel > 0 {
            score += 2.0 * skel.min(2) as f32;
            why.push("the skeleton's legs".into());
        }
        score += 3.0 * self.extent(up.index()) / self.size;
        (score, why)
    }

    /// How many of the cuts from 5% to 30% of the way up show mirrored pieces
    /// and nothing across the mid-line (0 to 6).
    fn leg_levels(&self, up: Axis, lateral: usize) -> usize {
        let tol = 0.04 * self.size;
        [0.05, 0.1, 0.15, 0.2, 0.25, 0.3]
            .into_iter()
            .filter(|&t| {
                let (p, c) = pairs(&self.cut(up, lateral, t), tol);
                p >= 1 && c == 0
            })
            .count()
    }

    /// Legs of the skeleton along `up`: leaf joints near the floor in mirrored
    /// pairs whose chains meet well above the floor (a hip they hang from, not
    /// a root bone at the floor holding IK targets), counted as separate limbs
    /// when more than a fifth of the body apart front to back.
    fn skeleton_legs(&self, up: Axis, lateral: usize) -> usize {
        if self.joints.len() < 6 {
            return 0;
        }
        let u = up.vec();
        let ua = up.index();
        let fa = 3 - ua - lateral;
        let ext = self.extent(ua);
        let lo = self.low(up);
        let mid = self.mid(lateral);
        let tol = 0.04 * self.size;
        let leaves: Vec<usize> = (0..self.joints.len())
            .filter(|&i| {
                let p = self.joints[i].position;
                self.children[i] == 0
                    && p.dot(u) - lo < 0.12 * ext
                    && (p[lateral] - mid).abs() > 0.02 * self.size
            })
            .collect();
        let ancestors = |i: usize| {
            let mut v = vec![i];
            let mut at = self.joints[i].parent;
            while let Some(k) = at {
                if v.contains(&k) || v.len() > self.joints.len() {
                    break;
                }
                v.push(k);
                at = self.joints[k].parent;
            }
            v
        };
        let mut limbs: Vec<f32> = Vec::new();
        for &a in leaves
            .iter()
            .filter(|&&i| self.joints[i].position[lateral] > mid)
        {
            let pa = self.joints[a].position;
            for &b in leaves
                .iter()
                .filter(|&&i| self.joints[i].position[lateral] < mid)
            {
                let pb = self.joints[b].position;
                if ((pa[lateral] - mid) + (pb[lateral] - mid)).abs() > tol
                    || (pa[fa] - pb[fa]).abs() > tol
                {
                    continue;
                }
                let up_a = ancestors(a);
                let Some(&meet) = ancestors(b).iter().find(|k| up_a.contains(k)) else {
                    continue;
                };
                let drop = self.joints[meet].position.dot(u) - 0.5 * (pa.dot(u) + pb.dot(u));
                if drop > 0.25 * ext && drop > 0.15 * self.size {
                    limbs.push(0.5 * (pa[fa] + pb[fa]));
                }
            }
        }
        if limbs.is_empty() {
            return 0;
        }
        limbs.sort_by(f32::total_cmp);
        1 + limbs
            .windows(2)
            .filter(|w| w[1] - w[0] > 0.2 * self.extent(fa))
            .count()
    }

    /// The bottom of the body along `up`, as a coordinate along it.
    fn low(&self, up: Axis) -> f32 {
        let ua = up.index();
        if up.vec()[ua] > 0.0 {
            self.min[ua]
        } else {
            -self.max[ua]
        }
    }

    /// Four legs: two mirrored pairs of feet at the bottom and of legs a quarter
    /// of the way up, or two pairs of skeleton legs at the two ends of the body.
    fn body_kind(&self, up: Axis) -> BodyKind {
        let lateral = self.lateral_axis(up);
        let tol = 0.04 * self.size;
        // Pairs at the front and the back of the body, not two pieces of the
        // same leg.
        let apart = |t: f32| {
            let (f, _) = paired(&self.cut(up, lateral, t), tol);
            let spread = f.iter().fold(f32::NEG_INFINITY, |a, b| a.max(*b))
                - f.iter().fold(f32::INFINITY, |a, b| a.min(*b));
            f.len() >= 2 && spread > 0.25 * self.extent(3 - up.index() - lateral)
        };
        let skel = self.skeleton_legs(up, lateral);
        if skel >= 2 || (skel == 0 && apart(0.02) && apart(0.25)) {
            BodyKind::Quadruped
        } else {
            BodyKind::Biped
        }
    }

    /// Which way along axis `fa` the character faces: positive votes for +.
    fn facing_vote(&self, up: Axis, fa: usize, body: BodyKind) -> (f32, Vec<String>) {
        let lateral = 3 - up.index() - fa;
        let mut vote = 0.0;
        let mut why = Vec::new();
        let u = up.vec();
        let ua = up.index();
        let mesh_lo = if u[ua] > 0.0 {
            self.min[ua]
        } else {
            -self.max[ua]
        };
        let h = self.extent(ua);
        match body {
            BodyKind::Biped => {
                // Mesh: each foot's sole reaches forward of its shin.
                let soles = self.cut(up, lateral, 0.015);
                let shins = self.cut(up, lateral, 0.12);
                let mut mesh_vote = 0.0;
                for s in soles.iter().filter(|p| p.min.x > 0.0 || p.max.x < 0.0) {
                    if let Some(shin) = shins
                        .iter()
                        .filter(|p| (p.center.x > 0.0) == (s.center.x > 0.0))
                        .min_by(|a, b| {
                            (a.center.x - s.center.x)
                                .abs()
                                .total_cmp(&(b.center.x - s.center.x).abs())
                        })
                    {
                        mesh_vote += (s.center.y - shin.center.y) / (0.02 * h);
                    }
                }
                let mesh_vote = mesh_vote.clamp(-2.0, 2.0);
                if mesh_vote.abs() >= 0.5 {
                    why.push("the toes".to_string());
                }
                vote += mesh_vote;
                // Skeleton: toe joints forward of the ankle above them.
                let mut skel_vote = 0.0;
                for (i, j) in self.joints.iter().enumerate() {
                    if self.children[i] != 0 || j.position.dot(u) - mesh_lo > 0.08 * h {
                        continue;
                    }
                    let mut a = j.parent;
                    while let Some(k) = a {
                        if self.joints[k].position.dot(u) - mesh_lo > 0.05 * h {
                            break;
                        }
                        a = self.joints[k].parent;
                    }
                    if let Some(k) = a {
                        let d = j.position[fa] - self.joints[k].position[fa];
                        let up_d = self.joints[k].position.dot(u) - j.position.dot(u);
                        if up_d < 0.25 * h && d.abs() > 0.01 * h {
                            skel_vote += d.signum();
                        }
                    }
                }
                if skel_vote != 0.0 {
                    why.push("the toe joints".to_string());
                }
                vote += 1.5 * skel_vote.clamp(-2.0, 2.0);
            }
            BodyKind::Quadruped => {
                // The head end: the top fifth of the body leans that way.
                let top = self.max[ua].max(-self.min[ua]);
                let mut sum = 0.0;
                let mut n = 0.0;
                for p in &self.mesh.positions {
                    let hgt = p.dot(u) - mesh_lo;
                    if hgt > 0.8 * h && hgt <= top - mesh_lo + 1.0 {
                        sum += p[fa];
                        n += 1.0;
                    }
                }
                if n > 0.0 {
                    let lean = (sum / n - self.mid(fa)) / (0.05 * self.extent(fa));
                    vote += lean.clamp(-2.0, 2.0);
                    if lean.abs() >= 0.5 {
                        why.push("the head end".to_string());
                    }
                }
                // Skeleton: the highest joint's end.
                if let Some(j) = self
                    .joints
                    .iter()
                    .max_by(|a, b| a.position.dot(u).total_cmp(&b.position.dot(u)))
                {
                    let lean = (j.position[fa] - self.mid(fa)) / (0.05 * self.extent(fa));
                    vote += lean.clamp(-1.0, 1.0);
                }
            }
        }
        (vote, why)
    }
}

/// The median of the points' coordinate along axis `a`.
fn median(points: &[Vec3], a: usize) -> f32 {
    let mut v: Vec<f32> = points.iter().map(|p| p[a]).collect();
    if v.is_empty() {
        return 0.0;
    }
    let k = v.len() / 2;
    *v.select_nth_unstable_by(k, f32::total_cmp).1
}

/// Mean distance (over a sample, capped) from each point mirrored across the
/// plane at `mid` along axis `a` to the nearest point, as a fraction of `size`.
fn mirror_error(points: &[Vec3], a: usize, mid: f32, size: f32) -> f32 {
    if points.is_empty() {
        return 0.0;
    }
    let cell = 0.02 * size;
    let key = |p: Vec3| {
        (
            (p.x / cell).floor() as i32,
            (p.y / cell).floor() as i32,
            (p.z / cell).floor() as i32,
        )
    };
    let mut grid: HashMap<(i32, i32, i32), Vec<Vec3>> = HashMap::new();
    for p in points {
        grid.entry(key(*p)).or_default().push(*p);
    }
    let stride = (points.len() / 3000).max(1);
    let cap = 1.5 * cell;
    let mut total = 0.0;
    let mut n = 0.0;
    for p in points.iter().step_by(stride) {
        let mut q = *p;
        q[a] = 2.0 * mid - q[a];
        let (x, y, z) = key(q);
        let mut best = cap;
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if let Some(v) = grid.get(&(x + dx, y + dy, z + dz)) {
                        for r in v {
                            best = best.min(r.distance(q));
                        }
                    }
                }
            }
        }
        total += best;
        n += 1.0;
    }
    total / n / size
}

/// Connected pieces of occupied cells (8-neighbours), dropping specks under 3%
/// of the cut's outline.
fn pieces(cells: &HashMap<(i32, i32), f32>, cell: f32) -> Vec<Piece> {
    let total: f32 = cells.values().sum();
    let mut seen: HashMap<(i32, i32), bool> = HashMap::new();
    let mut out = Vec::new();
    let mut keys: Vec<&(i32, i32)> = cells.keys().collect();
    keys.sort_unstable();
    for &start in keys {
        if seen.contains_key(&start) {
            continue;
        }
        let mut stack = vec![start];
        seen.insert(start, true);
        let (mut size, mut sum) = (0.0, Vec2::ZERO);
        let (mut min, mut max) = (Vec2::splat(f32::INFINITY), Vec2::splat(f32::NEG_INFINITY));
        while let Some((x, y)) = stack.pop() {
            let w = cells[&(x, y)];
            let c = Vec2::new((x as f32 + 0.5) * cell, (y as f32 + 0.5) * cell);
            size += w;
            sum += c * w;
            min = min.min(c - 0.5 * cell);
            max = max.max(c + 0.5 * cell);
            for dx in -1..=1 {
                for dy in -1..=1 {
                    let k = (x + dx, y + dy);
                    if cells.contains_key(&k) && !seen.contains_key(&k) {
                        seen.insert(k, true);
                        stack.push(k);
                    }
                }
            }
        }
        if size >= 0.03 * total && size > 0.0 {
            out.push(Piece {
                size,
                center: sum / size,
                min,
                max,
            });
        }
    }
    out
}

/// Mirrored pairs (one piece each side of the mid-line, matching in distance
/// from it and in forward position within `tol`) and pieces across the mid-line.
fn pairs(pieces: &[Piece], tol: f32) -> (usize, usize) {
    let (found, centred) = paired(pieces, tol);
    (found.len(), centred)
}

/// The forward positions of the mirrored pairs [`pairs`] counts, and the count
/// of pieces across the mid-line.
fn paired(pieces: &[Piece], tol: f32) -> (Vec<f32>, usize) {
    let crosses = |p: &Piece| p.min.x < 0.0 && p.max.x > 0.0 && p.size > 0.0;
    let centred = pieces.iter().filter(|p| crosses(p)).count();
    let mut left: Vec<&Piece> = pieces
        .iter()
        .filter(|p| !crosses(p) && p.center.x > 0.0)
        .collect();
    let right: Vec<&Piece> = pieces
        .iter()
        .filter(|p| !crosses(p) && p.center.x < 0.0)
        .collect();
    let mut found = Vec::new();
    for r in right {
        if let Some(i) = left.iter().position(|l| {
            (l.center.x + r.center.x).abs() < tol && (l.center.y - r.center.y).abs() < tol
        }) {
            found.push(0.5 * (left[i].center.y + r.center.y));
            left.remove(i);
        }
    }
    (found, centred)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;

    fn turned(mesh: &Mesh, o: Orientation) -> Mesh {
        let mut m = mesh.clone();
        turn_mesh(&mut m, o.to_source());
        m
    }

    #[test]
    fn axes_and_turns() {
        for o in Orientation::all() {
            let m = o.to_source();
            assert!((m.determinant() - 1.0).abs() < 1e-6, "{o}");
            assert!((m * Vec3::Y).distance(o.up.vec()) < 1e-6);
            assert!((m * Vec3::Z).distance(o.forward.vec()) < 1e-6);
            assert!((o.to_standard() * o.up.vec()).distance(Vec3::Y) < 1e-6);
        }
        assert_eq!(Orientation::all().count(), 24);
        assert_eq!("-z".parse::<Axis>().unwrap(), Axis::NegZ);
        assert!(Orientation::new(Axis::PosY, Axis::NegY).is_err());
        let json = serde_json::to_string(&Orientation::STANDARD).unwrap();
        assert_eq!(json, r#"{"up":"+y","forward":"+z"}"#);
    }

    /// The demo biped and dog, stood every one of the 24 ways in a file, are
    /// found standing that way.
    #[test]
    fn finds_every_orientation_of_a_biped_and_a_dog() {
        for (mesh, body) in [
            (demo::humanoid(0.0), BodyKind::Biped),
            (demo::humanoid(45.0), BodyKind::Biped),
            (demo::demo_quadruped(), BodyKind::Quadruped),
        ] {
            for o in Orientation::all() {
                let m = turned(&mesh, o);
                let d = detect(&m, &[], Choice::default());
                assert_eq!(d.orientation, o, "{body:?} stood {o}: {d:?}");
                assert_eq!(d.body, body, "{o}");
            }
        }
    }

    #[test]
    fn choice_overrides_detection() {
        let m = turned(
            &demo::humanoid(0.0),
            Orientation::new(Axis::PosZ, Axis::NegY).unwrap(),
        );
        let d = detect(
            &m,
            &[],
            Choice {
                up: Some(Axis::PosZ),
                forward: None,
            },
        );
        assert_eq!(d.orientation.forward, Axis::NegY);
        assert!(d.up_given && !d.forward_given);
        let d = detect(
            &m,
            &[],
            Choice {
                up: Some(Axis::PosY),
                forward: Some(Axis::PosX),
            },
        );
        assert_eq!(
            d.orientation,
            Orientation::new(Axis::PosY, Axis::PosX).unwrap()
        );
    }

    #[test]
    fn a_shapeless_mesh_keeps_its_axes() {
        let m = demo::sphere(Vec3::ZERO, 1.0, 16, 10);
        let d = detect(&m, &[], Choice::default());
        assert_eq!(d.orientation, Orientation::STANDARD);
        assert!(d.unsure);
    }
}
