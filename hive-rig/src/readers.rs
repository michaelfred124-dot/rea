//! Pose readers that look at more than one turn of a bone: cone readers and
//! RBF (radial basis function) readers, and the helper joints they move.
//!
//! A reader watches one bone's rotation relative to its parent (the same local
//! rotation the single-axis readers of [`crate::shapes::PoseReader`] read) and
//! gives one weight per output. Its outputs are shapes (whose weight it sets)
//! and helper joints (which it moves and scales). The maths is in
//! `docs/skinning.md` ("Cone and RBF readers").
//!
//! * A **cone reader** compares the direction the bone points in now with a
//!   direction of its own: fully on within `angle` degrees of it, fading out
//!   smoothly over the next `falloff` degrees (or the other way round with
//!   `away`). Twist about the bone doesn't change where it points, so it
//!   doesn't change the weight.
//! * An **RBF reader** holds sample poses of the bone, each with a value per
//!   output, and blends between them with a Gaussian kernel over the distance
//!   between rotations (or between pointing directions, with `swing`). The
//!   kernel weights are solved so that every sample pose gives back exactly its
//!   own values; the rest pose counts as a sample with every output off unless
//!   a sample sits there already.
//! * A **helper joint** hangs under a bone of the rig and is skinned like any
//!   other joint (automatic weights within its radius, and paintable). At
//!   weight `w` it moves `w * offset` in its parent's frame and scales about
//!   its head, `along` the parent bone and `across` it, by `1 + (s - 1) * w`.
//!
//! Readers live with the shapes on the mesh (`mesh.readers` in the rig file),
//! so a rebuild from edited guides keeps them and puts their helper joints
//! back on the new skeleton.

use std::collections::HashMap;

use glam::{Mat3, Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::rig::Rig;
use crate::shapes::Shape;
use crate::skeleton::{Side, Skeleton};

/// The `component` of the bones made for helper joints.
pub const HELPER_COMPONENT: &str = "pose_reader";
/// A sample within this many degrees of rest stands in for the rest pose.
pub const REST_NEAR: f32 = 1.0;
/// Samples closer than this (degrees) can't be told apart by the solve.
pub const MIN_APART: f32 = 1.0;
/// Without a radius, and without two samples to measure one from.
pub const DEFAULT_RADIUS: f32 = 45.0;
/// Added to the kernel matrix's diagonal so near-duplicate samples still solve.
const RIDGE: f64 = 1e-6;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Reader {
    pub name: String,
    /// The bone whose rotation (relative to its parent) is read.
    pub bone: String,
    #[serde(flatten)]
    pub kind: Kind,
    /// Shapes it drives, in output order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shapes: Vec<String>,
    /// Helper joints it moves, in output order after the shapes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub helpers: Vec<Helper>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Kind {
    Cone(Cone),
    Rbf(Rbf),
}

/// Fully on while the bone points within `angle` degrees of `direction`,
/// easing off to nothing at `angle + falloff` degrees.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cone {
    /// The cone's centre: a direction in the parent bone's rest frame (model
    /// axes, since hive-rig's bones carry no rest rotation).
    pub direction: Vec3,
    /// Half-angle of the fully-on cone, in degrees.
    pub angle: f32,
    /// How many degrees past `angle` the weight takes to fade to 0.
    pub falloff: f32,
    /// Turn it inside out: off inside the cone, on once the bone has swung
    /// `angle + falloff` degrees away from `direction` in any direction.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub away: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rbf {
    #[serde(default)]
    pub poses: Vec<Sample>,
    /// Width of the Gaussian kernel in degrees; 0 picks it from the samples
    /// (the mean distance from each to its nearest neighbour).
    #[serde(default)]
    pub radius: f32,
    /// Measure the distance between poses by where the bone points, ignoring
    /// its twist (otherwise by the whole rotation).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub swing: bool,
}

/// One sample pose of an RBF reader: the bone's local rotation and the value
/// each output takes there (shapes first, then helpers).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sample {
    pub rotation: Quat,
    pub values: Vec<f32>,
}

/// A joint a reader moves and scales: a volume joint at an elbow or knee.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Helper {
    pub name: String,
    /// The bone it hangs under.
    pub parent: String,
    /// Where it sits at rest (model space, metres).
    pub head: Vec3,
    /// How far it moves at full weight, in the parent's rest frame (metres).
    #[serde(default)]
    pub offset: Vec3,
    /// Its scale at full weight along the parent bone.
    #[serde(default = "one")]
    pub along: f32,
    /// Its scale at full weight across the parent bone.
    #[serde(default = "one")]
    pub across: f32,
    /// Only vertices within this distance (metres) take its skin weight.
    pub radius: f32,
}

fn one() -> f32 {
    1.0
}

// --------------------------------------------------------------------------- maths

/// `smoothstep`: 0 below `e0`, 1 above `e1`, and `t^2 (3 - 2t)` between.
pub fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0).max(1e-6)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The angle between two directions, in degrees (0 to 180).
pub fn swing_degrees(a: Vec3, b: Vec3) -> f32 {
    let (a, b) = (a.normalize_or_zero(), b.normalize_or_zero());
    a.dot(b).clamp(-1.0, 1.0).acos().to_degrees()
}

/// The angle of the turn taking one rotation to the other, in degrees (0 to
/// 180): `2 acos |a . b|`, the same for `q` and `-q`.
pub fn turn_between(a: Quat, b: Quat) -> f32 {
    (2.0 * a.dot(b).abs().min(1.0).acos()).to_degrees()
}

/// The Gaussian kernel at a distance of `d` degrees, `radius` degrees wide.
pub fn kernel(d: f32, radius: f32) -> f32 {
    let x = d / radius.max(1e-3);
    (-x * x).exp()
}

impl Cone {
    /// The weight for a bone pointing along `dir` (in its parent's rest frame).
    pub fn weight(&self, dir: Vec3) -> f32 {
        let theta = swing_degrees(dir, self.direction);
        let out = smoothstep(self.angle, self.angle + self.falloff, theta);
        if self.away { out } else { 1.0 - out }
    }
}

/// An RBF reader solved for evaluation: its centres (the samples, and rest
/// when no sample stands in for it) and the kernel weights that give each
/// centre back its values.
#[derive(Debug, Clone, PartialEq)]
pub struct Solved {
    pub centres: Vec<Quat>,
    /// Where the bone points at each centre (for `swing`).
    pub directions: Vec<Vec3>,
    /// One row per centre, one column per output.
    pub weights: Vec<Vec<f32>>,
    /// The kernel width used, in degrees.
    pub radius: f32,
    pub swing: bool,
    /// The bone's rest direction.
    pub along: Vec3,
}

