//! Skin weights: the automatic weights and the tools that edit them.
//!
//! The automatic weights measure each bone's distance to each vertex through the
//! solid volume of the mesh (geodesic voxel binding, see [`crate::voxel`]), so
//! weight never jumps across an air gap, whatever the creature. Weights fall off
//! with the inverse eighth power of that distance, are trimmed to the strongest
//! influences the rig allows, and are then smoothed over the mesh surface so
//! joints bend without hard creases. See `docs/skinning.md`.
//!
//! The tools: mirror, smooth, prune small influences, cap the influences per
//! vertex for a game engine, paint with a brush, keep hand edits across a
//! rebuild, and copy weights from another character.

use std::collections::HashMap;
use std::str::FromStr;

use glam::{Quat, Vec3};
use serde::{Deserialize, Serialize};

use crate::landmarks::mirror;
use crate::mesh::Mesh;
use crate::skeleton::{Side, Skeleton, turn_about};
use crate::voxel::{self, Volume};

/// Most influences per vertex any engine preset asks for.
pub const INFLUENCE_CAP: usize = 12;

/// Each vertex's (bone index, weight) influences, strongest first. Weights of a
/// vertex add up to 1. Files from earlier versions pad every vertex to four
/// entries with zero weights; zero entries are ignored everywhere.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SkinWeights {
    pub joints: Vec<Vec<u16>>,
    pub weights: Vec<Vec<f32>>,
}

/// How many influences a vertex may keep, and the smallest one worth keeping.
/// Stored with the guides so a rebuild keeps them.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SkinLimits {
    /// Most bones that may influence one vertex (1 to [`INFLUENCE_CAP`]).
    pub max_influences: usize,
    /// Influences below this weight are dropped and the rest renormalised
    /// (0 keeps everything).
    pub prune: f32,
}

impl Default for SkinLimits {
    fn default() -> Self {
        SkinLimits {
            max_influences: 4,
            prune: 0.0,
        }
    }
}

impl SkinLimits {
    pub fn is_default(&self) -> bool {
        *self == SkinLimits::default()
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            (1..=INFLUENCE_CAP).contains(&self.max_influences),
            "max influences must be between 1 and {INFLUENCE_CAP}, not {}",
            self.max_influences
        );
        anyhow::ensure!(
            self.prune.is_finite() && (0.0..0.5).contains(&self.prune),
            "the prune threshold must be at least 0 and below 0.5, not {}",
            self.prune
        );
        Ok(())
    }
}

/// Game engines' influence limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Engine {
    /// Unreal Engine's default skin cache: 8 influences.
    Unreal,
    /// Unreal with extra influences turned on: 12.
    Unreal12,
    /// Unity's usual quality setting: 4.
    Unity,
    /// glTF's first joints/weights set: 4.
    Gltf,
}

impl Engine {
    pub const ALL: [Engine; 4] = [
        Engine::Unreal,
        Engine::Unreal12,
        Engine::Unity,
        Engine::Gltf,
    ];

    pub fn max_influences(self) -> usize {
        match self {
            Engine::Unreal => 8,
            Engine::Unreal12 => 12,
            Engine::Unity | Engine::Gltf => 4,
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Engine::Unreal => "unreal",
            Engine::Unreal12 => "unreal12",
            Engine::Unity => "unity",
            Engine::Gltf => "gltf",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Engine::Unreal => "Unreal",
            Engine::Unreal12 => "Unreal (12)",
            Engine::Unity => "Unity",
            Engine::Gltf => "glTF",
        }
    }
}

impl FromStr for Engine {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> anyhow::Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "unreal" | "ue" | "ue5" | "unreal8" => Ok(Engine::Unreal),
            "unreal12" | "ue12" | "ue5_12" => Ok(Engine::Unreal12),
            "unity" => Ok(Engine::Unity),
            "gltf" | "glb" => Ok(Engine::Gltf),
            other => anyhow::bail!(
                "unknown engine '{other}' (expected unreal (8), unreal12, unity (4) or gltf (4))"
            ),
        }
    }
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

/// Voxels along the longest side of the mesh for the automatic weights: about
/// 7 mm on a 1.8 m character. Gaps narrower than a voxel or two (fingers held
/// together, thighs pressed together) are bridged.
pub const VOXEL_RESOLUTION: usize = 256;

/// Weights fall off with this power of the distance through the body. Measured
/// on real characters against their artists' weights (see docs/skinning.md),
/// 8 tracks them more closely than 4 or 6 and keeps elbows and knees fuller;
/// the smoothing passes then soften the joints.
const FALLOFF: i32 = 8;

/// Automatic weights for any mesh and any skeleton.
///
/// Each deforming bone's distance to each vertex is measured through the solid
/// volume of the mesh (see [`crate::voxel`]), so weight can't jump across the
/// air between legs, between an arm and the ribs, or between fingers further
/// apart than a voxel. Weights fall off with the inverse eighth power of that
/// distance (only within `reach` for bones that have one, such as face joints),
/// are trimmed to the strongest influences the rig allows, and smoothed over
/// the surface.
pub fn compute(mesh: &Mesh, sk: &Skeleton, opts: SkinOptions, limits: SkinLimits) -> SkinWeights {
    let max = limits.max_influences.clamp(1, INFLUENCE_CAP);
    let n_bones = sk.bones.len();
    let deform: Vec<usize> = (0..n_bones).filter(|&b| sk.bones[b].deform).collect();
    let (lo, hi) = mesh.bounds();
    // Distances are taken as a fraction of the mesh size, so the weights are
    // the same at any scale and stay well inside f32 range.
    let size = (hi - lo).max_element().max(1e-6);
    const EPS: f32 = 0.005 * 0.005;
    let dist = bone_distances(mesh, sk, &deform, VOXEL_RESOLUTION);

    let mut dense: Vec<Vec<f32>> = (0..mesh.vertex_count())
        .map(|v| {
            let mut w = vec![0.0f32; n_bones];
            for (k, &b) in deform.iter().enumerate() {
                let d = dist[k][v];
                if !d.is_finite() {
                    continue;
                }
                let falloff = match sk.bones[b].reach {
                    Some(r) if d >= r => continue,
                    Some(r) => (1.0 - d / r).powi(2),
                    None => 1.0,
                };
                let x = d / size;
                w[b] = falloff / (x * x + EPS).powi(FALLOFF / 2);
            }
            keep_top(&mut w, max);
            w
        })
        .collect();

    smooth_dense(&mut dense, &surface_adjacency(mesh), opts, None, max);
    let mut out = SkinWeights::from_dense(&dense);
    for v in 0..out.vertex_count() {
        if out.joints[v].is_empty() {
            out.set(
                v,
                vec![(nearest_deform_bone(sk, mesh.positions[v]) as u16, 1.0)],
            );
        }
    }
    limit(&out, limits)
}

