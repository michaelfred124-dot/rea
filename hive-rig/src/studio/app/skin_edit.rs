//! The weight-editing tools beyond the brush, as `Studio` operations: locked
//! bones, a vertex selection (click, box, grow, shrink) with copy, paste, flood
//! fill and a weight table, normalising, fixing spikes, putting the automatic
//! weights back on part of the mesh, the vertex under the pointer, and bending
//! the painted bone to watch the skin while painting.
//!
//! Every edit is one undo step on the built rig's weights. The weight maths is
//! in `hive_rig::skin::edit`; this module only feeds it the rig, the selection
//! and the locks.

use std::collections::BTreeSet;
use std::sync::Arc;

use eframe::egui::{Pos2, Rect};
use glam::Vec3;

use hive_rig::checks;
use hive_rig::skin::{self, SkinWeights, edit};
use hive_rig::voxel::weld_positions;

use super::{Pose, Studio, ray_triangle};
use crate::view::Projector;

/// How a pick changes the vertex selection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectOp {
    /// The picked vertices become the selection.
    Replace,
    /// Shift: add them.
    Add,
    /// Ctrl: take them out.
    Remove,
}

/// The weight tools' state: what is selected, locked, copied and hovered.
pub struct SkinTools {
    /// The select tool is on: clicks and drags on the mesh select vertices.
    pub selecting: bool,
    /// One flag per mesh vertex (empty when nothing is selected yet).
    pub selected: Vec<bool>,
    /// Bones (by exported name) whose weights no edit changes.
    pub locked: BTreeSet<String>,
    /// Weights copied from the selection, by bone name, for Paste.
    pub clipboard: Option<Vec<(String, f32)>>,
    /// The vertex under the pointer, for its influences.
    pub hover: Option<usize>,
    /// The weight Flood sets the shown bone to.
    pub flood: f32,
    /// How far a vertex must stand out from its neighbours to count as a spike.
    pub spike: f32,
    /// Where a box selection started and where it is now, while it is dragged.
    pub box_from: Option<Pos2>,
    pub box_to: Option<Pos2>,
    /// Frames left in which Properties scrolls down to the weight tools (for
    /// screenshots that show them).
    pub scroll_to_tools: u8,
    /// The next change in the weight table starts a new undo step (set when a
    /// cell starts being dragged or typed in, so the whole edit is one step).
    pub table_step: bool,
    /// Surface neighbours of the mesh (seam copies welded), built once per character.
    neighbours: Option<Arc<Vec<Vec<u32>>>>,
}

impl Default for SkinTools {
    fn default() -> Self {
        SkinTools {
            selecting: false,
            selected: Vec::new(),
            locked: BTreeSet::new(),
            clipboard: None,
            hover: None,
            flood: 1.0,
            spike: skin::SPIKE,
            box_from: None,
            box_to: None,
            table_step: true,
            scroll_to_tools: 0,
            neighbours: None,
        }
    }
}

impl SkinTools {
    /// The selected vertices, in order.
    pub fn selection(&self) -> Vec<usize> {
        (0..self.selected.len())
            .filter(|&v| self.selected[v])
            .collect()
    }

    pub fn selected_count(&self) -> usize {
        self.selected.iter().filter(|s| **s).count()
    }
}

/// The world axis a bone bends about for the paint preview: the one most
/// across the bone (ties go to X, then Y), so an arm along X bends about Y
/// and a leg or spine along Y about X.
pub fn bend_axis(direction: Vec3) -> usize {
    let mut best = 0;
    for axis in 1..3 {
        if direction[axis].abs() < direction[best].abs() - 1e-4 {
            best = axis;
        }
    }
    best
}

impl Studio {
    /// The weight tools need a built rig.
    fn need_rig(&mut self, what: &str) -> bool {
        if self.doc.as_ref().is_some_and(|d| d.rig.is_some()) {
            return true;
        }
        self.fail(format!("Build the rig first, then {what}."));
        false
    }

    /// One flag per bone of the built rig, set for the locked bones.
    pub fn lock_mask(&self) -> Vec<bool> {
        match self.doc.as_ref().and_then(|d| d.rig.as_ref()) {
            Some(rig) => edit::lock_mask(
                &rig.skeleton,
                self.skin_tools.locked.iter().map(String::as_str),
            ),
            None => Vec::new(),
        }
    }

