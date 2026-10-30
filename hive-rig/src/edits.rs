//! Edits made in another program, brought back into the rig: skin weights
//! painted in Blender or Maya and shapes sculpted there, as the small JSON
//! file the builder scripts' "Send to Hive" tools write.
//!
//! ```json
//! {
//!   "hive_rig_edits": 1,
//!   "rig": "hero",
//!   "source": "blender",
//!   "vertex_count": 2676,
//!   "weights": [{"upperarm_l": 0.75, "clavicle_l": 0.25}, ...],
//!   "shapes": [{
//!     "name": "elbow_fix_l", "weight": 0.0,
//!     "vertices": [12, 13], "offsets": [[0, 0.01, 0], [0, 0.02, 0]],
//!     "inbetweens": [{"weight": 0.5, "vertices": [12], "offsets": [[0, 0.004, 0]]}]
//!   }]
//! }
//! ```
//!
//! `weights` (one entry per vertex, bones by the rig's own names) and
//! `shapes` (offsets in metres in hive-rig's frame, Y up and facing +Z, at
//! rest) are both optional. Vertices are hive-rig's, in its order: the
//! builders keep that order, so a mesh whose vertices were added or removed
//! in the other program can't come back.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use glam::{DMat3, DVec3, Vec3};
use serde::{Deserialize, Serialize};

use crate::import::fbx::FbxScene;
use crate::rig::Rig;
use crate::shapes::{Inbetween, Shape};
use crate::skin;

/// The file format's version.
pub const VERSION: u32 = 1;

/// Weights within this of the rig's own count as unchanged (the other
/// program stores them in single precision and may renormalise).
const SAME_WEIGHT: f32 = 1e-4;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Edits {
    #[serde(rename = "hive_rig_edits")]
    pub version: u32,
    /// The rig the edits were made on.
    #[serde(default)]
    pub rig: String,
    /// The program that wrote them ("blender", "maya").
    #[serde(default)]
    pub source: String,
    pub vertex_count: usize,
    /// Each vertex's weights by bone name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weights: Option<Vec<BTreeMap<String, f32>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shapes: Option<Vec<ShapeEdit>>,
    /// How far a vertex's weights may be from the rig's and still count as
    /// unchanged, for a program that stores them coarsely (Unreal keeps 8-bit
    /// weights unless told otherwise). At least 1e-4.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight_tolerance: Option<f32>,
}

/// A shape as the other program has it.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ShapeEdit {
    pub name: String,
    /// Its slider; left alone on a shape a pose reader drives.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weight: Option<f32>,
    pub vertices: Vec<u32>,
    pub offsets: Vec<Vec3>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inbetweens: Vec<Inbetween>,
}

impl Edits {
    pub fn read(path: &Path) -> Result<Edits> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let edits: Edits = serde_json::from_str(&text)
            .with_context(|| format!("{} isn't a hive-rig edits file", path.display()))?;
        anyhow::ensure!(
            edits.version >= 1,
            "{} has no hive_rig_edits version",
            path.display()
        );
        if edits.version > VERSION {
            eprintln!(
                "warning: {} is edits format {}; this hive-rig reads {VERSION} and ignores what it doesn't know",
                path.display(),
                edits.version
            );
        }
        Ok(edits)
    }

    pub fn write(&self, path: &Path) -> Result<()> {
        std::fs::write(path, serde_json::to_string(self)?)
            .with_context(|| format!("writing {}", path.display()))
    }

    /// The rig's own weights and shapes in this format (what a builder's
    /// "Send to Hive" writes for an unedited rig).
    pub fn from_rig(rig: &Rig, source: &str) -> Edits {
        let names: Vec<&str> = rig.skeleton.bones.iter().map(|b| b.name.as_str()).collect();
        let weights = (0..rig.skin.vertex_count())
            .map(|v| {
                rig.skin
                    .influences(v)
                    .map(|(j, w)| (names[j].to_string(), w))
                    .collect()
            })
            .collect();
        let shapes = rig
            .mesh
            .shapes
            .iter()
            .map(|s| ShapeEdit {
                name: s.name.clone(),
                weight: s.driver.is_none().then_some(s.weight),
                vertices: s.vertices.clone(),
                offsets: s.offsets.clone(),
                inbetweens: s.inbetweens.clone(),
            })
            .collect();
        Edits {
            version: VERSION,
            rig: rig.name.clone(),
            source: source.to_string(),
            vertex_count: rig.mesh.vertex_count(),
            weights: Some(weights),
            shapes: Some(shapes),
            weight_tolerance: None,
        }
    }

    fn check_vertices(&self, rig: &Rig) -> Result<()> {
        anyhow::ensure!(
            self.vertex_count == rig.mesh.vertex_count(),
            "the edits are for a mesh of {} vertices; '{}' has {} (vertices added or removed in {} can't come back)",
            self.vertex_count,
            rig.name,
            rig.mesh.vertex_count(),
            if self.source.is_empty() {
                "the other program"
            } else {
                &self.source
            }
        );
        Ok(())
    }
}