/// For each bone in `bones`, its distance to every vertex through the mesh's
/// solid volume, never less than the straight-line distance. Measured on each
/// bone's skinned segment (see [`Skeleton::skin_segment`]), several bones at a
/// time.
fn bone_distances(mesh: &Mesh, sk: &Skeleton, bones: &[usize], resolution: usize) -> Vec<Vec<f32>> {
    let vol = Volume::new(mesh, resolution);
    let cells: Vec<Option<u32>> = mesh.positions.iter().map(|p| vol.cell_near(*p)).collect();
    let segments: Vec<(Vec3, Vec3)> = bones.iter().map(|&b| sk.skin_segment(b)).collect();
    let seeds: Vec<Vec<(u32, f32)>> = segments.iter().map(|&(a, b)| vol.seeds(a, b)).collect();
    // Pieces no bone passes through.
    let mut orphan = vec![true; vol.piece_count()];
    for &(c, _) in seeds.iter().flatten() {
        orphan[vol.piece_of(c)] = false;
    }
    let one = |k: usize, scratch: &mut Vec<f32>| -> Vec<f32> {
        let (head, tail) = segments[k];
        vol.distances(head, tail, &seeds[k], &orphan, scratch);
        mesh.positions
            .iter()
            .zip(&cells)
            .map(|(p, c)| {
                let straight = voxel::segment_distance(*p, head, tail);
                c.map_or(straight, |c| scratch[c as usize].max(straight))
            })
            .collect()
    };
    let threads = std::thread::available_parallelism()
        .map_or(1, |n| n.get())
        .clamp(1, bones.len().max(1));
    let mut out: Vec<Vec<f32>> = vec![Vec::new(); bones.len()];
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                let one = &one;
                scope.spawn(move || {
                    let mut scratch = Vec::new();
                    (t..bones.len())
                        .step_by(threads)
                        .map(|k| (k, one(k, &mut scratch)))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        for h in handles {
            for (k, d) in h.join().expect("a skin weight thread panicked") {
                out[k] = d;
            }
        }
    });
    out
}

/// Neighbours over the surface with vertices at the same position (split at
/// UV or normal seams) counted as one, so smoothing crosses seams and the two
/// copies of a seam vertex keep the same weights.
fn surface_adjacency(mesh: &Mesh) -> Vec<Vec<u32>> {
    let weld = voxel::weld_positions(&mesh.positions);
    let adj = mesh.adjacency();
    let mut merged: Vec<Vec<u32>> = vec![Vec::new(); adj.len()];
    for (v, nbrs) in adj.iter().enumerate() {
        let rep = weld[v] as usize;
        merged[rep].extend(nbrs.iter().map(|&n| weld[n as usize]));
    }
    for (rep, list) in merged.iter_mut().enumerate() {
        list.sort_unstable();
        list.dedup();
        list.retain(|&n| n as usize != rep);
    }
    (0..adj.len())
        .map(|v| merged[weld[v] as usize].clone())
        .collect()
}

/// Average each vertex's weights with its neighbours'. With `only`, just the
/// vertices that bone influences (and their neighbours) change.
fn smooth_dense(
    dense: &mut [Vec<f32>],
    adj: &[Vec<u32>],
    opts: SkinOptions,
    only: Option<usize>,
    max: usize,
) {
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
            keep_top(&mut dense[v], max);
        }
    }
}

impl SkinWeights {
    pub fn vertex_count(&self) -> usize {
        self.joints.len()
    }

    /// The vertex's (bone, weight) influences with a weight above zero.
    pub fn influences(&self, v: usize) -> impl Iterator<Item = (usize, f32)> + '_ {
        self.joints[v]
            .iter()
            .zip(&self.weights[v])
            .filter(|(_, w)| **w > 0.0)
            .map(|(j, w)| (*j as usize, *w))
    }

    /// How much `bone` weighs on vertex `v`.
    pub fn weight(&self, v: usize, bone: usize) -> f32 {
        self.influences(v)
            .filter(|(j, _)| *j == bone)
            .map(|(_, w)| w)
            .sum()
    }

    /// Replace a vertex's influences, dropping zeros and sorting strongest first.
    pub fn set(&mut self, v: usize, mut pairs: Vec<(u16, f32)>) {
        pairs.retain(|p| p.1 > 0.0);
        pairs.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        self.joints[v] = pairs.iter().map(|p| p.0).collect();
        self.weights[v] = pairs.iter().map(|p| p.1).collect();
    }

    /// The most influences any vertex has.
    pub fn width(&self) -> usize {
        (0..self.vertex_count())
            .map(|v| self.influences(v).count())
            .max()
            .unwrap_or(0)
    }

    fn pairs(&self, v: usize) -> Vec<(u16, f32)> {
        self.influences(v).map(|(j, w)| (j as u16, w)).collect()
    }

    fn to_dense(&self, n_bones: usize) -> Vec<Vec<f32>> {
        (0..self.vertex_count())
            .map(|v| {
                let mut d = vec![0.0f32; n_bones];
                for (b, x) in self.influences(v) {
                    if b < n_bones {
                        d[b] += x;
                    }
                }
                d
            })
            .collect()
    }

    fn from_dense(dense: &[Vec<f32>]) -> SkinWeights {
        let mut out = SkinWeights {
            joints: vec![Vec::new(); dense.len()],
            weights: vec![Vec::new(); dense.len()],
        };
        for (v, w) in dense.iter().enumerate() {
            out.set(
                v,
                w.iter()
                    .enumerate()
                    .filter(|(_, x)| **x > 0.0)
                    .map(|(b, x)| (b as u16, *x))
                    .collect(),
            );
        }
        out
    }
}

/// Merge repeated bones, sort strongest first, drop influences below the prune
/// threshold, keep at most `max_influences` and renormalise. `keep` is a bone
/// that stays even when it is among the smallest (the bone being painted).
fn limit_pairs(pairs: &mut Vec<(u16, f32)>, limits: SkinLimits, keep: Option<u16>) {
    let mut merged: Vec<(u16, f32)> = Vec::with_capacity(pairs.len());
    for &(j, w) in pairs.iter() {
        if !(w > 0.0 && w.is_finite()) {
            continue;
        }
        match merged.iter_mut().find(|m| m.0 == j) {
            Some(m) => m.1 += w,
            None => merged.push((j, w)),
        }
    }
    merged.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    normalise(&mut merged);
    if limits.prune > 0.0 && merged.len() > 1 {
        let strongest = merged[0].0;
        merged.retain(|p| p.1 >= limits.prune || p.0 == strongest || Some(p.0) == keep);
    }
    let max = limits.max_influences.clamp(1, INFLUENCE_CAP);
    if merged.len() > max {
        let kept = keep.and_then(|k| merged.iter().position(|p| p.0 == k));
        let held = kept.filter(|&i| i >= max).map(|i| merged[i]);
        merged.truncate(max);
        if let Some(h) = held {
            merged[max - 1] = h;
        }
    }
    normalise(&mut merged);
    merged.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    *pairs = merged;
}

