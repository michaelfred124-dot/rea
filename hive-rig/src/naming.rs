//! Naming conventions for exported joints and controls.
//!
//! Components build with one internal scheme (the Unreal mannequin's:
//! `upperarm_l`, `spine_01`, `hand_l_ik_ctrl`). A [`Naming`] renders those names
//! into the studio's own convention when the rig is built, so every exporter and
//! the rebuild tools see the same final names. Guides keep the internal names.
//!
//! An internal name is read as tokens split on `_`: the side is an `l` or `r`
//! token, a trailing `ctrl` marks a control, the tokens before the side are the
//! `{name}` and the tokens after it the `{desc}` (`fk`, `ik`, `pole`, `aim`, a
//! chain index). Patterns place `{name}`, `{side}` and `{desc}`; empty tokens and
//! the underscores around them are dropped.
//!
//! A naming can also carry an engine's skeleton map ([`BoneMap`]): the bones
//! an engine knows (hips, spine, arms, legs, hands, fingers, head) take that
//! engine's own names, Unity Humanoid's or Mixamo's, so its avatar or
//! animations find them; every other bone and every control still follows the
//! patterns.

use std::collections::{HashMap, HashSet};

use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};

use crate::controls::{ControlParent, ControlRig};
use crate::skeleton::Skeleton;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Naming {
    /// Pattern for joints (bones), e.g. `{name}_{side}_{desc}`.
    pub joint: String,
    /// Pattern for controls, e.g. `{side}_{name}_{desc}_ctrl`.
    pub control: String,
    pub left: String,
    pub right: String,
    /// Side token for centre parts. Empty leaves it out.
    #[serde(default)]
    pub center: String,
    /// An engine's bone names for the bones it knows; the rest follow `joint`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skeleton: Option<BoneMap>,
}

/// An engine's names for the humanoid bones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoneMap {
    /// Unity's Humanoid bone names (`Hips`, `LeftUpperArm`,
    /// `LeftIndexProximal`), which Unity's avatar setup maps on its own.
    Unity,
    /// Mixamo's names (`mixamorig:Hips`, `mixamorig:LeftArm`,
    /// `mixamorig:LeftHandIndex1`), so Mixamo animations play by name. Bones
    /// Mixamo has no name for keep the `joint` pattern under the same
    /// `mixamorig:` prefix.
    Mixamo,
}

/// The humanoid bones an engine map names: (internal name, ending `_l` for a
/// sided bone, whose `_r` twin is named too; Unity's human bone; Mixamo's
/// bone), with the side written `Left` or `Right` in front of the engine names.
const HUMAN_BONES: [(&str, &str, Option<&str>); 31] = [
    ("pelvis", "Hips", Some("Hips")),
    ("spine_01", "Spine", Some("Spine")),
    ("spine_02", "Chest", Some("Spine1")),
    ("spine_03", "UpperChest", Some("Spine2")),
    ("neck_01", "Neck", Some("Neck")),
    ("head", "Head", Some("Head")),
    ("jaw", "Jaw", None),
    ("eye_l", "Eye", Some("Eye")),
    ("clavicle_l", "Shoulder", Some("Shoulder")),
    ("upperarm_l", "UpperArm", Some("Arm")),
    ("lowerarm_l", "LowerArm", Some("ForeArm")),
    ("hand_l", "Hand", Some("Hand")),
    ("thigh_l", "UpperLeg", Some("UpLeg")),
    ("calf_l", "LowerLeg", Some("Leg")),
    ("foot_l", "Foot", Some("Foot")),
    ("ball_l", "Toes", Some("ToeBase")),
    ("thumb_01_l", "Thumb Proximal", Some("HandThumb1")),
    ("thumb_02_l", "Thumb Intermediate", Some("HandThumb2")),
    ("thumb_03_l", "Thumb Distal", Some("HandThumb3")),
    ("index_01_l", "Index Proximal", Some("HandIndex1")),
    ("index_02_l", "Index Intermediate", Some("HandIndex2")),
    ("index_03_l", "Index Distal", Some("HandIndex3")),
    ("middle_01_l", "Middle Proximal", Some("HandMiddle1")),
    ("middle_02_l", "Middle Intermediate", Some("HandMiddle2")),
    ("middle_03_l", "Middle Distal", Some("HandMiddle3")),
    ("ring_01_l", "Ring Proximal", Some("HandRing1")),
    ("ring_02_l", "Ring Intermediate", Some("HandRing2")),
    ("ring_03_l", "Ring Distal", Some("HandRing3")),
    ("pinky_01_l", "Little Proximal", Some("HandPinky1")),
    ("pinky_02_l", "Little Intermediate", Some("HandPinky2")),
    ("pinky_03_l", "Little Distal", Some("HandPinky3")),
];

