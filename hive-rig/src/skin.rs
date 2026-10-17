//! Automatic skin weights.
//!
//! Each vertex is first assigned a body region (left arm, right leg, torso...) from
//! the landmarks, which limits the bones that may influence it. Within that set,
//! weights fall off with the inverse fourth power of the distance to each bone
//! segment, are trimmed to the four strongest influences, and then smoothed over
//! the mesh surface so joints bend without hard creases.

use serde::{Deserialize, Serialize};

use crate::landmarks::Landmarks;
use crate::mesh::Mesh;
use crate::skeleton::{Bone, Region, Skeleton};

pub const MAX_INFLUENCES: usize = 4;

/// Up to four (bone index, weight) pairs per vertex; unused slots have weight 0.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SkinWeights {
    pub joints: Vec<[u16; MAX_INFLUENCES]>,
    pub weights: Vec<[f32; MAX_INFLUENCES]>,
}

#[derive(Debug, Clone, Copy)]
pub struct SkinOptions {
    /// Number of surface smoothing passes.
    pub smooth_iterations: usize,
    /// How much of the neighbour average to blend in per pass (0..1).
    pub smooth_strength: f32,
}

impl Default for SkinOptions {
    fn default() -> Self {
        SkinOptions {
            smooth_iterations: 3,
            smooth_strength: 0.5,
        }
    }
}

fn vertex_region(lm: &Landmarks, x: f32, y: f32) -> Region {
    let dx = x - lm.center_x;
    if y < lm.crotch_y {
        return if dx >= 0.0 {
            Region::LegL
        } else {
            Region::LegR
        };
    }
    if dx.abs() > lm.torso_half_width * 1.05 {
        return if dx > 0.0 { Region::ArmL } else { Region::ArmR };
    }
    Region::Center
}

/// Bones allowed to influence vertices of `region`.
fn allowed_bones(sk: &Skeleton, region: Region) -> Vec<usize> {
    let mut out = Vec::new();
    for (i, b) in sk.bones.iter().enumerate() {
        if !b.deform {
            continue;
        }
        let ok = match region {
            Region::Center => {
                b.region == Region::Center
                    || b.name.starts_with("upperarm")
                    || b.name.starts_with("thigh")
            }
            Region::LegL | Region::LegR => b.region == region || b.name == "pelvis",
            Region::ArmL | Region::ArmR => {
                let side = if region == Region::ArmL { "_l" } else { "_r" };
                b.region == region
                    || b.name == format!("clavicle{side}")
                    || b.name == format!("spine_{:02}", crate::biped::SPINE_COUNT)
            }
        };
        if ok {
            out.push(i);
        }
    }
    out
}

