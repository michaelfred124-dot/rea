//! The component library: the premade parts of the rigging window, the options
//! each one takes before it is added, and the operations that add them.
//!
//! Every part is a card in the window (drawn in `ui/library_panel.rs`). A card
//! lists the options it offers ([`Opt`]); the choices live in [`PartOptions`] so
//! they are remembered between additions and one click still adds a part with
//! sensible defaults. The parts are built by [`components::add_preset`], the
//! same code `hive-rig add` runs.

use std::collections::HashSet;

use glam::Vec3;
use hive_rig::components::{self, AddOptions, Component, Guides, Kind, Preset};
use hive_rig::skeleton::Side;

use super::{Category, FingerSides, Studio};

/// What a card's illustration shows.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Art {
    Arms,
    Legs,
    Spine,
    SplineSpine,
    NeckHead,
    Fingers,
    Toes,
    HindLegs,
    Tail,
    Tentacle,
    Wings,
    ExtraArms,
    ExtraLegs,
    Antlers,
    Horns,
    Ears,
    Face,
    Eyes,
    Jaw,
    FaceBoard,
    Ponytail,
    Cape,
    Skirt,
    Prop,
    Chain,
}

/// One choice a part offers before it is added.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Opt {
    /// Both, left or right.
    Sides,
    /// Left or right hand (a prop goes in one).
    Hand,
    /// Joints along the part.
    Joints(&'static str, usize, usize),
    /// A number of copies, fingers or feathers.
    Count(&'static str, usize, usize),
    /// Rows of feathers.
    Rows,
    /// Curve controls of a spline part.
    Controls,
    /// The bone it hangs from; "Automatic" lets the part choose, unless required.
    Parent { required: bool },
    /// Bend controls on a limb.
    Bendy,
    /// A curve (spline) drive instead of an FK control a bone.
    Spline,
    /// A name of its own.
    Id,
}

/// What clicking Add does for a part.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    /// Add a component with this preset.
    Preset(Preset),
    /// Change the spine that every rig has.
    Spine,
    /// Select the neck and head that every rig has.
    Head,
}

/// Starting choices of a part.
#[derive(Clone, Copy)]
pub struct Defaults {
    pub joints: usize,
    pub count: usize,
    pub rows: usize,
    pub controls: usize,
    pub parent: &'static str,
    pub spline: bool,
    pub sides: FingerSides,
}

const D0: Defaults = Defaults {
    joints: 3,
    count: 1,
    rows: 1,
    controls: 4,
    parent: "",
    spline: false,
    sides: FingerSides::Both,
};

/// How a part's components are told apart when counting what a rig has.
#[derive(Clone, Copy, PartialEq, Eq)]
enum By {
    /// One set for each id without its side or strip number.
    Id,
    /// One set for each bone the components hang from (a hand's fingers are one).
    Parent,
}

/// A premade part in the rigging window.
pub struct LibraryPart {
    pub key: &'static str,
    pub art: Art,
    pub title: &'static str,
    pub text: &'static str,
    /// What to adjust once it is in, shown after adding it.
    pub hint: &'static str,
    pub category: Category,
    pub action: Action,
    pub opts: &'static [Opt],
    pub defaults: Defaults,
    /// Parts a rig has one of: the card shows "Added" and stays off.
    pub single: bool,
    has: fn(&Component) -> bool,
    by: By,
}

/// The choices made for a part, kept between additions.
#[derive(Clone)]
pub struct PartOptions {
    pub sides: FingerSides,
    pub joints: usize,
    pub count: usize,
    pub rows: usize,
    pub controls: usize,
    /// The bone to hang from; empty is automatic.
    pub parent: String,
    pub bendy: bool,
    pub spline: bool,
    pub id: String,
}

impl PartOptions {
    pub fn new(part: &LibraryPart) -> PartOptions {
        let d = &part.defaults;
        PartOptions {
            sides: d.sides,
            joints: d.joints,
            count: d.count,
            rows: d.rows,
            controls: d.controls,
            parent: d.parent.to_string(),
            bendy: false,
            spline: d.spline,
            id: String::new(),
        }
    }