impl Rbf {
    /// The distance between two poses of a bone with rest direction `along`.
    pub fn distance(&self, a: Quat, b: Quat, along: Vec3) -> f32 {
        if self.swing {
            swing_degrees(a * along, b * along)
        } else {
            turn_between(a, b)
        }
    }

    /// The samples plus rest (all outputs off) unless a sample sits there.
    pub fn centres(&self, along: Vec3, outputs: usize) -> (Vec<Quat>, Vec<Vec<f32>>) {
        let mut q: Vec<Quat> = self.poses.iter().map(|p| p.rotation.normalize()).collect();
        let mut v: Vec<Vec<f32>> = self
            .poses
            .iter()
            .map(|p| {
                (0..outputs)
                    .map(|k| p.values.get(k).copied().unwrap_or(0.0))
                    .collect()
            })
            .collect();
        if !q
            .iter()
            .any(|&p| self.distance(p, Quat::IDENTITY, along) < REST_NEAR)
        {
            q.insert(0, Quat::IDENTITY);
            v.insert(0, vec![0.0; outputs]);
        }
        (q, v)
    }

    /// The kernel width: `radius`, or the mean nearest-neighbour distance.
    pub fn width(&self, centres: &[Quat], along: Vec3) -> f32 {
        if self.radius > 0.0 {
            return self.radius;
        }
        if centres.len() < 2 {
            return DEFAULT_RADIUS;
        }
        let near: f32 = centres
            .iter()
            .enumerate()
            .map(|(i, &a)| {
                centres
                    .iter()
                    .enumerate()
                    .filter(|(j, _)| *j != i)
                    .map(|(_, &b)| self.distance(a, b, along))
                    .fold(f32::INFINITY, f32::min)
            })
            .sum();
        (near / centres.len() as f32).max(MIN_APART)
    }

    /// Solve `(Phi + ridge I) W = V` for the kernel weights.
    pub fn solve(&self, along: Vec3, outputs: usize) -> anyhow::Result<Solved> {
        let along = along.normalize_or_zero();
        let (centres, values) = self.centres(along, outputs);
        let radius = self.width(&centres, along);
        let n = centres.len();
        let mut phi = vec![vec![0.0f64; n]; n];
        for i in 0..n {
            for j in 0..n {
                phi[i][j] = kernel(self.distance(centres[i], centres[j], along), radius) as f64;
            }
            phi[i][i] += RIDGE;
        }
        let rhs: Vec<Vec<f64>> = values
            .iter()
            .map(|r| r.iter().map(|&x| x as f64).collect())
            .collect();
        let w = solve_linear(phi, rhs).ok_or_else(|| {
            anyhow::anyhow!(
                "the sample poses can't be told apart; take one out or set a smaller radius"
            )
        })?;
        Ok(Solved {
            directions: centres.iter().map(|q| *q * along).collect(),
            centres,
            weights: w
                .into_iter()
                .map(|r| r.into_iter().map(|x| x as f32).collect())
                .collect(),
            radius,
            swing: self.swing,
            along,
        })
    }
}

impl Solved {
    /// The kernel of centre `i` for a bone at local rotation `local`.
    pub fn kernel_at(&self, i: usize, local: Quat) -> f32 {
        let d = if self.swing {
            swing_degrees(local * self.along, self.directions[i])
        } else {
            turn_between(local, self.centres[i])
        };
        kernel(d, self.radius)
    }

    /// Each output's value at `local`, held to 0..1.
    pub fn eval(&self, local: Quat) -> Vec<f32> {
        let m = self.weights.first().map_or(0, Vec::len);
        let mut out = vec![0.0f32; m];
        for (i, row) in self.weights.iter().enumerate() {
            let k = self.kernel_at(i, local);
            for (o, w) in out.iter_mut().zip(row) {
                *o += w * k;
            }
        }
        out.iter().map(|x| x.clamp(0.0, 1.0)).collect()
    }
}

/// Gaussian elimination with partial pivoting: `a x = b` for each column of
/// `b` (given as rows, one per equation). `None` when `a` is singular.
fn solve_linear(mut a: Vec<Vec<f64>>, mut b: Vec<Vec<f64>>) -> Option<Vec<Vec<f64>>> {
    let n = a.len();
    for c in 0..n {
        let p = (c..n).max_by(|&i, &j| a[i][c].abs().total_cmp(&a[j][c].abs()))?;
        if a[p][c].abs() < 1e-12 {
            return None;
        }
        a.swap(c, p);
        b.swap(c, p);
        let (a_top, a_rest) = a.split_at_mut(c + 1);
        let (b_top, b_rest) = b.split_at_mut(c + 1);
        let (pivot, pivot_b) = (&a_top[c], &b_top[c]);
        for (row, row_b) in a_rest.iter_mut().zip(b_rest.iter_mut()) {
            let f = row[c] / pivot[c];
            if f == 0.0 {
                continue;
            }
            for (x, p) in row[c..].iter_mut().zip(&pivot[c..]) {
                *x -= f * p;
            }
            for (x, p) in row_b.iter_mut().zip(pivot_b) {
                *x -= f * p;
            }
        }
    }
    for c in (0..n).rev() {
        for k in 0..b[c].len() {
            let mut s = b[c][k];
            for j in c + 1..n {
                s -= a[c][j] * b[j][k];
            }
            b[c][k] = s / a[c][c];
        }
    }
    Some(b)
}

impl Reader {
    /// How many weights it gives: one per shape, then one per helper.
    pub fn outputs(&self) -> usize {
        self.shapes.len() + self.helpers.len()
    }

