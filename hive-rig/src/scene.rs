//! Several characters in one file: a scene.
//!
//! A scene (`<name>.scene.json`) holds characters, each with its own guides,
//! where its mesh comes from, where it stands in the scene, and the rig it was
//! last built into. Characters are built and exported together or one at a
//! time; each gets its usual files (`<character>.glb`, builder scripts...) and
//! the scene gets builders of its own (`<scene>_scene_blender.py`, `_maya.py`,
//! `_unreal.py`) that build every character in one Blender, Maya or Unreal
//! scene and place it.
//!
//! A scene can also hold props across characters: a control of one character
//! (typically its prop socket, a one-bone chain in a hand) is held by a bone of
//! another, as a sword on character A's hand socket held in character B's hand.
//! The scene builders give that control a `held` setting from 0 (it stays on
//! its own socket) to 1 (it follows the holder's bone).

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail, ensure};
use glam::Vec3;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::components::{self, Component, Guide, Guides, Kind, Options};
use crate::export::{self, Target, sanitize_name};
use crate::orient::{Choice, Orientation};
use crate::rig::{Rig, RigOptions};
use crate::skeleton::Side;

pub const SCENE_FORMAT: &str = "hive-rig-scene";
pub const SCENE_VERSION: u32 = 1;
/// Scene file names end with this.
pub const SCENE_SUFFIX: &str = ".scene.json";

const BLENDER_TEMPLATE: &str = include_str!("../templates/scene_blender.py");
const MAYA_TEMPLATE: &str = include_str!("../templates/scene_maya.py");
const UNREAL_TEMPLATE: &str = include_str!("../templates/scene_unreal.py");
const JSON_SLOT: &str = "__HIVE_SCENE_JSON__";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scene {
    pub format: String,
    pub version: u32,
    #[serde(default, rename = "hive_rig", serialize_with = "crate::version::stamp")]
    pub written_by: Option<String>,
    pub name: String,
    pub characters: Vec<SceneCharacter>,
    /// Props held across characters.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holds: Vec<Hold>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneCharacter {
    /// The character's name: its rig's name and its files' names.
    pub name: String,
    /// Where its mesh comes from: a character file (.glb, .gltf, .fbx, .obj) or
    /// a saved rig (.hive.json), relative to the scene file, or a built-in
    /// demo (`demo`, `demo:dog`, `demo:bird`...).
    pub source: String,
    /// How the character stands in its source file (detected when it was added).
    #[serde(default, skip_serializing_if = "Orientation::is_standard")]
    pub orientation: Orientation,
    /// Where it stands in the scene, in metres (Y up, model space).
    #[serde(default)]
    pub offset: Vec3,
    pub guides: Guides,
    /// Its last built rig (`.hive.json`), relative to the scene file. A rebuild
    /// keeps the weights painted on it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rig: Option<String>,
}

/// A control of one character held by a bone of another.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Hold {
    /// The prop: a character and one of its controls (internal name, such as
    /// `prop_r_01_ctrl`).
    pub prop: PropRef,
    /// The hand: another character and one of its bones (internal name, such
    /// as `hand_l`).
    pub holder: BoneRef,
    /// The `held` setting it starts with: 1 in the holder's hand, 0 on its own
    /// socket.
    #[serde(default = "one")]
    pub held: f32,
}

