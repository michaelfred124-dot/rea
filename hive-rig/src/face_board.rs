//! The face board and the lip seal.
//!
//! The face board is a flat panel of slider controls standing beside the
//! head, facing the camera like the face does. Each slider is a handle that
//! slides in its track (up and down, side to side, or both), and its place in
//! the track is a value from -1 to 1 (0 to 1 for one-sided tracks). The
//! sliders pose the face joints by offsetting their controls (the animator
//! can still move each face control on top), and drive any blendshape linked
//! to them (blinks, smiles, an open jaw sculpted as a shape).
//!
//! The lip seal closes the lips together: each upper lip joint and the lower
//! lip joint under it are pulled toward the line between them (keeping the
//! gap they have at rest), by a weight that the zippers and sticky lips set.
//! A zipper closes the mouth from one corner across to the other as it goes
//! from 0 to 1. Sticky lips keep the lips together as the jaw opens until they
//! have been pulled a little apart, letting go at the corners first and the
//! middle last; the sticky slider sets how far apart that is.
//!
//! Everything here is in hive-rig's model space (Y up, metres, facing +Z, the
//! character's left at +X). Rotations are degrees about the model axes,
//! applied X first, then Y, then Z, about the joint's pivot, as the posed
//! control channels are.

use std::collections::BTreeMap;

use glam::{EulerRot, Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};

/// Which way along a slider's track a drive reads: the positive or negative
/// half of its X (side to side on the board) or Y (up and down).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Dir {
    #[serde(rename = "x+")]
    XPos,
    #[serde(rename = "x-")]
    XNeg,
    #[serde(rename = "y+")]
    YPos,
    #[serde(rename = "y-")]
    YNeg,
}

impl Dir {
    pub const ALL: [Dir; 4] = [Dir::XPos, Dir::XNeg, Dir::YPos, Dir::YNeg];

    /// 0 for X, 1 for Y.
    pub fn axis(self) -> usize {
        match self {
            Dir::XPos | Dir::XNeg => 0,
            Dir::YPos | Dir::YNeg => 1,
        }
    }

    pub fn sign(self) -> f32 {
        match self {
            Dir::XPos | Dir::YPos => 1.0,
            Dir::XNeg | Dir::YNeg => -1.0,
        }
    }

    /// How far a slider at `value` has gone this way (0 the other way).
    pub fn amount(self, value: [f32; 2]) -> f32 {
        (value[self.axis()] * self.sign()).max(0.0)
    }

    pub fn key(self) -> &'static str {
        match self {
            Dir::XPos => "x+",
            Dir::XNeg => "x-",
            Dir::YPos => "y+",
            Dir::YNeg => "y-",
        }
    }

    /// `x+`, `x-`, `y+`, `y-`, or a word: `right` (x+ on the board, the
    /// character's left), `left`, `up`, `down`.
    pub fn parse(text: &str) -> Option<Dir> {
        Some(match text.trim().to_ascii_lowercase().as_str() {
            "x+" | "+x" => Dir::XPos,
            "x-" | "-x" => Dir::XNeg,
            "y+" | "+y" | "up" => Dir::YPos,
            "y-" | "-y" | "down" => Dir::YNeg,
            _ => return None,
        })
    }

    /// The same move on the other side of the face: side to side swaps.
    pub fn mirrored(self) -> Dir {
        match self {
            Dir::XPos => Dir::XNeg,
            Dir::XNeg => Dir::XPos,
            d => d,
        }
    }
}

/// Slider values by part (`brow_l`, `jaw`, `zip_l`, ...): `[x, y]`.
pub type Values = BTreeMap<String, [f32; 2]>;

/// One slider of the board: a control whose move in the board's plane,
/// divided by `travel`, is its value, limited to the track.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Slider {
    /// What it drives, named the same under every naming convention.
    pub part: String,
    /// The control (named by the rig's convention).
    pub control: String,
    /// Metres of handle travel for a value of 1.
    pub travel: f32,
    /// The value's range side to side; `[0, 0]` locks it.
    pub x: [f32; 2],
    /// The value's range up and down; `[0, 0]` locks it.
    pub y: [f32; 2],
}

