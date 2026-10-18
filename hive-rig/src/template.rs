//! Rig templates: a studio's own component setups, saved to share and reuse.
//!
//! A template is a set of components with their options and control looks,
//! saved as `<name>.template.json`. Guide positions are stored in body heights,
//! measured from the root guide, so a template made on one character fits
//! another of any size. A template holds either a whole layout, which replaces a
//! character's guides, or a few premade parts, which are added to its rig the way
//! `components::import` adds them, except that ids already in use get a numeric
//! suffix instead of being refused.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result, bail, ensure};
use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::components::{
    self, AddOptions, Component, ControlStyle, Guides, Kind, Preset, id_names, is_premade_part,
};
use crate::naming::Naming;
use crate::skin::SkinLimits;

pub const TEMPLATE_FORMAT: &str = "hive-rig-template";
pub const TEMPLATE_VERSION: u32 = 1;
/// File names end with this.
pub const TEMPLATE_SUFFIX: &str = ".template.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    /// Every component of a rig: loading it replaces the character's guides.
    Layout,
    /// Some premade parts: loading them adds them to the character's rig.
    Parts,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Template {
    pub format: String,
    pub version: u32,
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub scope: Scope,
    /// Components in build order. Guide positions are in body heights from the
    /// root guide of the rig they were saved from.
    pub components: Vec<Part>,
    /// The naming convention of a layout, when it isn't the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub naming: Option<Naming>,
    /// The skin influence limits of a layout, when they aren't the default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skin: Option<SkinLimits>,
    /// Control looks keyed by control name, with sizes in body heights.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub control_styles: BTreeMap<String, ControlStyle>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Part {
    #[serde(flatten)]
    pub component: Component,
    /// In a parts template: the part that owns this one's parent bone, when that
    /// part is in the template too. Parts without one hang from a bone of the rig.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_part: Option<String>,
}

/// What loading a template changed.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Applied {
    /// Ids of the components the template added (all of them for a layout).
    pub added: Vec<String>,
    /// Parts whose id was taken, as (id in the template, id in the rig).
    pub renamed: Vec<(String, String)>,
}

impl Applied {
    /// One line for the user: what was added and what was renamed.
    pub fn summary(&self) -> String {
        let mut s = self.added.join(", ");
        if !self.renamed.is_empty() {
            let r: Vec<String> = self
                .renamed
                .iter()
                .map(|(a, b)| format!("{a} as {b}"))
                .collect();
            s.push_str(&format!(" (ids in use, so added {})", r.join(", ")));
        }
        s
    }
}

/// Where a template's guides are measured from: the root guide.
fn origin(g: &Guides) -> Result<Vec3> {
    g.components
        .iter()
        .find(|c| c.kind == Kind::Root)
        .and_then(|c| c.position("root"))
        .context("the layout has no root component")
}

impl Template {
    /// Make a template from a layout: all of it, or only the components `only`
    /// names (premade parts such as fingers, a tail, eyes, a jaw or the face).
    pub fn from_guides(g: &Guides, only: Option<&[String]>, name: &str) -> Result<Template> {
        let name = name.trim();
        ensure!(!name.is_empty(), "a template needs a name");
        let built = components::build(g).context("the layout doesn't build")?;
        let at = origin(g)?;
        let h = g.body.height;
        let picked: HashSet<&str> = match only {
            None => g.components.iter().map(|c| c.id.as_str()).collect(),
            Some(ids) => {
                ensure!(!ids.is_empty(), "pick at least one component to save");
                let mut set = HashSet::new();
                let mut body = Vec::new();
                for id in ids {
                    let c = g
                        .component(id)
                        .with_context(|| format!("no component named {id}"))?;
                    if !is_premade_part(c.kind) {
                        body.push(id.as_str());
                    }
                    set.insert(id.as_str());
                }
                ensure!(
                    body.is_empty(),
                    "{} {} part of the body, which a rig has only one of; save the whole layout to keep {}",
                    body.join(", "),
                    if body.len() == 1 { "is" } else { "are" },
                    if body.len() == 1 { "it" } else { "them" },
                );
                set
            }
        };
        let scope = if only.is_some() {
            Scope::Parts
        } else {
            Scope::Layout
        };
        let owner: HashMap<&str, &str> = built
            .skeleton
            .bones
            .iter()
            .map(|b| (b.name.as_str(), b.component.as_str()))
            .collect();
        let components = g
            .components
            .iter()
            .filter(|c| picked.contains(c.id.as_str()))
            .map(|c| {
                let mut c = c.clone();
                for guide in &mut c.guides {
                    guide.position = (guide.position - at) / h;
                }
                let parent_part = match scope {
                    Scope::Layout => None,
                    Scope::Parts => c
                        .parent
                        .as_deref()
                        .and_then(|p| owner.get(p))
                        .filter(|o| picked.contains(**o))
                        .map(|o| o.to_string()),
                };
                Part {
                    component: c,
                    parent_part,
                }
            })
            .collect();
        let control_styles = built
            .controls
            .controls
            .iter()
            .filter(|c| picked.contains(c.component.as_str()))
            .filter_map(|c| {
                let style = g.control_styles.get(&c.name)?;
                Some((c.name.clone(), scale_style(*style, 1.0 / h)))
            })
            .collect();
        Ok(Template {
            format: TEMPLATE_FORMAT.into(),
            version: TEMPLATE_VERSION,
            name: name.to_string(),
            description: String::new(),
            scope,
            components,
            naming: (scope == Scope::Layout && !g.naming.is_default()).then(|| g.naming.clone()),
            skin: (scope == Scope::Layout && !g.skin.is_default()).then_some(g.skin),
            control_styles,
        })
    }

