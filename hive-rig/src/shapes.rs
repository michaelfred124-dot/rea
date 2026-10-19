//! Blendshapes: per-vertex offsets added to the rest mesh before skinning, the
//! tools that make and edit them, and the pose readers that drive corrective
//! shapes from a joint's rotation.
//!
//! A shape stores only the vertices it moves (sparse), so a mouth shape on a
//! 50,000-vertex character stays small in the rig file. Offsets are in model
//! space at rest (metres), which is what glTF morph targets, FBX blend shapes,
//! Blender shape keys and Maya blendShape targets all hold.
//!
//! Poses here turn bones about their heads. hive-rig's joints carry no rest
//! rotation (every bone's frame at rest is the model frame), so a bone's local
//! rotation and a reader's axis are both in model axes.

use std::collections::HashMap;

use glam::{Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::rig::Rig;
use crate::skeleton::Skeleton;
use crate::skin::SkinWeights;

/// Offsets smaller than this (metres) are dropped as no change.
pub const EPSILON: f32 = 1e-7;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Shape {
    pub name: String,
    /// The vertices the shape moves, ascending.
    pub vertices: Vec<u32>,
    /// How far each of `vertices` moves at full weight (model space, metres).
    pub offsets: Vec<Vec3>,
    /// How each of `vertices`' normal changes, when the source file said.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub normal_offsets: Vec<Vec3>,
    /// The weight the shape sits at (0 off, 1 fully on), set with its slider.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub weight: f32,
    /// A corrective shape: a joint's rotation sets the weight instead.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver: Option<PoseReader>,
}

fn is_zero(v: &f32) -> bool {
    *v == 0.0
}

/// Reads how far a bone has turned about an axis and turns that into a shape
/// weight: 0 up to `start` degrees, rising evenly to 1 at `end` degrees and
/// staying there.
///
/// The turn is the bone's rotation relative to its parent, measured about
/// `axis` (model space at rest, right-handed: positive turns follow the curl of
/// the right hand's fingers with the thumb along `axis`). Rotation about other
/// axes is ignored, so an elbow reader doesn't react to the forearm twisting.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PoseReader {
    /// The bone whose rotation is read (its exported name).
    pub bone: String,
    pub axis: Vec3,
    pub start: f32,
    pub end: f32,
}

impl PoseReader {
    /// The shape weight for a bone turned `degrees` about the reader's axis.
    pub fn weight_at(&self, degrees: f32) -> f32 {
        let span = self.end - self.start;
        if span.abs() < 1e-6 {
            return if degrees >= self.end { 1.0 } else { 0.0 };
        }
        ((degrees - self.start) / span).clamp(0.0, 1.0)
    }

    /// The weight for a bone's local rotation.
    pub fn weight(&self, local: Quat) -> f32 {
        self.weight_at(turn_degrees(local, self.axis))
    }
}

/// How far `q` turns about `axis`, in degrees (-180..180): the angle of its
/// twist part about that axis. Unchanged when `q` is negated.
pub fn turn_degrees(q: Quat, axis: Vec3) -> f32 {
    let a = axis.normalize_or_zero();
    let s = q.x * a.x + q.y * a.y + q.z * a.z;
    let w = q.w;
    // The twist about `a` is (s, w) normalised; its angle is 2 atan2(s, w),
    // written so -q gives the same answer.
    (2.0 * w * s).atan2(w * w - s * s).to_degrees()
}

impl Shape {
    /// A shape from one offset per vertex, keeping the vertices that move.
    pub fn from_dense(name: &str, offsets: &[Vec3]) -> Shape {
        let mut vertices = Vec::new();
        let mut kept = Vec::new();
        for (v, d) in offsets.iter().enumerate() {
            if d.length_squared() > EPSILON * EPSILON {
                vertices.push(v as u32);
                kept.push(*d);
            }
        }
        Shape {
            name: name.to_string(),
            vertices,
            offsets: kept,
            normal_offsets: Vec::new(),
            weight: 0.0,
            driver: None,
        }
    }

    /// One offset per vertex of an `n`-vertex mesh.
    pub fn dense(&self, n: usize) -> Vec<Vec3> {
        let mut out = vec![Vec3::ZERO; n];
        for (&v, d) in self.vertices.iter().zip(&self.offsets) {
            if let Some(o) = out.get_mut(v as usize) {
                *o += *d;
            }
        }
        out
    }