fn normalise(pairs: &mut [(u16, f32)]) {
    let sum: f32 = pairs.iter().map(|p| p.1).sum();
    if sum > 0.0 {
        for p in pairs.iter_mut() {
            p.1 /= sum;
        }
    }
}

/// Apply a rig's limits to every vertex: prune small influences, cap the count
/// and renormalise.
pub fn limit(skin: &SkinWeights, limits: SkinLimits) -> SkinWeights {
    let mut out = skin.clone();
    for v in 0..out.vertex_count() {
        let mut pairs = skin.pairs(v);
        limit_pairs(&mut pairs, limits, None);
        out.set(v, pairs);
    }
    out
}

/// Smooth painted or computed weights over the surface: everywhere, or only
/// around the vertices `bone` influences.
pub fn smooth(
    skin: &SkinWeights,
    mesh: &Mesh,
    sk: &Skeleton,
    bone: Option<usize>,
    iterations: usize,
    limits: SkinLimits,
) -> SkinWeights {
    let mut dense = skin.to_dense(sk.bones.len());
    let opts = SkinOptions {
        smooth_iterations: iterations,
        ..SkinOptions::default()
    };
    smooth_dense(
        &mut dense,
        &surface_adjacency(mesh),
        opts,
        bone,
        limits.max_influences,
    );
    limit(&SkinWeights::from_dense(&dense), limits)
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
        out.weights[v] = skin.weights[from].clone();
        out.joints[v] = skin.joints[from]
            .iter()
            .map(|j| twins.get(*j as usize).map_or(*j, |t| *t as u16))
            .collect();
    }
    out
}

// --------------------------------------------------------------------------- paint

/// What a brush dab does to the painted bone's weight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrushMode {
    /// Raise the bone's weight; the vertex's other bones give way.
    Add,
    /// Lower the bone's weight; the vertex's other bones take it up.
    Subtract,
    /// Blend every bone's weight towards the neighbours' average.
    Smooth,
}

/// One touch of the weight brush.
#[derive(Debug, Clone, Copy)]
pub struct Dab {
    pub center: Vec3,
    /// Metres.
    pub radius: f32,
    /// 0..1: how far one dab moves a vertex at the brush centre.
    pub strength: f32,
    pub mode: BrushMode,
}

/// Paint one dab of `bone`'s weight onto the vertices within the brush, fading
/// to nothing at its rim. `positions` are where the vertices are drawn (posed or
/// at rest). Returns how many vertices changed.
pub fn paint(
    skin: &mut SkinWeights,
    positions: &[Vec3],
    adjacency: &[Vec<u32>],
    sk: &Skeleton,
    bone: usize,
    dab: Dab,
    limits: SkinLimits,
) -> usize {
    if bone >= sk.bones.len() || dab.radius <= 0.0 || dab.strength <= 0.0 {
        return 0;
    }
    let b = bone as u16;
    let strength = dab.strength.clamp(0.0, 1.0);
    let inside: Vec<(usize, f32)> = positions
        .iter()
        .enumerate()
        .filter_map(|(v, p)| {
            let d = p.distance(dab.center) / dab.radius;
            (d < 1.0).then(|| (v, strength * (1.0 - d * d).powi(2)))
        })
        .collect();
    let before = (dab.mode == BrushMode::Smooth).then(|| skin.clone());
    let fallback = fallback_bone(sk, bone);
    let mut changed = 0;
    for (v, t) in inside {
        if v >= skin.vertex_count() {
            continue;
        }
        let mut pairs = skin.pairs(v);
        let own: f32 = pairs.iter().filter(|p| p.0 == b).map(|p| p.1).sum();
        let others: f32 = pairs.iter().filter(|p| p.0 != b).map(|p| p.1).sum();
        let keep = match dab.mode {
            BrushMode::Add => {
                let target = own + t * (1.0 - own);
                scale_others(&mut pairs, b, target, others);
                Some(b)
            }
            BrushMode::Subtract => {
                let target = own * (1.0 - t);
                if others > 0.0 {
                    scale_others(&mut pairs, b, target, others);
                } else if let Some(f) = fallback {
                    pairs.retain(|p| p.0 != b);
                    pairs.push((b, target));
                    pairs.push((f as u16, own - target));
                }
                None
            }
            BrushMode::Smooth => {
                let before = before.as_ref().unwrap();
                let nbrs = adjacency.get(v).map_or(&[][..], Vec::as_slice);
                if nbrs.is_empty() {
                    continue;
                }
                let k = t / nbrs.len() as f32;
                let mut blended: Vec<(u16, f32)> =
                    pairs.iter().map(|&(j, w)| (j, w * (1.0 - t))).collect();
                for &n in nbrs {
                    blended.extend(
                        before
                            .pairs(n as usize)
                            .into_iter()
                            .map(|(j, w)| (j, w * k)),
                    );
                }
                pairs = blended;
                None
            }
        };
        limit_pairs(&mut pairs, limits, keep);
        if pairs != skin.pairs(v) {
            skin.set(v, pairs);
            changed += 1;
        }
    }
    changed
}

/// Set bone `b`'s weight to `target` and scale the other bones so the vertex
/// still adds up to 1.
fn scale_others(pairs: &mut Vec<(u16, f32)>, b: u16, target: f32, others: f32) {
    let k = if others > 0.0 {
        (1.0 - target) / others
    } else {
        0.0
    };
    pairs.retain(|p| p.0 != b);
    for p in pairs.iter_mut() {
        p.1 *= k;
    }
    pairs.push((b, if others > 0.0 { target } else { 1.0 }));
}

/// Where weight taken off a bone goes when nothing else holds the vertex: its
/// nearest deforming ancestor, or else its nearest deforming bone.
fn fallback_bone(sk: &Skeleton, bone: usize) -> Option<usize> {
    let mut at = sk.bones[bone].parent;
    while let Some(p) = at {
        if sk.bones[p].deform {
            return Some(p);
        }
        at = sk.bones[p].parent;
    }
    let mid = (sk.bones[bone].head + sk.bones[bone].tail) * 0.5;
    sk.bones
        .iter()
        .enumerate()
        .filter(|(i, b)| *i != bone && b.deform)
        .min_by(|a, b| a.1.distance_to(mid).total_cmp(&b.1.distance_to(mid)))
        .map(|(i, _)| i)
}

// --------------------------------------------------------------------------- rebuild

/// Whether two vertices' influences agree to within `tol` per bone.
fn same_vertex(a: &SkinWeights, b: &SkinWeights, v: usize, tol: f32) -> bool {
    let (pa, pb) = (a.pairs(v), b.pairs(v));
    pa.iter()
        .chain(&pb)
        .all(|&(j, _)| (a.weight(v, j as usize) - b.weight(v, j as usize)).abs() <= tol)
}