pub fn compute(mesh: &Mesh, sk: &Skeleton, lm: &Landmarks, opts: SkinOptions) -> SkinWeights {
    let regions = [
        Region::Center,
        Region::ArmL,
        Region::ArmR,
        Region::LegL,
        Region::LegR,
    ];
    let allowed: Vec<Vec<usize>> = regions.iter().map(|r| allowed_bones(sk, *r)).collect();
    let n_bones = sk.bones.len();
    let eps = (lm.height * 0.005).powi(2);
    // Distance is measured to each bone's skinned segment (see Skeleton::skin_segment).
    let segments: Vec<Bone> = (0..n_bones)
        .map(|i| {
            let (head, tail) = sk.skin_segment(i);
            Bone {
                head,
                tail,
                ..sk.bones[i].clone()
            }
        })
        .collect();

    // Dense per-vertex weights keep smoothing simple; skeletons are small.
    let mut dense: Vec<Vec<f32>> = mesh
        .positions
        .iter()
        .map(|p| {
            let r = vertex_region(lm, p.x, p.y);
            let ri = regions.iter().position(|x| *x == r).unwrap();
            let mut w = vec![0.0f32; n_bones];
            for &b in &allowed[ri] {
                let d = segments[b].distance_to(*p);
                w[b] = 1.0 / (d * d + eps).powi(2);
            }
            keep_top(&mut w);
            w
        })
        .collect();

    let adj = mesh.adjacency();
    for _ in 0..opts.smooth_iterations {
        let prev = dense.clone();
        for (v, nbrs) in adj.iter().enumerate() {
            if nbrs.is_empty() {
                continue;
            }
            let mut avg = vec![0.0f32; n_bones];
            for &nb in nbrs {
                for (a, x) in avg.iter_mut().zip(&prev[nb as usize]) {
                    *a += x;
                }
            }
            let k = opts.smooth_strength / nbrs.len() as f32;
            for (b, w) in dense[v].iter_mut().enumerate() {
                *w = *w * (1.0 - opts.smooth_strength) + avg[b] * k;
            }
            keep_top(&mut dense[v]);
        }
    }

    let mut out = SkinWeights::default();
    for (v, w) in dense.iter().enumerate() {
        let mut pairs: Vec<(usize, f32)> = w
            .iter()
            .copied()
            .enumerate()
            .filter(|(_, x)| *x > 0.0)
            .collect();
        pairs.sort_by(|a, b| b.1.total_cmp(&a.1));
        let mut j = [0u16; MAX_INFLUENCES];
        let mut ws = [0.0f32; MAX_INFLUENCES];
        for (slot, (b, x)) in pairs.iter().take(MAX_INFLUENCES).enumerate() {
            j[slot] = *b as u16;
            ws[slot] = *x;
        }
        if ws[0] == 0.0 {
            // Isolated vertex with no candidates: bind to the nearest deforming bone.
            j[0] = nearest_deform_bone(sk, mesh.positions[v]) as u16;
            ws[0] = 1.0;
        }
        out.joints.push(j);
        out.weights.push(ws);
    }
    out
}

fn nearest_deform_bone(sk: &Skeleton, p: glam::Vec3) -> usize {
    sk.bones
        .iter()
        .enumerate()
        .filter(|(_, b)| b.deform)
        .min_by(|a, b| a.1.distance_to(p).total_cmp(&b.1.distance_to(p)))
        .map(|(i, _)| i)
        .unwrap_or(0)
}

/// Zero all but the strongest `MAX_INFLUENCES` entries and renormalise to sum 1.
fn keep_top(w: &mut [f32]) {
    let mut order: Vec<usize> = (0..w.len()).collect();
    order.sort_by(|a, b| w[*b].total_cmp(&w[*a]));
    for &i in order.iter().skip(MAX_INFLUENCES) {
        w[i] = 0.0;
    }
    // Drop negligible influences so exports stay clean.
    let floor = 1e-4 * w[order[0]].max(f32::MIN_POSITIVE);
    for x in w.iter_mut() {
        if *x < floor {
            *x = 0.0;
        }
    }
    let sum: f32 = w.iter().sum();
    if sum > 0.0 {
        for x in w.iter_mut() {
            *x /= sum;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{biped, demo::demo_humanoid, landmarks::detect};

    #[test]
    fn weights_are_normalised_and_regional() {
        let mesh = demo_humanoid();
        let lm = detect(&mesh);
        let sk = biped::build(&lm);
        let skin = compute(&mesh, &sk, &lm, SkinOptions::default());
        assert_eq!(skin.weights.len(), mesh.vertex_count());
        for w in &skin.weights {
            let s: f32 = w.iter().sum();
            assert!((s - 1.0).abs() < 1e-4, "sum {s}");
        }
        // A vertex on the left fingertip is driven by the left hand only.
        let tip = mesh
            .positions
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.x.total_cmp(&b.1.x))
            .unwrap()
            .0;
        let hand = sk.index_of("hand_l").unwrap() as u16;
        assert_eq!(skin.joints[tip][0], hand);
        assert!(skin.weights[tip][0] > 0.9);
        // No vertex on the left shin is influenced by any right-side bone.
        for (i, p) in mesh.positions.iter().enumerate() {
            if p.y < 0.5 && p.x > 0.0 {
                for (j, w) in skin.joints[i].iter().zip(skin.weights[i]) {
                    if w > 0.0 {
                        assert!(!sk.bones[*j as usize].name.ends_with("_r"));
                    }
                }
            }
        }
    }
}