/// What importing weights did.
#[derive(Debug, Default, PartialEq)]
pub struct WeightReport {
    /// Vertices whose weights changed.
    pub changed: usize,
    /// Bone names the rig has no deforming bone for; their weight went to the
    /// vertex's other bones.
    pub unknown: Vec<String>,
    /// Vertices left as they were because the file gave them no weight on
    /// any of the rig's bones.
    pub empty: usize,
}

/// Put the file's weights on the rig, within its influence limits. Vertices
/// whose weights match the rig's (to 1e-4) keep the rig's exactly.
pub fn import_weights(rig: &mut Rig, edits: &Edits) -> Result<WeightReport> {
    edits.check_vertices(rig)?;
    let Some(weights) = &edits.weights else {
        anyhow::bail!("the edits file has no weights");
    };
    anyhow::ensure!(
        weights.len() == rig.mesh.vertex_count(),
        "the edits file has weights for {} vertices; the mesh has {}",
        weights.len(),
        rig.mesh.vertex_count()
    );
    let index: BTreeMap<&str, usize> = rig
        .skeleton
        .bones
        .iter()
        .enumerate()
        .filter(|(_, b)| b.deform)
        .map(|(i, b)| (b.name.as_str(), i))
        .collect();
    let same_weight = edits.weight_tolerance.unwrap_or(0.0).max(SAME_WEIGHT);
    let mut report = WeightReport::default();
    let mut unknown = std::collections::BTreeSet::new();
    let mut out = rig.skin.clone();
    for (v, given) in weights.iter().enumerate() {
        let mut pairs: Vec<(u16, f32)> = Vec::new();
        for (name, &w) in given {
            if !w.is_finite() || w <= 0.0 {
                continue;
            }
            match index.get(name.as_str()) {
                Some(&j) => pairs.push((j as u16, w)),
                None => {
                    unknown.insert(name.clone());
                }
            }
        }
        let total: f32 = pairs.iter().map(|p| p.1).sum();
        if total <= 1e-6 {
            report.empty += 1;
            continue;
        }
        for p in &mut pairs {
            p.1 /= total;
        }
        let mut probe = skin::SkinWeights {
            joints: vec![Vec::new()],
            weights: vec![Vec::new()],
        };
        probe.set(0, pairs);
        let probe = skin::limit(&probe, rig.guides.skin);
        let new: Vec<(u16, f32)> = probe.influences(0).map(|(j, w)| (j as u16, w)).collect();
        // Unchanged when every bone either side weighs the same to the
        // tolerance (a bone missing on one side weighs 0 there).
        let same = {
            let old: Vec<(usize, f32)> = rig.skin.influences(v).collect();
            let weight_of = |list: &[(usize, f32)], j: usize| {
                list.iter().find(|p| p.0 == j).map_or(0.0, |p| p.1)
            };
            let new: Vec<(usize, f32)> = new.iter().map(|&(j, w)| (j as usize, w)).collect();
            old.iter()
                .chain(&new)
                .all(|&(j, _)| (weight_of(&old, j) - weight_of(&new, j)).abs() <= same_weight)
        };
        if !same {
            out.set(v, new);
            report.changed += 1;
        }
    }
    report.unknown = unknown.into_iter().collect();
    rig.skin = out;
    Ok(report)
}

/// What importing shapes did.
#[derive(Debug, Default, PartialEq)]
pub struct ShapeReport {
    /// Shapes the rig had, now as the file has them.
    pub updated: Vec<String>,
    /// Shapes the rig didn't have.
    pub added: Vec<String>,
}

