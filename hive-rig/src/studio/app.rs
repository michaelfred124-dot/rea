//! The Hive Rig window: load a character, place and edit component guides in a
//! 3D view, build the rig, preview bones, controls and weights, and export.
//!
//! The editing operations are methods on `Studio` that don't touch the UI, so
//! they are tested directly; `ui` only lays out widgets that call them.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use eframe::egui::{self, Color32};
use glam::Vec3;

use hive_rig::checks::{self, Issue, Surface};
use hive_rig::components::{
    self, AddOptions, Built, Component, ControlStyle, Guide, Guides, Kind, Options, Preset,
};
use hive_rig::joints::Joint;
use hive_rig::naming::Naming;
use hive_rig::orient::{Choice, Detection, Orientation};
use hive_rig::rig::detect_guides;
use hive_rig::skeleton::{Side, turn_about};
use hive_rig::skin::{BrushMode, Dab, SkinLimits, SkinOptions, SkinWeights};
use hive_rig::template::{self, Listed, Scope, Template};
use hive_rig::{Rig, RigOptions, Target, build_rig, demo, export, import, mesh::Mesh, skin};

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
    /// The pre-build checks on the current guides.
    pub checks: Vec<Issue>,
    surface: Surface,
    undo: Vec<Guides>,
    redo: Vec<Guides>,
    /// The automatic weights of `rig`, before painting or other weight edits,
    /// so a rebuild can tell the edited vertices apart.
    auto_skin: Option<SkinWeights>,
    /// The last built rig (and its automatic weights) after an edit cleared it,
    /// so the next build keeps its hand-edited weights.
    kept: Option<(Rig, Option<SkinWeights>)>,
    /// Weight edits on the built rig (paint strokes, mirror, smooth), undone
    /// before any guide edit.
    skin_undo: Vec<SkinWeights>,
    skin_redo: Vec<SkinWeights>,
    /// How the character stood in its source file; builds record it so the
    /// .glb and .fbx put it back that way.
    pub orientation: Orientation,
    /// The character as read from its file (its own frame), and what was
    /// detected, so the orientation can be changed after opening.
    pub source: Option<SourceFile>,
    /// Where it stands in the scene, beside the other characters (metres).
    pub offset: Vec3,
    /// Where its mesh comes from, for a scene file: the file it was opened
    /// from, or `demo` / `demo:<creature>`.
    pub origin: Option<String>,
}

/// A character as its file has it, before it was turned into hive-rig's frame.
pub struct SourceFile {
    pub mesh: Mesh,
    pub joints: Vec<Joint>,
    pub detection: Detection,
}

impl Doc {
    fn new(name: String, mesh: Mesh, guides: Guides, out_dir: PathBuf) -> Doc {
        let surface = Surface::new(&mesh);
        let mut doc = Doc {
            name: export::sanitize_name(&name),
            mesh,
            guides,
            preview: Err(String::new()),
            rig: None,
            out_dir,
            exported: false,
            checks: Vec::new(),
            surface,
            undo: Vec::new(),
            redo: Vec::new(),
            auto_skin: None,
            kept: None,
            skin_undo: Vec::new(),
            skin_redo: Vec::new(),
            orientation: Orientation::STANDARD,
            source: None,
            offset: Vec3::ZERO,
            origin: None,
        };
        doc.refresh();
        doc
    }

    /// Put the pose readers' helper joints on the preview skeleton as the
    /// built rig has them (after its own bones), so the two line up bone for bone.
    pub fn place_helpers(&mut self) {
        if let Ok(built) = &mut self.preview {
            built
                .skeleton
                .bones
                .retain(|b| b.component != hive_rig::readers::HELPER_COMPONENT);
            hive_rig::readers::add_helper_bones(&self.mesh.readers, &mut built.skeleton);
        }
    }

    fn refresh(&mut self) {
        self.preview = components::build(&self.guides).map_err(|e| format!("{e:#}"));
        self.place_helpers();
        self.checks = checks::check_built(
            &self.guides,
            self.preview.as_ref().map_err(String::as_str),
            Some(&mut self.surface),
        );
        if let Some(rig) = self.rig.take() {
            self.kept = Some((rig, self.auto_skin.take()));
        }
        self.skin_undo.clear();
        self.skin_redo.clear();
        self.exported = false;
    }

    /// Record the built rig's weights before a weight edit, for undo.
    fn push_skin_undo(&mut self) {
        if let Some(rig) = &self.rig {
            self.skin_undo.push(rig.skin.clone());
            if self.skin_undo.len() > UNDO_LIMIT {
                self.skin_undo.remove(0);
            }
            self.skin_redo.clear();
        }
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

/// The rigging window's filter.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Category {
    All,
    Face,
    Body,
    Extras,
    /// Saved setups: the built-in templates and the ones in the template folders.
    Templates,
}

struct Drag {
    comp: usize,
    guide: usize,
}

/// A bone or control picked in the build stages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RigPick {
    Bone(String),
    Control(String),
}

/// A test pose: one bone (and everything hanging from it) turned about a world axis.
#[derive(Clone, Debug, PartialEq)]
pub struct Pose {
    pub bone: String,
    /// 0 = X, 1 = Y, 2 = Z.
    pub axis: usize,
    pub degrees: f32,
}

/// The bones, controls and skin as they show with the test pose applied.
pub struct Frame {
    pub bones: Vec<(Vec3, Vec3)>,
    pub controls: Vec<Vec3>,
    pub control_axes: Vec<Vec3>,
    /// Each control's posed transform, when the control rig poses the frame.
    pub control_mats: Option<Vec<glam::Mat4>>,
    /// Deformed mesh vertices, when the rig is built.
    pub mesh: Option<Vec<Vec3>>,
}

/// How joints and bones are drawn in the viewport.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct JointDisplay {
    /// Joints and bones are drawn at all.
    pub show: bool,
    /// Drawn over the mesh; off, the mesh in front hides them.
    pub xray: bool,
    /// Display size, a factor on the size picked from the character's height.
    pub size: f32,
}

impl JointDisplay {
    pub const MIN_SIZE: f32 = 0.25;
    pub const MAX_SIZE: f32 = 3.0;
}

impl Default for JointDisplay {
    fn default() -> Self {
        JointDisplay {
            show: true,
            xray: true,
            size: 1.0,
        }
    }
}

/// The weight brush: what it does, how big and how strong.
#[derive(Clone, Copy, Debug)]
pub struct Brush {
    pub mode: BrushMode,
    /// Radius as a fraction of the body height.
    pub radius: f32,
    /// 0..1 per dab at the brush centre.
    pub strength: f32,
    /// The weight Replace sets, or the factor Scale multiplies by.
    pub value: f32,
}

impl Default for Brush {
    fn default() -> Self {
        Brush {
            mode: BrushMode::Add,
            radius: 0.04,
            strength: 0.3,
            value: 0.5,
        }
    }
}

pub struct Studio {
    pub doc: Option<Doc>,
    pub camera: Camera,
    /// Selected component and, optionally, one of its guides.
    pub selected: Option<(usize, Option<usize>)>,
    /// Components picked with Shift, in pick order, each with the guide it was
    /// picked by. The last one is the parent when parenting.
    pub picked: Vec<(String, Option<usize>)>,
    pub symmetry: bool,
    pub mode: Mode,
    /// The floating outliner and properties panels are expanded.
    pub outliner_open: bool,
    pub properties_open: bool,
    /// The rigging window (component library) is open.
    pub library_open: bool,
    pub category: Category,
    /// The export window is open.
    pub export_open: bool,
    /// Custom naming being edited in the export window, before it's applied.
    pub naming_draft: Option<Naming>,
    pub show_mesh: bool,
    pub xray: bool,
    /// Joint and bone display: shown, drawn through the mesh, and their size.
    pub joints: JointDisplay,
    /// The joint display menu under the camera pill is open.
    pub display_open: bool,
    /// Deform bone whose weights colour the mesh (needs a built rig).
    pub weights_bone: Option<String>,
    /// The weight brush is on: dragging on the mesh paints `weights_bone`.
    pub painting: bool,
    pub brush: Brush,
    /// Where the brush last touched (or hovers over) the mesh.
    pub brush_at: Option<Vec3>,
    /// Keep hand-edited weights when the rig is built again.
    pub keep_weights: bool,
    /// Blender, Maya, Unreal scripts and the FBX file, in `Target::ALL` order.
    pub targets: [bool; 4],
    pub status: String,
    pub error: bool,
    /// The status is a warning: the operation worked, with something to look at.
    pub warn: bool,
    /// Bumped on every status change, so the UI can show it as a notification.
    pub status_serial: u64,
    pub finger_sides: FingerSides,
    pub tail_joints: usize,
    pub chain_parent: String,
    pub chain_joints: usize,
    pub chain_id: String,
    /// Bone or control picked in the build stages, shown in Properties.
    pub rig_pick: Option<RigPick>,
    pub pose: Option<Pose>,
    /// The control-rig pose, its control selection and manipulator.
    pub posing: posing::Posing,
    /// What a right-click in the viewport landed on, for its menu.
    pub menu_guide: Option<(usize, Option<usize>)>,
    pub menu_rig: Option<RigPick>,
    /// Component whose handle is being dragged onto a joint to attach it.
    pub handle_drag: Option<usize>,
    /// Templates found by the last scan of the template folders, and the files
    /// that couldn't be read. `None` until the first scan.
    pub templates: Option<(Vec<Listed>, Vec<String>)>,
    /// Vertex selection, locks, copied weights and the other weight tools.
    pub skin_tools: SkinTools,
    drag: Option<Drag>,
    /// Where the brush last dabbed in the stroke being painted.
    stroke: Option<Vec3>,
    weight_colors: Option<(String, Vec<Color32>)>,
    /// Shape lists before each shape edit, newest last.
    shape_undo: Vec<(Vec<hive_rig::shapes::Shape>, Vec<hive_rig::readers::Reader>)>,
    /// The shape the sculpt brush edits.
    pub shape_pick: Option<String>,
    /// The sculpt brush is on: dragging on the mesh moves vertices into `shape_pick`.
    pub sculpting: bool,
    /// Sculpt brush radius, as a fraction of the body height.
    pub sculpt_radius: f32,
    /// The sculpt brush, its strength and the target it sculpts.
    pub shape_tools: ShapeTools,
    /// The mesh images loaded for the viewport, one entry per mesh image
    /// (`None` where it couldn't be decoded), keyed by the document's serial.
    textures: Option<(u64, Vec<Option<egui::TextureHandle>>)>,
    /// Bumped whenever a document is opened, so caches can tell documents apart.
    doc_serial: u64,
    /// The scene's other characters and the props held across them.
    pub cast: cast::Cast,
    /// The user's custom control shapes, read on first use from `shape_lib_path`.
    shape_lib: Option<hive_rig::custom_shapes::ShapeLibrary>,
    pub shape_lib_path: Option<PathBuf>,
    /// A control shape being drawn in the shape editor.
    pub shape_draft: Option<control_shapes::ShapeDraft>,
    /// The in-app update (File > Update Hive Rig).
    pub updater: update::Updater,
}

impl Default for Studio {
    fn default() -> Self {
        Studio {
            doc: None,
            camera: Camera::default(),
            selected: None,
            picked: Vec::new(),
            rig_pick: None,
            pose: None,
            posing: posing::Posing::default(),
            menu_guide: None,
            menu_rig: None,
            handle_drag: None,
            templates: None,
            symmetry: true,
            mode: Mode::Guides,
            outliner_open: true,
            properties_open: true,
            library_open: false,
            category: Category::All,
            export_open: false,
            naming_draft: None,
            show_mesh: true,
            xray: false,
            joints: JointDisplay::default(),
            display_open: false,
            weights_bone: None,
            painting: false,
            brush: Brush::default(),
            brush_at: None,
            keep_weights: true,
            targets: [true, true, true, false],
            status: "Open a character to start, or try the demo.".into(),
            error: false,
            warn: false,
            status_serial: 0,
            finger_sides: FingerSides::Both,
            tail_joints: 5,
            chain_parent: String::new(),
            chain_joints: 3,
            chain_id: String::new(),
            skin_tools: SkinTools::default(),
            drag: None,
            stroke: None,
            weight_colors: None,
            shape_undo: Vec::new(),
            shape_pick: None,
            sculpting: false,
            sculpt_radius: 0.03,
            shape_tools: ShapeTools::default(),
            textures: None,
            doc_serial: 0,
            cast: cast::Cast::default(),
            shape_lib: None,
            shape_lib_path: control_shapes::default_library_path(),
            shape_draft: None,
            updater: update::Updater::default(),
        }
    }
}

/// The weights view's key for showing every bone's weights at once.
pub const ALL_BONES: &str = "\u{0}all bones";

// --------------------------------------------------------------------------- operations

/// "1 warning", "3 warnings".
fn count(n: usize, what: &str) -> String {
    if n == 1 {
        format!("1 {what}")
    } else {
        format!("{n} {what}s")
    }
}

fn hang(n: usize) -> &'static str {
    if n == 1 { "hangs" } else { "hang" }
}

impl Studio {
    fn info(&mut self, msg: impl Into<String>) {
        self.status = msg.into();
        self.error = false;
        self.warn = false;
        self.status_serial += 1;
    }

    fn warning(&mut self, msg: impl Into<String>) {
        self.info(msg);
        self.warn = true;
    }