    /// The preset's options for what this part lets the user choose.
    fn add_options(&self, part: &LibraryPart) -> AddOptions {
        let has = |f: fn(&Opt) -> bool| part.opts.iter().any(f);
        let side = match self.sides {
            FingerSides::Both => None,
            FingerSides::Left => Some(Side::Left),
            FingerSides::Right => Some(Side::Right),
        };
        let id = self.id.trim();
        AddOptions {
            side: side.filter(|_| has(|o| matches!(o, Opt::Sides | Opt::Hand))),
            parent: Some(self.parent.clone())
                .filter(|p| !p.is_empty() && has(|o| matches!(o, Opt::Parent { .. }))),
            joints: has(|o| matches!(o, Opt::Joints(..))).then_some(self.joints),
            id: Some(id.to_string()).filter(|i| !i.is_empty() && has(|o| *o == Opt::Id)),
            controls: ({
                has(|o| *o == Opt::Controls) && (self.spline || !has(|o| *o == Opt::Spline))
            })
            .then_some(self.controls),
            bendy: self.bendy && has(|o| *o == Opt::Bendy),
            count: has(|o| matches!(o, Opt::Count(..))).then_some(self.count),
            rows: has(|o| *o == Opt::Rows).then_some(self.rows),
            spline: self.spline && has(|o| *o == Opt::Spline),
        }
    }
}

fn base(c: &Component) -> &str {
    let id = c.id.as_str();
    let id = match c.side {
        Side::Left => id.strip_suffix("_l").unwrap_or(id),
        Side::Right => id.strip_suffix("_r").unwrap_or(id),
        Side::Center => id,
    };
    // A strip of a cape or skirt: cape_2 -> cape.
    match id.rsplit_once('_') {
        Some((head, n)) if c.side == Side::Center && n.chars().all(|d| d.is_ascii_digit()) => head,
        _ => id,
    }
}

/// `arm2` is the second pair of `arm`: the name with a number after the word.
fn numbered(base: &str, word: &str) -> bool {
    base.strip_prefix(word)
        .is_some_and(|n| !n.is_empty() && n.chars().all(|d| d.is_ascii_digit()))
}

fn is_arm(c: &Component) -> bool {
    c.kind == Kind::Arm && c.options.feathers == 0 && !numbered(base(c), "arm")
}
fn is_extra_arm(c: &Component) -> bool {
    c.kind == Kind::Arm && c.options.feathers == 0 && numbered(base(c), "arm")
}
fn is_leg(c: &Component) -> bool {
    c.kind == Kind::Leg
}
fn is_wing(c: &Component) -> bool {
    c.kind == Kind::Arm && (c.options.feathers > 0 || base(c).starts_with("wing"))
}
fn is_hind_leg(c: &Component) -> bool {
    c.kind == Kind::QuadLeg && !c.options.knee_up
}
fn is_spider_leg(c: &Component) -> bool {
    c.kind == Kind::QuadLeg && c.options.knee_up
}
fn is_spine(c: &Component) -> bool {
    c.kind == Kind::Spine
}
fn is_head(c: &Component) -> bool {
    c.kind == Kind::Head
}
fn is_finger(c: &Component) -> bool {
    c.kind == Kind::Chain && ["thumb", "index", "middle", "ring", "pinky"].contains(&base(c))
}
fn is_toe(c: &Component) -> bool {
    c.kind == Kind::Chain && base(c).starts_with("toe_")
}
fn is_tail(c: &Component) -> bool {
    matches!(c.kind, Kind::Chain | Kind::Spline) && base(c).starts_with("tail")
}
fn is_tentacle(c: &Component) -> bool {
    c.kind == Kind::Spline && !base(c).starts_with("tail")
}
fn is_antler(c: &Component) -> bool {
    c.kind == Kind::Chain && base(c).starts_with("antler")
}
fn is_horn(c: &Component) -> bool {
    c.kind == Kind::Chain && base(c).starts_with("horn")
}
fn is_ear(c: &Component) -> bool {
    c.kind == Kind::Chain && base(c).starts_with("ear")
}
fn is_face(c: &Component) -> bool {
    c.kind == Kind::Face
}
fn is_eyes(c: &Component) -> bool {
    c.kind == Kind::Eyes
}
fn is_jaw(c: &Component) -> bool {
    c.kind == Kind::Jaw
}
fn is_board(c: &Component) -> bool {
    c.kind == Kind::Face && c.options.board
}
fn is_ponytail(c: &Component) -> bool {
    c.kind == Kind::Chain && base(c).starts_with("ponytail")
}
fn is_cape(c: &Component) -> bool {
    c.kind == Kind::Chain && base(c).starts_with("cape")
}
fn is_skirt(c: &Component) -> bool {
    c.kind == Kind::Chain && base(c).starts_with("skirt")
}
fn is_prop(c: &Component) -> bool {
    c.kind == Kind::Chain && base(c).starts_with("prop")
}
fn is_plain_chain(c: &Component) -> bool {
    c.kind == Kind::Chain && base(c).starts_with("chain")
}

