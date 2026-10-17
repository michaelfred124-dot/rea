//! The Hive Rig window: load a character, place and edit component guides in a
//! 3D view, build the rig, preview bones, controls and weights, and export.
//!
//! The editing operations are methods on `Studio` that don't touch the UI, so
//! they are tested directly; `ui` only lays out widgets that call them.

use std::path::{Path, PathBuf};

use eframe::egui::Color32;
use glam::Vec3;

use hive_rig::components::{
    self, AddOptions, Built, Component, Guide, Guides, Kind, Options, Preset,
};
use hive_rig::rig::detect_guides;
use hive_rig::skeleton::Side;
use hive_rig::{Rig, RigOptions, Target, build_rig, demo, export, import, mesh::Mesh};

use crate::view::{self, Camera};

const UNDO_LIMIT: usize = 100;

/// The character being rigged.
pub struct Doc {
    pub name: String,
    pub mesh: Mesh,
    pub guides: Guides,
    /// Skeleton and controls built from the current guides, or why they don't build.
    pub preview: Result<Built, String>,
    /// The full rig with skin weights, from the last "Build Rig". Cleared by edits.
    pub rig: Option<Rig>,
    pub out_dir: PathBuf,
    /// Whether the current rig has been exported.
    pub exported: bool,
    undo: Vec<Guides>,
    redo: Vec<Guides>,
}

impl Doc {
    fn new(name: String, mesh: Mesh, guides: Guides, out_dir: PathBuf) -> Doc {
        let mut doc = Doc {
            name: export::sanitize_name(&name),
            mesh,
            guides,
            preview: Err(String::new()),
            rig: None,
            out_dir,
            exported: false,
            undo: Vec::new(),
            redo: Vec::new(),
        };
        doc.refresh();
        doc
    }

    fn refresh(&mut self) {
        self.preview = components::build(&self.guides).map_err(|e| format!("{e:#}"));
        self.rig = None;
        self.exported = false;
    }