    /// The mesh's surface neighbours, built the first time they're needed.
    pub fn neighbours(&mut self) -> Arc<Vec<Vec<u32>>> {
        if let Some(n) = &self.skin_tools.neighbours {
            return n.clone();
        }
        let n = Arc::new(
            self.doc
                .as_ref()
                .map(|d| edit::surface_neighbours(&d.mesh))
                .unwrap_or_default(),
        );
        self.skin_tools.neighbours = Some(n.clone());
        n
    }

    /// Replace the built rig's weights with `skin` as one undo step (or, with
    /// `new_step` off, as part of the step already started). Nothing changed
    /// leaves no undo step.
    fn commit_skin(&mut self, skin: SkinWeights, new_step: bool, msg: String) {
        let Some(doc) = &mut self.doc else { return };
        let Some(rig) = &doc.rig else { return };
        if rig.skin == skin {
            self.info(format!("{msg} Nothing changed."));
            return;
        }
        if new_step {
            doc.push_skin_undo();
        }
        doc.rig.as_mut().unwrap().skin = skin;
        doc.exported = false;
        self.weight_colors = None;
        self.info(msg);
    }

    /// Lock or unlock a bone's weights.
    pub fn toggle_lock(&mut self, bone: &str) {
        if !self.skin_tools.locked.remove(bone) {
            self.skin_tools.locked.insert(bone.to_string());
            self.info(format!(
                "Locked {bone}: painting and the weight tools leave its weights alone."
            ));
        } else {
            self.info(format!("Unlocked {bone}."));
        }
    }

    pub fn unlock_all(&mut self) {
        self.skin_tools.locked.clear();
        self.info("Unlocked every bone.");
    }

    /// Make every vertex add up to 1 within the rig's influence limits.
    pub fn normalize_weights(&mut self) {
        if !self.need_rig("normalize its weights") {
            return;
        }
        let locks = self.lock_mask();
        let rig = self.doc.as_ref().unwrap().rig.as_ref().unwrap();
        let (skin, n) = edit::normalize(&rig.skin, rig.guides.skin, &locks);
        self.commit_skin(
            skin,
            true,
            format!("Normalized the weights: {n} vertices changed."),
        );
    }

    /// Replace every spike on the mesh with its neighbours' weights.
    pub fn fix_spikes(&mut self) {
        if !self.need_rig("fix its spikes") {
            return;
        }
        let (locks, adj, threshold) = (self.lock_mask(), self.neighbours(), self.skin_tools.spike);
        let rig = self.doc.as_ref().unwrap().rig.as_ref().unwrap();
        let (skin, n) = edit::fix_spikes(&rig.skin, &adj, threshold, rig.guides.skin, &locks);
        self.commit_skin(
            skin,
            true,
            format!("Fixed {n} spiky vertices (standing out by more than {threshold:.2})."),
        );
    }

    /// Change the vertex selection. Copies of a vertex split at a seam come
    /// along, so edits never tear the seam open.
    pub fn select_vertices(&mut self, vertices: &[usize], op: SelectOp) {
        let Some(doc) = &self.doc else { return };
        let n = doc.mesh.vertex_count();
        let weld = weld_positions(&doc.mesh.positions);
        let picked: BTreeSet<u32> = vertices
            .iter()
            .filter(|&&v| v < n)
            .map(|&v| weld[v])
            .collect();
        let tools = &mut self.skin_tools;
        if tools.selected.len() != n || op == SelectOp::Replace {
            tools.selected = vec![false; n];
        }
        for (v, rep) in weld.iter().enumerate() {
            if picked.contains(rep) {
                tools.selected[v] = op != SelectOp::Remove;
            }
        }
    }

    pub fn select_none(&mut self) {
        self.skin_tools.selected.clear();
    }

    /// Select every vertex the shown bone influences.
    pub fn select_bone_skin(&mut self) {
        let Some(bone) = self.paint_bone() else {
            return self.fail("Pick a bone under Weights to select the skin it moves.");
        };
        let rig = self.doc.as_ref().unwrap().rig.as_ref().unwrap();
        let vs: Vec<usize> = (0..rig.skin.vertex_count())
            .filter(|&v| rig.skin.weight(v, bone) > 0.0)
            .collect();
        self.select_vertices(&vs, SelectOp::Replace);
        self.info(format!("Selected {} vertices.", vs.len()));
    }

