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
    BendSegment, Bendy, Control, ControlKind, ControlParent, ControlRig, FaceBoard, FootRoll,
    HockSegment, IkChain, SOFT_ZONE, Shape, SplineChain, ctrl_name, fk_name,
};
use crate::face_board::{self, Dir};
use crate::landmarks::{Landmarks, mirror};
use crate::naming::Naming;
use crate::skeleton::{Region, Side, Skeleton, Twist};
use crate::skin::SkinLimits;

pub const GUIDES_FORMAT: &str = "hive-rig-guides";
/// The newest guides version this program knows. A guides file's `version`
/// pins the builder [`Rules`] it is built with; new files get this one.
pub const GUIDES_VERSION: u32 = 1;
/// The default spine joint count of the current [`Rules`].
pub const SPINE_JOINTS: usize = RULES_V1.spine_joints;

/// Builder behaviours that aren't written in the guides themselves, pinned by
/// the guides `version` so a layout rebuilds the same way in later releases.
/// When a release changes one of these, it adds a guides version with the new
/// value; files of older versions keep the rules they were written with.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rules {
    /// Spine joints when the spine component doesn't set `joints`.
    pub spine_joints: usize,
    /// Foot and paw roll: degrees of heel lift before the roll moves to the toe tip.
    pub foot_break_degrees: f32,
    /// IK pole distance from the elbow or knee, as a fraction of the limb's length.
    pub pole_reach: f32,
    /// The least pole distance, as a fraction of the body height.
    pub pole_min_reach: f32,
    /// An arm whose elbow and wrist are below the belly line (`crotch_y`) is a
    /// front leg: skinned as a leg, starting in IK, with an upright IK control.
    pub planted_front_legs: bool,
}

/// The rules of guides version 1 (hive-rig 0.1.0 onwards).
pub const RULES_V1: Rules = Rules {
    spine_joints: 3,
    foot_break_degrees: 30.0,
    pole_reach: 0.5,
    pole_min_reach: 0.1,
    planted_front_legs: true,
};

/// The builder rules pinned by a guides `version`. Every version so far has
/// the version 1 rules; a version newer than this program builds with the
/// newest rules it knows (and loading it warns).
pub fn rules(_version: u32) -> Rules {
    RULES_V1
}

/// Bend joints on each segment of a bendy limb.
pub const BEND_JOINTS: usize = 4;

/// The editable rig layout: components and their guide positions (model space,
/// Y-up, metres, facing +Z).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Guides {
    pub format: String,
    /// Pins the builder [`Rules`]; kept as read when the file is saved again.
    pub version: u32,
    /// The hive-rig that last wrote the file (absent before 0.1.0 stamped it).
    /// Writing always stamps this program's version.
    #[serde(default, rename = "hive_rig", serialize_with = "crate::version::stamp")]
    pub written_by: Option<String>,
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
    /// Influence limits for the skin weights (most bones per vertex, smallest
    /// weight kept). Absent means four influences and no pruning.
    #[serde(default, skip_serializing_if = "SkinLimits::is_default")]
    pub skin: SkinLimits,
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

/// Whole-body measurements from the landmarks (the skin weights don't use them).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Body {
    pub floor: f32,
    pub height: f32,
    pub center_x: f32,
    /// Height of the crotch: an arm reaching below it is a planted front leg.
    pub crotch_y: f32,
    /// Half the torso's width at the chest.
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
    /// A three-segment hind leg for quadrupeds and creatures: thigh, calf, hock
    /// (metatarsus) and a paw with a toe, with the knee in front and the hock behind.
    QuadLeg,
    /// FK chain of any length: fingers, tails, ears, hair, props.
    Chain,
    /// A curve-driven chain (tentacle): a few curve controls bend a smooth
    /// curve that the joints follow, with stretch, volume and an FK layer.
    Spline,
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

/// The quad leg's guides: hip, knee (stifle), hock, ankle (where the paw
/// starts), ball (the pads), toe tip and heel (the back of the paw on the ground).
pub const QUAD_LEG_GUIDES: [&str; 7] = ["hip", "knee", "hock", "ankle", "ball", "toe", "heel"];

impl Kind {
    /// Guide names in order, or `None` for a chain (two or more guides, any names).
    pub fn guide_names(self) -> Option<&'static [&'static str]> {
        Some(match self {
            Kind::Root => &["root"],
            Kind::Spine => &["pelvis", "chest"],
            Kind::Head => &["neck", "head", "head_top"],
            Kind::Arm => &["clavicle", "shoulder", "elbow", "wrist", "hand_tip"],
            Kind::Leg => &["hip", "knee", "ankle", "ball", "toe", "heel"],
            Kind::QuadLeg => &QUAD_LEG_GUIDES,
            Kind::Eyes => &["eye_l", "eye_r", "target"],
            Kind::Jaw => &["jaw", "chin"],
            Kind::Face => &FACE_GUIDES,
            Kind::Chain | Kind::Spline => return None,
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
    /// Spine joints between the pelvis and the chest guide, or a spline chain's
    /// joints along its guides (one per guide segment when unset).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub joints: Option<usize>,
    /// Twist helper bones on arms and legs.
    #[serde(default = "yes")]
    pub twist: bool,
    /// Spine: the spine joints follow a curve between the hips and chest
    /// controls (a spline spine) instead of an FK control each.
    #[serde(default, skip_serializing_if = "is_false")]
    pub spline: bool,
    /// Curve controls of a spline chain or spline spine, ends included.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub controls: Option<usize>,
    /// Bendy arms, legs and quad legs (whose thigh and calf bend; the hock stays
    /// one bone): bend controls arc each segment's joints, and a pin
    /// control moves the elbow or knee. Replaces the twist helpers with
    /// [`BEND_JOINTS`] bend joints per segment (which still take the twist shares
    /// when `twist` is on).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub bendy: bool,
    /// Default of the IK limb's `soft` attribute (0 to 1), which eases the reach
    /// near full extension so the knee or elbow doesn't pop.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub soft: f32,
    /// Quad legs: default of the IK control's `spring` attribute (0 to 1). At 1
    /// the hock swings with the thigh as the paw moves (a spring hock, see
    /// [`crate::controls::spring_hock`]); at 0 it keeps the hock control's angle.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub spring: f32,
    /// Face: a face board beside the head (sliders posing the face joints
    /// and face blendshapes) with lip zippers and sticky lips. On a face,
    /// `joints` is the extra lip joints on each lip between its middle and
    /// each corner (0 to 3; none when unset).
    #[serde(default, skip_serializing_if = "is_false")]
    pub board: bool,
}

fn is_zero(v: &f32) -> bool {
    *v == 0.0
}

fn yes() -> bool {
    true
}

fn is_false(b: &bool) -> bool {
    !*b
}

impl Default for Options {
    fn default() -> Self {
        Options {
            joints: None,
            twist: true,
            spline: false,
            controls: None,
            bendy: false,
            soft: 0.0,
            spring: 0.0,
            board: false,
        }
    }
}

/// Curve controls on a spline chain (tentacle) unless the options say otherwise.
pub const SPLINE_CONTROLS: usize = 4;
/// Curve controls on a spline spine: hips, middle and chest.
pub const SPINE_SPLINE_CONTROLS: usize = 3;

impl Options {
    /// Curve controls of a spline chain (`spine` false) or spline spine.
    pub fn curve_controls(&self, spine: bool) -> usize {
        self.controls.unwrap_or(if spine {
            SPINE_SPLINE_CONTROLS
        } else {
            SPLINE_CONTROLS
        })
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
        Ok(g)
    }

    /// Write the guides as pretty JSON, stamped with this program's version.
    pub fn save(&self, path: &std::path::Path) -> Result<()> {
        std::fs::write(path, serde_json::to_vec_pretty(self)?)
            .with_context(|| format!("writing {}", path.display()))
    }

    /// The builder rules this layout's `version` pins.
    pub fn rules(&self) -> Rules {
        rules(self.version)
    }

    /// The spine joint count of a spine component.
    pub fn spine_joints(&self, c: &Component) -> usize {
        c.options.joints.unwrap_or(self.rules().spine_joints)
    }

    /// A warning when the file came from a newer hive-rig: a guides version
    /// this program doesn't know (built with the newest rules it has), or a
    /// newer program's stamp.
    pub fn newer_warning(&self) -> Option<String> {
        if self.version > GUIDES_VERSION {
            return Some(format!(
                "these guides are version {} from a newer hive-rig{}; this {} knows up to version {GUIDES_VERSION}, so they build with its rules and may differ",
                self.version,
                self.written_by
                    .as_deref()
                    .map(|v| format!(" ({v})"))
                    .unwrap_or_default(),
                crate::version::CURRENT
            ));
        }
        crate::version::newer_note("these guides", self.written_by.as_deref())
    }

    /// Where the layout came from, for display: "guides version 1, written by
    /// hive-rig 0.1.0".
    pub fn origin(&self) -> String {
        let by = match &self.written_by {
            Some(v) => format!("hive-rig {v}"),
            None => "a hive-rig from before version stamps".into(),
        };
        format!("guides version {}, written by {by}", self.version)
    }
}

pub fn spine_name(i: usize) -> String {
    format!("spine_{:02}", i + 1)
}

/// `upperarm_l` -> `upperarm_twist_01_l`.
pub fn twist_name(bone: &str) -> String {
    twist_joint_name(bone, 1)
}

/// Twist (or bend) joint `n` (from 1) of a limb segment: `upperarm_l`, 2 ->
/// `upperarm_twist_02_l`.
pub fn twist_joint_name(bone: &str, n: usize) -> String {
    match bone.rsplit_once('_') {
        Some((base, side)) => format!("{base}_twist_{n:02}_{side}"),
        None => format!("{bone}_twist_{n:02}"),
    }
}

/// Bone `i` (from 1) of a chain: `index_l` -> `index_01_l`, `tail` -> `tail_01`.
fn chain_bone_name(id: &str, side: Side, i: usize) -> String {
    let s = side.suffix();
    format!("{}_{i:02}{s}", id_base(id, side))
}

/// A sided component's id without its side: `tentacle_l` -> `tentacle`.
fn id_base(id: &str, side: Side) -> &str {
    let s = side.suffix();
    if s.is_empty() {
        id
    } else {
        id.strip_suffix(s).unwrap_or(id)
    }
}

/// Joints in a spline chain.
fn spline_joints(c: &Component) -> usize {
    c.options.joints.unwrap_or(c.guides.len().saturating_sub(1))
}

/// The bone and control names a component takes from its id, in build order
/// (chain bones and their controls, the eyes' master control). Every other name
/// a component builds is fixed by its kind.
pub fn id_names(c: &Component) -> (Vec<String>, Vec<String>) {
    match c.kind {
        Kind::Chain => {
            let bones: Vec<String> = (1..c.guides.len())
                .map(|i| chain_bone_name(&c.id, c.side, i))
                .collect();
            let controls = bones.iter().map(|b| ctrl_name(b)).collect();
            (bones, controls)
        }
        Kind::Spline => {
            let bones = (1..=spline_joints(c))
                .map(|i| chain_bone_name(&c.id, c.side, i))
                .collect();
            let (base, s) = (id_base(&c.id, c.side), c.side.suffix());
            let k = c.options.curve_controls(false);
            let curve = (1..=k).map(|i| format!("{base}{s}_ik_{i:02}_ctrl"));
            let fk = (1..k).map(|i| format!("{base}{s}_fk_{i:02}_ctrl"));
            (bones, curve.chain(fk).collect())
        }
        Kind::Eyes => (Vec::new(), vec![format!("{}_ctrl", c.id)]),
        _ => (Vec::new(), Vec::new()),
    }
}

/// Kinds that can be added to any rig: the body's own kinds are fixed to the
/// mannequin's bone names, so a rig has exactly one of each.
pub fn is_premade_part(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::Chain | Kind::Spline | Kind::Eyes | Kind::Jaw | Kind::Face
    )
}

// --------------------------------------------------------------------------- default layout

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
        written_by: None,
        body: Body {
            floor: lm.floor,
            height: lm.height,
            center_x: lm.center_x,
            crotch_y: lm.crotch_y,
            torso_half_width: lm.torso_half_width,
        },
        components,
        naming: Naming::default(),
        skin: SkinLimits::default(),
        control_styles: BTreeMap::new(),
    }
}

/// Guide points of a three-segment hind leg hanging from `hip` down to `floor`,
/// in [`QUAD_LEG_GUIDES`] order. The proportions are fractions of the hip height:
/// the knee sits forward, the hock back, and the paw rests on the floor facing +Z.
pub fn quad_leg_points(hip: Vec3, floor: f32) -> [Vec3; 7] {
    let len = (hip.y - floor).max(1e-3);
    // (height above the floor, forward of the hip), as fractions of the hip height.
    let at = |up: f32, fwd: f32| Vec3::new(hip.x, floor + up * len, hip.z + fwd * len);
    [
        hip,
        at(0.655, 0.21),  // knee (stifle), forward
        at(0.345, -0.10), // hock, back
        at(0.07, -0.035), // ankle: the paw starts here
        at(0.035, 0.10),  // ball: the pads
        at(0.017, 0.21),  // toe tip
        at(0.0, -0.085),  // heel: the back of the paw on the ground
    ]
}

/// The built-in quadruped (dog) layout for a character `height` metres tall
/// (floor to the top of the head), facing +Z: spine, neck and head, a tail, two
/// front legs and two three-segment hind legs. Front legs are arm components
/// (scapula, upper arm, forearm and a paw) whose elbows point back; the hind
/// legs are [`Kind::QuadLeg`].
pub fn quadruped(height: f32) -> Guides {
    let h = height;
    let v = |x: f32, y: f32, z: f32| Vec3::new(x, y, z) * h;
    let chest_bone = spine_name(SPINE_JOINTS - 1);
    let mut components = vec![
        Component::new("root", Kind::Root, Side::Center, None).guide("root", Vec3::ZERO),
        Component::new("spine", Kind::Spine, Side::Center, Some("root"))
            .guide("pelvis", v(0.0, 0.6, -0.38))
            .guide("chest", v(0.0, 0.64, 0.3)),
        Component::new("head", Kind::Head, Side::Center, Some(&chest_bone))
            .guide("neck", v(0.0, 0.68, 0.36))
            .guide("head", v(0.0, 0.88, 0.52))
            .guide("head_top", v(0.0, 0.84, 0.74)),
    ];
    for side in [Side::Left, Side::Right] {
        let sx = side.sign();
        let s = side.suffix();
        components.push(
            Component::new(format!("front_leg{s}"), Kind::Arm, side, Some(&chest_bone))
                .guide("clavicle", v(0.05 * sx, 0.7, 0.26))
                .guide("shoulder", v(0.1 * sx, 0.52, 0.34))
                .guide("elbow", v(0.1 * sx, 0.32, 0.24))
                .guide("wrist", v(0.1 * sx, 0.07, 0.29))
                .guide("hand_tip", v(0.1 * sx, 0.015, 0.38)),
        );
    }
    for side in [Side::Left, Side::Right] {
        let s = side.suffix();
        let mut leg = Component::new(format!("hind_leg{s}"), Kind::QuadLeg, side, Some("pelvis"));
        let points = quad_leg_points(v(0.1 * side.sign(), 0.56, -0.38), 0.0);
        for (name, p) in QUAD_LEG_GUIDES.iter().zip(points) {
            leg = leg.guide(*name, p);
        }
        components.push(leg);
    }
    components.push(chain(
        "tail".into(),
        Side::Center,
        "pelvis",
        (0..=5).map(|k| {
            let t = k as f32 / 5.0;
            v(0.0, 0.62 + 0.1 * t, -0.48 - 0.4 * t)
        }),
    ));
    // Below the belly line the skin belongs to a leg; everything above it is
    // body, however wide.
    creature(h, components, 0.45 * h, 0.5 * h)
}