const BONES: Opt = Opt::Parent { required: false };

pub const LIBRARY: [LibraryPart; 25] = [
    // ------------------------------------------------------------------ body
    LibraryPart {
        key: "arms",
        art: Art::Arms,
        title: "Arms",
        text: "Clavicle, shoulder, elbow and wrist with IK and FK.",
        hint: "Move the arm guides onto the shoulders and hands.",
        category: Category::Body,
        action: Action::Preset(Preset::Arms),
        opts: &[Opt::Sides, Opt::Bendy, BONES],
        defaults: D0,
        single: true,
        has: is_arm,
        by: By::Id,
    },
    LibraryPart {
        key: "legs",
        art: Art::Legs,
        title: "Legs",
        text: "Hip, knee, ankle and a foot that rolls, with IK and FK.",
        hint: "Move the leg guides onto the hips, knees and feet.",
        category: Category::Body,
        action: Action::Preset(Preset::Legs),
        opts: &[Opt::Sides, Opt::Bendy, BONES],
        defaults: D0,
        single: true,
        has: is_leg,
        by: By::Id,
    },
    LibraryPart {
        key: "spine",
        art: Art::Spine,
        title: "Spine",
        text: "Change its joint count, or drive it with a curve.",
        hint: "The spine guides stay where they are.",
        category: Category::Body,
        action: Action::Spine,
        opts: &[Opt::Joints("Joints", 1, 16), Opt::Spline, Opt::Controls],
        defaults: Defaults {
            joints: 3,
            controls: 3,
            ..D0
        },
        single: false,
        has: is_spine,
        by: By::Id,
    },
    LibraryPart {
        key: "neck_head",
        art: Art::NeckHead,
        title: "Neck and head",
        text: "Every rig has them. Select the guides to move.",
        hint: "Move the neck, head and head-top guides.",
        category: Category::Body,
        action: Action::Head,
        opts: &[],
        defaults: D0,
        single: false,
        has: is_head,
        by: By::Id,
    },
    LibraryPart {
        key: "fingers",
        art: Art::Fingers,
        title: "Fingers",
        text: "Thumb to pinky on each hand.",
        hint: "Spread the finger guides over the hands.",
        category: Category::Body,
        action: Action::Preset(Preset::Fingers),
        opts: &[
            Opt::Sides,
            Opt::Count("Fingers", 1, 5),
            Opt::Joints("Bones each", 1, 5),
        ],
        defaults: Defaults { count: 5, ..D0 },
        single: false,
        has: is_finger,
        by: By::Parent,
    },
    LibraryPart {
        key: "toes",
        art: Art::Toes,
        title: "Toes",
        text: "Toes on each foot, from the ball.",
        hint: "Fit the toe guides to the feet.",
        category: Category::Body,
        action: Action::Preset(Preset::Toes),
        opts: &[
            Opt::Sides,
            Opt::Count("Toes", 1, 5),
            Opt::Joints("Bones each", 1, 5),
        ],
        defaults: Defaults { count: 5, ..D0 },
        single: false,
        has: is_toe,
        by: By::Parent,
    },
    LibraryPart {
        key: "hind_legs",
        art: Art::HindLegs,
        title: "Hind legs",
        text: "Thigh, calf, hock and paw with IK, for quadrupeds. Remove the legs first.",
        hint: "Place the hips, hocks and paws.",
        category: Category::Body,
        action: Action::Preset(Preset::QuadLegs),
        opts: &[Opt::Sides, Opt::Bendy, BONES],
        defaults: D0,
        single: true,
        has: is_hind_leg,
        by: By::Id,
    },
    LibraryPart {
        key: "tail",
        art: Art::Tail,
        title: "Tail",
        text: "From the pelvis: an FK control a bone, or a curve.",
        hint: "Bend the tail guides to the shape you want.",
        category: Category::Body,
        action: Action::Preset(Preset::Tail),
        opts: &[
            Opt::Joints("Bones", 2, 24),
            Opt::Spline,
            Opt::Controls,
            BONES,
        ],
        defaults: Defaults { joints: 5, ..D0 },
        single: false,
        has: is_tail,
        by: By::Id,
    },
    LibraryPart {
        key: "tentacle",
        art: Art::Tentacle,
        title: "Tentacle",
        text: "Curve controls bend it smoothly, with stretch, volume and FK.",
        hint: "Lay the tentacle guides out along its length.",
        category: Category::Body,
        action: Action::Preset(Preset::Tentacle),
        opts: &[Opt::Joints("Joints", 2, 64), Opt::Controls, BONES],
        defaults: Defaults { joints: 8, ..D0 },
        single: false,
        has: is_tentacle,
        by: By::Id,
    },
    // ------------------------------------------------------- wings and creatures
    LibraryPart {
        key: "wings",
        art: Art::Wings,
        title: "Wings",
        text: "Wing arms with rows of feathers that fan open and shut.",
        hint: "Spread the wing guides; change the feathers in Properties.",
        category: Category::Creature,
        action: Action::Preset(Preset::Wings),
        opts: &[
            Opt::Sides,
            Opt::Count("Feathers a row", 0, 24),
            Opt::Rows,
            Opt::Bendy,
            BONES,
        ],
        defaults: Defaults { count: 6, ..D0 },
        single: false,
        has: is_wing,
        by: By::Id,
    },
    LibraryPart {
        key: "extra_arms",
        art: Art::ExtraArms,
        title: "Extra arms",
        text: "A second or third pair for four- and six-armed characters: arm2, arm3.",
        hint: "Move the new arm guides onto the extra shoulders.",
        category: Category::Creature,
        action: Action::Preset(Preset::Arms),
        opts: &[Opt::Sides, Opt::Bendy, BONES],
        defaults: D0,
        single: false,
        has: is_extra_arm,
        by: By::Id,
    },
    LibraryPart {
        key: "extra_legs",
        art: Art::ExtraLegs,
        title: "Extra legs",
        text: "Spider-style legs with the knee folding up, pairs along the spine.",
        hint: "Place the hips on the body and the feet on the floor.",
        category: Category::Creature,
        action: Action::Preset(Preset::SpiderLegs),
        opts: &[Opt::Sides, Opt::Count("Pairs", 1, 8), Opt::Bendy, BONES],
        defaults: D0,
        single: false,
        has: is_spider_leg,
        by: By::Id,
    },
    LibraryPart {
        key: "antlers",
        art: Art::Antlers,
        title: "Antlers",
        text: "Forking FK chains on the head, a control on every branch.",
        hint: "Shape the beams; add more forks in Properties.",
        category: Category::Creature,
        action: Action::Preset(Preset::Antlers),
        opts: &[BONES],
        defaults: D0,
        single: true,
        has: is_antler,
        by: By::Id,
    },
    LibraryPart {
        key: "horns",
        art: Art::Horns,
        title: "Horns",
        text: "A curved FK horn on each side of the head.",
        hint: "Curve the horn guides up from the head.",
        category: Category::Creature,
        action: Action::Preset(Preset::Horns),
        opts: &[Opt::Sides, Opt::Joints("Bones", 1, 8), BONES],
        defaults: D0,
        single: true,
        has: is_horn,
        by: By::Id,
    },
    LibraryPart {
        key: "ears",
        art: Art::Ears,
        title: "Ears",
        text: "A short chain on each side of the head.",
        hint: "Move the ear guides to the sides of the head.",
        category: Category::Creature,
        action: Action::Preset(Preset::Ears),
        opts: &[Opt::Sides, Opt::Joints("Bones", 1, 6), BONES],
        defaults: Defaults { joints: 2, ..D0 },
        single: true,
        has: is_ear,
        by: By::Id,
    },
    LibraryPart {
        key: "spline_spine",
        art: Art::SplineSpine,
        title: "Spline spine",
        text: "A curve between hips and chest bends the spine, for snakes and tentacles.",
        hint: "The hips, middle and chest controls bend the spine.",
        category: Category::Creature,
        action: Action::Spine,
        opts: &[Opt::Joints("Joints", 1, 16), Opt::Controls],
        defaults: Defaults {
            joints: 6,
            controls: 3,
            spline: true,
            ..D0
        },
        single: false,
        has: is_spine,
        by: By::Id,
    },
    // ------------------------------------------------------------------ face
    LibraryPart {
        key: "face",
        art: Art::Face,
        title: "Face",
        text: "Brows, eyelids, cheeks, nose and lips as joints, with a face board.",
        hint: "Nudge the face guides onto the features.",
        category: Category::Face,
        action: Action::Preset(Preset::Face),
        opts: &[Opt::Joints("Extra lip joints", 0, 3)],
        defaults: Defaults { joints: 1, ..D0 },
        single: true,
        has: is_face,
        by: By::Id,
    },
    LibraryPart {
        key: "eyes",
        art: Art::Eyes,
        title: "Eyes",
        text: "Both eyes aim at one look-at control.",
        hint: "Put the eye guides in the eyes; the target sits in front.",
        category: Category::Face,
        action: Action::Preset(Preset::Eyes),
        opts: &[],
        defaults: D0,
        single: true,
        has: is_eyes,
        by: By::Id,
    },
    LibraryPart {
        key: "jaw",
        art: Art::Jaw,
        title: "Jaw",
        text: "A hinge that opens the mouth.",
        hint: "Put the jaw guide at the hinge and the chin guide on the chin.",
        category: Category::Face,
        action: Action::Preset(Preset::Jaw),
        opts: &[],
        defaults: D0,
        single: true,
        has: is_jaw,
        by: By::Id,
    },
    LibraryPart {
        key: "face_board",
        art: Art::FaceBoard,
        title: "Face board",
        text: "Sliders beside the head that pose the face joints. Needs the face.",
        hint: "The board stands beside the head in the Rig stage.",
        category: Category::Face,
        action: Action::Preset(Preset::FaceBoard),
        opts: &[Opt::Joints("Extra lip joints", 0, 3)],
        defaults: Defaults { joints: 1, ..D0 },
        single: true,
        has: is_board,
        by: By::Id,
    },
    // ---------------------------------------------------------------- extras
    LibraryPart {
        key: "ponytail",
        art: Art::Ponytail,
        title: "Ponytail",
        text: "Hair that swings from the back of the head.",
        hint: "Hang the hair guides down the back of the head.",
        category: Category::Extras,
        action: Action::Preset(Preset::Ponytail),
        opts: &[Opt::Joints("Bones", 1, 12), BONES],
        defaults: Defaults { joints: 4, ..D0 },
        single: true,
        has: is_ponytail,
        by: By::Id,
    },
    LibraryPart {
        key: "cape",
        art: Art::Cape,
        title: "Cape",
        text: "FK strips hanging side by side from the upper back.",
        hint: "Spread the cape strips across the shoulders.",
        category: Category::Extras,
        action: Action::Preset(Preset::Cape),
        opts: &[
            Opt::Count("Strips", 1, 12),
            Opt::Joints("Bones each", 1, 12),
            BONES,
        ],
        defaults: Defaults {
            count: 3,
            joints: 4,
            ..D0
        },
        single: false,
        has: is_cape,
        by: By::Id,
    },
    LibraryPart {
        key: "skirt",
        art: Art::Skirt,
        title: "Skirt",
        text: "FK strips in a ring around the hips.",
        hint: "Pull the skirt strips out to the hem you want.",
        category: Category::Extras,
        action: Action::Preset(Preset::Skirt),
        opts: &[
            Opt::Count("Strips", 3, 16),
            Opt::Joints("Bones each", 1, 8),
            BONES,
        ],
        defaults: Defaults {
            count: 6,
            joints: 3,
            ..D0
        },
        single: false,
        has: is_skirt,
        by: By::Id,
    },
    LibraryPart {
        key: "prop",
        art: Art::Prop,
        title: "Prop",
        text: "A weapon or tool socket in a hand.",
        hint: "Point the prop guide the way the tool is held.",
        category: Category::Extras,
        action: Action::Preset(Preset::Prop),
        opts: &[Opt::Hand],
        defaults: Defaults {
            sides: FingerSides::Right,
            ..D0
        },
        single: false,
        has: is_prop,
        by: By::Id,
    },
    LibraryPart {
        key: "chain",
        art: Art::Chain,
        title: "Chain",
        text: "An FK chain on any bone: hair, cloth, a strap.",
        hint: "Lay the chain guides along what should swing.",
        category: Category::Extras,
        action: Action::Preset(Preset::Chain),
        opts: &[
            Opt::Parent { required: true },
            Opt::Joints("Bones", 1, 24),
            Opt::Id,
        ],
        defaults: Defaults {
            parent: "head",
            ..D0
        },
        single: false,
        has: is_plain_chain,
        by: By::Id,
    },
];

