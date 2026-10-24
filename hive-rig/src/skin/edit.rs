//! Hand edits on skin weights beyond the brush: locked bones, normalising,
//! finding and fixing spikes, setting one bone's weight on chosen vertices
//! (flood fill and the weight table), copying and pasting weights, putting the
//! automatic weights back on part of the mesh, and growing or shrinking a
//! vertex selection over the surface.
//!
//! Every edit takes a lock mask, one flag per bone: a locked bone's weight on a
//! vertex never changes, and the unlocked bones share what it leaves. Every
//! edit also applies the rig's influence limits, so the result adds up to 1
//! and is ready to export.

use super::{SkinLimits, SkinWeights, limit_pairs, relock, spike, surface_adjacency, with_weight};
use crate::mesh::Mesh;
use crate::skeleton::Skeleton;

/// One flag per bone of `sk`, set for the bones named in `names`. Names that
/// aren't bones of `sk` are ignored.
pub fn lock_mask<'a>(sk: &Skeleton, names: impl IntoIterator<Item = &'a str>) -> Vec<bool> {
    let mut mask = vec![false; sk.bones.len()];
    for name in names {
        if let Some(i) = sk.index_of(name) {
            mask[i] = true;
        }
    }
    mask
}

/// Each vertex's neighbours over the surface, with vertices at the same
/// position (split at UV or normal seams) counted as one, so edits and
/// selections cross seams.
pub fn surface_neighbours(mesh: &Mesh) -> Vec<Vec<u32>> {
    surface_adjacency(mesh)
}

/// Make every vertex add up to 1 and keep to the rig's limits, leaving the
/// locked bones' weights alone. Returns the weights and how many vertices
/// changed.
pub fn normalize(skin: &SkinWeights, limits: SkinLimits, locked: &[bool]) -> (SkinWeights, usize) {
    let mut out = skin.clone();
    let mut changed = 0;
    for v in 0..skin.vertex_count() {
        let old = skin.pairs(v);
        let mut pairs = old.clone();
        // A vertex that doesn't add up to 1 is scaled as a whole first: its
        // locked weights mean nothing until it does.
        let total: f32 = pairs.iter().map(|p| p.1).sum();
        if (total - 1.0).abs() > 1e-4 && total > 0.0 {
            pairs.iter_mut().for_each(|p| p.1 /= total);
        }
        limit_pairs(&mut pairs, limits, None, locked);
        if !same_pairs(&old, &pairs) {
            out.set(v, pairs);
            changed += 1;
        }
    }
    (out, changed)
}

/// How far each vertex's weights stand out from the average of its
/// neighbours' (see [`super::SPIKE`]): 0 where they agree, up to 1 where they share
/// no bone. Vertices without neighbours score 0.
pub fn spikes(skin: &SkinWeights, adjacency: &[Vec<u32>]) -> Vec<f32> {
    (0..skin.vertex_count())
        .map(|v| match adjacency.get(v) {
            Some(nbrs) if !nbrs.is_empty() => spike(
                &skin.pairs(v),
                &skin.average(nbrs.iter().map(|&n| n as usize)),
            ),
            _ => 0.0,
        })
        .collect()
}

/// Fix every spike on the mesh: each vertex whose weights stand out from its
/// neighbours' by more than `threshold` (see [`super::SPIKE`]) takes their average
/// instead. The averages come from the weights before the pass, so one spike
/// doesn't spread into the next. Returns the weights and how many vertices
/// changed.
pub fn fix_spikes(
    skin: &SkinWeights,
    adjacency: &[Vec<u32>],
    threshold: f32,
    limits: SkinLimits,
    locked: &[bool],
) -> (SkinWeights, usize) {
    let mut out = skin.clone();
    let mut changed = 0;
    for (v, score) in spikes(skin, adjacency).into_iter().enumerate() {
        if score <= threshold {
            continue;
        }
        let old = skin.pairs(v);
        let mut pairs = skin.average(adjacency[v].iter().map(|&n| n as usize));
        relock(&old, &mut pairs, locked);
        limit_pairs(&mut pairs, limits, None, locked);
        if !same_pairs(&old, &pairs) {
            out.set(v, pairs);
            changed += 1;
        }
    }
    (out, changed)
}