    fn bone_names(&self) -> Vec<String> {
        match &self.preview {
            Ok(b) => b.skeleton.bones.iter().map(|b| b.name.clone()).collect(),
            Err(_) => Vec::new(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FingerSides {
    Both,
    Left,
    Right,
}

/// What the viewport shows, after Hive's guides / skeleton / rig stages.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// Guides to place, with the bones they make.
    Guides,
    /// The deform skeleton.
    Skeleton,
    /// The animator controls on the skeleton.
    Rig,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tab {
    Components,
    Library,
}

struct Drag {
    comp: usize,
    guide: usize,
}

pub struct Studio {
    pub doc: Option<Doc>,
    pub camera: Camera,
    /// Selected component and, optionally, one of its guides.
    pub selected: Option<(usize, Option<usize>)>,
    pub symmetry: bool,
    pub mode: Mode,
    pub tab: Tab,
    pub show_mesh: bool,
    pub xray: bool,
    /// Deform bone whose weights colour the mesh (needs a built rig).
    pub weights_bone: Option<String>,
    pub targets: [bool; 3],
    pub status: String,
    pub error: bool,
    /// Bumped on every status change, so the UI can show it as a notification.
    pub status_serial: u64,
    pub finger_sides: FingerSides,
    pub tail_joints: usize,
    pub chain_parent: String,
    pub chain_joints: usize,
    pub chain_id: String,
    drag: Option<Drag>,
    weight_colors: Option<(String, Vec<Color32>)>,
}

impl Default for Studio {
    fn default() -> Self {
        Studio {
            doc: None,
            camera: Camera::default(),
            selected: None,
            symmetry: true,
            mode: Mode::Guides,
            tab: Tab::Components,
            show_mesh: true,
            xray: false,
            weights_bone: None,
            targets: [true; 3],
            status: "Open a character to start, or try the demo.".into(),
            error: false,
            status_serial: 0,
            finger_sides: FingerSides::Both,
            tail_joints: 5,
            chain_parent: String::new(),
            chain_joints: 3,
            chain_id: String::new(),
            drag: None,
            weight_colors: None,
        }
    }
}

// --------------------------------------------------------------------------- operations

impl Studio {
    fn info(&mut self, msg: impl Into<String>) {
        self.status = msg.into();
        self.error = false;
        self.status_serial += 1;
    }

    fn fail(&mut self, msg: impl Into<String>) {
        self.status = msg.into();
        self.error = true;
        self.status_serial += 1;
    }

    fn set_doc(&mut self, doc: Doc) {
        let (min, max) = doc.mesh.bounds();
        self.camera = Camera::default();
        self.camera.frame(min, max);
        self.selected = None;
        self.weights_bone = None;
        self.weight_colors = None;
        self.drag = None;
        self.doc = Some(doc);
    }

    /// Open a character (.glb, .gltf, .obj), a saved rig (`.hive.json`) or, onto
    /// the current character, a guides file (`.guides.json`).
    pub fn open(&mut self, path: &Path) {
        let file = path
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let result = if file.ends_with(".guides.json") {
            self.open_guides(path)
        } else if file.ends_with(".hive.json") {
            self.open_rig(path)
        } else {
            self.open_character(path)
        };
        if let Err(e) = result {
            self.fail(format!("Couldn't open {}: {e:#}", path.display()));
        }
    }

    fn open_character(&mut self, path: &Path) -> anyhow::Result<()> {
        let mesh = import::load_mesh(path)?;
        mesh.validate()?;
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "character".into());
        let out_dir = path
            .parent()
            .unwrap_or(Path::new("."))
            .join(format!("{stem}_rig"));
        let (guides, found) = detect_guides(&mesh);
        let missing: Vec<&str> = [
            (found.crotch, "legs"),
            (found.arms, "arms"),
            (found.neck, "neck"),
        ]
        .into_iter()
        .filter(|(ok, _)| !ok)
        .map(|(_, what)| what)
        .collect();
        let verts = mesh.vertex_count();
        self.set_doc(Doc::new(stem, mesh, guides, out_dir));
        if missing.is_empty() {
            self.info(format!(
                "Loaded {} ({verts} vertices). Guides placed on the body: drag them to adjust, then Build Rig.",
                path.display()
            ));
        } else {
            self.fail(format!(
                "Loaded {verts} vertices, but couldn't find the {} on the mesh; those guides use average proportions, so check them.",
                missing.join(", ")
            ));
        }
        Ok(())
    }

    fn open_rig(&mut self, path: &Path) -> anyhow::Result<()> {
        let rig = Rig::load(path)?;
        let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let mut doc = Doc::new(rig.name.clone(), rig.mesh.clone(), rig.guides.clone(), dir);
        doc.rig = Some(rig);
        self.set_doc(doc);
        self.info(format!("Opened rig {}", path.display()));
        Ok(())
    }

    fn open_guides(&mut self, path: &Path) -> anyhow::Result<()> {
        let guides = Guides::load(path)?;
        components::build(&guides)?;
        let Some(doc) = &mut self.doc else {
            anyhow::bail!("open a character first, then its guides");
        };
        push_undo(&mut doc.undo, &doc.guides);
        doc.guides = guides;
        doc.refresh();
        self.selected = None;
        self.info(format!("Loaded guides from {}", path.display()));
        Ok(())
    }

    pub fn load_demo(&mut self) {
        let mesh = demo::humanoid(0.0);
        let (guides, _) = detect_guides(&mesh);
        let out_dir = std::env::current_exe()
            .ok()
            .and_then(|e| e.parent().map(Path::to_path_buf))
            .unwrap_or_default()
            .join("demo_rig");
        self.set_doc(Doc::new("hive_demo".into(), mesh, guides, out_dir));
        self.info("Demo character loaded. Drag the guides to adjust, then Build Rig.");
    }

    pub fn save_guides(&mut self, path: &Path) {
        let Some(doc) = &self.doc else { return };
        let result = serde_json::to_vec_pretty(&doc.guides)
            .map_err(anyhow::Error::from)
            .and_then(|bytes| Ok(std::fs::write(path, bytes)?));
        match result {
            Ok(()) => self.info(format!("Saved guides to {}", path.display())),
            Err(e) => self.fail(format!("Couldn't save {}: {e:#}", path.display())),
        }
    }

    /// Apply a layout change as one undo step. A change that leaves the layout
    /// unbuildable is rolled back and reported.
    pub fn edit(
        &mut self,
        what: &str,
        change: impl FnOnce(&mut Guides) -> anyhow::Result<String>,
    ) -> bool {
        let Some(doc) = &mut self.doc else {
            return false;
        };
        let before = doc.guides.clone();
        let result = change(&mut doc.guides).and_then(|msg| {
            components::build(&doc.guides)?;
            Ok(msg)
        });
        match result {
            Ok(msg) => {
                push_undo(&mut doc.undo, &before);
                doc.redo.clear();
                doc.refresh();
                let n = doc.guides.components.len();
                if self.selected.is_some_and(|(c, _)| c >= n) {
                    self.selected = None;
                }
                self.weight_colors = None;
                self.info(msg);
                true
            }
            Err(e) => {
                doc.guides = before;
                self.fail(format!("Can't {what}: {e:#}"));
                false
            }
        }
    }

    pub fn undo(&mut self) {
        let Some(doc) = &mut self.doc else { return };
        match doc.undo.pop() {
            Some(g) => {
                let current = std::mem::replace(&mut doc.guides, g);
                doc.redo.push(current);
                doc.refresh();
                self.selected = None;
                self.weight_colors = None;
                self.info("Undone.");
            }
            None => self.info("Nothing to undo."),
        }
    }

    pub fn redo(&mut self) {
        let Some(doc) = &mut self.doc else { return };
        match doc.redo.pop() {
            Some(g) => {
                let current = std::mem::replace(&mut doc.guides, g);
                push_undo(&mut doc.undo, &current);
                doc.refresh();
                self.selected = None;
                self.weight_colors = None;
                self.info("Redone.");
            }
            None => self.info("Nothing to redo."),
        }
    }

    pub fn can_undo(&self) -> bool {
        self.doc.as_ref().is_some_and(|d| !d.undo.is_empty())
    }

    pub fn can_redo(&self) -> bool {
        self.doc.as_ref().is_some_and(|d| !d.redo.is_empty())
    }

    /// Move one guide (and its mirror twin when symmetry is on) without an undo
    /// step; dragging records one step when it starts.
    pub fn move_guide(&mut self, comp: usize, guide: usize, to: Vec3) {
        let symmetry = self.symmetry;
        let Some(doc) = &mut self.doc else { return };
        if comp >= doc.guides.components.len() {
            return;
        }
        let twin = if symmetry {
            components::mirror_guide(&doc.guides, comp, guide)
        } else {
            None
        };
        doc.guides.components[comp].guides[guide].position = to;
        let mirrored = components::mirror_point(&doc.guides, to);
        if let Some((tc, tg)) = twin {
            doc.guides.components[tc].guides[tg].position = mirrored;
        }
        doc.refresh();
        self.weight_colors = None;
    }

    pub fn add_fingers(&mut self) {
        let side = match self.finger_sides {
            FingerSides::Both => None,
            FingerSides::Left => Some(Side::Left),
            FingerSides::Right => Some(Side::Right),
        };
        let opts = AddOptions {
            side,
            ..AddOptions::default()
        };
        self.add(Preset::Fingers, opts);
    }

    pub fn add_tail(&mut self) {
        let opts = AddOptions {
            joints: Some(self.tail_joints),
            ..AddOptions::default()
        };
        self.add(Preset::Tail, opts);
    }

    pub fn add_chain(&mut self) {
        let id = self.chain_id.trim();
        let opts = AddOptions {
            parent: Some(self.chain_parent.clone()).filter(|p| !p.is_empty()),
            joints: Some(self.chain_joints),
            id: (!id.is_empty()).then(|| id.to_string()),
            ..AddOptions::default()
        };
        self.add(Preset::Chain, opts);
    }

    fn add(&mut self, preset: Preset, opts: AddOptions) {
        let added = self.edit("add that component", |g| {
            let ids = components::add_preset(g, preset, &opts)?;
            Ok(format!("Added {}.", ids.join(", ")))
        });
        if added && let Some(doc) = &self.doc {
            self.selected = Some((doc.guides.components.len() - 1, None));
        }
    }

    /// Eyes with a look-at control, pushed onto the front of the face.
    pub fn add_eyes(&mut self) {
        self.add(Preset::Eyes, AddOptions::default());
        self.snap_to_face(Kind::Eyes, &["eye_l", "eye_r"], 0.012);
    }

    /// A jaw, with the chin on the front of the face.
    pub fn add_jaw(&mut self) {
        self.add(Preset::Jaw, AddOptions::default());
        self.snap_to_face(Kind::Jaw, &["chin"], 0.0);
    }

    /// Move the named guides of the just-added component forward or back to the
    /// mesh surface in front of them, `inset` (fraction of the body height) inside it.
    fn snap_to_face(&mut self, kind: Kind, names: &[&str], inset: f32) {
        if self.error {
            return;
        }
        let Some(doc) = &mut self.doc else { return };
        let h = doc.guides.body.height;
        let Some(c) = doc.guides.components.iter_mut().find(|c| c.kind == kind) else {
            return;
        };
        let reach = 0.02 * h;
        for guide in c
            .guides
            .iter_mut()
            .filter(|g| names.contains(&g.name.as_str()))
        {
            let p = guide.position;
            let front = doc
                .mesh
                .positions
                .iter()
                .filter(|v| (v.x - p.x).abs() < reach && (v.y - p.y).abs() < reach)
                .map(|v| v.z)
                .fold(f32::NEG_INFINITY, f32::max);
            if front.is_finite() {
                guide.position.z = front - inset * h;
            }
        }
        doc.refresh();
    }

    /// A pair of ears on the head, two bones each.
    pub fn add_ears(&mut self) {
        self.add_custom("add ears", |g| {
            let (head, top, h) = head_frame(g)?;
            let mut ids = Vec::new();
            for side in [Side::Left, Side::Right] {
                let s = side.sign();
                let start = head.lerp(top, 0.45) + Vec3::X * (0.055 * h * s);
                let dir = Vec3::new(0.45 * s, 1.0, -0.15).normalize();
                let id = format!("ear{}", side.suffix());
                g.components
                    .push(chain(&id, side, "head", start, dir, 0.07 * h, 2));
                ids.push(id);
            }
            Ok(ids)
        });
    }

    /// A ponytail hanging from the back of the head.
    pub fn add_ponytail(&mut self) {
        self.add_custom("add a ponytail", |g| {
            let (head, top, h) = head_frame(g)?;
            let start = head.lerp(top, 0.6) - Vec3::Z * (0.06 * h);
            let dir = Vec3::new(0.0, -0.7, -1.0).normalize();
            g.components.push(chain(
                "ponytail",
                Side::Center,
                "head",
                start,
                dir,
                0.22 * h,
                4,
            ));
            Ok(vec!["ponytail".into()])
        });
    }

    /// A one-bone prop (weapon, tool) socket in the right hand.
    pub fn add_prop(&mut self) {
        self.add_custom("add a prop bone", |g| {
            let built = components::build(g)?;
            let hand = built
                .skeleton
                .bone("hand_r")
                .ok_or_else(|| anyhow::anyhow!("the rig has no right hand (hand_r)"))?;
            let h = g.body.height;
            let start = hand.head.lerp(hand.tail, 0.5);
            g.components.push(chain(
                "prop_r",
                Side::Right,
                "hand_r",
                start,
                Vec3::Z,
                0.12 * h,
                1,
            ));
            Ok(vec!["prop_r".into()])
        });
    }

    fn add_custom(
        &mut self,
        what: &str,
        change: impl FnOnce(&mut Guides) -> anyhow::Result<Vec<String>>,
    ) {
        let added = self.edit(what, |g| {
            let ids = change(g)?;
            for id in &ids {
                anyhow::ensure!(
                    g.components.iter().filter(|c| &c.id == id).count() == 1,
                    "{id} already exists"
                );
            }
            Ok(format!("Added {}.", ids.join(", ")))
        });
        if added && let Some(doc) = &self.doc {
            self.selected = Some((doc.guides.components.len() - 1, None));
        }
    }

    pub fn remove_selected(&mut self) {
        let Some((c, _)) = self.selected else { return };
        let Some(id) = self.doc.as_ref().map(|d| d.guides.components[c].id.clone()) else {
            return;
        };
        if self.edit("remove it", |g| {
            let removed = components::remove(g, &id)?;
            Ok(format!("Removed {}.", removed.join(", ")))
        }) {
            self.selected = None;
        }
    }

    pub fn mirror(&mut self, source: Side) {
        let label = if source == Side::Left {
            "left to right"
        } else {
            "right to left"
        };
        self.edit("mirror", |g| {
            let n = components::mirror_side(g, source);
            Ok(format!("Mirrored {n} component(s) {label}."))
        });
    }

    pub fn redetect(&mut self) {
        let Some(doc) = &self.doc else { return };
        let (fresh, _) = detect_guides(&doc.mesh);
        self.edit("reset the guides", |g| {
            *g = fresh;
            Ok("Guides reset to the detected body (Undo brings yours back).".into())
        });
        self.selected = None;
    }

    pub fn build(&mut self) -> bool {
        let Some(doc) = &mut self.doc else {
            return false;
        };
        match build_rig(
            doc.mesh.clone(),
            &doc.name,
            doc.guides.clone(),
            RigOptions::default(),
        ) {
            Ok(rig) => {
                let msg = format!(
                    "Rig built: {} bones, {} controls, {} IK limbs, weights on {} vertices. Pick a bone under Weights to check them, or Export.",
                    rig.skeleton.bones.len(),
                    rig.controls.controls.len(),
                    rig.controls.ik_chains.len(),
                    rig.mesh.vertex_count()
                );
                doc.rig = Some(rig);
                self.weight_colors = None;
                self.mode = Mode::Rig;
                self.info(msg);
                true
            }
            Err(e) => {
                self.fail(format!("Can't build the rig: {e:#}"));
                false
            }
        }
    }

    /// Build if needed and write the rig for the chosen programs into `dir`.
    pub fn export(&mut self, dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
        if self.doc.as_ref().is_some_and(|d| d.rig.is_none()) && !self.build() {
            anyhow::bail!("{}", self.status);
        }
        let doc = self
            .doc
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("nothing to export"))?;
        let targets: Vec<Target> = Target::ALL
            .into_iter()
            .zip(self.targets)
            .filter_map(|(t, on)| on.then_some(t))
            .collect();
        let rig = doc.rig.as_mut().unwrap();
        rig.name = doc.name.clone();
        let files = export::write_all(rig, dir, &targets)?;
        doc.out_dir = dir.to_path_buf();
        doc.exported = true;
        Ok(files)
    }

    fn do_export(&mut self, dir: PathBuf) {
        match self.export(&dir) {
            Ok(files) => {
                let names: Vec<String> = files
                    .iter()
                    .filter_map(|f| f.file_name().map(|n| n.to_string_lossy().into_owned()))
                    .collect();
                self.info(format!(
                    "Exported to {}: {}",
                    dir.display(),
                    names.join(", ")
                ));
            }
            Err(e) => self.fail(format!("Export failed: {e:#}")),
        }
    }

    fn weights_for(&mut self, bone: &str) -> Option<&[Color32]> {
        let doc = self.doc.as_ref()?;
        let rig = doc.rig.as_ref()?;
        if self.weight_colors.as_ref().is_none_or(|(b, _)| b != bone) {
            let index = rig.skeleton.index_of(bone)? as u16;
            let colors = rig
                .skin
                .joints
                .iter()
                .zip(&rig.skin.weights)
                .map(|(j, w)| {
                    let v: f32 = j
                        .iter()
                        .zip(w)
                        .filter(|(j, _)| **j == index)
                        .map(|(_, w)| w)
                        .sum();
                    view::weight_color(v)
                })
                .collect();
            self.weight_colors = Some((bone.to_string(), colors));
        }
        self.weight_colors.as_ref().map(|(_, c)| c.as_slice())
    }
}

/// The head component's head and head-top guides, and the body height.
fn head_frame(g: &Guides) -> anyhow::Result<(Vec3, Vec3, f32)> {
    let head = g
        .components
        .iter()
        .find(|c| c.kind == Kind::Head)
        .ok_or_else(|| anyhow::anyhow!("the rig has no head component"))?;
    Ok((
        head.position("head").unwrap(),
        head.position("head_top").unwrap(),
        g.body.height,
    ))
}

/// An FK chain component of `joints` bones from `start` along `dir`.
fn chain(
    id: &str,
    side: Side,
    parent: &str,
    start: Vec3,
    dir: Vec3,
    length: f32,
    joints: usize,
) -> Component {
    Component {
        id: id.into(),
        kind: Kind::Chain,
        side,
        parent: Some(parent.into()),
        guides: (0..=joints)
            .map(|k| Guide {
                name: format!("j{}", k + 1),
                position: start + dir * (length * k as f32 / joints as f32),
            })
            .collect(),
        options: Options::default(),
    }
}

fn push_undo(undo: &mut Vec<Guides>, g: &Guides) {
    undo.push(g.clone());
    if undo.len() > UNDO_LIMIT {
        undo.remove(0);
    }
}

mod ui;

#[cfg(test)]
mod tests {
    use super::*;

    fn demo_studio() -> Studio {
        let mut s = Studio::default();
        s.load_demo();
        s
    }

    fn comp(s: &Studio, id: &str) -> usize {
        s.doc
            .as_ref()
            .unwrap()
            .guides
            .components
            .iter()
            .position(|c| c.id == id)
            .unwrap()
    }

    #[test]
    fn edits_build_and_undo() {
        let mut s = demo_studio();
        let n = s.doc.as_ref().unwrap().guides.components.len();
        s.add_fingers();
        s.add_tail();
        assert!(!s.error, "{}", s.status);
        assert!(s.doc.as_ref().unwrap().guides.components.len() > n + 5);
        // Removing an arm removes the fingers on it too.
        s.selected = Some((comp(&s, "arm_l"), None));
        s.remove_selected();
        let g = &s.doc.as_ref().unwrap().guides;
        assert!(g.component("arm_l").is_none() && g.component("index_l").is_none());
        assert!(g.component("index_r").is_some());
        s.undo();
        assert!(
            s.doc
                .as_ref()
                .unwrap()
                .guides
                .component("index_l")
                .is_some()
        );
        // The root can't be removed; the layout is left as it was.
        s.selected = Some((comp(&s, "root"), None));
        let before = s.doc.as_ref().unwrap().guides.components.len();
        s.remove_selected();
        assert!(s.error);
        assert_eq!(s.doc.as_ref().unwrap().guides.components.len(), before);
    }

    #[test]
    fn symmetric_moves_and_mirror() {
        let mut s = demo_studio();
        let (l, r) = (comp(&s, "arm_l"), comp(&s, "arm_r"));
        let elbow = 2;
        let p = s.doc.as_ref().unwrap().guides.components[l].guides[elbow].position
            + Vec3::new(0.0, -0.05, 0.03);
        s.move_guide(l, elbow, p);
        let g = &s.doc.as_ref().unwrap().guides;
        let twin = g.components[r].guides[elbow].position;
        assert!((twin - components::mirror_point(g, p)).length() < 1e-5);

        s.symmetry = false;
        s.move_guide(r, elbow, twin + Vec3::Y * 0.1);
        s.mirror(Side::Left);
        let g = &s.doc.as_ref().unwrap().guides;
        assert!((g.components[r].guides[elbow].position - twin).length() < 1e-5);
    }

    #[test]
    fn build_weights_and_export() {
        let mut s = demo_studio();
        assert!(s.build(), "{}", s.status);
        assert!(
            s.weights_for("upperarm_l")
                .unwrap()
                .iter()
                .any(|c| c.r() > 200)
        );
        let dir = std::env::temp_dir().join(format!("hive_studio_test_{}", std::process::id()));
        s.targets = [true, false, true];
        let files = s.export(&dir).unwrap();
        let names: Vec<String> = files
            .iter()
            .map(|f| f.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"hive_demo_blender.py".to_string()));
        assert!(names.contains(&"hive_demo.guides.json".to_string()));
        assert!(!names.iter().any(|n| n.ends_with("_maya.py")));

        // Editing invalidates the built rig; export builds it again.
        s.add_tail();
        assert!(s.doc.as_ref().unwrap().rig.is_none());
        s.export(&dir).unwrap();
        assert!(
            s.doc
                .as_ref()
                .unwrap()
                .rig
                .as_ref()
                .unwrap()
                .skeleton
                .bone("tail_01")
                .is_some()
        );

        // A saved rig opens with its guides and weights.
        let mut t = Studio::default();
        t.open(&dir.join("hive_demo.hive.json"));
        assert!(!t.error, "{}", t.status);
        assert!(t.doc.as_ref().unwrap().guides.component("tail").is_some());
        assert!(t.doc.as_ref().unwrap().rig.is_some());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn library_presets_and_redo() {
        let mut s = demo_studio();
        s.add_ears();
        s.add_ponytail();
        s.add_prop();
        assert!(!s.error, "{}", s.status);
        assert!(s.build(), "{}", s.status);
        let rig = s.doc.as_ref().unwrap().rig.as_ref().unwrap();
        for b in ["ear_01_l", "ear_02_r", "ponytail_04", "prop_01_r"] {
            assert!(rig.skeleton.bone(b).is_some(), "missing {b}");
        }
        // Ears are a mirrored pair.
        let g = &s.doc.as_ref().unwrap().guides;
        let (l, r) = (g.component("ear_l").unwrap(), g.component("ear_r").unwrap());
        assert!(
            (components::mirror_point(g, l.guides[1].position) - r.guides[1].position).length()
                < 1e-5
        );
        // Adding them twice is refused and changes nothing.
        let n = g.components.len();
        s.add_ears();
        assert!(s.error);
        assert_eq!(s.doc.as_ref().unwrap().guides.components.len(), n);
        // Undo and redo step through the additions.
        s.undo();
        assert!(s.doc.as_ref().unwrap().guides.component("prop_r").is_none());
        assert!(s.can_redo());
        s.redo();
        assert!(s.doc.as_ref().unwrap().guides.component("prop_r").is_some());
        assert!(!s.can_redo());
    }

    #[test]
    fn spine_joints_keep_attachments() {
        let mut s = demo_studio();
        s.edit("change the spine", |g| {
            components::set_spine_joints(g, "spine", 5)?;
            Ok(String::new())
        });
        assert!(!s.error, "{}", s.status);
        let g = &s.doc.as_ref().unwrap().guides;
        assert_eq!(
            g.component("head").unwrap().parent.as_deref(),
            Some("spine_05")
        );
    }
}