    fn fail(&mut self, msg: impl Into<String>) {
        self.info(msg);
        self.error = true;
    }

    fn set_doc(&mut self, doc: Doc) {
        let (min, max) = doc.mesh.bounds();
        self.camera = Camera::default();
        self.camera.frame(min, max);
        self.selected = None;
        self.picked.clear();
        self.rig_pick = None;
        self.pose = None;
        self.weights_bone = None;
        self.weight_colors = None;
        self.painting = false;
        self.brush_at = None;
        self.skin_tools = SkinTools::default();
        self.drag = None;
        self.shape_undo.clear();
        self.shape_pick = None;
        self.sculpting = false;
        self.shape_tools = ShapeTools::default();
        self.textures = None;
        self.posing = posing::Posing::default();
        self.doc_serial += 1;
        self.doc = Some(doc);
    }

    /// Open a character (.glb, .gltf, .fbx, .obj), a saved rig (`.hive.json`) or, onto
    /// the current character, a guides file (`.guides.json`).
    pub fn open(&mut self, path: &Path) {
        let file = path
            .file_name()
            .map(|n| n.to_string_lossy().to_lowercase())
            .unwrap_or_default();
        let result = if file.ends_with(hive_rig::scene::SCENE_SUFFIX) {
            self.open_scene(path)
        } else if file.ends_with(".guides.json") {
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
        let (mesh, joints) = import::load_mesh_and_joints(path)?;
        mesh.validate()?;
        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "character".into());
        let out_dir = path
            .parent()
            .unwrap_or(Path::new("."))
            .join(format!("{stem}_rig"));
        self.place_character(
            stem,
            mesh,
            joints,
            Choice::default(),
            out_dir,
            &path.display().to_string(),
        )?;
        if let Some(doc) = &mut self.doc {
            doc.origin = Some(path.to_string_lossy().into_owned());
        }
        Ok(())
    }

    /// Turn the open character to `choice` (unset parts detected) and place its
    /// guides again. The view shows it turned at once.
    pub fn reorient(&mut self, choice: Choice) {
        let Some(doc) = &self.doc else { return };
        let Some(src) = &doc.source else {
            self.warning("Only a character opened from a file can be turned.");
            return;
        };
        let (name, mesh, joints, out_dir) = (
            doc.name.clone(),
            src.mesh.clone(),
            src.joints.clone(),
            doc.out_dir.clone(),
        );
        let (offset, origin) = (doc.offset, doc.origin.clone());
        if let Err(e) = self.place_character(name.clone(), mesh, joints, choice, out_dir, &name) {
            self.fail(format!("Couldn't turn the character: {e:#}"));
        }
        if let Some(doc) = &mut self.doc {
            (doc.offset, doc.origin) = (offset, origin);
        }
    }

