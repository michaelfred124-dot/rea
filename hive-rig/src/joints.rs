//! Joints to guides: read an existing skeleton back into editable guides.
//!
//! A character that already has joints (from Maya, Blender, Mixamo, an Unreal
//! mannequin, a 3ds Max biped, or unnamed) keeps its joint placement. The
//! skeleton's shape says which chain is which limb ([`crate::structure`]); each
//! guide moves onto the joint recognised by name when that joint lies in the
//! right limb, and otherwise onto the joint its place in the limb gives it
//! (hip, knee, ankle by where the chain bends and how long its parts are).
//! Eyes, a jaw, face joints and fingers are added by name when the skeleton has
//! them, and every guide neither finds keeps the position detected on the mesh.

use std::collections::HashMap;

use anyhow::{Context, Result};
use glam::Vec3;

use crate::components::{self, AddOptions, FACE_GUIDES, Guide, Guides, Kind, Preset};
use crate::orient::BodyKind;
use crate::skeleton::Side;
use crate::structure::{self, BodyMap, Part};

/// A joint of an imported skeleton, in the mesh's model space.
#[derive(Debug, Clone)]
pub struct Joint {
    pub name: String,
    pub position: Vec3,
    /// Index of its parent joint in the same list (the nearest ancestor that
    /// is a joint), if any.
    pub parent: Option<usize>,
}

/// What [`guides_from_joints`] did.
#[derive(Debug, Clone, Default)]
pub struct JointReport {
    /// Guides placed on a joint, as `component.guide`.
    pub placed: Vec<String>,
    /// Components added because the skeleton has them (eyes, jaw, face, fingers).
    pub added: Vec<String>,
    /// Of `placed`, the guides placed by the skeleton's shape rather than a name.
    pub by_shape: Vec<String>,
    /// Joints no guide used, such as twist, IK or helper joints.
    pub unused: Vec<String>,
}

/// Joint names each biped guide can come from, best first. Names are compared
/// after [`candidates`] has removed namespaces, side tokens and joint suffixes.
const BIPED: &[(&str, &str, &[&str])] = &[
    ("root", "root", &["root"]),
    ("spine", "pelvis", &["pelvis", "hips", "hip"]),
    ("spine", "chest", &["neck_01", "neck", "neck1"]),
    ("head", "neck", &["neck_01", "neck", "neck1"]),
    ("head", "head", &["head"]),
    (
        "head",
        "head_top",
        &["head_top", "headtop_end", "head_end", "headtop", "head_tip"],
    ),
    // The upper arm before the clavicle: Mixamo calls its clavicle "Shoulder".
    (
        "arm",
        "shoulder",
        &["upperarm", "upper_arm", "arm", "shoulder"],
    ),
    (
        "arm",
        "clavicle",
        &["clavicle", "collar", "collarbone", "shoulder"],
    ),
    (
        "arm",
        "elbow",
        &["lowerarm", "lower_arm", "forearm", "elbow"],
    ),
    ("arm", "wrist", &["hand", "wrist"]),
    (
        "arm",
        "hand_tip",
        &[
            "hand_end",
            "hand_tip",
            "handtip",
            "middle_03",
            "handmiddle3",
        ],
    ),
    (
        "leg",
        "hip",
        &["thigh", "upleg", "upperleg", "upper_leg", "hip"],
    ),
    (
        "leg",
        "knee",
        &["calf", "lowerleg", "lower_leg", "leg", "shin", "knee"],
    ),
    ("leg", "ankle", &["foot", "ankle"]),
    ("leg", "ball", &["ball", "toebase", "toe_base", "toes"]),
    ("leg", "toe", &["toe_end", "ball_end", "toebase_end", "toe"]),
    ("leg", "heel", &["heel"]),
];

/// Other names face joints go by (face guides also match their own names).
const FACE_ALIASES: &[(&str, &[&str])] = &[
    ("brow_in", &["brow_inner", "inner_brow", "brow_01"]),
    ("brow_mid", &["brow_middle", "brow_02"]),
    ("brow_out", &["brow_outer", "outer_brow", "brow_03"]),
    (
        "lid_up",
        &[
            "lid_upper",
            "upper_lid",
            "eyelid_up",
            "eyelid_upper",
            "upperlid",
        ],
    ),
    (
        "lid_low",
        &[
            "lid_lower",
            "lower_lid",
            "eyelid_low",
            "eyelid_lower",
            "lowerlid",
        ],
    ),
    ("lip_up", &["lip_upper", "upper_lip", "lip_top"]),
    ("lip_low", &["lip_lower", "lower_lip", "lip_bottom"]),
    ("lip_corner", &["mouth_corner", "corner_lip", "lip_corner"]),
];

const FINGERS: [&str; 5] = ["thumb", "index", "middle", "ring", "pinky"];

/// Suffixes and prefixes riggers add to joint names that say nothing about which
/// joint it is.
const NOISE: [&str; 9] = [
    "jnt", "joint", "jj", "bind", "bnd", "def", "drv", "skin", "jt",
];

/// The side of a joint name and the names it may be known by, most specific
/// (fewest leading tokens dropped) first: `Bip01 L UpperArm` gives Left and
/// `bip01_upperarm`, `upperarm`.
pub fn candidates(name: &str) -> (Side, Vec<String>) {
    let short = name.rsplit([':', '|']).next().unwrap_or(name);
    let mut lower = short.to_ascii_lowercase().replace(['.', '-', ' '], "_");
    if let Some(rest) = lower.strip_prefix("mixamorig") {
        lower = rest.trim_start_matches('_').to_string();
    }
    let mut side = Side::Center;
    let mut tokens: Vec<String> = Vec::new();
    for t in lower.split('_').filter(|t| !t.is_empty()) {
        match t {
            "l" | "left" | "lf" | "lft" => side = Side::Left,
            "r" | "right" | "rt" | "rgt" => side = Side::Right,
            t if NOISE.contains(&t) => {}
            t => {
                let (s, rest) = if let Some(rest) = t.strip_prefix("left") {
                    (Side::Left, rest)
                } else if let Some(rest) = t.strip_prefix("right") {
                    (Side::Right, rest)
                } else {
                    (Side::Center, t)
                };
                if s != Side::Center && !rest.is_empty() {
                    side = s;
                    tokens.push(rest.to_string());
                } else {
                    tokens.push(t.to_string());
                }
            }
        }
    }
    let names = (0..tokens.len()).map(|i| tokens[i..].join("_")).collect();
    (side, names)
}