/// Put the file's shapes on the rig: a shape it has takes the file's offsets
/// and in-betweens (keeping its pose reader; its slider weight too when a
/// reader drives it), a new one is added. With `only`, just those shapes.
pub fn import_shapes(rig: &mut Rig, edits: &Edits, only: &[String]) -> Result<ShapeReport> {
    edits.check_vertices(rig)?;
    let Some(shapes) = &edits.shapes else {
        anyhow::bail!("the edits file has no shapes");
    };
    for name in only {
        anyhow::ensure!(
            shapes.iter().any(|s| &s.name == name),
            "the edits file has no shape '{name}'"
        );
    }
    let n = rig.mesh.vertex_count();
    let mut report = ShapeReport::default();
    for e in shapes {
        if !only.is_empty() && !only.contains(&e.name) {
            continue;
        }
        anyhow::ensure!(
            e.vertices.len() == e.offsets.len(),
            "shape '{}' needs one offset per vertex",
            e.name
        );
        // Sparse and sorted, whatever order the file lists them in.
        let dense = |vertices: &[u32], offsets: &[Vec3]| -> Result<Vec<Vec3>> {
            let mut out = vec![Vec3::ZERO; n];
            for (&v, d) in vertices.iter().zip(offsets) {
                let slot = out.get_mut(v as usize).ok_or_else(|| {
                    anyhow::anyhow!("shape '{}' moves vertex {v}, past the mesh", e.name)
                })?;
                *slot = *d;
            }
            Ok(out)
        };
        let mut made = Shape::from_dense(&e.name, &dense(&e.vertices, &e.offsets)?);
        let mut ibs = e.inbetweens.clone();
        ibs.sort_by(|a, b| a.weight.total_cmp(&b.weight));
        for b in ibs {
            made.inbetweens.push(Inbetween::from_dense(
                (b.weight * 100.0).round() / 100.0,
                &dense(&b.vertices, &b.offsets)?,
            ));
        }
        match rig.mesh.shapes.iter_mut().find(|s| s.name == e.name) {
            Some(s) => {
                let moved = s.vertices != made.vertices || s.offsets != made.offsets;
                s.vertices = made.vertices;
                s.offsets = made.offsets;
                s.inbetweens = made.inbetweens;
                // The source's normal offsets no longer match a resculpted shape.
                if moved {
                    s.normal_offsets.clear();
                }
                if s.driver.is_none()
                    && let Some(w) = e.weight
                {
                    s.weight = w;
                }
                s.validate(n)?;
                report.updated.push(e.name.clone());
            }
            None => {
                made.weight = e.weight.unwrap_or(0.0);
                made.validate(n)?;
                rig.mesh.shapes.push(made);
                report.added.push(e.name.clone());
            }
        }
    }
    Ok(report)
}

/// How a skinned mesh another program exported lined up with the rig.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SceneMatch {
    /// Bones found in both, by name, that placed the file's frame.
    pub bones: usize,
    /// The farthest a matched bone sits from the rig's, once placed (metres).
    pub bone_error: f32,
    /// Rig vertices given a vertex of the file, and the farthest such pair.
    pub matched: usize,
    pub vertex_error: f32,
    /// Rig vertices with no vertex of the file at their place: they keep the
    /// rig's weights and shapes.
    pub unmatched: usize,
}

/// How far (as a share of the body height) a rig vertex may sit from the
/// file's vertex it takes its edits from.
const MATCH_SHARE: f32 = 1e-3;

/// The map from a file's frame onto hive-rig's, `hive = a * file + t`, fitted
/// to the same points in both (bone heads). Points spread in all three
/// directions give the best affine fit; points in a plane (a snake's
/// skeleton) give a turn and scale that keep handedness, as an export
/// from a program that read the rig's own .glb or .fbx does.
fn fit_frame(pairs: &[(DVec3, DVec3)]) -> Option<(DMat3, DVec3)> {
    if pairs.len() < 3 {
        return None;
    }
    let n = pairs.len() as f64;
    let cs = pairs.iter().map(|p| p.0).sum::<DVec3>() / n;
    let ch = pairs.iter().map(|p| p.1).sum::<DVec3>() / n;
    let outer = |a: DVec3, b: DVec3| DMat3::from_cols(a * b.x, a * b.y, a * b.z);
    let (mut ss, mut hs) = (DMat3::ZERO, DMat3::ZERO);
    for &(s, h) in pairs {
        ss += outer(s - cs, s - cs);
        hs += outer(h - ch, s - cs);
    }
    let size = (ss.x_axis.x + ss.y_axis.y + ss.z_axis.z) / 3.0;
    if size <= 0.0 {
        return None;
    }
    let a = if ss.determinant() > 1e-6 * size * size * size {
        hs * ss.inverse()
    } else {
        // In a plane: the two spokes from the centre spanning the most, and
        // their cross product, scaled from the file's size to the rig's.
        let spokes: Vec<(DVec3, DVec3)> = pairs.iter().map(|&(s, h)| (s - cs, h - ch)).collect();
        let first = spokes
            .iter()
            .copied()
            .max_by(|a, b| a.0.length_squared().total_cmp(&b.0.length_squared()))?;
        let second = spokes.iter().copied().max_by(|a, b| {
            a.0.cross(first.0)
                .length_squared()
                .total_cmp(&b.0.cross(first.0).length_squared())
        })?;
        let (u1, u2) = (first.1, second.1);
        let (v1, v2) = (first.0, second.0);
        let scale = v1.length() / u1.length().max(1e-12);
        let u = DMat3::from_cols(u1, u2, u1.cross(u2));
        let v = DMat3::from_cols(v1, v2, v1.cross(v2) / scale.max(1e-12));
        if v.determinant().abs() < 1e-12 * size * size * size {
            return None;
        }
        u * v.inverse()
    };
    Some((a, ch - a * cs))
}

