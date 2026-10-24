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
    /// In-between targets: where the vertices are at a weight between 0 and 1
    /// (ascending by weight). The shape moves through them in order: from rest
    /// to the first, from one to the next, and from the last to the full
    /// offsets at 1, blending linearly between neighbours (as Maya's
    /// blendShape in-betweens do).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inbetweens: Vec<Inbetween>,
}

/// One in-between target of a shape: the full offsets of its vertices at
/// `weight` (not added to the shape's own offsets).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Inbetween {
    /// The shape weight at which the vertices are exactly here (0 < weight < 1).
    pub weight: f32,
    /// The vertices it moves, ascending.
    pub vertices: Vec<u32>,
    /// Where each of `vertices` is at `weight`, as an offset from rest (metres).
    pub offsets: Vec<Vec3>,
}

impl Inbetween {
    /// The in-between from one offset per vertex, keeping the vertices that move.
    pub fn from_dense(weight: f32, offsets: &[Vec3]) -> Inbetween {
        let s = Shape::from_dense("", offsets);
        Inbetween {
            weight,
            vertices: s.vertices,
            offsets: s.offsets,
        }
    }

    pub fn dense(&self, n: usize) -> Vec<Vec3> {
        dense_of(&self.vertices, &self.offsets, n)
    }
}

fn dense_of(vertices: &[u32], offsets: &[Vec3], n: usize) -> Vec<Vec3> {
    let mut out = vec![Vec3::ZERO; n];
    for (&v, d) in vertices.iter().zip(offsets) {
        if let Some(o) = out.get_mut(v as usize) {
            *o += *d;
        }
    }
    out
}

/// How much of each target a weight `w` takes, for targets reached at
/// `points` (ascending, the last being 1): linear between the two targets
/// around `w`, rising from rest to the first, and carried on past either end
/// along the nearest segment (so with one target at 1 this is just `w`).
pub fn target_mix(points: &[f32], w: f32) -> Vec<f32> {
    let mut out = vec![0.0; points.len()];
    if points.is_empty() {
        return out;
    }
    if points.len() == 1 || w <= points[0] {
        out[0] = w / points[0].max(1e-6);
        return out;
    }
    // The segment [points[k], points[k + 1]] holding w (the last one past the end).
    let k = (0..points.len() - 1)
        .find(|&k| w <= points[k + 1])
        .unwrap_or(points.len() - 2);
    let span = (points[k + 1] - points[k]).max(1e-6);
    let t = (w - points[k]) / span;
    out[k] = 1.0 - t;
    out[k + 1] = t;
    out
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
            inbetweens: Vec::new(),
        }
    }

    /// One offset per vertex of an `n`-vertex mesh (at full weight).
    pub fn dense(&self, n: usize) -> Vec<Vec3> {
        dense_of(&self.vertices, &self.offsets, n)
    }

    /// The weights its targets are reached at: each in-between's, then 1.
    pub fn points(&self) -> Vec<f32> {
        self.inbetweens
            .iter()
            .map(|b| b.weight)
            .chain([1.0])
            .collect()
    }

    /// How much of each target (the in-betweens, then the full shape) the
    /// shape at weight `w` takes.
    pub fn mix(&self, w: f32) -> Vec<f32> {
        target_mix(&self.points(), w)
    }

    /// Add the shape at weight `w` to `out` (one position per vertex).
    pub fn add_to(&self, out: &mut [Vec3], w: f32) {
        if w == 0.0 {
            return;
        }
        let mix = self.mix(w);
        let targets = self
            .inbetweens
            .iter()
            .map(|b| (&b.vertices, &b.offsets))
            .chain([(&self.vertices, &self.offsets)]);
        for ((vertices, offsets), c) in targets.zip(mix) {
            if c == 0.0 {
                continue;
            }
            for (&v, d) in vertices.iter().zip(offsets) {
                if let Some(p) = out.get_mut(v as usize) {
                    *p += *d * c;
                }
            }
        }
    }

    /// One offset per vertex of an `n`-vertex mesh, at weight `w`.
    pub fn dense_at(&self, n: usize, w: f32) -> Vec<Vec3> {
        let mut out = vec![Vec3::ZERO; n];
        self.add_to(&mut out, w);
        out
    }

    /// The name an in-between goes by as a target of its own (glTF and Unreal
    /// morph targets, Blender shape keys): `<shape>_ib<percent>`.
    pub fn inbetween_name(&self, k: usize) -> String {
        inbetween_name(&self.name, self.inbetweens[k].weight)
    }

    /// The in-between reached at `weight` (to within half a percent).
    pub fn inbetween_at(&self, weight: f32) -> Option<usize> {
        self.inbetweens
            .iter()
            .position(|b| (b.weight - weight).abs() < 0.005)
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
        let mut last = 0.0f32;
        for b in &self.inbetweens {
            anyhow::ensure!(
                b.weight > 0.0 && b.weight < 1.0,
                "shape '{}' has an in-between at {}; they sit between 0 and 1",
                self.name,
                b.weight
            );
            anyhow::ensure!(
                b.weight - last >= 0.01 - 1e-5 && 1.0 - b.weight >= 0.01 - 1e-5,
                "shape '{}' has in-betweens out of order or closer than 0.01",
                self.name
            );
            last = b.weight;
            anyhow::ensure!(
                b.vertices.len() == b.offsets.len()
                    && b.vertices.windows(2).all(|w| w[0] < w[1])
                    && b.vertices.last().is_none_or(|&v| (v as usize) < n)
                    && b.offsets.iter().all(|d| d.is_finite()),
                "shape '{}' has a broken in-between at {}",
                self.name,
                b.weight
            );
        }
        Ok(())
    }
}