impl LibraryPart {
    /// How many of this part the layout has (pairs and strips count once).
    pub fn tally(&self, g: &Guides) -> usize {
        let mut seen = HashSet::new();
        for c in g.components.iter().filter(|c| (self.has)(c)) {
            match self.by {
                By::Id => seen.insert(base(c).to_string()),
                By::Parent => seen.insert(c.parent.clone().unwrap_or_default()),
            };
        }
        seen.len()
    }

    /// Whether the part is built into every rig and only changes (the spine).
    pub fn edits(&self) -> bool {
        !matches!(self.action, Action::Preset(_))
    }
}

pub fn part(key: &str) -> Option<&'static LibraryPart> {
    LIBRARY.iter().find(|p| p.key == key)
}

/// The rigging window's group for a component, which the outliner sorts by too.
pub fn group_of(c: &Component) -> Category {
    match c.kind {
        Kind::Root | Kind::Spine | Kind::Head | Kind::Leg => Category::Body,
        Kind::Arm => {
            if is_wing(c) {
                Category::Creature
            } else {
                Category::Body
            }
        }
        Kind::QuadLeg => {
            if c.options.knee_up {
                Category::Creature
            } else {
                Category::Body
            }
        }
        Kind::Eyes | Kind::Jaw | Kind::Face => Category::Face,
        Kind::Spline => Category::Body,
        Kind::Chain => {
            if is_finger(c) || is_toe(c) || is_tail(c) {
                Category::Body
            } else if is_antler(c) || is_horn(c) || is_ear(c) {
                Category::Creature
            } else {
                Category::Extras
            }
        }
    }
}