/// Carry hand-edited weights over to a rebuilt rig, the way the Blender and Maya
/// builders' Rebuild does: a vertex keeps its old weights unless the new rig
/// weights it to a bone the old rig didn't have. Bones are matched by name.
///
/// With `old_auto` (the automatic weights the old rig was built with), only the
/// vertices that were edited since are carried over, so the rest take the new
/// guides' automatic weights. Returns the weights and how many vertices kept theirs.
pub fn keep_edits(
    old: &SkinWeights,
    old_bones: &[String],
    old_auto: Option<&SkinWeights>,
    new: &SkinWeights,
    new_bones: &[String],
) -> (SkinWeights, usize) {
    let mut out = new.clone();
    if old.vertex_count() != new.vertex_count()
        || old_auto.is_some_and(|a| a.vertex_count() != old.vertex_count())
    {
        return (out, 0);
    }
    let index: HashMap<&str, usize> = new_bones
        .iter()
        .enumerate()
        .map(|(i, n)| (n.as_str(), i))
        .collect();
    let existed: std::collections::HashSet<&str> = old_bones.iter().map(String::as_str).collect();
    let mut kept = 0;
    for v in 0..old.vertex_count() {
        if old.influences(v).next().is_none()
            || old_auto.is_some_and(|a| same_vertex(old, a, v, 1e-4))
        {
            continue;
        }
        let new_bone_here = new.influences(v).any(|(j, _)| {
            new_bones
                .get(j)
                .is_none_or(|n| !existed.contains(n.as_str()))
        });
        if new_bone_here {
            continue;
        }
        let mut pairs: Vec<(u16, f32)> = old
            .influences(v)
            .filter_map(|(j, w)| {
                let name = old_bones.get(j)?;
                Some((*index.get(name.as_str())? as u16, w))
            })
            .collect();
        if pairs.is_empty() {
            continue;
        }
        normalise(&mut pairs);
        out.set(v, pairs);
        kept += 1;
    }
    (out, kept)
}

// --------------------------------------------------------------------------- transfer

/// One side of a weight transfer: a skinned character.
pub struct Skinned<'a> {
    pub mesh: &'a Mesh,
    pub skeleton: &'a Skeleton,
    pub skin: &'a SkinWeights,
}

/// Copy weights from `src` onto `dst`'s mesh. `dst_to_src` gives, for each of
/// `dst`'s bones, the matching bone of `src` (by name), if any.
///
/// Each target vertex is first carried into the source's pose and size: every
/// matched bone turns the vertex from its own direction onto the source bone's
/// and moves it with the bone's head, blended by the target's current weights,
/// so an A-pose arm lands on a T-pose arm. The vertex then takes the weights at
/// the closest point of the source surface, blended across that triangle's
/// corners, with each source bone renamed to its target bone. Source bones
/// without a match hand their weight to their nearest matched ancestor.
pub fn transfer(src: &Skinned, dst: &Skinned, dst_to_src: &[Option<usize>]) -> SkinWeights {
    let n_src = src.skeleton.bones.len();
    let n_dst = dst.skeleton.bones.len();
    let (smin, smax) = src.mesh.bounds();
    let (dmin, dmax) = dst.mesh.bounds();
    let scale = (smax.y - smin.y) / (dmax.y - dmin.y).max(1e-6);
    let anchor =
        |min: Vec3, max: Vec3| Vec3::new((min.x + max.x) * 0.5, min.y, (min.z + max.z) * 0.5);
    let (s_anchor, d_anchor) = (anchor(smin, smax), anchor(dmin, dmax));

    // Each target bone's move into source space: the matched bone, or the
    // nearest matched ancestor's.
    let mut moves: Vec<Option<(Vec3, Quat, Vec3)>> = vec![None; n_dst];
    for (b, bone) in dst.skeleton.bones.iter().enumerate() {
        moves[b] = match dst_to_src.get(b).copied().flatten() {
            Some(s) if s < n_src => {
                let sb = &src.skeleton.bones[s];
                let turn = if bone.length() > 1e-6 && sb.length() > 1e-6 {
                    Quat::from_rotation_arc(bone.direction(), sb.direction())
                } else {
                    Quat::IDENTITY
                };
                Some((bone.head, turn, sb.head))
            }
            _ => bone.parent.and_then(|p| moves.get(p).copied().flatten()),
        };
    }
    let carry = |v: usize, p: Vec3| -> Vec3 {
        let mut out = Vec3::ZERO;
        let mut total = 0.0;
        for (j, w) in dst.skin.influences(v) {
            if let Some((from, turn, to)) = moves.get(j).copied().flatten() {
                out += (to + turn * ((p - from) * scale)) * w;
                total += w;
            }
        }
        if total > 0.0 {
            out / total
        } else {
            s_anchor + (p - d_anchor) * scale
        }
    };

    // Each source bone's target: its match, or its nearest matched ancestor's.
    let mut src_to_dst: Vec<Option<usize>> = vec![None; n_src];
    for (d, s) in dst_to_src.iter().enumerate() {
        if let Some(s) = *s
            && s < n_src
            && src_to_dst[s].is_none()
        {
            src_to_dst[s] = Some(d);
        }
    }
    let renamed: Vec<Option<usize>> = (0..n_src)
        .map(|s| {
            let mut at = Some(s);
            while let Some(i) = at {
                if let Some(d) = src_to_dst[i] {
                    return Some(d);
                }
                at = src.skeleton.bones[i].parent;
            }
            None
        })
        .collect();

    let grid = TriangleGrid::new(src.mesh);
    let mut out = SkinWeights {
        joints: vec![Vec::new(); dst.mesh.vertex_count()],
        weights: vec![Vec::new(); dst.mesh.vertex_count()],
    };
    for (v, p) in dst.mesh.positions.iter().enumerate() {
        let at = carry(v, *p);
        let mut pairs: Vec<(u16, f32)> = Vec::new();
        if let Some((tri, bary)) = grid.closest(src.mesh, at) {
            for (corner, k) in tri.into_iter().zip(bary.to_array()) {
                for (s, w) in src.skin.influences(corner) {
                    if let Some(d) = renamed.get(s).copied().flatten() {
                        pairs.push((d as u16, w * k));
                    }
                }
            }
        }
        limit_pairs(
            &mut pairs,
            SkinLimits {
                max_influences: INFLUENCE_CAP,
                prune: 0.0,
            },
            None,
        );
        if pairs.is_empty() {
            // Nothing on the source maps here: keep the target's own weights.
            pairs = dst.skin.pairs(v);
        }
        if pairs.is_empty() {
            pairs = vec![(nearest_deform_bone(dst.skeleton, *p) as u16, 1.0)];
        }
        out.set(v, pairs);
    }
    out
}