/// `<shape>_ib<percent>`: an in-between's name as a target of its own.
pub fn inbetween_name(shape: &str, weight: f32) -> String {
    format!("{shape}_ib{}", (weight * 100.0).round() as i32)
}

/// Read `<shape>_ib<percent>` back: the shape's name and the weight.
pub fn parse_inbetween_name(name: &str) -> Option<(&str, f32)> {
    let (shape, pct) = name.rsplit_once("_ib")?;
    let pct: u32 = pct.parse().ok()?;
    (!shape.is_empty() && (1..100).contains(&pct)).then_some((shape, pct as f32 / 100.0))
}

/// `rest` with each shape added at its weight in `weights` (one per shape).
pub fn apply(rest: &[Vec3], shapes: &[Shape], weights: &[f32]) -> Vec<Vec3> {
    let mut out = rest.to_vec();
    for (s, &w) in shapes.iter().zip(weights) {
        s.add_to(&mut out, w);
    }
    out
}

/// One target of a shape, as formats without in-betweens carry it: a morph
/// target or shape key of its own.
#[derive(Debug, Clone)]
pub struct FlatTarget<'a> {
    /// The shape's name, or `<shape>_ib<percent>` for an in-between.
    pub name: String,
    /// Which shape of the list it belongs to.
    pub shape: usize,
    pub vertices: &'a [u32],
    pub offsets: &'a [Vec3],
    /// Normal offsets (the full shape's only; empty when it has none).
    pub normals: &'a [Vec3],
    /// The target's weight for the shape's own weight.
    pub weight: f32,
}

/// Every target of `shapes` as a target of its own, each shape's in-betweens
/// before its full target, weighted so that together they give the shape at
/// its weight.
pub fn flat_targets(shapes: &[Shape]) -> Vec<FlatTarget<'_>> {
    let mut out = Vec::new();
    for (i, s) in shapes.iter().enumerate() {
        let mix = s.mix(s.weight);
        for (k, b) in s.inbetweens.iter().enumerate() {
            out.push(FlatTarget {
                name: s.inbetween_name(k),
                shape: i,
                vertices: &b.vertices,
                offsets: &b.offsets,
                normals: &[],
                weight: mix[k],
            });
        }
        out.push(FlatTarget {
            name: s.name.clone(),
            shape: i,
            vertices: &s.vertices,
            offsets: &s.offsets,
            normals: &s.normal_offsets,
            weight: *mix.last().unwrap_or(&0.0),
        });
    }
    out
}