impl Studio {
    /// The choices for a part, as last set (or its defaults).
    pub fn part_options(&mut self, part: &LibraryPart) -> &mut PartOptions {
        self.lib_opts
            .entry(part.key)
            .or_insert_with(|| PartOptions::new(part))
    }

    /// Show a part's options, reading the rig's own values for a part that edits it.
    pub fn pick_part(&mut self, part: &'static LibraryPart) {
        self.lib_pick = Some(part.key);
        if part.action == Action::Spine
            && let Some(doc) = &self.doc
            && let Some(spine) = doc.guides.components.iter().find(|c| c.kind == Kind::Spine)
        {
            let joints = doc.guides.spine_joints(spine);
            let (spline, controls) = (spine.options.spline, spine.options.curve_controls(true));
            let o = self.part_options(part);
            o.joints = joints;
            o.controls = controls;
            if part.key == "spine" {
                o.spline = spline;
            }
        }
    }

    /// Add a part by its key, with the choices made for it. Returns whether it worked.
    pub fn add_part(&mut self, key: &str) -> bool {
        let Some(part) = part(key) else { return false };
        if part.single && self.doc.as_ref().is_some_and(|d| part.tally(&d.guides) > 0) {
            self.fail(format!(
                "The rig already has {}.",
                part.title.to_lowercase()
            ));
            return false;
        }
        let o = self.part_options(part).clone();
        match part.action {
            Action::Preset(preset) => {
                let ids = self.add(preset, o.add_options(part));
                if ids.is_empty() {
                    return false;
                }
                match part.key {
                    "eyes" => self.snap_to_face(Kind::Eyes, &["eye_l", "eye_r"], 0.012),
                    "jaw" => self.snap_to_face(Kind::Jaw, &["chin"], 0.0),
                    "face" => self.snap_to_face(Kind::Face, &components::FACE_GUIDES, 0.003),
                    _ => {}
                }
                self.show_new(&ids, format!("Added {}. {}", ids.join(", "), part.hint));
                true
            }
            Action::Spine => self.change_spine(&o, part),
            Action::Head => {
                let head = self.doc.as_ref().and_then(|d| {
                    d.guides
                        .components
                        .iter()
                        .position(|c| c.kind == Kind::Head)
                });
                match head {
                    Some(i) => {
                        let id = self.component_id(i).unwrap_or_default();
                        self.show_new(&[id], format!("Selected the neck and head. {}", part.hint));
                        true
                    }
                    None => {
                        self.fail("The rig has no head component.");
                        false
                    }
                }
            }
        }
    }

