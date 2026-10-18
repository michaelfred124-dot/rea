//! Checks run on a layout before it is built.
//!
//! Most are warnings: the rig still builds, but probably not the way the rigger
//! meant (a guide on the wrong side of the body, an elbow that IK would flip, a
//! hind leg hanging from the head). The one error is a layout that can't build.
//! Each issue names its component and guide, says what to do, and offers a
//! [`Fix`] when the change is mechanical (mirror the healthy twin, bend a
//! joint back, re-attach to the usual bone).
//!
//! Distances are fractions of the body height, so the checks scale with the
//! character.

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use glam::Vec3;

use crate::components::{self, Built, Component, Guides, Kind, mirror_guide, spine_name, twin_id};
use crate::mesh::Mesh;
use crate::skeleton::Side;

/// A sided guide may lean this far across the mid-line before it's on the wrong side.
pub const MIDLINE_TOLERANCE: f32 = 0.005;
/// A limb bends less than this (the joint's distance from the line through its
/// neighbours, over that line's length) counts as straight.
pub const STRAIGHT_BEND: f32 = 0.005;
/// The bend a straightened or flipped joint gets from its fix.
pub const FIX_BEND: f32 = 0.04;
/// Joints closer than this make bones too short to behave.
pub const TOO_CLOSE: f32 = 0.002;
/// A guide this far outside the mesh is clearly out of the body.
pub const OUTSIDE: f32 = 0.03;
/// Twins that differ by more than this aren't mirror images; centre guides this
/// far from the mid-line are off centre.
pub const ASYMMETRY: f32 = 0.02;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Warning,
    /// The layout can't build.
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Check {
    /// The guides don't build.
    Unbuildable,
    /// A left or right guide on the other side of the mid-line.
    WrongSide,
    /// An elbow bending forward, or a knee or hock bending the wrong way.
    BendsWrongWay,
    /// A limb with no bend, so IK can't tell which way to fold it.
    Straight,
    /// A joint guide well outside the mesh.
    OutsideMesh,
    /// Two joints almost on top of each other.
    TooClose,
    /// A left/right pair that isn't a mirror image.
    Asymmetric,
    /// A centre guide away from the mid-line.
    OffCentre,
    /// A component hanging from a bone that makes an odd hierarchy.
    OddParent,
}

impl Check {
    pub fn label(self) -> &'static str {
        match self {
            Check::Unbuildable => "doesn't build",
            Check::WrongSide => "wrong side",
            Check::BendsWrongWay => "bends the wrong way",
            Check::Straight => "straight limb",
            Check::OutsideMesh => "outside the mesh",
            Check::TooClose => "joints too close",
            Check::Asymmetric => "not symmetric",
            Check::OffCentre => "off centre",
            Check::OddParent => "odd parent",
        }
    }
}

/// A mechanical change that resolves an issue.
#[derive(Debug, Clone, PartialEq)]
pub enum Fix {
    /// Set each `(guide, from_guide)` pair's guide on `component` to the mirror
    /// image of `from_guide` on `from` (the same component for eyes and face).
    Mirror {
        component: String,
        from: String,
        pairs: Vec<(String, String)>,
    },
    /// Move one guide.
    Move {
        component: String,
        guide: String,
        to: Vec3,
    },
    /// Hang the component from another bone.
    Attach { component: String, bone: String },
}

#[derive(Debug, Clone, PartialEq)]
pub struct Issue {
    pub severity: Severity,
    pub check: Check,
    /// The component's id (empty for a problem with the whole layout).
    pub component: String,
    pub guide: Option<String>,
    pub message: String,
    /// What to do about it.
    pub suggestion: String,
    pub fix: Option<Fix>,
}

impl Issue {
    /// "arm_l · elbow", or the component alone.
    pub fn place(&self) -> String {
        match (&self.guide, self.component.is_empty()) {
            (_, true) => "layout".into(),
            (Some(g), false) => format!("{} · {g}", self.component),
            (None, false) => self.component.clone(),
        }
    }

    /// A short button label for the fix.
    pub fn fix_label(&self) -> Option<String> {
        Some(match self.fix.as_ref()? {
            Fix::Mirror {
                component, from, ..
            } if component == from => "Mirror L to R".into(),
            Fix::Mirror { from, .. } => format!("Mirror {from}"),
            Fix::Attach { bone, .. } => format!("Hang from {bone}"),
            Fix::Move { .. } => match self.check {
                Check::WrongSide => "Flip across".into(),
                Check::BendsWrongWay | Check::Straight => "Bend it".into(),
                Check::TooClose => "Push apart".into(),
                Check::OffCentre => "Centre it".into(),
                _ => "Fix".into(),
            },
        })
    }

    /// One line for a terminal.
    pub fn line(&self) -> String {
        let what = match self.severity {
            Severity::Warning => "warning",
            Severity::Error => "error",
        };
        let mut s = format!("{what}: {}: {}", self.place(), self.message);
        s.push_str(&format!("\n  -> {}", self.suggestion));
        if let Some(l) = self.fix_label() {
            s.push_str(&format!(" (fixable: {l})"));
        }
        s
    }
}

pub fn warnings(issues: &[Issue]) -> usize {
    issues
        .iter()
        .filter(|i| i.severity == Severity::Warning)
        .count()
}

pub fn errors(issues: &[Issue]) -> usize {
    issues.len() - warnings(issues)
}

// --------------------------------------------------------------------------- the mesh