    /// Add every neighbour of the selection.
    pub fn grow_selection(&mut self) {
        let adj = self.neighbours();
        self.skin_tools.selected = edit::grow(&self.skin_tools.selected, &adj);
    }

    /// Take the selection's border off.
    pub fn shrink_selection(&mut self) {
        let adj = self.neighbours();
        self.skin_tools.selected = edit::shrink(&self.skin_tools.selected, &adj);
    }

    /// Select the vertices whose drawn position falls in `rect` on screen:
    /// only those in front, unless X-ray is on.
    pub fn box_select(&mut self, pr: &Projector, rect: Rect, op: SelectOp) {
        let Some(positions) = self.shown_positions() else {
            return;
        };
        let doc = self.doc.as_ref().unwrap();
        let visible = (!self.xray).then(|| visible_vertices(pr, &doc.mesh, &positions));
        let vs: Vec<usize> = positions
            .iter()
            .enumerate()
            .filter(|(v, p)| {
                pr.pos(**p).is_some_and(|s| rect.contains(s))
                    && visible.as_ref().is_none_or(|vis| vis[*v])
            })
            .map(|(v, _)| v)
            .collect();
        self.select_vertices(&vs, op);
    }

    /// The vertex nearest where a ray first meets the mesh as drawn.
    pub fn hit_vertex(&self, origin: Vec3, dir: Vec3) -> Option<usize> {
        let doc = self.doc.as_ref()?;
        let positions = self.shown_positions()?;
        let mut best: Option<(f32, [usize; 3])> = None;
        for tri in doc.mesh.triangles() {
            let [a, b, c] = tri.map(|i| positions[i]);
            if let Some(t) = ray_triangle(origin, dir, a, b, c)
                && best.is_none_or(|(bt, _)| t < bt)
            {
                best = Some((t, tri));
            }
        }
        let (t, tri) = best?;
        let at = origin + dir * t;
        tri.into_iter().min_by(|a, b| {
            positions[*a]
                .distance(at)
                .total_cmp(&positions[*b].distance(at))
        })
    }

    /// The selection, or a message when it is empty.
    fn selection_or_fail(&mut self, what: &str) -> Option<Vec<usize>> {
        let sel = self.skin_tools.selection();
        if sel.is_empty() {
            self.fail(format!("Select some vertices first, then {what}."));
            return None;
        }
        Some(sel)
    }

    /// Copy the selection's weights (their average over several vertices).
    pub fn copy_vertex_weights(&mut self) {
        if !self.need_rig("copy weights") {
            return;
        }
        let Some(sel) = self.selection_or_fail("copy their weights") else {
            return;
        };
        let rig = self.doc.as_ref().unwrap().rig.as_ref().unwrap();
        let copied: Vec<(String, f32)> = rig
            .skin
            .average(sel.iter().copied())
            .into_iter()
            .map(|(j, w)| (rig.skeleton.bones[j as usize].name.clone(), w))
            .collect();
        let text: Vec<String> = copied.iter().map(|(b, w)| format!("{b} {w:.2}")).collect();
        let from = if sel.len() == 1 {
            format!("vertex {}", sel[0])
        } else {
            format!("the average of {} vertices", sel.len())
        };
        self.skin_tools.clipboard = Some(copied);
        self.info(format!("Copied {from}: {}.", text.join(", ")));
    }

    /// Paste the copied weights onto every selected vertex.
    pub fn paste_vertex_weights(&mut self) {
        if !self.need_rig("paste weights") {
            return;
        }
        let Some(copied) = self.skin_tools.clipboard.clone() else {
            return self.fail("Copy a vertex's weights first, then paste them.");
        };
        let Some(sel) = self.selection_or_fail("paste onto them") else {
            return;
        };
        let locks = self.lock_mask();
        let rig = self.doc.as_ref().unwrap().rig.as_ref().unwrap();
        let pairs: Vec<(u16, f32)> = copied
            .iter()
            .filter_map(|(b, w)| Some((rig.skeleton.index_of(b)? as u16, *w)))
            .collect();
        if pairs.is_empty() {
            return self.fail("None of the copied bones are in this rig.");
        }
        let mut skin = rig.skin.clone();
        let n = edit::paste(&mut skin, &sel, &pairs, rig.guides.skin, &locks);
        self.commit_skin(skin, true, format!("Pasted the weights onto {n} vertices."));
    }