    /// Joints and drive (FK or curve) of the spine every rig has.
    fn change_spine(&mut self, o: &PartOptions, part: &LibraryPart) -> bool {
        let Some(id) = self
            .doc
            .as_ref()
            .and_then(|d| d.guides.components.iter().find(|c| c.kind == Kind::Spine))
            .map(|c| c.id.clone())
        else {
            self.fail("The rig has no spine.");
            return false;
        };
        let (joints, spline, controls) = (o.joints, o.spline, o.controls);
        let changed = self.edit("change the spine", |g| {
            components::set_spine_joints(g, &id, joints)?;
            components::set_spine_spline(g, &id, spline, spline.then_some(controls))?;
            Ok(if spline {
                format!("The spine has {joints} joints on a curve of {controls} controls.")
            } else {
                format!("The spine has {joints} joints with FK controls.")
            })
        });
        if changed {
            let message = format!("{} {}", self.status, part.hint);
            self.show_new(&[id], message);
        }
        changed
    }

    /// Add a preset; returns the ids it added (none when it was refused).
    pub(super) fn add(&mut self, preset: Preset, opts: AddOptions) -> Vec<String> {
        let mut added = Vec::new();
        self.edit("add that component", |g| {
            added = components::add_preset(g, preset, &opts)?;
            Ok(format!("Added {}.", added.join(", ")))
        });
        if self.error { Vec::new() } else { added }
    }