/// The character's surface, for telling whether a guide is inside the body.
/// Results are cached per position, so re-checking while one guide is dragged
/// only measures the guides that moved.
pub struct Surface {
    tris: Vec<[Vec3; 3]>,
    cache: HashMap<[u32; 3], Option<f32>>,
}

impl Surface {
    pub fn new(mesh: &Mesh) -> Surface {
        Surface {
            tris: mesh
                .triangles()
                .map(|t| t.map(|i| mesh.positions[i]))
                .collect(),
            cache: HashMap::new(),
        }
    }

    /// How far `p` is outside the surface, or `None` inside it. Inside means a
    /// winding number of at least one half, which also holds for meshes made
    /// of overlapping closed parts.
    pub fn outside_by(&mut self, p: Vec3) -> Option<f32> {
        if self.tris.is_empty() {
            return None;
        }
        let key = p.to_array().map(f32::to_bits);
        if let Some(r) = self.cache.get(&key) {
            return *r;
        }
        let mut solid = 0.0f64;
        let mut nearest = f32::INFINITY;
        for t in &self.tris {
            solid += solid_angle(p, t) as f64;
            nearest = nearest.min(p.distance_squared(closest_on_triangle(p, t)));
        }
        let winding = solid / (4.0 * std::f64::consts::PI);
        let r = (winding.abs() < 0.5).then(|| nearest.sqrt());
        if self.cache.len() > 4096 {
            self.cache.clear();
        }
        self.cache.insert(key, r);
        r
    }
}

/// The signed solid angle triangle `t` covers seen from `p` (Van Oosterom and Strackee).
fn solid_angle(p: Vec3, t: &[Vec3; 3]) -> f32 {
    let [a, b, c] = t.map(|v| v - p);
    let (la, lb, lc) = (a.length(), b.length(), c.length());
    let num = a.dot(b.cross(c));
    let den = la * lb * lc + a.dot(b) * lc + b.dot(c) * la + c.dot(a) * lb;
    2.0 * num.atan2(den)
}

/// The point of triangle `t` nearest `p`.
fn closest_on_triangle(p: Vec3, t: &[Vec3; 3]) -> Vec3 {
    let [a, b, c] = *t;
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return a;
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return b;
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        return a + ab * (d1 / (d1 - d3));
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
    if d6 >= 0.0 && d5 <= d6 {
        return c;
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        return a + ac * (d2 / (d2 - d6));
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && (d4 - d3) >= 0.0 && (d5 - d6) >= 0.0 {
        return b + (c - b) * ((d4 - d3) / ((d4 - d3) + (d5 - d6)));
    }
    let denom = va + vb + vc;
    if denom.abs() < f32::EPSILON {
        return a;
    }
    a + ab * (vb / denom) + ac * (vc / denom)
}

// --------------------------------------------------------------------------- checking

/// A problem with one guide, before its fix is chosen.
struct Finding {
    check: Check,
    comp: usize,
    guide: usize,
    message: String,
    suggestion: String,
    /// Where the guide would go to fix it on its own (without its twin).
    nudge: Option<Vec3>,
}

/// Check a layout. `surface` (the character's mesh) enables the inside-the-body
/// check. Errors come first.
pub fn check(g: &Guides, surface: Option<&mut Surface>) -> Vec<Issue> {
    let built = components::build(g).map_err(|e| format!("{e:#}"));
    check_built(g, built.as_ref().map_err(String::as_str), surface)
}

/// [`check`] with the layout already built (or the reason it doesn't build).
pub fn check_built(
    g: &Guides,
    built: Result<&Built, &str>,
    surface: Option<&mut Surface>,
) -> Vec<Issue> {
    let mut out = Vec::new();
    if let Err(e) = built {
        out.push(Issue {
            severity: Severity::Error,
            check: Check::Unbuildable,
            component: String::new(),
            guide: None,
            message: e.to_string(),
            suggestion: "fix this first: the rig can't build until it's resolved".into(),
            fix: None,
        });
    }
    if g.body.height.is_nan() || g.body.height <= 0.0 {
        return out;
    }
    // One finding per guide, in order of importance: once a guide is flagged,
    // the later checks leave it alone.
    let mut found: Vec<Finding> = Vec::new();
    wrong_side(g, &mut found);
    off_centre(g, &mut found);
    too_close(g, &mut found);
    bends(g, &mut found);
    if let Some(s) = surface {
        outside(g, s, &mut found);
    }
    let flagged: HashSet<(usize, usize)> = found.iter().map(|f| (f.comp, f.guide)).collect();
    for f in found {
        let c = &g.components[f.comp];
        let name = c.guides[f.guide].name.clone();
        // A healthy twin is the best fix: mirror it over.
        let fix = match mirror_guide(g, f.comp, f.guide) {
            Some((tc, tg)) if !flagged.contains(&(tc, tg)) => Some(Fix::Mirror {
                component: c.id.clone(),
                from: g.components[tc].id.clone(),
                pairs: vec![(name.clone(), g.components[tc].guides[tg].name.clone())],
            }),
            _ => f.nudge.map(|to| Fix::Move {
                component: c.id.clone(),
                guide: name.clone(),
                to,
            }),
        };
        out.push(Issue {
            severity: Severity::Warning,
            check: f.check,
            component: c.id.clone(),
            guide: Some(name),
            message: f.message,
            suggestion: f.suggestion,
            fix,
        });
    }
    asymmetry(g, &flagged, &mut out);
    if let Ok(b) = built {
        hierarchy(g, b, &mut out);
    }
    out
}

fn cm(metres: f32) -> String {
    format!("{:.1} cm", metres * 100.0)
}

fn side_word(side: Side) -> &'static str {
    match side {
        Side::Left => "left",
        Side::Right => "right",
        Side::Center => "centre",
    }
}