/// The skin and shapes of a skinned mesh that another program exported (an
/// FBX Unreal wrote from the imported rig, say) as edits of `rig`. The file
/// may have its own frame, units and vertex order and split or weld vertices:
/// its frame is found from the bones both share (by name), and each rig
/// vertex takes the weights and shape offsets of the file's vertex at its
/// place. Weights within `weight_tolerance` of the rig's count as unchanged;
/// shape offsets within 0.01 mm of the rig's are the rig's.
pub fn from_scene(
    rig: &Rig,
    scene: &FbxScene,
    source: &str,
    weight_tolerance: Option<f32>,
) -> Result<(Edits, SceneMatch)> {
    let bare = |name: &str| name.rsplit(':').next().unwrap_or(name).to_string();
    let pairs: Vec<(DVec3, DVec3)> = scene
        .joints
        .iter()
        .filter_map(|j| {
            let b = rig.skeleton.bone(&bare(&j.name))?;
            Some((j.position.as_dvec3(), b.head.as_dvec3()))
        })
        .collect();
    let (a, t) = fit_frame(&pairs).ok_or_else(|| {
        anyhow::anyhow!(
            "the file shares only {} bones with '{}' that don't span a plane; it needs the rig's skeleton",
            pairs.len(),
            rig.name
        )
    })?;
    let height = rig.guides.body.height.max(1e-3);
    let mut report = SceneMatch {
        bones: pairs.len(),
        ..Default::default()
    };
    for &(s, h) in &pairs {
        report.bone_error = report.bone_error.max((a * s + t - h).length() as f32);
    }
    anyhow::ensure!(
        report.bone_error <= 0.01 * height,
        "the file's bones sit up to {:.1} cm from the rig's after lining them up: export it in the rest (reference) pose",
        report.bone_error * 100.0
    );

    // The file's vertices in hive-rig's frame, in a grid of cells the size
    // of the match distance.
    let reach = MATCH_SHARE * height;
    let cell = |p: Vec3| (p / reach).floor().as_ivec3();
    let placed: Vec<Vec3> = scene
        .mesh
        .positions
        .iter()
        .map(|p| (a * p.as_dvec3() + t).as_vec3())
        .collect();
    let mut grid: std::collections::HashMap<glam::IVec3, Vec<usize>> = Default::default();
    for (i, &p) in placed.iter().enumerate() {
        grid.entry(cell(p)).or_default().push(i);
    }
    // The file's vertices at each rig vertex's place (all those as near as
    // the nearest, to rounding).
    let tie = 0.01 * reach;
    let near: Vec<Vec<(usize, f32)>> = rig
        .mesh
        .positions
        .iter()
        .map(|&p| {
            let c = cell(p);
            let mut near: Vec<(usize, f32)> = Vec::new();
            for dx in -1..=1 {
                for dy in -1..=1 {
                    for dz in -1..=1 {
                        for &i in grid
                            .get(&(c + glam::IVec3::new(dx, dy, dz)))
                            .into_iter()
                            .flatten()
                        {
                            let d = placed[i].distance(p);
                            if d <= reach {
                                near.push((i, d));
                            }
                        }
                    }
                }
            }
            let closest = near.iter().map(|n| n.1).fold(f32::INFINITY, f32::min);
            near.retain(|n| n.1 <= closest + tie);
            near.sort_by_key(|n| n.0);
            near
        })
        .collect();
    // Vertices sharing a place (a seam's copies, or several surfaces meeting
    // at a point) are paired off one to one, each rig vertex with the file's
    // vertex whose skin is most like its own and that faces most its way, so
    // a copy repainted alone keeps its paint and its twins keep theirs.
    let names: Vec<String> = scene.joints.iter().map(|j| bare(&j.name)).collect();
    let normal_map = a.inverse().transpose();
    let file_normal = |i: usize| {
        scene
            .mesh
            .normals
            .get(i)
            .map(|n| (normal_map * n.as_dvec3()).as_vec3().normalize_or_zero())
    };
    let cost = |v: usize, i: usize| -> f32 {
        let theirs = scene.skin.get(i).map(Vec::as_slice).unwrap_or(&[]);
        let mut gap = 0.0;
        let mut seen = 0.0;
        for (j, w) in rig.skin.influences(v) {
            let name = &rig.skeleton.bones[j].name;
            let x: f32 = theirs
                .iter()
                .filter(|p| &names[p.0] == name)
                .map(|p| p.1)
                .sum();
            gap += (w - x).abs();
            seen += x;
        }
        let total: f32 = theirs.iter().map(|p| p.1).sum();
        let turn = match (rig.mesh.normals.get(v), file_normal(i)) {
            (Some(a), Some(b)) => 1.0 - a.dot(b),
            _ => 0.0,
        };
        gap + (total - seen).max(0.0) + turn
    };
    let mut matches: Vec<Option<usize>> = vec![None; near.len()];
    let mut groups: std::collections::BTreeMap<Vec<usize>, Vec<usize>> = Default::default();
    for (v, n) in near.iter().enumerate() {
        if !n.is_empty() {
            groups
                .entry(n.iter().map(|x| x.0).collect())
                .or_default()
                .push(v);
        }
    }
    for (files, rigs) in &groups {
        let mut pairs: Vec<(f32, usize, usize)> = rigs
            .iter()
            .flat_map(|&v| files.iter().map(move |&i| (v, i)))
            .map(|(v, i)| (cost(v, i), v, i))
            .collect();
        pairs.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        // One to one while the file has enough copies; then the best left.
        let mut taken = std::collections::BTreeSet::new();
        for &(_, v, i) in &pairs {
            if matches[v].is_none() && (files.len() < rigs.len() || !taken.contains(&i)) {
                matches[v] = Some(i);
                taken.insert(i);
            }
        }
    }
    for (v, m) in matches.iter().enumerate() {
        if let Some(i) = m {
            let d = near[v].iter().find(|n| n.0 == *i).map_or(0.0, |n| n.1);
            report.vertex_error = report.vertex_error.max(d);
        }
    }
    report.matched = matches.iter().flatten().count();
    report.unmatched = matches.len() - report.matched;
    anyhow::ensure!(
        report.matched > 0,
        "no vertex of the file sits where one of '{}' does: is it the rig's mesh?",
        rig.name
    );

    let weights = matches
        .iter()
        .map(|m| {
            let mut w = BTreeMap::new();
            for &(j, x) in m.and_then(|i| scene.skin.get(i)).into_iter().flatten() {
                *w.entry(names[j].clone()).or_insert(0.0) += x;
            }
            w
        })
        .collect();

    // A shape's offsets at the rig's vertices, turned into hive-rig's frame;
    // an offset (nearly) the rig's own stays exactly that.
    let n = rig.mesh.vertex_count();
    let take = |file: &[Vec3], own: Option<Vec<Vec3>>| -> (Vec<u32>, Vec<Vec3>) {
        let own = own.unwrap_or_else(|| vec![Vec3::ZERO; n]);
        let mut out = (Vec::new(), Vec::new());
        for (v, m) in matches.iter().enumerate() {
            let d = match m {
                Some(i) => {
                    let d = (a * file[*i].as_dvec3()).as_vec3();
                    if d.distance(own[v]) <= 1e-5 {
                        own[v]
                    } else {
                        d
                    }
                }
                None => own[v],
            };
            if d.length() > crate::shapes::EPSILON {
                out.0.push(v as u32);
                out.1.push(d);
            }
        }
        out
    };
    let count = scene.mesh.vertex_count();
    let shapes = scene
        .mesh
        .shapes
        .iter()
        .map(|s| {
            let own = rig.mesh.shapes.iter().find(|r| r.name == s.name);
            let (vertices, offsets) = take(&s.dense(count), own.map(|r| r.dense(n)));
            let inbetweens = s
                .inbetweens
                .iter()
                .map(|b| {
                    let mine = own.and_then(|r| {
                        r.inbetweens
                            .iter()
                            .find(|x| (x.weight - b.weight).abs() < 5e-3)
                    });
                    let (vertices, offsets) = take(&b.dense(count), mine.map(|x| x.dense(n)));
                    Inbetween {
                        weight: b.weight,
                        vertices,
                        offsets,
                    }
                })
                .collect();
            ShapeEdit {
                name: s.name.clone(),
                // The file's morph weights are its own sliders, not the rig's.
                weight: None,
                vertices,
                offsets,
                inbetweens,
            }
        })
        .collect();
    Ok((
        Edits {
            version: VERSION,
            rig: rig.name.clone(),
            source: source.to_string(),
            vertex_count: n,
            weights: Some(weights),
            shapes: Some(shapes),
            weight_tolerance,
        },
        report,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo() -> Rig {
        crate::rig::auto_rig(crate::demo::demo_humanoid(), "demo", Default::default()).unwrap()
    }

    #[test]
    fn weights_come_back_from_a_file_and_only_changed_vertices_move() {
        let mut rig = demo();
        let mut edits = Edits::from_rig(&rig, "blender");
        // Unchanged: nothing moves, even through a JSON round trip.
        let path = std::env::temp_dir().join(format!("hive-edits-{}.json", std::process::id()));
        edits.write(&path).unwrap();
        let read = Edits::read(&path).unwrap();
        std::fs::remove_file(&path).ok();
        let r = import_weights(&mut rig, &read).unwrap();
        assert_eq!(r.changed, 0);
        // Repaint one vertex onto the head, with an unknown group and a
        // weight that doesn't add up to 1.
        let w = edits.weights.as_mut().unwrap();
        w[7] = BTreeMap::from([("head".to_string(), 2.0), ("not_a_bone".to_string(), 1.0)]);
        w[8] = BTreeMap::new();
        let before = rig.skin.clone();
        let r = import_weights(&mut rig, &edits).unwrap();
        assert_eq!(r.changed, 1);
        assert_eq!(r.unknown, vec!["not_a_bone".to_string()]);
        assert_eq!(r.empty, 1);
        let head = rig.skeleton.index_of("head").unwrap();
        assert!((rig.skin.weight(7, head) - 1.0).abs() < 1e-6);
        assert_eq!(rig.skin.joints[8], before.joints[8]);
        // A different mesh is refused.
        edits.vertex_count += 1;
        assert!(import_weights(&mut rig, &edits).is_err());
    }

    #[test]
    fn shapes_come_back_with_inbetweens_keeping_their_drivers() {
        let mut rig = demo();
        let n = rig.mesh.vertex_count();
        let mut dense = vec![Vec3::ZERO; n];
        dense[2] = Vec3::Y * 0.01;
        let mut s = Shape::from_dense("fix", &dense);
        s.driver = Some(crate::shapes::PoseReader {
            bone: "lowerarm_l".into(),
            axis: Vec3::Z,
            start: 0.0,
            end: 90.0,
        });
        rig.mesh.shapes.push(s);
        let edits = Edits {
            version: 1,
            rig: "demo".into(),
            source: "maya".into(),
            vertex_count: n,
            weights: None,
            shapes: Some(vec![
                ShapeEdit {
                    name: "fix".into(),
                    weight: Some(0.7),
                    vertices: vec![5, 2],
                    offsets: vec![Vec3::X * 0.02, Vec3::Y * 0.03],
                    inbetweens: vec![Inbetween {
                        weight: 0.5,
                        vertices: vec![2],
                        offsets: vec![Vec3::Y * 0.01],
                    }],
                },
                ShapeEdit {
                    name: "smile".into(),
                    weight: Some(0.4),
                    vertices: vec![9],
                    offsets: vec![Vec3::Z * 0.01],
                    inbetweens: Vec::new(),
                },
            ]),
            weight_tolerance: None,
        };
        let r = import_shapes(&mut rig, &edits, &[]).unwrap();
        assert_eq!(r.updated, vec!["fix".to_string()]);
        assert_eq!(r.added, vec!["smile".to_string()]);
        let fix = crate::shapes::find(&rig, "fix").unwrap();
        assert_eq!(fix.vertices, vec![2, 5]);
        assert_eq!(fix.inbetweens.len(), 1);
        assert!(fix.driver.is_some());
        // A driven shape's slider stays as it was.
        assert_eq!(fix.weight, 0.0);
        assert_eq!(crate::shapes::find(&rig, "smile").unwrap().weight, 0.4);
        assert!(import_shapes(&mut rig, &edits, &["nope".to_string()]).is_err());
        rig.mesh.validate().unwrap();
        // And the rig's own shapes in the file format read back the same.
        let again = Edits::from_rig(&rig, "test");
        let mut copy = rig.clone();
        import_shapes(&mut copy, &again, &[]).unwrap();
        assert_eq!(copy.mesh.shapes, rig.mesh.shapes);
    }

    /// The demo with a shape carrying an in-between, as the rig and its FBX.
    fn shaped() -> Rig {
        let mut rig = demo();
        let n = rig.mesh.vertex_count();
        let (mut full, mut half) = (vec![Vec3::ZERO; n], vec![Vec3::ZERO; n]);
        for v in (0..n).step_by(97) {
            full[v] = Vec3::new(0.0, 0.02, 0.01);
            half[v] = Vec3::new(0.005, 0.008, 0.0);
        }
        let mut s = Shape::from_dense("puff", &full);
        s.inbetweens.push(Inbetween::from_dense(0.5, &half));
        rig.mesh.shapes.push(s);
        rig
    }

    /// The rig's own FBX, read back (its vertices split and ordered by the
    /// FBX reader), comes back as no edit at all.
    #[test]
    fn an_unedited_export_changes_nothing() {
        let mut rig = shaped();
        let scene = crate::import::fbx::read(&crate::export::fbx::write(&rig), None).unwrap();
        let (edits, m) = from_scene(&rig, &scene, "unreal", Some(0.005)).unwrap();
        assert_eq!(m.unmatched, 0, "{m:?}");
        assert!(m.bones >= 20 && m.bone_error < 1e-4, "{m:?}");
        let shapes = rig.mesh.shapes.clone();
        assert_eq!(import_weights(&mut rig, &edits).unwrap().changed, 0);
        let r = import_shapes(&mut rig, &edits, &[]).unwrap();
        assert_eq!(r.updated, vec!["puff".to_string()]);
        assert_eq!(rig.mesh.shapes, shapes);
    }

    /// A file in another frame (turned, scaled to centimetres, moved), with
    /// its vertices in another order, a seam vertex doubled and 8-bit
    /// weights, still lines up: only the repainted vertex and the resculpted
    /// offsets change, in hive-rig's frame.
    #[test]
    fn an_export_in_its_own_frame_and_order_lines_up() {
        let mut rig = shaped();
        let scene = crate::import::fbx::read(&crate::export::fbx::write(&rig), None).unwrap();
        let n = scene.mesh.vertex_count();
        // Z up, centimetres, off to one side: what a game engine might write.
        let turn = glam::Quat::from_rotation_x(std::f32::consts::FRAC_PI_2)
            * glam::Quat::from_rotation_z(0.4);
        let frame = |p: Vec3| turn * p * 100.0 + Vec3::new(30.0, -12.0, 5.0);
        // Reversed order, with the first vertex written twice.
        let order: Vec<usize> = (0..n).rev().chain([n - 1]).collect();
        let mut out = scene.clone();
        out.mesh.positions = order
            .iter()
            .map(|&i| frame(scene.mesh.positions[i]))
            .collect();
        out.mesh.normals = order
            .iter()
            .map(|&i| turn * scene.mesh.normals[i])
            .collect();
        // 8-bit weights, as a game engine keeps them.
        out.skin = order
            .iter()
            .map(|&i| {
                let q: Vec<(usize, f32)> = scene.skin[i]
                    .iter()
                    .map(|&(j, w)| (j, (w * 255.0).round() / 255.0))
                    .filter(|p| p.1 > 0.0)
                    .collect();
                let total: f32 = q.iter().map(|p| p.1).sum();
                q.into_iter().map(|(j, w)| (j, w / total)).collect()
            })
            .collect();
        for j in &mut out.joints {
            j.position = frame(j.position);
        }
        let reorder = |dense: Vec<Vec3>| -> Vec<Vec3> {
            order.iter().map(|&i| turn * dense[i] * 100.0).collect()
        };
        for (s, src) in out.mesh.shapes.iter_mut().zip(&scene.mesh.shapes) {
            let full = Shape::from_dense(&s.name, &reorder(src.dense(n)));
            s.vertices = full.vertices;
            s.offsets = full.offsets;
            for (b, sb) in s.inbetweens.iter_mut().zip(&src.inbetweens) {
                *b = Inbetween::from_dense(sb.weight, &reorder(sb.dense(n)));
            }
        }
        // Repaint the file's vertex at rig vertex 40 onto the head, and push
        // the full target out further at rig vertex 97.
        // (Every copy of the file's vertex there, as a seam splits it.)
        let at = |v: usize| -> Vec<usize> {
            let p = frame(rig.mesh.positions[v]);
            (0..out.mesh.positions.len())
                .filter(|&i| out.mesh.positions[i].distance(p) < 1e-3)
                .collect()
        };
        let head = out.joints.iter().position(|j| j.name == "head").unwrap();
        for i in at(40) {
            out.skin[i] = vec![(head, 1.0)];
        }
        let mut dense = out.mesh.shapes[0].dense(out.mesh.positions.len());
        for i in at(97) {
            dense[i] = turn * Vec3::new(0.0, 0.05, 0.0) * 100.0;
        }
        let puff = &mut out.mesh.shapes[0];
        let moved = Shape::from_dense("puff", &dense);
        puff.vertices = moved.vertices;
        puff.offsets = moved.offsets;
        let inbetweens = rig.mesh.shapes[0].inbetweens.clone();

        let (edits, m) = from_scene(&rig, &out, "unreal", Some(0.005)).unwrap();
        assert_eq!(
            (m.unmatched, m.matched),
            (0, rig.mesh.vertex_count()),
            "{m:?}"
        );
        assert!(m.vertex_error < 1e-4, "{m:?}");
        // Through the file format, as the tool hands it over.
        let path =
            std::env::temp_dir().join(format!("hive-scene-edits-{}.json", std::process::id()));
        edits.write(&path).unwrap();
        let edits = Edits::read(&path).unwrap();
        std::fs::remove_file(&path).ok();
        assert_eq!(edits.weight_tolerance, Some(0.005));
        let before = rig.skin.clone();
        let r = import_weights(&mut rig, &edits).unwrap();
        // The vertices sharing rig vertex 40's place (a seam) take the paint too.
        let same_place: Vec<usize> = (0..rig.mesh.vertex_count())
            .filter(|&v| rig.mesh.positions[v].distance(rig.mesh.positions[40]) < 1e-6)
            .collect();
        assert_eq!(r.changed, same_place.len(), "{r:?}");
        let head_bone = rig.skeleton.index_of("head").unwrap();
        assert!((rig.skin.weight(40, head_bone) - 1.0).abs() < 1e-6);
        assert_eq!(rig.skin.joints[41], before.joints[41]);
        // Without the tolerance every rounded vertex would count as repainted.
        let strict = Edits {
            weight_tolerance: None,
            ..edits.clone()
        };
        let mut again = shaped();
        assert!(import_weights(&mut again, &strict).unwrap().changed > 100);
        import_shapes(&mut rig, &edits, &[]).unwrap();
        let puff = crate::shapes::find(&rig, "puff").unwrap();
        let full = puff.dense(rig.mesh.vertex_count());
        assert!(
            full[97].distance(Vec3::new(0.0, 0.05, 0.0)) < 1e-5,
            "{}",
            full[97]
        );
        assert_eq!(full[194], Vec3::new(0.0, 0.02, 0.01));
        // The in-between came through the turn and order exactly.
        assert_eq!(puff.inbetweens, inbetweens);
    }

    /// The horse's body pieces meet at vertices sharing a place but not a
    /// skin; each keeps its own through the FBX (told apart by its normal).
    #[test]
    fn vertices_sharing_a_place_keep_their_own_skin() {
        let c = crate::demo::Creature::Horse;
        let mut rig =
            crate::rig::build_rig(c.mesh(), &c.rig_name(), c.guides(), Default::default()).unwrap();
        let p = rig.mesh.positions.clone();
        let shared = (0..p.len())
            .filter(|&v| {
                (0..p.len()).any(|u| {
                    u != v
                        && p[u].distance(p[v]) < 1e-6
                        && rig.skin.influences(u).collect::<Vec<_>>()
                            != rig.skin.influences(v).collect::<Vec<_>>()
                })
            })
            .count();
        assert!(
            shared > 0,
            "the horse no longer has such vertices: pick another body"
        );
        let mut scene = crate::import::fbx::read(&crate::export::fbx::write(&rig), None).unwrap();
        let (edits, m) = from_scene(&rig, &scene, "unreal", Some(0.005)).unwrap();
        assert_eq!(m.unmatched, 0);
        let original = rig.clone();
        assert_eq!(import_weights(&mut rig, &edits).unwrap().changed, 0);
        // Repaint one copy at such a place: one rig vertex there takes the
        // paint (one whose skin the copy had), and its twins keep theirs.
        let v = (0..p.len())
            .find(|&v| (0..p.len()).filter(|&u| p[u].distance(p[v]) < 1e-6).count() > 3)
            .unwrap();
        let skin_names = |r: &Rig, v: usize| -> Vec<(String, u32)> {
            r.skin
                .influences(v)
                .map(|(j, w)| (r.skeleton.bones[j].name.clone(), (w * 1e4) as u32))
                .collect()
        };
        let copy = (0..scene.mesh.vertex_count())
            .find(|&i| {
                scene.mesh.positions[i].distance(p[v]) < 1e-6 && {
                    let mut theirs: Vec<(String, u32)> = scene.skin[i]
                        .iter()
                        .map(|&(j, w)| (scene.joints[j].name.clone(), (w * 1e4) as u32))
                        .collect();
                    let mut own = skin_names(&original, v);
                    theirs.sort();
                    own.sort();
                    theirs == own
                }
            })
            .unwrap();
        let head = scene.joints.iter().position(|j| j.name == "head").unwrap();
        scene.skin[copy] = vec![(head, 1.0)];
        let (edits, _) = from_scene(&original, &scene, "unreal", Some(0.005)).unwrap();
        let mut rig = original.clone();
        assert_eq!(import_weights(&mut rig, &edits).unwrap().changed, 1);
        let head_bone = rig.skeleton.index_of("head").unwrap();
        let painted: Vec<usize> = (0..p.len())
            .filter(|&u| rig.skin.joints[u] != original.skin.joints[u])
            .collect();
        assert_eq!(painted.len(), 1);
        assert!(rig.skin.weight(painted[0], head_bone) > 0.999);
        assert!(p[painted[0]].distance(p[v]) < 1e-6);
        let mut a = skin_names(&original, painted[0]);
        let mut b = skin_names(&original, v);
        a.sort();
        b.sort();
        assert_eq!(a, b, "the paint went to a vertex with another skin");
    }

    #[test]
    fn frames_fit_from_spread_or_flat_bones() {
        let turn = DMat3::from_quat(
            glam::DQuat::from_rotation_y(1.1) * glam::DQuat::from_rotation_x(-0.3),
        );
        let to_file = |h: DVec3| turn * h * 100.0 + DVec3::new(1.0, 2.0, 3.0);
        let check = |hive: &[DVec3]| {
            let pairs: Vec<_> = hive.iter().map(|&h| (to_file(h), h)).collect();
            let (a, t) = fit_frame(&pairs).unwrap();
            for &(s, h) in &pairs {
                assert!((a * s + t - h).length() < 1e-9);
            }
            // A point off the bones lands right too.
            let off = DVec3::new(0.3, -0.2, 0.7);
            assert!((a * to_file(off) + t - off).length() < 1e-9);
        };
        check(&[DVec3::ZERO, DVec3::X, DVec3::Y, DVec3::Z, DVec3::ONE]);
        // A snake: every bone in the mid-plane.
        check(&[
            DVec3::ZERO,
            DVec3::new(0.0, 0.1, 1.0),
            DVec3::new(0.0, 0.3, -1.0),
            DVec3::new(0.0, 0.0, 2.0),
        ]);
        // A line or fewer than three bones can't place a frame.
        assert!(
            fit_frame(&[
                (DVec3::ZERO, DVec3::ZERO),
                (DVec3::X, DVec3::X),
                (DVec3::X * 2.0, DVec3::X * 2.0)
            ])
            .is_none()
        );
        assert!(fit_frame(&[(DVec3::ZERO, DVec3::ZERO)]).is_none());
    }
}