impl Slider {
    /// The slider's value for its control's move (translate channels, metres,
    /// in the board's axes), held to the track.
    pub fn value(&self, translate: Vec3) -> [f32; 2] {
        let t = self.travel.max(1e-6);
        [
            (translate.x / t).clamp(self.x[0], self.x[1]),
            (translate.y / t).clamp(self.y[0], self.y[1]),
        ]
    }

    /// The control's move held to the track (and to the board's plane).
    pub fn limit(&self, translate: Vec3) -> Vec3 {
        let [x, y] = self.value(translate);
        Vec3::new(x * self.travel, y * self.travel, 0.0)
    }

    /// Whether the handle slides side to side, up and down, or both.
    pub fn moves(&self) -> [bool; 2] {
        [self.x[0] != self.x[1], self.y[0] != self.y[1]]
    }
}

/// How far one slider direction moves and turns a face control, per unit of
/// slider: metres and degrees about the model axes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoardDrive {
    pub part: String,
    pub dir: Dir,
    #[serde(default, skip_serializing_if = "is_zero_vec")]
    pub translate: Vec3,
    #[serde(default, skip_serializing_if = "is_zero_vec")]
    pub rotate: Vec3,
}

fn is_zero_vec(v: &Vec3) -> bool {
    *v == Vec3::ZERO
}

/// The board's offset of one face control: added between the control and its
/// parent, about `pivot` (where the control turns its joint), so the control
/// still moves its joint from wherever the board puts it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoardOffset {
    pub control: String,
    /// Where the offset turns about, at rest (model space).
    pub pivot: Vec3,
    pub drives: Vec<BoardDrive>,
}

impl BoardOffset {
    /// The offset's move (metres) and turn (degrees, X then Y then Z) for
    /// these slider values.
    pub fn channels(&self, values: &Values) -> (Vec3, Vec3) {
        let mut t = Vec3::ZERO;
        let mut r = Vec3::ZERO;
        for d in &self.drives {
            let a = values.get(&d.part).map_or(0.0, |v| d.dir.amount(*v));
            t += d.translate * a;
            r += d.rotate * a;
        }
        (t, r)
    }

    /// The offset as a model-space transform: a move, then a turn about the
    /// (moved) pivot.
    pub fn matrix(&self, values: &Values) -> Mat4 {
        let (t, r) = self.channels(values);
        if t == Vec3::ZERO && r == Vec3::ZERO {
            return Mat4::IDENTITY;
        }
        Mat4::from_translation(self.pivot + t)
            * Mat4::from_quat(euler_xyz(r))
            * Mat4::from_translation(-self.pivot)
    }
}

/// Degrees about X, then Y, then Z, as one rotation.
pub fn euler_xyz(degrees: Vec3) -> Quat {
    let r = degrees * (std::f32::consts::PI / 180.0);
    Quat::from_euler(EulerRot::ZYX, r.z, r.y, r.x)
}

/// An upper lip joint and the lower lip joint under it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LipPair {
    pub upper: String,
    pub lower: String,
    pub upper_control: String,
    pub lower_control: String,
    /// Across the mouth: 0 at the left corner, 1 at the right.
    pub along: f32,
    /// The lips' gap at rest (metres).
    pub rest_gap: f32,
}

impl LipPair {
    /// How near the middle of the mouth: 0 at a corner, 1 in the middle.
    pub fn centre(&self) -> f32 {
        1.0 - (2.0 * self.along - 1.0).abs()
    }
}

/// Zippers and sticky lips on the lip joints.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LipSeal {
    /// The bone the lips' rest gap turns with (the head).
    pub frame_bone: String,
    /// How far apart (metres) fully sticky lips are pulled before the middle
    /// of the mouth lets go.
    pub release: f32,
    pub pairs: Vec<LipPair>,
}