    pub fn load(path: &Path) -> Result<Template> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        Template::parse(&text).with_context(|| format!("reading {}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Template> {
        let t: Template = serde_json::from_str(text)?;
        ensure!(
            t.format == TEMPLATE_FORMAT,
            "not a hive-rig template (format is '{}')",
            t.format
        );
        ensure!(
            t.version <= TEMPLATE_VERSION,
            "written by a newer hive-rig (template version {})",
            t.version
        );
        ensure!(!t.components.is_empty(), "the template has no components");
        let ids: HashSet<&str> = t
            .components
            .iter()
            .map(|p| p.component.id.as_str())
            .collect();
        ensure!(
            ids.len() == t.components.len(),
            "two parts of the template share an id"
        );
        if t.scope == Scope::Parts {
            for p in &t.components {
                let c = &p.component;
                ensure!(
                    is_premade_part(c.kind),
                    "{} is a {:?}, which a parts template can't hold",
                    c.id,
                    c.kind
                );
                if let Some(pp) = &p.parent_part {
                    ensure!(
                        ids.contains(pp.as_str()),
                        "{} hangs from {pp}, which isn't in the template",
                        c.id
                    );
                }
            }
        }
        Ok(t)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent().filter(|d| !d.as_os_str().is_empty()) {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        std::fs::write(path, serde_json::to_vec_pretty(self)?)
            .with_context(|| format!("writing {}", path.display()))
    }

    /// Load the template onto a layout. A layout template replaces the guides,
    /// scaled to this body; a parts template adds its parts. Parts that hang from
    /// a bone outside the template hang from `parent` when given, otherwise from
    /// the bone of the same name. Nothing changes unless the result builds.
    pub fn apply(&self, g: &mut Guides, parent: Option<&str>) -> Result<Applied> {
        let mut out = g.clone();
        let at = origin(g)?;
        let h = g.body.height;
        let place = |c: &Component| {
            let mut c = c.clone();
            for guide in &mut c.guides {
                guide.position = at + guide.position * h;
            }
            c
        };
        let applied = match self.scope {
            Scope::Layout => {
                ensure!(
                    parent.is_none(),
                    "a whole layout replaces the rig, so it doesn't hang from a bone"
                );
                out.components = self
                    .components
                    .iter()
                    .map(|p| place(&p.component))
                    .collect();
                if let Some(n) = &self.naming {
                    out.naming = n.clone();
                }
                if let Some(limits) = self.skin {
                    limits.validate()?;
                    out.skin = limits;
                }
                out.control_styles = self
                    .control_styles
                    .iter()
                    .map(|(k, s)| (k.clone(), scale_style(*s, h)))
                    .collect();
                components::build(&out).context("the template doesn't build on this character")?;
                Applied {
                    added: out.components.iter().map(|c| c.id.clone()).collect(),
                    renamed: Vec::new(),
                }
            }
            Scope::Parts => self.add_parts(&mut out, parent, place)?,
        };
        *g = out;
        Ok(applied)
    }

    fn add_parts(
        &self,
        g: &mut Guides,
        parent: Option<&str>,
        place: impl Fn(&Component) -> Component,
    ) -> Result<Applied> {
        let target = components::build(g).context("the current guides don't build")?;
        if let Some(bone) = parent {
            ensure!(
                target.skeleton.bone(bone).is_some(),
                "no bone named {bone} to attach to"
            );
        }
        for p in &self.components {
            let kind = p.component.kind;
            if matches!(kind, Kind::Eyes | Kind::Jaw | Kind::Face) {
                ensure!(
                    !g.components.iter().any(|x| x.kind == kind),
                    "the rig already has one {kind:?} component"
                );
            }
        }
        let mut applied = Applied::default();
        // Names a renamed part's bones and controls had in the template -> in the rig.
        let mut bone_map: HashMap<String, String> = HashMap::new();
        let mut ctrl_map: HashMap<String, String> = HashMap::new();
        let mut new_id: HashMap<&str, String> = HashMap::new();
        for p in &self.components {
            let mut c = place(&p.component);
            let id = free_id(g, &c);
            if id != c.id {
                let renamed = Component {
                    id: id.clone(),
                    ..c.clone()
                };
                let (old_bones, old_ctrls) = id_names(&c);
                let (new_bones, new_ctrls) = id_names(&renamed);
                bone_map.extend(old_bones.into_iter().zip(new_bones));
                ctrl_map.extend(old_ctrls.into_iter().zip(new_ctrls));
                applied.renamed.push((c.id.clone(), id.clone()));
                c = renamed;
            }
            c.parent = match (&p.parent_part, c.parent.take()) {
                (Some(owner), Some(bone)) => {
                    ensure!(
                        new_id.contains_key(owner.as_str()),
                        "{} must come after {owner}, the part it hangs from",
                        p.component.id
                    );
                    Some(bone_map.get(&bone).cloned().unwrap_or(bone))
                }
                _ => Some(match parent {
                    Some(bone) => bone.to_string(),
                    None => {
                        let bone = p.component.parent.clone().unwrap_or_default();
                        ensure!(
                            target.skeleton.bone(&bone).is_some(),
                            "{} hangs from {bone}, which this rig doesn't have; choose a bone to hang it from",
                            p.component.id
                        );
                        bone
                    }
                }),
            };
            new_id.insert(&p.component.id, id.clone());
            applied.added.push(id);
            g.components.push(c);
        }
        for (name, style) in &self.control_styles {
            let name = ctrl_map.get(name).unwrap_or(name);
            g.control_styles
                .insert(name.clone(), scale_style(*style, g.body.height));
        }
        components::build(g).context("the template's parts don't fit this rig")?;
        Ok(applied)
    }
}

fn scale_style(mut s: ControlStyle, by: f32) -> ControlStyle {
    s.size = s.size.map(|v| v * by);
    s
}

/// The component's id, or with a number before its side suffix when the rig
/// already has it: `index_l` -> `index2_l`, `tail` -> `tail2`.
fn free_id(g: &Guides, c: &Component) -> String {
    if g.component(&c.id).is_none() {
        return c.id.clone();
    }
    let s = c.side.suffix();
    let (base, suffix) = match c.id.strip_suffix(s) {
        Some(base) if !s.is_empty() && !base.is_empty() => (base, s),
        _ => (c.id.as_str(), ""),
    };
    (2..)
        .map(|n| format!("{base}{n}{suffix}"))
        .find(|id| g.component(id).is_none())
        .unwrap()
}

// --------------------------------------------------------------------------- library

/// Where a listed template comes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    /// Made in code and shipped with every copy.
    BuiltIn,
    File(PathBuf),
}

#[derive(Debug, Clone)]
pub struct Listed {
    pub template: Template,
    pub source: Source,
}

/// The template folder next to the program.
pub fn program_folder() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    Some(exe.parent()?.join("templates"))
}