/// Fold targets named `<shape>_ib<percent>` into `<shape>`'s in-betweens, the
/// way hive-rig writes them to formats without in-betweens (glTF morph
/// targets, Blender shape keys). The folded shape's weight is recovered from
/// the targets' weights. Targets without a matching shape stay as they are.
pub fn fold_inbetweens(shapes: Vec<Shape>) -> Vec<Shape> {
    let names: std::collections::HashSet<String> = shapes.iter().map(|s| s.name.clone()).collect();
    let mut out: Vec<Shape> = Vec::new();
    let mut parts: HashMap<String, Vec<Shape>> = HashMap::new();
    for s in shapes {
        match parse_inbetween_name(&s.name) {
            Some((base, _)) if names.contains(base) => {
                parts.entry(base.to_string()).or_default().push(s);
            }
            _ => out.push(s),
        }
    }
    for s in &mut out {
        let Some(mut ibs) = parts.remove(&s.name) else {
            continue;
        };
        ibs.sort_by(|a, b| {
            let wa = parse_inbetween_name(&a.name).map_or(0.0, |x| x.1);
            let wb = parse_inbetween_name(&b.name).map_or(0.0, |x| x.1);
            wa.total_cmp(&wb)
        });
        // Each target's weight times the weight it is reached at adds up to
        // the shape's weight (two neighbouring targets share it linearly).
        let mut w = s.weight;
        for ib in ibs {
            let at = parse_inbetween_name(&ib.name).map_or(0.5, |x| x.1);
            w += ib.weight * at;
            s.inbetweens.push(Inbetween {
                weight: at,
                vertices: ib.vertices,
                offsets: ib.offsets,
            });
        }
        s.weight = w;
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
        inbetweens: Vec::new(),
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
    out.inbetweens = shape
        .inbetweens
        .iter()
        .map(|b| {
            let src = b.dense(n);
            let dense: Vec<Vec3> = map
                .iter()
                .map(|m| m.map_or(Vec3::ZERO, |j| flip(src[j as usize])))
                .collect();
            Inbetween::from_dense(b.weight, &dense)
        })
        .collect();
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
            inbetweens: Vec::new(),
        };
        let share = |v: u32| {
            let s = left_share(positions[v as usize].x);
            if left { s } else { 1.0 - s }
        };
        for (k, (&v, d)) in shape.vertices.iter().zip(&shape.offsets).enumerate() {
            let s = share(v);
            if s <= 0.0 {
                continue;
            }
            out.vertices.push(v);
            out.offsets.push(*d * s);
            if let Some(nd) = shape.normal_offsets.get(k) {
                out.normal_offsets.push(*nd * s);
            }
        }
        for b in &shape.inbetweens {
            let mut half = Inbetween {
                weight: b.weight,
                vertices: Vec::new(),
                offsets: Vec::new(),
            };
            for (&v, d) in b.vertices.iter().zip(&b.offsets) {
                let s = share(v);
                if s > 0.0 {
                    half.vertices.push(v);
                    half.offsets.push(*d * s);
                }
            }
            out.inbetweens.push(half);
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
    posed_mesh_with(rig, locals, &skin_matrices(&rig.skeleton, locals))
}

/// [`posed_mesh`] with the skinning matrices given (a pose from the control
/// rig, which moves and scales bones as well as turning them); `locals` are
/// the same pose's local rotations, for the corrective readers.
pub fn posed_mesh_with(rig: &Rig, locals: &[Quat], mats: &[Mat4]) -> Vec<Vec3> {
    let weights = weights_in_pose(&rig.mesh.shapes, locals, |n| rig.skeleton.index_of(n));
    let base = apply(&rig.mesh.positions, &rig.mesh.shapes, &weights);
    deform(&base, &rig.skin, mats)
}

/// Where the skin puts each vertex in a pose with one target of the shape
/// `name` fully on (the in-between reached at `inbetween`, or the full
/// shape) and the other shapes at their weights in that pose: the mesh a
/// sculpt of that target starts from.
pub fn posed_target(
    rig: &Rig,
    name: &str,
    inbetween: Option<f32>,
    locals: &[Quat],
    mats: &[Mat4],
) -> Vec<Vec3> {
    let shapes = &rig.mesh.shapes;
    let mut weights = weights_in_pose(shapes, locals, |n| rig.skeleton.index_of(n));
    if let Some(i) = shapes.iter().position(|s| s.name == name) {
        weights[i] = inbetween.unwrap_or(1.0);
    }
    let base = apply(&rig.mesh.positions, shapes, &weights);
    deform(&base, &rig.skin, mats)
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
    let mats = skin_matrices(&rig.skeleton, locals);
    sculpt_with(rig, name, locals, &mats, moves)
}

/// [`sculpt`] in a pose given by its skinning matrices (and local rotations).
pub fn sculpt_with(
    rig: &mut Rig,
    name: &str,
    locals: &[Quat],
    mats: &[Mat4],
    moves: &[(usize, Vec3)],
) -> anyhow::Result<usize> {
    sculpt_target(rig, name, None, locals, mats, moves)
}

/// [`sculpt_with`] into one of the shape's targets: the in-between reached at
/// `inbetween` (which must exist), or the full shape with `None`.
pub fn sculpt_target(
    rig: &mut Rig,
    name: &str,
    inbetween: Option<f32>,
    locals: &[Quat],
    mats: &[Mat4],
    moves: &[(usize, Vec3)],
) -> anyhow::Result<usize> {
    anyhow::ensure!(!name.trim().is_empty(), "the shape needs a name");
    let shapes = &rig.mesh.shapes;
    let mine = shapes.iter().position(|s| s.name == name);
    let target = match (inbetween, mine) {
        (None, _) => None,
        (Some(w), Some(i)) => Some(
            shapes[i]
                .inbetween_at(w)
                .ok_or_else(|| anyhow::anyhow!("'{name}' has no in-between at {w:.2}"))?,
        ),
        (Some(_), None) => anyhow::bail!("the rig has no shape '{name}'"),
    };
    let mut weights = weights_in_pose(shapes, locals, |n| rig.skeleton.index_of(n));
    if let Some(i) = mine {
        // This target counts fully: the new offsets go on top of its own.
        weights[i] = target.map_or(1.0, |k| shapes[i].inbetweens[k].weight);
    }
    let base = apply(&rig.mesh.positions, shapes, &weights);
    let offsets = corrective_offsets(&base, &rig.skin, mats, moves);
    let n = rig.mesh.vertex_count();
    let mut dense = vec![Vec3::ZERO; n];
    for (v, d) in &offsets {
        if *v < n {
            dense[*v] = *d;
        }
    }
    let added = Shape::from_dense(name, &dense);
    let moved = added.vertices.len();
    match (mine, target) {
        (Some(i), Some(k)) => {
            let b = &mut rig.mesh.shapes[i].inbetweens[k];
            let mut sum = b.dense(n);
            for (s, d) in sum.iter_mut().zip(&dense) {
                *s += *d;
            }
            *b = Inbetween::from_dense(b.weight, &sum);
        }
        (Some(i), None) => {
            let old = rig.mesh.shapes[i].clone();
            let mut merged = combine(&[&old, &added], name);
            merged.weight = old.weight;
            merged.driver = old.driver;
            merged.inbetweens = old.inbetweens;
            rig.mesh.shapes[i] = merged;
        }
        (None, _) => rig.mesh.shapes.push(added),
    }
    Ok(moved)
}

/// Give `name` an in-between at `weight` (between 0 and 1), starting where the
/// shape puts the vertices at that weight now, so nothing moves until it is
/// sculpted. Returns its index among the shape's in-betweens.
pub fn add_inbetween(rig: &mut Rig, name: &str, weight: f32) -> anyhow::Result<usize> {
    anyhow::ensure!(
        weight.is_finite() && (0.01..=0.99).contains(&weight),
        "an in-between sits between 0.01 and 0.99, not {weight}"
    );
    let weight = (weight * 100.0).round() / 100.0;
    let n = rig.mesh.vertex_count();
    let i = index(rig, name)?;
    let s = &rig.mesh.shapes[i];
    anyhow::ensure!(
        s.inbetweens
            .iter()
            .all(|b| (b.weight - weight).abs() >= 0.01 - 1e-5),
        "'{name}' already has an in-between at {weight:.2}"
    );
    let b = Inbetween::from_dense(weight, &s.dense_at(n, weight));
    let s = &mut rig.mesh.shapes[i];
    let k = s.inbetweens.partition_point(|x| x.weight < weight);
    s.inbetweens.insert(k, b);
    Ok(k)
}

/// Take the in-between at `weight` off `name`.
pub fn remove_inbetween(rig: &mut Rig, name: &str, weight: f32) -> anyhow::Result<()> {
    let i = index(rig, name)?;
    let k = rig.mesh.shapes[i]
        .inbetween_at(weight)
        .ok_or_else(|| anyhow::anyhow!("'{name}' has no in-between at {weight:.2}"))?;
    rig.mesh.shapes[i].inbetweens.remove(k);
    Ok(())
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

/// What the sculpt brush does to the vertices under it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SculptBrush {
    /// Drag the vertices along with the pointer.
    #[default]
    Grab,
    /// Pull each vertex toward its neighbours' average (evens out bumps).
    Smooth,
    /// Like Smooth but only along the surface, so the shape's volume stays.
    Relax,
    /// Push the vertices out along their normals (in with a negative amount).
    Inflate,
    /// Take the shape's own offsets back off, toward the mesh without it.
    Erase,
}

impl SculptBrush {
    pub const ALL: [SculptBrush; 5] = [
        SculptBrush::Grab,
        SculptBrush::Smooth,
        SculptBrush::Relax,
        SculptBrush::Inflate,
        SculptBrush::Erase,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SculptBrush::Grab => "Grab",
            SculptBrush::Smooth => "Smooth",
            SculptBrush::Relax => "Relax",
            SculptBrush::Inflate => "Inflate",
            SculptBrush::Erase => "Erase",
        }
    }
}