    pub fn kind_label(&self) -> &'static str {
        match self.kind {
            Kind::Cone(_) => "cone",
            Kind::Rbf(_) => "RBF",
        }
    }

    /// Its outputs for the bone at local rotation `local`, where the bone
    /// points along `along` at rest.
    pub fn evaluate(&self, local: Quat, along: Vec3) -> Vec<f32> {
        match &self.kind {
            Kind::Cone(c) => vec![c.weight(local * along); self.outputs()],
            Kind::Rbf(r) => r
                .solve(along, self.outputs())
                .map(|s| s.eval(local))
                .unwrap_or_else(|_| vec![0.0; self.outputs()]),
        }
    }

    /// The weight of one of its outputs among `out` (its evaluation).
    pub fn shape_index(&self, shape: &str) -> Option<usize> {
        self.shapes.iter().position(|s| s == shape)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(!self.name.trim().is_empty(), "a pose reader has no name");
        let n = &self.name;
        match &self.kind {
            Kind::Cone(c) => anyhow::ensure!(
                c.direction.is_finite()
                    && c.direction.length_squared() > 1e-12
                    && c.angle.is_finite()
                    && (0.0..180.0).contains(&c.angle)
                    && c.falloff.is_finite()
                    && c.falloff > 0.0,
                "cone reader '{n}' needs a direction, an angle from 0 to 180 and a falloff above 0"
            ),
            Kind::Rbf(r) => {
                anyhow::ensure!(
                    r.radius.is_finite() && r.radius >= 0.0,
                    "RBF reader '{n}' has a bad radius"
                );
                for (k, p) in r.poses.iter().enumerate() {
                    anyhow::ensure!(
                        p.rotation.is_finite()
                            && p.rotation.length_squared() > 1e-6
                            && p.values.len() == self.outputs()
                            && p.values.iter().all(|v| v.is_finite()),
                        "RBF reader '{n}' pose {} needs a rotation and one value per output ({})",
                        k + 1,
                        self.outputs()
                    );
                }
            }
        }
        let mut seen = std::collections::HashSet::new();
        for s in &self.shapes {
            anyhow::ensure!(
                seen.insert(s.as_str()),
                "reader '{n}' lists shape '{s}' twice"
            );
        }
        for h in &self.helpers {
            anyhow::ensure!(
                !h.name.trim().is_empty() && seen.insert(h.name.as_str()),
                "reader '{n}' has a helper without a name, or twice"
            );
            anyhow::ensure!(
                h.head.is_finite()
                    && h.offset.is_finite()
                    && h.along.is_finite()
                    && h.across.is_finite()
                    && h.radius.is_finite()
                    && h.radius > 0.0,
                "helper '{}' needs a place, a radius above 0 and finite moves",
                h.name
            );
        }
        Ok(())
    }
}

// --------------------------------------------------------------------------- in a pose

/// Each reader's outputs in a pose given by the bones' local rotations. A
/// reader whose bone the skeleton lacks gives zeros.
pub fn outputs_in_pose(readers: &[Reader], sk: &Skeleton, locals: &[Quat]) -> Vec<Vec<f32>> {
    readers
        .iter()
        .map(|r| match sk.index_of(&r.bone) {
            Some(i) => r.evaluate(
                locals.get(i).copied().unwrap_or(Quat::IDENTITY),
                sk.bones[i].direction(),
            ),
            None => vec![0.0; r.outputs()],
        })
        .collect()
}

/// The weight a reader gives `shape` among the evaluated `outputs`, if one drives it.
pub fn shape_weight(readers: &[Reader], outputs: &[Vec<f32>], shape: &str) -> Option<f32> {
    readers
        .iter()
        .zip(outputs)
        .find_map(|(r, o)| r.shape_index(shape).and_then(|k| o.get(k).copied()))
}

/// The reader driving `shape`, if any.
pub fn reader_of<'a>(readers: &'a [Reader], shape: &str) -> Option<&'a Reader> {
    readers.iter().find(|r| r.shape_index(shape).is_some())
}

/// The scale a helper takes at weight `w`, about its head, in model axes.
pub fn helper_scale(h: &Helper, parent_dir: Vec3, w: f32) -> Mat3 {
    let along = 1.0 + (h.along - 1.0) * w;
    let across = 1.0 + (h.across - 1.0) * w;
    let d = parent_dir.normalize_or_zero();
    let outer = Mat3::from_cols(d * d.x, d * d.y, d * d.z);
    Mat3::IDENTITY * across + outer * (along - across)
}

/// A helper's skinning matrix at weight `w` under its parent's `parent`: in the
/// rest pose it moves `w * offset` and scales about its head, then rides on
/// its parent.
pub fn helper_matrix(parent: Mat4, h: &Helper, parent_dir: Vec3, w: f32) -> Mat4 {
    parent
        * Mat4::from_translation(h.head + h.offset * w)
        * Mat4::from_mat3(helper_scale(h, parent_dir, w))
        * Mat4::from_translation(-h.head)
}

/// Set the skinning matrices (`mats`, one per bone) of the readers' helper
/// joints for the pose with local rotations `locals`. Their parents' matrices
/// must be set already.
pub fn drive_helpers(readers: &[Reader], sk: &Skeleton, locals: &[Quat], mats: &mut [Mat4]) {
    if readers.iter().all(|r| r.helpers.is_empty()) {
        return;
    }
    let outputs = outputs_in_pose(readers, sk, locals);
    for (r, out) in readers.iter().zip(&outputs) {
        for (k, h) in r.helpers.iter().enumerate() {
            let (Some(i), Some(p)) = (sk.index_of(&h.name), sk.index_of(&h.parent)) else {
                continue;
            };
            if i >= mats.len() || p >= mats.len() {
                continue;
            }
            let w = out.get(r.shapes.len() + k).copied().unwrap_or(0.0);
            mats[i] = helper_matrix(mats[p], h, sk.bones[p].direction(), w);
        }
    }
}

/// Every helper joint of `readers`.
pub fn helpers(readers: &[Reader]) -> impl Iterator<Item = &Helper> {
    readers.iter().flat_map(|r| &r.helpers)
}

/// Add a bone for each helper joint whose parent the skeleton has (helpers
/// already there are left alone). Returns the helpers that couldn't be added.
pub fn add_helper_bones(readers: &[Reader], sk: &mut Skeleton) -> Vec<String> {
    let mut missing = Vec::new();
    for h in helpers(readers) {
        if sk.index_of(&h.name).is_some() {
            continue;
        }
        let Some(p) = sk.index_of(&h.parent) else {
            missing.push(h.name.clone());
            continue;
        };
        let parent = &sk.bones[p];
        // A short bone along its parent: Blender gives it the parent's
        // direction as its Y, so `along` and `across` scale its own axes.
        let tail = h.head + parent.direction() * (0.5 * h.radius).max(1e-3);
        let side = if h.head.x > 1e-4 {
            Side::Left
        } else if h.head.x < -1e-4 {
            Side::Right
        } else {
            Side::Center
        };
        let region = parent.region;
        let parent_name = parent.name.clone();
        let i = sk.add(
            h.name.clone(),
            Some(&parent_name),
            h.head,
            tail,
            side,
            region,
        );
        let b = &mut sk.bones[i];
        b.component = HELPER_COMPONENT.into();
        b.reach = Some(h.radius);
    }
    missing
}

/// Whether bone `i` of `sk` is a reader's helper joint.
pub fn is_helper_bone(sk: &Skeleton, i: usize) -> bool {
    sk.bones
        .get(i)
        .is_some_and(|b| b.component == HELPER_COMPONENT)
}

// --------------------------------------------------------------------------- on a rig

pub fn find<'a>(rig: &'a Rig, name: &str) -> anyhow::Result<&'a Reader> {
    rig.mesh
        .readers
        .iter()
        .find(|r| r.name == name)
        .ok_or_else(|| anyhow::anyhow!("the rig has no pose reader '{name}'"))
}