/// Closest point on triangle `abc` to `p`, as barycentric weights of a, b, c.
fn closest_on_triangle(p: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return Vec3::X;
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return Vec3::Y;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let t = d1 / (d1 - d3);
        return Vec3::new(1.0 - t, t, 0.0);
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return Vec3::Z;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let t = d2 / (d2 - d6);
        return Vec3::new(1.0 - t, 0.0, t);
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        let t = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return Vec3::new(0.0, 1.0 - t, t);
    }
    let denom = va + vb + vc;
    if denom.abs() < f32::MIN_POSITIVE {
        return Vec3::X;
    }
    let (v, w) = (vb / denom, vc / denom);
    Vec3::new(1.0 - v - w, v, w)
}

/// A uniform grid of a mesh's triangles for closest-surface-point lookups.
struct TriangleGrid {
    cell: f32,
    cells: HashMap<(i32, i32, i32), Vec<usize>>,
    /// The lowest and highest occupied cells, so searches know when to stop.
    lo: (i32, i32, i32),
    hi: (i32, i32, i32),
}

impl TriangleGrid {
    fn new(mesh: &Mesh) -> TriangleGrid {
        let (min, max) = mesh.bounds();
        let cell = ((max - min).max_element() / 48.0).max(1e-4);
        let mut cells: HashMap<(i32, i32, i32), Vec<usize>> = HashMap::new();
        for (t, [a, b, c]) in mesh.triangles().enumerate() {
            let (p, q, r) = (mesh.positions[a], mesh.positions[b], mesh.positions[c]);
            let lo = Grid::key(p.min(q).min(r), cell);
            let hi = Grid::key(p.max(q).max(r), cell);
            for x in lo.0..=hi.0 {
                for y in lo.1..=hi.1 {
                    for z in lo.2..=hi.2 {
                        cells.entry((x, y, z)).or_default().push(t);
                    }
                }
            }
        }
        TriangleGrid {
            cell,
            cells,
            lo: Grid::key(min, cell),
            hi: Grid::key(max, cell),
        }
    }

    /// The triangle (corner vertices) whose surface is closest to `p`, and the
    /// closest point as barycentric weights of those corners.
    fn closest(&self, mesh: &Mesh, p: Vec3) -> Option<([usize; 3], Vec3)> {
        let (x, y, z) = Grid::key(p, self.cell);
        let tris = mesh.indices.as_chunks::<3>().0;
        let mut best: Option<(f32, [usize; 3], Vec3)> = None;
        // The ring that reaches every occupied cell.
        let far = [
            (x - self.lo.0).abs().max((x - self.hi.0).abs()),
            (y - self.lo.1).abs().max((y - self.hi.1).abs()),
            (z - self.lo.2).abs().max((z - self.hi.2).abs()),
        ]
        .into_iter()
        .max()
        .unwrap_or(0);
        for r in 0..=far {
            // The shell of cells at ring r, clipped to the occupied box.
            for cx in (x - r).max(self.lo.0)..=(x + r).min(self.hi.0) {
                for cy in (y - r).max(self.lo.1)..=(y + r).min(self.hi.1) {
                    // Columns on the shell's sides take every z; inside, only
                    // the top and bottom caps.
                    let side = (cx - x).abs() == r || (cy - y).abs() == r;
                    let step = if side { 1 } else { 2 * r };
                    let zs = (z - r..=z + r)
                        .step_by(step.max(1) as usize)
                        .filter(|cz| (self.lo.2..=self.hi.2).contains(cz));
                    for cz in zs {
                        for &t in self.cells.get(&(cx, cy, cz)).into_iter().flatten() {
                            let tri = tris[t].map(|i| i as usize);
                            let [a, b, c] = tri.map(|i| mesh.positions[i]);
                            let bary = closest_on_triangle(p, a, b, c);
                            let q = a * bary.x + b * bary.y + c * bary.z;
                            let d = q.distance_squared(p);
                            if best.is_none_or(|b| d < b.0) {
                                best = Some((d, tri, bary));
                            }
                        }
                    }
                }
            }
            // Every cell not searched yet lies outside the cube of cells searched
            // so far, at least this far from `p`.
            let lo = Vec3::new((x - r) as f32, (y - r) as f32, (z - r) as f32) * self.cell;
            let hi =
                Vec3::new((x + r + 1) as f32, (y + r + 1) as f32, (z + r + 1) as f32) * self.cell;
            let margin = (p - lo).min(hi - p).min_element().max(0.0);
            if let Some((d, tri, bary)) = best
                && d <= margin * margin
            {
                return Some((tri, bary));
            }
        }
        best.map(|b| (b.1, b.2))
    }
}

/// A uniform grid over some of a mesh's vertices for nearest-vertex lookups.
struct Grid {
    cell: f32,
    cells: HashMap<(i32, i32, i32), Vec<usize>>,
}

