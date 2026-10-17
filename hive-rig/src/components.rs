//! Premade rig components placed by editable guides.
//!
//! A rig is an ordered list of components (root, spine, head, arms, legs, and
//! chains for fingers, tails and the like). Each component owns a few named guide
//! points. Building turns the guides into bones and animator controls. The guides
//! are saved next to the rig as `<name>.guides.json`, so they can be moved,
//! components can be added or removed, and the rig rebuilt from them.

use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result, bail, ensure};
use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::controls::{
    Control, ControlKind, ControlParent, ControlRig, FootRoll, IkChain, Shape, ctrl_name, fk_name,
};
use crate::landmarks::{Landmarks, mirror};
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
}

impl Kind {
    /// Guide names in order, or `None` for a chain (two or more guides, any names).
    pub fn guide_names(self) -> Option<&'static [&'static str]> {
        Some(match self {
            Kind::Root => &["root"],
            Kind::Spine => &["pelvis", "chest"],
            Kind::Head => &["neck", "head", "head_top"],
            Kind::Arm => &["clavicle", "shoulder", "elbow", "wrist", "hand_tip"],
            Kind::Leg => &["hip", "knee", "ankle", "ball", "toe", "heel"],
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
        }
    }
    assign_spaces(&mut b);
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

    let mid = (up.head + end.head) * 0.5;
    let bend = (lo.head - mid).try_normalize().unwrap_or(bend_fallback);
    let bend = if bend.dot(bend_fallback) < 0.0 {
        bend_fallback
    } else {
        bend
    };
    let pole = format!("{}_pole_ctrl", lo.name);
    b.control(
        pole.clone(),
        ControlKind::Pole,
        root,
        None,
        lo.head + bend * 0.3 * h,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{demo::demo_humanoid, landmarks::detect};

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
    fn guides_round_trip_through_json() {
        let g = demo_guides();
        let text = serde_json::to_string_pretty(&g).unwrap();
        let back: Guides = serde_json::from_str(&text).unwrap();
        assert_eq!(back.components.len(), g.components.len());
        assert!(back.components.iter().all(|c| c.options.twist));
    }
}