/// A zipper's closing edge: how much of the mouth's width it blends over.
pub const ZIP_FALLOFF: f32 = 0.25;
/// Sticky lips let go over this share of their release distance.
pub const STICKY_FADE: f32 = 0.5;

/// The board parts the lip seal reads: `[x, y]` amounts (0 to 1).
pub const ZIP_L: &str = "zip_l";
pub const ZIP_R: &str = "zip_r";
pub const STICKY: &str = "sticky";

impl LipSeal {
    /// How strongly one lip pair is sealed (0 open, 1 closed): the zippers'
    /// closing from either corner and the stickiness for this gap between
    /// the lips (metres beyond their rest gap), whichever is more.
    pub fn weight(&self, pair: &LipPair, zip: [f32; 2], sticky: f32, gap: f32) -> f32 {
        let f = ZIP_FALLOFF;
        let from_left = zip[0] * (1.0 + f) - pair.along;
        let from_right = zip[1] * (1.0 + f) - (1.0 - pair.along);
        let zipped = (from_left.max(from_right) / f).clamp(0.0, 1.0);
        let c = pair.centre();
        let reach = sticky.clamp(0.0, 1.0) * self.release * c;
        let fade = (STICKY_FADE * self.release * c).max(1e-6);
        let stuck = ((reach - gap.max(0.0)) / fade).clamp(0.0, 1.0);
        zipped.max(stuck)
    }

    /// The zipper and sticky amounts from the board's values.
    pub fn inputs(values: &Values) -> ([f32; 2], f32) {
        let get = |part: &str, dir: Dir| values.get(part).map_or(0.0, |v| dir.amount(*v));
        (
            [get(ZIP_L, Dir::XNeg), get(ZIP_R, Dir::XPos)],
            get(STICKY, Dir::XPos),
        )
    }

    /// Where a sealed lip pair's joints go: each from where its control put
    /// it toward the line between the two, at its rest gap from it (turned
    /// with the frame bone). `upper`, `lower` are where the controls put the
    /// joints, `rest` their rest places.
    pub fn sealed(
        upper: Vec3,
        lower: Vec3,
        rest: [Vec3; 2],
        frame: Quat,
        weight: f32,
    ) -> [Vec3; 2] {
        let mid = (upper + lower) * 0.5;
        let rest_mid = (rest[0] + rest[1]) * 0.5;
        let up = mid + frame * (rest[0] - rest_mid);
        let down = mid + frame * (rest[1] - rest_mid);
        [upper.lerp(up, weight), lower.lerp(down, weight)]
    }
}

/// The face board of a rig.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FaceBoard {
    /// The panel's control: moving it moves the whole board.
    pub frame: String,
    pub sliders: Vec<Slider>,
    /// The face controls the sliders move and turn.
    pub offsets: Vec<BoardOffset>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seal: Option<LipSeal>,
}

impl FaceBoard {
    pub fn slider(&self, control: &str) -> Option<&Slider> {
        self.sliders.iter().find(|s| s.control == control)
    }

    pub fn part(&self, part: &str) -> Option<&Slider> {
        self.sliders.iter().find(|s| s.part == part)
    }

    pub fn offset(&self, control: &str) -> Option<&BoardOffset> {
        self.offsets.iter().find(|o| o.control == control)
    }

    /// Every slider's value, from the translate channels of its control.
    pub fn values(&self, translate: impl Fn(&str) -> Vec3) -> Values {
        self.sliders
            .iter()
            .map(|s| (s.part.clone(), s.value(translate(&s.control))))
            .collect()
    }

    /// Whether the board's controls include this one (the frame or a slider).
    pub fn owns(&self, control: &str) -> bool {
        self.frame == control || self.slider(control).is_some()
    }