/// Joints by recognised name and side, keeping the most specific match.
struct Lookup<'a> {
    joints: &'a [Joint],
    best: HashMap<(String, Side), (usize, usize)>,
    used: Vec<bool>,
}

impl<'a> Lookup<'a> {
    fn new(joints: &'a [Joint]) -> Self {
        let mut best: HashMap<(String, Side), (usize, usize)> = HashMap::new();
        for (j, joint) in joints.iter().enumerate() {
            let (side, names) = candidates(&joint.name);
            for (rank, n) in names.into_iter().enumerate() {
                let slot = best.entry((n, side)).or_insert((rank, j));
                if rank < slot.0 {
                    *slot = (rank, j);
                }
            }
        }
        Lookup {
            joints,
            best,
            used: vec![false; joints.len()],
        }
    }

    /// The first unused joint with one of `names` on `side`.
    fn take(&mut self, names: &[&str], side: Side) -> Option<Vec3> {
        self.take_index(names, side, |_| true)
            .map(|j| self.joints[j].position)
    }

    /// The first unused joint with one of `names` on `side` that `fits`.
    fn take_index(
        &mut self,
        names: &[&str],
        side: Side,
        fits: impl Fn(usize) -> bool,
    ) -> Option<usize> {
        for n in names {
            if let Some(&(_, j)) = self.best.get(&(n.to_string(), side))
                && !self.used[j]
                && fits(j)
            {
                self.used[j] = true;
                return Some(j);
            }
        }
        None
    }

    fn has(&self, names: &[&str], side: Side) -> bool {
        names
            .iter()
            .any(|n| self.best.contains_key(&(n.to_string(), side)))
    }
}