/// The prefix Mixamo puts on every bone.
pub const MIXAMO_PREFIX: &str = "mixamorig:";

/// Unity's human bone name for an internal bone name (`thigh_l` ->
/// `LeftUpperLeg`, `index_01_r` -> `Right Index Proximal`), as Unity's avatar
/// and Human Template files spell it. `None` for bones Unity has no slot for.
pub fn unity_human_bone(internal: &str) -> Option<String> {
    human_entry(internal).map(|(side, unity, _)| sided(side, unity))
}

/// Every internal bone name an engine map knows, both sides.
pub fn human_bones() -> impl Iterator<Item = String> {
    HUMAN_BONES
        .iter()
        .flat_map(|(name, _, _)| match name.strip_suffix("_l") {
            Some(base) => vec![name.to_string(), format!("{base}_r")],
            None => vec![name.to_string()],
        })
}

/// (side word, Unity name, Mixamo name) of an internal bone.
fn human_entry(internal: &str) -> Option<(&'static str, &'static str, Option<&'static str>)> {
    HUMAN_BONES.iter().find_map(|&(name, unity, mixamo)| {
        let Some(base) = name.strip_suffix("_l") else {
            return (name == internal).then_some(("", unity, mixamo));
        };
        match internal.strip_prefix(base)? {
            "_l" => Some(("Left", unity, mixamo)),
            "_r" => Some(("Right", unity, mixamo)),
            _ => None,
        }
    })
}

/// `Left` + `UpperArm` -> `LeftUpperArm`; `Left` + `Thumb Proximal` -> `Left Thumb Proximal`.
fn sided(side: &str, name: &str) -> String {
    match (side, name.contains(' ')) {
        ("", _) => name.to_string(),
        (_, true) => format!("{side} {name}"),
        (_, false) => format!("{side}{name}"),
    }
}

impl BoneMap {
    pub fn key(self) -> &'static str {
        match self {
            BoneMap::Unity => "unity",
            BoneMap::Mixamo => "mixamo",
        }
    }

    /// The engine's name for an internal bone, if it has one.
    pub fn name(self, internal: &str) -> Option<String> {
        let (side, unity, mixamo) = human_entry(internal)?;
        Some(match self {
            BoneMap::Unity => sided(side, unity).replace(' ', ""),
            BoneMap::Mixamo => format!("{MIXAMO_PREFIX}{side}{}", mixamo?),
        })
    }

    /// What every bone name starts with.
    pub fn prefix(self) -> &'static str {
        match self {
            BoneMap::Unity => "",
            BoneMap::Mixamo => MIXAMO_PREFIX,
        }
    }
}

/// The built-in conventions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    /// Unreal mannequin: `upperarm_l`, `upperarm_l_fk_ctrl`. Retargets cleanly in Unreal.
    Game,
    /// Side first: `L_upperarm_jnt`, `L_upperarm_fk_ctrl`, `C_spine_01_jnt`.
    Prefix,
    /// Side last, upper-case type: `upperarm_L_JNT`, `upperarm_L_fk_CTRL`, `spine_01_M_JNT`.
    Suffix,
    /// Unity Humanoid bone names (`Hips`, `LeftUpperArm`) with the game names
    /// for every other bone and the controls: Unity maps the avatar itself.
    Unity,
    /// Mixamo bone names (`mixamorig:Hips`, `mixamorig:LeftArm`), so Mixamo
    /// animations play on the rig by name; other bones keep the game names
    /// under the same prefix.
    Mixamo,
}