/// Vertices whose weights add up to less than [`super::NO_WEIGHT`]: they don't
/// follow any bone and stay behind when the rig moves.
pub fn unweighted(skin: &SkinWeights) -> Vec<usize> {
    (0..skin.vertex_count())
        .filter(|&v| skin.influences(v).map(|(_, w)| w).sum::<f32>() < super::NO_WEIGHT)
        .collect()
}

/// Deforming bones of `sk` with no weight above [`super::NO_WEIGHT`] on any
/// vertex: they move nothing.
pub fn unskinned_bones(skin: &SkinWeights, sk: &Skeleton) -> Vec<usize> {
    let mut most = vec![0.0f32; sk.bones.len()];
    for v in 0..skin.vertex_count() {
        for (j, w) in skin.influences(v) {
            if let Some(m) = most.get_mut(j) {
                *m = m.max(w);
            }
        }
    }
    (0..sk.bones.len())
        .filter(|&b| sk.bones[b].deform && most[b] < super::NO_WEIGHT)
        .collect()
}

/// Give every unweighted vertex the average weights of its weighted
/// neighbours over the surface, working in from the weighted skin one ring at
/// a time, so a bare patch takes the weights round its edge. Vertices with no
/// weighted vertex anywhere on their piece keep none. Returns the weights and
/// how many vertices got weights.
pub fn fill_unweighted(
    skin: &SkinWeights,
    adjacency: &[Vec<u32>],
    limits: SkinLimits,
) -> (SkinWeights, usize) {
    let mut out = skin.clone();
    let mut bare: Vec<bool> = vec![false; skin.vertex_count()];
    for v in unweighted(skin) {
        bare[v] = true;
    }
    let mut filled = 0;
    loop {
        // Each ring takes its weights from the skin before the ring, so the
        // result doesn't depend on the vertex order.
        let ring: Vec<(usize, Vec<(u16, f32)>)> = (0..bare.len())
            .filter(|&v| bare[v])
            .filter_map(|v| {
                let from = adjacency
                    .get(v)?
                    .iter()
                    .map(|&n| n as usize)
                    .filter(|&n| !bare[n]);
                let mut pairs = out.average(from);
                if pairs.is_empty() {
                    return None;
                }
                limit_pairs(&mut pairs, limits, None, &[]);
                Some((v, pairs))
            })
            .collect();
        if ring.is_empty() {
            break;
        }
        for (v, pairs) in ring {
            out.set(v, pairs);
            bare[v] = false;
            filled += 1;
        }
    }
    (out, filled)
}

/// Set `bone`'s weight to `value` on each of `vertices` (flood fill, or one
/// cell of the weight table). The vertex's other unlocked bones scale to fill
/// the rest; a vertex held by `bone` alone hands the rest to its nearest
/// deforming parent. A locked `bone` isn't changed. Returns how many vertices
/// changed.
pub fn set_weight(
    skin: &mut SkinWeights,
    sk: &Skeleton,
    vertices: &[usize],
    bone: usize,
    value: f32,
    limits: SkinLimits,
    locked: &[bool],
) -> usize {
    if bone >= sk.bones.len() || locked.get(bone).copied().unwrap_or(false) {
        return 0;
    }
    let fallback = super::fallback_bone(sk, bone);
    let value = value.clamp(0.0, 1.0);
    let b = bone as u16;
    let mut changed = 0;
    for &v in vertices {
        if v >= skin.vertex_count() {
            continue;
        }
        let old = skin.pairs(v);
        let mut pairs = with_weight(&old, b, value, locked, fallback);
        limit_pairs(&mut pairs, limits, (value > 0.0).then_some(b), locked);
        if !same_pairs(&old, &pairs) {
            skin.set(v, pairs);
            changed += 1;
        }
    }
    changed
}