/// Check every reader against the rig: bones, shapes and helpers exist, and
/// no shape has two drivers.
pub fn validate_on(rig: &Rig) -> anyhow::Result<()> {
    let sk = &rig.skeleton;
    let mut driven: HashMap<&str, &str> = HashMap::new();
    let mut names = std::collections::HashSet::new();
    for r in &rig.mesh.readers {
        r.validate()?;
        anyhow::ensure!(
            names.insert(r.name.as_str()),
            "two pose readers are called '{}'",
            r.name
        );
        let b = sk.index_of(&r.bone).ok_or_else(|| {
            anyhow::anyhow!(
                "reader '{}' reads bone '{}', which the rig doesn't have",
                r.name,
                r.bone
            )
        })?;
        anyhow::ensure!(
            !is_helper_bone(sk, b),
            "reader '{}' reads helper joint '{}'; read a bone of the rig",
            r.name,
            r.bone
        );
        for s in &r.shapes {
            let shape = rig
                .mesh
                .shapes
                .iter()
                .find(|x| &x.name == s)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "reader '{}' drives shape '{s}', which the rig doesn't have",
                        r.name
                    )
                })?;
            anyhow::ensure!(
                shape.driver.is_none() && shape.board.is_none(),
                "shape '{s}' has a driver already; a shape has one"
            );
            if let Some(other) = driven.insert(s.as_str(), r.name.as_str()) {
                anyhow::bail!("shape '{s}' is driven by both '{other}' and '{}'", r.name);
            }
        }
        for h in &r.helpers {
            let p = sk.index_of(&h.parent).ok_or_else(|| {
                anyhow::anyhow!(
                    "helper '{}' hangs under '{}', which the rig doesn't have",
                    h.name,
                    h.parent
                )
            })?;
            anyhow::ensure!(
                !is_helper_bone(sk, p),
                "helper '{}' hangs under another helper; hang it under a bone of the rig",
                h.name
            );
            if let Some(i) = sk.index_of(&h.name) {
                anyhow::ensure!(
                    is_helper_bone(sk, i),
                    "helper '{}' has the name of a bone of the rig",
                    h.name
                );
            }
        }
        if let Kind::Rbf(rbf) = &r.kind {
            rbf.solve(sk.bones[b].direction(), r.outputs())?;
            let along = sk.bones[b].direction();
            for (i, a) in rbf.poses.iter().enumerate() {
                for c in &rbf.poses[i + 1..] {
                    anyhow::ensure!(
                        rbf.distance(a.rotation, c.rotation, along) >= MIN_APART,
                        "reader '{}' has two sample poses within {MIN_APART} degree of each other",
                        r.name
                    );
                }
            }
        }
    }
    Ok(())
}

/// Put the helper joints of the rig's readers on its skeleton as they are now
/// (adding, moving or removing their bones) and work the skin weights out
/// again: automatic weights for the new skeleton, with hand edits kept.
pub fn reskin(rig: &mut Rig) -> anyhow::Result<()> {
    let opts = crate::skin::SkinOptions::default();
    let old = rig.clone();
    let old_auto = old.auto_weights(opts);
    let mut sk = Skeleton { bones: Vec::new() };
    // Keep the rig's own bones (indices shift once helpers go; parents follow).
    let keep: Vec<usize> = (0..old.skeleton.bones.len())
        .filter(|&i| !is_helper_bone(&old.skeleton, i))
        .collect();
    let new_index: HashMap<usize, usize> = keep.iter().enumerate().map(|(n, &o)| (o, n)).collect();
    for &o in &keep {
        let mut b = old.skeleton.bones[o].clone();
        b.parent = b.parent.and_then(|p| new_index.get(&p).copied());
        sk.bones.push(b);
    }
    let missing = add_helper_bones(&rig.mesh.readers, &mut sk);
    anyhow::ensure!(
        missing.is_empty(),
        "helper {} hangs under a bone the rig doesn't have",
        missing.join(", ")
    );
    rig.skeleton = sk;
    rig.skin = rig.auto_weights(opts);
    crate::rig::keep_weights(&old, Some(&old_auto), rig);
    Ok(())
}

/// Whether the skeleton's helper bones match the readers' helpers (names,
/// parents, places and radii), so no reskin is needed.
pub fn helpers_in_place(rig: &Rig) -> bool {
    let sk = &rig.skeleton;
    let want: Vec<&Helper> = helpers(&rig.mesh.readers).collect();
    let have: Vec<usize> = (0..sk.bones.len())
        .filter(|&i| is_helper_bone(sk, i))
        .collect();
    want.len() == have.len()
        && want.iter().all(|h| {
            sk.index_of(&h.name).is_some_and(|i| {
                let b = &sk.bones[i];
                is_helper_bone(sk, i)
                    && b.parent.map(|p| sk.bones[p].name.as_str()) == Some(h.parent.as_str())
                    && b.head.distance(h.head) < 1e-6
                    && b.reach == Some(h.radius)
            })
        })
}

/// Add or replace a reader (by name), check it, and reskin when its helper
/// joints changed. Nothing changes when it doesn't check out.
pub fn set_reader(rig: &mut Rig, reader: Reader) -> anyhow::Result<()> {
    let before = rig.mesh.readers.clone();
    match rig.mesh.readers.iter_mut().find(|r| r.name == reader.name) {
        Some(r) => *r = reader,
        None => rig.mesh.readers.push(reader),
    }
    let result = validate_on(rig).and_then(|_| {
        if helpers_in_place(rig) {
            Ok(())
        } else {
            reskin(rig)
        }
    });
    if result.is_err() {
        rig.mesh.readers = before;
    }
    result
}

/// Take `shape` off every reader that drives it (with its sample values).
pub fn release_shape(readers: &mut [Reader], shape: &str) {
    for r in readers {
        if let Some(k) = r.shape_index(shape) {
            r.shapes.remove(k);
            if let Kind::Rbf(rbf) = &mut r.kind {
                for p in &mut rbf.poses {
                    if k < p.values.len() {
                        p.values.remove(k);
                    }
                }
            }
        }
    }
}

/// A shape was renamed: the readers drive it by its new name.
pub fn rename_shape(readers: &mut [Reader], old: &str, new: &str) {
    for r in readers {
        for s in &mut r.shapes {
            if s == old {
                *s = new.to_string();
            }
        }
    }
}

pub fn remove_reader(rig: &mut Rig, name: &str) -> anyhow::Result<()> {
    let i = rig
        .mesh
        .readers
        .iter()
        .position(|r| r.name == name)
        .ok_or_else(|| anyhow::anyhow!("the rig has no pose reader '{name}'"))?;
    let had_helpers = !rig.mesh.readers[i].helpers.is_empty();
    rig.mesh.readers.remove(i);
    if had_helpers {
        reskin(rig)?;
    }
    Ok(())
}