impl Preset {
    pub const ALL: [Preset; 5] = [
        Preset::Game,
        Preset::Prefix,
        Preset::Suffix,
        Preset::Unity,
        Preset::Mixamo,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Preset::Game => "Game (Unreal)",
            Preset::Prefix => "Side first",
            Preset::Suffix => "Side last",
            Preset::Unity => "Unity Humanoid",
            Preset::Mixamo => "Mixamo",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Preset::Game => "game",
            Preset::Prefix => "prefix",
            Preset::Suffix => "suffix",
            Preset::Unity => "unity",
            Preset::Mixamo => "mixamo",
        }
    }

    pub fn naming(self) -> Naming {
        let n = |joint: &str, control: &str, left: &str, right: &str, center: &str| Naming {
            joint: joint.into(),
            control: control.into(),
            left: left.into(),
            right: right.into(),
            center: center.into(),
            skeleton: None,
        };
        let mapped = |map: BoneMap| Naming {
            skeleton: Some(map),
            ..Preset::Game.naming()
        };
        match self {
            Preset::Unity => mapped(BoneMap::Unity),
            Preset::Mixamo => mapped(BoneMap::Mixamo),
            Preset::Game => n(
                "{name}_{side}_{desc}",
                "{name}_{side}_{desc}_ctrl",
                "l",
                "r",
                "",
            ),
            Preset::Prefix => n(
                "{side}_{name}_{desc}_jnt",
                "{side}_{name}_{desc}_ctrl",
                "L",
                "R",
                "C",
            ),
            Preset::Suffix => n(
                "{name}_{side}_{desc}_JNT",
                "{name}_{side}_{desc}_CTRL",
                "L",
                "R",
                "M",
            ),
        }
    }
}

impl std::str::FromStr for Preset {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        Preset::ALL
            .into_iter()
            .find(|p| p.key() == s.trim().to_ascii_lowercase())
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "unknown naming '{s}' (expected game, prefix, suffix, unity or mixamo)"
                )
            })
    }
}

impl Default for Naming {
    fn default() -> Self {
        Preset::Game.naming()
    }
}

/// An internal name split into its parts.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Parts<'a> {
    name: String,
    side: Option<&'a str>,
    desc: String,
}

/// Controls end in `_ctrl`, which is dropped so the pattern can place its own suffix.
fn parse(internal: &str, control: bool) -> Parts<'_> {
    let mut tokens: Vec<&str> = internal.split('_').collect();
    if control && tokens.len() > 1 && tokens.last() == Some(&"ctrl") {
        tokens.pop();
    }
    let side_at = tokens.iter().rposition(|t| *t == "l" || *t == "r");
    let (name, side, desc) = match side_at {
        Some(i) if i > 0 => (&tokens[..i], Some(tokens[i]), &tokens[i + 1..]),
        _ => (&tokens[..], None, &tokens[..0]),
    };
    Parts {
        name: name.join("_"),
        side,
        desc: desc.join("_"),
    }
}

impl Naming {
    /// The preset this naming equals, if any.
    pub fn preset(&self) -> Option<Preset> {
        Preset::ALL.into_iter().find(|p| p.naming() == *self)
    }

    pub fn is_default(&self) -> bool {
        *self == Naming::default()
    }

    fn render(&self, internal: &str, control: bool) -> String {
        let p = parse(internal, control);
        let side = match p.side {
            Some("l") => &self.left,
            Some(_) => &self.right,
            None => &self.center,
        };
        let pattern = if control { &self.control } else { &self.joint };
        let map = self.skeleton.filter(|_| !control);
        if let Some(name) = map.and_then(|m| m.name(internal)) {
            return name;
        }
        let prefix = map.map_or("", BoneMap::prefix);
        let name = pattern
            .replace("{name}", &p.name)
            .replace("{side}", side)
            .replace("{desc}", &p.desc)
            .split('_')
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join("_");
        format!("{prefix}{name}")
    }

