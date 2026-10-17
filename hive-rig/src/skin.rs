//! Automatic skin weights.
//!
//! Each vertex is first assigned a body region (left arm, right leg, torso...) from
//! the landmarks, which limits the bones that may influence it. Within that set,
//! weights fall off with the inverse fourth power of the distance to each bone
//! segment, are trimmed to the four strongest influences, and then smoothed over
//! the mesh surface so joints bend without hard creases.

use serde::{Deserialize, Serialize};

use crate::components::Body;
use crate::mesh::Mesh;
use glam::Vec3;

use crate::landmarks::mirror;
use crate::skeleton::{Bone, Region, Side, Skeleton};

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

fn vertex_region(body: &Body, x: f32, y: f32) -> Region {
    let dx = x - body.center_x;
    if y < body.crotch_y {
        return if dx >= 0.0 {
            Region::LegL
        } else {
            Region::LegR
        };
    }
    if dx.abs() > body.torso_half_width * 1.05 {
        return if dx > 0.0 { Region::ArmL } else { Region::ArmR };
    }
    Region::Center
}

/// Bones allowed to influence vertices of `region`.
fn allowed_bones(sk: &Skeleton, region: Region) -> Vec<usize> {
    sk.bones
        .iter()
        .enumerate()
        .filter(|(_, b)| b.deform && (b.region == region || b.links.contains(&region)))
        .map(|(i, _)| i)
        .collect()
}

pub fn compute(mesh: &Mesh, sk: &Skeleton, body: &Body, opts: SkinOptions) -> SkinWeights {
    let regions = [
        Region::Center,
        Region::ArmL,
        Region::ArmR,
        Region::LegL,
        Region::LegR,
    ];
    let allowed: Vec<Vec<usize>> = regions.iter().map(|r| allowed_bones(sk, *r)).collect();
    let n_bones = sk.bones.len();
    let eps = (body.height * 0.005).powi(2);
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
            let r = vertex_region(body, p.x, p.y);
            let ri = regions.iter().position(|x| *x == r).unwrap();
            let mut w = vec![0.0f32; n_bones];
            for &b in &allowed[ri] {
                let d = segments[b].distance_to(*p);
                let falloff = match segments[b].reach {
                    Some(r) if d >= r => continue,
                    Some(r) => (1.0 - d / r).powi(2),
                    None => 1.0,
                };
                w[b] = falloff / (d * d + eps).powi(2);
            }
            keep_top(&mut w);
            w
        })
        .collect();

    smooth_dense(&mut dense, &mesh.adjacency(), opts, None);
    let mut out = SkinWeights::from_dense(&dense);
    for (v, j) in out.joints.iter_mut().enumerate() {
        if out.weights[v][0] == 0.0 {
            j[0] = nearest_deform_bone(sk, mesh.positions[v]) as u16;
            out.weights[v][0] = 1.0;
        }
    }
    out
}