/// The brush's reach at distance `d` from its centre: 1 there, fading
/// smoothly to 0 at `radius`.
pub fn falloff(d: f32, radius: f32) -> f32 {
    if radius <= 0.0 || d >= radius {
        return 0.0;
    }
    let t = 1.0 - (d / radius).powi(2);
    t * t
}

/// Unit vertex normals of a posed mesh, the same on every copy of a vertex
/// split at a seam (`weld` gives each vertex its welded index).
pub fn welded_normals(posed: &[Vec3], indices: &[u32], weld: &[u32]) -> Vec<Vec3> {
    let raw = crate::mesh::smooth_normals(posed, indices);
    let mut sum = vec![Vec3::ZERO; posed.len()];
    for (v, n) in raw.iter().enumerate() {
        if let Some(&w) = weld.get(v)
            && let Some(s) = sum.get_mut(w as usize)
        {
            *s += *n;
        }
    }
    (0..posed.len())
        .map(|v| {
            let w = weld.get(v).map_or(v, |&w| w as usize);
            sum.get(w).copied().unwrap_or_default().normalize_or_zero()
        })
        .collect()
}

/// The vertices within `radius` of `at`, with their falloff.
fn under_brush(posed: &[Vec3], at: Vec3, radius: f32) -> impl Iterator<Item = (usize, f32)> + '_ {
    posed.iter().enumerate().filter_map(move |(v, p)| {
        let f = falloff(p.distance(at), radius);
        (f > 0.0).then_some((v, f))
    })
}