    /// Set the shown bone's weight to the flood value on every selected vertex.
    pub fn flood_weight(&mut self) {
        let Some(bone) = self.paint_bone() else {
            return self.fail("Build the rig and pick a bone under Weights, then flood it.");
        };
        let Some(sel) = self.selection_or_fail("flood them") else {
            return;
        };
        let value = self.skin_tools.flood;
        self.set_weight_on(&sel, bone, value, true);
    }

    /// Set one bone's weight on some vertices (a weight table cell, or a
    /// flood), as a new undo step or as part of the one being dragged.
    pub fn set_weight_on(&mut self, vertices: &[usize], bone: usize, value: f32, new_step: bool) {
        if !self.need_rig("edit its weights") {
            return;
        }
        let locks = self.lock_mask();
        let rig = self.doc.as_ref().unwrap().rig.as_ref().unwrap();
        let Some(name) = rig.skeleton.bones.get(bone).map(|b| b.name.clone()) else {
            return;
        };
        if locks.get(bone).copied().unwrap_or(false) {
            return self.fail(format!(
                "{name} is locked; unlock it to change its weights."
            ));
        }
        let mut skin = rig.skin.clone();
        let n = edit::set_weight(
            &mut skin,
            &rig.skeleton,
            vertices,
            bone,
            value,
            rig.guides.skin,
            &locks,
        );
        let msg = if vertices.len() == 1 {
            format!("Set {name} to {value:.3} on vertex {}.", vertices[0])
        } else {
            format!("Set {name} to {value:.3} on {n} vertices.")
        };
        self.commit_skin(skin, new_step, msg);
    }

    /// Put the automatic weights back on the selected vertices.
    pub fn reauto_selection(&mut self) {
        if !self.need_rig("re-run its automatic weights") {
            return;
        }
        let Some(sel) = self.selection_or_fail("re-run the automatic weights on them") else {
            return;
        };
        self.reauto(&sel, format!("{} selected vertices", sel.len()));
    }

    /// Put the automatic weights back wherever the shown bone has (or would
    /// have) weight.
    pub fn reauto_bone(&mut self) {
        let Some(bone) = self.paint_bone() else {
            return self.fail("Pick a bone under Weights, then re-run its automatic weights.");
        };
        let auto = self.auto_weights();
        let rig = self.doc.as_ref().unwrap().rig.as_ref().unwrap();
        let name = rig.skeleton.bones[bone].name.clone();
        let region = edit::bone_region(&rig.skin, &auto, bone);
        self.reauto(&region, format!("{name}'s {} vertices", region.len()));
    }

    /// The built-skin checks on the current weights (vertices with no weight,
    /// deforming bones with no skin), or `None` before Build.
    pub fn skin_report(&self) -> Option<checks::SkinReport> {
        let rig = self.doc.as_ref()?.rig.as_ref()?;
        Some(checks::check_skin(&rig.skeleton, &rig.skin))
    }

    /// Select the vertices with no weight, to look at or paint them.
    pub fn select_unweighted(&mut self) {
        let Some(report) = self.skin_report() else {
            return self.fail("Build the rig first, then check its skin.");
        };
        if report.unweighted.is_empty() {
            return self.info("Every vertex has weight.");
        }
        self.select_vertices(&report.unweighted, SelectOp::Replace);
        self.skin_tools.selecting = true;
        self.info(format!(
            "Selected the {} vertices with no weight.",
            report.unweighted.len()
        ));
    }

    /// Give every vertex with no weight the weights around it, as one undo step.
    pub fn fill_unweighted(&mut self) {
        if !self.need_rig("fill the vertices with no weight") {
            return;
        }
        let adjacency = self.neighbours();
        let rig = self.doc.as_ref().unwrap().rig.as_ref().unwrap();
        let (skin, n) = edit::fill_unweighted(&rig.skin, &adjacency, rig.guides.skin);
        self.commit_skin(
            skin,
            true,
            format!("Filled {n} vertices with no weight from the weights around them."),
        );
    }

    /// Show a bone with no skin under Weights, ready to paint or re-run.
    pub fn show_bone_weights(&mut self, bone: &str) {
        self.weights_bone = Some(bone.to_string());
        self.weight_colors = None;
    }