/// The side a guide belongs to: its component's, or for the two-sided eyes and
/// face, its name's `_l` / `_r`.
fn guide_side(c: &Component, name: &str) -> Side {
    match c.kind {
        Kind::Eyes | Kind::Face => {
            if name.ends_with("_l") {
                Side::Left
            } else if name.ends_with("_r") {
                Side::Right
            } else {
                Side::Center
            }
        }
        _ => c.side,
    }
}

fn wrong_side(g: &Guides, found: &mut Vec<Finding>) {
    let h = g.body.height;
    let cx = g.body.center_x;
    for (ci, c) in g.components.iter().enumerate() {
        for (gi, guide) in c.guides.iter().enumerate() {
            let side = guide_side(c, &guide.name);
            if side == Side::Center {
                continue;
            }
            let across = -(guide.position.x - cx) * side.sign();
            if across > MIDLINE_TOLERANCE * h {
                let p = guide.position;
                found.push(Finding {
                    check: Check::WrongSide,
                    comp: ci,
                    guide: gi,
                    message: format!(
                        "this {} guide is {} across the mid-line, on the {} side",
                        side_word(side),
                        cm(across),
                        side_word(if side == Side::Left {
                            Side::Right
                        } else {
                            Side::Left
                        })
                    ),
                    suggestion: format!(
                        "move it back to the character's {} ({} X)",
                        side_word(side),
                        if side == Side::Left { "+" } else { "-" }
                    ),
                    nudge: Some(Vec3::new(2.0 * cx - p.x, p.y, p.z)),
                });
            }
        }
    }
}

/// Centre guides that should sit on the mid-line: the body's own and the face's
/// middle joints. Tails and other chains may swing to a side.
fn off_centre(g: &Guides, found: &mut Vec<Finding>) {
    let h = g.body.height;
    let cx = g.body.center_x;
    let taken = flagged(found);
    for (ci, c) in g.components.iter().enumerate() {
        let names: &[&str] = match c.kind {
            Kind::Root => &["root"],
            Kind::Spine => &["pelvis", "chest"],
            Kind::Head => &["neck", "head", "head_top"],
            Kind::Jaw => &["jaw", "chin"],
            Kind::Face => &["nose", "lip_up", "lip_low"],
            _ => continue,
        };
        if c.side != Side::Center {
            continue;
        }
        for (gi, guide) in c.guides.iter().enumerate() {
            if !names.contains(&guide.name.as_str()) || taken.contains(&(ci, gi)) {
                continue;
            }
            let off = (guide.position.x - cx).abs();
            if off > ASYMMETRY * h {
                let p = guide.position;
                found.push(Finding {
                    check: Check::OffCentre,
                    comp: ci,
                    guide: gi,
                    message: format!("this centre guide is {} off the mid-line", cm(off)),
                    suggestion: "centre guides sit on the mid-line so both sides match; move it back to the middle".into(),
                    nudge: Some(Vec3::new(cx, p.y, p.z)),
                });
            }
        }
    }
}

fn flagged(found: &[Finding]) -> HashSet<(usize, usize)> {
    found.iter().map(|f| (f.comp, f.guide)).collect()
}

/// Pairs of guides that make a bone (by index).
fn bone_pairs(c: &Component) -> Vec<(usize, usize)> {
    let n = c.guides.len();
    match c.kind {
        Kind::Root | Kind::Eyes | Kind::Face => Vec::new(),
        // The heel is a roll pivot, not a joint.
        Kind::Leg => (0..4.min(n.saturating_sub(1)))
            .map(|i| (i, i + 1))
            .collect(),
        Kind::QuadLeg => (0..5.min(n.saturating_sub(1)))
            .map(|i| (i, i + 1))
            .collect(),
        _ => (0..n.saturating_sub(1)).map(|i| (i, i + 1)).collect(),
    }
}

fn too_close(g: &Guides, found: &mut Vec<Finding>) {
    let h = g.body.height;
    let taken = flagged(found);
    for (ci, c) in g.components.iter().enumerate() {
        for (a, b) in bone_pairs(c) {
            if taken.contains(&(ci, b)) {
                continue;
            }
            let at = |i: usize| c.guides[i].position;
            let (pa, pb) = (at(a), at(b));
            let d = pa.distance(pb);
            if d >= TOO_CLOSE * h {
                continue;
            }
            // Push the later joint on along the bone, or (when the two are on
            // top of each other, which can't build) along the bone before it.
            let dir = (pb - pa)
                .try_normalize()
                .or_else(|| a.checked_sub(1).and_then(|p| (pa - at(p)).try_normalize()))
                .or_else(|| {
                    c.guides
                        .get(b + 1)
                        .and_then(|n| (n.position - pb).try_normalize())
                })
                .unwrap_or(Vec3::Y);
            let other = &c.guides[a].name;
            found.push(Finding {
                check: Check::TooClose,
                comp: ci,
                guide: b,
                message: if d <= 1e-4 * h {
                    format!("on top of {other}, so the bone between them has no length")
                } else {
                    format!("only {} from {other}", cm(d))
                },
                suggestion: "a bone this short twists unpredictably; move the joints apart".into(),
                nudge: Some(pa + dir * (2.0 * TOO_CLOSE * h)),
            });
        }
    }
}

