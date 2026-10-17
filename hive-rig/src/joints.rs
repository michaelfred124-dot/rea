//! Joints to guides: read an existing skeleton back into editable guides.
//!
//! A character that already has joints (from Maya, Blender, Mixamo, an Unreal
//! mannequin or a 3ds Max biped) keeps its joint placement: each guide whose
//! joint can be recognised by name moves onto that joint, eyes, a jaw, face
//! joints and fingers are added when the skeleton has them, and every other
//! guide keeps the position detected on the mesh.

use std::collections::HashMap;

use anyhow::{Context, Result};
use glam::Vec3;

use crate::components::{self, AddOptions, FACE_GUIDES, Guides, Kind, Preset};
use crate::skeleton::Side;

/// A joint of an imported skeleton, in the mesh's model space.
#[derive(Debug, Clone)]
pub struct Joint {
    pub name: String,
    pub position: Vec3,
}

/// What [`guides_from_joints`] did.
#[derive(Debug, Clone, Default)]
pub struct JointReport {
    /// Guides placed on a joint, as `component.guide`.
    pub placed: Vec<String>,
    /// Components added because the skeleton has them (eyes, jaw, face, fingers).
    pub added: Vec<String>,
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
        for n in names {
            if let Some(&(_, j)) = self.best.get(&(n.to_string(), side))
                && !self.used[j]
            {
                self.used[j] = true;
                return Some(self.joints[j].position);
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
        for &(kind, guide, names) in BIPED {
            if kind != kind_name {
                continue;
            }
            // The chest guide marks the top of the spine, where the neck starts,
            // without using up the neck joint.
            let found = if guide == "chest" {
                names.iter().find_map(|n| {
                    look.best
                        .get(&(n.to_string(), Side::Center))
                        .map(|&(_, j)| joints[j].position)
                })
            } else {
                look.take(names, c.side)
            };
            if let Some(p) = found
                && let Some(gd) = c.guides.iter_mut().find(|x| x.name == guide)
            {
                gd.position = p;
                report.placed.push(format!("{}.{guide}", c.id));
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

    components::build(&g).context("the guides read from the joints don't build")?;
    report.unused = joints
        .iter()
        .zip(&look.used)
        .filter(|(_, used)| !**used)
        .map(|(j, _)| j.name.clone())
        .collect();
    Ok((g, report))
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
            },
            Joint {
                name: "hand_l".into(),
                position: Vec3::X,
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
}