/// Have reader `name` drive `shape` as well (making an empty shape when the
/// rig has none by that name). An RBF reader's samples give it `values` (one
/// per sample; 0 for any left out). Returns whether a shape was made.
pub fn add_shape(rig: &mut Rig, name: &str, shape: &str, values: &[f32]) -> anyhow::Result<bool> {
    let mut r = find(rig, name)?.clone();
    anyhow::ensure!(
        r.shape_index(shape).is_none(),
        "'{name}' drives '{shape}' already"
    );
    let made = !rig.mesh.shapes.iter().any(|s| s.name == shape);
    if made {
        anyhow::ensure!(!shape.trim().is_empty(), "the shape needs a name");
        rig.mesh.shapes.push(Shape::from_dense(shape, &[]));
    }
    let at = r.shapes.len();
    r.shapes.push(shape.to_string());
    if let Kind::Rbf(rbf) = &mut r.kind {
        for (k, p) in rbf.poses.iter_mut().enumerate() {
            p.values.insert(at, values.get(k).copied().unwrap_or(0.0));
        }
    }
    if let Err(e) = set_reader(rig, r) {
        if made {
            rig.mesh.shapes.retain(|s| s.name != shape);
        }
        return Err(e);
    }
    Ok(made)
}

/// Stop reader `name` driving `shape`; the shape stays, at its own weight.
pub fn remove_shape(rig: &mut Rig, name: &str, shape: &str) -> anyhow::Result<()> {
    let mut r = find(rig, name)?.clone();
    let k = r
        .shape_index(shape)
        .ok_or_else(|| anyhow::anyhow!("'{name}' doesn't drive '{shape}'"))?;
    r.shapes.remove(k);
    if let Kind::Rbf(rbf) = &mut r.kind {
        for p in &mut rbf.poses {
            p.values.remove(k);
        }
    }
    set_reader(rig, r)
}

/// Add a sample pose to RBF reader `name`: the bone's local rotation and the
/// value of each output named in `values` (outputs left out are 0). Shapes it
/// doesn't drive yet are added as outputs (and made empty if new). Returns the
/// sample's number (from 1).
pub fn add_sample(
    rig: &mut Rig,
    name: &str,
    rotation: Quat,
    values: &[(String, f32)],
) -> anyhow::Result<usize> {
    let before = rig.mesh.clone();
    let result = (|| {
        let r = find(rig, name)?;
        anyhow::ensure!(
            matches!(r.kind, Kind::Rbf(_)),
            "'{name}' is a cone reader; only RBF readers take sample poses"
        );
        for (out, _) in values {
            let r = find(rig, name)?;
            let known = r.shape_index(out).is_some() || r.helpers.iter().any(|h| &h.name == out);
            if !known {
                add_shape(rig, name, out, &[])?;
            }
        }
        let mut r = find(rig, name)?.clone();
        let mut v = vec![0.0; r.outputs()];
        for (out, x) in values {
            let k = r
                .shape_index(out)
                .or_else(|| {
                    r.helpers
                        .iter()
                        .position(|h| &h.name == out)
                        .map(|k| r.shapes.len() + k)
                })
                .ok_or_else(|| anyhow::anyhow!("'{name}' has no output '{out}'"))?;
            v[k] = *x;
        }
        let Kind::Rbf(rbf) = &mut r.kind else {
            unreachable!()
        };
        rbf.poses.push(Sample {
            rotation: rotation.normalize(),
            values: v,
        });
        let k = rbf.poses.len();
        set_reader(rig, r)?;
        Ok(k)
    })();
    if result.is_err() {
        rig.mesh = before;
    }
    result
}

/// Take sample `k` (from 1) off RBF reader `name`.
pub fn remove_sample(rig: &mut Rig, name: &str, k: usize) -> anyhow::Result<()> {
    let mut r = find(rig, name)?.clone();
    let Kind::Rbf(rbf) = &mut r.kind else {
        anyhow::bail!("'{name}' is a cone reader; it has no sample poses");
    };
    anyhow::ensure!(
        (1..=rbf.poses.len()).contains(&k),
        "'{name}' has {} sample poses",
        rbf.poses.len()
    );
    rbf.poses.remove(k - 1);
    set_reader(rig, r)
}

/// Set one value of sample `k` (from 1) of RBF reader `name`.
pub fn set_sample_value(
    rig: &mut Rig,
    name: &str,
    k: usize,
    output: &str,
    value: f32,
) -> anyhow::Result<()> {
    anyhow::ensure!(value.is_finite(), "the value must be a number");
    let mut r = find(rig, name)?.clone();
    let o = r
        .shape_index(output)
        .or_else(|| {
            r.helpers
                .iter()
                .position(|h| h.name == output)
                .map(|i| r.shapes.len() + i)
        })
        .ok_or_else(|| anyhow::anyhow!("'{name}' has no output '{output}'"))?;
    let Kind::Rbf(rbf) = &mut r.kind else {
        anyhow::bail!("'{name}' is a cone reader; it has no sample poses");
    };
    let p = rbf
        .poses
        .get_mut(k.wrapping_sub(1))
        .ok_or_else(|| anyhow::anyhow!("'{name}' has no sample pose {k}"))?;
    p.values[o] = value;
    set_reader(rig, r)
}

/// Add (or, by name, replace) a helper joint of reader `name`. A new helper
/// is on fully in the RBF samples given in `on` (numbers from 1) and off in
/// the others. The skin weights are worked out again, keeping hand edits.
pub fn set_helper(rig: &mut Rig, name: &str, helper: Helper, on: &[usize]) -> anyhow::Result<()> {
    let mut r = find(rig, name)?.clone();
    match r.helpers.iter_mut().find(|h| h.name == helper.name) {
        Some(h) => *h = helper,
        None => {
            r.helpers.push(helper);
            if let Kind::Rbf(rbf) = &mut r.kind {
                for (k, p) in rbf.poses.iter_mut().enumerate() {
                    p.values.push(if on.contains(&(k + 1)) { 1.0 } else { 0.0 });
                }
            }
        }
    }
    set_reader(rig, r)
}

pub fn remove_helper(rig: &mut Rig, name: &str, helper: &str) -> anyhow::Result<()> {
    let mut r = find(rig, name)?.clone();
    let k = r
        .helpers
        .iter()
        .position(|h| h.name == helper)
        .ok_or_else(|| anyhow::anyhow!("'{name}' has no helper '{helper}'"))?;
    let at = r.shapes.len() + k;
    r.helpers.remove(k);
    if let Kind::Rbf(rbf) = &mut r.kind {
        for p in &mut rbf.poses {
            p.values.remove(at);
        }
    }
    set_reader(rig, r)
}