    /// Select the first guide of the first new component, frame the new parts
    /// in the viewport and say what to adjust.
    fn show_new(&mut self, ids: &[String], message: String) {
        let Some(doc) = &self.doc else { return };
        let g = &doc.guides;
        let found: Vec<(usize, &Component)> = g
            .components
            .iter()
            .enumerate()
            .filter(|(_, c)| ids.contains(&c.id))
            .collect();
        let Some(&(first, _)) = found.first() else {
            return;
        };
        let points: Vec<Vec3> = found
            .iter()
            .flat_map(|(_, c)| c.guides.iter().map(|p| p.position))
            .collect();
        let pad = Vec3::splat(0.22 * g.body.height);
        let min = points.iter().fold(Vec3::splat(f32::MAX), |a, p| a.min(*p)) - pad;
        let max = points.iter().fold(Vec3::splat(f32::MIN), |a, p| a.max(*p)) + pad;
        self.camera.frame(min, max);
        self.pick_one(first, Some(0));
        self.mode = super::Mode::Guides;
        self.info(message);
    }
}

#[cfg(test)]
impl Studio {
    pub fn add_hind_legs(&mut self) {
        self.add_part("hind_legs");
    }
    pub fn add_fingers(&mut self) {
        self.add_part("fingers");
    }
    pub fn add_tail(&mut self) {
        self.add_part("tail");
    }
    pub fn add_tentacle(&mut self) {
        self.add_part("tentacle");
    }
    pub fn add_eyes(&mut self) {
        self.add_part("eyes");
    }
    pub fn add_face(&mut self) {
        self.add_part("face");
    }
    pub fn add_ears(&mut self) {
        self.add_part("ears");
    }
    pub fn add_ponytail(&mut self) {
        self.add_part("ponytail");
    }
    pub fn add_prop(&mut self) {
        self.add_part("prop");
    }
}