/// How a middle joint `b` bends off the line from `a` to `c`, relative to the
/// way it should (`expected`, squared to that line as the builder squares the
/// IK pole): (bend depth, depth along the expected way), both over the line's
/// length, and the position with the same depth bent the expected way.
fn bend(a: Vec3, b: Vec3, c: Vec3, expected: Vec3) -> Option<(f32, f32, Vec3)> {
    let len = a.distance(c);
    let axis = (c - a).try_normalize()?;
    let foot = a + axis * (b - a).dot(axis);
    let off = b - foot;
    let e = (expected - axis * expected.dot(axis)).try_normalize()?;
    let depth = off.length() / len;
    let along = off.dot(e) / len;
    Some((depth, along, foot + e * (len * depth.max(FIX_BEND))))
}

fn bends(g: &Guides, found: &mut Vec<Finding>) {
    for (ci, c) in g.components.iter().enumerate() {
        // (outer, middle, inner, the way the middle should point, its name for people)
        let joints: &[(&str, &str, &str, Vec3, &str)] = match c.kind {
            Kind::Arm => &[("shoulder", "elbow", "wrist", Vec3::NEG_Z, "back")],
            Kind::Leg => &[("hip", "knee", "ankle", Vec3::Z, "forward")],
            Kind::QuadLeg => &[
                ("hip", "knee", "hock", Vec3::Z, "forward"),
                ("knee", "hock", "ankle", Vec3::NEG_Z, "back"),
            ],
            _ => continue,
        };
        for &(a, m, z, expected, way) in joints {
            let (Some(pa), Some(pm), Some(pz)) = (c.position(a), c.position(m), c.position(z))
            else {
                continue;
            };
            let index = |n: &str| c.guides.iter().position(|x| x.name == n);
            let Some(gi) = index(m) else { continue };
            // A joint already flagged (or a neighbour of one) is fixed first:
            // its bend means little until then.
            let taken = flagged(found);
            if [a, m, z]
                .into_iter()
                .any(|n| index(n).is_some_and(|i| taken.contains(&(ci, i))))
            {
                continue;
            }
            let Some((depth, along, fixed)) = bend(pa, pm, pz, expected) else {
                continue;
            };
            let finding = if depth < STRAIGHT_BEND {
                Some((
                    Check::Straight,
                    format!("the limb is straight at the {m}"),
                    format!(
                        "with no bend IK can't tell which way to fold; bend the {m} slightly {way}"
                    ),
                ))
            } else if along < 0.0 {
                let wrong = if way == "back" { "forward" } else { "backward" };
                Some((
                    Check::BendsWrongWay,
                    format!("the {m} bends {wrong}"),
                    format!(
                        "IK folds the {m} {way}, so the limb would flip when posed; move the {m} {way} of the line from the {a} to the {z}"
                    ),
                ))
            } else {
                None
            };
            if let Some((check, message, suggestion)) = finding {
                found.push(Finding {
                    check,
                    comp: ci,
                    guide: gi,
                    message,
                    suggestion,
                    nudge: Some(fixed),
                });
            }
        }
    }
}

fn outside(g: &Guides, surface: &mut Surface, found: &mut Vec<Finding>) {
    let h = g.body.height;
    let taken = flagged(found);
    for (ci, c) in g.components.iter().enumerate() {
        // Joints that belong inside the body; tips, pivots and chains may not.
        let names: &[&str] = match c.kind {
            Kind::Spine => &["pelvis", "chest"],
            Kind::Head => &["neck", "head"],
            Kind::Arm => &["clavicle", "shoulder", "elbow", "wrist"],
            Kind::Leg => &["hip", "knee", "ankle"],
            Kind::QuadLeg => &["hip", "knee", "hock", "ankle"],
            Kind::Jaw => &["jaw"],
            _ => continue,
        };
        for (gi, guide) in c.guides.iter().enumerate() {
            if !names.contains(&guide.name.as_str()) || taken.contains(&(ci, gi)) {
                continue;
            }
            if let Some(d) = surface.outside_by(guide.position)
                && d > OUTSIDE * h
            {
                found.push(Finding {
                    check: Check::OutsideMesh,
                    comp: ci,
                    guide: gi,
                    message: format!("{} outside the mesh", cm(d)),
                    suggestion:
                        "joints sit inside the body so the skin follows them; drag it back in"
                            .into(),
                    nudge: None,
                });
            }
        }
    }
}