/// A new helper for reader `reader` on `rig`: at the read bone's head, under
/// its parent (so an elbow's helper rides on the upper arm, not the forearm),
/// reaching a third of the parent bone's length.
pub fn default_helper(rig: &Rig, reader: &Reader, name: &str) -> anyhow::Result<Helper> {
    let sk = &rig.skeleton;
    let b = sk
        .bone(&reader.bone)
        .ok_or_else(|| anyhow::anyhow!("the rig has no bone '{}'", reader.bone))?;
    let parent = b.parent.map_or(b, |p| &sk.bones[p]);
    Ok(Helper {
        name: name.to_string(),
        parent: parent.name.clone(),
        head: b.head,
        offset: Vec3::ZERO,
        along: 1.0,
        across: 1.0,
        radius: (parent.length() / 3.0).max(0.01),
    })
}

/// The bone's local rotation in the rig's saved studio pose.
pub fn studio_rotation(rig: &Rig, bone: &str) -> anyhow::Result<Quat> {
    let i = rig
        .skeleton
        .index_of(bone)
        .ok_or_else(|| anyhow::anyhow!("the rig has no bone '{bone}'"))?;
    let posed = crate::pose::evaluate(&rig.skeleton, &rig.controls, &rig.pose);
    Ok(posed.locals()[i])
}

/// The mirror image of a rotation across the YZ plane: its axis mirrored as a
/// pseudo-vector (x, -y, -z), its angle kept.
pub fn mirror_rotation(q: Quat) -> Quat {
    Quat::from_xyzw(q.x, -q.y, -q.z, q.w)
}

/// Mirror reader `name` onto the other side as `new` (default: its name with
/// the side swapped): the twin bone, mirrored directions and samples, the
/// mirrored shapes (which must exist, see `shape mirror`) and helpers.
pub fn mirror_on_rig(rig: &mut Rig, name: &str, new: Option<&str>) -> anyhow::Result<String> {
    let src = find(rig, name)?.clone();
    let side = |s: &str| crate::shapes::other_side(s).unwrap_or_else(|| format!("{s}_mirror"));
    let new = new.map_or_else(|| side(name), str::to_string);
    anyhow::ensure!(
        rig.mesh.readers.iter().all(|r| r.name != new),
        "the rig already has a pose reader '{new}'"
    );
    let body = &rig.guides.body;
    let twins = crate::skin::bone_twins(&rig.skeleton, body.center_x, 0.05 * body.height);
    let twin = |bone: &str| {
        rig.skeleton
            .index_of(bone)
            .and_then(|i| twins.get(i))
            .map(|&t| rig.skeleton.bones[t].name.clone())
            .filter(|t| t != bone)
            .or_else(|| crate::shapes::other_side(bone))
            .unwrap_or_else(|| bone.to_string())
    };
    let flip = |v: Vec3| Vec3::new(-v.x, v.y, v.z);
    let mut out = src.clone();
    out.name = new.clone();
    out.bone = twin(&src.bone);
    match &mut out.kind {
        Kind::Cone(c) => c.direction = flip(c.direction),
        Kind::Rbf(r) => {
            for p in &mut r.poses {
                p.rotation = mirror_rotation(p.rotation);
            }
        }
    }
    out.shapes = src.shapes.iter().map(|s| side(s)).collect();
    for s in &out.shapes {
        anyhow::ensure!(
            rig.mesh.shapes.iter().any(|x| &x.name == s),
            "mirror the shapes first: the rig has no '{s}' (`shape mirror`)"
        );
    }
    for h in &mut out.helpers {
        h.name = side(&h.name);
        h.parent = twin(&h.parent);
        h.head = Vec3::new(2.0 * body.center_x - h.head.x, h.head.y, h.head.z);
        h.offset = flip(h.offset);
    }
    set_reader(rig, out)?;
    Ok(new)
}

/// Rename the bones the readers read and hang their helpers under, after a
/// rebuild renamed them (`renamed`: old name to new). Returns the readers whose
/// bone is gone; they are dropped.
pub fn rename_bones(
    readers: &mut Vec<Reader>,
    renamed: &HashMap<String, String>,
    has: impl Fn(&str) -> bool,
) -> Vec<String> {
    let mut lost = Vec::new();
    readers.retain_mut(|r| {
        if let Some(n) = renamed.get(&r.bone) {
            r.bone = n.clone();
        }
        for h in &mut r.helpers {
            if let Some(n) = renamed.get(&h.parent) {
                h.parent = n.clone();
            }
        }
        let keep = has(&r.bone) && r.helpers.iter().all(|h| has(&h.parent));
        if !keep {
            lost.push(r.name.clone());
        }
        keep
    });
    lost
}

/// Turn the readers' directions, samples and helpers by the rotation `m`.
pub fn turn(readers: &mut [Reader], m: Mat3) {
    let q = Quat::from_mat3(&m);
    for r in readers {
        match &mut r.kind {
            Kind::Cone(c) => c.direction = m * c.direction,
            Kind::Rbf(rbf) => {
                for p in &mut rbf.poses {
                    p.rotation = (q * p.rotation * q.inverse()).normalize();
                }
            }
        }
        for h in &mut r.helpers {
            h.head = m * h.head;
            h.offset = m * h.offset;
        }
    }
}