/// The user's own template folder: `%APPDATA%\HiveRig\templates` on Windows,
/// otherwise `$XDG_CONFIG_HOME/hive-rig/templates` or `~/.config/hive-rig/templates`.
/// New templates are saved here.
pub fn user_folder() -> Option<PathBuf> {
    let var = |k: &str| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    if cfg!(windows) {
        return var("APPDATA").map(|d| d.join("HiveRig").join("templates"));
    }
    var("XDG_CONFIG_HOME")
        .or_else(|| var("HOME").map(|h| h.join(".config")))
        .map(|d| d.join("hive-rig").join("templates"))
}

/// The folders searched for templates, program folder first.
pub fn folders() -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for dir in [program_folder(), user_folder()].into_iter().flatten() {
        if !out.contains(&dir) {
            out.push(dir);
        }
    }
    out
}

/// `<folder>/<name>.template.json`, with the name made safe for a file name.
pub fn file_for(folder: &Path, name: &str) -> PathBuf {
    let safe: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '_' | '-' | ' ') {
                c
            } else {
                '_'
            }
        })
        .collect();
    folder.join(format!("{safe}{TEMPLATE_SUFFIX}"))
}

/// The name a template file is listed under: its file name without the suffix.
pub fn name_from_path(path: &Path) -> String {
    let file = path
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default();
    file.strip_suffix(TEMPLATE_SUFFIX)
        .or_else(|| file.strip_suffix(".json"))
        .unwrap_or(&file)
        .to_string()
}