/// Left/right twins that aren't mirror images. Guides already flagged by
/// another check are left out (fix those first).
fn asymmetry(g: &Guides, flagged: &HashSet<(usize, usize)>, out: &mut Vec<Issue>) {
    let h = g.body.height;
    let cx = g.body.center_x;
    let mirrored = |p: Vec3| Vec3::new(2.0 * cx - p.x, p.y, p.z);
    for (ri, r) in g.components.iter().enumerate() {
        // (right guide index, left component, left guide index)
        let pairs: Vec<(usize, usize, usize)> = match r.kind {
            Kind::Eyes | Kind::Face => r
                .guides
                .iter()
                .enumerate()
                .filter(|(_, x)| x.name.ends_with("_r"))
                .filter_map(|(gi, _)| mirror_guide(g, ri, gi).map(|(lc, lg)| (gi, lc, lg)))
                .collect(),
            _ if r.side == Side::Right => {
                let Some(twin) = twin_id(r) else { continue };
                let Some(li) = g.components.iter().position(|c| c.id == twin) else {
                    continue;
                };
                if g.components[li].guides.len() != r.guides.len() {
                    continue;
                }
                (0..r.guides.len()).map(|gi| (gi, li, gi)).collect()
            }
            _ => continue,
        };
        let off: Vec<(usize, usize, usize, f32)> = pairs
            .into_iter()
            .filter(|&(gi, lc, lg)| !flagged.contains(&(ri, gi)) && !flagged.contains(&(lc, lg)))
            .map(|(gi, lc, lg)| {
                let d =
                    mirrored(g.components[lc].guides[lg].position).distance(r.guides[gi].position);
                (gi, lc, lg, d)
            })
            .filter(|t| t.3 > ASYMMETRY * h)
            .collect();
        let Some(&(worst, lc, lg, d)) = off.iter().max_by(|a, b| a.3.total_cmp(&b.3)) else {
            continue;
        };
        let left = &g.components[lc];
        let others = off.len() - 1;
        out.push(Issue {
            severity: Severity::Warning,
            check: Check::Asymmetric,
            component: r.id.clone(),
            guide: Some(r.guides[worst].name.clone()),
            message: format!(
                "doesn't mirror {}{}: {} off{}",
                if left.id == r.id {
                    "its left side"
                } else {
                    &left.id
                },
                if left.id == r.id {
                    format!(" ({})", left.guides[lg].name)
                } else {
                    String::new()
                },
                cm(d),
                if others > 0 {
                    format!(", and {others} more guide(s)")
                } else {
                    String::new()
                }
            ),
            suggestion: "if the character is symmetric, mirror the left side onto the right".into(),
            fix: Some(Fix::Mirror {
                component: r.id.clone(),
                from: left.id.clone(),
                pairs: off
                    .iter()
                    .map(|&(gi, lc, lg, _)| {
                        (
                            r.guides[gi].name.clone(),
                            g.components[lc].guides[lg].name.clone(),
                        )
                    })
                    .collect(),
            }),
        });
    }
}

/// Components hanging from bones that make an odd hierarchy: a leg under the
/// head, an arm under a leg, a left part under a right one.
fn hierarchy(g: &Guides, built: &Built, out: &mut Vec<Issue>) {
    let owner: HashMap<&str, &Component> = built
        .skeleton
        .bones
        .iter()
        .filter_map(|b| Some((b.name.as_str(), g.component(&b.component)?)))
        .collect();
    let top_spine = g
        .components
        .iter()
        .find(|c| c.kind == Kind::Spine)
        .map(|s| spine_name(g.spine_joints(s) - 1));
    for c in &g.components {
        let Some(parent) = c.parent.as_deref() else {
            continue;
        };
        let Some(o) = owner.get(parent) else { continue };
        // (kinds it usually hangs from, the bone it usually hangs from, what that is)
        let (expected, usual, word): (&[Kind], Option<String>, &str) = match c.kind {
            Kind::Spine => (&[Kind::Root], Some("root".into()), "the root"),
            Kind::Head | Kind::Arm => (&[Kind::Spine], top_spine.clone(), "the chest"),
            Kind::Leg | Kind::QuadLeg => (&[Kind::Spine], Some("pelvis".into()), "the pelvis"),
            Kind::Eyes | Kind::Jaw | Kind::Face => {
                (&[Kind::Head, Kind::Jaw], Some("head".into()), "the head")
            }
            Kind::Chain | Kind::Spline | Kind::Root => (&[], None, ""),
        };
        let wrong_kind = !expected.is_empty() && !expected.contains(&o.kind);
        let crossed = c.side != Side::Center && o.side != Side::Center && o.side != c.side;
        if !wrong_kind && !crossed {
            continue;
        }
        let bone = if wrong_kind {
            usual
        } else {
            // The same bone on this component's own side.
            let (from, to) = if c.side == Side::Left {
                ("_r", "_l")
            } else {
                ("_l", "_r")
            };
            parent.strip_suffix(from).map(|b| format!("{b}{to}"))
        }
        .filter(|b| owner.get(b.as_str()).is_some_and(|x| x.id != c.id));
        let message = format!("hangs from {parent}, a bone of the {} component", o.id);
        let suggestion = if wrong_kind {
            format!(
                "{} usually hang from {word}; re-attach it so the hierarchy follows the body",
                match c.kind {
                    Kind::Head => "heads",
                    Kind::Arm => "arms and front legs",
                    Kind::Leg | Kind::QuadLeg => "legs",
                    Kind::Spine => "spines",
                    _ => "eyes, jaws and faces",
                }
            )
        } else {
            format!(
                "a {} part hanging from the {} side moves with the wrong limb; hang it from its own side",
                side_word(c.side),
                side_word(o.side)
            )
        };
        out.push(Issue {
            severity: Severity::Warning,
            check: Check::OddParent,
            component: c.id.clone(),
            guide: None,
            message,
            suggestion,
            fix: bone.map(|bone| Fix::Attach {
                component: c.id.clone(),
                bone,
            }),
        });
    }
}

// --------------------------------------------------------------------------- fixing