/// The readers as the builder scripts take them: each solved, with the read
/// bone's rest direction and each helper's parent direction.
pub fn export(rig: &Rig) -> Value {
    let sk = &rig.skeleton;
    let list: Vec<Value> = rig
        .mesh
        .readers
        .iter()
        .filter_map(|r| {
            let b = sk.bone(&r.bone)?;
            let along = b.direction();
            let helpers: Vec<Value> = r
                .helpers
                .iter()
                .filter_map(|h| {
                    let p = sk.bone(&h.parent)?;
                    Some(json!({
                        "name": h.name, "parent": h.parent, "head": h.head, "offset": h.offset,
                        "along": h.along, "across": h.across, "parent_dir": p.direction(),
                    }))
                })
                .collect();
            let mut v = json!({
                "name": r.name, "bone": r.bone, "kind": match r.kind { Kind::Cone(_) => "cone", Kind::Rbf(_) => "rbf" },
                "along": along, "shapes": r.shapes, "helpers": helpers,
            });
            match &r.kind {
                Kind::Cone(c) => {
                    v["direction"] = json!(c.direction.normalize_or_zero());
                    v["angle"] = json!(c.angle);
                    v["falloff"] = json!(c.falloff);
                    v["away"] = json!(c.away);
                }
                Kind::Rbf(rbf) => {
                    let s = rbf.solve(along, r.outputs()).ok()?;
                    v["swing"] = json!(s.swing);
                    v["radius"] = json!(s.radius);
                    v["centres"] = json!(s.centres.iter().map(|q| [q.x, q.y, q.z, q.w]).collect::<Vec<_>>());
                    v["directions"] = json!(s.directions);
                    v["weights"] = json!(s.weights);
                }
            }
            Some(v)
        })
        .collect();
    Value::Array(list)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn_deg(axis: Vec3, deg: f32) -> Quat {
        Quat::from_axis_angle(axis.normalize(), deg.to_radians())
    }

    /// On a built rig: a cone and an RBF reader drive their shapes, a helper
    /// joint joins the skeleton and the skin, poses move the skin through it,
    /// and mirroring, saving and removing keep the rig consistent.
    #[test]
    fn readers_on_a_rig_drive_shapes_and_a_skinned_helper() {
        let mut rig =
            crate::rig::auto_rig(crate::demo::demo_humanoid(), "demo", Default::default()).unwrap();
        let bones = rig.skeleton.bones.len();
        let lower = rig.skeleton.index_of("lowerarm_l").unwrap();
        let bend = turn_deg(Vec3::Y, 90.0);
        for s in ["elbow_fix_l", "shoulder_fix_l"] {
            rig.mesh
                .shapes
                .push(crate::shapes::Shape::from_dense(s, &[]));
        }
        set_reader(
            &mut rig,
            Reader {
                name: "elbow_l".into(),
                bone: "lowerarm_l".into(),
                kind: Kind::Rbf(Rbf {
                    poses: Vec::new(),
                    radius: 0.0,
                    swing: false,
                }),
                shapes: vec!["elbow_fix_l".into()],
                helpers: Vec::new(),
            },
        )
        .unwrap();
        assert_eq!(
            add_sample(&mut rig, "elbow_l", bend, &[("elbow_fix_l".into(), 1.0)]).unwrap(),
            1
        );
        // A shape the mesh lacks, and a second sample on the same pose, are refused.
        let near = turn_deg(Vec3::Y, 90.5);
        assert!(add_sample(&mut rig, "elbow_l", near, &[("nope".into(), 1.0)]).is_err());
        assert!(add_sample(&mut rig, "elbow_l", near, &[]).is_err());
        assert_eq!(rig.mesh.readers[0].outputs(), 1);
        let up = rig.skeleton.bones[rig.skeleton.index_of("upperarm_l").unwrap()].direction();
        set_reader(
            &mut rig,
            Reader {
                name: "shoulder_l".into(),
                bone: "upperarm_l".into(),
                kind: Kind::Cone(Cone {
                    direction: turn_deg(Vec3::Z, 60.0) * up,
                    angle: 10.0,
                    falloff: 20.0,
                    away: false,
                }),
                shapes: vec!["shoulder_fix_l".into()],
                helpers: Vec::new(),
            },
        )
        .unwrap();
        // A reader on a missing bone is refused and leaves the rig as it was.
        let mut bad = rig.mesh.readers[1].clone();
        bad.name = "bad".into();
        bad.bone = "nope".into();
        assert!(set_reader(&mut rig, bad).is_err());
        assert_eq!(rig.mesh.readers.len(), 2);

        let mut locals = vec![Quat::IDENTITY; bones];
        locals[lower] = bend;
        let out = outputs_in_pose(&rig.mesh.readers, &rig.skeleton, &locals);
        let weight = |s: &str| shape_weight(&rig.mesh.readers, &out, s).unwrap();
        assert!((weight("elbow_fix_l") - 1.0).abs() < 1e-3);
        assert!(weight("shoulder_fix_l") < 1e-3);

        // The helper: a deforming bone, weighted by the automatic skin.
        let mut h = default_helper(&rig, &rig.mesh.readers[0], "elbow_vol_l").unwrap();
        h.offset = Vec3::new(0.0, 0.0, -0.02);
        h.across = 1.3;
        set_helper(&mut rig, "elbow_l", h, &[1]).unwrap();
        assert!(helpers_in_place(&rig));
        let hi = rig.skeleton.index_of("elbow_vol_l").unwrap();
        assert_eq!(rig.skeleton.bones.len(), bones + 1);
        assert!(rig.skeleton.bones[hi].deform);
        let skinned: Vec<usize> = (0..rig.skin.joints.len())
            .filter(|&v| rig.skin.influences(v).any(|(j, w)| j == hi && w > 0.05))
            .collect();
        assert!(!skinned.is_empty());
        let mut locals = vec![Quat::IDENTITY; rig.skeleton.bones.len()];
        let rest = crate::shapes::posed_mesh(&rig, &locals);
        locals[rig.skeleton.index_of("lowerarm_l").unwrap()] = bend;
        let with = crate::shapes::posed_mesh(&rig, &locals);
        let held = {
            let mut r = rig.clone();
            r.mesh.readers[0].helpers.clear();
            crate::shapes::posed_mesh(&r, &locals)
        };
        let moved = skinned
            .iter()
            .map(|&v| with[v].distance(held[v]))
            .fold(0.0, f32::max);
        assert!(moved > 0.002, "the helper moves its skin by {moved}");
        assert!(skinned.iter().all(|&v| rest[v].is_finite()));

        // Mirroring (once its shape is mirrored) makes the right side's reader
        // and helper.
        assert!(mirror_on_rig(&mut rig, "elbow_l", None).is_err());
        rig.mesh
            .shapes
            .push(crate::shapes::Shape::from_dense("elbow_fix_r", &[]));
        mirror_on_rig(&mut rig, "elbow_l", None).unwrap();
        let r = find(&rig, "elbow_r").unwrap();
        assert_eq!(r.bone, "lowerarm_r");
        assert_eq!(r.helpers[0].name, "elbow_vol_r");
        assert!(rig.skeleton.index_of("elbow_vol_r").is_some());
        assert!(rig.mesh.validate().is_ok());

        // The rig file carries the readers; exported, each lists what it drives.
        let file: serde_json::Value =
            serde_json::from_slice(&crate::export::rig_file(&rig).unwrap()).unwrap();
        assert_eq!(file["pose_readers"].as_array().unwrap().len(), 3);
        let back: Rig = serde_json::from_value(file).unwrap();
        assert_eq!(back.mesh.readers, rig.mesh.readers);

        remove_reader(&mut rig, "elbow_r").unwrap();
        remove_reader(&mut rig, "elbow_l").unwrap();
        assert_eq!(rig.skeleton.bones.len(), bones);
        assert!(helpers_in_place(&rig));
    }

    #[test]
    fn a_cone_reads_swing_in_any_direction_and_ignores_twist() {
        let along = Vec3::X;
        let c = Cone {
            direction: Vec3::X,
            angle: 10.0,
            falloff: 40.0,
            away: true,
        };
        // At rest the bone points down the cone: off.
        assert_eq!(c.weight(along), 0.0);
        // 30 degrees away in two different directions give the same weight.
        let up = c.weight(turn_deg(Vec3::Z, 30.0) * along);
        let fwd = c.weight(turn_deg(Vec3::Y, -30.0) * along);
        assert!(
            (up - fwd).abs() < 1e-5 && up > 0.0 && up < 1.0,
            "{up} {fwd}"
        );
        assert!((up - smoothstep(10.0, 50.0, 30.0)).abs() < 1e-4);
        // Twisting about its own length changes nothing.
        let twisted = turn_deg(Vec3::Z, 30.0) * turn_deg(Vec3::X, 70.0);
        assert!((c.weight(twisted * along) - up).abs() < 1e-5);
        // Past angle + falloff it is fully on; the toward cone is the complement.
        assert_eq!(c.weight(turn_deg(Vec3::Z, 60.0) * along), 1.0);
        let toward = Cone { away: false, ..c };
        assert!((toward.weight(turn_deg(Vec3::Z, 30.0) * along) - (1.0 - up)).abs() < 1e-5);
        assert_eq!(toward.weight(turn_deg(Vec3::Z, 5.0) * along), 1.0);
    }

    #[test]
    fn rbf_hits_every_sample_and_rest_is_off() {
        let along = Vec3::X;
        let poses = [
            (turn_deg(Vec3::Z, 90.0), vec![1.0, 0.0]),
            (turn_deg(Vec3::Y, 90.0), vec![0.0, 1.0]),
            (turn_deg(Vec3::Z, -60.0), vec![0.5, 0.5]),
        ];
        for swing in [false, true] {
            let rbf = Rbf {
                poses: poses
                    .iter()
                    .map(|(q, v)| Sample {
                        rotation: *q,
                        values: v.clone(),
                    })
                    .collect(),
                radius: 0.0,
                swing,
            };
            let s = rbf.solve(along, 2).unwrap();
            assert_eq!(s.centres.len(), 4, "rest is a centre of its own");
            for (q, v) in &poses {
                let got = s.eval(*q);
                assert!(
                    (got[0] - v[0]).abs() < 1e-3 && (got[1] - v[1]).abs() < 1e-3,
                    "{got:?} {v:?}"
                );
                // -q is the same rotation.
                let neg = s.eval(-*q);
                assert!((neg[0] - got[0]).abs() < 1e-4);
            }
            assert!(s.eval(Quat::IDENTITY).iter().all(|x| x.abs() < 1e-3));
            // Half way to a sample, part way on.
            let half = s.eval(turn_deg(Vec3::Z, 45.0))[0];
            assert!(half > 0.05 && half < 0.95, "{half}");
        }
        // With swing, a twist doesn't move it; without, it does.
        let mk = |swing| Rbf {
            poses: vec![Sample {
                rotation: turn_deg(Vec3::Z, 90.0),
                values: vec![1.0],
            }],
            radius: 40.0,
            swing,
        };
        let twisted = turn_deg(Vec3::Z, 90.0) * turn_deg(Vec3::X, 60.0);
        let s = mk(true).solve(along, 1).unwrap();
        assert!((s.eval(twisted)[0] - 1.0).abs() < 1e-3);
        let s = mk(false).solve(along, 1).unwrap();
        assert!(s.eval(twisted)[0] < 0.5);
    }

    #[test]
    fn solving_matches_a_hand_worked_pair() {
        // One sample 60 degrees from rest, radius 60: Phi = [[1, e^-1], [e^-1, 1]].
        let rbf = Rbf {
            poses: vec![Sample {
                rotation: turn_deg(Vec3::Z, 60.0),
                values: vec![1.0],
            }],
            radius: 60.0,
            swing: false,
        };
        let s = rbf.solve(Vec3::X, 1).unwrap();
        let e = (-1.0f32).exp();
        // [w0, w1] = Phi^-1 [0, 1] = [-e, 1] / (1 - e^2).
        assert!((s.weights[0][0] + e / (1.0 - e * e)).abs() < 1e-4);
        assert!((s.weights[1][0] - 1.0 / (1.0 - e * e)).abs() < 1e-4);
    }

    #[test]
    fn helpers_move_and_scale_about_their_head_and_ride_on_the_parent() {
        let h = Helper {
            name: "h".into(),
            parent: "p".into(),
            head: Vec3::new(1.0, 0.0, 0.0),
            offset: Vec3::new(0.0, 0.1, 0.0),
            along: 1.0,
            across: 2.0,
            radius: 0.2,
        };
        let m = helper_matrix(Mat4::IDENTITY, &h, Vec3::X, 1.0);
        // The head moves by the offset; a point across the bone doubles its distance.
        assert!(
            m.transform_point3(h.head)
                .distance(Vec3::new(1.0, 0.1, 0.0))
                < 1e-6
        );
        assert!(
            m.transform_point3(Vec3::new(1.0, 0.0, 0.1))
                .distance(Vec3::new(1.0, 0.1, 0.2))
                < 1e-6
        );
        // Along the bone it keeps its length.
        assert!(
            m.transform_point3(Vec3::new(1.1, 0.0, 0.0))
                .distance(Vec3::new(1.1, 0.1, 0.0))
                < 1e-6
        );
        // At weight 0 it just follows its parent.
        let p = Mat4::from_rotation_z(0.5);
        assert!(helper_matrix(p, &h, Vec3::X, 0.0).abs_diff_eq(p, 1e-6));
    }

    #[test]
    fn readers_read_back_from_json() {
        let r = Reader {
            name: "elbow".into(),
            bone: "lowerarm_l".into(),
            kind: Kind::Rbf(Rbf {
                poses: vec![Sample {
                    rotation: turn_deg(Vec3::Y, 90.0),
                    values: vec![1.0],
                }],
                radius: 0.0,
                swing: true,
            }),
            shapes: vec!["elbow_fix".into()],
            helpers: Vec::new(),
        };
        let text = serde_json::to_string(&r).unwrap();
        assert!(text.contains("\"kind\":\"rbf\""), "{text}");
        assert_eq!(serde_json::from_str::<Reader>(&text).unwrap(), r);
        let c: Reader = serde_json::from_str(
            r#"{"name":"c","bone":"b","kind":"cone","direction":[1,0,0],"angle":10,"falloff":20}"#,
        )
        .unwrap();
        assert!(matches!(c.kind, Kind::Cone(Cone { away: false, .. })));
    }
}