impl Grid {
    fn new(mesh: &Mesh, vertices: &[usize], cell: f32) -> Grid {
        let cell = cell.max(1e-6);
        let mut cells: HashMap<(i32, i32, i32), Vec<usize>> = Default::default();
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

/// Zero all but the strongest `max` entries and renormalise to sum 1.
fn keep_top(w: &mut [f32], max: usize) {
    let mut order: Vec<usize> = (0..w.len()).collect();
    order.sort_by(|a, b| w[*b].total_cmp(&w[*a]));
    for &i in order.iter().skip(max) {
        w[i] = 0.0;
    }
    let Some(&top) = order.first() else { return };
    // Drop negligible influences so exports stay clean.
    let floor = 1e-4 * w[top].max(f32::MIN_POSITIVE);
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

/// Skin the mesh's rest positions with the bones in `moved` turned by `radians`
/// about `axis` through `pivot`, the rest of the bones staying put. Each vertex
/// takes its weighted blend of its bones' transforms, as a game engine would.
pub fn deform_turn(
    rest: &[Vec3],
    skin: &SkinWeights,
    moved: &[bool],
    pivot: Vec3,
    axis: Vec3,
    radians: f32,
) -> Vec<Vec3> {
    rest.iter()
        .enumerate()
        .map(|(v, &p)| {
            if v >= skin.vertex_count() {
                return p;
            }
            let turned = turn_about(pivot, axis, radians, p);
            let mut out = Vec3::ZERO;
            let mut total = 0.0;
            for (j, w) in skin.influences(v) {
                let moves = moved.get(j).copied().unwrap_or(false);
                out += if moves { turned } else { p } * w;
                total += w;
            }
            if total > 0.0 { out / total } else { p }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::Body;
    use crate::{components, demo, landmarks::detect};

    struct Built {
        mesh: Mesh,
        sk: Skeleton,
        body: Body,
    }

    fn demo_rig(arm_drop: f32) -> Built {
        let mesh = demo::humanoid(arm_drop);
        let guides = components::biped(&detect(&mesh));
        let sk = components::build(&guides).unwrap().skeleton;
        Built {
            mesh,
            sk,
            body: guides.body,
        }
    }

    fn auto(b: &Built, limits: SkinLimits) -> SkinWeights {
        compute(&b.mesh, &b.sk, SkinOptions::default(), limits)
    }

    fn skinned<'a>(b: &'a Built, skin: &'a SkinWeights) -> Skinned<'a> {
        Skinned {
            mesh: &b.mesh,
            skeleton: &b.sk,
            skin,
        }
    }

    fn assert_normalised(skin: &SkinWeights) {
        for v in 0..skin.vertex_count() {
            let s: f32 = skin.influences(v).map(|(_, w)| w).sum();
            assert!((s - 1.0).abs() < 1e-4, "vertex {v} sums to {s}");
        }
    }

    fn dominant(skin: &SkinWeights, v: usize) -> usize {
        skin.influences(v)
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map_or(usize::MAX, |p| p.0)
    }

    fn one(j: u16) -> (Vec<u16>, Vec<f32>) {
        (vec![j], vec![1.0])
    }

    #[test]
    fn mirror_and_smooth_weights() {
        let b = demo_rig(0.0);
        let (mesh, sk, body) = (&b.mesh, &b.sk, &b.body);
        let skin = auto(&b, SkinLimits::default());
        let twins = bone_twins(sk, body.center_x, 0.05 * body.height);
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
                (broken.joints[v], broken.weights[v]) = one(hand_l);
            }
        }
        let fixed = mirror_weights(&broken, mesh, sk, body.center_x, body.height, Side::Left);
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

        let smoothed = smooth(
            &skin,
            mesh,
            sk,
            Some(hand_l as usize),
            2,
            SkinLimits::default(),
        );
        assert_normalised(&smoothed);
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
    fn weights_are_normalised_and_limbs_stay_apart() {
        let b = demo_rig(0.0);
        let (mesh, sk) = (&b.mesh, &b.sk);
        let skin = auto(&b, SkinLimits::default());
        assert_eq!(skin.vertex_count(), mesh.vertex_count());
        assert_normalised(&skin);
        assert!(skin.width() <= 4);
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
                for (j, _) in skin.influences(i) {
                    assert!(
                        !sk.bones[j].name.ends_with("_r"),
                        "{} at {p}",
                        sk.bones[j].name
                    );
                }
            }
        }
    }

    /// The summed weight of the bones whose names start with any of `prefixes`.
    fn weight_of(skin: &SkinWeights, sk: &Skeleton, v: usize, prefixes: &[&str]) -> f32 {
        skin.influences(v)
            .filter(|(j, _)| prefixes.iter().any(|p| sk.bones[*j].name.starts_with(p)))
            .map(|(_, w)| w)
            .sum()
    }

    #[test]
    fn arms_hanging_by_the_body_keep_their_own_weights() {
        // Arms dropped 80 degrees hang a few centimetres from the ribs. Body
        // detection doesn't read arms that low, so the arm guides are put on
        // the demo's arms by hand.
        let drop = 80f32.to_radians();
        let mesh = demo::humanoid(80.0);
        let mut guides = components::biped(&detect(&demo::humanoid(45.0)));
        for c in &mut guides.components {
            if c.kind != components::Kind::Arm {
                continue;
            }
            let sx = c.side.sign();
            let shoulder = Vec3::new(0.12 * sx, 1.40, 0.0);
            let dir = Vec3::new(sx * drop.cos(), -drop.sin(), 0.0);
            for g in &mut c.guides {
                g.position = match g.name.as_str() {
                    "shoulder" => shoulder,
                    "elbow" => shoulder + dir * 0.32 - Vec3::Z * 0.01,
                    "wrist" => shoulder + dir * 0.64,
                    "hand_tip" => shoulder + dir * 0.74,
                    _ => g.position,
                };
            }
        }
        let b = Built {
            sk: components::build(&guides).unwrap().skeleton,
            body: guides.body,
            mesh,
        };
        let (mesh, sk) = (&b.mesh, &b.sk);
        let skin = auto(&b, SkinLimits::default());
        assert_normalised(&skin);
        let arm = ["upperarm", "lowerarm", "hand"];
        let mut checked = 0;
        for (v, p) in mesh.positions.iter().enumerate() {
            // The flanks of the torso, from the elbows down (the arm fuses
            // with the ribs above, where the armpit blends).
            if p.x.abs() > 0.08 && p.x.abs() < 0.1 && (0.9..1.09).contains(&p.y) {
                let w = weight_of(&skin, sk, v, &arm);
                assert!(w < 0.05, "torso vertex {p} takes {w} from the arm");
                checked += 1;
            }
        }
        assert!(checked > 10, "{checked}");
        // The forearm's far half goes to its twist bone, the near half to the
        // forearm, as the twist distribution expects.
        let lower = sk.index_of("lowerarm_l").unwrap();
        let twist = sk.index_of("lowerarm_twist_01_l").unwrap();
        let (elbow, wrist) = (sk.bones[lower].head, sk.bones[lower].tail);
        let near = |t: f32| {
            let at = elbow.lerp(wrist, t);
            (0..mesh.vertex_count())
                .min_by(|a, c| {
                    mesh.positions[*a]
                        .distance(at)
                        .total_cmp(&mesh.positions[*c].distance(at))
                })
                .unwrap()
        };
        assert_eq!(dominant(&skin, near(0.2)), lower);
        assert_eq!(dominant(&skin, near(0.85)), twist);
    }

    #[test]
    fn an_eyeball_piece_follows_its_eye_bone() {
        let mut mesh = demo::humanoid(0.0);
        let mut guides = components::biped(&detect(&mesh));
        components::add_preset(
            &mut guides,
            components::Preset::Eyes,
            &components::AddOptions::default(),
        )
        .unwrap();
        let sk = components::build(&guides).unwrap().skeleton;
        let eye = sk.index_of("eye_l").unwrap();
        let ball = demo::sphere(sk.bones[eye].head, 0.012, 12, 8);
        let first = mesh.vertex_count();
        mesh.append(&ball);
        let skin = compute(&mesh, &sk, SkinOptions::default(), SkinLimits::default());
        for v in first..mesh.vertex_count() {
            assert_eq!(dominant(&skin, v), eye, "eyeball vertex {v}");
            assert!(skin.weight(v, eye) > 0.8, "{}", skin.weight(v, eye));
        }
    }

    #[test]
    fn seam_copies_share_their_weights() {
        // Split the whole demo mesh as if along UV seams: every vertex gets a
        // twin at the same place, used by every other triangle.
        let b = demo_rig(0.0);
        let mut mesh = b.mesh.clone();
        let n = mesh.vertex_count() as u32;
        mesh.positions.extend(b.mesh.positions.clone());
        mesh.normals.extend(b.mesh.normals.clone());
        for (t, tri) in mesh.indices.chunks_mut(3).enumerate() {
            if t % 2 == 0 {
                for i in tri {
                    *i += n;
                }
            }
        }
        let skin = compute(&mesh, &b.sk, SkinOptions::default(), SkinLimits::default());
        for (v, nbrs) in b.mesh.adjacency().iter().enumerate() {
            if nbrs.is_empty() {
                continue;
            }
            for bone in 0..b.sk.bones.len() {
                let (x, y) = (skin.weight(v, bone), skin.weight(v + n as usize, bone));
                assert!((x - y).abs() < 1e-4, "vertex {v} bone {bone}: {x} vs {y}");
            }
        }
    }

    #[test]
    fn prune_drops_small_influences_and_renormalises() {
        let skin = SkinWeights {
            joints: vec![vec![3, 1, 2, 7, 5, 4], vec![2, 2, 9], vec![0, 0, 0, 0]],
            weights: vec![
                vec![0.5, 0.2, 0.15, 0.08, 0.05, 0.02],
                vec![0.3, 0.3, 0.0],
                vec![1.0, 0.0, 0.0, 0.0],
            ],
        };
        let limits = SkinLimits {
            max_influences: 8,
            prune: 0.1,
        };
        let skin = limit(&skin, limits);
        assert_normalised(&skin);
        // Only the influences of 0.1 or more survive, in order, rescaled.
        assert_eq!(skin.joints[0], vec![3, 1, 2]);
        assert!((skin.weights[0][0] - 0.5 / 0.85).abs() < 1e-5);
        // A bone listed twice is merged; zero padding from old files is dropped.
        assert_eq!(skin.joints[1], vec![2]);
        assert_eq!(skin.weights[1], vec![1.0]);
        assert_eq!(skin.joints[2], vec![0]);
        // Pruning never empties a vertex: the strongest influence stays.
        let even = SkinWeights {
            joints: vec![(0..10).collect()],
            weights: vec![vec![0.1; 10]],
        };
        let pruned = limit(
            &even,
            SkinLimits {
                max_influences: 12,
                prune: 0.45,
            },
        );
        assert_eq!(pruned.joints[0].len(), 1);
        assert_eq!(pruned.weights[0], vec![1.0]);

        // On the demo, pruning at 0.05 leaves nothing below 0.05.
        let b = demo_rig(0.0);
        let skin = auto(
            &b,
            SkinLimits {
                max_influences: 4,
                prune: 0.05,
            },
        );
        assert_normalised(&skin);
        for v in 0..skin.vertex_count() {
            assert!(skin.influences(v).all(|(_, w)| w >= 0.05 - 1e-6));
        }
    }

    #[test]
    fn max_influences_follow_the_engine() {
        assert_eq!("UE5".parse::<Engine>().unwrap().max_influences(), 8);
        assert_eq!("unreal12".parse::<Engine>().unwrap().max_influences(), 12);
        assert_eq!("unity".parse::<Engine>().unwrap().max_influences(), 4);
        assert_eq!("gltf".parse::<Engine>().unwrap().max_influences(), 4);
        assert!("godot".parse::<Engine>().is_err());
        let too_many = SkinLimits {
            max_influences: 13,
            prune: 0.0,
        };
        assert!(too_many.validate().is_err());

        let b = demo_rig(0.0);
        let four = auto(&b, SkinLimits::default());
        let eight = auto(
            &b,
            SkinLimits {
                max_influences: 8,
                prune: 0.0,
            },
        );
        assert_normalised(&eight);
        assert!(four.width() <= 4);
        let w = eight.width();
        assert!(w > 4 && w <= 8, "width {w}");
        // Capping the eight-influence weights to four keeps the four strongest.
        let capped = limit(&eight, SkinLimits::default());
        assert_normalised(&capped);
        assert!(capped.width() <= 4);
        let v = (0..eight.vertex_count())
            .find(|&v| eight.influences(v).count() > 4)
            .unwrap();
        assert_eq!(capped.joints[v][..], eight.joints[v][..4]);
        let total: f32 = eight.weights[v][..4].iter().sum();
        assert!((capped.weights[v][0] - eight.weights[v][0] / total).abs() < 1e-5);
    }

    #[test]
    fn brush_adds_subtracts_and_smooths() {
        let b = demo_rig(0.0);
        let (mesh, sk) = (&b.mesh, &b.sk);
        let limits = SkinLimits::default();
        let skin = auto(&b, limits);
        let adj = mesh.adjacency();
        let spine = sk.index_of("head").unwrap();
        let upper = sk.index_of("upperarm_l").unwrap();
        // The middle of the left upper arm, where the head has no say.
        let mid = (sk.bones[upper].head + sk.bones[upper].tail) * 0.5;
        let inside: Vec<usize> = (0..mesh.vertex_count())
            .filter(|&v| mesh.positions[v].distance(mid) < 0.06)
            .collect();
        assert!(!inside.is_empty());
        assert!(inside.iter().all(|&v| skin.weight(v, spine) == 0.0));

        let dab = |mode| Dab {
            center: mid,
            radius: 0.08,
            strength: 0.5,
            mode,
        };
        let stroke = |s: &mut SkinWeights, bone: usize, d: Dab| {
            paint(s, &mesh.positions, &adj, sk, bone, d, limits)
        };
        let mut painted = skin.clone();
        let changed = stroke(&mut painted, spine, dab(BrushMode::Add));
        assert!(changed >= inside.len());
        assert_normalised(&painted);
        assert!(painted.width() <= 4);
        assert!(inside.iter().all(|&v| painted.weight(v, spine) > 0.05));
        // Outside the radius nothing moves.
        for v in 0..mesh.vertex_count() {
            if mesh.positions[v].distance(mid) >= 0.08 {
                assert_eq!(painted.joints[v], skin.joints[v]);
                assert_eq!(painted.weights[v], skin.weights[v]);
            }
        }
        // A newly painted bone survives the influence cap even while small.
        let full = (0..mesh.vertex_count())
            .find(|&v| skin.influences(v).count() == 4 && skin.weight(v, spine) == 0.0)
            .unwrap();
        let mut faint = skin.clone();
        let tiny = Dab {
            center: mesh.positions[full],
            radius: 0.01,
            strength: 0.05,
            mode: BrushMode::Add,
        };
        stroke(&mut faint, spine, tiny);
        assert!(faint.weight(full, spine) > 0.0);
        assert!(faint.influences(full).count() <= 4);

        // Subtracting takes it away again; the arm takes the weight back.
        let before: Vec<f32> = inside.iter().map(|&v| painted.weight(v, spine)).collect();
        stroke(&mut painted, spine, dab(BrushMode::Subtract));
        assert_normalised(&painted);
        for (k, &v) in inside.iter().enumerate() {
            assert!(painted.weight(v, spine) < before[k]);
        }
        // Subtracting a bone that holds a vertex alone hands it to its parent.
        let hand = sk.index_of("hand_l").unwrap();
        let tip = (0..mesh.vertex_count())
            .max_by(|a, b| mesh.positions[*a].x.total_cmp(&mesh.positions[*b].x))
            .unwrap();
        let mut alone = skin.clone();
        (alone.joints[tip], alone.weights[tip]) = one(hand as u16);
        let wipe = Dab {
            center: mesh.positions[tip],
            radius: 0.01,
            strength: 1.0,
            mode: BrushMode::Subtract,
        };
        stroke(&mut alone, hand, wipe);
        assert_normalised(&alone);
        assert_eq!(alone.weight(tip, hand), 0.0);
        assert!(alone.weight(tip, sk.bones[hand].parent.unwrap()) > 0.99);

        // Smoothing evens out a hard edge.
        let mut edge = skin.clone();
        for &v in &inside {
            if mesh.positions[v].x < mid.x {
                (edge.joints[v], edge.weights[v]) = one(spine as u16);
            }
        }
        let spread = |s: &SkinWeights| {
            inside
                .iter()
                .map(|&v| s.weight(v, spine))
                .fold((1.0f32, 0.0f32), |(lo, hi), w| (lo.min(w), hi.max(w)))
        };
        let (lo0, hi0) = spread(&edge);
        for _ in 0..4 {
            stroke(&mut edge, spine, dab(BrushMode::Smooth));
        }
        let (lo1, hi1) = spread(&edge);
        assert_normalised(&edge);
        assert!(hi1 - lo1 < hi0 - lo0, "{lo0}..{hi0} -> {lo1}..{hi1}");
    }

    #[test]
    fn edits_survive_a_rebuild_unless_a_new_bone_claims_the_vertex() {
        let b = demo_rig(0.0);
        let names: Vec<String> = b.sk.bones.iter().map(|b| b.name.clone()).collect();
        let base = auto(&b, SkinLimits::default());
        let mut edited = base.clone();
        let spine = b.sk.index_of("spine_03").unwrap() as u16;
        (edited.joints[0], edited.weights[0]) = one(spine);
        (edited.joints[1], edited.weights[1]) = one(spine);

        // The rebuilt rig has one more bone, first; vertex 1 is now weighted to it.
        let mut new_names = names.clone();
        new_names.insert(0, "extra".into());
        let mut rebuilt = SkinWeights {
            joints: base
                .joints
                .iter()
                .map(|j| j.iter().map(|x| x + 1).collect())
                .collect(),
            weights: base.weights.clone(),
        };
        (rebuilt.joints[1], rebuilt.weights[1]) = one(0);
        (rebuilt.joints[2], rebuilt.weights[2]) = one(5);

        let (kept, n) = keep_edits(&edited, &names, Some(&base), &rebuilt, &new_names);
        assert_eq!(n, 1);
        assert_eq!(
            kept.joints[0],
            vec![spine + 1],
            "the painted vertex keeps its weights"
        );
        assert_eq!(kept.joints[1], vec![0], "the new bone wins its vertex");
        assert_eq!(
            kept.joints[2],
            vec![5],
            "unedited vertices take the new weights"
        );
        // Without the automatic weights every vertex keeps its weights where it can.
        let (_, all) = keep_edits(&edited, &names, None, &rebuilt, &new_names);
        assert_eq!(all, base.vertex_count() - 1);
    }

    #[test]
    fn weights_copy_between_t_and_a_pose() {
        let t = demo_rig(0.0);
        let a = demo_rig(45.0);
        let limits = SkinLimits::default();
        let (t_skin, a_skin) = (auto(&t, limits), auto(&a, limits));
        let by_name = |from: &Skeleton, to: &Skeleton| -> Vec<Option<usize>> {
            to.bones.iter().map(|b| from.index_of(&b.name)).collect()
        };
        for ((src, src_skin), (dst, dst_skin)) in [
            ((&t, &t_skin), (&a, &a_skin)),
            ((&a, &a_skin), (&t, &t_skin)),
        ] {
            let copied = transfer(
                &skinned(src, src_skin),
                &skinned(dst, dst_skin),
                &by_name(&src.sk, &dst.sk),
            );
            assert_normalised(&copied);
            let n = dst.mesh.vertex_count();
            let agree = (0..n)
                .filter(|&v| dominant(&copied, v) == dominant(dst_skin, v))
                .count();
            assert!(
                agree as f32 > 0.9 * n as f32,
                "dominant bone agrees on {agree} of {n}"
            );
            // Forearm and hand vertices land on the same side's arm.
            for v in 0..n {
                let bone = dominant(dst_skin, v);
                let name = &dst.sk.bones[bone].name;
                let on_it = dst.sk.bones[bone].distance_to(dst.mesh.positions[v]) < 0.1;
                if on_it && (name.starts_with("hand_") || name.starts_with("lowerarm_")) {
                    let got = &dst.sk.bones[dominant(&copied, v)].name;

                    assert_eq!(got.ends_with("_l"), name.ends_with("_l"), "{name} -> {got}");
                    assert!(
                        got.contains("arm") || got.starts_with("hand"),
                        "{name} -> {got}"
                    );
                }
            }
        }
        // Copying a rig onto itself changes almost nothing.
        let same = transfer(
            &skinned(&t, &t_skin),
            &skinned(&t, &t_skin),
            &by_name(&t.sk, &t.sk),
        );
        // (Where the demo's body parts pass through each other a vertex can sit
        // exactly on another part's surface and take that part's weights.)
        let n = t_skin.vertex_count();
        let moved = (0..n)
            .filter(|&v| {
                (0..t.sk.bones.len())
                    .any(|b| (same.weight(v, b) - t_skin.weight(v, b)).abs() > 1e-3)
            })
            .count();
        assert!(moved * 100 < n, "{moved} of {n} vertices changed");
    }

    #[test]
    fn turning_a_bone_moves_only_its_skin() {
        // Vertex 0 belongs to the turned bone, vertex 1 to a bone that stays put,
        // vertex 2 is shared half and half.
        let skin = SkinWeights {
            joints: vec![vec![1], vec![0], vec![0, 1]],
            weights: vec![vec![1.0], vec![1.0], vec![0.5, 0.5]],
        };
        let rest = [
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
            Vec3::new(1.0, 0.0, 0.0),
        ];
        let moved = [false, true];
        let posed = deform_turn(
            &rest,
            &skin,
            &moved,
            Vec3::ZERO,
            Vec3::Z,
            std::f32::consts::FRAC_PI_2,
        );
        assert!(posed[0].distance(Vec3::new(0.0, 1.0, 0.0)) < 1e-5);
        assert!(posed[1].distance(Vec3::new(0.0, 1.0, 0.0)) < 1e-5);
        // Half turned and half fixed: the midpoint of the two positions.
        assert!(posed[2].distance(Vec3::new(0.5, 0.5, 0.0)) < 1e-5);
        // No turn, no change.
        let still = deform_turn(&rest, &skin, &[false, false], Vec3::ZERO, Vec3::Z, 1.0);
        assert_eq!(still, rest.to_vec());
    }
}