/// Every template: the built-in ones, then the files in `folders`, sorted by name
/// within each folder. Files that can't be read are reported, not skipped silently.
pub fn list_in(folders: &[PathBuf]) -> (Vec<Listed>, Vec<String>) {
    let mut listed: Vec<Listed> = built_in()
        .iter()
        .map(|t| Listed {
            template: t.clone(),
            source: Source::BuiltIn,
        })
        .collect();
    let mut problems = Vec::new();
    for dir in folders {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => {
                problems.push(format!("{}: {e}", dir.display()));
                continue;
            }
        };
        let mut files: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| {
                p.file_name()
                    .is_some_and(|f| f.to_string_lossy().ends_with(TEMPLATE_SUFFIX))
            })
            .collect();
        files.sort();
        for path in files {
            match Template::load(&path) {
                Ok(template) => listed.push(Listed {
                    template,
                    source: Source::File(path),
                }),
                Err(e) => problems.push(format!("{e:#}")),
            }
        }
    }
    (listed, problems)
}

pub fn list() -> (Vec<Listed>, Vec<String>) {
    list_in(&folders())
}

/// A template given as a file, or by the name it's listed under.
pub fn find(spec: &str) -> Result<Template> {
    let path = Path::new(spec);
    if path.is_file() {
        return Template::load(path);
    }
    let (listed, _) = list();
    let found: Vec<&Listed> = listed
        .iter()
        .filter(|l| {
            l.template.name == spec
                || matches!(&l.source, Source::File(p) if name_from_path(p) == spec)
        })
        .collect();
    match found.as_slice() {
        [] => bail!("no template file or template named {spec}"),
        [one] => Ok(one.template.clone()),
        // A user's own template wins over the program's copy of the same name.
        many => Ok(many.last().unwrap().template.clone()),
    }
}