fn neighbour_mean(posed: &[Vec3], adjacency: &[Vec<u32>], v: usize) -> Option<Vec3> {
    let nb = adjacency.get(v)?;
    (!nb.is_empty()).then(|| nb.iter().map(|&j| posed[j as usize]).sum::<Vec3>() / nb.len() as f32)
}

/// Smooth: each vertex under the brush moves toward its neighbours' average
/// by `strength` (0 to 1) at the centre.
pub fn smooth_moves(
    posed: &[Vec3],
    adjacency: &[Vec<u32>],
    at: Vec3,
    radius: f32,
    strength: f32,
) -> Vec<(usize, Vec3)> {
    under_brush(posed, at, radius)
        .filter_map(|(v, f)| {
            let mean = neighbour_mean(posed, adjacency, v)?;
            Some((v, posed[v] + (mean - posed[v]) * (strength * f)))
        })
        .collect()
}

/// Relax: like [`smooth_moves`] with the move along the normal taken out, so
/// vertices even out across the surface without it shrinking.
pub fn relax_moves(
    posed: &[Vec3],
    normals: &[Vec3],
    adjacency: &[Vec<u32>],
    at: Vec3,
    radius: f32,
    strength: f32,
) -> Vec<(usize, Vec3)> {
    under_brush(posed, at, radius)
        .filter_map(|(v, f)| {
            let mean = neighbour_mean(posed, adjacency, v)?;
            let d = mean - posed[v];
            let n = normals.get(v).copied().unwrap_or_default();
            Some((v, posed[v] + (d - n * d.dot(n)) * (strength * f)))
        })
        .collect()
}

/// Inflate: each vertex under the brush moves `amount` metres along its
/// normal at the centre (inward when negative).
pub fn inflate_moves(
    posed: &[Vec3],
    normals: &[Vec3],
    at: Vec3,
    radius: f32,
    amount: f32,
) -> Vec<(usize, Vec3)> {
    under_brush(posed, at, radius)
        .map(|(v, f)| {
            let n = normals.get(v).copied().unwrap_or_default();
            (v, posed[v] + n * (amount * f))
        })
        .collect()
}

