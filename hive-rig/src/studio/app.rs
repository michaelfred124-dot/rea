//! The Hive Rig window: load a character, place and edit component guides in a
//! 3D view, build the rig, preview bones, controls and weights, and export.
//!
//! The editing operations are methods on `Studio` that don't touch the UI, so
//! they are tested directly; `ui` only lays out widgets that call them.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Color32, Key, PointerButton, Pos2, RichText, Sense, Stroke};
use glam::Vec3;

use hive_rig::components::{self, AddOptions, Built, Guides, Kind, Preset};
use hive_rig::rig::detect_guides;
use hive_rig::skeleton::Side;
use hive_rig::{Rig, RigOptions, Target, build_rig, demo, export, import, mesh::Mesh};

use crate::view::{self, Camera, Projector};

const PICK_RADIUS: f32 = 9.0;
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
    undo: Vec<Guides>,
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
            undo: Vec::new(),
        };
        doc.refresh();
        doc
    }

    fn refresh(&mut self) {
        self.preview = components::build(&self.guides).map_err(|e| format!("{e:#}"));
        self.rig = None;
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
    pub show_mesh: bool,
    pub show_guides: bool,
    pub show_bones: bool,
    pub show_controls: bool,
    pub mesh_opacity: f32,
    /// Deform bone whose weights colour the mesh (needs a built rig).
    pub weights_bone: Option<String>,
    pub targets: [bool; 3],
    pub status: String,
    pub error: bool,
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
            show_mesh: true,
            show_guides: true,
            show_bones: true,
            show_controls: true,
            mesh_opacity: 1.0,
            weights_bone: None,
            targets: [true; 3],
            status:
                "Open a character to start: File > Open Character, or drop a file on the window."
                    .into(),
            error: false,
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
    }

    fn fail(&mut self, msg: impl Into<String>) {
        self.status = msg.into();
        self.error = true;
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
                doc.guides = g;
                doc.refresh();
                self.selected = None;
                self.weight_colors = None;
                self.info("Undone.");
            }
            None => self.info("Nothing to undo."),
        }
    }

    /// Move one guide (and its mirror twin when symmetry is on) without an undo
    /// step; dragging records one step when it starts.
    pub fn move_guide(&mut self, comp: usize, guide: usize, to: Vec3) {
        let symmetry = self.symmetry;
        let Some(doc) = &mut self.doc else { return };
        let Some(c) = doc.guides.components.get(comp) else {
            return;
        };
        let twin = if symmetry {
            components::twin_id(c)
                .and_then(|t| doc.guides.components.iter().position(|o| o.id == t))
        } else {
            None
        };
        doc.guides.components[comp].guides[guide].position = to;
        let mirrored = components::mirror_point(&doc.guides, to);
        if let Some(t) = twin
            && let Some(g) = doc.guides.components[t].guides.get_mut(guide)
        {
            g.position = mirrored;
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

fn push_undo(undo: &mut Vec<Guides>, g: &Guides) {
    undo.push(g.clone());
    if undo.len() > UNDO_LIMIT {
        undo.remove(0);
    }
}

fn kind_label(k: Kind) -> &'static str {
    match k {
        Kind::Root => "root",
        Kind::Spine => "spine",
        Kind::Head => "head",
        Kind::Arm => "arm",
        Kind::Leg => "leg",
        Kind::Chain => "chain",
    }
}

fn side_color(side: Side) -> Color32 {
    match side {
        Side::Center => Color32::from_rgb(250, 210, 40),
        Side::Left => Color32::from_rgb(60, 140, 255),
        Side::Right => Color32::from_rgb(255, 70, 70),
    }
}

fn rgb(c: [f32; 3]) -> Color32 {
    Color32::from_rgb(
        (c[0] * 255.0) as u8,
        (c[1] * 255.0) as u8,
        (c[2] * 255.0) as u8,
    )
}

fn open_folder(dir: &Path) {
    let program = if cfg!(windows) {
        "explorer"
    } else if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(program).arg(dir).spawn();
}

fn character_dialog() -> rfd::FileDialog {
    rfd::FileDialog::new()
        .set_title("Open a character")
        .add_filter("Characters and rigs", &["glb", "gltf", "obj", "json"])
}

// --------------------------------------------------------------------------- UI

impl Studio {
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .collect()
        });
        if let Some(path) = dropped.first() {
            self.open(path);
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, Key::Z)) {
            self.undo();
        }

        egui::Panel::top("menu").show(ui, |ui| self.top_bar(ui));
        egui::Panel::bottom("status").show(ui, |ui| {
            let color = if self.error {
                ui.visuals().warn_fg_color
            } else {
                ui.visuals().text_color()
            };
            ui.label(RichText::new(&self.status).color(color));
        });
        if self.doc.is_some() {
            egui::Panel::left("components")
                .resizable(true)
                .default_size(270.0)
                .min_size(220.0)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| self.components_panel(ui));
                });
            egui::Panel::right("rig")
                .resizable(true)
                .default_size(250.0)
                .min_size(210.0)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| self.rig_panel(ui));
                });
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(Color32::from_rgb(38, 40, 46)))
            .show(ui, |ui| {
                if self.doc.is_some() {
                    self.viewport(ui);
                } else {
                    self.welcome(ui);
                }
            });
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.menu_button("File", |ui| {
                if ui.button("Open Character…").clicked() {
                    ui.close();
                    if let Some(p) = character_dialog().pick_file() {
                        self.open(&p);
                    }
                }
                if ui.button("Demo Character").clicked() {
                    ui.close();
                    self.load_demo();
                }
                ui.separator();
                let has_doc = self.doc.is_some();
                if ui
                    .add_enabled(has_doc, egui::Button::new("Load Guides…"))
                    .clicked()
                {
                    ui.close();
                    if let Some(p) = rfd::FileDialog::new()
                        .add_filter("Hive guides", &["json"])
                        .pick_file()
                    {
                        self.open(&p);
                    }
                }
                if ui
                    .add_enabled(has_doc, egui::Button::new("Save Guides…"))
                    .clicked()
                {
                    ui.close();
                    let name = self
                        .doc
                        .as_ref()
                        .map(|d| d.name.clone())
                        .unwrap_or_default();
                    if let Some(p) = rfd::FileDialog::new()
                        .set_file_name(format!("{name}.guides.json"))
                        .add_filter("Hive guides", &["json"])
                        .save_file()
                    {
                        self.save_guides(&p);
                    }
                }
                ui.separator();
                if ui.button("Quit").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Edit", |ui| {
                if ui.button("Undo   Ctrl+Z").clicked() {
                    ui.close();
                    self.undo();
                }
                if ui
                    .add_enabled(
                        self.doc.is_some(),
                        egui::Button::new("Reset Guides to Body"),
                    )
                    .clicked()
                {
                    ui.close();
                    self.redetect();
                }
            });
            ui.menu_button("View", |ui| {
                if ui.button("Frame Character   F").clicked() {
                    ui.close();
                    self.frame();
                }
                ui.checkbox(&mut self.show_mesh, "Mesh");
                ui.checkbox(&mut self.show_guides, "Guides");
                ui.checkbox(&mut self.show_bones, "Bones");
                ui.checkbox(&mut self.show_controls, "Controls");
            });
            ui.separator();
            ui.label(
                RichText::new("HIVE RIG")
                    .strong()
                    .color(Color32::from_rgb(250, 190, 40)),
            );
            if let Some(doc) = &self.doc {
                ui.label(format!("· {}", doc.name));
            }
        });
    }

    fn frame(&mut self) {
        if let Some(doc) = &self.doc {
            let (min, max) = doc.mesh.bounds();
            self.camera.frame(min, max);
        }
    }

    fn welcome(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() * 0.3).max(20.0));
            ui.label(
                RichText::new("HIVE RIG")
                    .size(40.0)
                    .strong()
                    .color(Color32::from_rgb(250, 190, 40)),
            );
            ui.label(
                RichText::new("Automatic character rigging for Blender, Maya and Unreal")
                    .size(16.0),
            );
            ui.add_space(24.0);
            if ui
                .add(
                    egui::Button::new(RichText::new("Open Character…").size(18.0))
                        .min_size(egui::vec2(240.0, 40.0)),
                )
                .clicked()
                && let Some(p) = character_dialog().pick_file()
            {
                self.open(&p);
            }
            ui.add_space(8.0);
            if ui
                .add(
                    egui::Button::new(RichText::new("Try the Demo Character").size(18.0))
                        .min_size(egui::vec2(240.0, 40.0)),
                )
                .clicked()
            {
                self.load_demo();
            }
            ui.add_space(16.0);
            ui.label("or drop a .glb, .gltf or .obj file on this window");
            ui.add_space(6.0);
            ui.label(
                RichText::new("Characters should stand in a T- or A-pose, Y-up, facing forward.")
                    .weak(),
            );
        });
    }

    fn components_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Components");
        let Some(doc) = &self.doc else { return };
        let rows: Vec<(String, Kind, Side, Option<String>)> = doc
            .guides
            .components
            .iter()
            .map(|c| (c.id.clone(), c.kind, c.side, c.parent.clone()))
            .collect();
        for (i, (id, kind, side, parent)) in rows.iter().enumerate() {
            let selected = self.selected.is_some_and(|(c, _)| c == i);
            let text =
                RichText::new(format!("{id}  ·  {}", kind_label(*kind))).color(side_color(*side));
            let r = ui.selectable_label(selected, text);
            let r = match parent {
                Some(p) => r.on_hover_text(format!("attached to {p}")),
                None => r,
            };
            if r.clicked() {
                self.selected = Some((i, None));
            }
        }
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.symmetry, "Symmetry")
                .on_hover_text("Moving a left or right guide moves its twin too");
        });
        ui.horizontal(|ui| {
            if ui.button("Mirror Left to Right").clicked() {
                self.mirror(Side::Left);
            }
            if ui.button("Mirror Right to Left").clicked() {
                self.mirror(Side::Right);
            }
        });

        if let Some((c, guide)) = self.selected {
            ui.separator();
            self.component_details(ui, c, guide);
        }

        ui.separator();
        ui.heading("Add Component");
        ui.horizontal(|ui| {
            if ui.button("Fingers").clicked() {
                self.add_fingers();
            }
            egui::ComboBox::from_id_salt("finger_sides")
                .selected_text(match self.finger_sides {
                    FingerSides::Both => "both hands",
                    FingerSides::Left => "left hand",
                    FingerSides::Right => "right hand",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.finger_sides, FingerSides::Both, "both hands");
                    ui.selectable_value(&mut self.finger_sides, FingerSides::Left, "left hand");
                    ui.selectable_value(&mut self.finger_sides, FingerSides::Right, "right hand");
                });
        });
        ui.horizontal(|ui| {
            if ui.button("Tail").clicked() {
                self.add_tail();
            }
            ui.add(
                egui::DragValue::new(&mut self.tail_joints)
                    .range(2..=24)
                    .suffix(" bones"),
            );
        });
        ui.add_space(4.0);
        ui.label("Chain (ears, hair, props…)");
        let bones = self.doc.as_ref().map(Doc::bone_names).unwrap_or_default();
        if !bones.contains(&self.chain_parent) {
            self.chain_parent = bones
                .iter()
                .find(|b| b.as_str() == "head")
                .or(bones.first())
                .cloned()
                .unwrap_or_default();
        }
        ui.horizontal(|ui| {
            ui.label("on");
            egui::ComboBox::from_id_salt("chain_parent")
                .selected_text(self.chain_parent.as_str())
                .height(320.0)
                .show_ui(ui, |ui| {
                    for b in &bones {
                        ui.selectable_value(&mut self.chain_parent, b.clone(), b);
                    }
                });
            ui.add(
                egui::DragValue::new(&mut self.chain_joints)
                    .range(1..=24)
                    .suffix(" bones"),
            );
        });
        ui.horizontal(|ui| {
            ui.label("name");
            ui.add(
                egui::TextEdit::singleline(&mut self.chain_id)
                    .hint_text("auto")
                    .desired_width(110.0),
            );
            if ui.button("Add Chain").clicked() {
                self.add_chain();
            }
        });
    }

    fn component_details(&mut self, ui: &mut egui::Ui, c: usize, guide: Option<usize>) {
        let Some(doc) = &self.doc else { return };
        let Some(comp) = doc.guides.components.get(c).cloned() else {
            return;
        };
        let own: Vec<String> = match &doc.preview {
            Ok(b) => b
                .skeleton
                .bones
                .iter()
                .filter(|b| b.component == comp.id)
                .map(|b| b.name.clone())
                .collect(),
            Err(_) => Vec::new(),
        };
        let parents: Vec<String> = doc
            .bone_names()
            .into_iter()
            .filter(|b| !own.contains(b))
            .collect();

        ui.heading(&comp.id);
        ui.label(format!(
            "{} component, {} bones",
            kind_label(comp.kind),
            own.len()
        ));

        if let Some(parent) = &comp.parent {
            let mut chosen = parent.clone();
            ui.horizontal(|ui| {
                ui.label("Attached to");
                egui::ComboBox::from_id_salt("parent")
                    .selected_text(chosen.as_str())
                    .height(320.0)
                    .show_ui(ui, |ui| {
                        for b in &parents {
                            ui.selectable_value(&mut chosen, b.clone(), b);
                        }
                    });
            });
            if &chosen != parent {
                let id = comp.id.clone();
                self.edit("re-attach it", |g| {
                    let target = g.components.iter_mut().find(|x| x.id == id).unwrap();
                    target.parent = Some(chosen.clone());
                    Ok(format!("{id} now hangs from {chosen}."))
                });
            }
        }
        match comp.kind {
            Kind::Spine => {
                let mut joints = comp.options.joints.unwrap_or(components::SPINE_JOINTS);
                let before = joints;
                ui.horizontal(|ui| {
                    ui.label("Spine joints");
                    ui.add(egui::DragValue::new(&mut joints).range(1..=16));
                });
                if joints != before {
                    let id = comp.id.clone();
                    self.edit("change the spine", |g| {
                        components::set_spine_joints(g, &id, joints)?;
                        Ok(format!("Spine now has {joints} joints."))
                    });
                }
            }
            Kind::Arm | Kind::Leg => {
                let mut twist = comp.options.twist;
                if ui.checkbox(&mut twist, "Twist bones").changed() {
                    let id = comp.id.clone();
                    self.edit("change twist bones", |g| {
                        g.components
                            .iter_mut()
                            .find(|x| x.id == id)
                            .unwrap()
                            .options
                            .twist = twist;
                        Ok(format!(
                            "Twist bones {} on {id}.",
                            if twist { "on" } else { "off" }
                        ))
                    });
                }
            }
            _ => {}
        }

        ui.add_space(4.0);
        ui.label(RichText::new("Guides").strong());
        egui::Grid::new("guides")
            .num_columns(4)
            .spacing([4.0, 2.0])
            .show(ui, |ui| {
                for (gi, g) in comp.guides.iter().enumerate() {
                    let is_sel = guide == Some(gi);
                    if ui.selectable_label(is_sel, &g.name).clicked() {
                        self.selected = Some((c, Some(gi)));
                    }
                    let mut p = g.position;
                    let mut changed = false;
                    for v in [&mut p.x, &mut p.y, &mut p.z] {
                        changed |= ui
                            .add(egui::DragValue::new(v).speed(0.002).max_decimals(3))
                            .changed();
                    }
                    if changed {
                        self.selected = Some((c, Some(gi)));
                        self.begin_change();
                        self.move_guide(c, gi, p);
                    }
                    ui.end_row();
                }
            });
        ui.add_space(4.0);
        if comp.kind != Kind::Root && ui.button("Remove Component").clicked() {
            self.remove_selected();
        }
    }

    /// Record an undo step for a guide move unless one is already in progress.
    fn begin_change(&mut self) {
        if self.drag.is_none()
            && let Some(doc) = &mut self.doc
            && doc.undo.last().is_none_or(|u| !same_layout(u, &doc.guides))
        {
            push_undo(&mut doc.undo, &doc.guides);
        }
    }

    fn rig_panel(&mut self, ui: &mut egui::Ui) {
        ui.heading("Rig");
        let build = egui::Button::new(RichText::new("Build Rig").size(18.0).strong())
            .min_size(egui::vec2(ui.available_width(), 36.0));
        if ui.add(build).clicked() {
            self.build();
        }
        let Some(doc) = &self.doc else { return };
        match (&doc.preview, &doc.rig) {
            (Err(e), _) => {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    format!("Guides don't build: {e}"),
                );
            }
            (Ok(b), rig) => {
                ui.label(format!(
                    "{} bones · {} controls · {} IK limbs",
                    b.skeleton.bones.len(),
                    b.controls.controls.len(),
                    b.controls.ik_chains.len()
                ));
                if rig.is_some() {
                    ui.label(
                        RichText::new("Built, with skin weights")
                            .color(Color32::from_rgb(110, 210, 120)),
                    );
                } else {
                    ui.label(RichText::new("Press Build Rig to compute the skin weights.").weak());
                }
            }
        }

        ui.separator();
        ui.heading("View");
        ui.checkbox(&mut self.show_mesh, "Mesh");
        if self.show_mesh {
            ui.add(egui::Slider::new(&mut self.mesh_opacity, 0.15..=1.0).text("opacity"));
        }
        ui.checkbox(&mut self.show_guides, "Guides");
        ui.checkbox(&mut self.show_bones, "Bones");
        ui.checkbox(&mut self.show_controls, "Controls");
        ui.add_space(4.0);
        let deform: Vec<String> = doc
            .rig
            .as_ref()
            .map(|r| {
                r.skeleton
                    .bones
                    .iter()
                    .filter(|b| b.deform)
                    .map(|b| b.name.clone())
                    .collect()
            })
            .unwrap_or_default();
        ui.add_enabled_ui(!deform.is_empty(), |ui| {
            ui.horizontal(|ui| {
                ui.label("Weights");
                egui::ComboBox::from_id_salt("weights")
                    .selected_text(self.weights_bone.as_deref().unwrap_or("off"))
                    .height(360.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.weights_bone, None, "off");
                        for b in &deform {
                            ui.selectable_value(&mut self.weights_bone, Some(b.clone()), b);
                        }
                    });
            });
        });
        if deform.is_empty() {
            ui.label(RichText::new("Build the rig to see its weights.").weak());
        }

        ui.separator();
        ui.heading("Export");
        let Some(doc) = &mut self.doc else { return };
        ui.horizontal(|ui| {
            ui.label("Name");
            let mut name = doc.name.clone();
            if ui.text_edit_singleline(&mut name).changed() {
                doc.name = export::sanitize_name(&name);
            }
        });
        ui.horizontal(|ui| {
            ui.checkbox(&mut self.targets[0], "Blender");
            ui.checkbox(&mut self.targets[1], "Maya");
            ui.checkbox(&mut self.targets[2], "Unreal");
        });
        ui.label(
            RichText::new(doc.out_dir.display().to_string())
                .small()
                .weak(),
        );
        let out_dir = doc.out_dir.clone();
        ui.horizontal(|ui| {
            if ui.button("Export").clicked() {
                self.do_export(out_dir.clone());
            }
            if ui.button("Export To…").clicked()
                && let Some(dir) = rfd::FileDialog::new()
                    .set_directory(out_dir.parent().unwrap_or(&out_dir))
                    .pick_folder()
            {
                self.do_export(dir);
            }
            if ui
                .add_enabled(out_dir.is_dir(), egui::Button::new("Open Folder"))
                .clicked()
            {
                open_folder(&out_dir);
            }
        });
        ui.add_space(6.0);
        egui::CollapsingHeader::new("Using the export").show(ui, |ui| {
            ui.label("Blender: Scripting tab > Open the _blender.py > Run. Edit and rebuild later from the Hive tab in the N panel.");
            ui.label("Maya: Script Editor > Python > open the _maya.py > Run. The Hive Rig window rebuilds it.");
            ui.label("Unreal: Tools > Execute Python Script > the _unreal.py (needs the Python and Control Rig plugins).");
            ui.label("Keep HiveRig.exe and hive-rig.exe together: Blender and Maya call hive-rig.exe to rebuild.");
        });
    }

    fn viewport(&mut self, ui: &mut egui::Ui) {
        let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = response.rect;

        // Camera controls.
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta().y);
            if scroll != 0.0 {
                self.camera.zoom(scroll);
            }
            if ui.input(|i| i.key_pressed(Key::F)) && !ui.ctx().egui_wants_keyboard_input() {
                self.frame();
            }
            if ui.input(|i| i.key_pressed(Key::Delete)) && !ui.ctx().egui_wants_keyboard_input() {
                self.remove_selected();
            }
        }
        let pr = self.camera.projector(rect);
        let pointer = response.interact_pointer_pos().or(response.hover_pos());

        // Pick where the button went down: a drag only starts once the pointer has moved.
        let press = ui.input(|i| i.pointer.press_origin());
        if response.drag_started_by(PointerButton::Primary)
            && let Some(p) = press.or(pointer)
            && let Some((c, g)) = self.pick_guide(&pr, p)
        {
            self.selected = Some((c, Some(g)));
            self.begin_change();
            self.drag = Some(Drag { comp: c, guide: g });
        }
        if response.dragged_by(PointerButton::Primary) {
            match (&self.drag, pointer) {
                (Some(d), Some(p)) => {
                    let (c, g) = (d.comp, d.guide);
                    let current =
                        self.doc.as_ref().unwrap().guides.components[c].guides[g].position;
                    let to = pr.drag_point(p, current);
                    self.move_guide(c, g, to);
                }
                _ => self.camera.orbit(response.drag_delta()),
            }
        }
        if response.dragged_by(PointerButton::Secondary)
            || response.dragged_by(PointerButton::Middle)
        {
            self.camera.pan(response.drag_delta(), rect.height());
        }
        if response.drag_stopped() {
            self.drag = None;
        }
        if response.clicked()
            && let Some(p) = pointer
        {
            self.selected = self.pick_guide(&pr, p).map(|(c, g)| (c, Some(g)));
        }
        let pr = self.camera.projector(rect);
        let painter = painter.with_clip_rect(rect);
        self.paint(&painter, &pr, pointer);

        painter.text(
            rect.left_bottom() + egui::vec2(10.0, -10.0),
            egui::Align2::LEFT_BOTTOM,
            "Left-drag: orbit / move guide   Right-drag: pan   Wheel: zoom   F: frame   Del: remove component",
            egui::FontId::proportional(12.0),
            Color32::from_gray(150),
        );
    }

    fn pick_guide(&self, pr: &Projector, at: Pos2) -> Option<(usize, usize)> {
        if !self.show_guides {
            return None;
        }
        let doc = self.doc.as_ref()?;
        let mut best: Option<(f32, f32, usize, usize)> = None;
        for (ci, c) in doc.guides.components.iter().enumerate() {
            for (gi, g) in c.guides.iter().enumerate() {
                let Some((s, depth)) = pr.project(g.position) else {
                    continue;
                };
                let d = s.distance(at);
                // Nearest on screen; ties (overlapping guides) go to the nearer one.
                if d <= PICK_RADIUS && best.is_none_or(|b| (d, depth) < (b.0, b.1)) {
                    best = Some((d, depth, ci, gi));
                }
            }
        }
        best.map(|b| (b.2, b.3))
    }

    fn paint(&mut self, painter: &egui::Painter, pr: &Projector, pointer: Option<Pos2>) {
        let weights = self.weights_bone.clone();
        let colors = match &weights {
            Some(b) if self.show_mesh => self.weights_for(b).map(<[Color32]>::to_vec),
            _ => None,
        };
        let Some(doc) = &self.doc else { return };
        if self.show_mesh {
            let alpha = if colors.is_some() {
                1.0
            } else {
                self.mesh_opacity
            };
            let base = Color32::from_rgb(170, 172, 180);
            view::draw_mesh(painter, pr, &doc.mesh, base, colors.as_deref(), alpha);
        }
        let Ok(built) = &doc.preview else { return };
        let bones = &built.skeleton.bones;

        if self.show_controls {
            for c in &built.controls.controls {
                let stroke = Stroke::new(1.5, rgb(c.color));
                view::ring(painter, pr, c.position, c.axis, c.size.max(0.01), stroke);
            }
        }
        if self.show_bones {
            for b in bones {
                let (Some(h), Some(t)) = (pr.pos(b.head), pr.pos(b.tail)) else {
                    continue;
                };
                let color = if weights.as_deref() == Some(b.name.as_str()) {
                    Color32::WHITE
                } else if b.deform {
                    Color32::from_rgb(225, 225, 235)
                } else {
                    Color32::from_rgb(130, 130, 150)
                };
                bone_shape(painter, h, t, color);
            }
        }
        if self.show_guides {
            let hover = pointer.and_then(|p| self.pick_guide(pr, p));
            for (ci, c) in doc.guides.components.iter().enumerate() {
                let comp_sel = self.selected.is_some_and(|(s, _)| s == ci);
                let color = side_color(c.side);
                let pts: Vec<Option<Pos2>> = c.guides.iter().map(|g| pr.pos(g.position)).collect();
                // Link the component to the bone it hangs from.
                if let (Some(parent), Some(Some(first))) = (&c.parent, pts.first())
                    && let Some(pb) = bones.iter().find(|b| &b.name == parent)
                    && let Some(ph) = pr.pos(pb.tail)
                {
                    dashed(
                        painter,
                        ph,
                        *first,
                        Stroke::new(1.0, Color32::from_gray(120)),
                    );
                }
                let line = Stroke::new(
                    if comp_sel { 2.5 } else { 1.5 },
                    color.gamma_multiply(if comp_sel { 1.0 } else { 0.7 }),
                );
                for w in pts.windows(2) {
                    if let [Some(a), Some(b)] = w {
                        painter.line_segment([*a, *b], line);
                    }
                }
                for (gi, p) in pts.iter().enumerate() {
                    let Some(p) = p else { continue };
                    let sel = self.selected == Some((ci, Some(gi)));
                    let hot = hover == Some((ci, gi));
                    let r = if sel || hot {
                        7.0
                    } else if comp_sel {
                        5.5
                    } else {
                        4.5
                    };
                    painter.circle_filled(*p, r, color);
                    painter.circle_stroke(
                        *p,
                        r,
                        Stroke::new(
                            if sel { 2.0 } else { 1.0 },
                            if sel { Color32::WHITE } else { Color32::BLACK },
                        ),
                    );
                    if sel || hot {
                        painter.text(
                            *p + egui::vec2(10.0, -10.0),
                            egui::Align2::LEFT_BOTTOM,
                            format!("{} · {}", c.id, c.guides[gi].name),
                            egui::FontId::proportional(13.0),
                            Color32::WHITE,
                        );
                    }
                }
            }
        }
    }
}