/// A creature layout standing on the floor at the origin, facing +Z, with its
/// belly line at `crotch_y` and half its body's width `torso_half_width`.
fn creature(h: f32, components: Vec<Component>, crotch_y: f32, torso_half_width: f32) -> Guides {
    Guides {
        format: GUIDES_FORMAT.into(),
        version: GUIDES_VERSION,
        written_by: None,
        body: Body {
            floor: 0.0,
            height: h,
            center_x: 0.0,
            crotch_y,
            torso_half_width,
        },
        components,
        naming: Naming::default(),
        skin: SkinLimits::default(),
        control_styles: BTreeMap::new(),
    }
}

/// A chain component through `points`, with guides `j1`, `j2`, ...
pub(crate) fn chain(
    id: String,
    side: Side,
    parent: &str,
    points: impl IntoIterator<Item = Vec3>,
) -> Component {
    let mut c = Component::new(id, Kind::Chain, side, Some(parent));
    for (k, p) in points.into_iter().enumerate() {
        c = c.guide(format!("j{}", k + 1), p);
    }
    c
}

/// The built-in bird layout for a bird `height` metres tall (floor to the top
/// of the head), facing +Z: a tilted spine, a long neck and head with the beak
/// along the head bone, two wings, two legs with hocks and a tail. The wings
/// are arm components spread to the sides (shoulder, elbow folded back, wrist
/// and the hand to the wing tip), so they get IK/FK and a pole like an arm;
/// the legs are [`Kind::QuadLeg`], whose knee and hock fold the way a bird's do.
pub fn bird(height: f32) -> Guides {
    let h = height;
    let v = |x: f32, y: f32, z: f32| Vec3::new(x, y, z) * h;
    let chest_bone = spine_name(SPINE_JOINTS - 1);
    let mut components = vec![
        Component::new("root", Kind::Root, Side::Center, None).guide("root", Vec3::ZERO),
        Component::new("spine", Kind::Spine, Side::Center, Some("root"))
            .guide("pelvis", v(0.0, 0.46, -0.12))
            .guide("chest", v(0.0, 0.56, 0.12)),
        Component::new("head", Kind::Head, Side::Center, Some(&chest_bone))
            .guide("neck", v(0.0, 0.6, 0.14))
            .guide("head", v(0.0, 0.84, 0.2))
            .guide("head_top", v(0.0, 0.86, 0.36)),
    ];
    for side in [Side::Left, Side::Right] {
        let sx = side.sign();
        components.push(
            Component::new(
                format!("wing{}", side.suffix()),
                Kind::Arm,
                side,
                Some(&chest_bone),
            )
            .guide("clavicle", v(0.03 * sx, 0.58, 0.1))
            .guide("shoulder", v(0.1 * sx, 0.6, 0.06))
            .guide("elbow", v(0.32 * sx, 0.62, -0.02))
            .guide("wrist", v(0.55 * sx, 0.62, 0.03))
            .guide("hand_tip", v(0.85 * sx, 0.6, -0.05)),
        );
    }
    for side in [Side::Left, Side::Right] {
        let mut leg = Component::new(
            format!("leg{}", side.suffix()),
            Kind::QuadLeg,
            side,
            Some("pelvis"),
        );
        let points = quad_leg_points(v(0.07 * side.sign(), 0.42, -0.04), 0.0);
        for (name, p) in QUAD_LEG_GUIDES.iter().zip(points) {
            leg = leg.guide(*name, p);
        }
        components.push(leg);
    }
    components.push(chain(
        "tail".into(),
        Side::Center,
        "pelvis",
        (0..4).map(|k| {
            let t = k as f32 / 3.0;
            v(0.0, 0.47 - 0.08 * t, -0.22 - 0.4 * t)
        }),
    ));
    // The thighs sit inside the body, as a bird's do, so the leg skin starts
    // below its belly; the wings reach out past the body.
    creature(h, components, 0.3 * h, 0.12 * h)
}

/// Legs of the built-in spider on each side, front to back.
pub const SPIDER_LEGS: usize = 4;

/// The built-in spider layout for a spider `height` metres tall (floor to the
/// top of its knees), facing +Z: a spine through the cephalothorax, a head with
/// the fangs along the head bone, an abdomen chain behind and four legs a side.
/// Each leg is a four-bone FK chain (coxa and femur up to the knee, then patella
/// and tibia, metatarsus and tarsus down to the floor) hung from the spine bone
/// it grows from, with the front legs reaching forward and the back legs back.
pub fn spider(height: f32) -> Guides {
    let h = height;
    let v = |x: f32, y: f32, z: f32| Vec3::new(x, y, z) * h;
    let mut components = vec![
        Component::new("root", Kind::Root, Side::Center, None).guide("root", Vec3::ZERO),
        Component::new("spine", Kind::Spine, Side::Center, Some("root"))
            .guide("pelvis", v(0.0, 0.42, -0.02))
            .guide("chest", v(0.0, 0.44, 0.3)),
        Component::new(
            "head",
            Kind::Head,
            Side::Center,
            Some(&spine_name(SPINE_JOINTS - 1)),
        )
        .guide("neck", v(0.0, 0.44, 0.33))
        .guide("head", v(0.0, 0.45, 0.4))
        .guide("head_top", v(0.0, 0.4, 0.54)),
        chain(
            "abdomen".into(),
            Side::Center,
            "pelvis",
            [
                v(0.0, 0.46, -0.08),
                v(0.0, 0.56, -0.42),
                v(0.0, 0.48, -0.78),
            ],
        ),
    ];
    // (where along the body it grows, the spine bone there, its sweep forward in degrees)
    let legs: [(f32, usize, f32); SPIDER_LEGS] = [
        (0.27, 2, 45.0),
        (0.17, 1, 15.0),
        (0.08, 0, -15.0),
        (0.0, 0, -45.0),
    ];
    for side in [Side::Left, Side::Right] {
        let sx = side.sign();
        for (n, (z, bone, sweep)) in legs.into_iter().enumerate() {
            let (sin, cos) = sweep.to_radians().sin_cos();
            // (distance out from the body's mid-line, height above the floor)
            let at = |out: f32, up: f32| v(sx * out * cos, up, z + out * sin);
            components.push(chain(
                format!("leg{}{}", n + 1, side.suffix()),
                side,
                &spine_name(bone),
                [
                    at(0.07, 0.44),
                    at(0.34, 0.95),
                    at(0.62, 0.48),
                    at(0.8, 0.12),
                    at(0.88, 0.0),
                ],
            ));
        }
    }
    // A low body with legs spread wide: no skin is cut off as a leg or an
    // arm region, so each leg's skin follows its nearest bones.
    creature(h, components, -h, 10.0 * h)
}

/// Joints of the built-in snake's spline spine (its top joint carries the head).
pub const SNAKE_SPINE_JOINTS: usize = 10;

/// The built-in snake layout, `height` metres being its size unit (the snake is
/// about four of them long), facing +Z along the floor: a spline spine with ten
/// joints driven by five curve controls, a raised neck and head with the jaw
/// along the head bone, and a spline tail tapering behind.
pub fn snake(height: f32) -> Guides {
    let h = height;
    let v = |x: f32, y: f32, z: f32| Vec3::new(x, y, z) * h;
    let mut spine = Component::new("spine", Kind::Spine, Side::Center, Some("root"))
        .guide("pelvis", v(0.0, 0.1, -1.2))
        .guide("chest", v(0.0, 0.14, 1.0));
    spine.options.joints = Some(SNAKE_SPINE_JOINTS);
    spine.options.spline = true;
    spine.options.controls = Some(5);
    let mut tail = Component::new("tail", Kind::Spline, Side::Center, Some("pelvis"));
    for k in 0..=8 {
        let t = k as f32 / 8.0;
        tail = tail.guide(
            format!("j{}", k + 1),
            v(0.0, 0.12 - 0.05 * t, -1.26 - 1.5 * t),
        );
    }
    tail.options.controls = Some(4);
    let components = vec![
        Component::new("root", Kind::Root, Side::Center, None).guide("root", Vec3::ZERO),
        spine,
        Component::new(
            "head",
            Kind::Head,
            Side::Center,
            Some(&spine_name(SNAKE_SPINE_JOINTS - 1)),
        )
        .guide("neck", v(0.0, 0.15, 1.05))
        .guide("head", v(0.0, 0.32, 1.3))
        .guide("head_top", v(0.0, 0.32, 1.55)),
        tail,
    ];
    // Lying on the floor: no leg or arm regions, the skin follows the bones.
    creature(h, components, -h, 10.0 * h)
}

/// The built-in horse layout for a horse `height` metres tall (floor to the top
/// of the raised head), facing +Z: a long level back, a long neck rising to the
/// head with the muzzle along the head bone, a hanging tail, two front legs and
/// two three-segment hind legs on long cannons.
///
/// The front legs are arm components whose elbows sit back against the belly:
/// the scapula, the upper arm from the point of the shoulder to the elbow, the
/// forearm and cannon as one bone down to the fetlock, and the pastern and hoof
/// as the hand. The hind legs are [`Kind::QuadLeg`] with the stifle forward, the
/// hock high and behind, and the pastern and hoof as the paw. A horse's stifle
/// and hock flex together, so the hind legs start with a full spring hock
/// (`spring` = 1): lifting a hoof folds the whole leg.
pub fn horse(height: f32) -> Guides {
    let h = height;
    let v = |x: f32, y: f32, z: f32| Vec3::new(x, y, z) * h;
    let chest_bone = spine_name(SPINE_JOINTS - 1);
    let mut components = vec![
        Component::new("root", Kind::Root, Side::Center, None).guide("root", Vec3::ZERO),
        Component::new("spine", Kind::Spine, Side::Center, Some("root"))
            .guide("pelvis", v(0.0, 0.7, -0.36))
            .guide("chest", v(0.0, 0.72, 0.2)),
        Component::new("head", Kind::Head, Side::Center, Some(&chest_bone))
            .guide("neck", v(0.0, 0.76, 0.27))
            .guide("head", v(0.0, 0.95, 0.47))
            .guide("head_top", v(0.0, 0.8, 0.66)),
    ];
    for side in [Side::Left, Side::Right] {
        let sx = side.sign();
        components.push(
            Component::new(
                format!("front_leg{}", side.suffix()),
                Kind::Arm,
                side,
                Some(&chest_bone),
            )
            .guide("clavicle", v(0.04 * sx, 0.78, 0.17))
            .guide("shoulder", v(0.09 * sx, 0.6, 0.29))
            .guide("elbow", v(0.09 * sx, 0.46, 0.19))
            .guide("wrist", v(0.09 * sx, 0.065, 0.22))
            .guide("hand_tip", v(0.09 * sx, 0.01, 0.27)),
        );
    }
    for side in [Side::Left, Side::Right] {
        let sx = side.sign();
        let mut leg = Component::new(
            format!("hind_leg{}", side.suffix()),
            Kind::QuadLeg,
            side,
            Some("pelvis"),
        );
        let points = [
            v(0.09 * sx, 0.66, -0.36),  // hip
            v(0.09 * sx, 0.48, -0.23),  // stifle, forward
            v(0.09 * sx, 0.3, -0.45),   // hock, high and behind
            v(0.09 * sx, 0.065, -0.41), // fetlock: the pastern starts here
            v(0.09 * sx, 0.03, -0.37),  // coronet: the top of the hoof
            v(0.09 * sx, 0.005, -0.33), // toe of the hoof
            v(0.09 * sx, 0.0, -0.42),   // heel of the hoof
        ];
        for (name, p) in QUAD_LEG_GUIDES.iter().zip(points) {
            leg = leg.guide(*name, p);
        }
        leg.options.spring = 1.0;
        components.push(leg);
    }
    components.push(chain(
        "tail".into(),
        Side::Center,
        "pelvis",
        (0..=4).map(|k| {
            let t = k as f32 / 4.0;
            v(0.0, 0.7 - 0.3 * t, -0.48 - 0.16 * t + 0.04 * t * t)
        }),
    ));
    // The belly line is above the elbows and stifles, so the front legs are
    // planted; the body is wide enough that nothing above it counts as a leg.
    creature(h, components, 0.52 * h, 0.5 * h)
}

/// Joints of the built-in dinosaur's spline tail.
pub const DINOSAUR_TAIL_JOINTS: usize = 8;