    /// Rename controls and bones (a naming convention).
    pub fn rename(&mut self, ctrl: impl Fn(&mut String), bone: impl Fn(&mut String)) {
        ctrl(&mut self.frame);
        for s in &mut self.sliders {
            ctrl(&mut s.control);
        }
        for o in &mut self.offsets {
            ctrl(&mut o.control);
        }
        if let Some(seal) = &mut self.seal {
            bone(&mut seal.frame_bone);
            for p in &mut seal.pairs {
                bone(&mut p.upper);
                bone(&mut p.lower);
                ctrl(&mut p.upper_control);
                ctrl(&mut p.lower_control);
            }
        }
    }
}

/// A blendshape driven by a board slider: its weight is `weight` times how
/// far the slider has gone in `dir` (held to 0..1).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoardLink {
    pub part: String,
    pub dir: Dir,
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub weight: f32,
}

fn one() -> f32 {
    1.0
}

fn is_one(v: &f32) -> bool {
    *v == 1.0
}

impl BoardLink {
    pub fn weight_at(&self, values: &Values) -> f32 {
        let a = values.get(&self.part).map_or(0.0, |v| self.dir.amount(*v));
        (a * self.weight).clamp(0.0, 1.0)
    }
}

/// One slider of the layout: its part, where its handle rests on the panel
/// (in units of the travel, from the panel's middle; +x is the character's
/// left, shown on the right as the face is), its ranges and travel (in units
/// of the travel step).
pub struct Slot {
    pub part: &'static str,
    pub at: [f32; 2],
    pub x: [f32; 2],
    pub y: [f32; 2],
    pub travel: f32,
}

const BOTH: [f32; 2] = [-1.0, 1.0];
const NONE: [f32; 2] = [0.0, 0.0];

/// The board's layout, laid out like a face: brows at the top, then the
/// lids, nose and cheeks, the mouth, the jaw, and the zippers and sticky lips
/// along the bottom.
pub const LAYOUT: [Slot; 17] = [
    Slot {
        part: "brow_l",
        at: [3.6, 7.4],
        x: BOTH,
        y: BOTH,
        travel: 1.0,
    },
    Slot {
        part: "brow_r",
        at: [-3.6, 7.4],
        x: BOTH,
        y: BOTH,
        travel: 1.0,
    },
    Slot {
        part: "lid_l",
        at: [3.6, 4.4],
        x: NONE,
        y: BOTH,
        travel: 1.0,
    },
    Slot {
        part: "lid_r",
        at: [-3.6, 4.4],
        x: NONE,
        y: BOTH,
        travel: 1.0,
    },
    Slot {
        part: "nose",
        at: [0.0, 4.4],
        x: NONE,
        y: BOTH,
        travel: 1.0,
    },
    Slot {
        part: "cheek_l",
        at: [5.8, 1.6],
        x: NONE,
        y: BOTH,
        travel: 1.0,
    },
    Slot {
        part: "cheek_r",
        at: [-5.8, 1.6],
        x: NONE,
        y: BOTH,
        travel: 1.0,
    },
    Slot {
        part: "lip_up",
        at: [0.0, 1.8],
        x: NONE,
        y: BOTH,
        travel: 1.0,
    },
    Slot {
        part: "corner_l",
        at: [3.6, -0.9],
        x: BOTH,
        y: BOTH,
        travel: 1.0,
    },
    Slot {
        part: "corner_r",
        at: [-3.6, -0.9],
        x: BOTH,
        y: BOTH,
        travel: 1.0,
    },
    Slot {
        part: "mouth",
        at: [0.0, -1.2],
        x: BOTH,
        y: BOTH,
        travel: 1.0,
    },
    Slot {
        part: "lip_low",
        at: [0.0, -4.2],
        x: NONE,
        y: BOTH,
        travel: 1.0,
    },
    Slot {
        part: "jaw",
        at: [0.0, -6.8],
        x: BOTH,
        y: BOTH,
        travel: 1.0,
    },
    Slot {
        part: ZIP_L,
        at: [5.6, -9.0],
        x: [-1.0, 0.0],
        y: NONE,
        travel: 2.4,
    },
    Slot {
        part: ZIP_R,
        at: [-5.6, -9.0],
        x: [0.0, 1.0],
        y: NONE,
        travel: 2.4,
    },
    Slot {
        part: STICKY,
        at: [-1.2, -9.0],
        x: [0.0, 1.0],
        y: NONE,
        travel: 2.4,
    },
    // Not a slider: where the panel's own control sits, above its top edge.
    Slot {
        part: "",
        at: [0.0, 11.2],
        x: NONE,
        y: NONE,
        travel: 0.0,
    },
];