    pub fn dense_normals(&self, n: usize) -> Vec<Vec3> {
        let mut out = vec![Vec3::ZERO; n];
        for (&v, d) in self.vertices.iter().zip(&self.normal_offsets) {
            if let Some(o) = out.get_mut(v as usize) {
                *o += *d;
            }
        }
        out
    }

    pub fn validate(&self, n: usize) -> anyhow::Result<()> {
        anyhow::ensure!(!self.name.trim().is_empty(), "a shape has no name");
        anyhow::ensure!(
            self.vertices.len() == self.offsets.len(),
            "shape '{}' needs one offset per vertex",
            self.name
        );
        anyhow::ensure!(
            self.normal_offsets.is_empty() || self.normal_offsets.len() == self.vertices.len(),
            "shape '{}' needs one normal offset per vertex",
            self.name
        );
        anyhow::ensure!(
            self.vertices.windows(2).all(|w| w[0] < w[1]),
            "shape '{}' lists its vertices out of order",
            self.name
        );
        anyhow::ensure!(
            self.vertices.last().is_none_or(|&v| (v as usize) < n),
            "shape '{}' moves a vertex the mesh doesn't have",
            self.name
        );
        anyhow::ensure!(
            self.offsets.iter().all(|d| d.is_finite()) && self.weight.is_finite(),
            "shape '{}' has non-finite values",
            self.name
        );
        if let Some(d) = &self.driver {
            anyhow::ensure!(
                d.axis.length_squared() > 1e-12 && d.start.is_finite() && d.end.is_finite(),
                "shape '{}' has a pose reader without an axis",
                self.name
            );
        }
        Ok(())
    }
}

/// `rest` with each shape added at its weight in `weights` (one per shape).
pub fn apply(rest: &[Vec3], shapes: &[Shape], weights: &[f32]) -> Vec<Vec3> {
    let mut out = rest.to_vec();
    for (s, &w) in shapes.iter().zip(weights) {
        if w == 0.0 {
            continue;
        }
        for (&v, d) in s.vertices.iter().zip(&s.offsets) {
            if let Some(p) = out.get_mut(v as usize) {
                *p += *d * w;
            }
        }
    }
    out
}

/// The sum of `shapes` as one shape (at full weight each).
pub fn combine(shapes: &[&Shape], name: &str) -> Shape {
    let mut sum: HashMap<u32, (Vec3, Vec3)> = HashMap::new();
    let mut normals = false;
    for s in shapes {
        normals |= !s.normal_offsets.is_empty();
        for (k, (&v, d)) in s.vertices.iter().zip(&s.offsets).enumerate() {
            let e = sum.entry(v).or_default();
            e.0 += *d;
            e.1 += s.normal_offsets.get(k).copied().unwrap_or(Vec3::ZERO);
        }
    }
    let mut keys: Vec<u32> = sum.keys().copied().collect();
    keys.sort_unstable();
    keys.retain(|v| sum[v].0.length_squared() > EPSILON * EPSILON || sum[v].1 != Vec3::ZERO);
    Shape {
        name: name.to_string(),
        offsets: keys.iter().map(|v| sum[v].0).collect(),
        normal_offsets: if normals {
            keys.iter().map(|v| sum[v].1).collect()
        } else {
            Vec::new()
        },
        vertices: keys,
        weight: 0.0,
        driver: None,
    }
}

/// For each vertex, the vertex nearest its mirror image across `x = center_x`
/// (within `tolerance`), or none.
pub fn mirror_map(positions: &[Vec3], center_x: f32, tolerance: f32) -> Vec<Option<u32>> {
    let cell = tolerance.max(1e-6);
    let key = |p: Vec3| {
        [
            (p.x / cell).floor() as i64,
            (p.y / cell).floor() as i64,
            (p.z / cell).floor() as i64,
        ]
    };
    let mut grid: HashMap<[i64; 3], Vec<u32>> = HashMap::new();
    for (i, p) in positions.iter().enumerate() {
        grid.entry(key(*p)).or_default().push(i as u32);
    }
    positions
        .iter()
        .map(|p| {
            let m = Vec3::new(2.0 * center_x - p.x, p.y, p.z);
            let k = key(m);
            let mut best: Option<(f32, u32)> = None;
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        let Some(list) = grid.get(&[k[0] + dx, k[1] + dy, k[2] + dz]) else {
                            continue;
                        };
                        for &j in list {
                            let d = positions[j as usize].distance(m);
                            if d <= tolerance && best.is_none_or(|b| d < b.0) {
                                best = Some((d, j));
                            }
                        }
                    }
                }
            }
            best.map(|b| b.1)
        })
        .collect()
}