/// Give each of `vertices` the influences `pairs` (copied from another
/// vertex), keeping the locked bones' weights. Returns how many changed.
pub fn paste(
    skin: &mut SkinWeights,
    vertices: &[usize],
    pairs: &[(u16, f32)],
    limits: SkinLimits,
    locked: &[bool],
) -> usize {
    let mut changed = 0;
    for &v in vertices {
        if v >= skin.vertex_count() {
            continue;
        }
        let old = skin.pairs(v);
        let mut new = pairs.to_vec();
        relock(&old, &mut new, locked);
        limit_pairs(&mut new, limits, None, locked);
        if !new.is_empty() && !same_pairs(&old, &new) {
            skin.set(v, new);
            changed += 1;
        }
    }
    changed
}

/// Put the automatic weights `auto` back on `vertices` (re-running the
/// automatic weights on part of the mesh), keeping the locked bones' weights.
/// Returns how many vertices changed.
pub fn restore(
    skin: &mut SkinWeights,
    auto: &SkinWeights,
    vertices: &[usize],
    limits: SkinLimits,
    locked: &[bool],
) -> usize {
    let mut changed = 0;
    for &v in vertices {
        if v >= skin.vertex_count() || v >= auto.vertex_count() {
            continue;
        }
        let old = skin.pairs(v);
        let mut new = auto.pairs(v);
        relock(&old, &mut new, locked);
        limit_pairs(&mut new, limits, None, locked);
        if !new.is_empty() && !same_pairs(&old, &new) {
            skin.set(v, new);
            changed += 1;
        }
    }
    changed
}

/// The vertices `bone` influences now or in the automatic weights: the region
/// re-running the automatic weights for one bone resets.
pub fn bone_region(skin: &SkinWeights, auto: &SkinWeights, bone: usize) -> Vec<usize> {
    (0..skin.vertex_count())
        .filter(|&v| {
            skin.weight(v, bone) > 0.0 || (v < auto.vertex_count() && auto.weight(v, bone) > 0.0)
        })
        .collect()
}

/// The selection with every neighbour of a selected vertex added.
pub fn grow(selected: &[bool], adjacency: &[Vec<u32>]) -> Vec<bool> {
    let mut out = selected.to_vec();
    for (v, nbrs) in adjacency.iter().enumerate() {
        if selected.get(v).copied().unwrap_or(false) {
            for &n in nbrs {
                if let Some(s) = out.get_mut(n as usize) {
                    *s = true;
                }
            }
        }
    }
    out
}

/// The selection without its border: a vertex stays only when every
/// neighbour is selected too.
pub fn shrink(selected: &[bool], adjacency: &[Vec<u32>]) -> Vec<bool> {
    selected
        .iter()
        .enumerate()
        .map(|(v, &on)| {
            on && adjacency.get(v).is_none_or(|nbrs| {
                nbrs.iter()
                    .all(|&n| selected.get(n as usize).copied().unwrap_or(false))
            })
        })
        .collect()
}

/// Whether two influence lists agree to within float noise.
fn same_pairs(a: &[(u16, f32)], b: &[(u16, f32)]) -> bool {
    let weight = |list: &[(u16, f32)], j: u16| -> f32 {
        list.iter().filter(|p| p.0 == j).map(|p| p.1).sum()
    };
    a.iter()
        .chain(b)
        .all(|&(j, _)| (weight(a, j) - weight(b, j)).abs() <= 1e-6)
}

#[cfg(test)]
mod tests {
    use super::super::{BrushMode, Dab, SPIKE, SkinOptions, compute, paint};
    use super::*;
    use crate::{components, demo, landmarks::detect};
    use glam::Vec3;

    struct Demo {
        mesh: Mesh,
        sk: Skeleton,
        skin: SkinWeights,
        adj: Vec<Vec<u32>>,
    }