    /// Orient a character read from a file, place its guides and open it.
    fn place_character(
        &mut self,
        stem: String,
        source_mesh: Mesh,
        source_joints: Vec<Joint>,
        choice: Choice,
        out_dir: PathBuf,
        shown: &str,
    ) -> anyhow::Result<()> {
        let ch = import::orient_character(source_mesh.clone(), source_joints.clone(), choice);
        let (mesh, joints, detection) = (ch.mesh, ch.joints, ch.detection);
        let (guides, found, from_joints) =
            hive_rig::rig::character_guides(&mesh, &joints, detection.body);
        let summary = format!(
            "{}; {} layout",
            detection.summary(),
            detection.body.template()
        );
        let mut doc = Doc::new(stem, mesh, guides, out_dir);
        doc.orientation = detection.orientation;
        doc.source = Some(SourceFile {
            mesh: source_mesh,
            joints: source_joints,
            detection,
        });
        let path = shown;
        let missing: Vec<&str> = [
            (found.crotch, "legs"),
            (found.arms, "arms"),
            (found.neck, "neck"),
        ]
        .into_iter()
        .filter(|(ok, _)| !ok)
        .map(|(_, what)| what)
        .collect();
        let verts = doc.mesh.vertex_count();
        self.set_doc(doc);
        match from_joints {
            Some(Ok(report)) if !report.placed.is_empty() => {
                let added = if report.added.is_empty() {
                    String::new()
                } else {
                    format!(", with {}", report.added.join(", "))
                };
                self.info(format!(
                    "Loaded {verts} vertices and read {} guides from its {} joints{added}. Orientation: {summary}. Check them, then Build Rig.",
                    report.placed.len(),
                    joints.len()
                ));
                return Ok(());
            }
            Some(Err(e)) => {
                self.fail(format!(
                    "Couldn't use this file's joints ({e:#}), so the guides were placed on the body instead."
                ));
                return Ok(());
            }
            _ => {}
        }
        if missing.is_empty() {
            self.info(format!(
                "Loaded {path} ({verts} vertices). Orientation: {summary}. Guides placed on the body: drag them to adjust, then Build Rig."
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
        let newer = rig.newer_warning();
        let dir = path.parent().unwrap_or(Path::new(".")).to_path_buf();
        let mut doc = Doc::new(rig.name.clone(), rig.mesh.clone(), rig.guides.clone(), dir);
        doc.orientation = rig.orientation;
        // The automatic weights of the saved guides tell its painted vertices apart.
        doc.auto_skin = Some(rig.auto_weights(SkinOptions::default()));
        doc.origin = Some(path.to_string_lossy().into_owned());
        let pose = rig.pose.clone();
        doc.rig = Some(rig);
        self.set_doc(doc);
        // The pose it was saved in comes back too.
        self.posing.pose = pose;
        match newer {
            Some(w) => self.warning(format!("Opened rig {}, but {w}.", path.display())),
            None => self.info(format!("Opened rig {}", path.display())),
        }
        Ok(())
    }

    fn open_guides(&mut self, path: &Path) -> anyhow::Result<()> {
        let guides = Guides::load(path)?;
        components::build(&guides)?;
        let newer = guides.newer_warning();
        let Some(doc) = &mut self.doc else {
            anyhow::bail!("open a character first, then its guides");
        };
        push_undo(&mut doc.undo, &doc.guides);
        doc.guides = guides;
        doc.refresh();
        let warnings = checks::warnings(&doc.checks);
        self.selected = None;
        let loaded = format!("Loaded guides from {}", path.display());
        match (newer, warnings) {
            (Some(w), _) => self.warning(format!("{loaded}, but {w}.")),
            (None, 0) => self.info(loaded),
            (None, n) => self.warning(format!("{loaded}: {}, see Checks.", count(n, "warning"))),
        }
        Ok(())
    }

    pub fn load_demo(&mut self) {
        let mesh = demo::humanoid(0.0);
        let (guides, _) = detect_guides(&mesh);
        let mut doc = Doc::new("hive_demo".into(), mesh, guides, demo_out_dir());
        doc.origin = Some("demo".into());
        self.set_doc(doc);
        self.info("Demo character loaded. Drag the guides to adjust, then Build Rig.");
    }

    /// Open a built-in creature (dog, bird, spider, snake, horse or dinosaur) with its layout.
    pub fn load_creature(&mut self, c: demo::Creature) {
        let mut doc = Doc::new(c.rig_name(), c.mesh(), c.guides(), demo_out_dir());
        doc.origin = Some(format!("demo:{}", c.name()));
        self.set_doc(doc);
        self.info(format!(
            "Demo {} loaded. Drag the guides to adjust, then Build Rig.",
            c.name()
        ));
    }

    /// A demo creature with `count` feathers a row in `rows` rows on its wings
    /// (or arms), and a vane on every feather in its mesh so they skin.
    pub fn load_feathered_creature(&mut self, c: demo::Creature, count: usize, rows: usize) {
        let mut guides = c.guides();
        if let Err(e) = components::set_feathers(&mut guides, None, count, rows) {
            self.fail(format!("Couldn't add feathers: {e:#}"));
            return self.load_creature(c);
        }
        let mesh = demo::with_feathers(c.mesh(), &guides);
        let mut doc = Doc::new(c.rig_name(), mesh, guides, demo_out_dir());
        doc.origin = Some(format!("demo:{}", c.name()));
        self.set_doc(doc);
        self.info(format!(
            "Demo {} loaded with {count} feathers a row on each wing.",
            c.name()
        ));
    }

    pub fn save_guides(&mut self, path: &Path) {
        let Some(doc) = &self.doc else { return };
        match doc.guides.save(path) {
            Ok(()) => self.info(format!("Saved guides to {}", path.display())),
            Err(e) => self.fail(format!("Couldn't save {}: {e:#}", path.display())),
        }
    }

    /// Apply a layout change as one undo step. A change that leaves the layout
    /// unbuildable is rolled back and reported.
    /// Set or clear one control's style (shape, size, colour), as one undo step.
    pub fn restyle_control(&mut self, control: &str, style: ControlStyle) {
        let name = control.to_string();
        self.edit("restyle the control", |g| {
            components::set_control_style(g, &name, style)?;
            Ok(format!("Restyled {name}."))
        });
    }

    /// Change the look of a control, its twin or its component's controls
    /// (see [`components::restyle`]), as one undo step.
    pub fn restyle_controls(
        &mut self,
        control: &str,
        change: components::StyleChange,
        scope: components::StyleScope,
    ) {
        let name = control.to_string();
        self.edit("restyle the control", |g| {
            let changed = components::restyle(g, &name, change, scope)?;
            Ok(match changed.len() {
                1 => format!("Restyled {name}."),
                n => format!("Restyled {name} and {} more.", n - 1),
            })
        });
    }

    /// Hang a component from a bone, as one undo step.
    pub fn attach_component(&mut self, comp: usize, bone: &str) {
        let Some(id) = self
            .doc
            .as_ref()
            .and_then(|d| d.guides.components.get(comp))
            .map(|c| c.id.clone())
        else {
            return;
        };
        let bone = bone.to_string();
        let guide = self
            .selected
            .and_then(|(c, g)| (c == comp).then_some(g).flatten());
        if self.edit("attach the component", |g| {
            components::attach(g, &id, &bone)?;
            Ok(format!("{id} now hangs from {bone}."))
        }) {
            self.reselect(vec![(id, guide)]);
        }
    }

    /// Keep the Shift picks in step with the plain selection: a selection made any
    /// other way (a click, a new part, undo) becomes the only pick.
    pub fn sync_picks(&mut self) {
        let Some(doc) = &self.doc else {
            self.picked.clear();
            return;
        };
        let comps = &doc.guides.components;
        self.picked
            .retain(|(id, _)| comps.iter().any(|c| &c.id == id));
        match self
            .selected
            .and_then(|(c, g)| comps.get(c).map(|c| (c.id.clone(), g)))
        {
            None => self.picked.clear(),
            Some((id, g)) => {
                if !self.picked.iter().any(|(p, _)| *p == id) {
                    self.picked = vec![(id, g)];
                }
            }
        }
    }

    /// Plain click: this component (and guide) alone.
    pub fn pick_one(&mut self, comp: usize, guide: Option<usize>) {
        self.selected = Some((comp, guide));
        self.picked.clear();
        self.sync_picks();
    }

    /// Press on a part that's already picked keeps the picks (so dragging one of
    /// several doesn't drop the rest); any other part becomes the only pick.
    pub fn keep_or_pick(&mut self, comp: usize, guide: Option<usize>) {
        self.sync_picks();
        let id = self.component_id(comp);
        if self.picked.len() > 1 && self.picked.iter().any(|(p, _)| Some(p) == id.as_ref()) {
            self.selected = Some((comp, guide));
        } else {
            self.pick_one(comp, guide);
        }
    }

    /// Shift+click: add a component to the picks, or drop it if it's already in.
    pub fn toggle_pick(&mut self, comp: usize, guide: Option<usize>) {
        self.sync_picks();
        let Some(id) = self.component_id(comp) else {
            return;
        };
        if let Some(at) = self.picked.iter().position(|(p, _)| *p == id) {
            self.picked.remove(at);
            let last = self.picked.last().cloned();
            self.selected = last.and_then(|(id, g)| self.component_index(&id).map(|c| (c, g)));
        } else {
            self.picked.push((id, guide));
            self.selected = Some((comp, guide));
        }
    }

    fn component_id(&self, comp: usize) -> Option<String> {
        let doc = self.doc.as_ref()?;
        doc.guides.components.get(comp).map(|c| c.id.clone())
    }

    pub fn component_index(&self, id: &str) -> Option<usize> {
        let doc = self.doc.as_ref()?;
        doc.guides.components.iter().position(|c| c.id == id)
    }

    /// The bone a part hangs from when parented under `comp`: the one nearest the
    /// guide it was picked by, else the component's first bone.
    pub fn parent_bone(&self, comp: &str, guide: Option<usize>) -> Option<String> {
        let doc = self.doc.as_ref()?;
        let built = doc.preview.as_ref().ok()?;
        let c = doc.guides.component(comp)?;
        let bones = built.skeleton.bones.iter().filter(|b| b.component == comp);
        match guide.and_then(|g| c.guides.get(g)) {
            Some(g) => bones
                .min_by(|a, b| {
                    a.head
                        .distance(g.position)
                        .total_cmp(&b.head.distance(g.position))
                })
                .map(|b| b.name.clone()),
            None => bones.map(|b| b.name.clone()).next(),
        }
    }

    /// Parent every pick under the last one picked, as one undo step.
    pub fn parent_picked(&mut self) {
        self.sync_picks();
        let Some(((parent, guide), children)) = self.picked.split_last() else {
            return;
        };
        if children.is_empty() {
            return self.fail("Shift-click the parts to parent, then the one they hang from last.");
        }
        let ids: Vec<String> = children.iter().map(|(id, _)| id.clone()).collect();
        let (parent, guide) = (parent.clone(), *guide);
        self.parent_under(&ids, &parent, guide);
    }

    /// Parent every pick under a guide of another component (right-click on it).
    pub fn parent_picked_to(&mut self, comp: usize, guide: Option<usize>) {
        self.sync_picks();
        let Some(parent) = self.component_id(comp) else {
            return;
        };
        let ids: Vec<String> = self
            .picked
            .iter()
            .map(|(id, _)| id.clone())
            .filter(|id| *id != parent)
            .collect();
        if ids.is_empty() {
            return self.fail("Pick the parts to parent first.");
        }
        self.parent_under(&ids, &parent, guide);
    }

    fn parent_under(&mut self, ids: &[String], parent: &str, guide: Option<usize>) {
        let Some(bone) = self.parent_bone(parent, guide) else {
            return self.fail(format!("{parent} has no bones to hang anything from."));
        };
        let keep = self.picked.clone();
        if self.edit("parent", |g| {
            for id in ids {
                components::attach(g, id, &bone)?;
            }
            Ok(format!(
                "{} now {} from {bone}.",
                ids.join(", "),
                hang(ids.len())
            ))
        }) {
            self.reselect(keep);
        }
    }

    /// Hang every pick straight from the root again, as one undo step.
    pub fn unparent_picked(&mut self) {
        self.sync_picks();
        let ids: Vec<String> = self
            .picked
            .iter()
            .map(|(id, _)| id.clone())
            .filter(|id| {
                self.doc
                    .as_ref()
                    .and_then(|d| d.guides.component(id))
                    .is_some_and(|c| c.kind != Kind::Root)
            })
            .collect();
        if ids.is_empty() {
            return self.fail("Pick a part to unparent (the root stays where it is).");
        }
        let keep = self.picked.clone();
        if self.edit("unparent", |g| {
            for id in &ids {
                components::attach(g, id, "root")?;
            }
            Ok(format!(
                "{} now {} from the root.",
                ids.join(", "),
                hang(ids.len())
            ))
        }) {
            self.reselect(keep);
        }
    }

    /// Parenting can reorder components, so find the picks again by id.
    fn reselect(&mut self, picks: Vec<(String, Option<usize>)>) {
        self.picked = picks;
        self.selected = self
            .picked
            .last()
            .and_then(|(id, g)| self.component_index(id).map(|c| (c, *g)));
    }

    /// Bring the components of another guides file in, hung from `bone`.
    pub fn import_components(&mut self, path: &Path, bone: &str) {
        let src = match Guides::load(path) {
            Ok(g) => g,
            Err(e) => return self.fail(format!("Couldn't read {}: {e:#}", path.display())),
        };
        let bone = bone.to_string();
        self.edit("import components", |g| {
            let added = components::import(g, &src, &bone)?;
            Ok(format!("Imported {} from {}.", added.join(", "), bone))
        });
    }

    /// Look for templates again (built-in ones and the template folders).
    pub fn scan_templates(&mut self) {
        self.templates = Some(template::list());
    }

    /// The listed templates, scanning the folders the first time.
    pub fn listed_templates(&mut self) -> &(Vec<Listed>, Vec<String>) {
        self.templates.get_or_insert_with(template::list)
    }

    /// Load a template onto the character as one undo step: a whole layout
    /// replaces the guides, parts are added (hung from `bone` when their parent
    /// isn't in the template and a bone is given).
    pub fn apply_template(&mut self, t: &Template, bone: Option<&str>) -> bool {
        let done = self.edit("load the template", |g| {
            let applied = t.apply(g, bone)?;
            Ok(match t.scope {
                Scope::Layout => format!(
                    "Loaded the {} layout: {} components.",
                    t.name,
                    applied.added.len()
                ),
                Scope::Parts => format!("Added {} from {}.", applied.summary(), t.name),
            })
        });
        if done {
            self.selected = None;
            self.picked.clear();
            self.mode = Mode::Guides;
        }
        done
    }

    /// Read a template file and load it onto the character.
    pub fn apply_template_file(&mut self, path: &Path, bone: Option<&str>) {
        match Template::load(path) {
            Ok(t) => {
                self.apply_template(&t, bone);
            }
            Err(e) => self.fail(format!("Couldn't load the template: {e:#}")),
        }
    }

    /// Save the whole layout, or only the picked components, as a template.
    pub fn save_template(&mut self, path: &Path, picked_only: bool) {
        let Some(doc) = &self.doc else { return };
        let only: Option<Vec<String>> =
            picked_only.then(|| self.picked.iter().map(|(id, _)| id.clone()).collect());
        let name = template::name_from_path(path);
        let result = Template::from_guides(&doc.guides, only.as_deref(), &name)
            .and_then(|t| t.save(path).map(|_| t));
        match result {
            Ok(t) => {
                self.scan_templates();
                let what = match t.scope {
                    Scope::Layout => "the layout".to_string(),
                    Scope::Parts => {
                        let ids: Vec<&str> = t
                            .components
                            .iter()
                            .map(|p| p.component.id.as_str())
                            .collect();
                        ids.join(", ")
                    }
                };
                self.info(format!(
                    "Saved {what} as the {} template in {}",
                    t.name,
                    path.display()
                ));
            }
            Err(e) => self.fail(format!("Couldn't save the template: {e:#}")),
        }
    }

    /// The bones, controls and skin as they show now, with any test pose applied.
    pub fn frame(&self) -> Option<Frame> {
        let doc = self.doc.as_ref()?;
        let built = doc.preview.as_ref().ok()?;
        let sk = &built.skeleton;
        let mut frame = Frame {
            bones: sk.bones.iter().map(|b| (b.head, b.tail)).collect(),
            controls: built.controls.controls.iter().map(|c| c.position).collect(),
            control_axes: built.controls.controls.iter().map(|c| c.axis).collect(),
            control_mats: None,
            mesh: None,
        };
        if self.pose_shows() {
            let posed = hive_rig::pose::evaluate_with(
                sk,
                &built.controls,
                &self.posing.pose,
                &doc.mesh.readers,
            );
            frame.bones = (0..sk.bones.len())
                .map(|i| (posed.head(sk, i), posed.tail(sk, i)))
                .collect();
            for (i, c) in built.controls.controls.iter().enumerate() {
                let m = posed.controls[i];
                frame.controls[i] = m.transform_point3(c.position);
                frame.control_axes[i] = m.transform_vector3(c.axis).normalize_or_zero();
            }
            frame.control_mats = Some(posed.controls.clone());
            if let Some(rig) = &doc.rig
                && rig.skeleton.bones.len() == sk.bones.len()
            {
                frame.mesh = Some(hive_rig::shapes::posed_mesh_board(
                    rig,
                    &posed.locals(),
                    &posed.bones,
                    &posed.board,
                ));
            }
            return Some(frame);
        }
        if self.pose.is_none() && self.shapes_show() {
            frame.mesh = self.shaped_rest();
        }
        let (Some(pose), Some(rig)) = (&self.pose, &doc.rig) else {
            return Some(frame);
        };
        let Some(pivot_bone) = sk.index_of(&pose.bone) else {
            return Some(frame);
        };
        let axis = [Vec3::X, Vec3::Y, Vec3::Z][pose.axis.min(2)];
        let radians = pose.degrees.to_radians();
        let pivot = sk.bones[pivot_bone].head;
        let moved: Vec<bool> = (0..sk.bones.len())
            .map(|i| sk.within(i, pivot_bone))
            .collect();
        for (i, (h, t)) in frame.bones.iter_mut().enumerate() {
            if moved[i] {
                *h = turn_about(pivot, axis, radians, *h);
                *t = turn_about(pivot, axis, radians, *t);
            }
        }
        for (i, c) in built.controls.controls.iter().enumerate() {
            let on = c
                .bone
                .as_deref()
                .and_then(|b| sk.index_of(b))
                .is_some_and(|b| moved[b]);
            if on {
                frame.controls[i] = turn_about(pivot, axis, radians, c.position);
                frame.control_axes[i] = turn_about(Vec3::ZERO, axis, radians, c.axis);
            }
        }
        frame.mesh = Some(
            if rig.mesh.shapes.is_empty() && rig.mesh.readers.is_empty() {
                skin::deform_turn(&doc.mesh.positions, &rig.skin, &moved, pivot, axis, radians)
            } else {
                // Shapes (and correctives driven by the pose) go on before the skin.
                hive_rig::shapes::posed_mesh(rig, &self.pose_locals())
            },
        );
        Some(frame)
    }

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
        // In the Rig stage the pose's own history comes first.
        if self.mode == Mode::Rig && self.posing.can_undo() {
            self.undo_pose();
            return;
        }
        let Some(doc) = &mut self.doc else { return };
        if let Some(rig) = &mut doc.rig
            && let Some(skin) = doc.skin_undo.pop()
        {
            doc.skin_redo.push(std::mem::replace(&mut rig.skin, skin));
            doc.exported = false;
            self.weight_colors = None;
            self.info("Undid the weight edit.");
            return;
        }
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
        if self.mode == Mode::Rig && self.posing.can_redo() {
            self.redo_pose();
            return;
        }
        let Some(doc) = &mut self.doc else { return };
        if let Some(rig) = &mut doc.rig
            && let Some(skin) = doc.skin_redo.pop()
        {
            doc.skin_undo.push(std::mem::replace(&mut rig.skin, skin));
            doc.exported = false;
            self.weight_colors = None;
            self.info("Redid the weight edit.");
            return;
        }
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
        (self.mode == Mode::Rig && self.posing.can_undo())
            || self
                .doc
                .as_ref()
                .is_some_and(|d| !d.undo.is_empty() || (d.rig.is_some() && !d.skin_undo.is_empty()))
    }

    pub fn can_redo(&self) -> bool {
        (self.mode == Mode::Rig && self.posing.can_redo())
            || self
                .doc
                .as_ref()
                .is_some_and(|d| !d.redo.is_empty() || (d.rig.is_some() && !d.skin_redo.is_empty()))
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

    /// Components carried along when `comp` moves: it and everything hanging
    /// from its bones, all the way down.
    pub fn component_family(&self, comp: usize) -> Vec<usize> {
        let Some(doc) = &self.doc else {
            return Vec::new();
        };
        let comps = &doc.guides.components;
        let owner = |bone: &str| -> Option<String> {
            let b = doc.preview.as_ref().ok()?.skeleton.bone(bone)?;
            Some(b.component.clone())
        };
        let mut family = vec![comp];
        let mut grew = true;
        while grew {
            grew = false;
            for (i, c) in comps.iter().enumerate() {
                if family.contains(&i) {
                    continue;
                }
                let hangs = c
                    .parent
                    .as_deref()
                    .and_then(owner)
                    .is_some_and(|o| family.iter().any(|&f| comps[f].id == o));
                if hangs {
                    family.push(i);
                    grew = true;
                }
            }
        }
        family
    }

    /// Move a whole component, by its first guide, to `to`. Everything hanging
    /// from it comes along, and with symmetry on its twin side mirrors the move.
    pub fn move_component(&mut self, comp: usize, to: Vec3) {
        let symmetry = self.symmetry;
        let family = self.component_family(comp);
        let Some(doc) = &mut self.doc else { return };
        let Some(first) = doc
            .guides
            .components
            .get(comp)
            .and_then(|c| c.guides.first())
            .map(|g| g.position)
        else {
            return;
        };
        let delta = to - first;
        let mut moved = Vec::new();
        for &c in &family {
            for (gi, g) in doc.guides.components[c].guides.iter_mut().enumerate() {
                g.position += delta;
                moved.push((c, gi));
            }
        }
        if symmetry {
            for &(c, gi) in &moved {
                if let Some((tc, tg)) = components::mirror_guide(&doc.guides, c, gi)
                    && !family.contains(&tc)
                {
                    let p = doc.guides.components[c].guides[gi].position;
                    doc.guides.components[tc].guides[tg].position =
                        components::mirror_point(&doc.guides, p);
                }
            }
        }
        doc.refresh();
        self.weight_colors = None;
    }

    /// Toes on the feet, on the same side choice as fingers.
    pub fn add_toes(&mut self) {
        let side = match self.finger_sides {
            FingerSides::Both => None,
            FingerSides::Left => Some(Side::Left),
            FingerSides::Right => Some(Side::Right),
        };
        self.add(
            Preset::Toes,
            AddOptions {
                side,
                ..AddOptions::default()
            },
        );
    }

    /// Three-segment hind legs on the pelvis, on the same side choice as fingers.
    /// A side that already has a leg is refused; remove that leg first.
    pub fn add_hind_legs(&mut self) {
        let side = match self.finger_sides {
            FingerSides::Both => None,
            FingerSides::Left => Some(Side::Left),
            FingerSides::Right => Some(Side::Right),
        };
        self.add(
            Preset::QuadLegs,
            AddOptions {
                side,
                ..AddOptions::default()
            },
        );
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

    /// A curve-driven tentacle from the pelvis (8 joints, 4 curve controls);
    /// its counts change in Properties.
    pub fn add_tentacle(&mut self) {
        self.add(Preset::Tentacle, AddOptions::default());
    }

    /// Forking FK chains on each side of the head; more forks come from Properties.
    pub fn add_antlers(&mut self) {
        self.add(Preset::Antlers, AddOptions::default());
    }

    /// Fork chain `id` at its guide `from` with a branch of `joints` (and its twin).
    pub fn add_branch(&mut self, id: &str, from: &str, joints: usize) {
        let (id, from) = (id.to_string(), from.to_string());
        self.edit("add a branch", |g| {
            let names = components::add_branch(g, &id, &from, joints)?;
            Ok(format!("{id} forks at {from}: {} new guides.", names.len()))
        });
    }

    /// Feathers on a wing and its twin: `count` a row in `rows` rows (0 off).
    pub fn set_feathers(&mut self, id: &str, count: usize, rows: usize) {
        let id = id.to_string();
        self.edit("change the feathers", |g| {
            let ids = components::set_feathers(g, Some(&id), count, rows)?;
            Ok(if count == 0 {
                format!("No feathers on {}.", ids.join(" and "))
            } else {
                format!(
                    "{}: {count} feathers a row in {}.",
                    ids.join(" and "),
                    self::count(rows, "row")
                )
            })
        });
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

    /// Face joints (brows, eyelids, cheeks, nose, lips), pushed onto the face.
    pub fn add_face(&mut self) {
        self.add(Preset::Face, AddOptions::default());
        self.snap_to_face(Kind::Face, &components::FACE_GUIDES, 0.003);
    }

    /// Copy one side's skin weights onto the other on the built rig.
    pub fn mirror_weights(&mut self, source: Side) {
        let Some(doc) = &mut self.doc else { return };
        doc.push_skin_undo();
        let Some(rig) = &mut doc.rig else {
            self.fail("Build the rig first, then mirror its weights.");
            return;
        };
        let body = rig.guides.body;
        rig.skin = hive_rig::skin::mirror_weights(
            &rig.skin,
            &rig.mesh,
            &rig.skeleton,
            body.center_x,
            body.height,
            source,
        );
        doc.exported = false;
        self.weight_colors = None;
        let (from, to) = match source {
            Side::Right => ("right", "left"),
            _ => ("left", "right"),
        };
        self.info(format!("Copied the {from} side's weights onto the {to}."));
    }

    /// Copy the skin weights of the rig saved at `path` (another character,
    /// or this one built earlier) onto the built rig, matching bones by name.
    pub fn copy_weights_from(&mut self, path: &Path) -> bool {
        let from = match Rig::load(path) {
            Ok(r) => r,
            Err(e) => {
                self.fail(format!("Couldn't read {}: {e:#}", path.display()));
                return false;
            }
        };
        let Some(doc) = &mut self.doc else {
            return false;
        };
        let Some(rig) = &doc.rig else {
            self.fail("Build the rig first, then copy weights onto it.");
            return false;
        };
        let (skin, report) = hive_rig::rig::copy_weights(&from, rig);
        doc.push_skin_undo();
        if let Some(rig) = &mut doc.rig {
            rig.skin = skin;
        }
        doc.exported = false;
        self.weight_colors = None;
        let name = path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into_owned(),
        );
        if report.unmatched.is_empty() {
            self.info(format!(
                "Copied the weights of {name} ({} bones matched).",
                report.matched
            ));
        } else {
            self.warning(format!(
                "Copied the weights of {name}; {} had no match and took their neighbours' skin: {}.",
                report.unmatched.len(),
                report.unmatched.join(", ")
            ));
        }
        true
    }

    /// Smooth the built rig's weights, around the bone shown in the weights view
    /// or everywhere.
    pub fn smooth_weights(&mut self) {
        let Some(doc) = &mut self.doc else { return };
        doc.push_skin_undo();
        let Some(rig) = &mut doc.rig else {
            self.fail("Build the rig first, then smooth its weights.");
            return;
        };
        let bone = self
            .weights_bone
            .as_deref()
            .and_then(|b| rig.skeleton.index_of(b));
        let locks = skin::edit::lock_mask(
            &rig.skeleton,
            self.skin_tools.locked.iter().map(String::as_str),
        );
        rig.skin = hive_rig::skin::smooth(
            &rig.skin,
            &rig.mesh,
            &rig.skeleton,
            bone,
            2,
            rig.guides.skin,
            &locks,
        );
        doc.exported = false;
        self.weight_colors = None;
        let what = self.weights_bone.as_deref().unwrap_or("every bone");
        self.info(format!("Smoothed the weights of {what}."));
    }

    /// Change the rig's skin influence limits (stored with the guides, so it
    /// needs a rebuild; edited weights are kept).
    pub fn set_skin_limits(&mut self, limits: SkinLimits) -> bool {
        if self.doc.as_ref().is_some_and(|d| d.guides.skin == limits) {
            return true;
        }
        self.edit("change the skin limits", |g| {
            limits.validate()?;
            g.skin = limits;
            Ok(format!(
                "Skin limits: up to {} influences per vertex{}. Build to apply them.",
                limits.max_influences,
                if limits.prune > 0.0 {
                    format!(", pruning below {}", limits.prune)
                } else {
                    String::new()
                }
            ))
        })
    }

    /// The mesh's images as viewport textures, one per mesh image (`None` where
    /// an image couldn't be read or decoded). Decoded once per document, and
    /// scaled down to at most `TEXTURE_MAX` pixels a side: the viewport needs
    /// no more.
    pub fn mesh_textures(&mut self, ctx: &egui::Context) -> Vec<Option<egui::TextureId>> {
        const TEXTURE_MAX: u32 = 2048;
        let Some(doc) = &self.doc else {
            return Vec::new();
        };
        let fresh = self.textures.as_ref().is_some_and(|(serial, t)| {
            *serial == self.doc_serial && t.len() == doc.mesh.images.len()
        });
        if !fresh {
            let handles = doc
                .mesh
                .images
                .iter()
                .enumerate()
                .map(|(i, img)| {
                    let bytes = img.bytes()?;
                    let mut decoded = image::load_from_memory(&bytes).ok()?;
                    if decoded.width() > TEXTURE_MAX || decoded.height() > TEXTURE_MAX {
                        decoded = decoded.thumbnail(TEXTURE_MAX, TEXTURE_MAX);
                    }
                    let rgba = decoded.to_rgba8();
                    let size = [rgba.width() as usize, rgba.height() as usize];
                    let pixels = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
                    Some(ctx.load_texture(
                        format!("mesh-image-{}-{i}", self.doc_serial),
                        pixels,
                        egui::TextureOptions::LINEAR_REPEAT,
                    ))
                })
                .collect();
            self.textures = Some((self.doc_serial, handles));
        }
        self.textures
            .as_ref()
            .map(|(_, t)| {
                t.iter()
                    .map(|h| h.as_ref().map(egui::TextureHandle::id))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The mesh positions as drawn: posed by the test pose, or at rest.
    fn shown_positions(&self) -> Option<Vec<Vec3>> {
        let doc = self.doc.as_ref()?;
        Some(
            self.frame()
                .and_then(|f| f.mesh)
                .unwrap_or_else(|| doc.mesh.positions.clone()),
        )
    }

    /// The nearest point where a ray meets the mesh as drawn.
    pub fn hit_mesh(&self, origin: Vec3, dir: Vec3) -> Option<Vec3> {
        let doc = self.doc.as_ref()?;
        let positions = self.shown_positions()?;
        let mut best: Option<f32> = None;
        for [a, b, c] in doc.mesh.triangles() {
            if let Some(t) = ray_triangle(origin, dir, positions[a], positions[b], positions[c])
                && best.is_none_or(|b| t < b)
            {
                best = Some(t);
            }
        }
        best.map(|t| origin + dir * t)
    }

    /// The bone the brush paints: the one shown in the weights view.
    fn paint_bone(&self) -> Option<usize> {
        let rig = self.doc.as_ref()?.rig.as_ref()?;
        rig.skeleton.index_of(self.weights_bone.as_deref()?)
    }

    /// Start a brush stroke: one undo step for the whole stroke.
    pub fn begin_stroke(&mut self) -> bool {
        let Some(bone) = self.paint_bone() else {
            self.fail("Build the rig and pick a bone under Weights, then paint it.");
            return false;
        };
        let evens_out = matches!(self.brush.mode, BrushMode::Smooth | BrushMode::Mend);
        if !evens_out && self.lock_mask().get(bone).copied().unwrap_or(false) {
            let name = self.weights_bone.clone().unwrap_or_default();
            self.fail(format!("{name} is locked; unlock it to paint it."));
            return false;
        }
        if let Some(doc) = &mut self.doc {
            doc.push_skin_undo();
        }
        true
    }

    /// One dab of the brush at `at` on the mesh, and its mirror image on the
    /// twin bone when symmetry is on.
    pub fn paint_at(&mut self, at: Vec3) -> usize {
        let Some(bone) = self.paint_bone() else {
            return 0;
        };
        let positions = self.shown_positions().unwrap_or_default();
        let (symmetry, brush) = (self.symmetry, self.brush);
        let (locks, adjacency) = (self.lock_mask(), self.neighbours());
        let Some(doc) = &mut self.doc else { return 0 };
        let Some(rig) = &mut doc.rig else { return 0 };
        let body = rig.guides.body;
        let mut dabs = vec![(bone, at)];
        if symmetry {
            let twins = skin::bone_twins(&rig.skeleton, body.center_x, 0.05 * body.height);
            let mirrored = hive_rig::landmarks::mirror(at, body.center_x);
            if mirrored.distance(at) > 1e-4 * body.height {
                dabs.push((twins[bone], mirrored));
            }
        }
        let mut changed = 0;
        for (b, center) in dabs {
            changed += skin::paint(
                &mut rig.skin,
                &positions,
                &adjacency,
                &rig.skeleton,
                b,
                Dab {
                    center,
                    radius: brush.radius * body.height,
                    strength: brush.strength,
                    mode: brush.mode,
                    value: brush.value,
                },
                rig.guides.skin,
                &locks,
            );
        }
        if changed > 0 {
            doc.exported = false;
            self.weight_colors = None;
        }
        self.brush_at = Some(at);
        changed
    }

    /// Put the brush on the surface over the middle of the painted bone, so it
    /// shows without a pointer (for the `--paint` screenshot).
    pub fn show_brush(&mut self) {
        self.painting = true;
        let Some(bone) = self.paint_bone() else {
            return;
        };
        let Some(positions) = self.shown_positions() else {
            return;
        };
        let rig = self.doc.as_ref().unwrap().rig.as_ref().unwrap();
        let b = &rig.skeleton.bones[bone];
        let mid = (b.head + b.tail) * 0.5;
        // The front of the mesh over the bone's middle, else its nearest vertex.
        let front = mid + Vec3::Z * (2.0 * rig.guides.body.height);
        self.brush_at = self.hit_mesh(front, -Vec3::Z).or_else(|| {
            positions
                .iter()
                .min_by(|p, q| p.distance(mid).total_cmp(&q.distance(mid)))
                .copied()
        });
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

    /// Change how exported joints and controls are named (one undo step).
    pub fn set_naming(&mut self, naming: Naming) -> bool {
        let label = naming.preset().map_or("custom", |p| p.label()).to_string();
        let example = naming.bone("upperarm_l");
        self.edit("change the naming", move |g| {
            naming.validate()?;
            g.naming = naming;
            Ok(format!(
                "Naming set to {label}: upperarm_l exports as {example}."
            ))
        })
    }

    pub fn build(&mut self) -> bool {
        let keep = self.keep_weights;
        let Some(doc) = &mut self.doc else {
            return false;
        };
        match build_rig(
            doc.mesh.clone(),
            &doc.name,
            doc.guides.clone(),
            RigOptions::default(),
        ) {
            Ok(mut rig) => {
                rig.orientation = doc.orientation;
                let auto = rig.skin.clone();
                // Building again over a built rig keeps its edits too.
                let old = match doc.rig.take() {
                    Some(r) => Some((r, doc.auto_skin.take())),
                    None => doc.kept.take(),
                };
                doc.kept = None;
                let kept = match old {
                    Some((old, old_auto)) if keep => {
                        hive_rig::rig::keep_weights(&old, old_auto.as_ref(), &mut rig)
                    }
                    _ => 0,
                };
                let warnings = checks::warnings(&doc.checks);
                let mut msg = format!(
                    "Rig built: {} bones, {} controls, {} IK limbs, weights on {} vertices.",
                    rig.skeleton.bones.len(),
                    rig.controls.controls.len(),
                    rig.controls.ik_chains.len(),
                    rig.mesh.vertex_count()
                );
                if kept > 0 {
                    msg = format!("{msg} Kept your edited weights on {kept} vertices.");
                }
                doc.auto_skin = Some(auto);
                doc.skin_undo.clear();
                doc.skin_redo.clear();
                doc.rig = Some(rig);
                self.weight_colors = None;
                self.mode = Mode::Rig;
                if warnings == 0 {
                    self.info(format!(
                        "{msg} Pick a bone under Weights to check them, or Export."
                    ));
                } else {
                    self.warning(format!(
                        "{msg} {} to look at: see Checks in Properties.",
                        count(warnings, "warning")
                    ));
                }
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
        // The pose posed in the Rig stage goes with the rig (and into the
        // Blender and Maya pose libraries).
        rig.pose = self.posing.pose.clone();
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

    /// Select the guide (or component) a check is about.
    pub fn select_issue(&mut self, i: usize) {
        let Some(doc) = &self.doc else { return };
        let Some(issue) = doc.checks.get(i) else {
            return;
        };
        let Some(c) = doc
            .guides
            .components
            .iter()
            .position(|c| c.id == issue.component)
        else {
            return;
        };
        let guide = issue.guide.as_ref().and_then(|g| {
            doc.guides.components[c]
                .guides
                .iter()
                .position(|x| &x.name == g)
        });
        self.mode = Mode::Guides;
        self.pick_one(c, guide);
    }

    /// Apply one check's fix, as one undo step.
    pub fn fix_issue(&mut self, i: usize) -> bool {
        let Some(fix) = self
            .doc
            .as_ref()
            .and_then(|d| d.checks.get(i))
            .and_then(|i| i.fix.clone())
        else {
            return false;
        };
        self.edit("apply the fix", |g| checks::apply_fix(g, &fix))
    }

    /// Apply every fix the checks offer, as one undo step.
    pub fn fix_all_issues(&mut self) -> bool {
        let Some(issues) = self.doc.as_ref().map(|d| d.checks.clone()) else {
            return false;
        };
        self.edit("apply the fixes", |g| {
            let n = checks::fix_all(g, &issues)?;
            Ok(if n == 1 {
                "Applied 1 fix.".to_string()
            } else {
                format!("Applied {n} fixes.")
            })
        })
    }

    /// The mesh's colours in the weights view: one bone's weights as a heat
    /// map, or, for [`ALL_BONES`], every bone in its own colour, blended.
    fn weights_for(&mut self, bone: &str) -> Option<&[Color32]> {
        let doc = self.doc.as_ref()?;
        let rig = doc.rig.as_ref()?;
        if self.weight_colors.as_ref().is_none_or(|(b, _)| b != bone) {
            let colors = if bone == ALL_BONES {
                (0..rig.skin.vertex_count())
                    .map(|v| {
                        let mut c = [0.0f32; 3];
                        for (j, w) in rig.skin.influences(v) {
                            let b = view::bone_color(j);
                            for (k, x) in [b.r(), b.g(), b.b()].into_iter().enumerate() {
                                c[k] += w * f32::from(x);
                            }
                        }
                        let [r, g, b] = c.map(|x| x.round().clamp(0.0, 255.0) as u8);
                        Color32::from_rgb(r, g, b)
                    })
                    .collect()
            } else {
                let index = rig.skeleton.index_of(bone)?;
                (0..rig.skin.vertex_count())
                    .map(|v| view::weight_color(rig.skin.weight(v, index)))
                    .collect()
            };
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
                from: None,
            })
            .collect(),
        options: Options::default(),
    }
}

/// Distance along the ray to where it crosses triangle abc, if it does.
fn ray_triangle(origin: Vec3, dir: Vec3, a: Vec3, b: Vec3, c: Vec3) -> Option<f32> {
    let (e1, e2) = (b - a, c - a);
    let p = dir.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let s = origin - a;
    let u = s.dot(p) * inv;
    if !(0.0..=1.0).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = dir.dot(q) * inv;
    if v < 0.0 || u + v > 1.0 {
        return None;
    }
    let t = e2.dot(q) * inv;
    (t > 0.0).then_some(t)
}

/// Where the demo characters' rigs are exported: `demo_rig` next to the program.
fn demo_out_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(Path::to_path_buf))
        .unwrap_or_default()
        .join("demo_rig")
}

/// The outliner's rows as (component index, depth): the attachment tree in
/// depth-first order, each component right under the one owning its parent
/// bone, siblings in layout order. Bone owners come from `built`; without it
/// (a layout that doesn't build) every component is a top-level row.
pub fn outliner_rows(g: &Guides, built: Option<&Built>) -> Vec<(usize, usize)> {
    let n = g.components.len();
    let index: HashMap<&str, usize> = g
        .components
        .iter()
        .enumerate()
        .map(|(i, c)| (c.id.as_str(), i))
        .collect();
    let owner: HashMap<&str, usize> = built
        .map(|b| {
            b.skeleton
                .bones
                .iter()
                .filter_map(|b| Some((b.name.as_str(), *index.get(b.component.as_str())?)))
                .collect()
        })
        .unwrap_or_default();
    let parent: Vec<Option<usize>> = g
        .components
        .iter()
        .enumerate()
        .map(|(i, c)| {
            c.parent
                .as_deref()
                .and_then(|p| owner.get(p).copied())
                .filter(|&o| o != i)
        })
        .collect();
    let mut children = vec![Vec::new(); n];
    let mut tops = Vec::new();
    for (i, p) in parent.iter().enumerate() {
        match p {
            Some(p) => children[*p].push(i),
            None => tops.push(i),
        }
    }
    let mut rows = Vec::with_capacity(n);
    let mut seen = vec![false; n];
    let mut stack: Vec<(usize, usize)> = tops.iter().rev().map(|&i| (i, 0)).collect();
    loop {
        while let Some((i, depth)) = stack.pop() {
            if std::mem::replace(&mut seen[i], true) {
                continue;
            }
            rows.push((i, depth));
            stack.extend(children[i].iter().rev().map(|&c| (c, depth + 1)));
        }
        // Parents that loop back on each other can't come from a built rig;
        // list any such component at the top rather than lose it.
        match seen.iter().position(|s| !s) {
            Some(i) => stack.push((i, 0)),
            None => break,
        }
    }
    rows
}

fn push_undo(undo: &mut Vec<Guides>, g: &Guides) {
    undo.push(g.clone());
    if undo.len() > UNDO_LIMIT {
        undo.remove(0);
    }
}

mod cast;
mod control_shapes;
mod posing;
mod readers;
mod shapes;
mod skin_edit;
mod ui;
pub mod update;

pub use control_shapes::ShapeDraft;
pub use posing::Tool;
pub use shapes::ShapeTools;
pub use skin_edit::{MeshTool, SelectOp, SkinTools};

#[cfg(test)]
mod tests {
    use super::*;
    use hive_rig::custom_shapes;
    use hive_rig::shapes::SculptBrush;

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

    /// A character saved lying along +Z opens upright, can be turned by hand,
    /// and the built rig keeps the orientation it was opened with.
    #[test]
    fn opens_a_turned_character_upright() {
        use hive_rig::orient::Axis;
        let o = Orientation::new(Axis::PosZ, Axis::NegY).unwrap();
        let mut rig =
            hive_rig::auto_rig(demo::humanoid(0.0), "turned", RigOptions::default()).unwrap();
        rig.orientation = o;
        let dir = std::env::temp_dir().join(format!("hive_studio_turned_{}", std::process::id()));
        export::write_all(&rig, &dir, &[]).unwrap();
        let mut s = Studio::default();
        s.open(&dir.join("turned.glb"));
        assert!(!s.error, "{}", s.status);
        let doc = s.doc.as_ref().unwrap();
        assert_eq!(doc.orientation, o);
        let (lo, hi) = doc.mesh.bounds();
        assert!(hi.y - lo.y > 2.0 * (hi.z - lo.z), "{lo} {hi}");
        assert!(s.build(), "{}", s.status);
        assert_eq!(s.doc.as_ref().unwrap().rig.as_ref().unwrap().orientation, o);
        // Forcing the file's own axes leaves it lying down.
        s.reorient(Choice {
            up: Some(Axis::PosY),
            forward: Some(Axis::PosZ),
        });
        let doc = s.doc.as_ref().unwrap();
        assert!(doc.orientation.is_standard());
        let (lo, hi) = doc.mesh.bounds();
        assert!(hi.z - lo.z > 2.0 * (hi.y - lo.y), "{lo} {hi}");
        // Detecting again stands it up.
        s.reorient(Choice::default());
        assert_eq!(s.doc.as_ref().unwrap().orientation, o);
        std::fs::remove_dir_all(&dir).ok();
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
    fn hind_legs_replace_the_biped_legs() {
        let mut s = demo_studio();
        // The biped already has legs, so the hind legs are refused.
        s.add_hind_legs();
        assert!(s.error);
        for id in ["leg_l", "leg_r"] {
            s.selected = Some((comp(&s, id), None));
            s.remove_selected();
        }
        s.add_hind_legs();
        assert!(!s.error, "{}", s.status);
        let g = &s.doc.as_ref().unwrap().guides;
        assert_eq!(g.component("hind_leg_r").unwrap().kind, Kind::QuadLeg);
        let built = components::build(g).unwrap();
        assert!(built.skeleton.bone("hock_l").is_some());
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
        s.targets = [true, false, true, true];
        let files = s.export(&dir).unwrap();
        let names: Vec<String> = files
            .iter()
            .map(|f| f.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert!(names.contains(&"hive_demo_blender.py".to_string()));
        assert!(names.contains(&"hive_demo.guides.json".to_string()));
        assert!(names.contains(&"hive_demo.fbx".to_string()));
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
    fn naming_is_undoable_and_reaches_the_export() {
        use hive_rig::naming::Preset;
        let mut s = demo_studio();
        assert!(s.set_naming(Preset::Prefix.naming()), "{}", s.status);
        assert!(s.build());
        let rig = s.doc.as_ref().unwrap().rig.as_ref().unwrap();
        assert!(rig.skeleton.bone("L_upperarm_jnt").is_some());
        assert!(rig.controls.control("L_hand_ik_ctrl").is_some());
        // Weights are looked up by the exported name.
        assert!(s.weights_for("L_upperarm_jnt").is_some());

        let mut bad = Preset::Prefix.naming();
        bad.joint = "{name}".into();
        assert!(!s.set_naming(bad) && s.error);
        assert_eq!(
            s.doc.as_ref().unwrap().guides.naming,
            Preset::Prefix.naming()
        );

        s.undo();
        assert!(s.doc.as_ref().unwrap().guides.naming.is_default());
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
    fn tentacle_and_spline_spine() {
        let mut s = demo_studio();
        s.add_tentacle();
        assert!(!s.error, "{}", s.status);
        let built = components::build(&s.doc.as_ref().unwrap().guides).unwrap();
        let spline = built
            .controls
            .splines
            .iter()
            .find(|c| c.name == "tentacle")
            .unwrap();
        assert_eq!((spline.bones.len(), spline.controls.len()), (8, 4));
        s.edit("change the spine", |g| {
            components::set_spine_spline(g, "spine", true, Some(4))?;
            Ok(String::new())
        });
        assert!(!s.error, "{}", s.status);
        let built = components::build(&s.doc.as_ref().unwrap().guides).unwrap();
        assert!(built.controls.control("hips_ctrl").is_some());
        assert!(built.controls.control("spine_mid_02_ctrl").is_some());
        // One undo brings the FK spine back.
        s.undo();
        let built = components::build(&s.doc.as_ref().unwrap().guides).unwrap();
        assert!(built.controls.control("spine_01_ctrl").is_some());
        assert_eq!(built.controls.splines.len(), 1);
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

    #[test]
    fn painted_weights_undo_and_survive_a_rebuild() {
        let mut s = demo_studio();
        assert!(s.build(), "{}", s.status);
        s.weights_bone = Some("upperarm_l".into());
        s.brush = Brush {
            mode: BrushMode::Subtract,
            radius: 0.03,
            strength: 1.0,
            value: 0.5,
        };
        s.show_brush();
        let at = s.brush_at.expect("the brush lands on the arm");
        fn rig(s: &Studio) -> &Rig {
            s.doc.as_ref().unwrap().rig.as_ref().unwrap()
        }
        let before = rig(&s).skin.clone();
        let (up_l, up_r) = (
            rig(&s).skeleton.index_of("upperarm_l").unwrap(),
            rig(&s).skeleton.index_of("upperarm_r").unwrap(),
        );
        let m = rig(&s).mesh.positions.clone();
        let nearest = |p: Vec3| {
            (0..m.len())
                .min_by(|a, b| m[*a].distance(p).total_cmp(&m[*b].distance(p)))
                .unwrap()
        };
        let v = nearest(at);
        let twin = nearest(hive_rig::landmarks::mirror(
            at,
            rig(&s).guides.body.center_x,
        ));
        assert!(s.begin_stroke());
        assert!(s.paint_at(at) > 0);
        let painted = rig(&s).skin.clone();
        assert!(painted.weight(v, up_l) < 0.5 * before.weight(v, up_l));
        // Symmetry paints the right arm too.
        assert!(painted.weight(twin, up_r) < 0.5 * before.weight(twin, up_r));
        assert!(s.can_undo());
        s.undo();
        assert_eq!(rig(&s).skin, before);
        s.redo();
        assert_eq!(rig(&s).skin, painted);

        // A guide edit clears the rig; building again keeps the stroke.
        s.add_tail();
        assert!(s.doc.as_ref().unwrap().rig.is_none());
        assert!(s.build(), "{}", s.status);
        assert!(
            s.status.contains("Kept your edited weights"),
            "{}",
            s.status
        );
        assert!((rig(&s).skin.weight(v, up_l) - painted.weight(v, up_l)).abs() < 1e-4);
        // ...and so does raising the influence limit, which the new rig follows.
        let limits = SkinLimits {
            max_influences: 8,
            prune: 0.0,
        };
        assert!(s.set_skin_limits(limits));
        assert!(s.build(), "{}", s.status);
        assert_eq!(rig(&s).guides.skin, limits);
        assert!(rig(&s).skin.width() > 4);
        assert!((rig(&s).skin.weight(v, up_l) - painted.weight(v, up_l)).abs() < 1e-4);
        // With keep off the rebuild starts from the automatic weights again.
        s.keep_weights = false;
        assert!(s.build(), "{}", s.status);
        assert!(rig(&s).skin.weight(v, up_l) > 0.5 * before.weight(v, up_l));
    }

    #[test]
    fn face_and_weight_tools() {
        let mut s = demo_studio();
        s.add_face();
        assert!(!s.error, "{}", s.status);
        s.add_face();
        assert!(s.error, "a rig has one face");
        assert!(s.build(), "{}", s.status);
        let rig = s.doc.as_ref().unwrap().rig.as_ref().unwrap();
        assert!(rig.skeleton.bone("brow_in_l").is_some_and(|b| b.translate));
        let before = serde_json::to_string(&rig.skin).unwrap();
        s.weights_bone = Some("brow_in_l".into());
        s.smooth_weights();
        s.mirror_weights(Side::Left);
        assert!(!s.error, "{}", s.status);
        let rig = s.doc.as_ref().unwrap().rig.as_ref().unwrap();
        assert_ne!(serde_json::to_string(&rig.skin).unwrap(), before);
        assert!(!s.doc.as_ref().unwrap().exported);
    }

    #[test]
    fn test_pose_turns_the_skin_and_controls_are_restyled() {
        let mut s = demo_studio();
        assert!(s.build(), "{}", s.status);
        let rest = s.frame().unwrap();
        assert!(rest.mesh.is_none());
        s.start_pose("upperarm_l");
        s.pose.as_mut().unwrap().degrees = 60.0;
        let posed = s.frame().unwrap();
        let skinned = posed.mesh.as_ref().unwrap();
        assert!(
            skinned
                .iter()
                .zip(&s.doc.as_ref().unwrap().mesh.positions)
                .any(|(a, b)| a.distance(*b) > 0.01)
        );
        // The chest, which doesn't hang from the arm, stays put.
        let built = s.doc.as_ref().unwrap().preview.as_ref().unwrap();
        let spine = built.skeleton.index_of("spine_01").unwrap();
        assert_eq!(posed.bones[spine], rest.bones[spine]);

        // A control's shape is saved in the guides and survives a rebuild.
        let name = built.controls.controls[0].name.clone();
        s.restyle_control(
            &name,
            hive_rig::components::ControlStyle {
                shape: Some(hive_rig::controls::Shape::Cross),
                ..Default::default()
            },
        );
        assert!(!s.error, "{}", s.status);
        let rebuilt = s.doc.as_ref().unwrap().preview.as_ref().unwrap();
        assert_eq!(
            rebuilt.controls.control(&name).unwrap().shape,
            hive_rig::controls::Shape::Cross
        );
        s.undo();
        let back = s.doc.as_ref().unwrap().preview.as_ref().unwrap();
        assert_ne!(
            back.controls.control(&name).unwrap().shape,
            hive_rig::controls::Shape::Cross
        );
    }

    #[test]
    fn restyling_with_symmetry_reaches_the_twin_in_one_undo_step() {
        use hive_rig::components::{StyleChange, StyleScope};
        use hive_rig::controls::Shape;
        let mut s = demo_studio();
        let style = |s: &Studio, n: &str| {
            s.doc
                .as_ref()
                .unwrap()
                .guides
                .control_styles
                .get(n)
                .cloned()
        };
        let change = StyleChange {
            shape: Some(Shape::CurvedArrow),
            color: Some([0.1, 0.45, 1.0]),
            ..Default::default()
        };
        s.restyle_controls("hand_l_ik_ctrl", change, StyleScope::WithTwin);
        assert!(!s.error, "{}", s.status);
        let (l, r) = (
            style(&s, "hand_l_ik_ctrl").unwrap(),
            style(&s, "hand_r_ik_ctrl").unwrap(),
        );
        assert_eq!(
            (l.shape, r.shape),
            (Some(Shape::CurvedArrow), Some(Shape::CurvedArrow))
        );
        assert_eq!(r.color, Some([1.0, 0.15, 0.15]), "the side colour swaps");
        let preview = s.doc.as_ref().unwrap().preview.as_ref().unwrap();
        assert_eq!(
            preview.controls.control("hand_r_ik_ctrl").unwrap().shape,
            Shape::CurvedArrow
        );
        // The whole component, without its twin.
        s.restyle_controls(
            "upperarm_l_fk_ctrl",
            StyleChange {
                shape: Some(Shape::Plus),
                ..Default::default()
            },
            StyleScope::Component { twins: false },
        );
        assert_eq!(
            style(&s, "lowerarm_l_fk_ctrl").unwrap().shape,
            Some(Shape::Plus)
        );
        assert_ne!(
            style(&s, "lowerarm_r_fk_ctrl").and_then(|x| x.shape),
            Some(Shape::Plus)
        );
        s.undo();
        s.undo();
        assert!(s.doc.as_ref().unwrap().guides.control_styles.is_empty());
    }

    #[test]
    fn skin_checks_find_and_fill_bare_vertices_and_bones() {
        let mut s = demo_studio();
        assert!(s.skin_report().is_none(), "no skin checks before Build");
        assert!(s.build(), "{}", s.status);
        assert!(s.skin_report().unwrap().issues.is_empty());
        // Strip a few vertices and the left hand's weights by hand.
        let doc = s.doc.as_mut().unwrap();
        let rig = doc.rig.as_mut().unwrap();
        let hand = rig.skeleton.index_of("hand_l").unwrap();
        let mut bare = vec![3, 4, 5];
        for v in 0..rig.skin.vertex_count() {
            if rig.skin.weight(v, hand) > 0.0 {
                bare.push(v);
            }
        }
        for &v in &bare {
            rig.skin.set(v, Vec::new());
        }
        let report = s.skin_report().unwrap();
        assert_eq!(report.unweighted.len(), {
            let mut b = bare.clone();
            b.sort();
            b.dedup();
            b.len()
        });
        assert_eq!(report.unskinned, vec![hand]);
        s.select_unweighted();
        assert_eq!(s.skin_tools.selected_count(), report.unweighted.len());
        s.fill_unweighted();
        assert!(!s.error, "{}", s.status);
        let after = s.skin_report().unwrap();
        assert!(after.unweighted.is_empty() && after.unskinned == vec![hand]);
        // Auto on the bone gives it skin again; each fix is one undo step.
        s.reauto_named_bone("hand_l");
        assert!(s.skin_report().unwrap().issues.is_empty(), "{}", s.status);
        assert_eq!(s.weights_bone.as_deref(), Some("hand_l"));
        s.undo();
        assert_eq!(s.skin_report().unwrap().unskinned, vec![hand]);
        s.undo();
        assert!(!s.skin_report().unwrap().unweighted.is_empty());
    }

    #[test]
    fn shift_picks_parent_under_the_last_and_unparent_to_the_root() {
        let mut s = demo_studio();
        s.add_tail();
        s.add_eyes();
        let parent_of = |s: &Studio, id: &str| {
            s.doc
                .as_ref()
                .unwrap()
                .guides
                .component(id)
                .unwrap()
                .parent
                .clone()
                .unwrap()
        };
        // Tail and eyes picked, then the head by its top guide: both hang from the
        // head bone nearest that guide.
        s.pick_one(comp(&s, "tail"), None);
        s.toggle_pick(comp(&s, "eyes"), None);
        let head = comp(&s, "head");
        s.toggle_pick(head, Some(2));
        assert_eq!(s.picked.len(), 3);
        s.parent_picked();
        assert!(!s.error, "{}", s.status);
        assert_eq!(parent_of(&s, "tail"), "head");
        assert_eq!(parent_of(&s, "eyes"), "head");
        // The picks survive the edit, and one undo puts both back.
        assert_eq!(s.picked.len(), 3);
        s.undo();
        assert_eq!(parent_of(&s, "tail"), "pelvis");

        // Shift-clicking a pick again drops it; a plain click starts over.
        s.pick_one(comp(&s, "tail"), None);
        s.toggle_pick(comp(&s, "eyes"), None);
        s.toggle_pick(comp(&s, "eyes"), None);
        assert_eq!(s.picked.len(), 1);
        s.pick_one(comp(&s, "arm_l"), None);
        assert_eq!(s.picked.len(), 1);

        // Picks right-clicked onto another part's guide hang from it.
        s.toggle_pick(comp(&s, "tail"), None);
        let leg = comp(&s, "leg_l");
        s.parent_picked_to(leg, Some(0));
        assert!(!s.error, "{}", s.status);
        assert_eq!(parent_of(&s, "arm_l"), "thigh_l");
        assert_eq!(parent_of(&s, "tail"), "thigh_l");
        // Selection follows the parts even though parenting reordered them.
        let (c, _) = s.selected.unwrap();
        assert_eq!(s.doc.as_ref().unwrap().guides.components[c].id, "tail");

        s.unparent_picked();
        assert_eq!(parent_of(&s, "arm_l"), "root");
        assert_eq!(parent_of(&s, "tail"), "root");
        assert!(s.build(), "{}", s.status);

        // A part can't be parented under its own child.
        s.pick_one(comp(&s, "spine"), None);
        s.toggle_pick(comp(&s, "head"), Some(1));
        s.parent_picked();
        assert!(s.error);
        assert_eq!(parent_of(&s, "spine"), "root");
    }

    #[test]
    fn dragging_a_component_moves_it_its_children_and_its_twin() {
        let mut s = demo_studio();
        s.add_fingers();
        let guides = |s: &Studio, id: &str| -> Vec<Vec3> {
            let d = s.doc.as_ref().unwrap();
            d.guides
                .component(id)
                .unwrap()
                .guides
                .iter()
                .map(|g| g.position)
                .collect()
        };
        let (arm, fingers, arm_r, leg) = (
            guides(&s, "arm_l"),
            guides(&s, "index_l"),
            guides(&s, "arm_r"),
            guides(&s, "leg_l"),
        );
        let shift = Vec3::new(0.05, 0.1, 0.0);
        s.move_component(comp(&s, "arm_l"), arm[0] + shift);
        // The whole arm keeps its shape, the fingers on its hand come with it.
        for (a, b) in guides(&s, "arm_l").iter().zip(&arm) {
            assert!((*a - *b - shift).length() < 1e-5);
        }
        for (a, b) in guides(&s, "index_l").iter().zip(&fingers) {
            assert!((*a - *b - shift).length() < 1e-5);
        }
        // The right arm mirrors it; the leg stays put.
        let r = guides(&s, "arm_r");
        assert!(
            (r[0].x - (arm_r[0].x - shift.x)).abs() < 1e-5
                && (r[0].y - arm_r[0].y - shift.y).abs() < 1e-5
        );
        assert_eq!(guides(&s, "leg_l"), leg);
        assert!(s.build(), "{}", s.status);
    }

    #[test]
    fn templates_save_picks_and_load_as_one_undo_step() {
        let mut s = demo_studio();
        s.add_tail();
        s.add_eyes();
        let dir =
            std::env::temp_dir().join(format!("hive_studio_templates_{}", std::process::id()));
        let path = dir.join(format!("bits{}", template::TEMPLATE_SUFFIX));
        // Picked parts only: the tail and the eyes.
        s.pick_one(comp(&s, "tail"), None);
        s.toggle_pick(comp(&s, "eyes"), None);
        s.save_template(&path, true);
        assert!(!s.error, "{}", s.status);
        let t = Template::load(&path).unwrap();
        assert_eq!(t.scope, Scope::Parts);
        assert_eq!(t.components.len(), 2);

        // The rig has eyes already, so the template is refused and nothing changes.
        s.apply_template_file(&path, None);
        assert!(s.error);
        // Without the eyes it loads; the tail id is taken, so it gets a number.
        s.edit("remove", |g| {
            components::remove(g, "eyes")?;
            Ok(String::new())
        });
        s.apply_template_file(&path, None);
        assert!(!s.error, "{}", s.status);
        let g = &s.doc.as_ref().unwrap().guides;
        assert!(g.component("tail2").is_some() && g.component("eyes").is_some());

        // A built-in layout replaces the whole layout; undo brings the parts back.
        let biped = template::built_in()[0].clone();
        assert!(s.apply_template(&biped, None), "{}", s.status);
        assert!(s.doc.as_ref().unwrap().guides.component("tail").is_none());
        s.undo();
        assert!(s.doc.as_ref().unwrap().guides.component("tail2").is_some());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn checks_select_fix_and_still_build() {
        let mut s = demo_studio();
        assert!(s.doc.as_ref().unwrap().checks.is_empty());
        // Bend the left elbow forward and hang the right leg from the head.
        let (arm, leg) = (comp(&s, "arm_l"), comp(&s, "leg_r"));
        let elbow = s.doc.as_ref().unwrap().guides.components[arm].guides[2].position;
        s.symmetry = false;
        s.move_guide(arm, 2, elbow + Vec3::Z * 0.08);
        s.attach_component(leg, "head");
        let checks = &s.doc.as_ref().unwrap().checks;
        assert_eq!(checks.len(), 2, "{checks:#?}");

        // Build still works and says how many warnings there are.
        assert!(s.build());
        assert!(s.warn && !s.error, "{}", s.status);
        assert!(s.status.contains("2 warnings"), "{}", s.status);

        // Clicking a check selects its guide.
        let i = s
            .doc
            .as_ref()
            .unwrap()
            .checks
            .iter()
            .position(|i| i.component == "arm_l");
        s.select_issue(i.unwrap());
        assert_eq!(s.selected, Some((arm, Some(2))));
        assert_eq!(s.mode, Mode::Guides);

        // Fix one (mirroring the other elbow), then the rest; each is one undo step.
        assert!(s.fix_issue(i.unwrap()), "{}", s.status);
        assert_eq!(s.doc.as_ref().unwrap().checks.len(), 1);
        assert!(s.fix_all_issues(), "{}", s.status);
        assert!(s.doc.as_ref().unwrap().checks.is_empty());
        assert_eq!(
            s.doc
                .as_ref()
                .unwrap()
                .guides
                .component("leg_r")
                .unwrap()
                .parent
                .as_deref(),
            Some("pelvis")
        );
        s.undo();
        assert_eq!(s.doc.as_ref().unwrap().checks.len(), 1);
        assert!(s.build());
        assert!(s.status.contains("1 warning "), "{}", s.status);
    }

    #[test]
    fn opening_guides_from_a_newer_hive_rig_warns() {
        let mut s = demo_studio();
        let dir = std::env::temp_dir().join(format!("hive_studio_newer_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("newer.guides.json");
        let mut json: serde_json::Value =
            serde_json::to_value(&s.doc.as_ref().unwrap().guides).unwrap();
        json["hive_rig"] = "99.0.0".into();
        json["a_setting_from_the_future"] = true.into();
        std::fs::write(&path, json.to_string()).unwrap();
        s.open(&path);
        assert!(s.warn && !s.error, "{}", s.status);
        assert!(s.status.contains("99.0.0"), "{}", s.status);
        // Saving stamps this version.
        s.save_guides(&path);
        let back = Guides::load(&path).unwrap();
        assert_eq!(back.written_by.as_deref(), Some(hive_rig::version::CURRENT));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The outliner rows as (id, depth).
    fn outline(s: &Studio) -> Vec<(String, usize)> {
        let doc = s.doc.as_ref().unwrap();
        outliner_rows(&doc.guides, doc.preview.as_ref().ok())
            .into_iter()
            .map(|(i, d)| (doc.guides.components[i].id.clone(), d))
            .collect()
    }

    #[test]
    fn outliner_shows_the_attachment_tree() {
        let mut s = demo_studio();
        let row = |id: &str, d: usize| (id.to_string(), d);
        assert_eq!(
            outline(&s),
            [
                row("root", 0),
                row("spine", 1),
                row("head", 2),
                row("arm_l", 2),
                row("arm_r", 2),
                row("leg_l", 2),
                row("leg_r", 2),
            ]
        );
        // A leg hung from the head is listed under the head, not after the
        // arms (where its indent read as a child of the right arm).
        s.attach_component(comp(&s, "leg_l"), "head");
        assert!(!s.error, "{}", s.status);
        s.add_tail();
        assert_eq!(
            outline(&s),
            [
                row("root", 0),
                row("spine", 1),
                row("head", 2),
                row("leg_l", 3),
                row("arm_l", 2),
                row("arm_r", 2),
                row("leg_r", 2),
                row("tail", 2),
            ]
        );
        // And the head hung from the tail, which was listed after it: the head,
        // with the leg on it, moves under the tail.
        s.attach_component(comp(&s, "head"), "tail_03");
        assert!(!s.error, "{}", s.status);
        assert_eq!(
            outline(&s),
            [
                row("root", 0),
                row("spine", 1),
                row("tail", 2),
                row("head", 3),
                row("leg_l", 4),
                row("arm_l", 2),
                row("arm_r", 2),
                row("leg_r", 2),
            ]
        );
        // Each row names its component's index, wherever the tree put it, so
        // clicking a row picks that component.
        let doc = s.doc.as_ref().unwrap();
        let rows = outliner_rows(&doc.guides, doc.preview.as_ref().ok());
        assert_eq!(rows.len(), doc.guides.components.len());
        assert_eq!(doc.guides.components[rows[4].0].id, "leg_l");
        assert_ne!(rows[4].0, 4);
        // A layout that doesn't build still lists every component.
        let mut broken = doc.guides.clone();
        broken.components[2].parent = Some("no_such_bone".into());
        let built = components::build(&broken);
        assert!(built.is_err());
        let flat = outliner_rows(&broken, built.ok().as_ref());
        assert_eq!(flat.len(), broken.components.len());
        assert!(flat.iter().all(|&(_, d)| d == 0));
    }

    /// Cone and RBF readers made from the test pose: their weights follow the
    /// pose, a helper joint is added to the skeleton and skin, and undo and a
    /// rebuild keep everything in step.
    #[test]
    fn readers_from_the_pose_with_a_helper() {
        let mut s = demo_studio();
        assert!(s.build(), "{}", s.status);
        s.pose = Some(Pose {
            bone: "upperarm_l".into(),
            axis: 2,
            degrees: 60.0,
        });
        assert!(s.new_cone_reader(), "{}", s.status);
        assert_eq!(s.shape_tools.reader.as_deref(), Some("upperarm_l_cone"));
        assert_eq!(s.shape_pick.as_deref(), Some("upperarm_l_cone_fix"));
        let weight = |s: &Studio, name: &str| {
            let rig = s.doc.as_ref().unwrap().rig.clone().unwrap();
            let r = s
                .doc
                .as_ref()
                .unwrap()
                .mesh
                .readers
                .iter()
                .find(|r| r.name == name)
                .unwrap();
            s.reader_outputs_now(r, &rig)
        };
        assert!((weight(&s, "upperarm_l_cone")[0] - 1.0).abs() < 1e-4);
        s.pose.as_mut().unwrap().degrees = 0.0;
        assert!(weight(&s, "upperarm_l_cone")[0] < 1e-4);

        // An RBF reader on the elbow with two poses, each with its shape.
        s.pose = Some(Pose {
            bone: "lowerarm_l".into(),
            axis: 1,
            degrees: 90.0,
        });
        assert!(s.new_rbf_reader(), "{}", s.status);
        s.pose.as_mut().unwrap().degrees = 45.0;
        assert!(s.add_pose_to_reader(), "{}", s.status);
        let w = weight(&s, "lowerarm_l_rbf");
        assert!(w[0] < 1e-3 && (w[1] - 1.0).abs() < 1e-3, "{w:?}");
        s.pose.as_mut().unwrap().degrees = 90.0;
        let w = weight(&s, "lowerarm_l_rbf");
        assert!((w[0] - 1.0).abs() < 1e-3 && w[1] < 1e-3, "{w:?}");
        // The pose a sample already has is refused at rest.
        s.pose.as_mut().unwrap().degrees = 0.0;
        assert!(!s.add_pose_to_reader());

        // A helper joint: a deforming bone of the rig with skin of its own,
        // moved by the reader in the pose.
        s.pose.as_mut().unwrap().degrees = 90.0;
        assert!(s.add_reader_helper(), "{}", s.status);
        let helper = "lowerarm_l_vol";
        let has_helper = |s: &Studio| {
            let rig = s.doc.as_ref().unwrap().rig.as_ref().unwrap();
            rig.skeleton.index_of(helper).is_some_and(|h| {
                (0..rig.skin.joints.len()).any(|v| rig.skin.influences(v).any(|(j, _)| j == h))
            })
        };
        assert!(has_helper(&s));
        let w = weight(&s, "lowerarm_l_rbf");
        assert_eq!(w.len(), 3);
        assert!((w[2] - 1.0).abs() < 1e-3, "{w:?}");
        assert!(s.frame().unwrap().mesh.is_some());
        s.undo_shapes();
        assert!(!has_helper(&s));
        s.undo_shapes();
        assert_eq!(s.doc.as_ref().unwrap().mesh.readers.len(), 2);
        assert!(s.add_reader_helper(), "{}", s.status);
        assert!(s.build(), "{}", s.status);
        assert!(has_helper(&s));
        assert_eq!(s.doc.as_ref().unwrap().mesh.readers.len(), 2);
        assert!(s.remove_reader("lowerarm_l_rbf"), "{}", s.status);
        assert!(!has_helper(&s));
    }

    #[test]
    fn correctives_sculpt_in_the_pose_mirror_and_undo() {
        let mut s = demo_studio();
        assert!(!s.new_shape(), "shapes need a built rig");
        assert!(s.build(), "{}", s.status);
        s.pose = Some(Pose {
            bone: "lowerarm_l".into(),
            axis: 1,
            degrees: 90.0,
        });
        assert!(s.new_corrective(), "{}", s.status);
        assert_eq!(s.shape_pick.as_deref(), Some("lowerarm_l_fix"));
        assert!(s.sculpting);
        let elbow = {
            let rig = s.doc.as_ref().unwrap().rig.as_ref().unwrap();
            rig.skeleton.bones[rig.skeleton.index_of("lowerarm_l").unwrap()].head
        };
        let before = s.frame().unwrap().mesh.unwrap();
        assert!(s.begin_sculpt());
        let up = Vec3::new(0.0, 0.02, 0.0);
        assert!(s.sculpt_at(elbow, up) > 0, "{}", s.status);
        let after = s.frame().unwrap().mesh.unwrap();
        let moved = before
            .iter()
            .zip(&after)
            .filter(|(a, b)| a.distance(**b) > 1e-4)
            .count();
        assert!(moved > 0);
        // In the pose (driver fully on at 90) the vertices moved along the
        // offset, by at most its length.
        for (a, b) in before.iter().zip(&after) {
            let d = *b - *a;
            if d.length() > 1e-4 {
                assert!(d.normalize().dot(Vec3::Y) > 0.99, "{d}");
                assert!(d.length() < up.length() + 1e-4, "{d}");
            }
        }
        // Unposed, the corrective is off and the mesh is at rest.
        s.pose = None;
        assert!(s.frame().unwrap().mesh.is_none());
        assert!(s.mirror_shape("lowerarm_l_fix"), "{}", s.status);
        let names: Vec<&str> = s.shapes().iter().map(|x| x.name.as_str()).collect();
        assert_eq!(names, ["lowerarm_l_fix", "lowerarm_r_fix"]);
        assert_eq!(s.shapes()[1].driver.as_ref().unwrap().bone, "lowerarm_r");
        // A weight on a free shape shows at rest.
        assert!(s.new_shape());
        assert!(s.begin_sculpt());
        assert!(s.sculpt_at(elbow, up) > 0);
        assert!(s.frame().unwrap().mesh.is_some());
        s.undo_shapes();
        s.undo_shapes();
        assert_eq!(s.shapes().len(), 2);
        // The built rig carries the shapes, and a rebuild keeps them.
        assert_eq!(
            s.doc
                .as_ref()
                .unwrap()
                .rig
                .as_ref()
                .unwrap()
                .mesh
                .shapes
                .len(),
            2
        );
        assert!(s.build(), "{}", s.status);
        assert_eq!(
            s.doc
                .as_ref()
                .unwrap()
                .rig
                .as_ref()
                .unwrap()
                .mesh
                .shapes
                .len(),
            2
        );
    }

    /// The vertex of the mesh as shown nearest to `p`.
    fn nearest_shown(s: &Studio, p: Vec3) -> Vec3 {
        let shown = s.shown_positions().unwrap();
        *shown
            .iter()
            .min_by(|a, b| a.distance(p).total_cmp(&b.distance(p)))
            .unwrap()
    }

    #[test]
    fn inbetweens_and_sculpt_brushes_work_on_the_picked_target() {
        let mut s = demo_studio();
        assert!(s.build(), "{}", s.status);
        s.pose = Some(Pose {
            bone: "lowerarm_l".into(),
            axis: 1,
            degrees: 90.0,
        });
        assert!(s.new_corrective(), "{}", s.status);
        let elbow = {
            let rig = s.doc.as_ref().unwrap().rig.as_ref().unwrap();
            rig.skeleton.bones[rig.skeleton.index_of("lowerarm_l").unwrap()].head
        };
        let up = Vec3::new(0.0, 0.02, 0.0);
        assert!(s.begin_sculpt());
        assert!(s.sculpt_at(elbow, up) > 0, "{}", s.status);
        // Half way through the bend the shape is at 0.5: an in-between goes there.
        s.pose.as_mut().unwrap().degrees = 45.0;
        let shape = s.shapes()[0].clone();
        assert!((s.shape_weight_now(&shape) - 0.5).abs() < 0.01);
        assert!(s.add_inbetween_here(), "{}", s.status);
        assert_eq!(s.shape_tools.target, Some(0.5));
        assert_eq!(s.mesh_tool(), MeshTool::Sculpt);
        let ib = s.shapes()[0].inbetweens[0].clone();
        assert!((ib.weight - 0.5).abs() < 1e-6);
        // It starts where the shape was at 0.5: half the full offsets.
        let full = &s.shapes()[0];
        for (v, o) in ib.vertices.iter().zip(&ib.offsets) {
            let k = full.vertices.iter().position(|x| x == v).unwrap();
            assert!(o.distance(full.offsets[k] * 0.5) < 1e-5);
        }
        // Sculpting now moves the in-between, not the full shape.
        let full_before = s.shapes()[0].offsets.clone();
        assert!(s.begin_sculpt());
        assert!(s.sculpt_at(elbow, Vec3::new(0.0, 0.0, 0.01)) > 0);
        assert_eq!(s.shapes()[0].offsets, full_before);
        assert_ne!(s.shapes()[0].inbetweens[0].offsets, ib.offsets);
        // Pose there puts the bone where the in-between is fully on.
        s.pose.as_mut().unwrap().degrees = 10.0;
        assert!(s.pose_to_target());
        assert!((s.pose.as_ref().unwrap().degrees - 45.0).abs() < 1e-3);

        // The dab brushes, each on the in-between at its pose.
        let at = nearest_shown(&s, elbow);
        let dense = |s: &Studio| {
            let n = s.doc.as_ref().unwrap().mesh.positions.len();
            s.shapes()[0].inbetweens[0].dense(n)
        };
        for brush in [
            SculptBrush::Inflate,
            SculptBrush::Smooth,
            SculptBrush::Relax,
        ] {
            s.shape_tools.brush = brush;
            let before = dense(&s);
            assert!(s.begin_sculpt());
            assert!(s.sculpt_dab(at, false) > 0, "{brush:?}: {}", s.status);
            assert_ne!(dense(&s), before, "{brush:?}");
        }
        s.shape_tools.brush = SculptBrush::Erase;
        s.shape_tools.strength = 1.0;
        assert!(s.sculpt_dab(at, false) > 0, "{}", s.status);
        // The undo steps come back off one by one, and the in-between goes.
        assert!(s.remove_inbetween(0.5), "{}", s.status);
        assert!(s.shapes()[0].inbetweens.is_empty());
        assert_eq!(s.shape_tools.target, None);
        s.undo_shapes();
        assert_eq!(s.shapes()[0].inbetweens.len(), 1);

        // A free shape's slider follows the target being sculpted.
        assert!(s.new_shape());
        assert!(s.sculpt_at(elbow, up) > 0);
        assert!(s.add_inbetween_here(), "{}", s.status);
        let free = s.shapes().last().unwrap();
        assert_eq!(free.inbetweens.len(), 1);
        assert!((free.weight - 0.5).abs() < 1e-6);
        s.pick_target(None);
        assert_eq!(s.shapes().last().unwrap().weight, 1.0);
    }

    #[test]
    fn mesh_tools_are_one_at_a_time() {
        let mut s = demo_studio();
        s.set_mesh_tool(MeshTool::Paint);
        assert_eq!(
            s.mesh_tool(),
            MeshTool::View,
            "nothing to paint before a build"
        );
        assert!(s.build(), "{}", s.status);
        s.weights_bone = Some("upperarm_l".into());
        s.set_mesh_tool(MeshTool::Paint);
        assert_eq!(s.mesh_tool(), MeshTool::Paint);
        s.set_mesh_tool(MeshTool::Select);
        assert!(!s.painting && s.skin_tools.selecting);
        s.set_mesh_tool(MeshTool::Sculpt);
        assert_eq!(s.mesh_tool(), MeshTool::Select, "no shape to sculpt yet");
        assert!(s.warn);
        assert!(s.new_shape());
        assert_eq!(s.mesh_tool(), MeshTool::Sculpt);
        assert!(!s.skin_tools.selecting);
        s.set_mesh_tool(MeshTool::Paint);
        assert!(s.painting && !s.sculpting);
        s.set_mesh_tool(MeshTool::View);
        assert!(!s.painting && !s.sculpting && !s.skin_tools.selecting);
    }

    #[test]
    fn weights_copy_from_a_saved_rig_and_show_all_bones() {
        let mut s = demo_studio();
        assert!(s.build(), "{}", s.status);
        let dir = std::env::temp_dir().join(format!("hive_studio_copy_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("demo.hive.json");
        let saved = s.doc.as_ref().unwrap().rig.clone().unwrap();
        std::fs::write(&path, serde_json::to_string(&saved).unwrap()).unwrap();
        // Spoil the left side, then take the saved weights back.
        s.mirror_weights(Side::Right);
        assert!(s.copy_weights_from(&path), "{}", s.status);
        let rig = s.doc.as_ref().unwrap().rig.as_ref().unwrap();
        let n = rig.skin.vertex_count();
        let bone = rig.skeleton.index_of("upperarm_l").unwrap();
        let off = (0..n)
            .filter(|&v| (rig.skin.weight(v, bone) - saved.skin.weight(v, bone)).abs() > 0.05)
            .count();
        assert!(off * 50 < n, "{off} of {n} vertices differ");
        assert!(!s.copy_weights_from(&dir.join("missing.hive.json")));
        std::fs::remove_dir_all(&dir).ok();
        // Every bone in its own colour: a vertex on one bone takes its colour.
        let rig = s.doc.as_ref().unwrap().rig.clone().unwrap();
        let colors = s.weights_for(ALL_BONES).unwrap().to_vec();
        assert_eq!(colors.len(), n);
        let solo = (0..n)
            .find(|&v| rig.skin.influences(v).count() == 1)
            .unwrap();
        let (j, _) = rig.skin.influences(solo).next().unwrap();
        let (a, b) = (colors[solo], view::bone_color(j));
        assert!((i32::from(a.r()) - i32::from(b.r())).abs() <= 1);
        assert!((i32::from(a.g()) - i32::from(b.g())).abs() <= 1);
    }

    #[test]
    fn creatures_open_build_and_are_templates() {
        let listed: Vec<String> = template::built_in()
            .iter()
            .map(|t| t.name.clone())
            .collect();
        for c in demo::Creature::ALL {
            let mut s = Studio::default();
            s.load_creature(c);
            assert!(!s.error, "{c:?}: {}", s.status);
            let doc = s.doc.as_ref().unwrap();
            assert_eq!(doc.name, c.rig_name());
            assert!(doc.checks.is_empty(), "{c:?}: {:?}", doc.checks);
            assert!(s.build(), "{c:?}: {}", s.status);
            assert!(listed.contains(&c.name().to_string()), "{c:?}");
            // Its template loads onto the humanoid demo.
            let t = template::built_in()
                .iter()
                .find(|t| t.name == c.name())
                .unwrap();
            let mut s = demo_studio();
            assert!(s.apply_template(t, None), "{c:?}: {}", s.status);
        }
    }

    fn temp_dir(what: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hive_studio_{what}_{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Drawn, imported and library shapes go on controls (twins too), into
    /// the user's library file and the built rig; line widths too; undo works.
    #[test]
    fn custom_shapes_are_drawn_kept_and_used() {
        let dir = temp_dir("custom_shapes");
        let lib = dir.join("shapes.json");
        let mut s = demo_studio();
        s.shape_lib_path = Some(lib.clone());
        s.symmetry = true;
        // Draw half a crown with mirror on.
        let mut draft = ShapeDraft::new(Some("hand_l_ik_ctrl".into()));
        for p in [[0.0, -0.5], [0.7, -0.5], [0.9, 0.5], [0.5, 0.1], [0.0, 0.6]] {
            draft.add_point(p);
        }
        draft.add_point([0.0, 0.6]); // the same point twice is one point
        assert_eq!(draft.lines[0].points.len(), 5);
        draft.undo_point();
        draft.add_point([0.01, 0.61]); // snaps onto the grid
        assert_eq!(draft.lines[0].points[4], [0.0, 0.625]);
        let shape = draft.shape();
        assert!(
            s.keep_custom_shape("crown", shape.clone(), Some("hand_l_ik_ctrl")),
            "{}",
            s.status
        );
        // In the library file, the guides, and on both hands.
        let saved = custom_shapes::ShapeLibrary::load(&lib).unwrap();
        assert!(saved.shapes.contains_key("crown"));
        let g = &s.doc.as_ref().unwrap().guides;
        assert!(g.custom_shapes.contains_key("crown"));
        for c in ["hand_l_ik_ctrl", "hand_r_ik_ctrl"] {
            assert_eq!(g.control_styles[c].custom.as_deref(), Some("crown"), "{c}");
        }
        // A bad name or an empty shape is refused.
        assert!(!s.keep_custom_shape("no good!", shape.clone(), None));
        assert!(!s.keep_custom_shape("empty", ShapeDraft::new(None).shape(), None));
        // A fresh character offers the library's shape and uses it.
        let mut t = demo_studio();
        t.shape_lib_path = Some(lib.clone());
        let choices = t.custom_shape_choices();
        assert!(
            choices
                .iter()
                .any(|(n, _, in_guides)| n == "crown" && !in_guides)
        );
        assert!(t.use_custom_shape("head_ctrl", "crown"), "{}", t.status);
        assert!(
            t.doc
                .as_ref()
                .unwrap()
                .guides
                .custom_shapes
                .contains_key("crown")
        );
        assert!(!t.use_custom_shape("head_ctrl", "missing"));
        t.undo();
        assert!(
            !t.doc
                .as_ref()
                .unwrap()
                .guides
                .custom_shapes
                .contains_key("crown")
        );
        // Import from SVG onto the head, then thicker lines.
        let svg = dir.join("badge.svg");
        std::fs::write(
            &svg,
            r#"<svg xmlns="http://www.w3.org/2000/svg"><polygon points="0,0 10,0 10,10 0,10"/></svg>"#,
        )
        .unwrap();
        assert!(
            s.import_custom_shape(&svg, Some("head_ctrl")),
            "{}",
            s.status
        );
        s.set_line_width("head_ctrl", 3.0);
        assert!(s.build(), "{}", s.status);
        let rig = s.doc.as_ref().unwrap().rig.as_ref().unwrap();
        let head = rig.controls.control("head_ctrl").unwrap();
        assert_eq!(head.custom.as_ref().map(|c| c.name.as_str()), Some("badge"));
        assert_eq!(head.line_width, Some(3.0));
        // Programs that only draw built-in shapes get the nearest: a square's.
        use hive_rig::controls::Shape;
        assert!(
            matches!(head.shape, Shape::Square | Shape::Cube),
            "{:?}",
            head.shape
        );
        let hand = rig.controls.control("hand_r_ik_ctrl").unwrap();
        assert_eq!(hand.custom.as_ref().map(|c| c.name.as_str()), Some("crown"));
        assert!(
            custom_shapes::ShapeLibrary::load(&lib)
                .unwrap()
                .shapes
                .contains_key("badge")
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Two characters side by side: switch between them, hold one's prop in
    /// the other's hand, build both, save and open the scene, export it.
    #[test]
    fn several_characters_in_one_scene() {
        let dir = temp_dir("scene");
        let mut s = demo_studio();
        s.shape_lib_path = None;
        s.add_prop();
        assert!(!s.error, "{}", s.status);
        assert!(s.add_demo_character(None), "{}", s.status);
        assert_eq!(s.character_count(), 2);
        let names = s.character_names();
        assert_eq!(names.len(), 2);
        assert_ne!(names[0], names[1], "each character has its own name");
        // The new one stands beside the first, not inside it.
        let other = s.doc.as_ref().unwrap();
        assert!(other.offset.x > 0.3, "{}", other.offset);
        assert_eq!(s.cast.active, 1);
        // Back to the first: its edits are still there; the second's hand holds its prop.
        assert!(s.switch_character(0));
        let prop = s
            .doc
            .as_ref()
            .unwrap()
            .preview
            .as_ref()
            .unwrap()
            .controls
            .controls
            .iter()
            .map(|c| c.name.clone())
            .find(|n| n.starts_with("prop"))
            .expect("the prop's control");
        assert!(
            !s.hold_prop(&prop, &names[0], "hand_l", 1.0),
            "not by itself"
        );
        assert!(
            !s.hold_prop(&prop, &names[1], "wing_l", 1.0),
            "not by a missing bone"
        );
        assert!(s.hold_prop(&prop, &names[1], "hand_l", 1.0), "{}", s.status);
        assert_eq!(s.hold_of(&prop).unwrap().holder.character, names[1]);
        assert!(s.build_all(), "{}", s.status);
        assert_eq!(s.cast.active, 0);
        assert!(s.character(1).unwrap().rig.is_some());
        // Save, open again: both characters, their rigs and the hold come back.
        let path = dir.join("pair.scene.json");
        assert!(s.save_scene(&path), "{}", s.status);
        assert!(
            dir.join("pair_rigs")
                .join(format!("{}.hive.json", names[1]))
                .is_file()
        );
        let mut t = Studio {
            shape_lib_path: None,
            ..Default::default()
        };
        t.open(&path);
        assert!(!t.error, "{}", t.status);
        assert_eq!(t.character_names(), names);
        assert_eq!(t.cast.holds.len(), 1);
        assert!(
            t.doc.as_ref().unwrap().rig.is_some(),
            "the saved rig came back"
        );
        assert!((t.character(1).unwrap().offset - s.character(1).unwrap().offset).length() < 1e-5);
        // Export every character and the scene builders.
        let out = dir.join("out");
        let files = t.export_scene(&out).unwrap();
        for n in &names {
            assert!(out.join(format!("{n}_blender.py")).is_file(), "{n}");
        }
        let blender = out.join("pair_scene_blender.py");
        assert!(files.contains(&blender), "{files:?}");
        let text = std::fs::read_to_string(&blender).unwrap();
        assert!(text.contains(&prop) && text.contains("hand_l"));
        // Taking the holder out drops the hold.
        assert!(t.remove_character(1));
        assert_eq!(t.character_count(), 1);
        assert!(t.cast.holds.is_empty());
        assert!(!t.remove_character(0), "a scene keeps one character");
        std::fs::remove_dir_all(&dir).ok();
    }
}