    /// Re-run the automatic weights for one bone (a bone the checks found
    /// without skin), as one undo step.
    pub fn reauto_named_bone(&mut self, bone: &str) {
        self.show_bone_weights(bone);
        self.reauto_bone();
    }

    /// The automatic weights of the built rig: the ones it was built with, or
    /// computed again (geodesic voxel binding) when they aren't known.
    fn auto_weights(&mut self) -> SkinWeights {
        let doc = self.doc.as_mut().unwrap();
        let rig = doc.rig.as_ref().unwrap();
        doc.auto_skin
            .get_or_insert_with(|| rig.auto_weights(skin::SkinOptions::default()))
            .clone()
    }

    fn reauto(&mut self, vertices: &[usize], what: String) {
        let locks = self.lock_mask();
        let auto = self.auto_weights();
        let rig = self.doc.as_ref().unwrap().rig.as_ref().unwrap();
        let mut skin = rig.skin.clone();
        let n = edit::restore(&mut skin, &auto, vertices, rig.guides.skin, &locks);
        self.commit_skin(
            skin,
            true,
            format!("Re-ran the automatic weights on {what}: {n} changed."),
        );
    }

    /// The preview skeleton's name for the painted bone (the test pose uses
    /// those names; the weights list uses the exported ones).
    fn paint_bone_internal(&self) -> Option<(String, Vec3)> {
        let bone = self.paint_bone()?;
        let doc = self.doc.as_ref()?;
        let b = doc.preview.as_ref().ok()?.skeleton.bones.get(bone)?;
        Some((b.name.clone(), b.direction()))
    }

    /// How far the painted bone is bent for the paint preview (0 when the test
    /// pose is on another bone or off).
    pub fn paint_bend(&self) -> f32 {
        match (self.paint_bone_internal(), &self.pose) {
            (Some((name, _)), Some(p)) if p.bone == name => p.degrees,
            _ => 0.0,
        }
    }

    /// The axis the painted bone bends about: the test pose's when it is on
    /// that bone, else the one across the bone (see [`bend_axis`]).
    pub fn paint_bend_axis(&self) -> Option<usize> {
        let (bone, dir) = self.paint_bone_internal()?;
        Some(match &self.pose {
            Some(p) if p.bone == bone => p.axis,
            _ => bend_axis(dir),
        })
    }

    /// Bend the painted bone at its joint by `degrees` about world axis `axis`
    /// (0 = X, 1 = Y, 2 = Z; `None` keeps the current one), through the test
    /// pose, so the skin deforms while it is painted. 0 straightens it.
    pub fn bend_paint_bone(&mut self, degrees: f32, axis: Option<usize>) {
        let Some((bone, _)) = self.paint_bone_internal() else {
            return;
        };
        let Some(axis) = axis.or_else(|| self.paint_bend_axis()) else {
            return;
        };
        self.pose = Some(Pose {
            bone,
            axis: axis.min(2),
            degrees,
        });
    }
}