    fn demo_rig() -> Demo {
        let mesh = demo::humanoid(0.0);
        let guides = components::biped(&detect(&mesh));
        let sk = components::build(&guides).unwrap().skeleton;
        let skin = compute(&mesh, &sk, SkinOptions::default(), SkinLimits::default());
        let adj = surface_neighbours(&mesh);
        Demo {
            mesh,
            sk,
            skin,
            adj,
        }
    }

    #[test]
    fn bare_vertices_and_bones_are_found_and_filled_from_around() {
        let d = demo_rig();
        assert!(unweighted(&d.skin).is_empty());
        assert!(unskinned_bones(&d.skin, &d.sk).is_empty());
        // Strip a patch on the upper arm, and every weight of the left hand.
        let (_, patch) = around(&d, "upperarm_l", 0.04);
        assert!(patch.len() > 5);
        let hand = d.sk.index_of("hand_l").unwrap();
        let mut skin = d.skin.clone();
        for &v in &patch {
            skin.set(v, Vec::new());
        }
        let held: Vec<usize> = (0..skin.vertex_count())
            .filter(|&v| skin.weight(v, hand) > 0.0)
            .collect();
        for &v in &held {
            let rest: Vec<(u16, f32)> = skin
                .influences(v)
                .filter(|(j, _)| *j != hand)
                .map(|(j, w)| (j as u16, w))
                .collect();
            let total: f32 = rest.iter().map(|p| p.1).sum();
            skin.set(
                v,
                rest.into_iter()
                    .map(|(j, w)| (j, w / total.max(1e-9)))
                    .collect(),
            );
        }
        let bare = unweighted(&skin);
        assert!(patch.iter().all(|v| bare.contains(v)));
        assert!(unskinned_bones(&skin, &d.sk).contains(&hand));
        // Filling gives the patch the arm's weights from round its edge.
        let (filled, n) = fill_unweighted(&skin, &d.adj, SkinLimits::default());
        assert_eq!(n, bare.len());
        assert!(unweighted(&filled).is_empty());
        assert_normalised(&filled);
        let up = d.sk.index_of("upperarm_l").unwrap();
        let mean = |s: &SkinWeights| {
            patch.iter().map(|&v| s.weight(v, up)).sum::<f32>() / patch.len() as f32
        };
        assert!(
            (mean(&filled) - mean(&d.skin)).abs() < 0.25,
            "{} vs {}",
            mean(&filled),
            mean(&d.skin)
        );
        // A piece with no weight anywhere stays bare.
        let empty = SkinWeights {
            joints: vec![Vec::new(); 3],
            weights: vec![Vec::new(); 3],
        };
        let (still, n) = fill_unweighted(
            &empty,
            &[vec![1], vec![0, 2], vec![1]],
            SkinLimits::default(),
        );
        assert_eq!((n, unweighted(&still).len()), (0, 3));
    }

    fn assert_normalised(skin: &SkinWeights) {
        for v in 0..skin.vertex_count() {
            let s: f32 = skin.influences(v).map(|(_, w)| w).sum();
            assert!((s - 1.0).abs() < 1e-4, "vertex {v} sums to {s}");
        }
    }

    /// The surface point nearest the middle of a bone, and the vertices
    /// within `r` of it.
    fn around(d: &Demo, bone: &str, r: f32) -> (Vec3, Vec<usize>) {
        let b = &d.sk.bones[d.sk.index_of(bone).unwrap()];
        let mid = (b.head + b.tail) * 0.5;
        let at = d
            .mesh
            .positions
            .iter()
            .copied()
            .min_by(|p, q| p.distance(mid).total_cmp(&q.distance(mid)))
            .unwrap();
        let near = (0..d.mesh.vertex_count())
            .filter(|&v| d.mesh.positions[v].distance(at) < r)
            .collect();
        (at, near)
    }

