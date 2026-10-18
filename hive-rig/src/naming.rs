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
}

impl Preset {
    pub const ALL: [Preset; 3] = [Preset::Game, Preset::Prefix, Preset::Suffix];

    pub fn label(self) -> &'static str {
        match self {
            Preset::Game => "Game (Unreal)",
            Preset::Prefix => "Side first",
            Preset::Suffix => "Side last",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Preset::Game => "game",
            Preset::Prefix => "prefix",
            Preset::Suffix => "suffix",
        }
    }

    pub fn naming(self) -> Naming {
        let n = |joint: &str, control: &str, left: &str, right: &str, center: &str| Naming {
            joint: joint.into(),
            control: control.into(),
            left: left.into(),
            right: right.into(),
            center: center.into(),
        };
        match self {
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
                anyhow::anyhow!("unknown naming '{s}' (expected game, prefix or suffix)")
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
        pattern
            .replace("{name}", &p.name)
            .replace("{side}", side)
            .replace("{desc}", &p.desc)
            .split('_')
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join("_")
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
            if name.is_empty() || name.starts_with(|c: char| c.is_ascii_digit()) {
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
        }
        for s in &mut controls.splines {
            s.bones.iter_mut().for_each(bone);
            s.controls.iter_mut().for_each(ctrl);
            s.fk_controls.iter_mut().for_each(ctrl);
            ctrl(&mut s.settings);
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
}