    /// Final name of an internal bone name.
    pub fn bone(&self, internal: &str) -> String {
        if self.is_default() {
            return internal.to_string();
        }
        self.render(internal, false)
    }

    /// Final name of an internal control name.
    pub fn control(&self, internal: &str) -> String {
        if self.is_default() {
            return internal.to_string();
        }
        self.render(internal, true)
    }

    /// Check the patterns before using them.
    pub fn validate(&self) -> Result<()> {
        for (what, p) in [("joint", &self.joint), ("control", &self.control)] {
            ensure!(p.contains("{name}"), "the {what} pattern needs {{name}}");
            ensure!(
                p.contains("{side}"),
                "the {what} pattern needs {{side}} so left and right stay apart"
            );
            ensure!(
                p.contains("{desc}"),
                "the {what} pattern needs {{desc}} (fk, ik, pole and chain numbers)"
            );
            let literal = p
                .replace("{name}", "")
                .replace("{side}", "")
                .replace("{desc}", "");
            ensure!(
                literal
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_'),
                "the {what} pattern can only add letters, digits and underscores"
            );
        }
        for (what, s) in [
            ("left", &self.left),
            ("right", &self.right),
            ("centre", &self.center),
        ] {
            ensure!(
                s.chars().all(|c| c.is_ascii_alphanumeric()),
                "the {what} side can only use letters and digits"
            );
        }
        ensure!(
            !self.left.is_empty() && !self.right.is_empty() && self.left != self.right,
            "left and right need different, non-empty side names"
        );
        ensure!(
            self.center != self.left && self.center != self.right,
            "the centre side must differ from left and right"
        );
        Ok(())
    }