/// The panel's half width and half height, in travel steps.
pub const PANEL: [f32; 2] = [7.0, 10.2];
/// A travel step, as a share of the body height.
pub const STEP: f32 = 0.0085;
/// A handle's size, in travel steps.
pub const HANDLE: f32 = 0.5;

/// The board's lines at rest, in model space: the panel's edge and a track
/// around each slider's handle. `center` is the panel's middle, `step` a
/// travel step in metres.
pub fn panel_lines(center: Vec3, step: f32) -> Vec<Vec<Vec3>> {
    let rect = |cx: f32, cy: f32, hx: f32, hy: f32| -> Vec<Vec3> {
        let p = |x: f32, y: f32| center + Vec3::new(x, y, 0.0) * step;
        vec![
            p(cx - hx, cy - hy),
            p(cx + hx, cy - hy),
            p(cx + hx, cy + hy),
            p(cx - hx, cy + hy),
            p(cx - hx, cy - hy),
        ]
    };
    let mut out = vec![rect(0.0, 0.0, PANEL[0], PANEL[1])];
    let pad = HANDLE + 0.15;
    for s in LAYOUT.iter().filter(|s| !s.part.is_empty()) {
        let span = |r: [f32; 2], at: f32| {
            let (lo, hi) = (at + r[0] * s.travel, at + r[1] * s.travel);
            ((lo + hi) * 0.5, (hi - lo) * 0.5 + pad)
        };
        let (cx, hx) = span(s.x, s.at[0]);
        let (cy, hy) = span(s.y, s.at[1]);
        out.push(rect(cx, cy, hx, hy));
        // A tick where the handle rests, on two-way tracks.
        if s.x == BOTH || s.y == BOTH {
            let p = |x: f32, y: f32| center + Vec3::new(x, y, 0.0) * step;
            let (x, y) = (s.at[0], s.at[1]);
            if s.y == BOTH {
                out.push(vec![p(x - hx, y), p(x - hx + 0.3, y)]);
                out.push(vec![p(x + hx - 0.3, y), p(x + hx, y)]);
            }
            if s.x == BOTH {
                out.push(vec![p(x, y - hy), p(x, y - hy + 0.3)]);
                out.push(vec![p(x, y + hy - 0.3), p(x, y + hy)]);
            }
        }
    }
    out
}