/// The mirror image of `shape` across the body's centre line: each vertex
/// takes the offset of its twin on the other side with X flipped, so a shape
/// that lifts the left brow becomes one that lifts the right brow. Vertices
/// without a twin don't move. The driver, if any, is mirrored by the caller.
pub fn mirror(
    shape: &Shape,
    positions: &[Vec3],
    center_x: f32,
    tolerance: f32,
    name: &str,
) -> Shape {
    let n = positions.len();
    let map = mirror_map(positions, center_x, tolerance);
    let src = shape.dense(n);
    let src_n = shape.dense_normals(n);
    let flip = |d: Vec3| Vec3::new(-d.x, d.y, d.z);
    let dense: Vec<Vec3> = map
        .iter()
        .map(|m| m.map_or(Vec3::ZERO, |j| flip(src[j as usize])))
        .collect();
    let mut out = Shape::from_dense(name, &dense);
    if !shape.normal_offsets.is_empty() {
        out.normal_offsets = out
            .vertices
            .iter()
            .map(|&v| map[v as usize].map_or(Vec3::ZERO, |j| flip(src_n[j as usize])))
            .collect();
    }
    out.weight = shape.weight;
    out
}

/// Split `shape` into a left half (+X) and a right half, blended over a band
/// `width` wide across the centre line so the halves add up to the whole.
pub fn split(
    shape: &Shape,
    positions: &[Vec3],
    center_x: f32,
    width: f32,
    names: [&str; 2],
) -> [Shape; 2] {
    let left_share = |x: f32| {
        if width <= 0.0 {
            return if x > center_x {
                1.0
            } else if x < center_x {
                0.0
            } else {
                0.5
            };
        }
        let t = ((x - center_x) / width + 0.5).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let half = |left: bool, name: &str| {
        let mut out = Shape {
            name: name.to_string(),
            vertices: Vec::new(),
            offsets: Vec::new(),
            normal_offsets: Vec::new(),
            weight: shape.weight,
            driver: None,
        };
        for (k, (&v, d)) in shape.vertices.iter().zip(&shape.offsets).enumerate() {
            let s = left_share(positions[v as usize].x);
            let s = if left { s } else { 1.0 - s };
            if s <= 0.0 {
                continue;
            }
            out.vertices.push(v);
            out.offsets.push(*d * s);
            if let Some(nd) = shape.normal_offsets.get(k) {
                out.normal_offsets.push(*nd * s);
            }
        }
        out
    };
    [half(true, names[0]), half(false, names[1])]
}

/// The name of a shape or bone on the other side: `_l`/`_r`, `_L`/`_R`,
/// `L_`/`R_`, `Left`/`Right`, `left`/`right` swapped. None when it has no side.
pub fn other_side(name: &str) -> Option<String> {
    let swaps: [(&str, &str); 6] = [
        ("Left", "Right"),
        ("left", "right"),
        ("_l", "_r"),
        ("_L", "_R"),
        (".l", ".r"),
        (".L", ".R"),
    ];
    for (a, b) in swaps {
        for (from, to) in [(a, b), (b, a)] {
            // A side token at the end, or followed by an underscore.
            if let Some(stem) = name.strip_suffix(from) {
                return Some(format!("{stem}{to}"));
            }
            let mid = format!("{from}_");
            if let Some(i) = name.find(&mid) {
                return Some(format!("{}{to}_{}", &name[..i], &name[i + mid.len()..]));
            }
        }
    }
    for (from, to) in [("L_", "R_"), ("R_", "L_"), ("l_", "r_"), ("r_", "l_")] {
        if let Some(rest) = name.strip_prefix(from) {
            return Some(format!("{to}{rest}"));
        }
    }
    None
}

/// The mirror image of a rotation axis across the YZ plane. Axes are
/// pseudo-vectors, so they flip the other way from positions: (x, y, z) to
/// (x, -y, -z).
pub fn mirror_axis(axis: Vec3) -> Vec3 {
    Vec3::new(axis.x, -axis.y, -axis.z)
}

// --------------------------------------------------------------------------- poses

/// Each bone's model-space transform for local rotations `local` (one per bone,
/// relative to the parent; missing entries are no turn). Bones turn about their
/// heads, children follow.
pub fn bone_matrices(sk: &Skeleton, local: &[Quat]) -> Vec<Mat4> {
    let mut world: Vec<Mat4> = Vec::with_capacity(sk.bones.len());
    for (i, b) in sk.bones.iter().enumerate() {
        let r = local.get(i).copied().unwrap_or(Quat::IDENTITY);
        let own = Mat4::from_translation(sk.local_offset(i)) * Mat4::from_quat(r);
        // Bones are listed parents first.
        let m = match b.parent {
            Some(p) if p < world.len() => world[p] * own,
            _ => own,
        };
        world.push(m);
    }
    world
}

/// The skinning matrix of each bone: rest model space to posed model space.
pub fn skin_matrices(sk: &Skeleton, local: &[Quat]) -> Vec<Mat4> {
    bone_matrices(sk, local)
        .into_iter()
        .zip(&sk.bones)
        .map(|(m, b)| m * Mat4::from_translation(-b.head))
        .collect()
}

/// A vertex's blended skinning matrix.
fn blended(skin: &SkinWeights, mats: &[Mat4], v: usize) -> Mat4 {
    if v >= skin.vertex_count() {
        return Mat4::IDENTITY;
    }
    let mut m = Mat4::ZERO;
    let mut total = 0.0;
    for (j, w) in skin.influences(v) {
        if let Some(s) = mats.get(j) {
            m += *s * w;
            total += w;
        }
    }
    if total > 0.0 {
        m * (1.0 / total)
    } else {
        Mat4::IDENTITY
    }
}

/// Linear blend skinning of `positions` (rest, shapes already added).
pub fn deform(positions: &[Vec3], skin: &SkinWeights, mats: &[Mat4]) -> Vec<Vec3> {
    positions
        .iter()
        .enumerate()
        .map(|(v, p)| blended(skin, mats, v).transform_point3(*p))
        .collect()
}

/// The rest-space offsets that move each listed vertex from where the pose puts
/// it to `targets` (posed positions), so the shape, added before skinning, lands
/// the vertex there in that pose: the skinning is undone per vertex.
/// `base` is the rest mesh with the shapes already on.
pub fn corrective_offsets(
    base: &[Vec3],
    skin: &SkinWeights,
    mats: &[Mat4],
    targets: &[(usize, Vec3)],
) -> Vec<(usize, Vec3)> {
    targets
        .iter()
        .filter_map(|&(v, target)| {
            let m = blended(skin, mats, v);
            if m.determinant().abs() < 1e-9 {
                return None;
            }
            Some((v, m.inverse().transform_point3(target) - base[v]))
        })
        .collect()
}

/// Every shape's weight in a pose: driven shapes from their readers, the rest
/// at their own weight. `bone_index` finds a reader's bone.
pub fn weights_in_pose(
    shapes: &[Shape],
    local: &[Quat],
    bone_index: impl Fn(&str) -> Option<usize>,
) -> Vec<f32> {
    shapes
        .iter()
        .map(|s| match &s.driver {
            Some(d) => bone_index(&d.bone)
                .map(|i| d.weight(local.get(i).copied().unwrap_or(Quat::IDENTITY)))
                .unwrap_or(0.0),
            None => s.weight,
        })
        .collect()
}

// --------------------------------------------------------------------------- on a rig

/// A test pose of one bone: turned `degrees` about `axis` (model space).
#[derive(Debug, Clone, PartialEq)]
pub struct BonePose {
    pub bone: usize,
    pub axis: Vec3,
    pub degrees: f32,
}

impl BonePose {
    /// Local rotations for every bone of `rig` (this bone turned, the rest at rest).
    pub fn locals(&self, rig: &Rig) -> Vec<Quat> {
        let mut out = vec![Quat::IDENTITY; rig.skeleton.bones.len()];
        if let Some(q) = out.get_mut(self.bone) {
            *q = Quat::from_axis_angle(self.axis.normalize_or_zero(), self.degrees.to_radians());
        }
        out
    }
}

/// A direction word (`up`, `down`, `forward`, `back`, `left`, `right`: the
/// character's own, left being +X) or `x,y,z`.
pub fn parse_direction(text: &str) -> anyhow::Result<Vec3> {
    let v = match text.trim().to_ascii_lowercase().as_str() {
        "up" => Vec3::Y,
        "down" => -Vec3::Y,
        "forward" | "front" => Vec3::Z,
        "back" | "backward" => -Vec3::Z,
        "left" => Vec3::X,
        "right" => -Vec3::X,
        other => {
            let n: Vec<f32> = other
                .split(',')
                .map(|x| x.trim().parse::<f32>())
                .collect::<Result<_, _>>()
                .map_err(|_| {
                    anyhow::anyhow!(
                        "'{text}' isn't a direction (up, down, forward, back, left, right or x,y,z)"
                    )
                })?;
            anyhow::ensure!(n.len() == 3, "'{text}' needs three numbers");
            Vec3::new(n[0], n[1], n[2])
        }
    };
    v.try_normalize()
        .ok_or_else(|| anyhow::anyhow!("'{text}' has no direction"))
}

/// The axis a bone bends about. With `toward`, the bone's tip swings toward
/// that direction. Without it, a limb's middle bone (forearm, shin) bends away
/// from its pole, the way its IK bends it.
pub fn bend_axis(rig: &Rig, bone: usize, toward: Option<Vec3>) -> anyhow::Result<Vec3> {
    let sk = &rig.skeleton;
    let b = &sk.bones[bone];
    let along = b.direction();
    if let Some(t) = toward {
        return along.cross(t).try_normalize().ok_or_else(|| {
            anyhow::anyhow!(
                "{} points along that direction already; pick another",
                b.name
            )
        });
    }
    let chain = rig
        .controls
        .ik_chains
        .iter()
        .find(|c| c.lower == b.name)
        .ok_or_else(|| {
            anyhow::anyhow!(
                "{} isn't the middle bone of a limb; say which way it bends (--toward up, --axis 0,0,1)",
                b.name
            )
        })?;
    let pole = rig
        .controls
        .controls
        .iter()
        .find(|c| c.name == chain.pole_control)
        .map(|c| c.position)
        .ok_or_else(|| anyhow::anyhow!("{} has no pole control", chain.name))?;
    let to_pole = pole - b.head;
    let to_pole = to_pole - along * to_pole.dot(along);
    to_pole
        .cross(along)
        .try_normalize()
        .ok_or_else(|| anyhow::anyhow!("{}'s pole lies along the bone", chain.name))
}

/// Read `BONE=DEGREES` or `BONE=DEGREES@DIRECTION` (the direction the bone's
/// tip swings toward; a limb's middle bone bends the way its IK does without one).
pub fn parse_pose(rig: &Rig, text: &str) -> anyhow::Result<BonePose> {
    let (bone, rest) = text.split_once('=').ok_or_else(|| {
        anyhow::anyhow!("a pose is BONE=DEGREES[@DIRECTION], such as lowerarm_l=90")
    })?;
    let (deg, toward) = match rest.split_once('@') {
        Some((d, t)) => (d, Some(parse_direction(t)?)),
        None => (rest, None),
    };
    let index = rig
        .skeleton
        .index_of(bone.trim())
        .ok_or_else(|| anyhow::anyhow!("the rig has no bone '{bone}'"))?;
    let degrees: f32 = deg
        .trim()
        .parse()
        .map_err(|_| anyhow::anyhow!("'{deg}' isn't a number of degrees"))?;
    Ok(BonePose {
        bone: index,
        axis: bend_axis(rig, index, toward)?,
        degrees,
    })
}

/// Where the skin puts each vertex in a pose, with the shapes at their
/// weights in that pose (corrective shapes from their readers).
pub fn posed_mesh(rig: &Rig, locals: &[Quat]) -> Vec<Vec3> {
    let weights = weights_in_pose(&rig.mesh.shapes, locals, |n| rig.skeleton.index_of(n));
    let base = apply(&rig.mesh.positions, &rig.mesh.shapes, &weights);
    deform(&base, &rig.skin, &skin_matrices(&rig.skeleton, locals))
}

/// Add `moves` (vertex, where it should be in the pose) to the shape `name`,
/// making it if needed: the offsets are worked back through the skin, so at
/// full weight in that pose each vertex lands where it was put. Other shapes
/// stay at their weights in the pose. Returns how many vertices moved.
pub fn sculpt(
    rig: &mut Rig,
    name: &str,
    locals: &[Quat],
    moves: &[(usize, Vec3)],
) -> anyhow::Result<usize> {
    anyhow::ensure!(!name.trim().is_empty(), "the shape needs a name");
    let shapes = &rig.mesh.shapes;
    let mine = shapes.iter().position(|s| s.name == name);
    let mut weights = weights_in_pose(shapes, locals, |n| rig.skeleton.index_of(n));
    if let Some(i) = mine {
        // This shape counts fully: the new offsets go on top of its own.
        weights[i] = 1.0;
    }
    let base = apply(&rig.mesh.positions, shapes, &weights);
    let mats = skin_matrices(&rig.skeleton, locals);
    let offsets = corrective_offsets(&base, &rig.skin, &mats, moves);
    let n = rig.mesh.vertex_count();
    let mut dense = vec![Vec3::ZERO; n];
    for (v, d) in &offsets {
        if *v < n {
            dense[*v] = *d;
        }
    }
    let added = Shape::from_dense(name, &dense);
    let moved = added.vertices.len();
    match mine {
        Some(i) => {
            let old = rig.mesh.shapes[i].clone();
            let mut merged = combine(&[&old, &added], name);
            merged.weight = old.weight;
            merged.driver = old.driver;
            rig.mesh.shapes[i] = merged;
        }
        None => rig.mesh.shapes.push(added),
    }
    Ok(moved)
}

/// The moves of a soft brush in a pose: every vertex within `radius` of `at`
/// (in the posed mesh) moves by `offset`, fading to nothing at the edge.
pub fn brush_moves(posed: &[Vec3], at: Vec3, radius: f32, offset: Vec3) -> Vec<(usize, Vec3)> {
    posed
        .iter()
        .enumerate()
        .filter_map(|(v, p)| {
            let d = p.distance(at);
            (d < radius).then(|| {
                let t = 1.0 - (d / radius).powi(2);
                (v, *p + offset * (t * t))
            })
        })
        .collect()
}

/// The shape of a rig by name.
pub fn find<'a>(rig: &'a Rig, name: &str) -> anyhow::Result<&'a Shape> {
    rig.mesh
        .shapes
        .iter()
        .find(|s| s.name == name)
        .ok_or_else(|| anyhow::anyhow!("the rig has no shape '{name}'"))
}