    #[test]
    fn a_locked_bone_keeps_its_weights_whatever_is_painted() {
        let d = demo_rig();
        let limits = SkinLimits::default();
        // The elbow, where the upper arm and the forearm share the skin.
        let elbow = d.sk.bones[d.sk.index_of("lowerarm_l").unwrap()].head;
        let upper = d.sk.index_of("upperarm_twist_01_l").unwrap();
        let lower = d.sk.index_of("lowerarm_l").unwrap();
        let shared: Vec<usize> = (0..d.mesh.vertex_count())
            .filter(|&v| {
                d.mesh.positions[v].distance(elbow) < 0.06
                    && d.skin.weight(v, upper) > 0.05
                    && d.skin.weight(v, lower) > 0.05
            })
            .collect();
        assert!(!shared.is_empty());
        let locked = lock_mask(&d.sk, ["upperarm_twist_01_l"]);
        for mode in [
            BrushMode::Add,
            BrushMode::Subtract,
            BrushMode::Replace,
            BrushMode::Scale,
            BrushMode::Smooth,
            BrushMode::Mend,
        ] {
            let mut skin = d.skin.clone();
            let dab = Dab {
                center: elbow,
                radius: 0.08,
                strength: 1.0,
                mode,
                value: 0.7,
            };
            paint(
                &mut skin,
                &d.mesh.positions,
                &d.adj,
                &d.sk,
                lower,
                dab,
                limits,
                &locked,
            );
            assert_normalised(&skin);
            for &v in &shared {
                assert!(
                    (skin.weight(v, upper) - d.skin.weight(v, upper)).abs() < 1e-5,
                    "{mode:?} moved the locked bone on vertex {v}"
                );
            }
            if mode == BrushMode::Add {
                // The forearm grew into whatever the lock left free.
                for &v in &shared {
                    let room = 1.0 - d.skin.weight(v, upper);
                    assert!((skin.weight(v, lower) - room).abs() < 1e-4);
                }
            }
        }
        // The locked bone itself can't be painted.
        let mut skin = d.skin.clone();
        let dab = Dab {
            center: elbow,
            radius: 0.08,
            strength: 1.0,
            mode: BrushMode::Add,
            value: 1.0,
        };
        let n = paint(
            &mut skin,
            &d.mesh.positions,
            &d.adj,
            &d.sk,
            upper,
            dab,
            limits,
            &locked,
        );
        assert_eq!(n, 0);
        assert_eq!(skin, d.skin);
        // Smoothing the whole mesh leaves it alone too.
        let smoothed = super::super::smooth(&d.skin, &d.mesh, &d.sk, None, 3, limits, &locked);
        for v in 0..d.mesh.vertex_count() {
            assert!((smoothed.weight(v, upper) - d.skin.weight(v, upper)).abs() < 1e-5);
        }
    }

    #[test]
    fn replace_and_scale_brushes() {
        let d = demo_rig();
        let limits = SkinLimits::default();
        let head = d.sk.index_of("head").unwrap();
        let (mid, near) = around(&d, "upperarm_l", 0.03);
        let dab = |mode, value| Dab {
            center: mid,
            radius: 0.05,
            strength: 1.0,
            mode,
            value,
        };
        let stroke = |skin: &mut SkinWeights, bone: usize, dab: Dab| {
            paint(
                skin,
                &d.mesh.positions,
                &d.adj,
                &d.sk,
                bone,
                dab,
                limits,
                &[],
            )
        };
        // At the brush centre Replace sets the weight outright; nearer the rim
        // it moves part of the way.
        let mut skin = d.skin.clone();
        stroke(&mut skin, head, dab(BrushMode::Replace, 0.4));
        assert_normalised(&skin);
        let centre = near
            .iter()
            .copied()
            .min_by(|a, b| {
                d.mesh.positions[*a]
                    .distance(mid)
                    .total_cmp(&d.mesh.positions[*b].distance(mid))
            })
            .unwrap();
        assert!(
            (skin.weight(centre, head) - 0.4).abs() < 0.08,
            "{}",
            skin.weight(centre, head)
        );
        assert!(near.iter().all(|&v| skin.weight(v, head) <= 0.4 + 1e-4));
        // Scale by a half halves it at the centre.
        let before = skin.weight(centre, head);
        stroke(&mut skin, head, dab(BrushMode::Scale, 0.5));
        assert_normalised(&skin);
        let after = skin.weight(centre, head);
        assert!(
            after < 0.6 * before && after > 0.4 * before,
            "{before} -> {after}"
        );
        // Replace with 0 clears the bone.
        let mut cleared = skin.clone();
        for _ in 0..3 {
            stroke(&mut cleared, head, dab(BrushMode::Replace, 0.0));
        }
        assert_eq!(cleared.weight(centre, head), 0.0);
    }