/// Words a blendshape's name is matched on for each slider move, with the side
/// taken off (`eyeBlinkLeft`, `blink_l` and `Blink.L` all read as `blink` on
/// the left). Sided moves only take sided names.
const SHAPE_WORDS: &[(&str, Dir, &[&str])] = &[
    (
        "brow",
        Dir::YPos,
        &[
            "browup",
            "browraise",
            "browouterup",
            "browsup",
            "browraiser",
        ],
    ),
    (
        "brow",
        Dir::YNeg,
        &[
            "browdown",
            "browlower",
            "browsdown",
            "browlowerer",
            "frownbrow",
        ],
    ),
    (
        "brow",
        Dir::XNeg,
        &["browin", "browsqueeze", "browtogether"],
    ),
    (
        "lid",
        Dir::YNeg,
        &[
            "blink",
            "eyeblink",
            "eyeclose",
            "eyeclosed",
            "lidclose",
            "eyesclosed",
        ],
    ),
    (
        "lid",
        Dir::YPos,
        &["eyewide", "lidwide", "eyeopen", "eyewiden"],
    ),
    (
        "cheek",
        Dir::YPos,
        &["cheekraise", "cheeksquint", "cheekup", "cheekraiser"],
    ),
    (
        "corner",
        Dir::YPos,
        &["smile", "mouthsmile", "cornerup", "lipcornerup"],
    ),
    (
        "corner",
        Dir::YNeg,
        &["frown", "mouthfrown", "cornerdown", "lipcornerdown"],
    ),
    (
        "corner",
        Dir::XPos,
        &["mouthstretch", "mouthwide", "cornerout", "lipstretch"],
    ),
    (
        "corner",
        Dir::XNeg,
        &["cornerin", "mouthnarrow", "lipnarrow"],
    ),
    (
        "jaw",
        Dir::YNeg,
        &["jawopen", "mouthopen", "jawdrop", "open"],
    ),
    ("jaw", Dir::XPos, &["jawleft"]),
    ("jaw", Dir::XNeg, &["jawright"]),
    (
        "nose",
        Dir::YPos,
        &["sneer", "nosesneer", "noseup", "nosewrinkle"],
    ),
    (
        "lip_up",
        Dir::YPos,
        &["upperlipup", "upperlipraise", "mouthupperup", "lipupperup"],
    ),
    (
        "lip_low",
        Dir::YNeg,
        &[
            "lowerlipdown",
            "lowerlipdepress",
            "mouthlowerdown",
            "liplowerdown",
        ],
    ),
    ("mouth", Dir::XPos, &["mouthleft"]),
    ("mouth", Dir::XNeg, &["mouthright"]),
];

/// Parts with a left and a right slider.
const SIDED: [&str; 4] = ["brow", "lid", "cheek", "corner"];