/// Move `detected` guides onto the matching joints of an existing skeleton, and
/// add the eyes, jaw, face and fingers it has. Fails only when the result
/// doesn't build.
pub fn guides_from_joints(detected: Guides, joints: &[Joint]) -> Result<(Guides, JointReport)> {
    let mut g = detected;
    let mut report = JointReport::default();
    let mut look = Lookup::new(joints);
    let body = structure::map(joints, &g.body, BodyKind::Biped);
    let shaped = body
        .as_ref()
        .map(|m| biped_roles(m, joints))
        .unwrap_or_default();

    // Spine joints between the pelvis and the neck.
    let spine: Vec<usize> = joints
        .iter()
        .enumerate()
        .filter(|(_, j)| {
            let (side, names) = candidates(&j.name);
            side == Side::Center
                && names.iter().any(|n| {
                    let digits = n.trim_start_matches("spine").trim_start_matches('_');
                    (n.starts_with("spine") && digits.chars().all(|c| c.is_ascii_digit()))
                        || matches!(n.as_str(), "chest" | "upperchest" | "upper_chest")
                })
        })
        .map(|(i, _)| i)
        .collect();
    if (2..=16).contains(&spine.len()) {
        components::set_spine_joints(&mut g, "spine", spine.len())?;
        for i in spine {
            look.used[i] = true;
        }
    } else if let Some(m) = &body
        && m.chest.is_some()
        && (1..=16).contains(&m.spine.len())
    {
        components::set_spine_joints(&mut g, "spine", m.spine.len())?;
        for &i in &m.spine {
            look.used[i] = true;
        }
    }

    for c in &mut g.components {
        let kind_name = match c.kind {
            Kind::Root => "root",
            Kind::Spine => "spine",
            Kind::Head => "head",
            Kind::Arm => "arm",
            Kind::Leg => "leg",
            _ => continue,
        };
        let side_index = usize::from(c.side == Side::Right);
        let part = match c.kind {
            Kind::Spine => Some(Part::Spine),
            Kind::Head => Some(Part::Head),
            Kind::Arm => Some(Part::Arm(side_index)),
            Kind::Leg => Some(Part::Leg(side_index)),
            _ => None,
        };
        // A name only counts when its joint lies in the limb the skeleton's
        // shape gives this component (when the shape found that limb).
        let members: Option<Vec<usize>> = body
            .as_ref()
            .zip(part)
            .map(|(m, p)| m.part(p))
            .filter(|v| v.len() > 1);
        for &(kind, guide, names) in BIPED {
            if kind != kind_name {
                continue;
            }
            let fits = |j: usize| members.as_ref().is_none_or(|v| v.contains(&j));
            // The chest guide marks the top of the spine, where the neck starts,
            // without using up the neck joint.
            let by_name = if guide == "chest" {
                let head = body.as_ref().map(|m| m.part(Part::Head));
                names.iter().find_map(|n| {
                    look.best
                        .get(&(n.to_string(), Side::Center))
                        .map(|&(_, j)| j)
                        .filter(|j| head.as_ref().is_none_or(|v| v.len() < 2 || v.contains(j)))
                })
            } else {
                look.take_index(names, c.side, fits)
            };
            // A role read from the shape never takes a joint a name placed.
            let shape = shaped
                .get(&(c.id.clone(), guide))
                .copied()
                .filter(|&j| !look.used[j]);
            let found = by_name.map(|j| (j, false)).or(shape.map(|j| (j, true)));
            if let Some((j, from_shape)) = found
                && let Some(gd) = c.guides.iter_mut().find(|x| x.name == guide)
            {
                gd.position = joints[j].position;
                if guide != "chest" {
                    look.used[j] = true;
                }
                report.placed.push(format!("{}.{guide}", c.id));
                if from_shape {
                    report.by_shape.push(format!("{}.{guide}", c.id));
                }
            }
        }
    }

    // Eyes and jaw.
    if look.has(&["eye"], Side::Left) && look.has(&["eye"], Side::Right) {
        let id = add(&mut g, Preset::Eyes, AddOptions::default(), &mut report)?;
        for (guide, side) in [("eye_l", Side::Left), ("eye_r", Side::Right)] {
            if let Some(p) = look.take(&["eye"], side) {
                place(&mut g, &id, guide, p, &mut report);
            }
        }
    }
    if look.has(&["jaw"], Side::Center) {
        let id = add(&mut g, Preset::Jaw, AddOptions::default(), &mut report)?;
        let jaw = look.take(&["jaw"], Side::Center);
        if let Some(p) = jaw {
            place(&mut g, &id, "jaw", p, &mut report);
        }
        if let Some(p) = look.take(&["chin", "jaw_end", "jaw_tip"], Side::Center) {
            place(&mut g, &id, "chin", p, &mut report);
        }
    }

    // Face joints, by their own names or a common alias.
    let face: Vec<(&str, Vec<&str>, Side)> = FACE_GUIDES
        .iter()
        .map(|&guide| {
            let (side, base) = match guide.strip_suffix("_l") {
                Some(b) => (Side::Left, b),
                None => match guide.strip_suffix("_r") {
                    Some(b) => (Side::Right, b),
                    None => (Side::Center, guide),
                },
            };
            let mut names = vec![base];
            if let Some((_, more)) = FACE_ALIASES.iter().find(|(b, _)| *b == base) {
                names.extend_from_slice(more);
            }
            (guide, names, side)
        })
        .collect();
    if face.iter().any(|(_, names, side)| look.has(names, *side)) {
        let id = add(&mut g, Preset::Face, AddOptions::default(), &mut report)?;
        let eyes = g.components.iter().find(|c| c.kind == Kind::Eyes);
        let eye = |side: Side| eyes.and_then(|c| c.position(&format!("eye{}", side.suffix())));
        let (eye_l, eye_r) = (eye(Side::Left), eye(Side::Right));
        let near = 0.002 * g.body.height;
        for (guide, names, side) in &face {
            let Some(p) = look.take(names, *side) else {
                continue;
            };
            // A lid joint that pivots on the eye says where the lid turns, not
            // where its guide sits; keep the guide the face preset placed.
            let pivot = if *side == Side::Left { eye_l } else { eye_r };
            if guide.starts_with("lid_") && pivot.is_some_and(|e| e.distance(p) < near) {
                continue;
            }
            place(&mut g, &id, guide, p, &mut report);
        }
    }

    // Fingers: `index_01_l`, `LeftHandIndex1`, `L_index1_jnt` ...
    for side in [Side::Left, Side::Right] {
        let found: Vec<(usize, Vec<Vec3>)> = FINGERS
            .iter()
            .enumerate()
            .filter_map(|(f, name)| {
                let points = finger_joints(&mut look, name, side);
                (points.len() >= 3).then_some((f, points))
            })
            .collect();
        if found.is_empty() {
            continue;
        }
        let opts = AddOptions {
            side: Some(side),
            ..AddOptions::default()
        };
        components::add_preset(&mut g, Preset::Fingers, &opts)?;
        for (f, name) in FINGERS.iter().enumerate() {
            let id = format!("{name}{}", side.suffix());
            match found.iter().find(|(i, _)| *i == f) {
                Some((_, points)) => {
                    let mut pts = points.clone();
                    if pts.len() == 3 {
                        // No end joint: continue the last bone.
                        pts.push(pts[2] + (pts[2] - pts[1]));
                    }
                    for (k, p) in pts.iter().take(4).enumerate() {
                        place(&mut g, &id, &format!("j{}", k + 1), *p, &mut report);
                    }
                    report.added.push(id);
                }
                None => {
                    components::remove(&mut g, &id)?;
                }
            }
        }
    }

    // The skeleton's pelvis marks the mid-line better than the mesh's bounds
    // (one arm raised or longer moves those).
    if report.placed.iter().any(|p| p == "spine.pelvis")
        && let Some(pelvis) = g.component("spine").and_then(|c| c.position("pelvis"))
    {
        g.body.center_x = pelvis.x;
        if let Some(c) = g.components.iter_mut().find(|c| c.kind == Kind::Root)
            && let Some(gd) = c.guides.first_mut()
        {
            gd.position.x = pelvis.x;
        }
    }
    // Ears, tails, hair and other parts the layout has no guide for.
    add_unknown_chains(&mut g, joints, &mut look.used, &mut report);
    components::build(&g).context("the guides read from the joints don't build")?;
    report.unused = joints
        .iter()
        .zip(&look.used)
        .filter(|(_, used)| !**used)
        .map(|(j, _)| j.name.clone())
        .collect();
    Ok((g, report))
}

