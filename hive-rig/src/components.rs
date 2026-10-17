//! Premade rig components placed by editable guides.
//!
//! A rig is an ordered list of components (root, spine, head, arms, legs, and
//! chains for fingers, tails and the like). Each component owns a few named guide
//! points. Building turns the guides into bones and animator controls. The guides
//! are saved next to the rig as `<name>.guides.json`, so they can be moved,
//! components can be added or removed, and the rig rebuilt from them.

use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::{Context, Result, bail, ensure};
use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::controls::{
    Control, ControlKind, ControlParent, ControlRig, FootRoll, IkChain, Shape, ctrl_name, fk_name,
};
use crate::landmarks::{Landmarks, mirror};
use crate::naming::Naming;
use crate::skeleton::{Region, Side, Skeleton, Twist};

pub const GUIDES_FORMAT: &str = "hive-rig-guides";
pub const GUIDES_VERSION: u32 = 1;
pub const SPINE_JOINTS: usize = 3;

/// The editable rig layout: components and their guide positions (model space,
/// Y-up, metres, facing +Z).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Guides {
    pub format: String,
    pub version: u32,
    pub body: Body,
    pub components: Vec<Component>,
    /// How exported joints and controls are named. Absent means the default
    /// (Unreal mannequin) names.
    #[serde(default, skip_serializing_if = "Naming::is_default")]
    pub naming: Naming,
    /// Changes to individual controls' look, keyed by the control's internal name.
    /// They survive rebuilds; a control that no longer exists is ignored.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub control_styles: BTreeMap<String, ControlStyle>,
}

/// How one control looks. Unset fields keep the component's default.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct ControlStyle {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<Shape>,
    /// Radius or half-extent in metres.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<[f32; 3]>,
}

/// Whole-body measurements used to split the mesh into skin regions.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Body {
    pub floor: f32,
    pub height: f32,
    pub center_x: f32,
    /// Below this height, vertices belong to a leg.
    pub crotch_y: f32,
    /// Further than this from the centre line, vertices above the crotch belong to an arm.
    pub torso_half_width: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Root,
    Spine,
    Head,
    Arm,
    Leg,
    /// FK chain of any length: fingers, tails, ears, hair, props.
    Chain,
    /// Two eyes that look at a movable target.
    Eyes,
    /// A jaw hinging open from below the ears.
    Jaw,
    /// Joint-based face: brows, eyelids, cheeks, nose and lips, each a joint
    /// with a control that moves and rotates it.
    Face,
}

/// The face component's guides, which are also its joint names.
pub const FACE_GUIDES: [&str; 17] = [
    "brow_in_l",
    "brow_mid_l",
    "brow_out_l",
    "brow_in_r",
    "brow_mid_r",
    "brow_out_r",
    "lid_up_l",
    "lid_low_l",
    "lid_up_r",
    "lid_low_r",
    "cheek_l",
    "cheek_r",
    "nose",
    "lip_up",
    "lip_low",
    "lip_corner_l",
    "lip_corner_r",
];