fn index(rig: &Rig, name: &str) -> anyhow::Result<usize> {
    rig.mesh
        .shapes
        .iter()
        .position(|s| s.name == name)
        .ok_or_else(|| anyhow::anyhow!("the rig has no shape '{name}'"))
}

fn ensure_new(rig: &Rig, name: &str) -> anyhow::Result<()> {
    anyhow::ensure!(!name.trim().is_empty(), "the shape needs a name");
    anyhow::ensure!(
        rig.mesh.shapes.iter().all(|s| s.name != name),
        "the rig already has a shape '{name}'"
    );
    Ok(())
}

/// Mirror `name` onto the other side as `new` (default: its name with the side
/// swapped). A corrective shape's reader moves to the twin bone, with its axis
/// mirrored, so the right elbow drives the right elbow's shape.
pub fn mirror_on_rig(rig: &mut Rig, name: &str, new: Option<&str>) -> anyhow::Result<String> {
    let src = find(rig, name)?.clone();
    let new = match new {
        Some(n) => n.to_string(),
        None => other_side(name).unwrap_or_else(|| format!("{name}_mirror")),
    };
    ensure_new(rig, &new)?;
    let body = &rig.guides.body;
    let tolerance = (body.height * 0.002).max(1e-5);
    let mut out = mirror(&src, &rig.mesh.positions, body.center_x, tolerance, &new);
    if let Some(d) = &src.driver {
        let sk = &rig.skeleton;
        let twins = crate::skin::bone_twins(sk, body.center_x, 0.05 * body.height);
        let bone = sk
            .index_of(&d.bone)
            .and_then(|i| twins.get(i))
            .map(|&t| sk.bones[t].name.clone())
            .or_else(|| other_side(&d.bone))
            .unwrap_or_else(|| d.bone.clone());
        out.driver = Some(PoseReader {
            bone,
            axis: mirror_axis(d.axis),
            start: d.start,
            end: d.end,
        });
    }
    rig.mesh.shapes.push(out);
    Ok(new)
}