fn one() -> f32 {
    1.0
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PropRef {
    pub character: String,
    pub control: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoneRef {
    pub character: String,
    pub bone: String,
}

impl Scene {
    pub fn new(name: &str) -> Scene {
        Scene {
            format: SCENE_FORMAT.into(),
            version: SCENE_VERSION,
            written_by: None,
            name: sanitize_name(name),
            characters: Vec::new(),
            holds: Vec::new(),
        }
    }

    pub fn load(path: &Path) -> Result<Scene> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Scene::parse(&text).with_context(|| format!("reading {}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Scene> {
        let s: Scene = serde_json::from_str(text)?;
        ensure!(
            s.format == SCENE_FORMAT,
            "not a hive-rig scene (format is '{}')",
            s.format
        );
        ensure!(
            s.version <= SCENE_VERSION,
            "written by a newer hive-rig (scene version {})",
            s.version
        );
        s.validate()?;
        Ok(s)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        std::fs::write(path, serde_json::to_vec_pretty(self)?)
            .with_context(|| format!("writing {}", path.display()))
    }

    pub fn character(&self, name: &str) -> Option<&SceneCharacter> {
        self.characters.iter().find(|c| c.name == name)
    }

    pub fn character_mut(&mut self, name: &str) -> Option<&mut SceneCharacter> {
        self.characters.iter_mut().find(|c| c.name == name)
    }

    fn index(&self, name: &str) -> Result<usize> {
        self.characters
            .iter()
            .position(|c| c.name == name)
            .ok_or_else(|| {
                let names: Vec<&str> = self.characters.iter().map(|c| c.name.as_str()).collect();
                anyhow::anyhow!(
                    "no character named '{name}' in the scene (it has {})",
                    names.join(", ")
                )
            })
    }

    /// Names are unique and safe, and every hold names two characters of the
    /// scene. (Whether the control and bone exist is [`Scene::check_holds`].)
    pub fn validate(&self) -> Result<()> {
        ensure!(!self.name.is_empty(), "a scene needs a name");
        for (i, c) in self.characters.iter().enumerate() {
            ensure!(
                sanitize_name(&c.name) == c.name,
                "'{}' can't name a character: use letters, digits and '_'",
                c.name
            );
            ensure!(
                !self.characters[..i].iter().any(|o| o.name == c.name),
                "two characters are named '{}'",
                c.name
            );
        }
        for h in &self.holds {
            self.index(&h.prop.character)?;
            self.index(&h.holder.character)?;
            ensure!(
                h.prop.character != h.holder.character,
                "{} would hold its own {}: use a space switch within one character",
                h.prop.character,
                h.prop.control
            );
            ensure!(
                (0.0..=1.0).contains(&h.held),
                "held is from 0 to 1, not {}",
                h.held
            );
        }
        for (i, h) in self.holds.iter().enumerate() {
            ensure!(
                !self.holds[..i].iter().any(|o| o.prop == h.prop),
                "{}'s {} is held twice",
                h.prop.character,
                h.prop.control
            );
        }
        Ok(())
    }

    /// Every held control and holding bone exists in its character's rig.
    pub fn check_holds(&self) -> Result<()> {
        for h in &self.holds {
            let a = self.character(&h.prop.character).unwrap();
            let b = self.character(&h.holder.character).unwrap();
            let built_a = components::build(&a.guides)
                .with_context(|| format!("{}'s guides don't build", a.name))?;
            ensure!(
                built_a.controls.control(&h.prop.control).is_some(),
                "{} has no control named {}",
                a.name,
                h.prop.control
            );
            let built_b = components::build(&b.guides)
                .with_context(|| format!("{}'s guides don't build", b.name))?;
            ensure!(
                built_b.skeleton.bone(&h.holder.bone).is_some(),
                "{} has no bone named {}",
                b.name,
                h.holder.bone
            );
        }
        Ok(())
    }

    /// Add a character, refusing a name already in the scene.
    pub fn add(&mut self, c: SceneCharacter) -> Result<()> {
        ensure!(
            self.character(&c.name).is_none(),
            "the scene already has a character named '{}'",
            c.name
        );
        self.characters.push(c);
        self.validate()
    }

    /// Remove a character and the holds that name it.
    pub fn remove(&mut self, name: &str) -> Result<SceneCharacter> {
        let i = self.index(name)?;
        self.holds
            .retain(|h| h.prop.character != name && h.holder.character != name);
        Ok(self.characters.remove(i))
    }

    /// Let `holder` hold `prop` (replacing any hold on that prop).
    pub fn hold(&mut self, hold: Hold) -> Result<()> {
        let before = self.holds.clone();
        self.holds.retain(|h| h.prop != hold.prop);
        self.holds.push(hold);
        if let Err(e) = self.validate().and_then(|_| self.check_holds()) {
            self.holds = before;
            return Err(e);
        }
        Ok(())
    }

    /// Stop holding a prop. True if it was held.
    pub fn release(&mut self, prop: &PropRef) -> bool {
        let before = self.holds.len();
        self.holds.retain(|h| &h.prop != prop);
        self.holds.len() != before
    }

    /// A character name not yet used, from `wanted`.
    pub fn free_name(&self, wanted: &str) -> String {
        let base = sanitize_name(wanted);
        if self.character(&base).is_none() {
            return base;
        }
        (2..)
            .map(|n| format!("{base}_{n}"))
            .find(|n| self.character(n).is_none())
            .unwrap()
    }
}

/// `NAME=FILE[@x,y,z]` as given on the command line.
pub fn parse_character_arg(text: &str) -> Result<(String, String, Vec3)> {
    let (name, rest) = text
        .split_once('=')
        .ok_or_else(|| anyhow::anyhow!("'{text}': give a character as NAME=FILE[@x,y,z]"))?;
    let (source, offset) = match rest.rsplit_once('@') {
        Some((s, at)) => {
            let v: Vec<f32> = at
                .split(',')
                .map(|x| x.trim().parse::<f32>())
                .collect::<std::result::Result<_, _>>()
                .map_err(|_| anyhow::anyhow!("'{at}' isn't an x,y,z place"))?;
            ensure!(v.len() == 3, "'{at}' isn't an x,y,z place");
            (s, Vec3::new(v[0], v[1], v[2]))
        }
        None => (rest, Vec3::ZERO),
    };
    ensure!(!source.is_empty(), "'{text}' names no file");
    Ok((name.trim().to_string(), source.trim().to_string(), offset))
}

/// A built-in demo source: `demo` (the humanoid) or `demo:<creature>`.
fn demo_source(source: &str) -> Option<Result<Option<crate::demo::Creature>>> {
    let rest = source.strip_prefix("demo")?;
    if rest.is_empty() {
        return Some(Ok(None));
    }
    let name = rest.strip_prefix(':')?;
    Some(
        crate::demo::Creature::from_name(name)
            .map(Some)
            .ok_or_else(|| anyhow::anyhow!("no built-in creature named '{name}'")),
    )
}

/// The file a source names, relative to the scene's folder.
fn source_path(source: &str, dir: &Path) -> PathBuf {
    let p = Path::new(source);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        dir.join(p)
    }
}

/// A character for the scene from a source (see [`SceneCharacter::source`]),
/// its guides placed as `hive-rig rig` places them. `dir` is the scene's folder.
pub fn character_from_source(
    name: &str,
    source: &str,
    dir: &Path,
    offset: Vec3,
) -> Result<SceneCharacter> {
    let (guides, orientation) = match demo_source(source) {
        Some(creature) => match creature? {
            Some(c) => (c.guides(), Orientation::STANDARD),
            None => (
                crate::rig::detect_guides(&crate::demo::demo_humanoid()).0,
                Orientation::STANDARD,
            ),
        },
        None => {
            let path = source_path(source, dir);
            if is_rig_file(&path) {
                let rig = Rig::load(&path)?;
                (rig.guides, rig.orientation)
            } else {
                let ch = crate::import::load_character(&path, Choice::default())?;
                ch.mesh.validate()?;
                let (g, _, _) =
                    crate::rig::character_guides(&ch.mesh, &ch.joints, ch.detection.body);
                (g, ch.detection.orientation)
            }
        }
    };
    Ok(SceneCharacter {
        name: sanitize_name(name),
        source: source.to_string(),
        orientation,
        offset,
        guides,
        rig: None,
    })
}

fn is_rig_file(path: &Path) -> bool {
    path.to_string_lossy()
        .to_ascii_lowercase()
        .ends_with(".hive.json")
}

/// The character's mesh, in hive-rig's frame, from its source.
pub fn load_mesh(c: &SceneCharacter, dir: &Path) -> Result<crate::mesh::Mesh> {
    if let Some(creature) = demo_source(&c.source) {
        return Ok(match creature? {
            Some(cr) => cr.mesh(),
            None => crate::demo::demo_humanoid(),
        });
    }
    let path = source_path(&c.source, dir);
    if is_rig_file(&path) {
        return Ok(Rig::load(&path)?.mesh);
    }
    let choice = Choice {
        up: Some(c.orientation.up),
        forward: Some(c.orientation.forward),
    };
    let ch = crate::import::load_character(&path, choice)
        .with_context(|| format!("loading {}'s mesh", c.name))?;
    Ok(ch.mesh)
}

/// The character's last built rig, if its file is there.
pub fn saved_rig(c: &SceneCharacter, dir: &Path) -> Option<Rig> {
    let path = source_path(c.rig.as_deref()?, dir);
    Rig::load(&path).ok()
}

/// Build the rig of one character: its mesh, its guides, and the weights
/// painted on its last built rig.
pub fn build_character(c: &SceneCharacter, dir: &Path, opts: RigOptions) -> Result<Rig> {
    let mesh = load_mesh(c, dir)?;
    let mut rig = crate::rig::build_rig(mesh, &c.name, c.guides.clone(), opts)
        .with_context(|| format!("building {}", c.name))?;
    rig.orientation = c.orientation;
    if let Some(old) = saved_rig(c, dir)
        && old.mesh.vertex_count() == rig.mesh.vertex_count()
    {
        let auto = old.auto_weights(opts.skin);
        crate::rig::keep_weights(&old, Some(&auto), &mut rig);
        rig.pose = old.pose;
    }
    Ok(rig)
}

/// One-bone prop socket in `hand` (such as `hand_r`): a chain component named
/// `id`, whose control animators move the prop with. Returns the control's name.
pub fn add_prop_socket(g: &mut Guides, hand: &str, id: &str) -> Result<String> {
    ensure!(g.component(id).is_none(), "{id} already exists");
    let built = components::build(g)?;
    let bone = built
        .skeleton
        .bone(hand)
        .ok_or_else(|| anyhow::anyhow!("the rig has no bone named {hand}"))?;
    let side = if hand.ends_with("_l") {
        Side::Left
    } else if hand.ends_with("_r") {
        Side::Right
    } else {
        Side::Center
    };
    let start = bone.head.lerp(bone.tail, 0.5);
    let length = 0.12 * g.body.height;
    g.components.push(Component {
        id: id.to_string(),
        kind: Kind::Chain,
        side,
        parent: Some(hand.to_string()),
        guides: (0..=1)
            .map(|k| Guide {
                name: format!("j{}", k + 1),
                position: start + Vec3::Z * (length * k as f32),
            })
            .collect(),
        options: Options::default(),
    });
    let built = match components::build(g) {
        Ok(b) => b,
        Err(e) => {
            g.components.pop();
            return Err(e);
        }
    };
    built
        .controls
        .controls
        .iter()
        .find(|c| c.component == id)
        .map(|c| c.name.clone())
        .context("the prop socket made no control")
}

/// What the scene builders read: each character's builder script and place,
/// and the holds in exported names.
fn script_data(scene: &Scene, target: Target) -> serde_json::Value {
    let characters: Vec<_> = scene
        .characters
        .iter()
        .map(|c| {
            json!({
                "name": c.name,
                "script": format!("{}_{}.py", c.name, target.key()),
                "offset": c.offset,
            })
        })
        .collect();
    let holds: Vec<_> = scene
        .holds
        .iter()
        .map(|h| {
            let a = scene.character(&h.prop.character).unwrap();
            let b = scene.character(&h.holder.character).unwrap();
            json!({
                "prop_character": a.name,
                "prop_control": a.guides.naming.control(&h.prop.control),
                "holder_character": b.name,
                "holder_bone": b.guides.naming.bone(&h.holder.bone),
                "held": h.held,
            })
        })
        .collect();
    json!({
        "hive_scene": SCENE_VERSION,
        "name": scene.name,
        "characters": characters,
        "holds": holds,
    })
}

/// The scene builder for one DCC.
pub fn script(scene: &Scene, target: Target) -> Result<String> {
    let template = match target {
        Target::Blender => BLENDER_TEMPLATE,
        Target::Maya => MAYA_TEMPLATE,
        Target::Unreal => UNREAL_TEMPLATE,
        Target::Fbx => bail!("FBX has no scene builder"),
    };
    let data = serde_json::to_string(&script_data(scene, target))?;
    debug_assert!(!data.contains("\"\"\""));
    Ok(template.replace(JSON_SLOT, &data))
}

/// Build and export the scene into `out`: every character, or only those in
/// `only` (the others keep the files of their last export, which must be in
/// `out`). Writes each character's files, records its rig in the scene, and
/// writes the scene builders for the script targets. `dir` is the scene's
/// folder. Returns the files written.
pub fn build_and_write(
    scene: &mut Scene,
    dir: &Path,
    out: &Path,
    only: &[String],
    targets: &[Target],
    opts: RigOptions,
) -> Result<Vec<PathBuf>> {
    ensure!(!scene.characters.is_empty(), "the scene has no characters");
    for name in only {
        scene.index(name)?;
    }
    scene.check_holds()?;
    let mut written = Vec::new();
    for i in 0..scene.characters.len() {
        let c = &scene.characters[i];
        if !only.is_empty() && !only.contains(&c.name) {
            for &t in targets.iter().filter(|t| **t != Target::Fbx) {
                let file = out.join(format!("{}_{}.py", c.name, t.key()));
                ensure!(
                    file.is_file(),
                    "{} hasn't been exported to {} yet ({} is missing): build it too",
                    c.name,
                    out.display(),
                    file.display()
                );
            }
            continue;
        }
        let rig = build_character(c, dir, opts)?;
        written.extend(export::write_all(&rig, out, targets)?);
        let rig_file = std::path::absolute(out.join(format!("{}.hive.json", rig.name)))?;
        let dir_abs = std::path::absolute(dir)?;
        scene.characters[i].rig = Some(
            rig_file
                .strip_prefix(&dir_abs)
                .map(|p| p.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| rig_file.to_string_lossy().into_owned()),
        );
    }
    for &t in targets.iter().filter(|t| **t != Target::Fbx) {
        let path = out.join(format!("{}_scene_{}.py", scene.name, t.key()));
        std::fs::write(&path, script(scene, t)?)?;
        written.push(path);
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_characters() -> Scene {
        let dir = Path::new(".");
        let mut s = Scene::new("duel");
        s.add(character_from_source("hero", "demo", dir, Vec3::ZERO).unwrap())
            .unwrap();
        s.add(character_from_source("rival", "demo", dir, Vec3::new(1.2, 0.0, 0.0)).unwrap())
            .unwrap();
        s
    }

    #[test]
    fn characters_and_holds_are_checked() {
        let mut s = two_characters();
        assert!(
            s.add(character_from_source("hero", "demo", Path::new("."), Vec3::ZERO).unwrap())
                .is_err()
        );
        assert_eq!(s.free_name("hero"), "hero_2");
        let socket = add_prop_socket(&mut s.characters[0].guides, "hand_r", "sword").unwrap();
        assert_eq!(socket, "sword_01_r_ctrl");
        let hold = Hold {
            prop: PropRef {
                character: "hero".into(),
                control: socket.clone(),
            },
            holder: BoneRef {
                character: "rival".into(),
                bone: "hand_l".into(),
            },
            held: 1.0,
        };
        s.hold(hold.clone()).unwrap();
        // A bone the holder doesn't have, a control the prop's character
        // doesn't have, or a character holding its own control are refused.
        let mut bad = hold.clone();
        bad.holder.bone = "wing_l".into();
        assert!(s.hold(bad).is_err());
        let mut bad = hold.clone();
        bad.prop.control = "nope_ctrl".into();
        assert!(s.hold(bad).is_err());
        let mut bad = hold.clone();
        bad.holder.character = "hero".into();
        assert!(s.hold(bad).is_err());
        assert_eq!(s.holds, vec![hold.clone()]);
        // The scene round-trips, and removing a character drops its holds.
        let text = serde_json::to_string(&s).unwrap();
        let back = Scene::parse(&text).unwrap();
        assert_eq!(back.characters.len(), 2);
        assert_eq!(back.holds, s.holds);
        assert!(back.written_by.is_some());
        let mut other = back.clone();
        other.remove("rival").unwrap();
        assert!(other.holds.is_empty());
        assert!(s.release(&hold.prop) && s.holds.is_empty());
        // Parsing refuses a hold on a character the scene lacks.
        let mut v: serde_json::Value = serde_json::from_str(&text).unwrap();
        v["holds"][0]["holder"]["character"] = json!("ghost");
        assert!(Scene::parse(&v.to_string()).is_err());
    }

    #[test]
    fn parses_character_arguments() {
        assert_eq!(
            parse_character_arg("hero=hero.glb@1,0,-2").unwrap(),
            ("hero".into(), "hero.glb".into(), Vec3::new(1.0, 0.0, -2.0))
        );
        assert_eq!(
            parse_character_arg("dog=demo:dog").unwrap().1,
            "demo:dog".to_string()
        );
        assert!(parse_character_arg("hero.glb").is_err());
        assert!(parse_character_arg("a=b@1,2").is_err());
        assert!(character_from_source("x", "demo:unicorn", Path::new("."), Vec3::ZERO).is_err());
    }

    #[test]
    fn builds_exports_and_writes_scene_builders() {
        let dir = std::env::temp_dir().join(format!("hive-rig-scene-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let mut s = two_characters();
        s.add(character_from_source("dog", "demo:dog", &dir, Vec3::new(-1.5, 0.0, 0.0)).unwrap())
            .unwrap();
        let socket = add_prop_socket(&mut s.characters[0].guides, "hand_r", "sword").unwrap();
        s.hold(Hold {
            prop: PropRef {
                character: "hero".into(),
                control: socket,
            },
            holder: BoneRef {
                character: "rival".into(),
                bone: "hand_l".into(),
            },
            held: 1.0,
        })
        .unwrap();
        // Exported names follow each character's own naming.
        s.characters[1].guides.naming = crate::naming::Preset::Suffix.naming();
        let out = dir.join("out");
        let targets = [Target::Blender, Target::Maya, Target::Unreal];
        // One at a time: the others haven't been exported, so it is refused.
        assert!(
            build_and_write(
                &mut s,
                &dir,
                &out,
                &["hero".into()],
                &targets,
                Default::default()
            )
            .is_err()
        );
        let files = build_and_write(&mut s, &dir, &out, &[], &targets, Default::default()).unwrap();
        for name in ["hero", "rival", "dog"] {
            assert!(
                files
                    .iter()
                    .any(|f| f.ends_with(format!("{name}_blender.py"))),
                "{name}"
            );
            assert_eq!(
                s.character(name).unwrap().rig.as_deref(),
                Some(format!("out/{name}.hive.json").as_str())
            );
        }
        let blender = std::fs::read_to_string(out.join("duel_scene_blender.py")).unwrap();
        let start = blender.find("r\"\"\"").unwrap() + 4;
        let end = start + blender[start..].find("\"\"\"").unwrap();
        let data: serde_json::Value = serde_json::from_str(&blender[start..end]).unwrap();
        assert_eq!(data["characters"].as_array().unwrap().len(), 3);
        assert_eq!(data["characters"][1]["script"], "rival_blender.py");
        assert_eq!(
            data["holds"][0]["holder_bone"],
            s.characters[1].guides.naming.bone("hand_l")
        );
        assert_ne!(data["holds"][0]["holder_bone"], "hand_l");
        assert_eq!(data["holds"][0]["prop_control"], "sword_01_r_ctrl");
        for t in ["maya", "unreal"] {
            assert!(out.join(format!("duel_scene_{t}.py")).is_file());
        }
        // Now one at a time works, and keeps the others' files.
        let again = build_and_write(
            &mut s,
            &dir,
            &out,
            &["dog".into()],
            &targets,
            Default::default(),
        )
        .unwrap();
        assert!(again.iter().any(|f| f.ends_with("dog.glb")));
        assert!(!again.iter().any(|f| f.ends_with("hero.glb")));
        // The saved scene loads back, rigs and all.
        let path = dir.join("duel.scene.json");
        s.save(&path).unwrap();
        let back = Scene::load(&path).unwrap();
        assert!(saved_rig(back.character("hero").unwrap(), &dir).is_some());
        assert!(script(&back, Target::Fbx).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