/// Joints by (component id, guide) for the biped layout, from the skeleton's shape.
fn biped_roles(m: &BodyMap, joints: &[Joint]) -> HashMap<(String, &'static str), usize> {
    let mut out = HashMap::new();
    out.insert(("spine".to_string(), "pelvis"), m.pelvis);
    if let Some(chest) = m.chest {
        let top = joints
            .iter()
            .map(|j| j.position.y)
            .fold(f32::NEG_INFINITY, f32::max);
        let h = &m.head;
        out.insert(
            ("spine".to_string(), "chest"),
            h.first().copied().unwrap_or(chest),
        );
        match h.len() {
            0 => {}
            1 => {
                out.insert(("head".to_string(), "neck"), chest);
                out.insert(("head".to_string(), "head"), h[0]);
            }
            n => {
                out.insert(("head".to_string(), "neck"), h[0]);
                // The last joint is the top of the head only when it is the
                // top of the skeleton; then the head is the one before it.
                let last = h[n - 1];
                let is_top = n >= 3 && top - joints[last].position.y < 1e-4;
                if is_top {
                    out.insert(("head".to_string(), "head"), h[n - 2]);
                    out.insert(("head".to_string(), "head_top"), last);
                } else {
                    out.insert(("head".to_string(), "head"), last);
                }
            }
        }
    }
    for (k, side) in ["l", "r"].iter().enumerate() {
        let arm = &m.arms[k];
        let r = structure::arm_roles(joints, arm);
        for (guide, role) in [
            ("clavicle", r.clavicle),
            ("shoulder", r.shoulder),
            ("elbow", r.elbow),
            ("wrist", r.wrist),
            ("hand_tip", r.hand_tip),
        ] {
            if let Some(i) = role {
                out.insert((format!("arm_{side}"), guide), arm[i]);
            }
        }
        let leg = &m.legs[k];
        let r = structure::leg_roles(joints, leg);
        for (guide, role) in [
            ("hip", r.hip),
            ("knee", r.knee),
            ("ankle", r.ankle),
            ("ball", r.ball),
            ("toe", r.toe),
        ] {
            if let Some(i) = role {
                out.insert((format!("leg_{side}"), guide), leg[i]);
            }
        }
    }
    out
}

/// The dog layout stretched over a four-legged body: as tall as it, as long
/// (nose to tail tip) as it along +Z, standing on its floor at its mid-line.
pub fn fit_quadruped(mesh: &crate::mesh::Mesh) -> Guides {
    let (min, max) = mesh.bounds();
    let h = (max.y - min.y).max(1e-3);
    let mut g = components::quadruped(h);
    let (lo, hi) = g
        .components
        .iter()
        .flat_map(|c| c.guides.iter())
        .fold((f32::INFINITY, f32::NEG_INFINITY), |a, gd| {
            (a.0.min(gd.position.z), a.1.max(gd.position.z))
        });
    let scale_z = (max.z - min.z) / (hi - lo).max(1e-6);
    let cx = 0.5 * (min.x + max.x);
    for c in &mut g.components {
        for gd in &mut c.guides {
            let p = gd.position;
            gd.position = Vec3::new(cx + p.x, min.y + p.y, min.z + (p.z - lo) * scale_z);
        }
    }
    g.body.floor = min.y;
    g.body.center_x = cx;
    g.body.crotch_y += min.y;
    g
}

fn put(g: &mut Guides, id: &str, guide: &str, at: Vec3, report: Option<&mut JointReport>) {
    if let Some(c) = g.components.iter_mut().find(|c| c.id == id)
        && let Some(gd) = c.guides.iter_mut().find(|x| x.name == guide)
    {
        gd.position = at;
        if let Some(report) = report {
            report.placed.push(format!("{id}.{guide}"));
            report.by_shape.push(format!("{id}.{guide}"));
        }
    }
}