impl Kind {
    /// Guide names in order, or `None` for a chain (two or more guides, any names).
    pub fn guide_names(self) -> Option<&'static [&'static str]> {
        Some(match self {
            Kind::Root => &["root"],
            Kind::Spine => &["pelvis", "chest"],
            Kind::Head => &["neck", "head", "head_top"],
            Kind::Arm => &["clavicle", "shoulder", "elbow", "wrist", "hand_tip"],
            Kind::Leg => &["hip", "knee", "ankle", "ball", "toe", "heel"],
            Kind::Eyes => &["eye_l", "eye_r", "target"],
            Kind::Jaw => &["jaw", "chin"],
            Kind::Face => &FACE_GUIDES,
            Kind::Chain => return None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Component {
    pub id: String,
    pub kind: Kind,
    #[serde(default)]
    pub side: Side,
    /// Bone this component hangs from. Only the root has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    pub guides: Vec<Guide>,
    #[serde(default)]
    pub options: Options,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Guide {
    pub name: String,
    pub position: Vec3,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Options {
    /// Spine joints between the pelvis and the chest guide.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub joints: Option<usize>,
    /// Twist helper bones on arms and legs.
    #[serde(default = "yes")]
    pub twist: bool,
}

fn yes() -> bool {
    true
}

impl Default for Options {
    fn default() -> Self {
        Options {
            joints: None,
            twist: true,
        }
    }
}

impl Component {
    fn new(id: impl Into<String>, kind: Kind, side: Side, parent: Option<&str>) -> Self {
        Component {
            id: id.into(),
            kind,
            side,
            parent: parent.map(str::to_string),
            guides: Vec::new(),
            options: Options::default(),
        }
    }

    fn guide(mut self, name: impl Into<String>, position: Vec3) -> Self {
        self.guides.push(Guide {
            name: name.into(),
            position,
        });
        self
    }

    pub fn position(&self, name: &str) -> Option<Vec3> {
        self.guides
            .iter()
            .find(|g| g.name == name)
            .map(|g| g.position)
    }
}

impl Guides {
    pub fn component(&self, id: &str) -> Option<&Component> {
        self.components.iter().find(|c| c.id == id)
    }

    pub fn load(path: &std::path::Path) -> Result<Guides> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let g: Guides =
            serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        ensure!(
            g.format == GUIDES_FORMAT,
            "{} is not a hive-rig guides file",
            path.display()
        );
        ensure!(
            g.version <= GUIDES_VERSION,
            "{} was written by a newer hive-rig (guides version {})",
            path.display(),
            g.version
        );
        Ok(g)
    }
}

pub fn spine_name(i: usize) -> String {
    format!("spine_{:02}", i + 1)
}

/// `upperarm_l` -> `upperarm_twist_01_l`.
pub fn twist_name(bone: &str) -> String {
    match bone.rsplit_once('_') {
        Some((base, side)) => format!("{base}_twist_01_{side}"),
        None => format!("{bone}_twist_01"),
    }
}

/// Bone `i` (from 1) of a chain: `index_l` -> `index_01_l`, `tail` -> `tail_01`.
fn chain_bone_name(id: &str, side: Side, i: usize) -> String {
    let s = side.suffix();
    let base = if s.is_empty() {
        id
    } else {
        id.strip_suffix(s).unwrap_or(id)
    };
    format!("{base}_{i:02}{s}")
}

// --------------------------------------------------------------------------- templates

/// The default biped layout (mannequin bone names) placed on detected landmarks.
pub fn biped(lm: &Landmarks) -> Guides {
    let floor_center = Vec3::new(lm.center_x, lm.floor, lm.pelvis.z);
    let chest_bone = spine_name(SPINE_JOINTS - 1);
    let mut components = vec![
        Component::new("root", Kind::Root, Side::Center, None).guide("root", floor_center),
        Component::new("spine", Kind::Spine, Side::Center, Some("root"))
            .guide("pelvis", lm.pelvis)
            .guide("chest", lm.neck),
        Component::new("head", Kind::Head, Side::Center, Some(&chest_bone))
            .guide("neck", lm.neck)
            .guide("head", lm.neck.lerp(lm.head_top, 0.25))
            .guide("head_top", lm.head_top),
    ];
    for side in [Side::Left, Side::Right] {
        let m = |p: Vec3| {
            if side == Side::Left {
                p
            } else {
                mirror(p, lm.center_x)
            }
        };
        let s = side.suffix();
        components.push(
            Component::new(format!("arm{s}"), Kind::Arm, side, Some(&chest_bone))
                .guide("clavicle", m(lm.clavicle_l))
                .guide("shoulder", m(lm.shoulder_l))
                .guide("elbow", m(lm.elbow_l))
                .guide("wrist", m(lm.wrist_l))
                .guide("hand_tip", m(lm.hand_tip_l)),
        );
    }
    for side in [Side::Left, Side::Right] {
        let m = |p: Vec3| {
            if side == Side::Left {
                p
            } else {
                mirror(p, lm.center_x)
            }
        };
        let s = side.suffix();
        components.push(
            Component::new(format!("leg{s}"), Kind::Leg, side, Some("pelvis"))
                .guide("hip", m(lm.hip_l))
                .guide("knee", m(lm.knee_l))
                .guide("ankle", m(lm.ankle_l))
                .guide("ball", m(lm.ball_l))
                .guide("toe", m(lm.toe_l))
                .guide("heel", m(lm.heel_l)),
        );
    }
    Guides {
        format: GUIDES_FORMAT.into(),
        version: GUIDES_VERSION,
        body: Body {
            floor: lm.floor,
            height: lm.height,
            center_x: lm.center_x,
            crotch_y: lm.crotch_y,
            torso_half_width: lm.torso_half_width,
        },
        components,
        naming: Naming::default(),
        control_styles: BTreeMap::new(),
    }
}

/// Premade components that can be added to an existing layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Preset {
    /// Thumb, index, middle, ring and pinky on a hand (3 joints each).
    Fingers,
    /// A tail behind the pelvis.
    Tail,
    /// A plain FK chain continuing from any bone (`--parent`).
    Chain,
    /// Eyes on the head with a look-at control.
    Eyes,
    /// A jaw on the head.
    Jaw,
    /// Face joints: brows, eyelids, cheeks, nose and lips.
    Face,
    /// Five toes on each foot, from the ball of the foot (3 joints each).
    Toes,
}

#[derive(Debug, Clone, Default)]
pub struct AddOptions {
    /// Which hands get fingers (both when unset).
    pub side: Option<Side>,
    /// Bone to attach to (tail: `pelvis`; chain: required).
    pub parent: Option<String>,
    /// Bones in a tail or chain.
    pub joints: Option<usize>,
    /// Component id for a tail or chain.
    pub id: Option<String>,
}

/// Add a premade component with guides placed from the current layout. Returns the
/// ids of the added components.
pub fn add_preset(g: &mut Guides, preset: Preset, opts: &AddOptions) -> Result<Vec<String>> {
    let built = build(g).context("the current guides don't build")?;
    let sk = &built.skeleton;
    let h = g.body.height;
    let mut added = Vec::new();
    match preset {
        Preset::Fingers => {
            let sides = match opts.side {
                Some(Side::Center) => bail!("fingers need --side left or right"),
                Some(s) => vec![s],
                None => vec![Side::Left, Side::Right],
            };
            for side in sides {
                let arm = g
                    .components
                    .iter()
                    .find(|c| c.kind == Kind::Arm && c.side == side)
                    .with_context(|| format!("no {side:?} arm to add fingers to"))?;
                let wrist = arm.position("wrist").unwrap();
                let tip = arm.position("hand_tip").unwrap();
                let hand = format!("hand{}", side.suffix());
                ensure!(sk.bone(&hand).is_some(), "no bone named {hand}");
                let len = wrist.distance(tip);
                let dir = (tip - wrist).normalize();
                // Across the palm, pointing to the front of the character (thumb side).
                let mut across = dir.cross(Vec3::Y).try_normalize().unwrap_or(Vec3::Z);
                if across.z < 0.0 {
                    across = -across;
                }
                let fingers: [(&str, f32, f32, f32); 5] = [
                    // name, distance along the hand, offset across, length (fractions of hand length)
                    ("thumb", 0.12, 0.30, 0.42),
                    ("index", 0.48, 0.16, 0.50),
                    ("middle", 0.48, 0.04, 0.52),
                    ("ring", 0.46, -0.08, 0.49),
                    ("pinky", 0.43, -0.19, 0.40),
                ];
                for (name, along, off, flen) in fingers {
                    let id = format!("{name}{}", side.suffix());
                    ensure!(g.component(&id).is_none(), "{id} already exists");
                    let base = wrist + dir * along * len + across * off * len;
                    let fdir = if name == "thumb" {
                        (dir + across).normalize()
                    } else {
                        dir
                    };
                    let mut c = Component::new(&id, Kind::Chain, side, Some(&hand));
                    for k in 0..4 {
                        let p = base + fdir * flen * len * (k as f32 / 3.0);
                        c = c.guide(format!("j{}", k + 1), p);
                    }
                    g.components.push(c);
                    added.push(id);
                }
            }
        }
        Preset::Toes => {
            let sides = match opts.side {
                Some(Side::Center) => bail!("toes need --side left or right"),
                Some(s) => vec![s],
                None => vec![Side::Left, Side::Right],
            };
            for side in sides {
                let leg = g
                    .components
                    .iter()
                    .find(|c| c.kind == Kind::Leg && c.side == side)
                    .with_context(|| format!("no {side:?} leg to add toes to"))?;
                let ball_at = leg.position("ball").unwrap();
                let toe_tip = leg.position("toe").unwrap();
                let ball = format!("ball{}", side.suffix());
                ensure!(sk.bone(&ball).is_some(), "no bone named {ball}");
                let len = ball_at.distance(toe_tip);
                let dir = (toe_tip - ball_at).normalize();
                // Across the foot, pointing towards the big toe (the inside).
                let mut across = dir.cross(Vec3::Y).try_normalize().unwrap_or(Vec3::X);
                if (across.x < 0.0) == (side == Side::Left) {
                    across = -across;
                }
                let toes: [(&str, f32, f32, f32); 5] = [
                    // name, offset across, length (fractions of the foot), joint spacing
                    ("big", 0.36, 0.45, 0.30),
                    ("second", 0.18, 0.50, 0.30),
                    ("middle", 0.0, 0.52, 0.30),
                    ("fourth", -0.16, 0.48, 0.30),
                    ("little", -0.32, 0.40, 0.30),
                ];
                for (name, off, flen, _) in toes {
                    let id = format!("toe_{name}{}", side.suffix());
                    ensure!(g.component(&id).is_none(), "{id} already exists");
                    let base = ball_at + across * off * len * 0.5;
                    let mut c = Component::new(&id, Kind::Chain, side, Some(&ball));
                    for k in 0..4 {
                        let p = base + dir * flen * len * (k as f32 / 3.0);
                        c = c.guide(format!("j{}", k + 1), p);
                    }
                    g.components.push(c);
                    added.push(id);
                }
            }
        }
        Preset::Eyes | Preset::Jaw => {
            let kind = if preset == Preset::Eyes {
                Kind::Eyes
            } else {
                Kind::Jaw
            };
            ensure!(
                !g.components.iter().any(|c| c.kind == kind),
                "the rig already has {}",
                if kind == Kind::Eyes { "eyes" } else { "a jaw" }
            );
            let head = g
                .components
                .iter()
                .find(|c| c.kind == Kind::Head)
                .context("eyes and jaws go on a head component")?;
            let (base, top) = (
                head.position("head").unwrap(),
                head.position("head_top").unwrap(),
            );
            let parent = opts.parent.clone().unwrap_or_else(|| "head".into());
            ensure!(sk.bone(&parent).is_some(), "no bone named {parent}");
            let up = (top - base).try_normalize().unwrap_or(Vec3::Y);
            let id = opts
                .id
                .clone()
                .unwrap_or_else(|| (if kind == Kind::Eyes { "eyes" } else { "jaw" }).to_string());
            ensure!(g.component(&id).is_none(), "{id} already exists");
            let mut c = Component::new(&id, kind, Side::Center, Some(&parent));
            if kind == Kind::Eyes {
                let level = base + (top - base) * 0.45 + Vec3::Z * (0.04 * h);
                for (name, sx) in [("eye_l", 1.0), ("eye_r", -1.0)] {
                    c = c.guide(name, level + Vec3::X * (0.0175 * h * sx));
                }
                c = c.guide("target", level + Vec3::Z * (0.35 * h));
            } else {
                c = c
                    .guide("jaw", base + up * (0.02 * h) + Vec3::Z * (0.01 * h))
                    .guide("chin", base - up * (0.025 * h) + Vec3::Z * (0.06 * h));
            }
            g.components.push(c);
            added.push(id);
        }
        Preset::Face => {
            ensure!(
                !g.components.iter().any(|c| c.kind == Kind::Face),
                "the rig already has a face"
            );
            let head = g
                .components
                .iter()
                .find(|c| c.kind == Kind::Head)
                .context("the face goes on a head component")?;
            let (base, top) = (
                head.position("head").unwrap(),
                head.position("head_top").unwrap(),
            );
            let parent = opts.parent.clone().unwrap_or_else(|| "head".into());
            ensure!(sk.bone(&parent).is_some(), "no bone named {parent}");
            let id = opts.id.clone().unwrap_or_else(|| "face".into());
            ensure!(g.component(&id).is_none(), "{id} already exists");
            // Lay the face out around the eyes, using the eye guides when there are some.
            let level = base + (top - base) * 0.45 + Vec3::Z * (0.04 * h);
            let eyes = g.components.iter().find(|c| c.kind == Kind::Eyes);
            let eye = |sx: f32| {
                let name = if sx > 0.0 { "eye_l" } else { "eye_r" };
                eyes.and_then(|c| c.position(name))
                    .unwrap_or(level + Vec3::X * (0.0175 * h * sx))
            };
            let v = |x: f32, y: f32, z: f32| Vec3::new(x, y, z) * h;
            let mouth = level + v(0.0, -0.045, 0.012);
            let mut c = Component::new(&id, Kind::Face, Side::Center, Some(&parent));
            for (sx, s) in [(1.0f32, "l"), (-1.0, "r")] {
                let brow = level + v(0.0, 0.018, 0.006);
                for (part, x) in [("in", 0.008), ("mid", 0.018), ("out", 0.028)] {
                    c = c.guide(
                        format!("brow_{part}_{s}"),
                        brow + v(x * sx, 0.0, -0.002 * x / 0.008),
                    );
                }
            }
            for (sx, s) in [(1.0f32, "l"), (-1.0, "r")] {
                c = c
                    .guide(format!("lid_up_{s}"), eye(sx) + v(0.0, 0.007, 0.008))
                    .guide(format!("lid_low_{s}"), eye(sx) + v(0.0, -0.007, 0.007));
            }
            for (sx, s) in [(1.0f32, "l"), (-1.0, "r")] {
                c = c.guide(
                    format!("cheek_{s}"),
                    eye(sx) + v(0.007 * sx, -0.022, -0.002),
                );
            }
            c = c
                .guide("nose", level + v(0.0, -0.02, 0.022))
                .guide("lip_up", mouth + v(0.0, 0.004, 0.0))
                .guide("lip_low", mouth + v(0.0, -0.004, 0.0))
                .guide("lip_corner_l", mouth + v(0.014, 0.0, -0.004))
                .guide("lip_corner_r", mouth + v(-0.014, 0.0, -0.004));
            g.components.push(c);
            added.push(id);
        }
        Preset::Tail | Preset::Chain => {
            let joints = opts
                .joints
                .unwrap_or(if preset == Preset::Tail { 5 } else { 3 });
            ensure!(
                (1..=64).contains(&joints),
                "--joints must be between 1 and 64"
            );
            let (parent, start, dir, length) = if preset == Preset::Tail {
                let parent = opts.parent.clone().unwrap_or_else(|| "pelvis".into());
                let p = sk
                    .bone(&parent)
                    .with_context(|| format!("no bone named {parent}"))?;
                // Behind the body (the character faces +Z), sloping down.
                let dir = Vec3::new(0.0, -0.35, -1.0).normalize();
                (parent, p.head - Vec3::Z * 0.06 * h, dir, 0.45 * h)
            } else {
                let parent = opts
                    .parent
                    .clone()
                    .context("a chain needs --parent <bone>")?;
                let p = sk
                    .bone(&parent)
                    .with_context(|| format!("no bone named {parent}"))?;
                (parent, p.tail, p.direction(), p.length().max(0.05 * h))
            };
            let side = sk.bone(&parent).map(|b| b.side).unwrap_or_default();
            let default_id = if preset == Preset::Tail {
                "tail"
            } else {
                "chain"
            };
            let id = match &opts.id {
                Some(id) => id.clone(),
                None => unique_id(g, default_id, side),
            };
            ensure!(g.component(&id).is_none(), "{id} already exists");
            let mut c = Component::new(&id, Kind::Chain, side, Some(&parent));
            for k in 0..=joints {
                c = c.guide(
                    format!("j{}", k + 1),
                    start + dir * length * (k as f32 / joints as f32),
                );
            }
            g.components.push(c);
            added.push(id);
        }
    }
    build(g).context("the new component doesn't build")?;
    Ok(added)
}

fn unique_id(g: &Guides, base: &str, side: Side) -> String {
    let s = side.suffix();
    let first = format!("{base}{s}");
    if g.component(&first).is_none() {
        return first;
    }
    (2..)
        .map(|n| format!("{base}{n}{s}"))
        .find(|id| g.component(id).is_none())
        .unwrap()
}

/// Remove a component and every component hanging from it. Returns the removed ids.
pub fn remove(g: &mut Guides, id: &str) -> Result<Vec<String>> {
    ensure!(g.component(id).is_some(), "no component named {id}");
    ensure!(
        g.component(id).unwrap().kind != Kind::Root,
        "the root component can't be removed"
    );
    let built = build(g)?;
    let owner: HashMap<&str, &str> = built
        .skeleton
        .bones
        .iter()
        .map(|b| (b.name.as_str(), b.component.as_str()))
        .collect();
    let mut removed: HashSet<String> = HashSet::from([id.to_string()]);
    // Components are ordered parents-first, so one pass catches every descendant.
    for c in &g.components {
        if let Some(p) = &c.parent
            && owner.get(p.as_str()).is_some_and(|o| removed.contains(*o))
        {
            removed.insert(c.id.clone());
        }
    }
    let ids: Vec<String> = g
        .components
        .iter()
        .filter(|c| removed.contains(&c.id))
        .map(|c| c.id.clone())
        .collect();
    g.components.retain(|c| !removed.contains(&c.id));
    build(g).context("the layout doesn't build without that component")?;
    Ok(ids)
}

/// Hang a component from another bone. The caller checks that the layout still builds.
pub fn attach(g: &mut Guides, id: &str, bone: &str) -> Result<()> {
    let c = g
        .components
        .iter_mut()
        .find(|c| c.id == id)
        .with_context(|| format!("no component named {id}"))?;
    ensure!(c.kind != Kind::Root, "the root component stays where it is");
    let owner_of: HashMap<String, String> = build(g)?
        .skeleton
        .bones
        .into_iter()
        .map(|b| (b.name, b.component))
        .collect();
    let owner = owner_of
        .get(bone)
        .with_context(|| format!("there is no bone called {bone}"))?;
    ensure!(owner != id, "{id} can't hang from one of its own bones");
    let c = g.components.iter_mut().find(|c| c.id == id).unwrap();
    c.parent = Some(bone.to_string());
    // Components build in order, so each must follow the one owning its parent
    // bone. Keep the existing order wherever that already holds.
    let mut order = Vec::with_capacity(g.components.len());
    let mut state: HashMap<&str, bool> = HashMap::new(); // false = visiting
    fn visit<'a>(
        g: &'a Guides,
        owner_of: &HashMap<String, String>,
        i: usize,
        state: &mut HashMap<&'a str, bool>,
        order: &mut Vec<usize>,
    ) -> Result<()> {
        let c = &g.components[i];
        match state.get(c.id.as_str()) {
            Some(true) => return Ok(()),
            Some(false) => bail!("{} would end up hanging from its own child", c.id),
            None => {}
        }
        state.insert(&c.id, false);
        if let Some(owner) = c.parent.as_ref().and_then(|p| owner_of.get(p))
            && let Some(j) = g.components.iter().position(|o| &o.id == owner)
        {
            visit(g, owner_of, j, state, order)?;
        }
        state.insert(&c.id, true);
        order.push(i);
        Ok(())
    }
    for i in 0..g.components.len() {
        visit(g, &owner_of, i, &mut state, &mut order)?;
    }
    let mut slots: Vec<Option<Component>> = g.components.drain(..).map(Some).collect();
    g.components = order
        .into_iter()
        .map(|i| slots[i].take().unwrap())
        .collect();
    build(g).map(|_| ())
}

/// Set one control's look, or clear it when the style is empty.
pub fn set_control_style(g: &mut Guides, control: &str, style: ControlStyle) -> Result<()> {
    if style == ControlStyle::default() {
        g.control_styles.remove(control);
    } else {
        if let Some(size) = style.size {
            ensure!(size > 0.0, "control size must be positive");
        }
        g.control_styles.insert(control.to_string(), style);
    }
    Ok(())
}

/// Bring the components of another layout in, hung from `bone`. Only premade
/// parts (chains, eyes, jaw, face) can be imported: the body's own components
/// are fixed to the mannequin's bones. Components keep their ids, so an id
/// already in use is refused rather than renamed, because bone names depend on it.
pub fn import(g: &mut Guides, src: &Guides, bone: &str) -> Result<Vec<String>> {
    let target = build(g)?;
    ensure!(
        target.skeleton.bone(bone).is_some(),
        "no bone named {bone} to attach to"
    );
    let source = build(src).context("the file to import doesn't build")?;
    let owner: HashMap<&str, &str> = source
        .skeleton
        .bones
        .iter()
        .map(|b| (b.name.as_str(), b.component.as_str()))
        .collect();
    let imported: Vec<&Component> = src
        .components
        .iter()
        .filter(|c| matches!(c.kind, Kind::Chain | Kind::Eyes | Kind::Jaw | Kind::Face))
        .collect();
    let ids: HashSet<&str> = imported.iter().map(|c| c.id.as_str()).collect();
    for c in &imported {
        ensure!(
            g.component(&c.id).is_none(),
            "the rig already has a component named {}; rename it or remove it first",
            c.id
        );
        if matches!(c.kind, Kind::Eyes | Kind::Jaw | Kind::Face) {
            ensure!(
                !g.components.iter().any(|x| x.kind == c.kind),
                "the rig already has one {:?} component",
                c.kind
            );
        }
    }
    let mut added = Vec::new();
    for c in imported {
        let mut c = c.clone();
        // Parts that hang from another imported part keep that link; the rest hang from `bone`.
        let inside = c
            .parent
            .as_deref()
            .and_then(|p| owner.get(p))
            .is_some_and(|o| ids.contains(o));
        if !inside {
            c.parent = Some(bone.to_string());
        }
        added.push(c.id.clone());
        g.components.push(c);
    }
    build(g).context("the imported parts don't fit this rig")?;
    Ok(added)
}

/// The other side's twin of a sided component (`arm_l` <-> `arm_r`), if any.
pub fn twin_id(c: &Component) -> Option<String> {
    let (from, to) = match c.side {
        Side::Left => ("_l", "_r"),
        Side::Right => ("_r", "_l"),
        Side::Center => return None,
    };
    c.id.strip_suffix(from).map(|base| format!("{base}{to}"))
}

/// `p` reflected across the body's centre line.
pub fn mirror_point(g: &Guides, p: Vec3) -> Vec3 {
    mirror(p, g.body.center_x)
}

/// Copy every `source`-side component's guides onto its twin, mirrored across
/// the body. Returns how many components were updated.
pub fn mirror_side(g: &mut Guides, source: Side) -> usize {
    let mut updates = Vec::new();
    for (i, c) in g.components.iter().enumerate() {
        if c.side != source {
            continue;
        }
        let Some(twin) = twin_id(c) else { continue };
        if let Some(j) = g.components.iter().position(|t| t.id == twin)
            && g.components[j].guides.len() == c.guides.len()
        {
            updates.push((i, j));
        }
    }
    let cx = g.body.center_x;
    // Eyes and the face keep both sides in one component.
    let suffix = match source {
        Side::Left => "_l",
        Side::Right => "_r",
        Side::Center => "",
    };
    let mut paired = 0;
    for c in g
        .components
        .iter_mut()
        .filter(|c| matches!(c.kind, Kind::Eyes | Kind::Face))
    {
        let moves: Vec<(usize, Vec3)> = c
            .guides
            .iter()
            .filter(|x| !suffix.is_empty() && x.name.ends_with(suffix))
            .filter_map(|x| {
                let twin = paired_guide(&x.name)?;
                let j = c.guides.iter().position(|y| y.name == twin)?;
                Some((j, mirror(x.position, cx)))
            })
            .collect();
        if !moves.is_empty() {
            paired += 1;
        }
        for (j, p) in moves {
            c.guides[j].position = p;
        }
    }
    for &(i, j) in &updates {
        let points: Vec<Vec3> = g.components[i].guides.iter().map(|x| x.position).collect();
        for (guide, p) in g.components[j].guides.iter_mut().zip(points) {
            guide.position = mirror(p, cx);
        }
    }
    updates.len() + paired
}

/// Change the spine component's joint count, re-attaching components that hung
/// from the old top joint to the new one.
pub fn set_spine_joints(g: &mut Guides, spine_id: &str, joints: usize) -> Result<()> {
    ensure!((1..=16).contains(&joints), "spine joints must be 1 to 16");
    let spine = g
        .components
        .iter_mut()
        .find(|c| c.id == spine_id && c.kind == Kind::Spine)
        .with_context(|| format!("no spine component named {spine_id}"))?;
    let old_top = spine_name(spine.options.joints.unwrap_or(SPINE_JOINTS) - 1);
    spine.options.joints = Some(joints);
    let new_top = spine_name(joints - 1);
    for c in &mut g.components {
        if c.parent.as_deref() == Some(old_top.as_str()) {
            c.parent = Some(new_top.clone());
        }
    }
    build(g).map(|_| ())
}

// --------------------------------------------------------------------------- building

pub struct Built {
    pub skeleton: Skeleton,
    pub controls: ControlRig,
}

const CENTER: [f32; 3] = [1.0, 0.85, 0.1];
const LEFT: [f32; 3] = [0.1, 0.45, 1.0];
const RIGHT: [f32; 3] = [1.0, 0.15, 0.15];
const ROOT: [f32; 3] = [0.85, 0.3, 0.9];

fn side_color(side: Side) -> [f32; 3] {
    match side {
        Side::Center => CENTER,
        Side::Left => LEFT,
        Side::Right => RIGHT,
    }
}

struct Builder {
    h: f32,
    sk: Skeleton,
    rig: ControlRig,
    /// Bone -> the FK control that drives it.
    ctrl_of: HashMap<String, String>,
    /// Bones driven by an IK/FK blend, whose controls don't follow them in IK.
    blended: HashSet<String>,
    /// Arm IK chain -> the control of the bone its clavicle hangs from (the chest).
    arm_chest: HashMap<String, String>,
    head_chest: Option<String>,
    component: String,
}

impl Builder {
    fn bone(
        &mut self,
        name: &str,
        parent: &str,
        head: Vec3,
        tail: Vec3,
        side: Side,
        region: Region,
    ) -> Result<usize> {
        self.check_bone(name, head, tail)?;
        ensure!(
            self.sk.bone(parent).is_some(),
            "component {}: parent bone '{parent}' doesn't exist (components must come after the component that owns their parent)",
            self.component
        );
        let i = self.sk.add(name, Some(parent), head, tail, side, region);
        self.sk.bones[i].component = self.component.clone();
        Ok(i)
    }

    fn check_bone(&self, name: &str, head: Vec3, tail: Vec3) -> Result<()> {
        ensure!(
            self.sk.bone(name).is_none(),
            "component {}: a bone named '{name}' already exists",
            self.component
        );
        ensure!(
            head.distance(tail) > 1e-4 * self.h,
            "component {}: bone '{name}' has no length (two guides are in the same place)",
            self.component
        );
        Ok(())
    }

    fn link(&mut self, bone: &str, region: Region) {
        if let Some(i) = self.sk.index_of(bone)
            && !self.sk.bones[i].links.contains(&region)
        {
            self.sk.bones[i].links.push(region);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn control(
        &mut self,
        name: String,
        kind: ControlKind,
        parent: ControlParent,
        bone: Option<&str>,
        position: Vec3,
        axis: Vec3,
        shape: Shape,
        size: f32,
        color: [f32; 3],
    ) {
        if let (ControlKind::Fk | ControlKind::Root, Some(b)) = (kind, bone) {
            self.ctrl_of.insert(b.to_string(), name.clone());
        }
        self.rig.controls.push(Control {
            name,
            kind,
            parent,
            bone: bone.map(str::to_string),
            position,
            axis,
            shape,
            size,
            color,
            spaces: Vec::new(),
        });
    }

    /// The control a component attached to `bone` should hang from.
    fn parent_control(&self, bone: &str) -> ControlParent {
        match self.ctrl_of.get(bone) {
            Some(c) if !self.blended.contains(bone) => ControlParent::Control(c.clone()),
            _ => ControlParent::Bone(bone.to_string()),
        }
    }

    fn has_control(&self, name: &str) -> bool {
        self.rig.control(name).is_some()
    }
}

/// Turn guides into a skeleton and control rig.
pub fn build(g: &Guides) -> Result<Built> {
    validate(g)?;
    let mut b = Builder {
        h: g.body.height,
        sk: Skeleton::default(),
        rig: ControlRig::default(),
        ctrl_of: HashMap::new(),
        blended: HashSet::new(),
        arm_chest: HashMap::new(),
        head_chest: None,
        component: String::new(),
    };
    for c in &g.components {
        b.component = c.id.clone();
        let p = |name: &str| c.position(name).unwrap();
        let parent = c.parent.as_deref().unwrap_or("");
        match c.kind {
            Kind::Root => build_root(&mut b, p("root"))?,
            Kind::Spine => build_spine(&mut b, c, parent)?,
            Kind::Head => build_head(&mut b, parent, p("neck"), p("head"), p("head_top"))?,
            Kind::Arm => build_arm(&mut b, c, parent)?,
            Kind::Leg => build_leg(&mut b, c, parent)?,
            Kind::Chain => build_chain(&mut b, c, parent)?,
            Kind::Eyes => build_eyes(&mut b, c, parent)?,
            Kind::Jaw => build_jaw(&mut b, p("jaw"), p("chin"), parent)?,
            Kind::Face => build_face(&mut b, c, parent)?,
        }
    }
    assign_spaces(&mut b);
    b.rig.find_mirror_pairs();
    for (name, style) in &g.control_styles {
        let Some(c) = b.rig.controls.iter_mut().find(|c| &c.name == name) else {
            continue;
        };
        if let Some(shape) = style.shape {
            c.shape = shape;
        }
        if let Some(size) = style.size {
            c.size = size.max(0.001);
        }
        if let Some(color) = style.color {
            c.color = color;
        }
    }
    Ok(Built {
        skeleton: b.sk,
        controls: b.rig,
    })
}

fn validate(g: &Guides) -> Result<()> {
    ensure!(
        g.body.height > 0.0,
        "body height must be positive (got {})",
        g.body.height
    );
    ensure!(
        g.components.first().map(|c| c.kind) == Some(Kind::Root),
        "the first component must be the root"
    );
    let mut ids = HashSet::new();
    for c in &g.components {
        ensure!(
            !c.id.is_empty()
                && c.id
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || ch == '_'),
            "component id '{}' must be letters, digits and underscores",
            c.id
        );
        ensure!(ids.insert(&c.id), "two components are called {}", c.id);
        ensure!(
            (c.kind == Kind::Root) == c.parent.is_none(),
            "component {}: every component except the root needs a parent bone",
            c.id
        );
        match c.kind.guide_names() {
            Some(names) => {
                let have: Vec<&str> = c.guides.iter().map(|g| g.name.as_str()).collect();
                ensure!(
                    have == names,
                    "component {} ({:?}) needs guides {names:?}, found {have:?}",
                    c.id,
                    c.kind
                );
            }
            None => ensure!(
                c.guides.len() >= 2,
                "component {}: a chain needs at least two guides",
                c.id
            ),
        }
        if matches!(c.kind, Kind::Arm | Kind::Leg) {
            ensure!(
                c.side != Side::Center,
                "component {}: arms and legs need a side (left or right)",
                c.id
            );
        }
        if c.kind == Kind::Spine {
            let n = c.options.joints.unwrap_or(SPINE_JOINTS);
            ensure!(
                (1..=16).contains(&n),
                "component {}: spine joints must be 1 to 16",
                c.id
            );
        }
        for guide in &c.guides {
            ensure!(
                guide.position.is_finite(),
                "component {}: guide {} isn't a finite position",
                c.id,
                guide.name
            );
        }
    }
    ensure!(
        g.components.iter().filter(|c| c.kind == Kind::Root).count() == 1,
        "there must be exactly one root component"
    );
    Ok(())
}

fn build_root(b: &mut Builder, at: Vec3) -> Result<()> {
    let tail = at + Vec3::Z * 0.15 * b.h;
    b.check_bone("root", at, tail)?;
    let i =
        b.sk.add("root", None, at, tail, Side::Center, Region::Center);
    b.sk.bones[i].deform = false;
    b.sk.bones[i].component = b.component.clone();
    b.control(
        ctrl_name("root"),
        ControlKind::Root,
        ControlParent::None,
        Some("root"),
        at,
        Vec3::Y,
        Shape::Circle,
        0.25 * b.h,
        ROOT,
    );
    Ok(())
}

fn build_spine(b: &mut Builder, c: &Component, parent: &str) -> Result<()> {
    let h = b.h;
    let pelvis = c.position("pelvis").unwrap();
    let chest = c.position("chest").unwrap();
    let joints = c.options.joints.unwrap_or(SPINE_JOINTS);
    // The pelvis points up into the first spine joint.
    let start = pelvis + Vec3::Y * 0.04 * h;
    b.bone(
        "pelvis",
        parent,
        pelvis,
        start,
        Side::Center,
        Region::Center,
    )?;
    let mut prev = "pelvis".to_string();
    for i in 0..joints {
        let name = spine_name(i);
        let t0 = i as f32 / joints as f32;
        let t1 = (i + 1) as f32 / joints as f32;
        b.bone(
            &name,
            &prev,
            start.lerp(chest, t0),
            start.lerp(chest, t1),
            Side::Center,
            Region::Center,
        )?;
        prev = name;
    }

    let cog_parent = b.parent_control(parent);
    b.control(
        "cog_ctrl".into(),
        ControlKind::Cog,
        cog_parent,
        None,
        pelvis,
        Vec3::Y,
        Shape::Square,
        0.17 * h,
        CENTER,
    );
    b.control(
        ctrl_name("pelvis"),
        ControlKind::Fk,
        ControlParent::Control("cog_ctrl".into()),
        Some("pelvis"),
        pelvis,
        Vec3::Y,
        Shape::Circle,
        0.13 * h,
        CENTER,
    );
    let mut parent_ctrl = "cog_ctrl".to_string();
    for i in 0..joints {
        let n = spine_name(i);
        let bone = b.sk.bone(&n).unwrap().clone();
        b.control(
            ctrl_name(&n),
            ControlKind::Fk,
            ControlParent::Control(parent_ctrl),
            Some(&n),
            bone.head,
            bone.direction(),
            Shape::Circle,
            0.12 * h,
            CENTER,
        );
        parent_ctrl = ctrl_name(&n);
    }
    Ok(())
}

fn build_head(b: &mut Builder, parent: &str, neck: Vec3, head: Vec3, top: Vec3) -> Result<()> {
    b.bone("neck_01", parent, neck, head, Side::Center, Region::Center)?;
    b.bone("head", "neck_01", head, top, Side::Center, Region::Center)?;
    let mut parent_ctrl = b.parent_control(parent);
    if let ControlParent::Control(c) = &parent_ctrl {
        b.head_chest = Some(c.clone());
    }
    for (n, size) in [("neck_01", 0.06), ("head", 0.08)] {
        let bone = b.sk.bone(n).unwrap().clone();
        b.control(
            ctrl_name(n),
            ControlKind::Fk,
            parent_ctrl,
            Some(n),
            bone.head,
            bone.direction(),
            Shape::Circle,
            size * b.h,
            CENTER,
        );
        parent_ctrl = ControlParent::Control(ctrl_name(n));
    }
    Ok(())
}

fn limb_region(kind: Kind, side: Side) -> Region {
    match (kind, side) {
        (Kind::Arm, Side::Left) => Region::ArmL,
        (Kind::Arm, _) => Region::ArmR,
        (_, Side::Left) => Region::LegL,
        _ => Region::LegR,
    }
}

fn build_arm(b: &mut Builder, c: &Component, parent: &str) -> Result<()> {
    let side = c.side;
    let s = side.suffix();
    let region = limb_region(Kind::Arm, side);
    let p = |n: &str| c.position(n).unwrap();
    let clav = format!("clavicle{s}");
    b.bone(
        &clav,
        parent,
        p("clavicle"),
        p("shoulder"),
        side,
        Region::Center,
    )?;
    b.link(&clav, region);
    b.link(parent, region);
    let names = [
        format!("upperarm{s}"),
        format!("lowerarm{s}"),
        format!("hand{s}"),
    ];
    let points = [p("shoulder"), p("elbow"), p("wrist"), p("hand_tip")];
    limb_bones(b, &names, &clav, &points, side, region, c.options.twist)?;

    let color = side_color(side);
    let clav_parent = b.parent_control(parent);
    if let ControlParent::Control(chest) = &clav_parent {
        b.arm_chest.insert(format!("arm{s}"), chest.clone());
    }
    let cb = b.sk.bone(&clav).unwrap().clone();
    b.control(
        ctrl_name(&clav),
        ControlKind::Fk,
        clav_parent,
        Some(&clav),
        cb.tail,
        cb.direction(),
        Shape::Diamond,
        0.035 * b.h,
        color,
    );
    limb_controls(
        b,
        "arm",
        side,
        &names,
        ControlParent::Control(ctrl_name(&clav)),
        0.0,
        None,
    );
    Ok(())
}

fn build_leg(b: &mut Builder, c: &Component, parent: &str) -> Result<()> {
    let side = c.side;
    let s = side.suffix();
    let region = limb_region(Kind::Leg, side);
    let p = |n: &str| c.position(n).unwrap();
    b.link(parent, region);
    let names = [format!("thigh{s}"), format!("calf{s}"), format!("foot{s}")];
    let points = [p("hip"), p("knee"), p("ankle"), p("ball")];
    limb_bones(b, &names, parent, &points, side, region, c.options.twist)?;
    let ball = format!("ball{s}");
    b.bone(&ball, &names[2], p("ball"), p("toe"), side, region)?;

    let fk_parent = b.parent_control(parent);
    let foot = FootRoll {
        heel: p("heel"),
        ball: p("ball"),
        toe_tip: p("toe"),
        ball_bone: ball.clone(),
        ball_control: ctrl_name(&ball),
        break_angle: 30.0,
    };
    limb_controls(b, "leg", side, &names, fk_parent, 1.0, Some(foot));

    // Toes follow the foot in both IK and FK.
    let bb = b.sk.bone(&ball).unwrap().clone();
    b.control(
        ctrl_name(&ball),
        ControlKind::Fk,
        ControlParent::Bone(names[2].clone()),
        Some(&ball),
        bb.head,
        bb.direction(),
        Shape::Circle,
        0.035 * b.h,
        side_color(side),
    );
    b.blended.insert(ball);
    Ok(())
}

/// Upper, lower and end bones through `points`, plus twist helpers on the far half
/// of the upper and lower segments (mannequin names). Upper twists counter half of
/// their own twist so the shoulder and hip don't candy-wrap; lower twists carry
/// half of the hand or foot twist.
fn limb_bones(
    b: &mut Builder,
    names: &[String; 3],
    parent: &str,
    points: &[Vec3; 4],
    side: Side,
    region: Region,
    twist: bool,
) -> Result<()> {
    let mut prev = parent.to_string();
    for (i, name) in names.iter().enumerate() {
        b.bone(name, &prev, points[i], points[i + 1], side, region)?;
        prev = name.clone();
    }
    // The upper bone also blends into the torso.
    let upper = b.sk.index_of(&names[0]).unwrap();
    b.sk.bones[upper].links.push(Region::Center);
    if twist {
        for (seg, factor, source) in [(&names[0], -0.5, &names[0]), (&names[1], 0.5, &names[2])] {
            let bone = b.sk.bone(seg).unwrap().clone();
            let i = b.bone(
                &twist_name(seg),
                seg,
                bone.head.lerp(bone.tail, 0.5),
                bone.tail,
                side,
                region,
            )?;
            b.sk.bones[i].twist = Some(Twist {
                source: source.clone(),
                factor,
            });
        }
    }
    Ok(())
}

fn limb_controls(
    b: &mut Builder,
    kind: &str,
    side: Side,
    names: &[String; 3],
    fk_parent: ControlParent,
    blend: f32,
    foot: Option<FootRoll>,
) {
    let h = b.h;
    let color = side_color(side);
    let s = side.suffix();
    let [up, lo, end] = names.clone().map(|n| b.sk.bone(&n).unwrap().clone());
    let (size, bend_fallback) = if kind == "arm" {
        (0.045 * h, -Vec3::Z)
    } else {
        (0.06 * h, Vec3::Z)
    };

    let mut parent = fk_parent;
    for bone in [&up, &lo, &end] {
        b.control(
            fk_name(&bone.name),
            ControlKind::Fk,
            parent,
            Some(&bone.name),
            bone.head,
            bone.direction(),
            Shape::Circle,
            size,
            color,
        );
        parent = ControlParent::Control(fk_name(&bone.name));
        b.blended.insert(bone.name.clone());
    }

    let ik = format!("{}_ik_ctrl", end.name);
    let ik_axis = if kind == "arm" {
        up.direction()
    } else {
        Vec3::Y
    };
    let root = if b.has_control("root_ctrl") {
        ControlParent::Control("root_ctrl".into())
    } else {
        ControlParent::None
    };
    b.control(
        ik.clone(),
        ControlKind::IkEffector,
        root.clone(),
        Some(&end.name),
        end.head,
        ik_axis,
        Shape::Cube,
        size,
        color,
    );

    // The pole sits straight ahead of the knee (behind the elbow): forward or
    // back, square to the limb, at about half the limb's length. A sideways lean
    // in the guides doesn't swing it out to the side.
    let axis = (end.head - up.head).try_normalize().unwrap_or(Vec3::NEG_Y);
    let bend = (bend_fallback - axis * bend_fallback.dot(axis))
        .try_normalize()
        .unwrap_or(bend_fallback);
    let reach = 0.5 * (up.head.distance(lo.head) + lo.head.distance(end.head));
    let pole = format!("{}_pole_ctrl", lo.name);
    b.control(
        pole.clone(),
        ControlKind::Pole,
        root,
        None,
        lo.head + bend * reach.max(0.1 * h),
        bend,
        Shape::Diamond,
        0.025 * h,
        color,
    );

    b.rig.ik_chains.push(IkChain {
        name: format!("{kind}{s}"),
        upper: up.name.clone(),
        lower: lo.name.clone(),
        end: end.name.clone(),
        ik_control: ik,
        pole_control: pole,
        fk_controls: [fk_name(&up.name), fk_name(&lo.name), fk_name(&end.name)],
        default_blend: blend,
        default_stretch: 0.0,
        foot,
    });
}

fn build_chain(b: &mut Builder, c: &Component, parent: &str) -> Result<()> {
    let region =
        b.sk.bone(parent)
            .map(|p| p.region)
            .unwrap_or(Region::Center);
    let names: Vec<String> = (1..c.guides.len())
        .map(|i| chain_bone_name(&c.id, c.side, i))
        .collect();
    let mut prev = parent.to_string();
    for (i, name) in names.iter().enumerate() {
        b.bone(
            name,
            &prev,
            c.guides[i].position,
            c.guides[i + 1].position,
            c.side,
            region,
        )?;
        prev = name.clone();
    }
    let mut parent_ctrl = b.parent_control(parent);
    for name in &names {
        let bone = b.sk.bone(name).unwrap().clone();
        let size = (bone.length() * 0.6).clamp(0.008 * b.h, 0.06 * b.h);
        b.control(
            ctrl_name(name),
            ControlKind::Fk,
            parent_ctrl,
            Some(name),
            bone.head,
            bone.direction(),
            Shape::Circle,
            size,
            side_color(c.side),
        );
        parent_ctrl = ControlParent::Control(ctrl_name(name));
    }
    Ok(())
}

/// One bone per eye, pointing at the target, aimed by a control in front of it;
/// both aim controls hang from a master that moves the gaze as a whole.
fn build_eyes(b: &mut Builder, c: &Component, parent: &str) -> Result<()> {
    let p = |n: &str| c.position(n).unwrap();
    let (l, r, target) = (p("eye_l"), p("eye_r"), p("target"));
    let mid = (l + r) * 0.5;
    let gaze = target - mid;
    ensure!(
        gaze.length() > 0.02 * b.h,
        "component {}: the eye target must be in front of the eyes",
        c.id
    );
    let dir = gaze.normalize();
    let master = format!("{}_ctrl", c.id);
    let parent_ctrl = b.parent_control(parent);
    let spread = l.distance(r).max(0.01 * b.h);
    b.control(
        master.clone(),
        ControlKind::Aim,
        parent_ctrl,
        None,
        target,
        dir,
        Shape::Square,
        spread * 1.1,
        CENTER,
    );
    for (name, side, eye) in [("eye_l", Side::Left, l), ("eye_r", Side::Right, r)] {
        b.bone(
            name,
            parent,
            eye,
            eye + dir * (0.02 * b.h),
            side,
            Region::Center,
        )?;
        b.control(
            format!("{name}_aim_ctrl"),
            ControlKind::Aim,
            ControlParent::Control(master.clone()),
            Some(name),
            eye + gaze,
            dir,
            Shape::Circle,
            spread * 0.3,
            side_color(side),
        );
    }
    Ok(())
}

fn build_jaw(b: &mut Builder, jaw: Vec3, chin: Vec3, parent: &str) -> Result<()> {
    b.bone("jaw", parent, jaw, chin, Side::Center, Region::Center)?;
    let parent_ctrl = b.parent_control(parent);
    b.control(
        ctrl_name("jaw"),
        ControlKind::Fk,
        parent_ctrl,
        Some("jaw"),
        chin,
        (chin - jaw).normalize(),
        Shape::Circle,
        0.03 * b.h,
        CENTER,
    );
    Ok(())
}

/// Face joints hang from the head (the lower lip from the jaw when there is one).
/// Each is a short joint pointing out of the face with a control that moves and
/// rotates it, and only weights the skin near it. With eyes built first, the
/// eyelids pivot on the eyeball so rotating their control blinks.
fn build_face(b: &mut Builder, c: &Component, parent: &str) -> Result<()> {
    let len = 0.012 * b.h;
    for g in &c.guides {
        let name = g.name.as_str();
        let side = if name.ends_with("_l") {
            Side::Left
        } else if name.ends_with("_r") {
            Side::Right
        } else {
            Side::Center
        };
        let bone_parent = if name == "lip_low" && b.sk.bone("jaw").is_some() {
            "jaw"
        } else {
            parent
        };
        let eye = name
            .strip_prefix("lid_")
            .and_then(|_| b.sk.bone(&format!("eye{}", side.suffix())))
            .map(|e| e.head);
        let (head, tail) = match eye {
            Some(e) if e.distance(g.position) > 1e-3 * b.h => (e, g.position),
            _ => (g.position, g.position + Vec3::Z * len),
        };
        let i = b.bone(name, bone_parent, head, tail, side, Region::Center)?;
        let reach = if name.starts_with("lip") || name.starts_with("lid") {
            0.022
        } else {
            0.03
        };
        let bone = &mut b.sk.bones[i];
        bone.translate = true;
        bone.reach = Some(reach * b.h);
        let parent_ctrl = b.parent_control(bone_parent);
        b.control(
            ctrl_name(name),
            ControlKind::Fk,
            parent_ctrl,
            Some(name),
            g.position + Vec3::Z * (0.012 * b.h),
            Vec3::Z,
            Shape::Diamond,
            0.0045 * b.h,
            side_color(side),
        );
    }
    Ok(())
}

/// Space switching, as animators expect from production rigs: hands can follow the
/// world, body, chest or head; feet the world or body; poles their goal; the head
/// its neck, the chest, the body or the world.
fn assign_spaces(b: &mut Builder) {
    let optional = |b: &Builder, names: &[Option<&str>]| -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for n in names.iter().flatten() {
            if b.has_control(n) && !out.iter().any(|o| o == n) {
                out.push(n.to_string());
            }
        }
        out
    };
    let mut assignments: Vec<(String, Vec<String>)> = Vec::new();
    for chain in &b.rig.ik_chains {
        let spaces = if chain.foot.is_none() {
            let chest = b.arm_chest.get(&chain.name).map(String::as_str);
            optional(
                b,
                &[
                    Some("root_ctrl"),
                    Some("cog_ctrl"),
                    chest,
                    Some("head_ctrl"),
                ],
            )
        } else {
            optional(b, &[Some("root_ctrl"), Some("cog_ctrl")])
        };
        assignments.push((chain.ik_control.clone(), spaces));
        assignments.push((
            chain.pole_control.clone(),
            optional(b, &[Some("root_ctrl"), Some(&chain.ik_control)]),
        ));
    }
    if b.has_control("head_ctrl") && b.has_control("neck_01_ctrl") {
        let chest = b.head_chest.clone();
        assignments.push((
            "head_ctrl".into(),
            optional(
                b,
                &[
                    Some("neck_01_ctrl"),
                    chest.as_deref(),
                    Some("cog_ctrl"),
                    Some("root_ctrl"),
                ],
            ),
        ));
    }
    let eyes: Vec<(String, String)> = b
        .rig
        .controls
        .iter()
        .filter(|c| c.kind == ControlKind::Aim && c.bone.is_none())
        .filter_map(|c| match &c.parent {
            ControlParent::Control(p) => Some((c.name.clone(), p.clone())),
            _ => None,
        })
        .collect();
    for (name, parent) in eyes {
        let spaces = optional(b, &[Some(&parent), Some("cog_ctrl"), Some("root_ctrl")]);
        assignments.push((name, spaces));
    }
    for (name, spaces) in assignments {
        // The first space must be the control's own parent, and a single space is
        // no switch at all.
        let Some(ctrl) = b.rig.controls.iter_mut().find(|c| c.name == name) else {
            continue;
        };
        if spaces.len() > 1 && ctrl.parent == ControlParent::Control(spaces[0].clone()) {
            ctrl.spaces = spaces;
        }
    }
}

/// The guide that mirrors guide `index` of component `comp`, as (component, guide):
/// the same guide on the twin component, or the other eye.
/// The other side's name of a guide named `*_l` or `*_r` in a two-sided component.
fn paired_guide(name: &str) -> Option<String> {
    if let Some(base) = name.strip_suffix("_l") {
        Some(format!("{base}_r"))
    } else {
        name.strip_suffix("_r").map(|base| format!("{base}_l"))
    }
}

pub fn mirror_guide(g: &Guides, comp: usize, index: usize) -> Option<(usize, usize)> {
    let c = g.components.get(comp)?;
    if matches!(c.kind, Kind::Eyes | Kind::Face) {
        let other = paired_guide(&c.guides.get(index)?.name)?;
        return Some((comp, c.guides.iter().position(|x| x.name == other)?));
    }
    let twin = twin_id(c)?;
    let t = g.components.iter().position(|o| o.id == twin)?;
    (index < g.components[t].guides.len()).then_some((t, index))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{demo::demo_humanoid, landmarks::detect};

    #[test]
    fn face_joints() {
        let mut g = demo_guides();
        add_preset(&mut g, Preset::Eyes, &AddOptions::default()).unwrap();
        add_preset(&mut g, Preset::Jaw, &AddOptions::default()).unwrap();
        add_preset(&mut g, Preset::Face, &AddOptions::default()).unwrap();
        assert!(add_preset(&mut g, Preset::Face, &AddOptions::default()).is_err());
        let Built {
            skeleton: sk,
            controls,
        } = build(&g).unwrap();
        for name in FACE_GUIDES {
            let bone = sk.bone(name).unwrap();
            assert!(bone.translate && bone.reach.is_some(), "{name}");
            let ctrl = controls.control(&ctrl_name(name)).unwrap();
            assert_eq!(ctrl.bone.as_deref(), Some(name));
        }
        let parent_of = |n: &str| sk.bones[sk.bone(n).unwrap().parent.unwrap()].name.clone();
        assert_eq!(parent_of("lip_low"), "jaw");
        assert_eq!(parent_of("brow_in_l"), "head");
        // Eyelids pivot on the eyeball.
        assert_eq!(
            sk.bone("lid_up_l").unwrap().head,
            sk.bone("eye_l").unwrap().head
        );
        assert_eq!(sk.bone("brow_out_l").unwrap().side, Side::Left);

        // Mirroring and symmetric moves pair the two sides inside the component.
        let face = g
            .components
            .iter()
            .position(|c| c.kind == Kind::Face)
            .unwrap();
        let i = g.components[face]
            .guides
            .iter()
            .position(|x| x.name == "cheek_l")
            .unwrap();
        let (fc, j) = mirror_guide(&g, face, i).unwrap();
        assert_eq!(
            (fc, g.components[fc].guides[j].name.as_str()),
            (face, "cheek_r")
        );
        g.components[face].guides[i].position.y += 0.01;
        mirror_side(&mut g, Side::Left);
        let (l, r) = (
            g.components[face].guides[i].position,
            g.components[face].guides[j].position,
        );
        assert!((l.y - r.y).abs() < 1e-6 && (l.x + r.x - 2.0 * g.body.center_x).abs() < 1e-5);
    }

    #[test]
    fn eyes_and_jaw() {
        let mut g = demo_guides();
        add_preset(&mut g, Preset::Eyes, &AddOptions::default()).unwrap();
        add_preset(&mut g, Preset::Jaw, &AddOptions::default()).unwrap();
        assert!(add_preset(&mut g, Preset::Eyes, &AddOptions::default()).is_err());
        let Built {
            skeleton: sk,
            controls,
        } = build(&g).unwrap();
        for bone in ["eye_l", "eye_r", "jaw"] {
            assert_eq!(sk.bone(bone).unwrap().parent, sk.index_of("head"));
        }
        let master = controls.control("eyes_ctrl").unwrap();
        assert_eq!(
            (master.kind, master.bone.as_deref()),
            (ControlKind::Aim, None)
        );
        assert_eq!(master.parent, ControlParent::Control("head_ctrl".into()));
        assert!(master.spaces.contains(&"root_ctrl".to_string()));
        let aim = controls.control("eye_l_aim_ctrl").unwrap();
        assert_eq!(aim.bone.as_deref(), Some("eye_l"));
        // Each eye bone points at its own aim control: a parallel gaze.
        let eye = sk.bone("eye_l").unwrap();
        assert!(eye.direction().dot((aim.position - eye.head).normalize()) > 0.9999);
        assert_eq!(
            controls.control("jaw_ctrl").unwrap().bone.as_deref(),
            Some("jaw")
        );
        let jaw = sk.bone("jaw").unwrap();
        assert!(
            jaw.tail.y < jaw.head.y && jaw.tail.z > jaw.head.z,
            "the chin is below and in front of the hinge"
        );

        // The eyes mirror as a pair.
        let i = g
            .components
            .iter()
            .position(|c| c.kind == Kind::Eyes)
            .unwrap();
        assert_eq!(mirror_guide(&g, i, 0), Some((i, 1)));
        assert_eq!(mirror_guide(&g, i, 2), None);
        g.components[i].guides[0].position.y += 0.01;
        mirror_side(&mut g, Side::Left);
        let e = &g.components[i];
        assert!(
            (mirror(e.guides[0].position, g.body.center_x) - e.guides[1].position).length() < 1e-6
        );
    }

    fn demo_guides() -> Guides {
        biped(&detect(&demo_humanoid()))
    }

    #[test]
    fn biped_builds_a_symmetric_connected_skeleton() {
        let Built { skeleton: sk, .. } = build(&demo_guides()).unwrap();
        assert_eq!(sk.bones.len(), 1 + 1 + SPINE_JOINTS + 2 + 2 * (4 + 4 + 4));
        let t = sk.bone("lowerarm_twist_01_l").unwrap();
        assert_eq!(t.twist.as_ref().unwrap().source, "hand_l");
        assert_eq!(t.component, "arm_l");
        assert_eq!(twist_name("calf_r"), "calf_twist_01_r");
        assert_eq!(sk.bones.iter().filter(|b| b.parent.is_none()).count(), 1);
        let l = sk.bone("hand_l").unwrap();
        let r = sk.bone("hand_r").unwrap();
        assert!((l.head.x + r.head.x).abs() < 1e-4 && (l.head.y - r.head.y).abs() < 1e-5);
        for b in &sk.bones {
            assert!(b.length() > 1e-3, "{} is degenerate", b.name);
        }
        // Skin blends across the joints between regions.
        assert!(sk.bone("spine_03").unwrap().links.contains(&Region::ArmL));
        assert!(sk.bone("pelvis").unwrap().links.contains(&Region::LegR));
        assert!(sk.bone("thigh_l").unwrap().links.contains(&Region::Center));
    }

    #[test]
    fn every_control_parent_exists_and_precedes_its_children() {
        let Built {
            skeleton: sk,
            controls: rig,
        } = build(&demo_guides()).unwrap();
        for (i, c) in rig.controls.iter().enumerate() {
            match &c.parent {
                ControlParent::Control(p) => {
                    let pi = rig.controls.iter().position(|x| &x.name == p).expect(p);
                    assert!(pi < i, "{} listed before parent {p}", c.name);
                }
                ControlParent::Bone(b) => assert!(sk.bone(b).is_some()),
                ControlParent::None => assert_eq!(c.kind, ControlKind::Root),
            }
            if let Some(b) = &c.bone {
                assert!(sk.bone(b).is_some(), "{b}");
            }
            // Spaces start with the control's parent and only name earlier controls.
            if let Some(first) = c.spaces.first() {
                assert_eq!(
                    c.parent,
                    ControlParent::Control(first.clone()),
                    "{}",
                    c.name
                );
            }
            for sp in &c.spaces {
                let si = rig.controls.iter().position(|x| &x.name == sp).expect(sp);
                assert!(si < i, "{} space {sp} comes later", c.name);
            }
        }
        assert_eq!(rig.ik_chains.len(), 4);
        // Every left limb control has its right twin for pose mirroring.
        for chain in rig.ik_chains.iter().filter(|c| c.name.ends_with("_l")) {
            let mut limb = vec![chain.ik_control.clone(), chain.pole_control.clone()];
            limb.extend(chain.fk_controls.iter().cloned());
            for left in limb {
                let right = left.replace("_l_", "_r_");
                assert!(rig.mirror_pairs.contains(&[left.clone(), right]), "{left}");
            }
        }
        assert!(rig.mirror_pairs.iter().all(|[l, r]| l != r));
        assert_eq!(rig.control("hand_l_ik_ctrl").unwrap().spaces.len(), 4);
        assert_eq!(
            rig.control("head_ctrl").unwrap().spaces,
            ["neck_01_ctrl", "spine_03_ctrl", "cog_ctrl", "root_ctrl"]
        );
        let legs: Vec<_> = rig.ik_chains.iter().filter(|c| c.foot.is_some()).collect();
        assert_eq!(legs.len(), 2);
        let f = legs[0].foot.as_ref().unwrap();
        assert!(f.heel.z < f.ball.z && f.ball.z < f.toe_tip.z);
        // Poles sit in front of knees and behind elbows.
        assert!(
            rig.control("calf_l_pole_ctrl").unwrap().position.z > sk.bone("calf_l").unwrap().head.z
        );
        assert!(
            rig.control("lowerarm_l_pole_ctrl").unwrap().position.z
                < sk.bone("lowerarm_l").unwrap().head.z
        );
        // Straight ahead, not off to the side.
        for (pole, joint) in [
            ("calf_l_pole_ctrl", "calf_l"),
            ("lowerarm_l_pole_ctrl", "lowerarm_l"),
        ] {
            let off = rig.control(pole).unwrap().position - sk.bone(joint).unwrap().head;
            assert!(
                off.x.abs() < 0.05 * off.length(),
                "{pole} leans sideways: {off}"
            );
        }
    }

    #[test]
    fn moving_a_guide_moves_the_rebuilt_bone() {
        let mut g = demo_guides();
        let arm = g.components.iter_mut().find(|c| c.id == "arm_l").unwrap();
        arm.guides[2].position += Vec3::new(0.0, 0.05, -0.02);
        let elbow = arm.guides[2].position;
        let sk = build(&g).unwrap().skeleton;
        assert_eq!(sk.bone("lowerarm_l").unwrap().head, elbow);
        assert_eq!(sk.bone("upperarm_l").unwrap().tail, elbow);
    }

    #[test]
    fn spine_joint_count_is_an_option() {
        let mut g = demo_guides();
        g.components[1].options.joints = Some(5);
        // The head and arms hang from the top spine joint.
        for c in &mut g.components {
            if c.parent.as_deref() == Some("spine_03") {
                c.parent = Some("spine_05".into());
            }
        }
        let Built {
            skeleton: sk,
            controls: rig,
        } = build(&g).unwrap();
        assert!(sk.bone("spine_05").is_some());
        assert_eq!(
            rig.control("clavicle_l_ctrl").unwrap().parent,
            ControlParent::Control("spine_05_ctrl".into())
        );
    }

    #[test]
    fn fingers_tail_and_chain_presets_add_and_remove() {
        let mut g = demo_guides();
        let fingers = add_preset(&mut g, Preset::Fingers, &AddOptions::default()).unwrap();
        assert_eq!(fingers.len(), 10);
        let tail = add_preset(&mut g, Preset::Tail, &AddOptions::default()).unwrap();
        assert_eq!(tail, ["tail"]);
        let ear = AddOptions {
            parent: Some("head".into()),
            joints: Some(2),
            id: Some("ear".into()),
            ..Default::default()
        };
        assert_eq!(add_preset(&mut g, Preset::Chain, &ear).unwrap(), ["ear"]);
        let Built {
            skeleton: sk,
            controls: rig,
        } = build(&g).unwrap();
        for n in [
            "thumb_01_l",
            "pinky_03_r",
            "index_02_l",
            "tail_05",
            "ear_02",
        ] {
            assert!(sk.bone(n).is_some(), "{n}");
        }
        assert_eq!(sk.bone("index_01_l").unwrap().region, Region::ArmL);
        // Fingers follow the hand bone in IK and FK; the tail hangs from the pelvis control.
        assert_eq!(
            rig.control("index_01_l_ctrl").unwrap().parent,
            ControlParent::Bone("hand_l".into())
        );
        assert_eq!(
            rig.control("tail_01_ctrl").unwrap().parent,
            ControlParent::Control("pelvis_ctrl".into())
        );
        // Thumbs sit in front of the other fingers.
        assert!(sk.bone("thumb_01_l").unwrap().head.z > sk.bone("index_01_l").unwrap().head.z);

        // Removing an arm takes its fingers with it.
        let removed = remove(&mut g, "arm_l").unwrap();
        assert_eq!(removed.len(), 6);
        assert!(g.component("index_l").is_none() && g.component("index_r").is_some());
        assert!(add_preset(&mut g, Preset::Fingers, &AddOptions::default()).is_err());
    }

    #[test]
    fn rejects_broken_layouts() {
        let mut g = demo_guides();
        g.components[3].parent = Some("nope".into());
        let e = build(&g).err().unwrap().to_string();
        assert!(e.contains("nope"), "{e}");

        let mut g = demo_guides();
        g.components[3].guides.pop();
        assert!(build(&g).is_err());

        let mut g = demo_guides();
        let dup = g.components[3].clone();
        g.components.push(dup);
        assert!(build(&g).is_err());

        let mut g = demo_guides();
        let p = g.components[4].guides[1].position;
        g.components[4].guides[2].position = p;
        let e = build(&g).err().unwrap().to_string();
        assert!(e.contains("no length"), "{e}");
    }

    #[test]
    fn mirror_and_spine_joint_helpers() {
        let mut g = demo_guides();
        g.components[3].guides[2].position += Vec3::new(0.0, -0.1, 0.0);
        assert_eq!(mirror_side(&mut g, Side::Left), 2);
        let l = g.component("arm_l").unwrap().guides[2].position;
        let r = g.component("arm_r").unwrap().guides[2].position;
        assert!((mirror_point(&g, l) - r).length() < 1e-5);
        assert_eq!(
            twin_id(g.component("leg_r").unwrap()).as_deref(),
            Some("leg_l")
        );

        set_spine_joints(&mut g, "spine", 5).unwrap();
        assert_eq!(
            g.component("head").unwrap().parent.as_deref(),
            Some("spine_05")
        );
        let sk = build(&g).unwrap().skeleton;
        assert_eq!(
            sk.bone("clavicle_l").unwrap().parent,
            sk.index_of("spine_05")
        );
        assert!(set_spine_joints(&mut g, "spine", 0).is_err());
    }

    #[test]
    fn guides_round_trip_through_json() {
        let g = demo_guides();
        let text = serde_json::to_string_pretty(&g).unwrap();
        let back: Guides = serde_json::from_str(&text).unwrap();
        assert_eq!(back.components.len(), g.components.len());
        assert!(back.components.iter().all(|c| c.options.twist));
    }

    #[test]
    fn control_styles_import_and_attach() {
        let mut g = demo_guides();
        // A styled control keeps its look through a rebuild, and clears when reset.
        let built = build(&g).unwrap();
        let name = built.controls.controls[0].name.clone();
        set_control_style(
            &mut g,
            &name,
            ControlStyle {
                shape: Some(Shape::Cube),
                size: Some(0.02),
                color: None,
            },
        )
        .unwrap();
        let styled = build(&g).unwrap();
        let c = styled.controls.control(&name).unwrap();
        assert_eq!((c.shape, c.size), (Shape::Cube, 0.02));
        let json = serde_json::to_string(&g).unwrap();
        let back: Guides = serde_json::from_str(&json).unwrap();
        assert_eq!(
            back.control_styles.get(&name).unwrap().shape,
            Some(Shape::Cube)
        );
        set_control_style(&mut g, &name, ControlStyle::default()).unwrap();
        assert!(g.control_styles.is_empty());
        assert!(
            set_control_style(
                &mut g,
                &name,
                ControlStyle {
                    size: Some(0.0),
                    ..Default::default()
                }
            )
            .is_err()
        );

        // A tail from another layout comes in on the chosen bone and builds.
        let mut src = demo_guides();
        add_preset(&mut src, Preset::Tail, &AddOptions::default()).unwrap();
        let mut dst = demo_guides();
        let ids = import(&mut dst, &src, "pelvis").unwrap();
        assert_eq!(ids, vec!["tail".to_string()]);
        assert_eq!(
            dst.component("tail").unwrap().parent.as_deref(),
            Some("pelvis")
        );
        assert!(build(&dst).unwrap().skeleton.bone("tail_01").is_some());
        // Importing it again clashes with the id, and body parts are refused.
        assert!(import(&mut dst, &src, "pelvis").is_err());
        assert!(
            import(&mut demo_guides(), &demo_guides(), "pelvis")
                .unwrap()
                .is_empty()
        );
        assert!(import(&mut demo_guides(), &src, "no_such_bone").is_err());

        // Attaching moves a part and keeps the layout buildable.
        attach(&mut dst, "tail", "head").unwrap();
        assert!(
            build(&dst)
                .unwrap()
                .skeleton
                .bone("tail_01")
                .unwrap()
                .parent
                .is_some()
        );
        assert!(attach(&mut dst, "tail", "no_such_bone").is_err());

        // A part can hang from one built after it: the order follows the parent.
        let last = build(&dst)
            .unwrap()
            .skeleton
            .bones
            .iter()
            .rev()
            .find(|b| b.component == "tail")
            .unwrap()
            .name
            .clone();
        attach(&mut dst, "arm_l", &last).unwrap();
        let at = |id: &str| dst.components.iter().position(|c| c.id == id).unwrap();
        assert!(at("tail") < at("arm_l"));
        // Hanging a part from its own child, or its own bones, is refused.
        let before = dst.components.len();
        assert!(attach(&mut dst, "tail", "hand_l").is_err());
        assert!(attach(&mut dst, "tail", &last).is_err());
        assert_eq!(dst.components.len(), before);
    }

    #[test]
    fn toes_hang_from_the_ball_of_each_foot() {
        let mut g = demo_guides();
        let ids = add_preset(&mut g, Preset::Toes, &AddOptions::default()).unwrap();
        assert_eq!(ids.len(), 10);
        let built = build(&g).unwrap();
        for side in ["_l", "_r"] {
            let toe = built.skeleton.bone(&format!("toe_big_01{side}")).unwrap();
            let ball = built.skeleton.bone(&format!("ball{side}")).unwrap();
            assert_eq!(toe.parent, built.skeleton.index_of(&ball.name));
        }
        // Toes are plain FK chains: a control per joint, as fingers have.
        assert!(
            built.controls.control("ctrl_toe_big_01_l").is_some()
                || built
                    .controls
                    .controls
                    .iter()
                    .any(|c| c.name.contains("toe_big"))
        );
        let mut no_side = demo_guides();
        assert!(
            add_preset(
                &mut no_side,
                Preset::Toes,
                &AddOptions {
                    side: Some(Side::Center),
                    ..Default::default()
                }
            )
            .is_err()
        );
    }
}