/// Split `name` into left and right halves (default names: `<name>_l` and
/// `<name>_r`), blended over `width` metres across the centre line. The
/// original stays.
pub fn split_on_rig(
    rig: &mut Rig,
    name: &str,
    names: Option<[&str; 2]>,
    width: Option<f32>,
) -> anyhow::Result<[String; 2]> {
    let src = find(rig, name)?.clone();
    let names = names.map_or_else(
        || [format!("{name}_l"), format!("{name}_r")],
        |n| [n[0].to_string(), n[1].to_string()],
    );
    for n in &names {
        ensure_new(rig, n)?;
    }
    let body = &rig.guides.body;
    let width = width.unwrap_or(0.02 * body.height);
    let halves = split(
        &src,
        &rig.mesh.positions,
        body.center_x,
        width,
        [&names[0], &names[1]],
    );
    rig.mesh.shapes.extend(halves);
    Ok(names)
}

/// Add the shapes in `names` together as a new shape.
pub fn combine_on_rig(rig: &mut Rig, names: &[String], new: &str) -> anyhow::Result<()> {
    anyhow::ensure!(names.len() >= 2, "combine needs two shapes or more");
    ensure_new(rig, new)?;
    let parts: Vec<Shape> = names
        .iter()
        .map(|n| find(rig, n).cloned())
        .collect::<anyhow::Result<_>>()?;
    let refs: Vec<&Shape> = parts.iter().collect();
    rig.mesh.shapes.push(combine(&refs, new));
    Ok(())
}