    /// The demo with one vertex in the middle of the left upper arm given
    /// wholly to the head bone.
    fn spiked() -> (Demo, usize, usize) {
        let mut d = demo_rig();
        let (_, near) = around(&d, "upperarm_l", 0.05);
        let v = near[near.len() / 2];
        let head = d.sk.index_of("head").unwrap();
        d.skin.set(v, vec![(head as u16, 1.0)]);
        (d, v, head)
    }

    #[test]
    fn mend_fixes_a_spike_and_leaves_smooth_skin_alone() {
        let (d, v, head) = spiked();
        let limits = SkinLimits::default();
        let scores = spikes(&d.skin, &d.adj);
        assert!(scores[v] > 0.9, "{}", scores[v]);
        let dab = Dab {
            center: d.mesh.positions[v],
            radius: 0.06,
            strength: 1.0,
            mode: BrushMode::Mend,
            value: 1.0,
        };
        let mut skin = d.skin.clone();
        let upper = d.sk.index_of("upperarm_l").unwrap();
        paint(
            &mut skin,
            &d.mesh.positions,
            &d.adj,
            &d.sk,
            upper,
            dab,
            limits,
            &[],
        );
        assert_normalised(&skin);
        assert_eq!(
            skin.weight(v, head),
            0.0,
            "the spike took its neighbours' weights"
        );
        let average = d.skin.average(d.adj[v].iter().map(|&n| n as usize));
        for (j, w) in average {
            assert!((skin.weight(v, j as usize) - w).abs() < 1e-3);
        }
        // Its neighbours didn't stand out, so they are untouched.
        for &n in &d.adj[v] {
            assert_eq!(skin.pairs(n as usize), d.skin.pairs(n as usize));
        }
        let untouched = (0..skin.vertex_count())
            .filter(|&u| u != v && skin.pairs(u) != d.skin.pairs(u))
            .count();
        assert_eq!(untouched, 0);
    }

    #[test]
    fn fixing_spikes_over_the_whole_mesh() {
        let (d, v, head) = spiked();
        let limits = SkinLimits::default();
        let (fixed, n) = fix_spikes(&d.skin, &d.adj, SPIKE, limits, &[]);
        assert!(n >= 1);
        assert_normalised(&fixed);
        assert_eq!(fixed.weight(v, head), 0.0);
        // The automatic weights have no spikes of their own to speak of.
        let clean = demo_rig();
        let (_, n) = fix_spikes(&clean.skin, &clean.adj, SPIKE, limits, &[]);
        assert!(
            n * 200 < clean.skin.vertex_count(),
            "{n} spikes on clean weights"
        );
        // A locked head keeps even its spike.
        let locked = lock_mask(&d.sk, ["head"]);
        let (kept, _) = fix_spikes(&d.skin, &d.adj, SPIKE, limits, &locked);
        assert_eq!(kept.weight(v, head), 1.0);
    }