    /// Rename every bone and control, and every reference to them. Fails if two
    /// names collide or a name would start with a digit.
    pub fn apply(&self, skeleton: &mut Skeleton, controls: &mut ControlRig) -> Result<()> {
        if self.is_default() {
            return Ok(());
        }
        self.validate()?;
        let bones: HashMap<String, String> = skeleton
            .bones
            .iter()
            .map(|b| (b.name.clone(), self.bone(&b.name)))
            .collect();
        let ctrls: HashMap<String, String> = controls
            .controls
            .iter()
            .map(|c| (c.name.clone(), self.control(&c.name)))
            .collect();

        // Maya and Unreal keep joints and controls in one namespace.
        let mut seen = HashSet::new();
        for (internal, name) in bones.iter().chain(ctrls.iter()) {
            let bare = name.strip_prefix(MIXAMO_PREFIX).unwrap_or(name);
            if bare.is_empty() || bare.starts_with(|c: char| c.is_ascii_digit()) {
                bail!("'{internal}' would be named '{name}', which isn't a valid name");
            }
            if !seen.insert(name.as_str()) {
                bail!("two parts would both be named '{name}'; change the patterns");
            }
        }

        let bone = |n: &mut String| {
            if let Some(new) = bones.get(n.as_str()) {
                *n = new.clone();
            }
        };
        let ctrl = |n: &mut String| {
            if let Some(new) = ctrls.get(n.as_str()) {
                *n = new.clone();
            }
        };
        for b in &mut skeleton.bones {
            bone(&mut b.name);
            if let Some(t) = &mut b.twist {
                bone(&mut t.source);
            }
        }
        for c in &mut controls.controls {
            ctrl(&mut c.name);
            if let Some(b) = &mut c.bone {
                bone(b);
            }
            match &mut c.parent {
                ControlParent::Control(n) => ctrl(n),
                ControlParent::Bone(n) => bone(n),
                ControlParent::None => {}
            }
            c.spaces.iter_mut().for_each(ctrl);
        }
        controls.mirror_pairs.iter_mut().flatten().for_each(ctrl);
        for k in &mut controls.ik_chains {
            bone(&mut k.upper);
            bone(&mut k.lower);
            bone(&mut k.end);
            ctrl(&mut k.ik_control);
            ctrl(&mut k.pole_control);
            k.fk_controls.iter_mut().for_each(ctrl);
            if let Some(f) = &mut k.foot {
                bone(&mut f.ball_bone);
                ctrl(&mut f.ball_control);
            }
            if let Some(h) = &mut k.hock {
                bone(&mut h.bone);
                ctrl(&mut h.fk_control);
                ctrl(&mut h.control);
            }
            if let Some(bendy) = &mut k.bendy {
                ctrl(&mut bendy.pin);
                for seg in [&mut bendy.upper, &mut bendy.lower] {
                    bone(&mut seg.bone);
                    ctrl(&mut seg.control);
                    seg.joints.iter_mut().for_each(bone);
                }
            }
        }
        for s in &mut controls.splines {
            s.bones.iter_mut().for_each(bone);
            s.controls.iter_mut().for_each(ctrl);
            s.fk_controls.iter_mut().for_each(ctrl);
            ctrl(&mut s.settings);
        }
        if let Some(fb) = &mut controls.face_board {
            fb.rename(ctrl, bone);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{components, demo::demo_humanoid, rig};

    #[test]
    fn renders_presets() {
        let p = Preset::Prefix.naming();
        assert_eq!(p.bone("upperarm_l"), "L_upperarm_jnt");
        assert_eq!(p.bone("spine_01"), "C_spine_01_jnt");
        assert_eq!(p.bone("upperarm_twist_01_r"), "R_upperarm_twist_01_jnt");
        assert_eq!(p.bone("index_02_l"), "L_index_02_jnt");
        assert_eq!(p.control("upperarm_l_fk_ctrl"), "L_upperarm_fk_ctrl");
        assert_eq!(p.control("hand_r_ik_ctrl"), "R_hand_ik_ctrl");
        assert_eq!(p.control("root_ctrl"), "C_root_ctrl");
        assert_eq!(p.control("eye_l_aim_ctrl"), "L_eye_aim_ctrl");
        let s = Preset::Suffix.naming();
        assert_eq!(s.bone("calf_r"), "calf_R_JNT");
        assert_eq!(s.control("ball_l_ctrl"), "ball_L_CTRL");
        assert_eq!(s.control("cog_ctrl"), "cog_M_CTRL");
        assert_eq!("Prefix".parse::<Preset>().unwrap(), Preset::Prefix);
    }

    #[test]
    fn game_is_identity_and_presets_rename_every_reference() {
        let mesh = demo_humanoid();
        let (mut guides, _) = rig::detect_guides(&mesh);
        for preset in [
            components::Preset::Fingers,
            components::Preset::Eyes,
            components::Preset::Jaw,
            components::Preset::Tail,
        ] {
            components::add_preset(&mut guides, preset, &Default::default()).unwrap();
        }
        components::set_limb_options(&mut guides, true, 0.2);
        let built = components::build(&guides).unwrap();
        let game = Preset::Game.naming();
        for b in &built.skeleton.bones {
            assert_eq!(game.render(&b.name, false), b.name);
        }
        for c in &built.controls.controls {
            assert_eq!(game.render(&c.name, true), c.name);
        }

        for preset in [Preset::Prefix, Preset::Suffix] {
            let naming = preset.naming();
            let (mut sk, mut ct) = (built.skeleton.clone(), built.controls.clone());
            naming.apply(&mut sk, &mut ct).unwrap();
            let bones: HashSet<_> = sk.bones.iter().map(|b| b.name.clone()).collect();
            let ctrls: HashSet<_> = ct.controls.iter().map(|c| c.name.clone()).collect();
            assert_eq!(bones.len(), sk.bones.len());
            assert!(bones.contains(&naming.bone("pelvis")));
            for c in &ct.controls {
                if let Some(b) = &c.bone {
                    assert!(bones.contains(b), "{preset:?}: {b}");
                }
                match &c.parent {
                    ControlParent::Control(n) => assert!(ctrls.contains(n), "{n}"),
                    ControlParent::Bone(n) => assert!(bones.contains(n), "{n}"),
                    ControlParent::None => {}
                }
                assert!(c.spaces.iter().all(|s| ctrls.contains(s)));
            }
            for k in &ct.ik_chains {
                assert!(bones.contains(&k.end) && ctrls.contains(&k.ik_control));
                assert!(k.fk_controls.iter().all(|f| ctrls.contains(f)));
                let bendy = k.bendy.as_ref().unwrap();
                assert!(ctrls.contains(&bendy.pin));
                for seg in [&bendy.upper, &bendy.lower] {
                    assert!(bones.contains(&seg.bone) && ctrls.contains(&seg.control));
                    assert!(seg.joints.iter().all(|j| bones.contains(j)), "{preset:?}");
                }
            }
            assert!(!ct.mirror_pairs.is_empty());
            for [l, r] in &ct.mirror_pairs {
                assert!(ctrls.contains(l) && ctrls.contains(r), "{l} {r}");
            }
            for b in &sk.bones {
                if let Some(t) = &b.twist {
                    assert!(bones.contains(&t.source));
                }
            }
        }
    }

    #[test]
    fn rejects_bad_patterns() {
        let mut n = Preset::Prefix.naming();
        n.joint = "{name}_jnt".into();
        assert!(n.validate().is_err());
        let mut n = Preset::Prefix.naming();
        n.right = "L".into();
        assert!(n.validate().is_err());
        let mut n = Preset::Prefix.naming();
        n.control = "{side}-{name}{desc}".into();
        assert!(n.validate().is_err());
        // Joints and controls that render to the same name collide.
        let mut n = Preset::Prefix.naming();
        n.joint = "{side}_{name}_{desc}_ctrl".into();
        let built = components::build(&rig::detect_guides(&demo_humanoid()).0).unwrap();
        let (mut sk, mut ct) = (built.skeleton, built.controls);
        assert!(n.apply(&mut sk, &mut ct).is_err());
    }

    /// The demo biped with fingers, eyes, a jaw and a tail.
    fn full_biped() -> components::Built {
        let mesh = demo_humanoid();
        let (mut guides, _) = rig::detect_guides(&mesh);
        for preset in [
            components::Preset::Fingers,
            components::Preset::Eyes,
            components::Preset::Jaw,
            components::Preset::Tail,
        ] {
            components::add_preset(&mut guides, preset, &Default::default()).unwrap();
        }
        components::build(&guides).unwrap()
    }

    #[test]
    fn unity_and_mixamo_name_the_humanoid_bones() {
        let unity = Preset::Unity.naming();
        assert_eq!(unity.bone("pelvis"), "Hips");
        assert_eq!(unity.bone("spine_03"), "UpperChest");
        assert_eq!(unity.bone("upperarm_l"), "LeftUpperArm");
        assert_eq!(unity.bone("calf_r"), "RightLowerLeg");
        assert_eq!(unity.bone("ball_l"), "LeftToes");
        assert_eq!(unity.bone("index_01_l"), "LeftIndexProximal");
        assert_eq!(unity.bone("pinky_03_r"), "RightLittleDistal");
        assert_eq!(unity.bone("eye_r"), "RightEye");
        // Bones Unity has no slot for, and every control, keep the game names.
        assert_eq!(unity.bone("upperarm_twist_01_l"), "upperarm_twist_01_l");
        assert_eq!(unity.bone("tail_01"), "tail_01");
        assert_eq!(unity.control("hand_l_ik_ctrl"), "hand_l_ik_ctrl");
        assert_eq!(
            unity_human_bone("thumb_02_l").unwrap(),
            "Left Thumb Intermediate"
        );
        assert_eq!(unity_human_bone("thigh_r").unwrap(), "RightUpperLeg");
        assert_eq!(unity_human_bone("spine_04"), None);
        assert_eq!(unity_human_bone("leg2_thigh_l"), None);

        let mixamo = Preset::Mixamo.naming();
        assert_eq!(mixamo.bone("pelvis"), "mixamorig:Hips");
        assert_eq!(mixamo.bone("spine_02"), "mixamorig:Spine1");
        assert_eq!(mixamo.bone("upperarm_l"), "mixamorig:LeftArm");
        assert_eq!(mixamo.bone("lowerarm_r"), "mixamorig:RightForeArm");
        assert_eq!(mixamo.bone("thigh_l"), "mixamorig:LeftUpLeg");
        assert_eq!(mixamo.bone("ball_r"), "mixamorig:RightToeBase");
        assert_eq!(mixamo.bone("middle_02_l"), "mixamorig:LeftHandMiddle2");
        // Mixamo has no jaw: it keeps its game name in the same namespace.
        assert_eq!(mixamo.bone("jaw"), "mixamorig:jaw");
        assert_eq!(mixamo.bone("calf_twist_01_r"), "mixamorig:calf_twist_01_r");
        assert_eq!(mixamo.control("thigh_l_fk_ctrl"), "thigh_l_fk_ctrl");
        for p in [Preset::Unity, Preset::Mixamo] {
            assert_eq!(p.key().parse::<Preset>().unwrap(), p);
            assert_eq!(p.naming().preset(), Some(p));
            assert!(!p.naming().is_default());
        }

        // A whole rig renames cleanly: every humanoid bone mapped, no collisions,
        // and the mapping survives the guides file.
        let built = full_biped();
        for (preset, hips, hand) in [
            (Preset::Unity, "Hips", "LeftHand"),
            (Preset::Mixamo, "mixamorig:Hips", "mixamorig:LeftHand"),
        ] {
            let naming = preset.naming();
            let (mut sk, mut ct) = (built.skeleton.clone(), built.controls.clone());
            naming.apply(&mut sk, &mut ct).unwrap();
            assert!(sk.bone(hips).is_some() && sk.bone(hand).is_some());
            let arm = ct.ik_chains.iter().find(|k| k.name == "arm_l").unwrap();
            assert_eq!(arm.end, hand);
            let json = serde_json::to_string(&naming).unwrap();
            assert!(json.contains(&format!("\"skeleton\":\"{}\"", preset.key())));
            assert_eq!(serde_json::from_str::<Naming>(&json).unwrap(), naming);
        }
        // Older files without the field read as no map.
        let old: Naming = serde_json::from_str(
            r#"{"joint":"{name}_{side}_{desc}","control":"{name}_{side}_{desc}_ctrl","left":"l","right":"r"}"#,
        )
        .unwrap();
        assert!(old.is_default());
    }

    #[test]
    fn mixamo_names_read_back_as_guides() {
        // A rig exported with Mixamo names becomes the same guides again.
        let mesh = demo_humanoid();
        let (mut guides, _) = rig::detect_guides(&mesh);
        for p in [components::Preset::Fingers, components::Preset::Eyes] {
            components::add_preset(&mut guides, p, &Default::default()).unwrap();
        }
        let built = components::build(&guides).unwrap();
        for preset in [Preset::Mixamo, Preset::Unity] {
            let (mut sk, mut ct) = (built.skeleton.clone(), built.controls.clone());
            preset.naming().apply(&mut sk, &mut ct).unwrap();
            let joints: Vec<crate::joints::Joint> = sk
                .bones
                .iter()
                .map(|b| crate::joints::Joint {
                    name: b.name.clone(),
                    position: b.head,
                    parent: b.parent,
                })
                .collect();
            let (fresh, _) = rig::detect_guides(&mesh);
            let (g, _) = crate::joints::guides_from_joints(fresh, &joints).unwrap();
            for (id, guide, bone) in [
                ("arm_l", "shoulder", "upperarm_l"),
                ("arm_r", "elbow", "lowerarm_r"),
                ("leg_l", "knee", "calf_l"),
                ("leg_r", "ankle", "foot_r"),
                ("index_l", "j2", "index_02_l"),
                ("eyes", "eye_l", "eye_l"),
            ] {
                let at = g
                    .component(id)
                    .unwrap_or_else(|| panic!("{preset:?}: no {id}"))
                    .position(guide)
                    .unwrap();
                let want = built.skeleton.bone(bone).unwrap().head;
                assert!(at.distance(want) < 1e-4, "{preset:?} {id}.{guide}");
            }
        }
    }
}