/// Apply one fix. Returns a line saying what changed.
pub fn apply_fix(g: &mut Guides, fix: &Fix) -> Result<String> {
    match fix {
        Fix::Mirror {
            component,
            from,
            pairs,
        } => {
            let src = g
                .component(from)
                .with_context(|| format!("no component named {from}"))?;
            let moves: Vec<(String, Vec3)> = pairs
                .iter()
                .map(|(to, from_guide)| {
                    let p = src
                        .position(from_guide)
                        .with_context(|| format!("{from} has no guide {from_guide}"))?;
                    Ok((to.clone(), components::mirror_point(g, p)))
                })
                .collect::<Result<_>>()?;
            let dst = g
                .components
                .iter_mut()
                .find(|c| &c.id == component)
                .with_context(|| format!("no component named {component}"))?;
            for (name, p) in &moves {
                let guide = dst
                    .guides
                    .iter_mut()
                    .find(|x| &x.name == name)
                    .with_context(|| format!("{component} has no guide {name}"))?;
                guide.position = *p;
            }
            let names: Vec<&str> = moves.iter().map(|(n, _)| n.as_str()).collect();
            Ok(if component == from {
                format!("Mirrored {component}'s left side onto its right.")
            } else {
                format!("Mirrored {from} onto {component} ({}).", names.join(", "))
            })
        }
        Fix::Move {
            component,
            guide,
            to,
        } => {
            let c = g
                .components
                .iter_mut()
                .find(|c| &c.id == component)
                .with_context(|| format!("no component named {component}"))?;
            let x = c
                .guides
                .iter_mut()
                .find(|x| &x.name == guide)
                .with_context(|| format!("{component} has no guide {guide}"))?;
            x.position = *to;
            Ok(format!("Moved {component}'s {guide}."))
        }
        Fix::Attach { component, bone } => {
            components::attach(g, component, bone)?;
            Ok(format!("{component} now hangs from {bone}."))
        }
    }
}