    #[test]
    fn flood_paste_restore_and_normalize() {
        let d = demo_rig();
        let limits = SkinLimits::default();
        let (_, near) = around(&d, "thigh_l", 0.05);
        let spine = d.sk.index_of("spine_01").unwrap();
        let mut skin = d.skin.clone();
        // Flood 0.25 of the spine onto the thigh.
        let n = set_weight(&mut skin, &d.sk, &near, spine, 0.25, limits, &[]);
        assert_eq!(n, near.len());
        assert_normalised(&skin);
        for &v in &near {
            assert!((skin.weight(v, spine) - 0.25).abs() < 1e-5);
        }
        // A locked bone isn't flooded.
        let locked = lock_mask(&d.sk, ["spine_01"]);
        let mut same = skin.clone();
        assert_eq!(
            set_weight(&mut same, &d.sk, &near, spine, 0.9, limits, &locked),
            0
        );

        // Copy one vertex and paste it onto another far away.
        let from = near[0];
        let (_, far) = around(&d, "upperarm_r", 0.04);
        let copied = skin.average([from]);
        let n = paste(&mut skin, &far, &copied, limits, &[]);
        assert_eq!(n, far.len());
        for &v in &far {
            assert_eq!(skin.pairs(v), skin.pairs(from));
        }
        // Pasting with the right upper arm locked keeps its share.
        let up_r = d.sk.index_of("upperarm_r").unwrap();
        let mut locked_paste = d.skin.clone();
        let locked = lock_mask(&d.sk, ["upperarm_r"]);
        paste(&mut locked_paste, &far, &copied, limits, &locked);
        assert_normalised(&locked_paste);
        for &v in &far {
            assert!((locked_paste.weight(v, up_r) - d.skin.weight(v, up_r)).abs() < 1e-5);
        }

        // Re-running the automatic weights on the thigh's region puts them back.
        let thigh = d.sk.index_of("thigh_l").unwrap();
        let region = bone_region(&skin, &d.skin, thigh);
        assert!(near.iter().all(|v| region.contains(v)));
        let n = restore(&mut skin, &d.skin, &region, limits, &[]);
        assert!(n >= near.len());
        for &v in &near {
            assert_eq!(skin.pairs(v), d.skin.pairs(v));
        }
        // The pasted right arm took the thigh's weights, so it is in the
        // thigh's region too and got its own automatic weights back.
        assert!(far.iter().all(|v| region.contains(v)));
        assert_eq!(skin.pairs(far[0]), d.skin.pairs(far[0]));

        // Normalising fixes vertices that don't add up, and leaves good ones.
        let mut off = d.skin.clone();
        off.weights[near[0]].iter_mut().for_each(|w| *w *= 3.0);
        let (fixed, n) = normalize(&off, limits, &[]);
        assert_eq!(n, 1);
        assert_normalised(&fixed);
        let (_, n) = normalize(&d.skin, limits, &[]);
        assert_eq!(n, 0);
    }

    #[test]
    fn locked_weights_survive_the_influence_cap() {
        // Five influences on a four-influence rig, the smallest one locked.
        let mut pairs = vec![(0u16, 0.4), (1, 0.3), (2, 0.15), (3, 0.1), (4, 0.05)];
        let locked = [false, false, false, false, true];
        limit_pairs(&mut pairs, SkinLimits::default(), None, &locked);
        assert_eq!(pairs.len(), 4);
        let w4 = pairs.iter().find(|p| p.0 == 4).map(|p| p.1);
        assert!((w4.unwrap() - 0.05).abs() < 1e-6);
        let sum: f32 = pairs.iter().map(|p| p.1).sum();
        assert!((sum - 1.0).abs() < 1e-5);
        assert!(
            !pairs.iter().any(|p| p.0 == 3),
            "the weakest unlocked bone went"
        );
    }

    #[test]
    fn selections_grow_and_shrink_over_the_surface() {
        let d = demo_rig();
        let mut sel = vec![false; d.mesh.vertex_count()];
        let (_, near) = around(&d, "spine_02", 0.04);
        let start = near[0];
        sel[start] = true;
        let grown = grow(&sel, &d.adj);
        let count = |s: &[bool]| s.iter().filter(|x| **x).count();
        assert_eq!(count(&grown), 1 + d.adj[start].len());
        let twice = grow(&grown, &d.adj);
        assert!(count(&twice) > count(&grown));
        // Shrinking takes the outer ring off again.
        let shrunk = shrink(&twice, &d.adj);
        assert!((0..sel.len()).all(|v| !grown[v] || shrunk[v]));
        assert!(count(&shrunk) < count(&twice));
        assert_eq!(count(&shrink(&sel, &d.adj)), 0);
    }
}