/// Move the dog layout's guides onto a four-legged skeleton: the spine from the
/// pelvis to the shoulders, neck and head, the front legs (shoulder, elbow,
/// wrist), the hind legs (hip, knee, hock, and the paw under it) and the tail.
/// `None` when the skeleton doesn't show four legs.
pub fn quad_guides_from_joints(
    fitted: Guides,
    joints: &[Joint],
) -> Result<Option<(Guides, JointReport)>> {
    let Some(m) = structure::map(joints, &fitted.body, BodyKind::Quadruped) else {
        return Ok(None);
    };
    let mut g = fitted;
    let mut report = JointReport::default();
    let mut used = vec![false; joints.len()];
    let p = |i: usize| joints[i].position;
    used[m.pelvis] = true;
    put(&mut g, "spine", "pelvis", p(m.pelvis), Some(&mut report));
    if let Some(chest) = m.chest {
        if (1..=16).contains(&m.spine.len()) {
            components::set_spine_joints(&mut g, "spine", m.spine.len())?;
        }
        for &i in &m.spine {
            used[i] = true;
        }
        let base = m.head.first().copied().unwrap_or(chest);
        put(&mut g, "spine", "chest", p(base), Some(&mut report));
        let head_names = ["neck", "head", "head_top"];
        for (n, &i) in head_names.iter().zip(&m.head) {
            used[i] = true;
            put(&mut g, "head", n, p(i), Some(&mut report));
        }
    }
    for (k, side) in ["l", "r"].iter().enumerate() {
        // Front legs: an arm laid out as a leg.
        let arm = &m.arms[k];
        let r = structure::arm_roles(joints, arm);
        let id = format!("front_leg_{side}");
        let before = g.component(&id).cloned();
        for (guide, role) in [
            ("clavicle", r.clavicle),
            ("shoulder", r.shoulder),
            ("elbow", r.elbow),
            ("wrist", r.wrist),
            ("hand_tip", r.hand_tip),
        ] {
            if let Some(i) = role {
                used[arm[i]] = true;
                put(&mut g, &id, guide, p(arm[i]), Some(&mut report));
            }
        }
        // Without a hand end, the paw keeps the layout's length past the wrist.
        if r.hand_tip.is_none()
            && let (Some(c), Some(i)) = (&before, r.wrist)
            && let (Some(old), Some(tip)) = (c.position("wrist"), c.position("hand_tip"))
        {
            put(&mut g, &id, "hand_tip", p(arm[i]) + (tip - old), None);
        }
        // Hind legs: hip, knee and hock from the chain; the paw keeps the
        // layout's shape under the hock, on the joint at the floor if there is one.
        let leg = &m.legs[k];
        let id = format!("hind_leg_{side}");
        let Some(c) = g.component(&id) else { continue };
        let old: Vec<Vec3> = c.guides.iter().map(|x| x.position).collect();
        let floor = g.body.floor;
        for (n, &i) in ["hip", "knee", "hock"].iter().zip(leg.iter()) {
            used[i] = true;
            put(&mut g, &id, n, p(i), Some(&mut report));
        }
        if leg.len() >= 3 && old.len() == 7 {
            let shift = p(leg[2]) - old[2];
            let mut paw = [old[3], old[4], old[5], old[6]]
                .map(|q| Vec3::new(q.x + shift.x, q.y, q.z + shift.z));
            if let Some(&end) = leg.get(3) {
                used[end] = true;
                let e = p(end);
                if e.y - floor < 0.05 * g.body.height {
                    // The joint at the floor is the ball of the paw.
                    let d = Vec3::new(e.x - paw[1].x, 0.0, e.z - paw[1].z);
                    paw = paw.map(|q| q + d);
                    paw[1] = e;
                    // The ankle (where the paw starts) on the way down from the
                    // hock, at the layout's height for it.
                    let hock = p(leg[2]);
                    let target = paw[0].y.clamp(e.y, hock.y);
                    let t = (hock.y - target) / (hock.y - e.y).max(1e-6);
                    paw[0] = hock.lerp(e, t);
                } else {
                    paw[0] = e;
                }
            }
            for (n, q) in ["ankle", "ball", "toe", "heel"].iter().zip(paw) {
                put(&mut g, &id, n, q, None);
            }
        }
    }
    // Tail: its joints and a tip continuing the last bone.
    if m.tail.len() >= 2
        && let Some(c) = g.components.iter_mut().find(|c| c.id == "tail")
    {
        let mut pts: Vec<Vec3> = m.tail.iter().map(|&i| p(i)).collect();
        let n = pts.len();
        pts.push(pts[n - 1] + (pts[n - 1] - pts[n - 2]));
        c.guides = pts
            .into_iter()
            .enumerate()
            .map(|(k, position)| Guide {
                name: format!("j{}", k + 1),
                position,
            })
            .collect();
        for &i in &m.tail {
            used[i] = true;
        }
        report.placed.push("tail".into());
        report.by_shape.push("tail".into());
    }
    add_unknown_chains(&mut g, joints, &mut used, &mut report);
    components::build(&g).context("the dog guides read from the joints don't build")?;
    report.unused = joints
        .iter()
        .zip(&used)
        .filter(|(_, u)| !**u)
        .map(|(j, _)| j.name.clone())
        .collect();
    Ok(Some((g, report)))
}

/// Name tokens of rig helpers (IK targets, twist and roll joints, sockets),
/// which are never body parts of their own.
const HELPERS: [&str; 12] = [
    "ik",
    "twist",
    "roll",
    "helper",
    "corrective",
    "pole",
    "target",
    "socket",
    "dummy",
    "ctrl",
    "locator",
    "ikh",
];

/// Name tokens of a chain's end joint (it marks where the last bone ends).
const ENDS: [&str; 5] = ["end", "tip", "nub", "top", "leaf"];

fn tokens(name: &str) -> Vec<String> {
    let (_, names) = candidates(name);
    names
        .first()
        .map(|n| n.split('_').map(str::to_string).collect())
        .unwrap_or_default()
}

fn is_helper(name: &str) -> bool {
    let t = tokens(name);
    t.iter()
        .any(|t| HELPERS.contains(&t.as_str()) || t.contains("twist"))
}

fn is_end(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    tokens(name).last().is_some_and(|t| {
        ENDS.contains(&t.as_str()) || ENDS.iter().any(|e| t.ends_with(e) && t.len() > e.len() + 2)
    }) || lower.ends_with("_end")
}

/// A component id from a joint's name: its words without side, numbers or
/// joint suffixes (`Ear.L` -> `ear`, `hair_03` -> `hair`, `b_Tail01_019` ->
/// `b_tail`), or `chain` when nothing is left.
fn chain_id_base(name: &str) -> String {
    let words: Vec<String> = tokens(name)
        .into_iter()
        .map(|t| {
            t.trim_end_matches(|c: char| c.is_ascii_digit())
                .chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .collect::<String>()
        })
        .filter(|t| !t.is_empty())
        .collect();
    let id = words.join("_");
    if id.is_empty() || id.starts_with(|c: char| c.is_ascii_digit()) {
        "chain".into()
    } else {
        id
    }
}