/// Whether two layouts have identical guide positions and components.
fn same_layout(a: &Guides, b: &Guides) -> bool {
    a.components.len() == b.components.len()
        && a.components.iter().zip(&b.components).all(|(x, y)| {
            x.id == y.id
                && x.guides.len() == y.guides.len()
                && x.guides
                    .iter()
                    .zip(&y.guides)
                    .all(|(p, q)| p.position == q.position)
        })
}

/// A bone as a thin diamond from head to tail.
fn bone_shape(painter: &egui::Painter, head: Pos2, tail: Pos2, color: Color32) {
    let d = tail - head;
    let len = d.length();
    if len < 1.0 {
        painter.circle_filled(head, 2.0, color);
        return;
    }
    let n = egui::vec2(-d.y, d.x) / len * (len * 0.1).clamp(1.5, 6.0);
    let mid = head + d * 0.2;
    let outline = vec![head, mid + n, tail, mid - n];
    painter.add(egui::Shape::convex_polygon(
        outline,
        color.gamma_multiply(0.35),
        Stroke::new(1.2, color),
    ));
    painter.circle_filled(head, 2.5, color);
}

fn dashed(painter: &egui::Painter, a: Pos2, b: Pos2, stroke: Stroke) {
    painter.extend(egui::Shape::dashed_line(&[a, b], stroke, 4.0, 4.0));
}

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