/// Apply every offered fix, in order. Returns how many were applied.
pub fn fix_all(g: &mut Guides, issues: &[Issue]) -> Result<usize> {
    let mut n = 0;
    for fix in issues.iter().filter_map(|i| i.fix.as_ref()) {
        apply_fix(g, fix)?;
        n += 1;
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{AddOptions, Preset, add_preset, biped, quadruped};
    use crate::demo::{self, demo_humanoid};
    use crate::landmarks::detect;

    fn human() -> (Guides, Surface) {
        let mesh = demo_humanoid();
        (biped(&detect(&mesh)), Surface::new(&mesh))
    }

    fn dog() -> (Guides, Surface) {
        (
            quadruped(demo::DOG_HEIGHT),
            Surface::new(&demo::demo_quadruped()),
        )
    }

    fn comp<'a>(g: &'a mut Guides, id: &str) -> &'a mut Component {
        g.components.iter_mut().find(|c| c.id == id).unwrap()
    }

    fn set(g: &mut Guides, id: &str, guide: &str, p: Vec3) {
        let c = comp(g, id);
        c.guides
            .iter_mut()
            .find(|x| x.name == guide)
            .unwrap()
            .position = p;
    }

    fn at(g: &Guides, id: &str, guide: &str) -> Vec3 {
        g.component(id).unwrap().position(guide).unwrap()
    }

    /// Exactly one issue, of `check`, on `id` / `guide`; its fix leaves none.
    fn one_and_fixed(mut g: Guides, s: &mut Surface, check: Check, id: &str, guide: Option<&str>) {
        let issues = super::check(&g, Some(s));
        assert_eq!(issues.len(), 1, "{issues:#?}");
        let i = &issues[0];
        assert_eq!(
            (i.check, i.component.as_str(), i.guide.as_deref()),
            (check, id, guide),
            "{i:#?}"
        );
        assert_eq!(i.severity, Severity::Warning);
        assert!(i.fix.is_some() && i.fix_label().is_some(), "{i:#?}");
        apply_fix(&mut g, i.fix.as_ref().unwrap()).unwrap();
        let after = super::check(&g, Some(s));
        assert!(after.is_empty(), "{after:#?}");
    }

    #[test]
    fn clean_layouts_have_no_warnings() {
        let (mut g, mut s) = human();
        assert_eq!(check(&g, Some(&mut s)), Vec::new());
        for p in [
            Preset::Fingers,
            Preset::Toes,
            Preset::Tail,
            Preset::Eyes,
            Preset::Jaw,
            Preset::Face,
        ] {
            add_preset(&mut g, p, &AddOptions::default()).unwrap();
        }
        assert_eq!(check(&g, Some(&mut s)), Vec::new());
        let (g, mut s) = dog();
        assert_eq!(check(&g, Some(&mut s)), Vec::new());
        // The A-pose too.
        let mesh = demo::humanoid(45.0);
        let g = biped(&detect(&mesh));
        assert_eq!(check(&g, Some(&mut Surface::new(&mesh))), Vec::new());
    }

    #[test]
    fn wrong_side_is_mirrored_from_the_twin_or_flipped() {
        let (mut g, mut s) = human();
        let p = at(&g, "arm_l", "clavicle");
        let cx = g.body.center_x;
        set(
            &mut g,
            "arm_l",
            "clavicle",
            Vec3::new(2.0 * cx - p.x - 0.02, p.y, p.z),
        );
        one_and_fixed(
            g.clone(),
            &mut s,
            Check::WrongSide,
            "arm_l",
            Some("clavicle"),
        );
        // Both clavicles across: each is flipped back on its own.
        let q = at(&g, "arm_r", "clavicle");
        set(
            &mut g,
            "arm_r",
            "clavicle",
            Vec3::new(2.0 * cx - q.x + 0.02, q.y, q.z),
        );
        let issues = check(&g, Some(&mut s));
        assert_eq!(issues.len(), 2, "{issues:#?}");
        assert!(
            issues
                .iter()
                .all(|i| i.check == Check::WrongSide && matches!(i.fix, Some(Fix::Move { .. })))
        );
        fix_all(&mut g, &issues).unwrap();
        assert_eq!(check(&g, Some(&mut s)), Vec::new());
        // An eye on the other side of the face.
        let (mut g, mut s) = human();
        add_preset(&mut g, Preset::Eyes, &AddOptions::default()).unwrap();
        let e = at(&g, "eyes", "eye_l");
        set(
            &mut g,
            "eyes",
            "eye_l",
            Vec3::new(2.0 * cx - e.x - 0.01, e.y, e.z),
        );
        let issues = check(&g, Some(&mut s));
        assert!(
            issues
                .iter()
                .any(|i| i.check == Check::WrongSide && i.guide.as_deref() == Some("eye_l")),
            "{issues:#?}"
        );
    }

    #[test]
    fn elbows_bending_forward_and_knees_backward() {
        let (g, mut s) = human();
        for (id, joint, lean) in [("arm_l", "elbow", 0.05), ("leg_r", "knee", -0.05)] {
            let mut b = g.clone();
            let p = at(&b, id, joint);
            set(&mut b, id, joint, p + Vec3::Z * lean);
            one_and_fixed(b, &mut s, Check::BendsWrongWay, id, Some(joint));
        }
        // Both elbows forward: no healthy twin, so each is bent back on its own,
        // and the two fixes keep the arms mirror images.
        let mut b = g.clone();
        for id in ["arm_l", "arm_r"] {
            let p = at(&b, id, "elbow");
            set(&mut b, id, "elbow", p + Vec3::Z * 0.05);
        }
        let issues = check(&b, Some(&mut s));
        assert_eq!(issues.len(), 2, "{issues:#?}");
        assert!(issues.iter().all(
            |i| i.check == Check::BendsWrongWay && i.fix_label().as_deref() == Some("Bend it")
        ));
        fix_all(&mut b, &issues).unwrap();
        assert_eq!(check(&b, Some(&mut s)), Vec::new());
        assert!(at(&b, "arm_l", "elbow").z < at(&b, "arm_l", "shoulder").z);
        // The dog: a hind knee bent back and a hock bent forward.
        let (d, mut s) = dog();
        for (joint, lean) in [("knee", -0.3), ("hock", 0.12)] {
            let mut b = d.clone();
            let p = at(&b, "hind_leg_l", joint);
            set(&mut b, "hind_leg_l", joint, p + Vec3::Z * lean);
            one_and_fixed(b, &mut s, Check::BendsWrongWay, "hind_leg_l", Some(joint));
        }
        // A front leg's elbow bent forward.
        let mut b = d.clone();
        let p = at(&b, "front_leg_r", "elbow");
        set(&mut b, "front_leg_r", "elbow", p + Vec3::Z * 0.15);
        one_and_fixed(
            b,
            &mut s,
            Check::BendsWrongWay,
            "front_leg_r",
            Some("elbow"),
        );
    }

    #[test]
    fn a_straight_limb_gets_a_bend() {
        let (g, mut s) = human();
        let mut b = g.clone();
        let (hip, ankle) = (at(&b, "leg_l", "hip"), at(&b, "leg_l", "ankle"));
        set(&mut b, "leg_l", "knee", hip.lerp(ankle, 0.5));
        one_and_fixed(b, &mut s, Check::Straight, "leg_l", Some("knee"));
        // Both legs straight: bent forward without a twin to copy.
        let mut b = g.clone();
        for id in ["leg_l", "leg_r"] {
            let (hip, ankle) = (at(&b, id, "hip"), at(&b, id, "ankle"));
            set(&mut b, id, "knee", hip.lerp(ankle, 0.5));
        }
        let issues = check(&b, Some(&mut s));
        assert_eq!(issues.len(), 2, "{issues:#?}");
        fix_all(&mut b, &issues).unwrap();
        assert_eq!(check(&b, Some(&mut s)), Vec::new());
        let leg = b.component("leg_l").unwrap();
        let (hip, knee, ankle) = (
            leg.position("hip").unwrap(),
            leg.position("knee").unwrap(),
            leg.position("ankle").unwrap(),
        );
        assert!(knee.z > hip.lerp(ankle, 0.5).z, "the knee bends forward");
    }

    #[test]
    fn guides_outside_the_mesh() {
        let (g, mut s) = human();
        let mut b = g.clone();
        let p = at(&b, "arm_l", "wrist");
        set(&mut b, "arm_l", "wrist", p + Vec3::Y * 0.2);
        one_and_fixed(b, &mut s, Check::OutsideMesh, "arm_l", Some("wrist"));
        // Without the mesh there is nothing to check against.
        let mut b = g.clone();
        set(&mut b, "arm_l", "wrist", p + Vec3::Y * 0.2);
        let issues = check(&b, None);
        assert!(issues.iter().all(|i| i.check != Check::OutsideMesh));
        // The spine out behind the back has no twin, so no fix.
        let mut b = g.clone();
        let p = at(&b, "spine", "pelvis");
        set(&mut b, "spine", "pelvis", p - Vec3::Z * 0.5);
        let issues = check(&b, Some(&mut s));
        assert_eq!(issues.len(), 1, "{issues:#?}");
        assert_eq!(issues[0].check, Check::OutsideMesh);
        assert!(issues[0].fix.is_none());
    }

    #[test]
    fn joints_too_close() {
        let (g, mut s) = human();
        let mut b = g.clone();
        let p = at(&b, "arm_r", "elbow");
        set(&mut b, "arm_r", "wrist", p + Vec3::new(-0.003, 0.0, 0.0));
        let issues = check(&b, Some(&mut s));
        assert_eq!(issues.len(), 1, "{issues:#?}");
        assert_eq!(issues[0].check, Check::TooClose);
        one_and_fixed(b, &mut s, Check::TooClose, "arm_r", Some("wrist"));
        // A finger joint pushed apart when its twin is close too.
        let mut b = g.clone();
        add_preset(&mut b, Preset::Tail, &AddOptions::default()).unwrap();
        let p = at(&b, "tail", "j2");
        set(&mut b, "tail", "j3", p + Vec3::new(0.0, 0.001, -0.001));
        let issues = check(&b, Some(&mut s));
        assert_eq!(issues.len(), 1, "{issues:#?}");
        assert_eq!(issues[0].fix_label().as_deref(), Some("Push apart"));
        fix_all(&mut b, &issues).unwrap();
        assert_eq!(check(&b, Some(&mut s)), Vec::new());
    }

    #[test]
    fn asymmetric_twins_and_off_centre_guides() {
        let (g, mut s) = human();
        // The right hand a little higher than the left: mirror the left over.
        let mut b = g.clone();
        let p = at(&b, "arm_r", "hand_tip");
        set(&mut b, "arm_r", "hand_tip", p + Vec3::Y * 0.06);
        one_and_fixed(b, &mut s, Check::Asymmetric, "arm_r", Some("hand_tip"));
        // The left eye higher than the right one.
        let mut b = g.clone();
        add_preset(&mut b, Preset::Eyes, &AddOptions::default()).unwrap();
        let p = at(&b, "eyes", "eye_l");
        set(&mut b, "eyes", "eye_l", p + Vec3::Y * 0.05);
        one_and_fixed(b, &mut s, Check::Asymmetric, "eyes", Some("eye_r"));
        // Within the tolerance is fine.
        let mut b = g.clone();
        let p = at(&b, "arm_r", "hand_tip");
        set(&mut b, "arm_r", "hand_tip", p + Vec3::Y * 0.01);
        assert_eq!(check(&b, Some(&mut s)), Vec::new());
        // The head off to one side.
        let mut b = g.clone();
        let p = at(&b, "head", "head_top");
        set(&mut b, "head", "head_top", p + Vec3::X * 0.06);
        one_and_fixed(b, &mut s, Check::OffCentre, "head", Some("head_top"));
    }

    #[test]
    fn odd_parents_are_reattached() {
        let (g, mut s) = human();
        // A leg hanging from the head.
        let mut b = g.clone();
        components::attach(&mut b, "leg_l", "head").unwrap();
        one_and_fixed(b, &mut s, Check::OddParent, "leg_l", None);
        // An arm hanging from the pelvis is fine (a spine bone); from a thigh it isn't.
        let mut b = g.clone();
        components::attach(&mut b, "arm_l", "thigh_l").unwrap();
        let issues = check(&b, Some(&mut s));
        assert_eq!(issues.len(), 1, "{issues:#?}");
        assert!(matches!(&issues[0].fix, Some(Fix::Attach { bone, .. }) if bone == "spine_03"));
        // Left fingers on the right hand.
        let mut b = g.clone();
        add_preset(&mut b, Preset::Fingers, &AddOptions::default()).unwrap();
        components::attach(&mut b, "index_l", "hand_r").unwrap();
        one_and_fixed(b, &mut s, Check::OddParent, "index_l", None);
        // The eyes on the chest.
        let mut b = g.clone();
        add_preset(&mut b, Preset::Eyes, &AddOptions::default()).unwrap();
        components::attach(&mut b, "eyes", "spine_02").unwrap();
        one_and_fixed(b, &mut s, Check::OddParent, "eyes", None);
        // A tail can hang anywhere.
        let mut b = g.clone();
        add_preset(&mut b, Preset::Tail, &AddOptions::default()).unwrap();
        components::attach(&mut b, "tail", "head").unwrap();
        assert_eq!(check(&b, Some(&mut s)), Vec::new());
    }

    #[test]
    fn an_unbuildable_layout_is_the_one_error() {
        let (mut g, mut s) = human();
        let p = at(&g, "arm_l", "elbow");
        set(&mut g, "arm_l", "wrist", p);
        let issues = check(&g, Some(&mut s));
        assert_eq!(errors(&issues), 1, "{issues:#?}");
        assert_eq!(issues[0].check, Check::Unbuildable);
        assert!(issues[0].message.contains("no length"), "{issues:#?}");
        // The guides on top of each other are named, with a fix that builds.
        assert_eq!(warnings(&issues), 1, "{issues:#?}");
        let w = &issues[1];
        assert_eq!(
            (w.check, w.component.as_str(), w.guide.as_deref()),
            (Check::TooClose, "arm_l", Some("wrist"))
        );
        fix_all(&mut g, &issues).unwrap();
        assert_eq!(check(&g, Some(&mut s)), Vec::new());
        // With no twin to copy, the wrist is pushed on along the forearm's line.
        let (mut g, mut s) = human();
        for id in ["arm_l", "arm_r"] {
            let p = at(&g, id, "elbow");
            set(&mut g, id, "wrist", p);
        }
        let issues = check(&g, Some(&mut s));
        assert_eq!((errors(&issues), warnings(&issues)), (1, 2), "{issues:#?}");
        fix_all(&mut g, &issues).unwrap();
        assert!(components::build(&g).is_ok());
    }

    #[test]
    fn surface_inside_and_outside() {
        let mesh = demo_humanoid();
        let mut s = Surface::new(&mesh);
        let g = biped(&detect(&mesh));
        assert_eq!(s.outside_by(at(&g, "spine", "pelvis")), None);
        let far = s
            .outside_by(at(&g, "spine", "pelvis") + Vec3::Z * 1.0)
            .unwrap();
        assert!(far > 0.8 && far < 1.0, "{far}");
        // Cached: the same answer again.
        assert_eq!(s.outside_by(at(&g, "spine", "pelvis")), None);
    }
}