pub fn set_weight(rig: &mut Rig, name: &str, weight: f32) -> anyhow::Result<()> {
    anyhow::ensure!(weight.is_finite(), "the weight must be a number");
    let i = index(rig, name)?;
    rig.mesh.shapes[i].weight = weight;
    Ok(())
}

/// Drive `name` with a pose reader (or stop driving it with `None`).
pub fn set_driver(rig: &mut Rig, name: &str, reader: Option<PoseReader>) -> anyhow::Result<()> {
    let i = index(rig, name)?;
    if let Some(r) = &reader {
        anyhow::ensure!(
            rig.skeleton.index_of(&r.bone).is_some(),
            "the rig has no bone '{}'",
            r.bone
        );
        anyhow::ensure!(r.axis.length_squared() > 1e-12, "the reader needs an axis");
        anyhow::ensure!(
            r.start.is_finite() && r.end.is_finite() && r.start != r.end,
            "the reader needs two different angles"
        );
    }
    rig.mesh.shapes[i].driver = reader;
    Ok(())
}

pub fn remove(rig: &mut Rig, name: &str) -> anyhow::Result<()> {
    let i = index(rig, name)?;
    rig.mesh.shapes.remove(i);
    Ok(())
}

pub fn rename(rig: &mut Rig, name: &str, new: &str) -> anyhow::Result<()> {
    let i = index(rig, name)?;
    ensure_new(rig, new)?;
    rig.mesh.shapes[i].name = new.to_string();
    Ok(())
}