/// Read the joints no guide took that hang from the rest of the skeleton
/// (ears, tails, hair, antennae: branches whose every joint went unused) back
/// as FK chain components, each hung from the rig bone at the joint it grows
/// from. A branching part becomes a chain along its longest run, with chains
/// for its side branches hung from it. Helpers (IK, twist, roll, sockets) and
/// single joints with nothing to aim at are left out. Returns the added ids.
pub fn add_unknown_chains(
    g: &mut Guides,
    joints: &[Joint],
    used: &mut [bool],
    report: &mut JointReport,
) -> Vec<String> {
    let n = joints.len();
    let mut children = vec![Vec::new(); n];
    for (i, j) in joints.iter().enumerate() {
        if let Some(p) = j.parent.filter(|&p| p < n && p != i) {
            children[p].push(i);
        }
    }
    // Whether every joint below (and at) i went unused; a cycle counts as used.
    let mut free = vec![None::<bool>; n];
    fn all_free(
        i: usize,
        used: &[bool],
        children: &[Vec<usize>],
        free: &mut [Option<bool>],
    ) -> bool {
        let mut order = vec![i];
        let mut k = 0;
        while k < order.len() {
            let at = order[k];
            if order.len() > used.len() {
                return false;
            }
            order.extend(children[at].iter().copied());
            k += 1;
        }
        for &at in order.iter().rev() {
            let v = !used[at] && children[at].iter().all(|&c| free[c].unwrap_or(false));
            free[at] = Some(v);
        }
        free[i].unwrap_or(false)
    }
    let h = g.body.height.max(1e-6);
    let pos = |i: usize| joints[i].position;
    let reach = |i: usize, children: &[Vec<usize>]| -> f32 {
        let mut best = 0.0f32;
        let mut stack = vec![(i, 0.0f32, 0usize)];
        while let Some((at, d, depth)) = stack.pop() {
            best = best.max(d);
            if depth > n {
                break;
            }
            for &c in &children[at] {
                stack.push((c, d + pos(at).distance(pos(c)), depth + 1));
            }
        }
        best
    };
    // Branch roots: free subtrees hanging from a joint a guide took (a part
    // hanging from joints nothing recognised is more likely a misread limb).
    let mut roots: Vec<(usize, Option<usize>)> = (0..n)
        .filter(|&i| {
            let Some(p) = joints[i].parent.filter(|&p| p < n) else {
                return false;
            };
            used[p] && all_free(i, used, &children, &mut free)
        })
        .map(|i| (i, None))
        .collect();
    let mut added = Vec::new();
    // Each entry: the root joint and, for a side branch, the chain it grows from.
    while let Some((root, from_chain)) = roots.pop() {
        if is_helper(&joints[root].name) {
            continue;
        }
        // The run: on through the child reaching furthest; other children
        // start their own chains.
        let mut path = vec![root];
        let mut branches: Vec<(usize, usize)> = Vec::new(); // (joint, path index)
        loop {
            let at = *path.last().unwrap();
            let kids: Vec<usize> = children[at]
                .iter()
                .copied()
                .filter(|&c| !is_helper(&joints[c].name))
                .collect();
            let Some(&next) = kids.iter().max_by(|a, b| {
                (reach(**a, &children) + pos(at).distance(pos(**a)))
                    .total_cmp(&(reach(**b, &children) + pos(at).distance(pos(**b))))
            }) else {
                break;
            };
            for &k in &kids {
                if k != next {
                    branches.push((k, path.len() - 1));
                }
            }
            if path.len() > n {
                break;
            }
            path.push(next);
        }
        let mut points: Vec<Vec3> = Vec::new();
        for &j in &path {
            if points.last().is_none_or(|p| p.distance(pos(j)) > 1e-4 * h) {
                points.push(pos(j));
            }
        }
        let last = *path.last().unwrap();
        if points.len() >= 2 && !is_end(&joints[last].name) {
            // No end joint: carry the last bone on, as long again.
            let k = points.len();
            points.push(points[k - 1] + (points[k - 1] - points[k - 2]));
        }
        if points.len() < 2 || points[0].distance(*points.last().unwrap()) < 0.005 * h {
            continue;
        }
        let Ok(built) = components::build(g) else {
            break;
        };
        // The bone it hangs from: the rig bone through the parent joint
        // (starting there, when several are), or the chain it branches from.
        let anchor = match from_chain {
            Some(j) => pos(j),
            None => pos(joints[root].parent.unwrap()),
        };
        let bones = &built.skeleton.bones;
        let Some(parent) = (0..bones.len()).min_by(|&a, &b| {
            let key = |i: usize| {
                let d = bones[i].distance_to(anchor);
                ((d / (1e-3 * h)).round(), bones[i].head.distance(anchor))
            };
            let (ka, kb) = (key(a), key(b));
            ka.0.total_cmp(&kb.0).then(ka.1.total_cmp(&kb.1))
        }) else {
            break;
        };
        let x = points[0].x - g.body.center_x;
        let side = if x > 0.01 * h {
            Side::Left
        } else if x < -0.01 * h {
            Side::Right
        } else {
            Side::Center
        };
        let id = components::unique_id(g, &chain_id_base(&joints[root].name), side);
        let parent_name = bones[parent].name.clone();
        g.components
            .push(components::chain(id.clone(), side, &parent_name, points));
        if components::build(g).is_err() {
            g.components.pop();
            continue;
        }
        for &j in &path {
            used[j] = true;
        }
        report.added.push(id.clone());
        added.push(id);
        for (b, k) in branches {
            roots.push((b, Some(path[k])));
        }
    }
    added
}

/// A finger's joints from its base to its tip, as many as the skeleton has.
fn finger_joints(look: &mut Lookup, finger: &str, side: Side) -> Vec<Vec3> {
    let aliases: Vec<&str> = if finger == "pinky" {
        vec!["pinky", "little", "pinkie"]
    } else {
        vec![finger]
    };
    let mut points = Vec::new();
    for k in 1..=4 {
        let names: Vec<String> = aliases
            .iter()
            .flat_map(|a| {
                let mut v = vec![
                    format!("{a}_{k:02}"),
                    format!("{a}{k}"),
                    format!("{a}_{k}"),
                    format!("hand{a}{k}"),
                    format!("finger_{a}_{k:02}"),
                ];
                if k == 4 {
                    v.push(format!("{a}_end"));
                    v.push(format!("{a}_03_end"));
                    v.push(format!("{a}_tip"));
                }
                v
            })
            .collect();
        let refs: Vec<&str> = names.iter().map(String::as_str).collect();
        match look.take(&refs, side) {
            Some(p) => points.push(p),
            None => break,
        }
    }
    points
}