/// Average each vertex's weights with its neighbours'. With `only`, just the
/// vertices that bone influences (and their neighbours) change.
fn smooth_dense(dense: &mut [Vec<f32>], adj: &[Vec<u32>], opts: SkinOptions, only: Option<usize>) {
    let n_bones = dense.first().map_or(0, Vec::len);
    let touched = |w: &[f32]| only.is_none_or(|b| w[b] > 0.0);
    for _ in 0..opts.smooth_iterations {
        let prev = dense.to_vec();
        for (v, nbrs) in adj.iter().enumerate() {
            if nbrs.is_empty()
                || !(touched(&prev[v]) || nbrs.iter().any(|&n| touched(&prev[n as usize])))
            {
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
}

impl SkinWeights {
    fn to_dense(&self, n_bones: usize) -> Vec<Vec<f32>> {
        self.joints
            .iter()
            .zip(&self.weights)
            .map(|(j, w)| {
                let mut d = vec![0.0f32; n_bones];
                for (b, x) in j.iter().zip(w) {
                    if (*b as usize) < n_bones {
                        d[*b as usize] += x;
                    }
                }
                d
            })
            .collect()
    }

    fn from_dense(dense: &[Vec<f32>]) -> SkinWeights {
        let mut out = SkinWeights::default();
        for w in dense {
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
            out.joints.push(j);
            out.weights.push(ws);
        }
        out
    }
}

/// Smooth painted or computed weights over the surface: everywhere, or only
/// around the vertices `bone` influences.
pub fn smooth(
    skin: &SkinWeights,
    mesh: &Mesh,
    sk: &Skeleton,
    bone: Option<usize>,
    iterations: usize,
) -> SkinWeights {
    let mut dense = skin.to_dense(sk.bones.len());
    let opts = SkinOptions {
        smooth_iterations: iterations,
        ..SkinOptions::default()
    };
    smooth_dense(&mut dense, &mesh.adjacency(), opts, bone);
    SkinWeights::from_dense(&dense)
}

/// For each bone, the bone on the other side of the body (its own index on the
/// centre line), found by mirrored position so it works with any naming.
pub fn bone_twins(sk: &Skeleton, center_x: f32, tolerance: f32) -> Vec<usize> {
    (0..sk.bones.len())
        .map(|i| {
            let b = &sk.bones[i];
            if b.side == Side::Center {
                return i;
            }
            let target = mirror(b.head, center_x);
            sk.bones
                .iter()
                .enumerate()
                .filter(|(_, o)| o.side != b.side && o.side != Side::Center)
                .map(|(j, o)| {
                    (
                        j,
                        o.head.distance(target) + o.tail.distance(mirror(b.tail, center_x)),
                    )
                })
                .filter(|(_, d)| *d < tolerance)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map_or(i, |(j, _)| j)
        })
        .collect()
}

/// Copy the weights of the `source` side onto the other side: each vertex on the
/// far side takes the weights of the nearest vertex to its mirror image, with
/// left and right bones swapped. Centre-line vertices are left alone.
pub fn mirror_weights(
    skin: &SkinWeights,
    mesh: &Mesh,
    sk: &Skeleton,
    center_x: f32,
    height: f32,
    source: Side,
) -> SkinWeights {
    let sign = source.sign();
    let eps = 1e-3 * height;
    let twins = bone_twins(sk, center_x, 0.05 * height);
    let sources: Vec<usize> = (0..mesh.positions.len())
        .filter(|&v| (mesh.positions[v].x - center_x) * sign > -eps)
        .collect();
    let grid = Grid::new(mesh, &sources, height / 64.0);
    let mut out = skin.clone();
    for (v, p) in mesh.positions.iter().enumerate() {
        if (p.x - center_x) * sign >= -eps {
            continue;
        }
        let Some(from) = grid.nearest(mesh, mirror(*p, center_x)) else {
            continue;
        };
        out.weights[v] = skin.weights[from];
        for (slot, j) in skin.joints[from].iter().enumerate() {
            out.joints[v][slot] = twins[*j as usize] as u16;
        }
    }
    out
}

/// A uniform grid over some of a mesh's vertices for nearest-vertex lookups.
struct Grid {
    cell: f32,
    cells: std::collections::HashMap<(i32, i32, i32), Vec<usize>>,
}

impl Grid {
    fn new(mesh: &Mesh, vertices: &[usize], cell: f32) -> Grid {
        let cell = cell.max(1e-6);
        let mut cells: std::collections::HashMap<(i32, i32, i32), Vec<usize>> = Default::default();
        for &v in vertices {
            cells
                .entry(Grid::key(mesh.positions[v], cell))
                .or_default()
                .push(v);
        }
        Grid { cell, cells }
    }

    fn key(p: Vec3, cell: f32) -> (i32, i32, i32) {
        let k = (p / cell).floor();
        (k.x as i32, k.y as i32, k.z as i32)
    }

    fn nearest(&self, mesh: &Mesh, p: Vec3) -> Option<usize> {
        let (x, y, z) = Grid::key(p, self.cell);
        let mut best: Option<(f32, usize)> = None;
        // Grow the search ring until a hit is found and the ring is past it.
        for r in 0..64i32 {
            for dx in -r..=r {
                for dy in -r..=r {
                    for dz in -r..=r {
                        if dx.abs().max(dy.abs()).max(dz.abs()) != r {
                            continue;
                        }
                        for &v in self
                            .cells
                            .get(&(x + dx, y + dy, z + dz))
                            .into_iter()
                            .flatten()
                        {
                            let d = mesh.positions[v].distance(p);
                            if best.is_none_or(|b| d < b.0) {
                                best = Some((d, v));
                            }
                        }
                    }
                }
            }
            if let Some((d, v)) = best
                && d <= r as f32 * self.cell
            {
                return Some(v);
            }
        }
        best.map(|b| b.1)
    }
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
    use crate::{components, demo::demo_humanoid, landmarks::detect};

    #[test]
    fn mirror_and_smooth_weights() {
        let mesh = demo_humanoid();
        let guides = components::biped(&detect(&mesh));
        let sk = components::build(&guides).unwrap().skeleton;
        let body = &guides.body;
        let skin = compute(&mesh, &sk, body, SkinOptions::default());
        let twins = bone_twins(&sk, body.center_x, 0.05 * body.height);
        assert_eq!(
            twins[sk.index_of("hand_l").unwrap()],
            sk.index_of("hand_r").unwrap()
        );
        assert_eq!(
            twins[sk.index_of("spine_01").unwrap()],
            sk.index_of("spine_01").unwrap()
        );

        // Wreck the right side, then mirror the left back onto it.
        let mut broken = skin.clone();
        let hand_l = sk.index_of("hand_l").unwrap() as u16;
        for (v, p) in mesh.positions.iter().enumerate() {
            if p.x < body.center_x - 0.01 {
                broken.joints[v] = [hand_l, 0, 0, 0];
                broken.weights[v] = [1.0, 0.0, 0.0, 0.0];
            }
        }
        let fixed = mirror_weights(&broken, &mesh, &sk, body.center_x, body.height, Side::Left);
        let right: Vec<usize> = (0..mesh.vertex_count())
            .filter(|&v| mesh.positions[v].x < body.center_x - 0.01)
            .collect();
        let same = right
            .iter()
            .filter(|&&v| fixed.joints[v][0] == skin.joints[v][0])
            .count();
        assert!(
            same as f32 > 0.9 * right.len() as f32,
            "{same} of {}",
            right.len()
        );
        assert!(right.iter().all(|&v| fixed.joints[v][0] != hand_l));

        let smoothed = smooth(&skin, &mesh, &sk, Some(hand_l as usize), 2);
        for w in &smoothed.weights {
            assert!((w.iter().sum::<f32>() - 1.0).abs() < 1e-4);
        }
        // Smoothing around the left hand leaves the right leg alone.
        let foot_r = mesh
            .positions
            .iter()
            .enumerate()
            .min_by(|a, b| (a.1.y - a.1.x).total_cmp(&(b.1.y - b.1.x)))
            .unwrap()
            .0;
        assert_eq!(smoothed.weights[foot_r], skin.weights[foot_r]);
    }

    #[test]
    fn weights_are_normalised_and_regional() {
        let mesh = demo_humanoid();
        let guides = components::biped(&detect(&mesh));
        let sk = components::build(&guides).unwrap().skeleton;
        let skin = compute(&mesh, &sk, &guides.body, SkinOptions::default());
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