/// Carry the pose readers of a rebuilt rig's shapes onto the new skeleton's
/// bone names (they change with the naming convention). Readers whose bone
/// is gone are dropped and named in the result.
pub fn carry_drivers(old: &Rig, new: &mut Rig) -> Vec<String> {
    let map = crate::rig::bone_map(old, new);
    let mut renamed: HashMap<String, String> = HashMap::new();
    for (to, from) in map.iter().enumerate() {
        if let Some(f) = from {
            renamed.insert(
                old.skeleton.bones[*f].name.clone(),
                new.skeleton.bones[to].name.clone(),
            );
        }
    }
    let mut lost = Vec::new();
    for s in &mut new.mesh.shapes {
        let Some(d) = &mut s.driver else { continue };
        match renamed.get(&d.bone) {
            Some(n) => d.bone = n.clone(),
            None if new.skeleton.index_of(&d.bone).is_some() => {}
            None => {
                lost.push(s.name.clone());
                s.driver = None;
            }
        }
    }
    lost
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turn_about_an_axis_ignores_other_turns() {
        let q = Quat::from_axis_angle(Vec3::Z, 60f32.to_radians());
        assert!((turn_degrees(q, Vec3::Z) - 60.0).abs() < 1e-3);
        assert!((turn_degrees(-q, Vec3::Z) - 60.0).abs() < 1e-3);
        assert!((turn_degrees(q, -Vec3::Z) + 60.0).abs() < 1e-3);
        // A twist about the bone (X) on top doesn't change the bend about Z.
        let twisted = q * Quat::from_axis_angle(Vec3::X, 30f32.to_radians());
        assert!((turn_degrees(twisted, Vec3::Z) - 60.0).abs() < 2.0);
        let r = PoseReader {
            bone: "b".into(),
            axis: Vec3::Z,
            start: 10.0,
            end: 90.0,
        };
        assert_eq!(r.weight(Quat::IDENTITY), 0.0);
        assert!((r.weight(q) - 0.625).abs() < 1e-3);
        assert_eq!(r.weight_at(120.0), 1.0);
    }

    #[test]
    fn a_sculpted_corrective_lands_where_it_was_put_and_rests_off() {
        let mut rig =
            crate::rig::auto_rig(crate::demo::demo_humanoid(), "demo", Default::default()).unwrap();
        let bone = rig.skeleton.index_of("lowerarm_l").unwrap();
        let axis = bend_axis(&rig, bone, None).unwrap();
        let locals = BonePose {
            bone,
            axis,
            degrees: 90.0,
        }
        .locals(&rig);
        let elbow = rig.skeleton.bones[bone].head;
        let posed = posed_mesh(&rig, &locals);
        // The rest vertex nearest the elbow, and where it is in the pose.
        let v = (0..rig.mesh.vertex_count())
            .min_by(|&a, &b| {
                let da = rig.mesh.positions[a].distance(elbow);
                da.total_cmp(&rig.mesh.positions[b].distance(elbow))
            })
            .unwrap();
        let moves = brush_moves(&posed, posed[v], 0.04, Vec3::new(0.0, 0.03, 0.0));
        assert!(moves.iter().any(|m| m.0 == v));
        assert!(sculpt(&mut rig, "elbow_fix", &locals, &moves).unwrap() > 0);
        set_driver(
            &mut rig,
            "elbow_fix",
            Some(PoseReader {
                bone: "lowerarm_l".into(),
                axis,
                start: 0.0,
                end: 90.0,
            }),
        )
        .unwrap();
        let fixed = posed_mesh(&rig, &locals);
        for (w, to) in &moves {
            assert!(fixed[*w].distance(*to) < 1e-4, "vertex {w}");
        }
        // At rest the driver is off and the mesh is untouched.
        let rest = posed_mesh(&rig, &vec![Quat::IDENTITY; rig.skeleton.bones.len()]);
        for (a, b) in rest.iter().zip(&rig.mesh.positions) {
            assert!(a.distance(*b) < 1e-5);
        }
        // Mirroring drives the twin from the other arm.
        let twin = mirror_on_rig(&mut rig, "elbow_fix", Some("elbow_fix_r")).unwrap();
        let d = find(&rig, &twin).unwrap().driver.clone().unwrap();
        assert_eq!(d.bone, "lowerarm_r");
        // A rebuild from the same guides keeps the drivers on their bones.
        let mut again = crate::rig::build_rig(
            rig.mesh.clone(),
            "demo",
            rig.guides.clone(),
            Default::default(),
        )
        .unwrap();
        assert!(carry_drivers(&rig, &mut again).is_empty());
        assert_eq!(
            find(&again, "elbow_fix")
                .unwrap()
                .driver
                .as_ref()
                .unwrap()
                .bone,
            "lowerarm_l"
        );
    }

    #[test]
    fn mirror_split_and_combine() {
        let positions = vec![
            Vec3::new(0.5, 1.0, 0.0),
            Vec3::new(-0.5, 1.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        ];
        let s = Shape::from_dense(
            "smile_l",
            &[
                Vec3::new(0.1, 0.2, 0.0),
                Vec3::ZERO,
                Vec3::new(0.0, 0.1, 0.0),
            ],
        );
        let m = mirror(&s, &positions, 0.0, 0.01, "smile_r");
        assert_eq!(m.vertices, vec![1, 2]);
        assert_eq!(m.offsets[0], Vec3::new(-0.1, 0.2, 0.0));
        let [l, r] = split(&s, &positions, 0.0, 0.2, ["a_l", "a_r"]);
        assert_eq!(l.vertices, vec![0, 2]);
        assert_eq!(r.vertices, vec![2]);
        let back = combine(&[&l, &r], "a");
        for (v, d) in s.dense(3).iter().zip(back.dense(3)) {
            assert!((*v - d).length() < 1e-6);
        }
        assert_eq!(other_side("smile_l").as_deref(), Some("smile_r"));
        assert_eq!(other_side("L_brow_up").as_deref(), Some("R_brow_up"));
        assert_eq!(other_side("upperarm_r").as_deref(), Some("upperarm_l"));
        assert_eq!(other_side("cheekLeft").as_deref(), Some("cheekRight"));
        assert_eq!(other_side("jaw_open"), None);
    }
}