/// The built-in dinosaur (a biped theropod) layout for one `height` metres tall
/// (floor to the top of the head), facing +Z: a level spine balanced over the
/// hips, a curved neck and a big head with the snout along the head bone, two
/// short arms held under the chest, two three-segment legs that stand on their
/// toes, and a long spline tail behind. The arms are arm components (FK, as
/// arms start); the legs are [`Kind::QuadLeg`]: thigh, shin, the long
/// metatarsus as the hock and the toes as the paw.
pub fn dinosaur(height: f32) -> Guides {
    let h = height;
    let v = |x: f32, y: f32, z: f32| Vec3::new(x, y, z) * h;
    let chest_bone = spine_name(SPINE_JOINTS - 1);
    let mut tail = Component::new("tail", Kind::Spline, Side::Center, Some("pelvis"));
    for k in 0..=DINOSAUR_TAIL_JOINTS {
        let t = k as f32 / DINOSAUR_TAIL_JOINTS as f32;
        tail = tail.guide(
            format!("j{}", k + 1),
            v(0.0, 0.66 - 0.16 * t, -0.16 - 1.0 * t),
        );
    }
    tail.options.controls = Some(4);
    let mut components = vec![
        Component::new("root", Kind::Root, Side::Center, None).guide("root", Vec3::ZERO),
        Component::new("spine", Kind::Spine, Side::Center, Some("root"))
            .guide("pelvis", v(0.0, 0.66, -0.08))
            .guide("chest", v(0.0, 0.7, 0.3)),
        Component::new("head", Kind::Head, Side::Center, Some(&chest_bone))
            .guide("neck", v(0.0, 0.73, 0.36))
            .guide("head", v(0.0, 0.9, 0.52))
            .guide("head_top", v(0.0, 0.86, 0.8)),
    ];
    for side in [Side::Left, Side::Right] {
        let sx = side.sign();
        components.push(
            Component::new(
                format!("arm{}", side.suffix()),
                Kind::Arm,
                side,
                Some(&chest_bone),
            )
            .guide("clavicle", v(0.03 * sx, 0.66, 0.32))
            .guide("shoulder", v(0.09 * sx, 0.62, 0.36))
            .guide("elbow", v(0.11 * sx, 0.54, 0.33))
            .guide("wrist", v(0.11 * sx, 0.52, 0.42))
            .guide("hand_tip", v(0.11 * sx, 0.48, 0.48)),
        );
    }
    for side in [Side::Left, Side::Right] {
        let mut leg = Component::new(
            format!("leg{}", side.suffix()),
            Kind::QuadLeg,
            side,
            Some("pelvis"),
        );
        let points = quad_leg_points(v(0.11 * side.sign(), 0.6, -0.06), 0.0);
        for (name, p) in QUAD_LEG_GUIDES.iter().zip(points) {
            leg = leg.guide(*name, p);
        }
        components.push(leg);
    }
    components.push(tail);
    // The arms hang above the belly line, so they aren't planted front legs.
    creature(h, components, 0.4 * h, 0.16 * h)
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
    /// Face joints: brows, eyelids, cheeks, nose and lips, with a lip joint
    /// between the middle and each corner and a face board.
    Face,
    /// A face board for a face that has none: sliders beside the head for the
    /// brows, lids, cheeks, nose, mouth corners, lips and jaw, lip zippers and
    /// sticky lips. `--joints` sets the extra lip joints a side (0 to 3).
    #[value(name = "face_board", alias = "board", alias = "face-board")]
    FaceBoard,
    /// Five toes on each foot, from the ball of the foot (3 joints each).
    Toes,
    /// Three-segment hind legs (thigh, calf, hock, paw) on the pelvis, left and right.
    #[value(name = "quad_legs", alias = "quad_leg", alias = "quad-legs")]
    QuadLegs,
    /// A curve-driven tentacle: curve controls with stretch, volume and an FK
    /// layer, from the pelvis or any bone (`--parent`).
    Tentacle,
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
    /// Curve controls on a tentacle.
    pub controls: Option<usize>,
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
        Preset::QuadLegs => {
            let sides = match opts.side {
                Some(Side::Center) => bail!("hind legs need --side left or right"),
                Some(s) => vec![s],
                None => vec![Side::Left, Side::Right],
            };
            let parent = opts.parent.clone().unwrap_or_else(|| "pelvis".into());
            let pelvis = sk
                .bone(&parent)
                .with_context(|| format!("no bone named {parent}"))?
                .head;
            let floor = g.body.floor;
            ensure!(
                pelvis.y > floor + 0.05 * h,
                "{parent} is too close to the floor for a hind leg"
            );
            for side in sides {
                ensure!(
                    !g.components
                        .iter()
                        .any(|c| matches!(c.kind, Kind::Leg | Kind::QuadLeg) && c.side == side),
                    "the rig already has a {side:?} leg; remove it first"
                );
                let id = format!("hind_leg{}", side.suffix());
                ensure!(g.component(&id).is_none(), "{id} already exists");
                let len = pelvis.y - floor;
                let hip = Vec3::new(
                    g.body.center_x + side.sign() * 0.17 * len,
                    pelvis.y,
                    pelvis.z,
                );
                let mut c = Component::new(&id, Kind::QuadLeg, side, Some(&parent));
                for (name, p) in QUAD_LEG_GUIDES.iter().zip(quad_leg_points(hip, floor)) {
                    c = c.guide(*name, p);
                }
                g.components.push(c);
                added.push(id);
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
            // New faces come with a lip joint between the middle and each
            // corner (so the lips can zip) and a face board.
            c.options.joints = Some(opts.joints.unwrap_or(1));
            c.options.board = true;
            g.components.push(c);
            added.push(id);
        }
        Preset::FaceBoard => {
            let face = g
                .components
                .iter_mut()
                .find(|c| c.kind == Kind::Face)
                .context("add the face first (`add <guides> face`): the board poses its joints")?;
            ensure!(!face.options.board, "the face already has a face board");
            face.options.board = true;
            if let Some(n) = opts.joints {
                face.options.joints = Some(n);
            }
            added.push(face.id.clone());
        }
        Preset::Tentacle => {
            let joints = opts.joints.unwrap_or(8);
            ensure!(
                (2..=64).contains(&joints),
                "--joints must be between 2 and 64"
            );
            if let Some(k) = opts.controls {
                ensure!((2..=16).contains(&k), "--controls must be between 2 and 16");
            }
            let parent = opts.parent.clone().unwrap_or_else(|| "pelvis".into());
            let p = sk
                .bone(&parent)
                .with_context(|| format!("no bone named {parent}"))?;
            let side = p.side;
            let (start, dir, length) = if parent == "pelvis" {
                // Out behind the body (the character faces +Z), nearly level.
                let dir = Vec3::new(0.0, -0.2, -1.0).normalize();
                (p.head - Vec3::Z * 0.07 * h, dir, 0.6 * h)
            } else {
                (p.tail, p.direction(), (p.length() * 3.0).max(0.2 * h))
            };
            let id = match &opts.id {
                Some(id) => id.clone(),
                None => unique_id(g, "tentacle", side),
            };
            ensure!(g.component(&id).is_none(), "{id} already exists");
            let mut c = Component::new(&id, Kind::Spline, side, Some(&parent));
            c.options.controls = opts.controls;
            for k in 0..=joints {
                c = c.guide(
                    format!("j{}", k + 1),
                    start + dir * length * (k as f32 / joints as f32),
                );
            }
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

pub(crate) fn unique_id(g: &Guides, base: &str, side: Side) -> String {
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

/// Which controls a change of look goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StyleScope {
    /// The control alone.
    One,
    /// The control and its left/right twin (what symmetry does in the app).
    WithTwin,
    /// Every control its component built (and their twins, with `twins`).
    Component { twins: bool },
}

/// A change to some controls' look: the fields that are set replace what the
/// controls have, `scale` multiplies their current size, and the rest is kept.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct StyleChange {
    pub shape: Option<Shape>,
    pub size: Option<f32>,
    pub scale: Option<f32>,
    pub color: Option<[f32; 3]>,
}

/// The colour a twin on the other side gets: the left and right side colours
/// swap, any other colour is kept.
pub fn twin_color(color: [f32; 3]) -> [f32; 3] {
    let near = |a: [f32; 3], b: [f32; 3]| a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-3);
    if near(color, LEFT) {
        RIGHT
    } else if near(color, RIGHT) {
        LEFT
    } else {
        color
    }
}

/// The default colours: the centre, the left and right sides, and the root.
pub const SIDE_COLORS: [(&str, [f32; 3]); 4] = [
    ("Centre", CENTER),
    ("Left", LEFT),
    ("Right", RIGHT),
    ("Root", ROOT),
];

/// More colours offered for controls, by name.
pub const MORE_COLORS: [(&str, [f32; 3]); 4] = [
    ("Green", [0.2, 0.85, 0.35]),
    ("Cyan", [0.1, 0.85, 0.9]),
    ("Orange", [1.0, 0.55, 0.1]),
    ("White", [0.95, 0.95, 0.95]),
];

/// The controls a change of look on `control` reaches under `scope`, the
/// control itself first. Each comes with whether it is a twin (on the other
/// side) of the control it follows.
pub fn style_targets(
    built: &Built,
    control: &str,
    scope: StyleScope,
) -> Result<Vec<(String, bool)>> {
    let rig = &built.controls;
    let c = rig
        .control(control)
        .ok_or_else(|| anyhow::anyhow!("no control named '{control}'"))?;
    let twin_of = |name: &str| -> Option<String> {
        rig.mirror_pairs.iter().find_map(|pair| {
            let [l, r] = pair;
            if l == name {
                Some(r.clone())
            } else if r == name {
                Some(l.clone())
            } else {
                None
            }
        })
    };
    let mut out = vec![(c.name.clone(), false)];
    let (own, twins) = match scope {
        StyleScope::One => (vec![], false),
        StyleScope::WithTwin => (vec![], true),
        StyleScope::Component { twins } => (
            rig.controls
                .iter()
                .filter(|o| o.component == c.component && o.name != c.name)
                .map(|o| o.name.clone())
                .collect(),
            twins,
        ),
    };
    out.extend(own.into_iter().map(|n| (n, false)));
    if twins {
        let sided: Vec<String> = out.iter().map(|(n, _)| n.clone()).collect();
        for n in sided {
            if let Some(t) = twin_of(&n)
                && !out.iter().any(|(o, _)| *o == t)
            {
                out.push((t, true));
            }
        }
    }
    Ok(out)
}

/// Change the look of `control` (and, by `scope`, its twin or its component's
/// controls) as written in the guides. A twin gets the same shape and size and
/// the other side's colour when the colour is a side colour. Returns the
/// controls changed.
pub fn restyle(
    g: &mut Guides,
    control: &str,
    change: StyleChange,
    scope: StyleScope,
) -> Result<Vec<String>> {
    if let Some(size) = change.size {
        ensure!(size > 0.0, "control size must be positive");
    }
    if let Some(scale) = change.scale {
        ensure!(scale > 0.0, "control scale must be positive");
    }
    let built = build(g)?;
    let targets = style_targets(&built, control, scope)?;
    for (name, twin) in &targets {
        let Some(c) = built.controls.control(name) else {
            continue;
        };
        let mut style = g.control_styles.get(name).copied().unwrap_or_default();
        if let Some(shape) = change.shape {
            style.shape = Some(shape);
        }
        if let Some(size) = change.size {
            style.size = Some(size);
        }
        if let Some(scale) = change.scale {
            style.size = Some((c.size * scale).max(0.001));
        }
        if let Some(color) = change.color {
            style.color = Some(if *twin { twin_color(color) } else { color });
        }
        set_control_style(g, name, style)?;
    }
    Ok(targets.into_iter().map(|(n, _)| n).collect())
}

/// Give the controls `names` their component's default look again.
pub fn reset_styles<'a>(g: &mut Guides, names: impl IntoIterator<Item = &'a str>) -> usize {
    names
        .into_iter()
        .filter(|n| g.control_styles.remove(*n).is_some())
        .count()
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
        .filter(|c| is_premade_part(c.kind))
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

/// Set `bendy` and the default `soft` on every arm, leg and quad leg.
pub fn set_limb_options(g: &mut Guides, bendy: bool, soft: f32) {
    for c in &mut g.components {
        if matches!(c.kind, Kind::Arm | Kind::Leg | Kind::QuadLeg) {
            c.options.bendy = bendy;
            c.options.soft = soft;
        }
    }
}

/// Set the default hock `spring` on every quad leg; returns how many there are.
pub fn set_hock_spring(g: &mut Guides, spring: f32) -> usize {
    let legs = g.components.iter_mut().filter(|c| c.kind == Kind::QuadLeg);
    legs.map(|c| c.options.spring = spring).count()
}

/// Change the spine component's joint count, re-attaching components that hung
/// from the old top joint to the new one. Nothing changes when the result
/// wouldn't build.
pub fn set_spine_joints(g: &mut Guides, spine_id: &str, joints: usize) -> Result<()> {
    ensure!((1..=16).contains(&joints), "spine joints must be 1 to 16");
    change_spine(g, spine_id, |next, i| {
        let default_joints = next.rules().spine_joints;
        let spine = &mut next.components[i];
        let old_top = spine_name(spine.options.joints.unwrap_or(default_joints) - 1);
        spine.options.joints = Some(joints);
        let new_top = spine_name(joints - 1);
        for c in &mut next.components {
            if c.parent.as_deref() == Some(old_top.as_str()) {
                c.parent = Some(new_top.clone());
            }
        }
    })
}

/// Turn the spine's spline (curve) drive on or off, optionally changing its
/// number of curve controls. The bones keep their names, so everything hung
/// from the spine stays where it is. Nothing changes when the result wouldn't build.
pub fn set_spine_spline(
    g: &mut Guides,
    spine_id: &str,
    on: bool,
    controls: Option<usize>,
) -> Result<()> {
    if let Some(k) = controls {
        ensure!((2..=16).contains(&k), "curve controls must be 2 to 16");
    }
    change_spine(g, spine_id, |next, i| {
        let options = &mut next.components[i].options;
        options.spline = on;
        if controls.is_some() {
            options.controls = controls;
        }
    })
}

/// Apply `change` to a copy of the layout (with the spine's index) and keep it
/// only if it builds.
fn change_spine(
    g: &mut Guides,
    spine_id: &str,
    change: impl FnOnce(&mut Guides, usize),
) -> Result<()> {
    let i = g
        .components
        .iter()
        .position(|c| c.id == spine_id && c.kind == Kind::Spine)
        .with_context(|| format!("no spine component named {spine_id}"))?;
    let mut next = g.clone();
    change(&mut next, i);
    build(&next)?;
    *g = next;
    Ok(())
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
    /// Bones without an FK control that a control stands in for as a space: a
    /// spline spine's top joint -> its chest control.
    chest_space: HashMap<String, String>,
    component: String,
    /// The body's belly or crotch line: skin below it belongs to a leg.
    crotch_y: f32,
    rules: Rules,
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
        let parent_deforms = self.sk.bone(parent).is_some_and(|p| p.deform);
        let i = self.sk.add(name, Some(parent), head, tail, side, region);
        self.sk.bones[i].component = self.component.clone();
        self.sk.bones[i].translate = !parent_deforms;
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
            extra: Vec::new(),
            component: self.component.clone(),
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

    /// The control a part hung from `parent` (its parent control) offers as its
    /// chest space: the parent control itself, or the control standing in for
    /// a bone that has none (a spline spine's chest).
    fn chest_of(&self, parent: &ControlParent) -> Option<String> {
        match parent {
            ControlParent::Control(c) => Some(c.clone()),
            ControlParent::Bone(bone) => self.chest_space.get(bone).cloned(),
            ControlParent::None => None,
        }
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
        chest_space: HashMap::new(),
        component: String::new(),
        crotch_y: g.body.crotch_y,
        rules: g.rules(),
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
            Kind::QuadLeg => build_quad_leg(&mut b, c, parent)?,
            Kind::Chain => build_chain(&mut b, c, parent)?,
            Kind::Spline => build_spline(&mut b, c, parent)?,
            Kind::Eyes => build_eyes(&mut b, c, parent)?,
            Kind::Jaw => build_jaw(&mut b, p("jaw"), p("chin"), parent)?,
            Kind::Face => build_face(&mut b, c, parent)?,
        }
    }
    // The face board goes on last: it drives the jaw and the face, whatever
    // order they were added in.
    if let Some(face) = g
        .components
        .iter()
        .find(|c| c.kind == Kind::Face && c.options.board)
    {
        b.component = face.id.clone();
        build_board(&mut b, face)?;
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
        if c.kind == Kind::Spline {
            ensure!(
                (2..=64).contains(&spline_joints(c)),
                "component {}: a spline chain needs 2 to 64 joints",
                c.id
            );
        }
        if c.kind == Kind::Spine && c.options.spline {
            ensure!(
                c.options.joints.unwrap_or(SPINE_JOINTS) >= 2,
                "component {}: a spline spine needs at least 2 joints",
                c.id
            );
        }
        if c.kind == Kind::Spline || (c.kind == Kind::Spine && c.options.spline) {
            ensure!(
                (2..=16).contains(&c.options.curve_controls(c.kind == Kind::Spine)),
                "component {}: a spline needs 2 to 16 curve controls",
                c.id
            );
        }
        if matches!(c.kind, Kind::Arm | Kind::Leg | Kind::QuadLeg) {
            ensure!(
                c.side != Side::Center,
                "component {}: arms and legs need a side (left or right)",
                c.id
            );
        }
        let limb = matches!(c.kind, Kind::Arm | Kind::Leg | Kind::QuadLeg);
        ensure!(
            !c.options.bendy || limb,
            "component {}: only arms and legs can be bendy",
            c.id
        );
        ensure!(
            c.options.soft == 0.0 || limb,
            "component {}: only arms and legs have soft IK",
            c.id
        );
        ensure!(
            (0.0..=1.0).contains(&c.options.soft),
            "component {}: soft must be 0 to 1 (got {})",
            c.id,
            c.options.soft
        );
        ensure!(
            c.options.spring == 0.0 || c.kind == Kind::QuadLeg,
            "component {}: only quad legs have a spring hock",
            c.id
        );
        ensure!(
            (0.0..=1.0).contains(&c.options.spring),
            "component {}: spring must be 0 to 1 (got {})",
            c.id,
            c.options.spring
        );
        ensure!(
            !c.options.board || c.kind == Kind::Face,
            "component {}: only the face has a face board",
            c.id
        );
        if c.kind == Kind::Face {
            ensure!(
                c.options.joints.unwrap_or(0) <= MAX_LIP_JOINTS,
                "component {}: a face takes 0 to {MAX_LIP_JOINTS} extra lip joints a side",
                c.id
            );
        }
        if c.kind == Kind::Spine {
            let n = g.spine_joints(c);
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
    let joints = c.options.joints.unwrap_or(b.rules.spine_joints);
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
    if c.options.spline {
        // The hips control drives the pelvis and starts the curve; the chest
        // control ends it and turns the top joint.
        b.control(
            "hips_ctrl".into(),
            ControlKind::Fk,
            ControlParent::Control("cog_ctrl".into()),
            Some("pelvis"),
            pelvis,
            Vec3::Y,
            Shape::Circle,
            0.14 * h,
            CENTER,
        );
        let k = c.options.curve_controls(true);
        let mut curve = vec!["hips_ctrl".to_string()];
        curve.extend((1..k - 1).map(|i| {
            if k == 3 {
                "spine_mid_ctrl".to_string()
            } else {
                format!("spine_mid_{i:02}_ctrl")
            }
        }));
        curve.push("chest_ctrl".into());
        let fk: Vec<String> = (1..k).map(|i| format!("spine_fk_{i:02}_ctrl")).collect();
        let bones: Vec<String> = (0..joints).map(spine_name).collect();
        spline_controls(
            b,
            Spline {
                name: &c.id,
                bones: &bones,
                curve: &curve,
                fk: &fk,
                parent: ControlParent::Control("cog_ctrl".into()),
                curve_size: 0.12 * h,
                fk_size: 0.15 * h,
                color: CENTER,
            },
        );
        b.chest_space
            .insert(spine_name(joints - 1), "chest_ctrl".into());
        return Ok(());
    }
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
    b.head_chest = b.chest_of(&parent_ctrl);
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
    let p = |n: &str| c.position(n).unwrap();
    // A quadruped's front leg: elbow and wrist below the belly line. Its skin
    // falls in the leg region, it starts in IK and its IK control stands upright.
    let planted =
        b.rules.planted_front_legs && p("elbow").y < b.crotch_y && p("wrist").y < b.crotch_y;
    let region = limb_region(if planted { Kind::Leg } else { Kind::Arm }, side);
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
    let bend = limb_bones(b, &names, &clav, &points, side, region, &c.options)?;

    let color = side_color(side);
    let clav_parent = b.parent_control(parent);
    if let Some(chest) = b.chest_of(&clav_parent) {
        b.arm_chest.insert(format!("arm{s}"), chest);
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
        Limb {
            blend: if planted { 1.0 } else { 0.0 },
            upright_ik: planted,
            soft: c.options.soft,
            bend,
            ..Limb::default()
        },
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
    let bend = limb_bones(b, &names, parent, &points, side, region, &c.options)?;
    let ball = format!("ball{s}");
    b.bone(&ball, &names[2], p("ball"), p("toe"), side, region)?;

    let fk_parent = b.parent_control(parent);
    let foot = FootRoll {
        heel: p("heel"),
        ball: p("ball"),
        toe_tip: p("toe"),
        ball_bone: ball.clone(),
        ball_control: ctrl_name(&ball),
        break_angle: b.rules.foot_break_degrees,
    };
    limb_controls(
        b,
        "leg",
        side,
        &names,
        fk_parent,
        Limb {
            blend: 1.0,
            upright_ik: true,
            foot: Some(foot),
            soft: c.options.soft,
            bend,
            ..Limb::default()
        },
    );
    toe_control(b, side, &names[2], ball);
    Ok(())
}

/// The toe (ball) control: it follows the foot bone in both IK and FK.
fn toe_control(b: &mut Builder, side: Side, foot: &str, ball: String) {
    let bb = b.sk.bone(&ball).unwrap().clone();
    b.control(
        ctrl_name(&ball),
        ControlKind::Fk,
        ControlParent::Bone(foot.to_string()),
        Some(&ball),
        bb.head,
        bb.direction(),
        Shape::Circle,
        0.035 * b.h,
        side_color(side),
    );
    b.blended.insert(ball);
}

/// A three-segment hind leg: thigh, calf and hock (twist helpers on the thigh
/// and calf), then the paw (`foot`) and toes (`ball`). The thigh and calf solve
/// as a two-bone IK aimed at the hock, which a hock control at the ankle swings;
/// the paw keeps the biped leg's reverse-foot roll. Bendy, the thigh and calf
/// get bend joints and the knee a pin, as a leg's do; `soft` eases the reach to
/// the hock and `spring` lets the hock swing with the thigh.
fn build_quad_leg(b: &mut Builder, c: &Component, parent: &str) -> Result<()> {
    let side = c.side;
    let s = side.suffix();
    let region = limb_region(Kind::Leg, side);
    let p = |n: &str| c.position(n).unwrap();
    b.link(parent, region);
    let segments = [format!("thigh{s}"), format!("calf{s}"), format!("hock{s}")];
    let points = [p("hip"), p("knee"), p("hock"), p("ankle")];
    let bend = limb_bones(b, &segments, parent, &points, side, region, &c.options)?;
    let foot = format!("foot{s}");
    b.bone(&foot, &segments[2], p("ankle"), p("ball"), side, region)?;
    let ball = format!("ball{s}");
    b.bone(&ball, &foot, p("ball"), p("toe"), side, region)?;

    let fk_parent = b.parent_control(parent);
    let roll = FootRoll {
        heel: p("heel"),
        ball: p("ball"),
        toe_tip: p("toe"),
        ball_bone: ball.clone(),
        ball_control: ctrl_name(&ball),
        break_angle: b.rules.foot_break_degrees,
    };
    let names = [segments[0].clone(), segments[1].clone(), foot.clone()];
    limb_controls(
        b,
        "leg",
        side,
        &names,
        fk_parent,
        Limb {
            blend: 1.0,
            upright_ik: true,
            foot: Some(roll),
            hock: Some(segments[2].clone()),
            soft: c.options.soft,
            spring: c.options.spring,
            bend,
        },
    );
    toe_control(b, side, &foot, ball);
    Ok(())
}

/// Upper, lower and end bones through `points`, plus twist helpers on the far half
/// of the upper and lower segments (mannequin names). Upper twists counter half of
/// their own twist so the shoulder and hip don't candy-wrap; lower twists carry
/// half of the hand or foot twist.
///
/// A bendy limb instead gets [`BEND_JOINTS`] joints along the whole of each
/// segment, which take its skin (the segment bone stops deforming). Their twist
/// shares grow toward the far end, up to the share the single helper has.
/// Returns the bend joints of the upper and lower segments.
fn limb_bones(
    b: &mut Builder,
    names: &[String; 3],
    parent: &str,
    points: &[Vec3; 4],
    side: Side,
    region: Region,
    options: &Options,
) -> Result<Option<[Vec<String>; 2]>> {
    let mut prev = parent.to_string();
    for (i, name) in names.iter().enumerate() {
        b.bone(name, &prev, points[i], points[i + 1], side, region)?;
        prev = name.clone();
    }
    // The upper bone also blends into the torso.
    let upper = b.sk.index_of(&names[0]).unwrap();
    b.sk.bones[upper].links.push(Region::Center);
    if options.bendy {
        let t = |i: usize| i as f32 / BEND_JOINTS as f32;
        let mut out: [Vec<String>; 2] = Default::default();
        for (k, (seg, factor, source)) in
            [(&names[0], -0.5, &names[0]), (&names[1], 0.5, &names[2])]
                .into_iter()
                .enumerate()
        {
            let si = b.sk.index_of(seg).unwrap();
            let bone = b.sk.bones[si].clone();
            for n in 0..BEND_JOINTS {
                let name = twist_joint_name(seg, n + 1);
                let i = b.bone(
                    &name,
                    seg,
                    bone.head.lerp(bone.tail, t(n)),
                    bone.head.lerp(bone.tail, t(n + 1)),
                    side,
                    region,
                )?;
                // Each joint blends across the same regions as its segment (the
                // upper one into the torso).
                b.sk.bones[i].links = bone.links.clone();
                if options.twist {
                    b.sk.bones[i].twist = Some(Twist {
                        source: source.clone(),
                        factor: factor * t(n + 1),
                    });
                }
                out[k].push(name);
            }
            // After its joints, so they aren't built as hanging from a helper.
            b.sk.bones[si].deform = false;
        }
        return Ok(Some(out));
    }
    if options.twist {
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
    Ok(None)
}

/// How a limb's IK is set up.
#[derive(Default)]
struct Limb {
    /// Initial IK/FK blend: 1 = IK.
    blend: f32,
    /// The IK control stands upright (feet, paws) instead of following the upper bone.
    upright_ik: bool,
    foot: Option<FootRoll>,
    /// The hock bone of a three-segment leg, between the lower and end bones.
    hock: Option<String>,
    /// Default of the `soft` attribute.
    soft: f32,
    /// Default of the hock's `spring` attribute (three-segment legs).
    spring: f32,
    /// Bend joints of the upper and lower segments, for a bendy limb.
    bend: Option<[Vec<String>; 2]>,
}

fn limb_controls(
    b: &mut Builder,
    kind: &str,
    side: Side,
    names: &[String; 3],
    fk_parent: ControlParent,
    limb: Limb,
) {
    let h = b.h;
    let color = side_color(side);
    let s = side.suffix();
    let [up, lo, end] = names.clone().map(|n| b.sk.bone(&n).unwrap().clone());
    let hock = limb.hock.as_ref().map(|n| b.sk.bone(n).unwrap().clone());
    let (size, bend_fallback) = if kind == "arm" {
        (0.045 * h, -Vec3::Z)
    } else {
        (0.06 * h, Vec3::Z)
    };

    let mut parent = fk_parent;
    for bone in [Some(&up), Some(&lo), hock.as_ref(), Some(&end)]
        .into_iter()
        .flatten()
    {
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
    let ik_axis = if limb.upright_ik {
        Vec3::Y
    } else {
        up.direction()
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
    // in the guides doesn't swing it out to the side. On a three-segment leg the
    // two-bone solve ends at the hock, so that is the limb it squares to.
    let solve_end = hock.as_ref().map_or(end.head, |k| k.head);
    let axis = (solve_end - up.head).try_normalize().unwrap_or(Vec3::NEG_Y);
    let bend = (bend_fallback - axis * bend_fallback.dot(axis))
        .try_normalize()
        .unwrap_or(bend_fallback);
    let reach = b.rules.pole_reach * (up.head.distance(lo.head) + lo.head.distance(solve_end));
    let pole = format!("{}_pole_ctrl", lo.name);
    b.control(
        pole.clone(),
        ControlKind::Pole,
        root,
        None,
        lo.head + bend * reach.max(b.rules.pole_min_reach * h),
        bend,
        Shape::Diamond,
        0.025 * h,
        color,
    );

    // The hock control stands at the ankle and points up the hock: turning it
    // swings the hock about the ankle, which moves where the knee solve aims.
    let hock_segment = hock.map(|k| {
        let control = format!("{}_ik_ctrl", k.name);
        b.control(
            control.clone(),
            ControlKind::Hock,
            ControlParent::Control(ik.clone()),
            None,
            k.tail,
            (k.head - k.tail).try_normalize().unwrap_or(Vec3::Y),
            Shape::Diamond,
            0.03 * h,
            color,
        );
        HockSegment {
            fk_control: fk_name(&k.name),
            bone: k.name,
            control,
            default_spring: limb.spring,
        }
    });

    // Bendy: a pin at the elbow or knee, following the lower bone, and a bend
    // control at the middle of each segment, following its segment.
    let bendy = limb.bend.map(|[upper_joints, lower_joints]| {
        let pin = format!("{}_pin_ctrl", lo.name);
        b.control(
            pin.clone(),
            ControlKind::Pin,
            ControlParent::Bone(lo.name.clone()),
            None,
            lo.head,
            lo.direction(),
            Shape::Sphere,
            0.025 * h,
            color,
        );
        let mut segment = |bone: &crate::skeleton::Bone, joints: Vec<String>| {
            let control = format!("{}_bend_ctrl", bone.name);
            b.control(
                control.clone(),
                ControlKind::Bend,
                ControlParent::Bone(bone.name.clone()),
                None,
                bone.head.lerp(bone.tail, 0.5),
                bone.direction(),
                Shape::Octagon,
                0.8 * size,
                color,
            );
            BendSegment {
                bone: bone.name.clone(),
                control,
                joints,
            }
        };
        Bendy {
            pin,
            upper: segment(&up, upper_joints),
            lower: segment(&lo, lower_joints),
        }
    });

    b.rig.ik_chains.push(IkChain {
        name: format!("{kind}{s}"),
        upper: up.name.clone(),
        lower: lo.name.clone(),
        end: end.name.clone(),
        ik_control: ik,
        pole_control: pole,
        fk_controls: [fk_name(&up.name), fk_name(&lo.name), fk_name(&end.name)],
        default_blend: limb.blend,
        default_stretch: 0.0,
        foot: limb.foot,
        hock: hock_segment,
        default_soft: limb.soft,
        soft_zone: SOFT_ZONE,
        bendy,
    });
}

fn build_chain(b: &mut Builder, c: &Component, parent: &str) -> Result<()> {
    let region =
        b.sk.bone(parent)
            .map(|p| p.region)
            .unwrap_or(Region::Center);
    let (names, _) = id_names(c);
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

/// A tentacle: joints spread evenly along the guides (one per guide segment
/// unless the options set a count), driven by curve controls with an FK layer.
fn build_spline(b: &mut Builder, c: &Component, parent: &str) -> Result<()> {
    let region =
        b.sk.bone(parent)
            .map(|p| p.region)
            .unwrap_or(Region::Center);
    let (names, controls) = id_names(c);
    let guides: Vec<Vec3> = c.guides.iter().map(|g| g.position).collect();
    let points = if names.len() + 1 == guides.len() {
        guides
    } else {
        resample(&guides, names.len() + 1)
    };
    let mut prev = parent.to_string();
    for (i, name) in names.iter().enumerate() {
        b.bone(name, &prev, points[i], points[i + 1], c.side, region)?;
        prev = name.clone();
    }
    let k = c.options.curve_controls(false);
    let (curve, fk) = controls.split_at(k);
    let length: f32 = points.windows(2).map(|w| w[0].distance(w[1])).sum();
    let size = (length / k as f32 * 0.3).clamp(0.01 * b.h, 0.07 * b.h);
    let parent_ctrl = b.parent_control(parent);
    spline_controls(
        b,
        Spline {
            name: &c.id,
            bones: &names,
            curve,
            fk,
            parent: parent_ctrl,
            curve_size: size,
            fk_size: size * 1.5,
            color: side_color(c.side),
        },
    );
    Ok(())
}

/// What `spline_controls` builds on a chain of bones already in the skeleton.
struct Spline<'a> {
    name: &'a str,
    bones: &'a [String],
    /// Curve control names, start to end. A first name that is already a
    /// control (the spine's hips) is used as it is.
    curve: &'a [String],
    /// FK layer names, one fewer than the curve controls.
    fk: &'a [String],
    /// Where the start control and the FK layer hang.
    parent: ControlParent,
    curve_size: f32,
    fk_size: f32,
    color: [f32; 3],
}

/// Curve controls spread along a chain (near the peaks of their curve
/// weights), an FK layer carrying them, and the chain's [`SplineChain`].
fn spline_controls(b: &mut Builder, s: Spline) {
    let bones: Vec<_> = s
        .bones
        .iter()
        .map(|n| b.sk.bone(n).unwrap().clone())
        .collect();
    // The curve runs through the joints' heads; the last joint hangs off its end.
    let points: Vec<Vec3> = bones.iter().map(|bone| bone.head).collect();
    let params = arc_params(&points);
    let k = s.curve.len();
    let spots: Vec<(Vec3, Vec3)> = greville(k)
        .into_iter()
        .map(|t| {
            let i = segment_at(&params, t);
            let span = (params[i + 1] - params[i]).max(1e-9);
            let f = ((t - params[i]) / span).clamp(0.0, 1.0);
            let dir = (points[i + 1] - points[i]).try_normalize();
            (points[i].lerp(points[i + 1], f), dir.unwrap_or(Vec3::Y))
        })
        .collect();
    if !b.has_control(&s.curve[0]) {
        b.control(
            s.curve[0].clone(),
            ControlKind::Spline,
            s.parent.clone(),
            None,
            spots[0].0,
            spots[0].1,
            Shape::Square,
            s.curve_size,
            s.color,
        );
    }
    let mut parent = s.parent;
    for i in 0..k - 1 {
        b.control(
            s.fk[i].clone(),
            ControlKind::Fk,
            parent,
            None,
            spots[i].0,
            spots[i].1,
            Shape::Circle,
            s.fk_size,
            s.color,
        );
        parent = ControlParent::Control(s.fk[i].clone());
        b.control(
            s.curve[i + 1].clone(),
            ControlKind::Spline,
            parent.clone(),
            None,
            spots[i + 1].0,
            spots[i + 1].1,
            Shape::Square,
            s.curve_size,
            s.color,
        );
    }
    let weights = params.iter().map(|&u| spline_basis(k, u)).collect();
    b.rig.splines.push(SplineChain {
        name: s.name.to_string(),
        bones: s.bones.to_vec(),
        controls: s.curve.to_vec(),
        fk_controls: s.fk.to_vec(),
        settings: s.curve[k - 1].clone(),
        points,
        params,
        weights,
        default_stretch: 1.0,
        default_volume: 0.0,
    });
}

/// Each point's distance along the polyline through them, as a fraction of its length.
fn arc_params(points: &[Vec3]) -> Vec<f32> {
    let mut acc = vec![0.0f32];
    for w in points.windows(2) {
        acc.push(acc.last().unwrap() + w[0].distance(w[1]));
    }
    let total = acc.last().copied().unwrap_or(0.0).max(1e-9);
    acc.into_iter().map(|d| d / total).collect()
}

/// The polyline segment holding fraction `t` of its length.
fn segment_at(params: &[f32], t: f32) -> usize {
    params
        .windows(2)
        .position(|w| t <= w[1])
        .unwrap_or(params.len() - 2)
}

/// `count` points evenly spaced along the polyline through `points`.
fn resample(points: &[Vec3], count: usize) -> Vec<Vec3> {
    let params = arc_params(points);
    (0..count)
        .map(|j| {
            let t = j as f32 / (count - 1).max(1) as f32;
            let i = segment_at(&params, t);
            let span = (params[i + 1] - params[i]).max(1e-9);
            points[i].lerp(points[i + 1], ((t - params[i]) / span).clamp(0.0, 1.0))
        })
        .collect()
}

/// Knots of a clamped uniform B-spline with `count` control points, and its degree.
fn spline_knots(count: usize) -> (Vec<f32>, usize) {
    let degree = (count - 1).min(3);
    let inner = count - degree - 1;
    let mut knots = vec![0.0; degree + 1];
    knots.extend((1..=inner).map(|i| i as f32 / (inner + 1) as f32));
    knots.extend(std::iter::repeat_n(1.0, degree + 1));
    (knots, degree)
}

/// The weight of each of `count` curve controls at `u` (0 to 1 along the
/// chain): the basis of a clamped uniform B-spline of degree up to 3. The
/// weights sum to 1, change smoothly, and the end controls own the ends.
pub fn spline_basis(count: usize, u: f32) -> Vec<f32> {
    let (knots, degree) = spline_knots(count);
    let u = u.clamp(0.0, 1.0);
    let mut n: Vec<f32> = (0..knots.len() - 1)
        .map(|i| {
            let inside = if u >= 1.0 {
                i == count - 1
            } else {
                knots[i] <= u && u < knots[i + 1]
            };
            if inside { 1.0 } else { 0.0 }
        })
        .collect();
    let ratio = |num: f32, den: f32| if den > 0.0 { num / den } else { 0.0 };
    for p in 1..=degree {
        n = (0..n.len() - 1)
            .map(|i| {
                ratio(u - knots[i], knots[i + p] - knots[i]) * n[i]
                    + ratio(knots[i + p + 1] - u, knots[i + p + 1] - knots[i + 1]) * n[i + 1]
            })
            .collect();
    }
    n
}

/// Where each curve control sits along the chain (0 to 1): the Greville
/// abscissae of the B-spline, near the peak of each control's weight.
fn greville(count: usize) -> Vec<f32> {
    let (knots, degree) = spline_knots(count);
    (0..count)
        .map(|i| knots[i + 1..=i + degree].iter().sum::<f32>() / degree as f32)
        .collect()
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
    let master = id_names(c).1.remove(0);
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
    let extras: Vec<Guide> = lip_joints(c)
        .into_iter()
        .map(|j| Guide {
            name: j.name,
            position: j.position,
        })
        .collect();
    for g in c.guides.iter().chain(&extras) {
        let name = g.name.as_str();
        let side = if name.ends_with("_l") {
            Side::Left
        } else if name.ends_with("_r") {
            Side::Right
        } else {
            Side::Center
        };
        let bone_parent = if name.starts_with("lip_low") && b.sk.bone("jaw").is_some() {
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

/// The most extra lip joints a face takes on each lip, each side.
pub const MAX_LIP_JOINTS: usize = 3;

/// An extra lip joint of a face: between the middle of a lip and a corner.
#[derive(Debug, Clone, PartialEq)]
pub struct LipJoint {
    /// `lip_up_1_l`: the upper lip's first joint out from the middle, on the left.
    pub name: String,
    pub position: Vec3,
    pub upper: bool,
    /// How far out toward the corner: 0 in the middle, 1 at the corner.
    pub out: f32,
    pub side: Side,
}

/// The face's extra lip joints (its `joints` option, none when unset): on each
/// lip, evenly from the middle toward each corner, on an arc that bows forward
/// like the mouth does.
pub fn lip_joints(c: &Component) -> Vec<LipJoint> {
    let n = c.options.joints.unwrap_or(0).min(MAX_LIP_JOINTS);
    let mut out = Vec::new();
    for (lip, upper) in [("up", true), ("low", false)] {
        let Some(mid) = c.position(&format!("lip_{lip}")) else {
            continue;
        };
        for (side, s) in [(Side::Left, "l"), (Side::Right, "r")] {
            let Some(corner) = c.position(&format!("lip_corner_{s}")) else {
                continue;
            };
            for k in 1..=n {
                let t = k as f32 / (n + 1) as f32;
                let mut p = mid.lerp(corner, t);
                p.z = corner.z + (mid.z - corner.z) * (1.0 - t * t);
                out.push(LipJoint {
                    name: format!("lip_{lip}_{k}_{s}"),
                    position: p,
                    upper,
                    out: t,
                    side,
                });
            }
        }
    }
    out
}

/// The face board's offsets as they are made: each face control's drives.
struct Drives<'a> {
    b: &'a Builder,
    offsets: Vec<face_board::BoardOffset>,
}

impl Drives<'_> {
    /// `bone`'s control moves `t` metres and turns `r` degrees for a unit of
    /// `part` going `dir`. Skipped when the rig has no such control.
    fn add(&mut self, bone: &str, part: &str, dir: Dir, t: Vec3, r: Vec3) {
        let control = ctrl_name(bone);
        let Some(head) = self.b.sk.bone(bone).map(|x| x.head) else {
            return;
        };
        if !self.b.has_control(&control) || (t == Vec3::ZERO && r == Vec3::ZERO) {
            return;
        }
        let i = match self.offsets.iter().position(|o| o.control == control) {
            Some(i) => i,
            None => {
                self.offsets.push(face_board::BoardOffset {
                    control,
                    pivot: head,
                    drives: Vec::new(),
                });
                self.offsets.len() - 1
            }
        };
        let drives = &mut self.offsets[i].drives;
        match drives.iter_mut().find(|d| d.part == part && d.dir == dir) {
            Some(d) => {
                d.translate += t;
                d.rotate += r;
            }
            None => drives.push(face_board::BoardDrive {
                part: part.to_string(),
                dir,
                translate: t,
                rotate: r,
            }),
        }
    }
}

/// The face board: a panel beside the head (on the character's left, level
/// with the face) whose sliders move and turn the face controls, and the lip
/// seal on the lip joints. See [`face_board`].
fn build_board(b: &mut Builder, c: &Component) -> Result<()> {
    use face_board::{HANDLE, LAYOUT, PANEL, STEP};
    let h = b.h;
    let step = STEP * h;
    let parent = c.parent.as_deref().unwrap_or("head");
    let (mut lo, mut hi) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
    for g in &c.guides {
        lo = lo.min(g.position);
        hi = hi.max(g.position);
    }
    ensure!(
        lo.x <= hi.x,
        "component {}: the face has no guides to place its board by",
        c.id
    );
    let center = Vec3::new(hi.x + 0.05 * h + PANEL[0] * step, (lo.y + hi.y) * 0.5, hi.z);
    let at = |p: [f32; 2]| center + Vec3::new(p[0], p[1], 0.0) * step;
    let frame = "board_ctrl".to_string();
    let top = LAYOUT
        .iter()
        .find(|s| s.part.is_empty())
        .map_or([0.0, PANEL[1]], |s| s.at);
    let parent_ctrl = b.parent_control(parent);
    b.control(
        frame.clone(),
        ControlKind::Board,
        parent_ctrl,
        None,
        at(top),
        Vec3::Z,
        Shape::FourArrows,
        0.8 * step,
        CENTER,
    );
    if let Some(ctrl) = b.rig.controls.last_mut() {
        ctrl.extra = face_board::panel_lines(center, step);
    }
    let mut sliders = Vec::new();
    for slot in LAYOUT.iter().filter(|s| !s.part.is_empty()) {
        let name = format!("board_{}_ctrl", slot.part);
        let side = if slot.part.ends_with("_l") {
            Side::Left
        } else if slot.part.ends_with("_r") {
            Side::Right
        } else {
            Side::Center
        };
        b.control(
            name.clone(),
            ControlKind::Slider,
            ControlParent::Control(frame.clone()),
            None,
            at(slot.at),
            Vec3::Z,
            Shape::Circle,
            HANDLE * step,
            side_color(side),
        );
        sliders.push(face_board::Slider {
            part: slot.part.to_string(),
            control: name,
            travel: slot.travel * step,
            x: slot.x,
            y: slot.y,
        });
    }

    let v = |x: f32, y: f32, z: f32| Vec3::new(x, y, z) * h;
    let turn = |x: f32| Vec3::new(x, 0.0, 0.0);
    let lips = lip_joints(c);
    let mut d = Drives {
        b,
        offsets: Vec::new(),
    };
    for (s, sx) in [("l", 1.0f32), ("r", -1.0)] {
        // The right side mirrors the left: side to side swaps.
        let m = |t: Vec3| Vec3::new(t.x * sx, t.y, t.z);
        let side_dir = |dir: Dir| if sx < 0.0 { dir.mirrored() } else { dir };
        let brow = format!("brow_{s}");
        let brows = [
            (
                "in",
                v(0.0, 0.010, 0.001),
                v(0.0, -0.006, 0.001),
                v(-0.005, -0.002, 0.001),
                Vec3::ZERO,
            ),
            (
                "mid",
                v(0.0, 0.011, 0.001),
                v(0.0, -0.006, 0.0),
                v(-0.003, -0.001, 0.0),
                v(0.002, 0.0, 0.0),
            ),
            (
                "out",
                v(0.0, 0.008, 0.0),
                v(0.0, -0.005, 0.0),
                Vec3::ZERO,
                v(0.003, 0.001, 0.0),
            ),
        ];
        for (bone, up, down, inward, outward) in brows {
            let bone = format!("brow_{bone}_{s}");
            d.add(&bone, &brow, Dir::YPos, up, Vec3::ZERO);
            d.add(&bone, &brow, Dir::YNeg, down, Vec3::ZERO);
            d.add(&bone, &brow, side_dir(Dir::XNeg), m(inward), Vec3::ZERO);
            d.add(&bone, &brow, side_dir(Dir::XPos), m(outward), Vec3::ZERO);
        }
        // Lids: turned about the eye to meet a little below the middle of
        // the eye when they pivot on it, else slid shut.
        let lid = format!("lid_{s}");
        let (up, low) = (format!("lid_up_{s}"), format!("lid_low_{s}"));
        let lid_bones = (d.b.sk.bone(&up).cloned(), d.b.sk.bone(&low).cloned());
        if let (Some(ub), Some(lb)) = lid_bones {
            let on_eye = c
                .position(&up)
                .is_some_and(|g| g.distance(ub.head) > 1e-4 * h);
            if on_eye {
                let angle = |p: Vec3, eye: Vec3| (p.y - eye.y).atan2(p.z - eye.z).to_degrees();
                let (au, al) = (angle(ub.tail, ub.head), angle(lb.tail, lb.head));
                let meet = al + 0.3 * (au - al);
                d.add(&up, &lid, Dir::YNeg, Vec3::ZERO, turn(au - meet));
                d.add(&low, &lid, Dir::YNeg, Vec3::ZERO, turn(al - meet));
                d.add(&up, &lid, Dir::YPos, Vec3::ZERO, turn(-15.0));
                d.add(&low, &lid, Dir::YPos, Vec3::ZERO, turn(8.0));
            } else {
                let gap = (ub.head.y - lb.head.y).max(0.002 * h);
                let y = |k: f32| Vec3::new(0.0, k * gap, 0.0);
                d.add(&up, &lid, Dir::YNeg, y(-0.7), Vec3::ZERO);
                d.add(&low, &lid, Dir::YNeg, y(0.3), Vec3::ZERO);
                d.add(&up, &lid, Dir::YPos, y(0.3), Vec3::ZERO);
                d.add(&low, &lid, Dir::YPos, y(-0.2), Vec3::ZERO);
            }
        }
        let cheek = format!("cheek_{s}");
        d.add(
            &cheek,
            &cheek,
            Dir::YPos,
            m(v(0.001, 0.006, 0.002)),
            Vec3::ZERO,
        );
        d.add(&cheek, &cheek, Dir::YNeg, v(0.0, -0.003, 0.0), Vec3::ZERO);
        // Mouth corners, with the lip joints on that side taking a share by
        // how far out toward the corner they are.
        let corner = format!("corner_{s}");
        let suffix = format!("_{s}");
        let mut on_side: Vec<(String, f32)> = vec![(format!("lip_corner_{s}"), 1.0)];
        on_side.extend(
            lips.iter()
                .filter(|j| j.side.suffix() == suffix)
                .map(|j| (j.name.clone(), j.out * j.out)),
        );
        for (bone, k) in &on_side {
            let k = *k;
            d.add(
                bone,
                &corner,
                Dir::YPos,
                m(v(0.002, 0.007, -0.002)) * k,
                Vec3::ZERO,
            );
            d.add(
                bone,
                &corner,
                Dir::YNeg,
                v(0.0, -0.006, 0.0) * k,
                Vec3::ZERO,
            );
            let wide = m(v(0.007, 0.0, -0.002)) * k;
            d.add(bone, &corner, side_dir(Dir::XPos), wide, Vec3::ZERO);
            let narrow = m(v(-0.006, 0.0, 0.003)) * k;
            d.add(bone, &corner, side_dir(Dir::XNeg), narrow, Vec3::ZERO);
        }
        d.add(&cheek, &corner, Dir::YPos, v(0.0, 0.003, 0.001), Vec3::ZERO);
    }
    d.add("nose", "nose", Dir::YPos, v(0.0, 0.004, 0.001), Vec3::ZERO);
    d.add("nose", "nose", Dir::YNeg, v(0.0, -0.002, 0.0), Vec3::ZERO);
    let mut every_lip: Vec<String> = ["lip_up", "lip_low", "lip_corner_l", "lip_corner_r"]
        .map(String::from)
        .to_vec();
    every_lip.extend(lips.iter().map(|j| j.name.clone()));
    for bone in &every_lip {
        d.add(bone, "mouth", Dir::XPos, v(0.006, 0.0, 0.0), Vec3::ZERO);
        d.add(bone, "mouth", Dir::XNeg, v(-0.006, 0.0, 0.0), Vec3::ZERO);
        d.add(bone, "mouth", Dir::YPos, v(0.0, 0.005, 0.0), Vec3::ZERO);
        d.add(bone, "mouth", Dir::YNeg, v(0.0, -0.005, 0.0), Vec3::ZERO);
    }
    for (lip, upper, raise, lower) in [
        (
            "lip_up",
            true,
            v(0.0, 0.004, 0.0005),
            v(0.0, -0.003, -0.001),
        ),
        (
            "lip_low",
            false,
            v(0.0, 0.003, -0.001),
            v(0.0, -0.004, 0.0005),
        ),
    ] {
        let mut bones = vec![(lip.to_string(), 1.0)];
        bones.extend(
            lips.iter()
                .filter(|j| j.upper == upper)
                .map(|j| (j.name.clone(), 1.0 - j.out)),
        );
        for (bone, k) in &bones {
            d.add(bone, lip, Dir::YPos, raise * *k, Vec3::ZERO);
            d.add(bone, lip, Dir::YNeg, lower * *k, Vec3::ZERO);
        }
    }
    d.add("jaw", "jaw", Dir::YNeg, Vec3::ZERO, turn(25.0));
    d.add("jaw", "jaw", Dir::YPos, Vec3::ZERO, turn(-3.0));
    d.add(
        "jaw",
        "jaw",
        Dir::XPos,
        Vec3::ZERO,
        Vec3::new(0.0, 8.0, 0.0),
    );
    d.add(
        "jaw",
        "jaw",
        Dir::XNeg,
        Vec3::ZERO,
        Vec3::new(0.0, -8.0, 0.0),
    );
    let offsets = d.offsets;

    // The lip seal: each upper lip joint with the lower one across from it.
    let head = |name: &str| b.sk.bone(name).map(|x| x.head);
    let (Some(left), Some(right)) = (head("lip_corner_l"), head("lip_corner_r")) else {
        bail!("component {}: the face has no lip corners", c.id);
    };
    let width = (left.x - right.x).abs().max(1e-4 * h);
    let mut pairs = vec![("lip_up".to_string(), "lip_low".to_string())];
    for j in lips.iter().filter(|j| j.upper) {
        pairs.push((j.name.clone(), j.name.replacen("lip_up", "lip_low", 1)));
    }
    let pairs = pairs
        .into_iter()
        .filter_map(|(u, l)| {
            let (hu, hl) = (head(&u)?, head(&l)?);
            Some(face_board::LipPair {
                upper_control: ctrl_name(&u),
                lower_control: ctrl_name(&l),
                along: ((left.x - hu.x) / width).clamp(0.0, 1.0),
                rest_gap: hu.distance(hl),
                upper: u,
                lower: l,
            })
        })
        .collect();
    b.rig.face_board = Some(FaceBoard {
        frame,
        sliders,
        offsets,
        seal: Some(face_board::LipSeal {
            frame_bone: parent.to_string(),
            release: 0.3 * width,
            pairs,
        }),
    });
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
        // Options left at their defaults stay out of the file.
        assert!(!text.contains("bendy") && !text.contains("soft"));
        let mut g = demo_guides();
        set_limb_options(&mut g, true, 0.3);
        let back: Guides = serde_json::from_str(&serde_json::to_string(&g).unwrap()).unwrap();
        let arm = back.component("arm_l").unwrap();
        assert!(arm.options.bendy && arm.options.soft == 0.3);
        assert!(!back.component("spine").unwrap().options.bendy);
    }

    #[test]
    fn bendy_limbs_arc_their_joints_from_bend_and_pin_controls() {
        let mut g = demo_guides();
        set_limb_options(&mut g, true, 0.4);
        let built = build(&g).unwrap();
        assert_controls_consistent(&built);
        let (sk, rig) = (&built.skeleton, &built.controls);
        // Each limb trades its two twist helpers for BEND_JOINTS per segment.
        let plain = build(&demo_guides()).unwrap().skeleton.bones.len();
        assert_eq!(sk.bones.len(), plain + 4 * 2 * (BEND_JOINTS - 1));
        assert_eq!(rig.ik_chains.len(), 4);
        for chain in &rig.ik_chains {
            assert_eq!(chain.default_soft, 0.4);
            let bendy = chain.bendy.as_ref().expect(&chain.name);
            let pin = rig.control(&bendy.pin).unwrap();
            assert_eq!(pin.kind, ControlKind::Pin);
            assert_eq!(pin.parent, ControlParent::Bone(chain.lower.clone()));
            assert_eq!(pin.position, sk.bone(&chain.lower).unwrap().head);
            for (seg, bone) in [(&bendy.upper, &chain.upper), (&bendy.lower, &chain.lower)] {
                assert_eq!(&seg.bone, bone);
                let segment = sk.bone(bone).unwrap();
                assert!(!segment.deform, "{bone} hands its skin to the bend joints");
                let ctrl = rig.control(&seg.control).unwrap();
                assert_eq!(ctrl.kind, ControlKind::Bend);
                assert_eq!(ctrl.parent, ControlParent::Bone(bone.clone()));
                assert!((ctrl.position - segment.head.lerp(segment.tail, 0.5)).length() < 1e-5);
                // Evenly spaced joints from the segment's start to its end, in order.
                assert_eq!(seg.joints.len(), BEND_JOINTS);
                let mut at = segment.head;
                let mut share = 0.0f32;
                for (n, name) in seg.joints.iter().enumerate() {
                    let j = sk.bone(name).unwrap();
                    assert_eq!(name, &twist_joint_name(bone, n + 1));
                    assert!(j.deform && !j.translate, "{name}");
                    assert_eq!(sk.bones[j.parent.unwrap()].name, *bone);
                    assert!(
                        (j.head - at).length() < 1e-5,
                        "{name} starts where the last ended"
                    );
                    at = j.tail;
                    assert_eq!(j.links, segment.links, "{name} blends like its segment");
                    let t = j.twist.as_ref().unwrap();
                    assert!(
                        t.factor.abs() > share,
                        "{name}: twist shares grow along the segment"
                    );
                    share = t.factor.abs();
                }
                assert!((at - segment.tail).length() < 1e-5);
                assert!(
                    (share - 0.5).abs() < 1e-6,
                    "the far joint takes the usual share"
                );
            }
            // The bendy controls are mirrored with the rest of the limb.
            if chain.name.ends_with("_l") {
                for c in [&bendy.pin, &bendy.upper.control, &bendy.lower.control] {
                    assert!(rig.mirror_pairs.iter().any(|[l, _]| l == c), "{c}");
                }
            }
        }
        // Segment bones stop deforming but still turn their children (no position copy).
        assert!(!sk.bone("lowerarm_l").unwrap().translate);
        assert!(!sk.bone("hand_l").unwrap().translate);
        assert!(
            sk.bone("pelvis").unwrap().translate,
            "hangs from the root helper"
        );
        // Without twist the bend joints still arc, just without twist shares.
        g.components
            .iter_mut()
            .for_each(|c| c.options.twist = false);
        let sk = build(&g).unwrap().skeleton;
        assert!(
            sk.bone("calf_twist_04_l")
                .is_some_and(|b| b.twist.is_none())
        );
    }

    #[test]
    fn bendy_and_soft_are_checked() {
        let mut g = quadruped(1.0);
        let tail = g.components.iter_mut().find(|c| c.id == "tail").unwrap();
        tail.options.bendy = true;
        let e = build(&g).err().unwrap().to_string();
        assert!(e.contains("only arms and legs can be bendy"), "{e}");
        let mut g = quadruped(1.0);
        g.components
            .iter_mut()
            .find(|c| c.id == "tail")
            .unwrap()
            .options
            .soft = 0.2;
        assert!(build(&g).is_err());
        // The hock spring belongs to quad legs only, from 0 to 1.
        let mut g = demo_guides();
        g.components
            .iter_mut()
            .find(|c| c.kind == Kind::Leg)
            .unwrap()
            .options
            .spring = 0.5;
        let e = build(&g).err().unwrap().to_string();
        assert!(e.contains("only quad legs have a spring hock"), "{e}");
        let mut g = quadruped(1.0);
        assert_eq!(set_hock_spring(&mut g, 1.5), 2);
        let e = build(&g).err().unwrap().to_string();
        assert!(e.contains("spring must be 0 to 1"), "{e}");
        let mut g = demo_guides();
        set_limb_options(&mut g, false, 1.5);
        let e = build(&g).err().unwrap().to_string();
        assert!(e.contains("soft must be 0 to 1"), "{e}");
        // Soft alone keeps the plain skeleton; every IK limb carries its default.
        let mut g = demo_guides();
        set_limb_options(&mut g, false, 0.25);
        let built = build(&g).unwrap();
        assert_eq!(
            built.skeleton.bones.len(),
            build(&demo_guides()).unwrap().skeleton.bones.len()
        );
        assert!(
            built
                .controls
                .ik_chains
                .iter()
                .all(|c| c.default_soft == 0.25 && c.bendy.is_none())
        );
    }

    /// Quad legs take the bendy and soft options the arms and legs have: bend
    /// joints on the thigh and calf (the hock stays one bone), a knee pin, the
    /// soft default, and the hock spring's default on the hock segment.
    #[test]
    fn quad_legs_are_bendy_soft_and_sprung() {
        let mut g = quadruped(0.8);
        set_limb_options(&mut g, true, 0.3);
        assert_eq!(set_hock_spring(&mut g, 0.75), 2);
        let built = build(&g).unwrap();
        let sk = &built.skeleton;
        for s in ["_l", "_r"] {
            let leg = built
                .controls
                .ik_chains
                .iter()
                .find(|c| c.name == format!("leg{s}"))
                .unwrap();
            assert_eq!(leg.default_soft, 0.3);
            let hock = leg.hock.as_ref().unwrap();
            assert_eq!(hock.default_spring, 0.75);
            let bendy = leg.bendy.as_ref().expect("bendy hind leg");
            assert_eq!(bendy.upper.bone, format!("thigh{s}"));
            assert_eq!(bendy.lower.bone, format!("calf{s}"));
            assert_eq!(bendy.pin, format!("calf{s}_pin_ctrl"));
            for seg in [&bendy.upper, &bendy.lower] {
                assert_eq!(seg.joints.len(), BEND_JOINTS);
                let bone = sk.bone(&seg.bone).unwrap();
                assert!(
                    !bone.deform,
                    "{} hands its skin to its bend joints",
                    seg.bone
                );
                // The bend joints run end to end along their segment.
                let first = sk.bone(&seg.joints[0]).unwrap();
                let last = sk.bone(seg.joints.last().unwrap()).unwrap();
                assert!(first.head.distance(bone.head) < 1e-5);
                assert!(last.tail.distance(bone.tail) < 1e-5);
                assert!(seg.joints.iter().all(|j| sk.bone(j).unwrap().deform));
            }
            // The lower curve ends at the hock joint, where the calf ends.
            assert!(
                sk.bone(&format!("calf{s}"))
                    .unwrap()
                    .tail
                    .distance(sk.bone(&hock.bone).unwrap().head)
                    < 1e-5
            );
            // The hock keeps one deforming bone and its FK control.
            assert!(sk.bone(&hock.bone).unwrap().deform);
            assert!(built.controls.control(&hock.fk_control).is_some());
            assert!(built.controls.control(&bendy.upper.control).is_some());
        }
        // The front legs (arms) are bendy too; the guides round-trip the options.
        assert!(built.controls.ik_chains.iter().all(|c| c.bendy.is_some()));
        let text = serde_json::to_string(&g).unwrap();
        let back: Guides = serde_json::from_str(&text).unwrap();
        let hind = back.component("hind_leg_l").unwrap();
        assert!(hind.options.bendy && hind.options.soft == 0.3 && hind.options.spring == 0.75);
        // Unset, the spring isn't written and reads back as 0.
        let plain = serde_json::to_string(&quadruped(0.8)).unwrap();
        assert!(!plain.contains("spring"));
    }

    /// The horse's hind legs start with a full spring hock; the dog's, bird's
    /// and dinosaur's with none.
    #[test]
    fn horse_hocks_spring_and_dinosaur_stands_on_quad_legs() {
        let spring = |g: &Guides| -> Vec<f32> {
            build(g)
                .unwrap()
                .controls
                .ik_chains
                .iter()
                .filter_map(|c| c.hock.as_ref().map(|h| h.default_spring))
                .collect()
        };
        assert_eq!(spring(&horse(1.8)), [1.0, 1.0]);
        assert_eq!(spring(&quadruped(0.8)), [0.0, 0.0]);
        assert_eq!(spring(&bird(0.5)), [0.0, 0.0]);
        let dino = dinosaur(1.6);
        assert_eq!(spring(&dino), [0.0, 0.0]);
        let built = build(&dino).unwrap();
        // Short arms start in FK (they hang above the belly); the legs in IK.
        for c in &built.controls.ik_chains {
            let want = if c.name.starts_with("arm") { 0.0 } else { 1.0 };
            assert_eq!(c.default_blend, want, "{}", c.name);
        }
        assert_eq!(built.controls.splines.len(), 1, "the spline tail");
        assert_eq!(built.controls.splines[0].bones.len(), DINOSAUR_TAIL_JOINTS);
        // The horse's front legs are planted and its layout reads as four-legged.
        let horse = build(&horse(1.8)).unwrap();
        let front = horse
            .controls
            .ik_chains
            .iter()
            .find(|c| c.name == "arm_l")
            .unwrap();
        assert_eq!(front.default_blend, 1.0);
        assert_eq!(crate::orient::layout_name(&super::horse(1.8)), "dog");
        assert_eq!(crate::orient::layout_name(&dino), "creature");
    }

    /// Bones and controls as comparable values.
    fn fingerprint(g: &Guides) -> Vec<String> {
        let b = build(g).unwrap();
        let bones = b.skeleton.bones.iter().map(|x| {
            format!(
                "{} {:?} {:.5} {:.5} {}",
                x.name,
                x.parent,
                x.head,
                x.tail,
                x.links.len()
            )
        });
        let controls = b.controls.controls.iter().map(|c| {
            format!(
                "{} {:?} {:.5} {:.5} {:.5}",
                c.name, c.parent, c.position, c.axis, c.size
            )
        });
        let chains = b.controls.ik_chains.iter().map(|k| {
            format!(
                "{} {} {:?}",
                k.name,
                k.default_blend,
                k.foot.as_ref().map(|f| f.break_angle)
            )
        });
        bones.chain(controls).chain(chains).collect()
    }

    #[test]
    fn files_are_stamped_and_old_ones_keep_their_rules() {
        assert_eq!(rules(1), RULES_V1);
        assert_eq!(SPINE_JOINTS, RULES_V1.spine_joints);
        let dir = std::env::temp_dir().join(format!("hive_versions_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        for g in [demo_guides(), quadruped(1.0)] {
            // Written: stamped with this version.
            let path = dir.join("a.guides.json");
            g.save(&path).unwrap();
            let text = std::fs::read_to_string(&path).unwrap();
            let json: serde_json::Value = serde_json::from_str(&text).unwrap();
            assert_eq!(json["hive_rig"], crate::version::CURRENT);
            assert_eq!(json["version"], GUIDES_VERSION);
            let back = Guides::load(&path).unwrap();
            assert_eq!(back.written_by.as_deref(), Some(crate::version::CURRENT));
            assert!(back.newer_warning().is_none());
            assert_eq!(fingerprint(&back), fingerprint(&g));

            // A file from before stamps (no `hive_rig`, version 1) opens and
            // builds exactly as before; saving it stamps it and keeps its version.
            let mut old = json.clone();
            old.as_object_mut().unwrap().remove("hive_rig");
            std::fs::write(&path, old.to_string()).unwrap();
            let back = Guides::load(&path).unwrap();
            assert_eq!(back.written_by, None);
            assert!(
                back.origin().contains("before version stamps"),
                "{}",
                back.origin()
            );
            assert!(back.newer_warning().is_none());
            assert_eq!(fingerprint(&back), fingerprint(&g));
            back.save(&path).unwrap();
            let again = Guides::load(&path).unwrap();
            assert_eq!(again.version, 1);
            assert_eq!(again.written_by.as_deref(), Some(crate::version::CURRENT));

            // From a newer hive-rig: loads (unknown settings ignored), warns, and
            // builds with the newest rules this one knows.
            for (key, value) in [
                ("hive_rig", serde_json::json!("99.0.0")),
                ("version", serde_json::json!(GUIDES_VERSION + 1)),
            ] {
                let mut newer = json.clone();
                newer[key] = value;
                newer["a_setting_from_the_future"] = serde_json::json!({ "x": 1 });
                std::fs::write(&path, newer.to_string()).unwrap();
                let back = Guides::load(&path).unwrap();
                let w = back.newer_warning().expect("a newer file warns");
                assert!(w.contains("newer"), "{w}");
                assert_eq!(back.rules(), RULES_V1);
                assert_eq!(fingerprint(&back), fingerprint(&g));
            }
        }
        // The rig file is stamped too, and warns when a newer hive-rig wrote it.
        let mesh = crate::demo::demo_humanoid();
        let rig = crate::rig::build_rig(mesh, "v", demo_guides(), Default::default()).unwrap();
        let mut json = serde_json::to_value(&rig).unwrap();
        assert_eq!(json["hive_rig"], crate::version::CURRENT);
        assert_eq!(json["guides"]["hive_rig"], crate::version::CURRENT);
        let back: crate::rig::Rig = serde_json::from_value(json.clone()).unwrap();
        assert!(back.newer_warning().is_none());
        json["hive_rig"] = "99.0.0".into();
        let back: crate::rig::Rig = serde_json::from_value(json).unwrap();
        assert!(back.newer_warning().unwrap().contains("99.0.0"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn every_shape_styles_a_control_from_guides_json() {
        let mut g = demo_guides();
        let names: Vec<String> = build(&g)
            .unwrap()
            .controls
            .controls
            .iter()
            .map(|c| c.name.clone())
            .collect();
        for (name, shape) in names.iter().zip(Shape::ALL) {
            let style = ControlStyle {
                shape: Some(shape),
                ..Default::default()
            };
            set_control_style(&mut g, name, style).unwrap();
        }
        // Through guides.json and back, as the CLI, app and DCC rebuilds read it.
        let back: Guides = serde_json::from_str(&serde_json::to_string(&g).unwrap()).unwrap();
        let built = build(&back).unwrap();
        for (name, shape) in names.iter().zip(Shape::ALL) {
            let c = built.controls.control(name).unwrap();
            assert_eq!(c.shape, shape);
            assert_eq!(c.outline().len(), shape.strokes().len(), "{name}");
        }
    }

    #[test]
    fn restyling_reaches_the_twin_and_the_component() {
        let mut g = demo_guides();
        let built = build(&g).unwrap();
        let ik = built
            .controls
            .ik_chains
            .iter()
            .find(|c| c.name == "arm_l")
            .unwrap();
        let (hand, hand_r) = (ik.ik_control.clone(), ik.ik_control.replace("_l_", "_r_"));
        assert!(built.controls.control(&hand_r).is_some(), "{hand_r}");
        // One control: only it changes.
        let changed = restyle(
            &mut g,
            &hand,
            StyleChange {
                shape: Some(Shape::Gear),
                ..Default::default()
            },
            StyleScope::One,
        )
        .unwrap();
        assert_eq!(changed, vec![hand.clone()]);
        assert!(!g.control_styles.contains_key(&hand_r));
        // With its twin: same shape and size; a side colour swaps sides,
        // any other colour is the same on both.
        restyle(
            &mut g,
            &hand,
            StyleChange {
                size: Some(0.09),
                color: Some(LEFT),
                ..Default::default()
            },
            StyleScope::WithTwin,
        )
        .unwrap();
        let (l, r) = (g.control_styles[&hand], g.control_styles[&hand_r]);
        assert_eq!(
            (l.shape, l.size, l.color),
            (Some(Shape::Gear), Some(0.09), Some(LEFT))
        );
        assert_eq!((r.shape, r.size, r.color), (None, Some(0.09), Some(RIGHT)));
        let green = [0.2, 0.9, 0.3];
        restyle(
            &mut g,
            &hand_r,
            StyleChange {
                color: Some(green),
                ..Default::default()
            },
            StyleScope::WithTwin,
        )
        .unwrap();
        assert_eq!(g.control_styles[&hand].color, Some(green));
        assert_eq!(g.control_styles[&hand_r].color, Some(green));
        // A whole component, twins too, scaled from each control's own size.
        let before = build(&g).unwrap();
        let comp = before
            .controls
            .control(&ik.fk_controls[0])
            .unwrap()
            .component
            .clone();
        let changed = restyle(
            &mut g,
            &ik.fk_controls[0],
            StyleChange {
                scale: Some(2.0),
                ..Default::default()
            },
            StyleScope::Component { twins: true },
        )
        .unwrap();
        let own: Vec<_> = before
            .controls
            .controls
            .iter()
            .filter(|c| c.component == comp)
            .collect();
        assert!(
            own.len() > 3 && changed.len() == 2 * own.len(),
            "{changed:?}"
        );
        let after = build(&g).unwrap();
        for name in &changed {
            let (a, b) = (
                after.controls.control(name).unwrap(),
                before.controls.control(name).unwrap(),
            );
            assert!((a.size - 2.0 * b.size).abs() < 1e-6, "{name}");
        }
        // Reset takes the looks off again, and a bad size is refused.
        assert_eq!(
            reset_styles(&mut g, changed.iter().map(String::as_str)),
            changed.len()
        );
        assert!(!g.control_styles.contains_key(&hand));
        let bad = StyleChange {
            size: Some(-1.0),
            ..Default::default()
        };
        assert!(restyle(&mut g, &hand, bad, StyleScope::One).is_err());
        assert!(
            restyle(
                &mut g,
                "no_such_ctrl",
                StyleChange::default(),
                StyleScope::One
            )
            .is_err()
        );
        assert_eq!(twin_color(CENTER), CENTER);
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

    /// Control parents and spaces only name controls listed earlier, and every
    /// bone a control or chain names exists.
    fn assert_controls_consistent(built: &Built) {
        let (sk, rig) = (&built.skeleton, &built.controls);
        for (i, c) in rig.controls.iter().enumerate() {
            match &c.parent {
                ControlParent::Control(p) => {
                    let pi = rig.controls.iter().position(|x| &x.name == p).expect(p);
                    assert!(pi < i, "{} listed before parent {p}", c.name);
                }
                ControlParent::Bone(b) => assert!(sk.bone(b).is_some(), "{b}"),
                ControlParent::None => assert_eq!(c.kind, ControlKind::Root),
            }
            if let Some(b) = &c.bone {
                assert!(sk.bone(b).is_some(), "{b}");
            }
            for sp in &c.spaces {
                let si = rig.controls.iter().position(|x| &x.name == sp).expect(sp);
                assert!(si < i, "{} space {sp} comes later", c.name);
            }
        }
        for k in &rig.ik_chains {
            for b in k.bones() {
                assert!(sk.bone(b).is_some(), "{b}");
            }
            let mut ctrls: Vec<&str> = vec![&k.ik_control, &k.pole_control];
            ctrls.extend(k.fk_controls.iter().map(String::as_str));
            if let Some(h) = &k.hock {
                ctrls.extend([h.fk_control.as_str(), h.control.as_str()]);
            }
            for c in ctrls {
                assert!(rig.control(c).is_some(), "{c}");
            }
        }
    }

    #[test]
    fn quadruped_layout_builds_three_segment_hind_legs() {
        let g = quadruped(0.8);
        let built = build(&g).unwrap();
        assert_controls_consistent(&built);
        let Built {
            skeleton: sk,
            controls: rig,
        } = &built;
        let parent_of = |n: &str| sk.bones[sk.bone(n).unwrap().parent.unwrap()].name.clone();
        for s in ["_l", "_r"] {
            // thigh > calf > hock > foot (paw) > ball (toes), from the pelvis.
            assert_eq!(parent_of(&format!("thigh{s}")), "pelvis");
            assert_eq!(parent_of(&format!("calf{s}")), format!("thigh{s}"));
            assert_eq!(parent_of(&format!("hock{s}")), format!("calf{s}"));
            assert_eq!(parent_of(&format!("foot{s}")), format!("hock{s}"));
            assert_eq!(parent_of(&format!("ball{s}")), format!("foot{s}"));
            assert_eq!(
                sk.bone(&format!("calf_twist_01{s}"))
                    .unwrap()
                    .twist
                    .as_ref()
                    .unwrap()
                    .source,
                format!("hock{s}")
            );
            // Front legs are planted arms: leg skin, IK by default.
            assert_eq!(parent_of(&format!("hand{s}")), format!("lowerarm{s}"));
        }
        assert_eq!(sk.bone("lowerarm_l").unwrap().region, Region::LegL);
        assert_eq!(sk.bone("hock_r").unwrap().region, Region::LegR);
        assert!(sk.bone("tail_05").is_some() && sk.bone("head").is_some());
        assert_eq!(rig.ik_chains.len(), 4);

        let leg = rig.ik_chains.iter().find(|k| k.name == "leg_l").unwrap();
        assert_eq!(leg.bones(), ["thigh_l", "calf_l", "hock_l", "foot_l"]);
        assert_eq!(leg.default_blend, 1.0);
        let hock = leg.hock.as_ref().unwrap();
        assert_eq!(
            (hock.bone.as_str(), hock.fk_control.as_str()),
            ("hock_l", "hock_l_fk_ctrl")
        );
        let hc = rig.control(&hock.control).unwrap();
        assert_eq!(hc.kind, ControlKind::Hock);
        assert_eq!(hc.parent, ControlParent::Control(leg.ik_control.clone()));
        assert_eq!(hc.position, sk.bone("foot_l").unwrap().head);
        // FK runs thigh > calf > hock > paw.
        assert_eq!(
            rig.control("foot_l_fk_ctrl").unwrap().parent,
            ControlParent::Control("hock_l_fk_ctrl".into())
        );
        assert_eq!(
            rig.control("hock_l_fk_ctrl").unwrap().parent,
            ControlParent::Control("calf_l_fk_ctrl".into())
        );
        let f = leg.foot.as_ref().unwrap();
        assert!(f.heel.z < f.ball.z && f.ball.z < f.toe_tip.z);
        assert_eq!(f.ball_bone, "ball_l");

        // The knee points forward and the hock back, and the pole sits straight
        // ahead of the knee, square to the hip-to-hock line.
        let (hip, knee, hk) = (
            sk.bone("thigh_l").unwrap().head,
            sk.bone("calf_l").unwrap().head,
            sk.bone("hock_l").unwrap().head,
        );
        assert!(knee.z > hip.lerp(hk, 0.5).z && hk.z < sk.bone("foot_l").unwrap().tail.z);
        let off = rig.control(&leg.pole_control).unwrap().position - knee;
        assert!(off.z > 0.0 && off.x.abs() < 0.05 * off.length(), "{off}");
        assert!(off.normalize().dot((hk - hip).normalize()).abs() < 1e-4);

        let front = rig.ik_chains.iter().find(|k| k.name == "arm_l").unwrap();
        assert!(front.hock.is_none() && front.foot.is_none());
        assert_eq!(front.default_blend, 1.0);
        let elbow = sk.bone("lowerarm_l").unwrap().head;
        assert!(rig.control(&front.pole_control).unwrap().position.z < elbow.z);
        assert_eq!(rig.control(&front.ik_control).unwrap().axis, Vec3::Y);
        // A biped's arm is unchanged: FK by default, IK control along the arm.
        let biped_rig = build(&demo_guides()).unwrap().controls;
        let arm = biped_rig
            .ik_chains
            .iter()
            .find(|k| k.name == "arm_l")
            .unwrap();
        assert_eq!(arm.default_blend, 0.0);
        assert!(arm.hock.is_none());

        // Guides round-trip, mirror and keep their kind's guide names.
        let json = serde_json::to_string(&g).unwrap();
        assert!(json.contains("\"quad_leg\""));
        let mut back: Guides = serde_json::from_str(&json).unwrap();
        let i = back
            .components
            .iter()
            .position(|c| c.id == "hind_leg_l")
            .unwrap();
        back.components[i].guides[2].position.z -= 0.02;
        assert_eq!(mirror_side(&mut back, Side::Left), 2);
        let (l, r) = (
            back.component("hind_leg_l").unwrap().guides[2].position,
            back.component("hind_leg_r").unwrap().guides[2].position,
        );
        assert!((mirror_point(&back, l) - r).length() < 1e-6);
        assert!(build(&back).is_ok());
    }

    #[test]
    fn quad_legs_preset_and_naming() {
        // Swap a biped's legs for hind legs.
        let mut g = demo_guides();
        assert!(add_preset(&mut g, Preset::QuadLegs, &AddOptions::default()).is_err());
        remove(&mut g, "leg_l").unwrap();
        remove(&mut g, "leg_r").unwrap();
        let ids = add_preset(&mut g, Preset::QuadLegs, &AddOptions::default()).unwrap();
        assert_eq!(ids, ["hind_leg_l", "hind_leg_r"]);
        let built = build(&g).unwrap();
        assert_controls_consistent(&built);
        let sk = &built.skeleton;
        let floor = g.body.floor;
        let toe = sk.bone("ball_l").unwrap().tail;
        assert!(
            (toe.y - floor).abs() < 0.05 * g.body.height,
            "the paw reaches the floor"
        );
        assert!(sk.bone("hock_l").unwrap().head.x > g.body.center_x);
        assert!(sk.bone("hock_r").unwrap().head.x < g.body.center_x);
        assert!(
            add_preset(
                &mut g,
                Preset::QuadLegs,
                &AddOptions {
                    side: Some(Side::Center),
                    ..Default::default()
                }
            )
            .is_err()
        );

        // Another naming convention renames the hock parts with everything else.
        let mut q = quadruped(0.8);
        q.naming = crate::naming::Preset::Prefix.naming();
        let rig = crate::rig::build_rig(
            crate::demo::demo_quadruped(),
            "dog",
            q,
            crate::RigOptions::default(),
        )
        .unwrap();
        let built = Built {
            skeleton: rig.skeleton,
            controls: rig.controls,
        };
        assert_controls_consistent(&built);
        let hock = built.controls.ik_chains[2].hock.as_ref().unwrap();
        assert_eq!(
            (hock.bone.as_str(), hock.control.as_str()),
            ("L_hock_jnt", "L_hock_ik_ctrl")
        );
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

    #[test]
    fn spline_weights_are_a_smooth_partition_of_unity() {
        for count in 2..=8 {
            let at = greville(count);
            assert_eq!((at[0], at[count - 1]), (0.0, 1.0), "{count}");
            assert!(at.windows(2).all(|w| w[0] < w[1]), "{count}: {at:?}");
            for i in 0..=40 {
                let u = i as f32 / 40.0;
                let w = spline_basis(count, u);
                assert_eq!(w.len(), count);
                assert!((w.iter().sum::<f32>() - 1.0).abs() < 1e-5, "{count} {u}");
                assert!(w.iter().all(|x| *x >= -1e-6), "{count} {u} {w:?}");
            }
            // The end controls own the ends of the curve.
            assert!((spline_basis(count, 0.0)[0] - 1.0).abs() < 1e-6);
            assert!((spline_basis(count, 1.0)[count - 1] - 1.0).abs() < 1e-6);
        }
        // Four controls make one cubic segment: the Bernstein weights.
        let w = spline_basis(4, 0.5);
        for (got, want) in w.iter().zip([0.125, 0.375, 0.375, 0.125]) {
            assert!((got - want).abs() < 1e-6, "{w:?}");
        }
    }

    #[test]
    fn tentacle_preset_builds_a_curve_driven_chain() {
        let mut g = demo_guides();
        let added = add_preset(&mut g, Preset::Tentacle, &AddOptions::default()).unwrap();
        assert_eq!(added, ["tentacle"]);
        let built = build(&g).unwrap();
        assert_controls_consistent(&built);
        let sp = &built.controls.splines[0];
        assert_eq!(sp.bones.len(), 8);
        assert_eq!(sp.bones[0], "tentacle_01");
        assert_eq!(
            sp.controls,
            [
                "tentacle_ik_01_ctrl",
                "tentacle_ik_02_ctrl",
                "tentacle_ik_03_ctrl",
                "tentacle_ik_04_ctrl"
            ]
        );
        assert_eq!(sp.fk_controls.len(), 3);
        assert_eq!(sp.settings, "tentacle_ik_04_ctrl");
        let base = built.skeleton.bone("tentacle_01").unwrap().parent.unwrap();
        assert_eq!(built.skeleton.bones[base].name, "pelvis");
        // A point per joint head, each weighted by the controls.
        assert_eq!(
            (sp.points.len(), sp.params.len(), sp.weights.len()),
            (8, 8, 8)
        );
        for w in &sp.weights {
            assert!((w.iter().sum::<f32>() - 1.0).abs() < 1e-5);
        }
        // The FK layer carries the curve controls: each FK control's child is the next curve control.
        for (i, fk) in sp.fk_controls.iter().enumerate() {
            let next = built.controls.control(&sp.controls[i + 1]).unwrap();
            assert_eq!(next.parent, ControlParent::Control(fk.clone()));
            assert_eq!(next.kind, ControlKind::Spline);
        }
        // The id names list every bone and control the component builds.
        let (bones, ctrls) = id_names(g.component("tentacle").unwrap());
        assert_eq!(bones, sp.bones);
        for c in ctrls {
            assert!(built.controls.control(&c).is_some(), "{c}");
        }

        // Joint and control counts are options; the joints are resampled along the guides.
        let c = g
            .components
            .iter_mut()
            .find(|c| c.id == "tentacle")
            .unwrap();
        c.options.joints = Some(12);
        c.options.controls = Some(5);
        let first = c.guides[0].position;
        let last = c.guides.last().unwrap().position;
        let built = build(&g).unwrap();
        assert_controls_consistent(&built);
        let sp = &built.controls.splines[0];
        assert_eq!(
            (sp.bones.len(), sp.controls.len(), sp.fk_controls.len()),
            (12, 5, 4)
        );
        let sk = &built.skeleton;
        assert!((sk.bone("tentacle_01").unwrap().head - first).length() < 1e-5);
        assert!((sk.bone("tentacle_12").unwrap().tail - last).length() < 1e-4);
        let lengths: Vec<f32> = sp
            .bones
            .iter()
            .map(|b| sk.bone(b).unwrap().length())
            .collect();
        assert!(
            lengths.iter().all(|l| (l - lengths[0]).abs() < 1e-4),
            "{lengths:?}"
        );

        // Bad counts are refused.
        let c = g
            .components
            .iter_mut()
            .find(|c| c.id == "tentacle")
            .unwrap();
        c.options.controls = Some(1);
        assert!(build(&g).is_err());
        let c = g
            .components
            .iter_mut()
            .find(|c| c.id == "tentacle")
            .unwrap();
        c.options.controls = None;
        c.options.joints = Some(1);
        assert!(build(&g).is_err());
        let mut g = demo_guides();
        let opts = AddOptions {
            parent: Some("head".into()),
            joints: Some(4),
            controls: Some(3),
            id: Some("tendril".into()),
            ..AddOptions::default()
        };
        add_preset(&mut g, Preset::Tentacle, &opts).unwrap();
        let built = build(&g).unwrap();
        assert_eq!(built.controls.splines[0].controls.len(), 3);
        assert!(built.skeleton.bone("tendril_04").is_some());
        assert!(
            add_preset(
                &mut g,
                Preset::Tentacle,
                &AddOptions {
                    joints: Some(1),
                    ..AddOptions::default()
                }
            )
            .is_err()
        );
    }

    #[test]
    fn spline_spine_uses_hips_and_chest_controls() {
        let mut g = demo_guides();
        set_spine_spline(&mut g, "spine", true, None).unwrap();
        let built = build(&g).unwrap();
        assert_controls_consistent(&built);
        let rig = &built.controls;
        let sp = &rig.splines[0];
        assert_eq!(sp.bones, ["spine_01", "spine_02", "spine_03"]);
        assert_eq!(sp.controls, ["hips_ctrl", "spine_mid_ctrl", "chest_ctrl"]);
        assert_eq!(sp.fk_controls, ["spine_fk_01_ctrl", "spine_fk_02_ctrl"]);
        // The hips drive the pelvis (and the legs hang from them); no per-joint FK.
        let hips = rig.control("hips_ctrl").unwrap();
        assert_eq!(
            (hips.kind, hips.bone.as_deref()),
            (ControlKind::Fk, Some("pelvis"))
        );
        assert!(rig.control("pelvis_ctrl").is_none() && rig.control("spine_01_ctrl").is_none());
        assert_eq!(
            rig.control("thigh_l_fk_ctrl").unwrap().parent,
            ControlParent::Control("hips_ctrl".into())
        );
        // Parts on the top joint follow the bone, with the chest control as their chest space.
        assert_eq!(
            rig.control("clavicle_l_ctrl").unwrap().parent,
            ControlParent::Bone("spine_03".into())
        );
        assert!(
            rig.control("hand_l_ik_ctrl")
                .unwrap()
                .spaces
                .contains(&"chest_ctrl".to_string())
        );
        assert!(
            rig.control("head_ctrl")
                .unwrap()
                .spaces
                .contains(&"chest_ctrl".to_string())
        );
        // The bones are the same as the FK spine's, so attachments are untouched.
        let fk = build(&demo_guides()).unwrap();
        let names = |b: &Built| {
            b.skeleton
                .bones
                .iter()
                .map(|x| x.name.clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(names(&built), names(&fk));

        // More curve controls, a joint count change, then back to FK.
        set_spine_spline(&mut g, "spine", true, Some(5)).unwrap();
        set_spine_joints(&mut g, "spine", 6).unwrap();
        let built = build(&g).unwrap();
        assert_controls_consistent(&built);
        let sp = &built.controls.splines[0];
        assert_eq!(sp.controls.len(), 5);
        assert_eq!(sp.controls[1], "spine_mid_01_ctrl");
        assert_eq!(sp.bones.len(), 6);
        assert!(
            set_spine_joints(&mut g, "spine", 1).is_err(),
            "a spline spine needs two joints"
        );
        assert!(set_spine_spline(&mut g, "spine", true, Some(1)).is_err());
        set_spine_spline(&mut g, "spine", false, None).unwrap();
        let built = build(&g).unwrap();
        assert!(built.controls.splines.is_empty());
        assert!(built.controls.control("spine_06_ctrl").is_some());
        // The option is saved only when on.
        let json = serde_json::to_string(&g).unwrap();
        assert!(!json.contains("\"spline\""));
        set_spine_spline(&mut g, "spine", true, None).unwrap();
        let back: Guides = serde_json::from_str(&serde_json::to_string(&g).unwrap()).unwrap();
        assert!(back.component("spine").unwrap().options.spline);
    }

    #[test]
    fn spline_names_follow_the_naming_convention_and_sides() {
        let mut g = demo_guides();
        set_spine_spline(&mut g, "spine", true, None).unwrap();
        let opts = AddOptions {
            parent: Some("hand_l".into()),
            joints: Some(3),
            ..AddOptions::default()
        };
        add_preset(&mut g, Preset::Tentacle, &opts).unwrap();
        let opts = AddOptions {
            parent: Some("hand_r".into()),
            joints: Some(3),
            ..AddOptions::default()
        };
        add_preset(&mut g, Preset::Tentacle, &opts).unwrap();
        let mut built = build(&g).unwrap();
        let sp = built
            .controls
            .splines
            .iter()
            .find(|s| s.name == "tentacle_l")
            .unwrap();
        assert_eq!(sp.bones[0], "tentacle_01_l");
        assert_eq!(sp.controls[0], "tentacle_l_ik_01_ctrl");
        assert!(built.controls.mirror_pairs.contains(&[
            "tentacle_l_ik_02_ctrl".into(),
            "tentacle_r_ik_02_ctrl".into()
        ]));
        let naming = crate::naming::Preset::Prefix.naming();
        naming
            .apply(&mut built.skeleton, &mut built.controls)
            .unwrap();
        for sp in &built.controls.splines {
            for b in &sp.bones {
                assert!(built.skeleton.bone(b).is_some(), "{b}");
            }
            for c in sp
                .controls
                .iter()
                .chain(&sp.fk_controls)
                .chain([&sp.settings])
            {
                assert!(built.controls.control(c).is_some(), "{c}");
            }
        }
        let sp = built
            .controls
            .splines
            .iter()
            .find(|s| s.name == "tentacle_l")
            .unwrap();
        assert_eq!(sp.controls[0], "L_tentacle_ik_01_ctrl");
        assert_eq!(built.controls.splines[0].controls[0], "C_hips_ctrl");
    }
}