/// Which vertices are in front, seen through `pr`: a coarse depth buffer of
/// the drawn triangles, each vertex compared with the nearest surface over it.
fn visible_vertices(pr: &Projector, mesh: &hive_rig::mesh::Mesh, positions: &[Vec3]) -> Vec<bool> {
    const CELL: f32 = 3.0;
    let r = pr.rect;
    let (w, h) = (
        (r.width() / CELL).ceil().max(1.0) as usize,
        (r.height() / CELL).ceil().max(1.0) as usize,
    );
    let depth = |p: Vec3| (p - pr.eye).dot(pr.forward);
    let cell = |s: Pos2| -> (f32, f32) { ((s.x - r.left()) / CELL, (s.y - r.top()) / CELL) };
    let screen: Vec<Option<(f32, f32, f32)>> = positions
        .iter()
        .map(|&p| {
            let s = pr.pos(p)?;
            let (x, y) = cell(s);
            let d = depth(p);
            (d > 0.0).then_some((x, y, d))
        })
        .collect();
    let mut zbuf = vec![f32::INFINITY; w * h];
    for [a, b, c] in mesh.triangles() {
        let (Some(pa), Some(pb), Some(pc)) = (screen[a], screen[b], screen[c]) else {
            continue;
        };
        let x0 = pa.0.min(pb.0).min(pc.0).floor().max(0.0) as usize;
        let x1 = (pa.0.max(pb.0).max(pc.0).ceil() as usize).min(w - 1);
        let y0 = pa.1.min(pb.1).min(pc.1).floor().max(0.0) as usize;
        let y1 = (pa.1.max(pb.1).max(pc.1).ceil() as usize).min(h - 1);
        let area = (pb.0 - pa.0) * (pc.1 - pa.1) - (pc.0 - pa.0) * (pb.1 - pa.1);
        if area.abs() < 1e-9 || x0 > x1 || y0 > y1 {
            continue;
        }
        for y in y0..=y1 {
            for x in x0..=x1 {
                let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
                let wa = ((pb.0 - px) * (pc.1 - py) - (pc.0 - px) * (pb.1 - py)) / area;
                let wb = ((pc.0 - px) * (pa.1 - py) - (pa.0 - px) * (pc.1 - py)) / area;
                let wc = 1.0 - wa - wb;
                if wa < 0.0 || wb < 0.0 || wc < 0.0 {
                    continue;
                }
                let d = wa * pa.2 + wb * pb.2 + wc * pc.2;
                let z = &mut zbuf[y * w + x];
                *z = z.min(d);
            }
        }
    }
    let (lo, hi) = mesh.bounds();
    let tolerance = 0.015 * (hi - lo).max_element().max(1e-3);
    screen
        .iter()
        .map(|s| {
            let Some((x, y, d)) = *s else { return false };
            let (cx, cy) = (x.floor() as isize, y.floor() as isize);
            // The nearest surface in the vertex's cell and the ones around it,
            // so a vertex on the silhouette still counts.
            let mut nearest = f32::INFINITY;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    let (xx, yy) = (cx + dx, cy + dy);
                    if xx >= 0 && yy >= 0 && (xx as usize) < w && (yy as usize) < h {
                        nearest = nearest.min(zbuf[yy as usize * w + xx as usize]);
                    }
                }
            }
            !nearest.is_finite() || d <= nearest + tolerance
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::super::{Brush, Rig};
    use super::*;
    use crate::view::Camera;
    use hive_rig::skin::BrushMode;

    fn built() -> Studio {
        let mut s = Studio::default();
        s.load_demo();
        assert!(s.build(), "{}", s.status);
        s
    }

    fn rig(s: &Studio) -> &Rig {
        s.doc.as_ref().unwrap().rig.as_ref().unwrap()
    }

    fn bone(s: &Studio, name: &str) -> usize {
        rig(s).skeleton.index_of(name).unwrap()
    }

    /// The vertex nearest the middle of a bone.
    fn vertex_on(s: &Studio, name: &str) -> usize {
        let b = &rig(s).skeleton.bones[bone(s, name)];
        let mid = (b.head + b.tail) * 0.5;
        let m = &rig(s).mesh.positions;
        (0..m.len())
            .min_by(|a, b| m[*a].distance(mid).total_cmp(&m[*b].distance(mid)))
            .unwrap()
    }

    fn sums_to_one(s: &Studio) {
        let skin = &rig(s).skin;
        for v in 0..skin.vertex_count() {
            let t: f32 = skin.influences(v).map(|(_, w)| w).sum();
            assert!((t - 1.0).abs() < 1e-4, "vertex {v}: {t}");
        }
    }

    #[test]
    fn a_locked_bone_is_left_alone_by_the_brush_and_one_undo_takes_a_stroke_back() {
        let mut s = built();
        s.symmetry = false;
        s.weights_bone = Some("lowerarm_l".into());
        s.toggle_lock("upperarm_twist_01_l");
        let elbow = rig(&s).skeleton.bones[bone(&s, "lowerarm_l")].head;
        let before = rig(&s).skin.clone();
        let twist = bone(&s, "upperarm_twist_01_l");
        s.brush = Brush {
            mode: BrushMode::Add,
            radius: 0.05,
            strength: 1.0,
            value: 1.0,
        };
        // A stroke of several dabs is one undo step.
        assert!(s.begin_stroke());
        let mut changed = 0;
        for k in 0..4 {
            changed += s.paint_at(elbow + Vec3::X * (0.01 * k as f32));
        }
        assert!(changed > 0);
        sums_to_one(&s);
        for v in 0..before.vertex_count() {
            assert!((rig(&s).skin.weight(v, twist) - before.weight(v, twist)).abs() < 1e-5);
        }
        // Painting the locked bone itself is refused, leaving no undo step.
        s.weights_bone = Some("upperarm_twist_01_l".into());
        let painted = rig(&s).skin.clone();
        assert!(!s.begin_stroke());
        assert!(s.error, "{}", s.status);
        assert_eq!(s.paint_at(elbow), 0);
        assert_eq!(rig(&s).skin, painted);
        s.undo();
        assert_eq!(rig(&s).skin, before);
        s.unlock_all();
        assert!(s.lock_mask().iter().all(|l| !l));
    }

    #[test]
    fn select_copy_paste_flood_and_undo() {
        let mut s = built();
        let (from, to) = (vertex_on(&s, "thigh_l"), vertex_on(&s, "upperarm_r"));
        let before = rig(&s).skin.clone();
        s.select_vertices(&[from], SelectOp::Replace);
        s.copy_vertex_weights();
        assert!(!s.error, "{}", s.status);
        assert!(s.skin_tools.clipboard.is_some());
        // Paste onto the right arm vertex and its neighbours.
        s.select_vertices(&[to], SelectOp::Replace);
        s.grow_selection();
        let sel = s.skin_tools.selection();
        assert!(sel.len() > 3);
        s.paste_vertex_weights();
        assert!(!s.error, "{}", s.status);
        for &v in &sel {
            for (j, w) in before.influences(from) {
                assert!((rig(&s).skin.weight(v, j) - w).abs() < 1e-5);
            }
        }
        sums_to_one(&s);
        // Flood half of the spine onto the same vertices, as one more step.
        s.weights_bone = Some("spine_01".into());
        s.skin_tools.flood = 0.5;
        s.flood_weight();
        let spine = bone(&s, "spine_01");
        assert!(
            sel.iter()
                .all(|&v| (rig(&s).skin.weight(v, spine) - 0.5).abs() < 1e-5)
        );
        sums_to_one(&s);
        s.undo();
        assert!(sel.iter().all(|&v| rig(&s).skin.weight(v, spine) < 0.5));
        s.undo();
        assert_eq!(rig(&s).skin, before);
        // Nothing selected: the tools say so and change nothing.
        s.select_none();
        s.paste_vertex_weights();
        assert!(s.error);
        assert_eq!(rig(&s).skin, before);
        // Grow then shrink, and the selection is the single ring again.
        s.select_vertices(&[to], SelectOp::Replace);
        s.grow_selection();
        let ring = s.skin_tools.selected_count();
        s.grow_selection();
        s.shrink_selection();
        assert!(s.skin_tools.selected_count() >= ring);
        s.select_vertices(&[to], SelectOp::Remove);
        assert!(!s.skin_tools.selected[to]);
    }

    #[test]
    fn weight_table_edits_drag_as_one_step() {
        let mut s = built();
        let v = vertex_on(&s, "calf_l");
        let calf = bone(&s, "calf_l");
        let before = rig(&s).skin.clone();
        // Dragging a cell: the first change starts the step, the rest join it.
        s.set_weight_on(&[v], calf, 0.6, true);
        s.set_weight_on(&[v], calf, 0.4, false);
        s.set_weight_on(&[v], calf, 0.3, false);
        assert!((rig(&s).skin.weight(v, calf) - 0.3).abs() < 1e-5);
        sums_to_one(&s);
        s.undo();
        assert_eq!(rig(&s).skin, before);
        assert!(s.doc.as_ref().unwrap().skin_undo.is_empty());
    }

    #[test]
    fn normalize_fix_spikes_and_reauto() {
        let mut s = built();
        let v = vertex_on(&s, "upperarm_l");
        let head = bone(&s, "head");
        let auto = rig(&s).skin.clone();
        // A spike: one arm vertex given to the head.
        s.set_weight_on(&[v], head, 1.0, true);
        assert_eq!(rig(&s).skin.weight(v, head), 1.0);
        s.fix_spikes();
        assert!(!s.error, "{}", s.status);
        assert_eq!(rig(&s).skin.weight(v, head), 0.0);
        sums_to_one(&s);
        // Re-running the automatic weights on the upper arm puts them back.
        s.set_weight_on(&[v], head, 1.0, true);
        s.weights_bone = Some("upperarm_l".into());
        s.reauto_bone();
        assert_eq!(rig(&s).skin.weight(v, head), 0.0);
        let up = bone(&s, "upperarm_l");
        assert!((rig(&s).skin.weight(v, up) - auto.weight(v, up)).abs() < 1e-5);
        // ...and on a selection.
        s.set_weight_on(&[v], head, 1.0, true);
        s.select_vertices(&[v], SelectOp::Replace);
        s.reauto_selection();
        for b in 0..rig(&s).skeleton.bones.len() {
            assert!((rig(&s).skin.weight(v, b) - auto.weight(v, b)).abs() < 1e-5);
        }
        // Normalize finds nothing to do on good weights, and leaves no step.
        let steps = s.doc.as_ref().unwrap().skin_undo.len();
        s.normalize_weights();
        assert_eq!(s.doc.as_ref().unwrap().skin_undo.len(), steps);
        // Each of the six edits was one undo step.
        for _ in 0..6 {
            s.undo();
        }
        assert_eq!(rig(&s).skin, auto);
    }

    #[test]
    fn clicks_and_boxes_select_the_vertices_in_front() {
        let mut s = built();
        let rect = Rect::from_min_size(Pos2::ZERO, eframe::egui::vec2(1400.0, 860.0));
        let (min, max) = s.doc.as_ref().unwrap().mesh.bounds();
        s.camera = Camera::default();
        s.camera.frame(min, max);
        let pr = s.camera.projector(rect);
        // A click on the middle of the chest picks a vertex on the front.
        let chest = pr.pos(Vec3::new(0.0, 1.3, 0.0)).unwrap();
        let (o, d) = pr.ray(chest);
        let v = s.hit_vertex(o, d).expect("the ray hits the chest");
        let p = s.doc.as_ref().unwrap().mesh.positions[v];
        assert!(p.z > 0.0 && (p.y - 1.3).abs() < 0.1, "{p}");
        // A box over the chest takes front vertices only; with X-ray the back too.
        let r = Rect::from_center_size(chest, eframe::egui::vec2(60.0, 60.0));
        s.box_select(&pr, r, SelectOp::Replace);
        let front = s.skin_tools.selection();
        assert!(!front.is_empty());
        let pos = s.doc.as_ref().unwrap().mesh.positions.clone();
        assert!(
            front.iter().all(|&v| pos[v].z > -0.02),
            "a back vertex was selected"
        );
        s.xray = true;
        s.box_select(&pr, r, SelectOp::Replace);
        let through = s.skin_tools.selection();
        assert!(through.len() > front.len());
        assert!(through.iter().any(|&v| pos[v].z < -0.05));
        // Ctrl takes the box out again.
        s.box_select(&pr, r, SelectOp::Remove);
        assert_eq!(s.skin_tools.selected_count(), 0);
    }

    #[test]
    fn bending_the_painted_bone_moves_its_skin() {
        let mut s = built();
        s.weights_bone = Some("lowerarm_l".into());
        assert_eq!(s.paint_bend(), 0.0);
        s.bend_paint_bone(90.0, None);
        assert_eq!(s.paint_bend(), 90.0);
        let pose = s.pose.clone().unwrap();
        // The forearm lies along X, so it bends about Y.
        assert_eq!((pose.bone.as_str(), pose.axis), ("lowerarm_l", 1));
        let frame = s.frame().unwrap();
        let posed = frame.mesh.unwrap();
        let hand = vertex_on(&s, "hand_l");
        let rest = s.doc.as_ref().unwrap().mesh.positions[hand];
        assert!(posed[hand].distance(rest) > 0.1);
        // Painting now lands on the bent skin.
        s.show_brush();
        assert!(s.brush_at.is_some());
        // Another bone shows straight until it is bent.
        s.weights_bone = Some("calf_l".into());
        assert_eq!(s.paint_bend(), 0.0);
        assert_eq!(bend_axis(Vec3::new(0.0, -1.0, 0.05)), 0);
        // The axis can be chosen, and stays while the angle changes.
        s.weights_bone = Some("lowerarm_l".into());
        s.bend_paint_bone(30.0, Some(2));
        s.bend_paint_bone(45.0, None);
        assert_eq!(
            s.pose.as_ref().map(|p| (p.axis, p.degrees)),
            Some((2, 45.0))
        );
        s.bend_paint_bone(0.0, None);
        assert_eq!(s.paint_bend(), 0.0);
    }
}