/// The templates every copy ships with, made in code from the demo character's
/// default layout.
pub fn built_in() -> &'static [Template] {
    static BUILT_IN: OnceLock<Vec<Template>> = OnceLock::new();
    BUILT_IN.get_or_init(|| {
        let (biped, _) = crate::rig::detect_guides(&crate::demo::humanoid(0.0));
        let mut full = biped.clone();
        for preset in [Preset::Eyes, Preset::Jaw, Preset::Face, Preset::Fingers] {
            components::add_preset(&mut full, preset, &AddOptions::default())
                .expect("the built-in parts fit the demo layout");
        }
        let make = |g: &Guides, name: &str, text: &str| {
            let mut t = Template::from_guides(g, None, name).expect("the demo layout builds");
            t.description = text.into();
            t
        };
        vec![
            make(
                &biped,
                "biped",
                "Root, spine, head, arms and legs with IK/FK limbs.",
            ),
            make(
                &full,
                "biped_face_fingers",
                "The biped with eyes, a jaw, face joints and five fingers a hand.",
            ),
            make(
                &components::quadruped(crate::demo::DOG_HEIGHT),
                "dog",
                "Four legs with three-segment hind legs, a neck, head and tail.",
            ),
        ]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::controls::Shape;
    use crate::skeleton::Side;

    fn demo() -> Guides {
        crate::rig::detect_guides(&crate::demo::humanoid(0.0)).0
    }

    fn pos(g: &Guides, id: &str, guide: &str) -> Vec3 {
        g.component(id).unwrap().position(guide).unwrap()
    }

    /// The same character scaled `k` times about its root guide.
    fn scaled(g: &Guides, k: f32) -> Guides {
        let at = origin(g).unwrap();
        let mut s = g.clone();
        for c in &mut s.components {
            for guide in &mut c.guides {
                guide.position = at + (guide.position - at) * k;
            }
        }
        s.body.height *= k;
        s
    }

    #[test]
    fn layout_round_trips_through_json() {
        let mut g = demo();
        components::add_preset(&mut g, Preset::Tail, &AddOptions::default()).unwrap();
        components::set_spine_joints(&mut g, "spine", 4).unwrap();
        g.components
            .iter_mut()
            .find(|c| c.id == "arm_l")
            .unwrap()
            .options
            .twist = false;
        g.naming = crate::naming::Preset::Prefix.naming();
        g.skin = SkinLimits {
            max_influences: 8,
            prune: 0.02,
        };
        let style = ControlStyle {
            shape: Some(Shape::Cube),
            size: Some(0.2),
            color: Some([0.1, 0.9, 0.2]),
        };
        components::set_control_style(&mut g, "tail_01_ctrl", style).unwrap();

        let t = Template::from_guides(&g, None, "studio").unwrap();
        assert_eq!(t.scope, Scope::Layout);
        let text = serde_json::to_string_pretty(&t).unwrap();
        let back = Template::parse(&text).unwrap();

        let mut target = demo();
        let applied = back.apply(&mut target, None).unwrap();
        assert_eq!(applied.added.len(), g.components.len());
        for (a, b) in g.components.iter().zip(&target.components) {
            assert_eq!((&a.id, a.kind, &a.parent), (&b.id, b.kind, &b.parent));
            assert_eq!(a.options.joints, b.options.joints);
            assert_eq!(a.options.twist, b.options.twist);
            for (x, y) in a.guides.iter().zip(&b.guides) {
                assert!(
                    x.position.distance(y.position) < 1e-5,
                    "{} {}",
                    a.id,
                    x.name
                );
            }
        }
        assert_eq!(target.naming, g.naming);
        assert_eq!(target.skin, g.skin);
        let got = target.control_styles["tail_01_ctrl"];
        assert_eq!((got.shape, got.color), (style.shape, style.color));
        assert!((got.size.unwrap() - 0.2).abs() < 1e-6);
    }

    #[test]
    fn layout_scales_to_the_body_height() {
        let g = demo();
        let mut fancy = g.clone();
        components::add_preset(&mut fancy, Preset::Fingers, &AddOptions::default()).unwrap();
        components::set_control_style(
            &mut fancy,
            "index_01_l_ctrl",
            ControlStyle {
                size: Some(0.05),
                ..Default::default()
            },
        )
        .unwrap();
        let t = Template::from_guides(&fancy, None, "fancy").unwrap();
        let mut big = scaled(&g, 2.0);
        t.apply(&mut big, None).unwrap();
        let want = scaled(&fancy, 2.0);
        for (id, guide) in [("arm_l", "wrist"), ("index_l", "j4"), ("head", "head_top")] {
            assert!(
                pos(&big, id, guide).distance(pos(&want, id, guide)) < 1e-4,
                "{id} {guide}"
            );
        }
        let size = big.control_styles["index_01_l_ctrl"].size.unwrap();
        assert!((size - 0.1).abs() < 1e-5);
        // The body measurements are the character's own.
        assert_eq!(big.body.height, g.body.height * 2.0);
    }

    #[test]
    fn clashing_ids_get_a_number_and_inner_links_follow() {
        let mut src = demo();
        components::add_preset(&mut src, Preset::Fingers, &AddOptions::default()).unwrap();
        components::add_preset(&mut src, Preset::Tail, &AddOptions::default()).unwrap();
        // A tassel hanging from the tail's second bone.
        let opts = AddOptions {
            parent: Some("tail_02".into()),
            joints: Some(2),
            id: Some("tassel".into()),
            ..Default::default()
        };
        components::add_preset(&mut src, Preset::Chain, &opts).unwrap();
        components::set_control_style(
            &mut src,
            "tail_03_ctrl",
            ControlStyle {
                shape: Some(Shape::Square),
                ..Default::default()
            },
        )
        .unwrap();
        let only: Vec<String> = ["index_l", "tail", "tassel"].map(String::from).to_vec();
        let t = Template::from_guides(&src, Some(&only), "bits").unwrap();
        assert_eq!(t.scope, Scope::Parts);
        assert_eq!(t.components[2].parent_part.as_deref(), Some("tail"));
        let t = Template::parse(&serde_json::to_string(&t).unwrap()).unwrap();

        // Onto the same rig: every id is taken.
        let mut g = src.clone();
        let applied = t.apply(&mut g, None).unwrap();
        assert_eq!(applied.added, ["index2_l", "tail2", "tassel2"]);
        assert_eq!(applied.renamed.len(), 3);
        let c = |id: &str| g.component(id).unwrap();
        assert_eq!(c("index2_l").parent.as_deref(), Some("hand_l"));
        assert_eq!(c("index2_l").side, Side::Left);
        assert_eq!(c("tail2").parent.as_deref(), Some("pelvis"));
        // The tassel follows its renamed tail.
        assert_eq!(c("tassel2").parent.as_deref(), Some("tail2_02"));
        assert!(g.control_styles.contains_key("tail2_03_ctrl"));
        let built = components::build(&g).unwrap();
        assert!(built.skeleton.bone("index2_01_l").is_some());
        assert_eq!(
            built.controls.control("tail2_03_ctrl").unwrap().shape,
            Shape::Square
        );
        // Again: the next free number.
        let applied = t.apply(&mut g, None).unwrap();
        assert_eq!(applied.added, ["index3_l", "tail3", "tassel3"]);

        // Hung from a chosen bone, except the tassel, which stays on its tail.
        let mut g = demo();
        t.apply(&mut g, Some("head")).unwrap();
        let c = |id: &str| g.component(id).unwrap();
        assert_eq!(c("index_l").parent.as_deref(), Some("head"));
        assert_eq!(c("tail").parent.as_deref(), Some("head"));
        assert_eq!(c("tassel").parent.as_deref(), Some("tail_02"));
        assert!(t.apply(&mut demo(), Some("no_such_bone")).is_err());
    }

    #[test]
    fn parts_refuse_body_components_and_missing_parents() {
        let mut g = demo();
        let only = vec!["arm_l".to_string()];
        assert!(Template::from_guides(&g, Some(&only), "arm").is_err());
        components::add_preset(&mut g, Preset::Eyes, &AddOptions::default()).unwrap();
        let t = Template::from_guides(&g, Some(&["eyes".to_string()]), "eyes").unwrap();
        // The rig already has eyes; nothing changes.
        let before = serde_json::to_string(&g).unwrap();
        assert!(t.apply(&mut g, None).is_err());
        assert_eq!(serde_json::to_string(&g).unwrap(), before);
        // A rig without a head bone needs a bone to hang the eyes from.
        let mut headless = demo();
        components::remove(&mut headless, "head").unwrap();
        let err = t.apply(&mut headless, None).unwrap_err();
        assert!(format!("{err:#}").contains("choose a bone"), "{err:#}");
        t.apply(&mut headless, Some("spine_03")).unwrap();
    }

    #[test]
    fn built_in_templates_fit_and_files_are_listed() {
        for t in built_in() {
            let mut g = scaled(&demo(), 0.6);
            t.apply(&mut g, None).unwrap();
            let t2 = Template::parse(&serde_json::to_string(t).unwrap()).unwrap();
            assert_eq!(t2.components.len(), t.components.len());
        }
        assert!(
            built_in()[1]
                .components
                .iter()
                .any(|p| p.component.kind == Kind::Face)
        );

        let dir = std::env::temp_dir().join(format!("hive_templates_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut g = demo();
        components::add_preset(&mut g, Preset::Toes, &AddOptions::default()).unwrap();
        let only = vec!["toe_big_l".to_string()];
        let t = Template::from_guides(&g, Some(&only), "big toe").unwrap();
        let path = file_for(&dir, &t.name);
        t.save(&path).unwrap();
        std::fs::write(dir.join("broken.template.json"), "{").unwrap();
        std::fs::write(dir.join("ignored.json"), "{").unwrap();
        let (listed, problems) = list_in(std::slice::from_ref(&dir));
        assert_eq!(listed.len(), built_in().len() + 1);
        assert_eq!(listed.last().unwrap().source, Source::File(path.clone()));
        assert_eq!(name_from_path(&path), "big toe");
        assert_eq!(problems.len(), 1, "{problems:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
