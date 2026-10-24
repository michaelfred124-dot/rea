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
use glam::Vec3;
use serde::{Deserialize, Serialize};

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
        let same = {
            let old: Vec<(usize, f32)> = rig.skin.influences(v).collect();
            old.len() == new.len()
                && new.iter().all(|&(j, w)| {
                    old.iter()
                        .any(|&(oj, ow)| oj == j as usize && (ow - w).abs() <= SAME_WEIGHT)
                })
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
}