fn add(
    g: &mut Guides,
    preset: Preset,
    opts: AddOptions,
    report: &mut JointReport,
) -> Result<String> {
    let ids = components::add_preset(g, preset, &opts)?;
    report.added.extend(ids.iter().cloned());
    ids.into_iter().next().context("nothing was added")
}

fn place(g: &mut Guides, id: &str, guide: &str, p: Vec3, report: &mut JointReport) {
    if let Some(c) = g.components.iter_mut().find(|c| c.id == id)
        && let Some(gd) = c.guides.iter_mut().find(|x| x.name == guide)
    {
        gd.position = p;
        report.placed.push(format!("{id}.{guide}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{demo, rig};

    #[test]
    fn joint_names_from_other_tools() {
        let cases = [
            ("mixamorig:LeftUpLeg", Side::Left, "upleg"),
            ("Bip01 L UpperArm", Side::Left, "upperarm"),
            ("L_upperarm_jnt", Side::Left, "upperarm"),
            ("upperarm_R_JNT", Side::Right, "upperarm"),
            ("thigh.L", Side::Left, "thigh"),
            ("Character1_RightHand", Side::Right, "hand"),
            ("spine_02", Side::Center, "spine_02"),
            ("ik_hand_l", Side::Left, "hand"),
        ];
        for (name, side, base) in cases {
            let (s, names) = candidates(name);
            assert_eq!(s, side, "{name}");
            assert!(names.iter().any(|n| n == base), "{name}: {names:?}");
        }
        // The real hand wins over an IK helper of the same name.
        let joints = [
            Joint {
                name: "ik_hand_l".into(),
                position: Vec3::ONE,
                parent: None,
            },
            Joint {
                name: "hand_l".into(),
                position: Vec3::X,
                parent: None,
            },
        ];
        let mut look = Lookup::new(&joints);
        assert_eq!(look.take(&["hand"], Side::Left), Some(Vec3::X));
    }

    /// Our own export, renamed Mixamo-style and moved, comes back as the same
    /// layout with its fingers, eyes, jaw and face.
    #[test]
    fn rebuilds_a_skeleton_as_guides() {
        let mesh = demo::humanoid(0.0);
        let (mut guides, _) = rig::detect_guides(&mesh);
        for p in [Preset::Fingers, Preset::Eyes, Preset::Jaw, Preset::Face] {
            components::add_preset(&mut guides, p, &AddOptions::default()).unwrap();
        }
        let built = components::build(&guides).unwrap();
        let shift = Vec3::new(0.0, 0.0, 0.01);
        let mixamo = |n: &str| -> String {
            let map = [
                ("pelvis", "Hips"),
                ("upperarm_l", "LeftArm"),
                ("clavicle_l", "LeftShoulder"),
                ("lowerarm_l", "LeftForeArm"),
                ("hand_l", "LeftHand"),
                ("thigh_l", "LeftUpLeg"),
                ("calf_l", "LeftLeg"),
                ("index_01_l", "LeftHandIndex1"),
                ("index_02_l", "LeftHandIndex2"),
                ("index_03_l", "LeftHandIndex3"),
            ];
            let n = map.iter().find(|(a, _)| *a == n).map_or(n, |(_, b)| b);
            format!("mixamorig:{n}")
        };
        let joints: Vec<Joint> = built
            .skeleton
            .bones
            .iter()
            .map(|b| Joint {
                name: mixamo(&b.name),
                position: b.head + shift,
                parent: b.parent,
            })
            .collect();
        let (fresh, _) = rig::detect_guides(&mesh);
        let (g, report) = guides_from_joints(fresh, &joints).unwrap();
        let pos = |id: &str, guide: &str| g.component(id).unwrap().position(guide).unwrap();
        let want = |bone: &str| built.skeleton.bone(bone).unwrap().head + shift;
        assert!(pos("arm_l", "shoulder").distance(want("upperarm_l")) < 1e-5);
        assert!(pos("arm_l", "clavicle").distance(want("clavicle_l")) < 1e-5);
        assert!(pos("leg_l", "knee").distance(want("calf_l")) < 1e-5);
        assert!(pos("face", "brow_in_l").distance(want("brow_in_l")) < 1e-5);
        assert!(pos("eyes", "eye_r").distance(want("eye_r")) < 1e-5);
        assert!(pos("index_l", "j2").distance(want("index_02_l")) < 1e-5);
        // Lids pivot on the eye, so their guides stay off the eye.
        assert!(pos("face", "lid_up_l").distance(pos("eyes", "eye_l")) > 1e-3);
        for id in ["eyes", "jaw", "face", "index_l", "pinky_r"] {
            assert!(
                report.added.iter().any(|a| a == id),
                "{id}: {:?}",
                report.added
            );
        }
        assert!(report.unused.iter().any(|u| u.contains("twist")));
    }

    /// Ears, a branching tail and hair the layout has no guides for come back
    /// as chains on the bones they grow from; IK helpers and lone end joints
    /// don't.
    #[test]
    fn unknown_chains_become_chain_components() {
        let mesh = demo::humanoid(0.0);
        let (guides, _) = rig::detect_guides(&mesh);
        let built = components::build(&guides).unwrap();
        let sk = &built.skeleton;
        let h = guides.body.height;
        let mut joints: Vec<Joint> = sk
            .bones
            .iter()
            .map(|b| Joint {
                name: b.name.clone(),
                position: b.head,
                parent: b.parent,
            })
            .collect();
        let chain = |joints: &mut Vec<Joint>, from: &str, names: &[&str], step: Vec3| {
            let mut parent = joints.iter().position(|j| j.name == from).unwrap();
            let mut at = joints[parent].position;
            let mut out = Vec::new();
            for n in names {
                at += step * h;
                joints.push(Joint {
                    name: n.to_string(),
                    position: at,
                    parent: Some(parent),
                });
                parent = joints.len() - 1;
                out.push(parent);
            }
            out
        };
        let head_top = sk.bone("head").unwrap().tail;
        let ear_l = chain(
            &mut joints,
            "head",
            &["Ear.L", "Ear.L.001", "Ear_end.L"],
            Vec3::new(0.04, 0.03, 0.0),
        );
        let ear_r = chain(
            &mut joints,
            "head",
            &["Ear.R", "Ear.R.001", "Ear_end.R"],
            Vec3::new(-0.04, 0.03, 0.0),
        );
        // The ears start off the head joint: move them up beside its top.
        for (k, &j) in ear_l.iter().chain(&ear_r).enumerate() {
            joints[j].position.y +=
                head_top.y - sk.bone("head").unwrap().head.y - 0.02 * h * (k % 3) as f32;
        }
        let tail = chain(
            &mut joints,
            "pelvis",
            &["tail1", "tail2", "tail3", "tail4"],
            Vec3::new(0.0, -0.03, -0.06),
        );
        // A side branch off the second tail joint (a fin), unnamed.
        let fin_parent = tail[1];
        let p = joints[fin_parent].position;
        joints.push(Joint {
            name: "Bone.012".into(),
            position: p + Vec3::new(0.0, 0.05, 0.0) * h,
            parent: Some(fin_parent),
        });
        joints.push(Joint {
            name: "Bone.013".into(),
            position: p + Vec3::new(0.0, 0.09, -0.01) * h,
            parent: Some(joints.len() - 1),
        });
        let hair = chain(
            &mut joints,
            "head",
            &["hair_01", "hair_02", "hair_03"],
            Vec3::new(0.0, -0.04, -0.03),
        );
        // Helpers (an IK chain off the floor, so the legs are still the
        // lowest) and a lone leaf.
        chain(
            &mut joints,
            "root",
            &["ik_foot_root", "ik_foot_l"],
            Vec3::new(0.05, 0.2, 0.0),
        );
        chain(
            &mut joints,
            "hand_l",
            &["weapon_l"],
            Vec3::new(0.02, 0.0, 0.0),
        );
        let (fresh, _) = rig::detect_guides(&mesh);
        let (g, report) = guides_from_joints(fresh, &joints).unwrap();
        let c = |id: &str| {
            g.component(id)
                .unwrap_or_else(|| panic!("no {id}: {:?}", report.added))
        };
        let at = |j: usize| joints[j].position;
        // Ears on the head, on their own sides, through their joints to the end joint.
        for (id, js, side) in [
            ("ear_l", &ear_l, Side::Left),
            ("ear_r", &ear_r, Side::Right),
        ] {
            let e = c(id);
            assert_eq!(e.kind, Kind::Chain);
            assert_eq!(e.side, side);
            assert_eq!(e.parent.as_deref(), Some("head"));
            assert_eq!(e.guides.len(), 3);
            for (gd, &j) in e.guides.iter().zip(js.iter()) {
                assert!(gd.position.distance(at(j)) < 1e-6);
            }
        }
        // The tail from the pelvis, carried one bone past its last joint; the
        // fin on the tail's second bone.
        let t = c("tail");
        assert_eq!(t.parent.as_deref(), Some("pelvis"));
        assert_eq!(t.guides.len(), 5);
        let fin = g
            .components
            .iter()
            .find(|x| x.id.starts_with("bone"))
            .expect("fin");
        assert_eq!(fin.parent.as_deref(), Some("tail_02"));
        let hr = c("hair");
        assert_eq!(hr.parent.as_deref(), Some("head"));
        assert!(hr.guides[0].position.distance(at(hair[0])) < 1e-6);
        assert!(
            !g.components
                .iter()
                .any(|x| x.id.contains("ik") || x.id.contains("weapon"))
        );
        assert!(
            report.unused.iter().any(|u| u == "ik_foot_l"),
            "{:?} {:?}",
            report.unused,
            report.added
        );
        assert!(
            !report
                .unused
                .iter()
                .any(|u| u.starts_with("Ear") || u.starts_with("tail"))
        );
        components::build(&g).unwrap();
        // The same on the dog: ears on its head.
        let dog = components::quadruped(demo::DOG_HEIGHT);
        let built = components::build(&dog).unwrap();
        let mut joints: Vec<Joint> = built
            .skeleton
            .bones
            .iter()
            .map(|b| Joint {
                name: String::new(),
                position: b.head,
                parent: b.parent,
            })
            .collect();
        for (i, b) in built.skeleton.bones.iter().enumerate() {
            if b.tail.y < 0.03 * dog.body.height
                && !built.skeleton.bones.iter().any(|c| c.parent == Some(i))
            {
                joints.push(Joint {
                    name: String::new(),
                    position: b.tail,
                    parent: Some(i),
                });
            }
        }
        let head = built.skeleton.index_of("head").unwrap();
        let top = built.skeleton.bones[head].tail;
        for s in [1.0f32, -1.0] {
            let d = dog.body.height;
            joints.push(Joint {
                name: "ear".into(),
                position: top + Vec3::new(0.05 * s, 0.0, -0.02) * d,
                parent: Some(head),
            });
            joints.push(Joint {
                name: "ear_tip".into(),
                position: top + Vec3::new(0.08 * s, 0.1, -0.03) * d,
                parent: Some(joints.len() - 1),
            });
        }
        let fitted = fit_quadruped(&demo::demo_quadruped());
        let (g, report) = quad_guides_from_joints(fitted, &joints).unwrap().unwrap();
        for id in ["ear_l", "ear_r"] {
            let e = g
                .component(id)
                .unwrap_or_else(|| panic!("no {id}: {:?}", report.added));
            assert_eq!(e.parent.as_deref(), Some("head"));
        }
    }
}