/// Erase: scale the target's offsets under the brush (measured on the mesh
/// as `posed` shows it) down by `strength` at the centre, toward no change.
/// Returns how many vertices changed.
pub fn erase(
    rig: &mut Rig,
    name: &str,
    inbetween: Option<f32>,
    posed: &[Vec3],
    at: Vec3,
    radius: f32,
    strength: f32,
) -> anyhow::Result<usize> {
    let i = index(rig, name)?;
    let s = &mut rig.mesh.shapes[i];
    let (vertices, offsets) = match inbetween {
        Some(w) => {
            let k = s
                .inbetween_at(w)
                .ok_or_else(|| anyhow::anyhow!("'{name}' has no in-between at {w:.2}"))?;
            let b = &mut s.inbetweens[k];
            (&b.vertices, &mut b.offsets)
        }
        None => (&s.vertices, &mut s.offsets),
    };
    let mut changed = 0;
    for (&v, d) in vertices.iter().zip(offsets.iter_mut()) {
        let f = posed
            .get(v as usize)
            .map_or(0.0, |p| falloff(p.distance(at), radius));
        if f > 0.0 && *d != Vec3::ZERO {
            *d *= (1.0 - strength * f).max(0.0);
            changed += 1;
        }
    }
    // Drop the vertices that no longer move (keeping normal offsets in step).
    if inbetween.is_none() && !s.normal_offsets.is_empty() {
        return Ok(changed);
    }
    match inbetween {
        Some(w) => {
            let k = s.inbetween_at(w).unwrap_or_default();
            let n = rig.mesh.positions.len();
            let b = &mut rig.mesh.shapes[i].inbetweens[k];
            *b = Inbetween::from_dense(b.weight, &b.dense(n));
        }
        None => {
            let n = rig.mesh.positions.len();
            let old = rig.mesh.shapes[i].clone();
            let mut fresh = Shape::from_dense(name, &old.dense(n));
            fresh.weight = old.weight;
            fresh.driver = old.driver;
            fresh.inbetweens = old.inbetweens;
            rig.mesh.shapes[i] = fresh;
        }
    }
    Ok(changed)
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
    fn inbetweens_blend_in_order_and_fold_back() {
        // Without in-betweens the weight scales the offsets, past 1 too.
        assert_eq!(target_mix(&[1.0], 0.3), vec![0.3]);
        assert_eq!(target_mix(&[1.0], 1.5), vec![1.5]);
        let m = target_mix(&[0.25, 0.5, 1.0], 0.375);
        assert!((m[0] - 0.5).abs() < 1e-6 && (m[1] - 0.5).abs() < 1e-6 && m[2] == 0.0);
        assert!((target_mix(&[0.25, 0.5, 1.0], 0.125)[0] - 0.5).abs() < 1e-6);
        let m = target_mix(&[0.25, 0.5, 1.0], 0.75);
        assert!((m[1] - 0.5).abs() < 1e-6 && (m[2] - 0.5).abs() < 1e-6);

        let mut rig =
            crate::rig::auto_rig(crate::demo::demo_humanoid(), "demo", Default::default()).unwrap();
        let n = rig.mesh.vertex_count();
        let rest = vec![Quat::IDENTITY; rig.skeleton.bones.len()];
        let mut dense = vec![Vec3::ZERO; n];
        dense[3] = Vec3::new(0.0, 0.1, 0.0);
        let mut s = Shape::from_dense("lift", &dense);
        s.weight = 0.5;
        rig.mesh.shapes.push(s);
        // A new in-between starts where the shape is: nothing moves.
        let before = posed_mesh(&rig, &rest);
        assert_eq!(add_inbetween(&mut rig, "lift", 0.5).unwrap(), 0);
        assert!(add_inbetween(&mut rig, "lift", 0.5).is_err());
        assert!(add_inbetween(&mut rig, "lift", 1.0).is_err());
        let after = posed_mesh(&rig, &rest);
        assert!(before[3].distance(after[3]) < 1e-6);
        // Sculpt the in-between sideways: at 0.5 the vertex is there, at 1 the
        // full shape is untouched, at 0.25 it is half way to the in-between.
        let to = after[3] + Vec3::new(0.05, 0.0, 0.0);
        let mats = skin_matrices(&rig.skeleton, &rest);
        sculpt_target(&mut rig, "lift", Some(0.5), &rest, &mats, &[(3, to)]).unwrap();
        assert!(posed_mesh(&rig, &rest)[3].distance(to) < 1e-5);
        set_weight(&mut rig, "lift", 1.0).unwrap();
        let full = posed_mesh(&rig, &rest)[3] - rig.mesh.positions[3];
        assert!(full.distance(Vec3::new(0.0, 0.1, 0.0)) < 1e-5);
        set_weight(&mut rig, "lift", 0.25).unwrap();
        let quarter = posed_mesh(&rig, &rest)[3] - rig.mesh.positions[3];
        assert!(
            quarter.distance(Vec3::new(0.025, 0.025, 0.0)) < 1e-5,
            "{quarter}"
        );
        rig.mesh.validate().unwrap();
        // Out to formats without in-betweens and back: the same shape.
        let flat = flat_targets(&rig.mesh.shapes);
        assert_eq!(
            flat.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
            ["lift_ib50", "lift"]
        );
        assert!((flat[0].weight - 0.5).abs() < 1e-6 && flat[1].weight == 0.0);
        let as_shapes: Vec<Shape> = flat
            .iter()
            .map(|t| {
                let mut s = Shape::from_dense(&t.name, &[]);
                s.vertices = t.vertices.to_vec();
                s.offsets = t.offsets.to_vec();
                s.weight = t.weight;
                s
            })
            .collect();
        let back = fold_inbetweens(as_shapes);
        assert_eq!(back.len(), 1);
        assert_eq!(back[0].inbetweens, rig.mesh.shapes[0].inbetweens);
        assert!((back[0].weight - 0.25).abs() < 1e-6);
        assert_eq!(parse_inbetween_name("a_b_ib35"), Some(("a_b", 0.35)));
        assert_eq!(parse_inbetween_name("ib35"), None);
        // Mirror and split carry the in-betweens.
        let twin = mirror_on_rig(&mut rig, "lift", Some("lift_m")).unwrap();
        assert_eq!(find(&rig, &twin).unwrap().inbetweens.len(), 1);
        let [l, _] = split_on_rig(&mut rig, "lift", None, None).unwrap();
        assert_eq!(find(&rig, &l).unwrap().inbetweens.len(), 1);
        remove_inbetween(&mut rig, "lift", 0.5).unwrap();
        assert!(find(&rig, "lift").unwrap().inbetweens.is_empty());
    }

    #[test]
    fn sculpt_brushes_smooth_relax_inflate_and_erase() {
        // A bumpy row of points: the middle one sticks up.
        let posed = vec![
            Vec3::new(-0.1, 0.0, 0.0),
            Vec3::new(0.0, 0.05, 0.0),
            Vec3::new(0.1, 0.0, 0.0),
        ];
        let adj = vec![vec![1], vec![0, 2], vec![1]];
        let s = smooth_moves(&posed, &adj, posed[1], 0.05, 1.0);
        let mid = s.iter().find(|m| m.0 == 1).unwrap().1;
        assert!(mid.y.abs() < 1e-6, "smooth flattens the bump: {mid}");
        // Relax with the normal along Y doesn't flatten it: it only slides.
        let normals = vec![Vec3::Y; 3];
        let r = relax_moves(&posed, &normals, &adj, posed[1], 0.05, 1.0);
        assert!((r.iter().find(|m| m.0 == 1).unwrap().1.y - 0.05).abs() < 1e-6);
        let i = inflate_moves(&posed, &normals, posed[1], 0.05, 0.01);
        assert!((i.iter().find(|m| m.0 == 1).unwrap().1.y - 0.06).abs() < 1e-6);
        assert!(falloff(0.05, 0.05) == 0.0 && falloff(0.0, 0.05) == 1.0);

        let mut rig =
            crate::rig::auto_rig(crate::demo::demo_humanoid(), "demo", Default::default()).unwrap();
        let n = rig.mesh.vertex_count();
        let mut dense = vec![Vec3::ZERO; n];
        dense[0] = Vec3::X * 0.01;
        dense[n - 1] = Vec3::X * 0.01;
        rig.mesh.shapes.push(Shape::from_dense("e", &dense));
        let posed = rig.mesh.positions.clone();
        let at = posed[0];
        let radius = 1e-4;
        assert_eq!(
            erase(&mut rig, "e", None, &posed, at, radius, 1.0).unwrap(),
            1
        );
        assert_eq!(find(&rig, "e").unwrap().vertices, vec![n as u32 - 1]);
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