/// The word tokens of a name: split on `_`, `-`, `.`, spaces and lower-to-
/// upper case changes, lowercased.
fn tokens(name: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut prev_lower = false;
    for ch in name.chars() {
        if !ch.is_ascii_alphanumeric() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            prev_lower = false;
            continue;
        }
        if ch.is_ascii_uppercase() && prev_lower && !cur.is_empty() {
            out.push(std::mem::take(&mut cur));
        }
        prev_lower = ch.is_ascii_lowercase() || ch.is_ascii_digit();
        cur.push(ch.to_ascii_lowercase());
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// The board move a blendshape's name says it is (`blink_l` is the left lid
/// closing; `jawOpen` the jaw opening), or `None`.
pub fn match_shape(name: &str) -> Option<BoardLink> {
    let mut t = tokens(name);
    let side = match t.last().map(String::as_str) {
        Some("l" | "left" | "lf" | "lt") => Some("l"),
        Some("r" | "right" | "rt") => Some("r"),
        _ => None,
    };
    if side.is_some() {
        t.pop();
    }
    // A side word right before the end too: `eye_blink_left` and `browDownLeft`.
    let base: String = t.concat();
    for (part, dir, words) in SHAPE_WORDS {
        if !words.contains(&base.as_str()) {
            continue;
        }
        let sided = SIDED.contains(part);
        return match (sided, side) {
            (true, Some(s)) => {
                // The right side's side-to-side moves are mirrored on the board.
                let dir = if s == "r" { dir.mirrored() } else { *dir };
                Some(BoardLink {
                    part: format!("{part}_{s}"),
                    dir,
                    weight: 1.0,
                })
            }
            (false, None) => Some(BoardLink {
                part: part.to_string(),
                dir: *dir,
                weight: 1.0,
            }),
            _ => None,
        };
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shape_names_match_board_moves() {
        let m = |n: &str| match_shape(n).map(|l| (l.part, l.dir));
        assert_eq!(m("blink_l"), Some(("lid_l".into(), Dir::YNeg)));
        assert_eq!(m("eyeBlinkRight"), Some(("lid_r".into(), Dir::YNeg)));
        assert_eq!(m("Smile.L"), Some(("corner_l".into(), Dir::YPos)));
        assert_eq!(m("jawOpen"), Some(("jaw".into(), Dir::YNeg)));
        assert_eq!(m("browDownLeft"), Some(("brow_l".into(), Dir::YNeg)));
        // Side to side mirrors on the right: squeezing the right brow in is x+.
        assert_eq!(m("brow_in_r"), Some(("brow_r".into(), Dir::XPos)));
        // A sided move needs a side, a centred one none.
        assert_eq!(m("blink"), None);
        assert_eq!(m("jaw_open_l"), None);
        assert_eq!(m("bicep_bulge"), None);
    }

    #[test]
    fn sliders_hold_to_their_tracks() {
        let s = Slider {
            part: "zip_l".into(),
            control: "c".into(),
            travel: 0.02,
            x: [-1.0, 0.0],
            y: [0.0, 0.0],
        };
        assert_eq!(s.value(Vec3::new(-0.01, 0.5, 0.3)), [-0.5, 0.0]);
        assert_eq!(s.value(Vec3::new(0.05, 0.0, 0.0)), [0.0, 0.0]);
        assert_eq!(
            s.limit(Vec3::new(-0.08, 0.1, 0.1)),
            Vec3::new(-0.02, 0.0, 0.0)
        );
        assert_eq!(Dir::XNeg.amount(s.value(Vec3::new(-0.01, 0.0, 0.0))), 0.5);
    }

    #[test]
    fn zippers_close_from_the_corners_and_sticky_lips_let_go_at_the_corners_first() {
        let seal = LipSeal {
            frame_bone: "head".into(),
            release: 0.01,
            pairs: Vec::new(),
        };
        let pair = |along: f32| LipPair {
            upper: String::new(),
            lower: String::new(),
            upper_control: String::new(),
            lower_control: String::new(),
            along,
            rest_gap: 0.0,
        };
        let (left, mid, right) = (pair(0.2), pair(0.5), pair(0.8));
        // Nothing on: open.
        assert_eq!(seal.weight(&mid, [0.0, 0.0], 0.0, 0.0), 0.0);
        // The left zipper half way closes the left side, not the right.
        assert_eq!(seal.weight(&left, [0.5, 0.0], 0.0, 0.0), 1.0);
        assert_eq!(seal.weight(&right, [0.5, 0.0], 0.0, 0.0), 0.0);
        // Either zipper all the way closes the whole mouth.
        for p in [&left, &mid, &right] {
            assert_eq!(seal.weight(p, [0.0, 1.0], 0.0, 0.0), 1.0);
        }
        // Sticky: held while the lips are close, the corners letting go first.
        assert_eq!(seal.weight(&mid, [0.0, 0.0], 1.0, 0.0), 1.0);
        let gap = 0.005;
        assert!(seal.weight(&left, [0.0; 2], 1.0, gap) < seal.weight(&mid, [0.0; 2], 1.0, gap));
        assert_eq!(seal.weight(&mid, [0.0; 2], 1.0, 0.02), 0.0);
        // Less stickiness lets go sooner.
        assert!(seal.weight(&mid, [0.0; 2], 0.4, gap) < seal.weight(&mid, [0.0; 2], 1.0, gap));
        // Sealed lips meet at their rest gap from the line between them.
        let rest = [Vec3::new(0.0, 1.0, 0.0), Vec3::new(0.0, 0.98, 0.0)];
        let [u, d] = LipSeal::sealed(rest[0], Vec3::new(0.0, 0.9, 0.0), rest, Quat::IDENTITY, 1.0);
        assert!((u - Vec3::new(0.0, 0.96, 0.0)).length() < 1e-6, "{u}");
        assert!((d - Vec3::new(0.0, 0.94, 0.0)).length() < 1e-6, "{d}");
    }
}
