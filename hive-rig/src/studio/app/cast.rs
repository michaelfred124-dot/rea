//! Several characters in the app: a scene of characters, each with its own
//! guides, rig, undo history and pose, one of them being edited at a time
//! (`Studio::doc`) while the others stand beside it in the viewport. Built
//! and exported together or one at a time, saved as a `.scene.json`, and with
//! props held across characters (a control of one held by a bone of
//! another). The scene file and its builders are `hive_rig::scene`.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Align2, Color32, FontId, Pos2, RichText, Sense, Stroke, Ui, Vec2};
use glam::Vec3;

use hive_rig::export;
use hive_rig::scene::{self, BoneRef, Hold, PropRef, Scene, SceneCharacter};
use hive_rig::skin::SkinOptions;
use hive_rig::{Target, demo};

use super::{Doc, Mode, Studio, posing::Posing};
use crate::theme::{ACCENT, BORDER, CARD_HI, TEXT, TEXT_DIM, TEXT_FAINT};
use crate::view::{self, Projector};

/// Gap between characters placed side by side, as a fraction of the taller one.
const SIDE_GAP: f32 = 0.25;

/// The characters of the scene and the props held across them.
#[derive(Default)]
pub struct Cast {
    /// Every character in scene order. The one being edited is in
    /// `Studio::doc`, its slot here empty; with one character this may be empty.
    slots: Vec<Option<(Doc, Posing)>>,
    /// The slot of the character being edited.
    pub active: usize,
    pub holds: Vec<Hold>,
    /// The scene's name and file, once it has one.
    pub name: String,
    pub path: Option<PathBuf>,
}

impl Studio {
    fn ensure_slots(&mut self) {
        if self.cast.slots.is_empty() && self.doc.is_some() {
            self.cast.slots.push(None);
            self.cast.active = 0;
        }
    }

    pub fn character_count(&self) -> usize {
        self.cast.slots.len().max(self.doc.is_some() as usize)
    }

    /// The character in slot `i`.
    pub fn character(&self, i: usize) -> Option<&Doc> {
        if i == self.cast.active {
            return self.doc.as_ref();
        }
        self.cast.slots.get(i)?.as_ref().map(|(d, _)| d)
    }

    pub fn character_names(&self) -> Vec<String> {
        (0..self.character_count())
            .filter_map(|i| self.character(i).map(|d| d.name.clone()))
            .collect()
    }

    pub fn character_index(&self, name: &str) -> Option<usize> {
        (0..self.character_count()).find(|&i| self.character(i).is_some_and(|d| d.name == name))
    }

    /// Edit another character: the one being edited steps aside with its pose.
    pub fn switch_character(&mut self, i: usize) -> bool {
        self.ensure_slots();
        if i == self.cast.active || i >= self.cast.slots.len() {
            return false;
        }
        let Some((doc, posing)) = self.cast.slots[i].take() else {
            return false;
        };
        if let Some(cur) = self.doc.take() {
            let pose = std::mem::take(&mut self.posing);
            self.cast.slots[self.cast.active] = Some((cur, pose));
        }
        let mode = self.mode;
        self.set_doc(doc);
        self.posing = posing;
        self.mode = mode;
        self.cast.active = i;
        true
    }

    /// Add a character with `load` (which opens one into `Studio::doc`), next
    /// to the others, and edit it. The one being edited stays in the scene.
    fn add_with(&mut self, load: impl FnOnce(&mut Studio)) -> bool {
        self.ensure_slots();
        let had = self
            .doc
            .take()
            .map(|d| (d, std::mem::take(&mut self.posing)));
        let before = self.cast.active;
        let mode = self.mode;
        if let Some(cur) = had {
            self.cast.slots[before] = Some(cur);
        }
        self.cast.slots.push(None);
        self.cast.active = self.cast.slots.len() - 1;
        load(self);
        if self.doc.is_none() {
            // It didn't open: back to how it was.
            self.cast.slots.pop();
            self.cast.active = before;
            if let Some(Some((d, p))) = self.cast.slots.get_mut(before).map(Option::take) {
                let status = (self.status.clone(), self.error);
                self.set_doc(d);
                self.posing = p;
                (self.status, self.error) = status;
                self.mode = mode;
            }
            if self.cast.slots.len() == 1 && self.doc.is_none() {
                self.cast.slots.clear();
            }
            return false;
        }
        let names: Vec<String> = (0..self.cast.slots.len() - 1)
            .filter_map(|i| self.character(i).map(|d| d.name.clone()))
            .collect();
        let offset = self.free_place();
        let doc = self.doc.as_mut().unwrap();
        let base = doc.name.clone();
        let mut name = base.clone();
        let mut n = 2;
        while names.contains(&name) {
            name = format!("{base}_{n}");
            n += 1;
        }
        doc.name = name.clone();
        doc.offset = offset;
        let msg = format!("{} Added {name} to the scene.", self.status);
        if self.error {
            self.fail(msg);
        } else {
            self.info(msg);
        }
        true
    }

    /// Where a new character (now in `Studio::doc`) stands: to the right of
    /// the others, on the same floor.
    fn free_place(&self) -> Vec3 {
        let Some(doc) = &self.doc else {
            return Vec3::ZERO;
        };
        let (min, max) = doc.mesh.bounds();
        let mut right = f32::NEG_INFINITY;
        let mut tallest = max.y - min.y;
        for i in 0..self.character_count() {
            if i == self.cast.active {
                continue;
            }
            if let Some(o) = self.character(i) {
                let (a, b) = o.mesh.bounds();
                right = right.max(o.offset.x + b.x);
                tallest = tallest.max(b.y - a.y);
            }
        }
        if !right.is_finite() {
            return Vec3::ZERO;
        }
        Vec3::new(right + SIDE_GAP * tallest - min.x, 0.0, 0.0)
    }

    pub fn add_demo_character(&mut self, creature: Option<demo::Creature>) -> bool {
        self.add_with(|s| match creature {
            Some(c) => s.load_creature(c),
            None => s.load_demo(),
        })
    }

    pub fn add_character_file(&mut self, path: &Path) -> bool {
        let path = path.to_path_buf();
        self.add_with(move |s| s.open(&path))
    }

    /// Take a character out of the scene, and the holds that name it.
    pub fn remove_character(&mut self, i: usize) -> bool {
        self.ensure_slots();
        if self.cast.slots.len() < 2 || i >= self.cast.slots.len() {
            self.warning("A scene keeps at least one character.");
            return false;
        }
        if i == self.cast.active {
            self.switch_character(if i == 0 { 1 } else { i - 1 });
        }
        let Some((gone, _)) = self.cast.slots.remove(i) else {
            return false;
        };
        if self.cast.active > i {
            self.cast.active -= 1;
        }
        self.cast
            .holds
            .retain(|h| h.prop.character != gone.name && h.holder.character != gone.name);
        self.info(format!("Took {} out of the scene.", gone.name));
        true
    }

    /// Let `holder`'s `bone` hold the edited character's `control`.
    pub fn hold_prop(&mut self, control: &str, holder: &str, bone: &str, held: f32) -> bool {
        let Some(doc) = &self.doc else { return false };
        let me = doc.name.clone();
        let result = (|| -> anyhow::Result<()> {
            anyhow::ensure!(
                holder != me,
                "{me} can't hold its own control: use a space switch"
            );
            let built = doc.preview.as_ref().map_err(|e| anyhow::anyhow!("{e}"))?;
            anyhow::ensure!(
                built.controls.control(control).is_some(),
                "{me} has no control named {control}"
            );
            let i = self
                .character_index(holder)
                .ok_or_else(|| anyhow::anyhow!("no character named {holder}"))?;
            let other = self.character(i).unwrap();
            let skeleton = &other
                .preview
                .as_ref()
                .map_err(|e| anyhow::anyhow!("{holder}'s guides don't build: {e}"))?
                .skeleton;
            anyhow::ensure!(
                skeleton.bone(bone).is_some(),
                "{holder} has no bone named {bone}"
            );
            anyhow::ensure!((0.0..=1.0).contains(&held), "held is from 0 to 1");
            Ok(())
        })();
        if let Err(e) = result {
            self.fail(format!("Can't hold the prop: {e:#}"));
            return false;
        }
        let prop = PropRef {
            character: me.clone(),
            control: control.to_string(),
        };
        self.cast.holds.retain(|h| h.prop != prop);
        self.cast.holds.push(Hold {
            prop,
            holder: BoneRef {
                character: holder.to_string(),
                bone: bone.to_string(),
            },
            held,
        });
        self.info(format!("{me}'s {control} is held by {holder}'s {bone}."));
        true
    }

    pub fn release_prop(&mut self, control: &str) -> bool {
        let Some(me) = self.doc.as_ref().map(|d| d.name.clone()) else {
            return false;
        };
        let before = self.cast.holds.len();
        self.cast
            .holds
            .retain(|h| !(h.prop.character == me && h.prop.control == control));
        let done = self.cast.holds.len() != before;
        if done {
            self.info(format!("{me}'s {control} is on its own socket again."));
        }
        done
    }

    /// The hold on one of the edited character's controls.
    pub fn hold_of(&self, control: &str) -> Option<&Hold> {
        let me = &self.doc.as_ref()?.name;
        self.cast
            .holds
            .iter()
            .find(|h| &h.prop.character == me && h.prop.control == control)
    }

    /// Build every character's rig; back to the one being edited afterwards.
    pub fn build_all(&mut self) -> bool {
        self.ensure_slots();
        let start = self.cast.active;
        let n = self.character_count();
        let mut built = 0;
        for i in 0..n {
            self.switch_character(i);
            if !self.build() {
                let msg = format!("{} ({})", self.status, self.doc.as_ref().unwrap().name);
                self.switch_character(start);
                self.fail(msg);
                return false;
            }
            built += 1;
        }
        self.switch_character(start);
        self.mode = Mode::Rig;
        self.info(format!("Built all {built} characters."));
        true
    }

    /// The scene as a file holds it, with each character's mesh source and,
    /// once built, its rig file relative to `dir`.
    pub fn to_scene(&self, dir: &Path, rigs: &[Option<PathBuf>]) -> anyhow::Result<Scene> {
        let name = if self.cast.name.is_empty() {
            self.doc
                .as_ref()
                .map_or("scene".into(), |d| format!("{}_scene", d.name))
        } else {
            self.cast.name.clone()
        };
        let mut s = Scene::new(&name);
        let dir_abs = std::path::absolute(dir)?;
        let relative = |p: &Path| -> String {
            let abs = std::path::absolute(p).unwrap_or_else(|_| p.to_path_buf());
            abs.strip_prefix(&dir_abs)
                .map(|r| r.to_string_lossy().replace('\\', "/"))
                .unwrap_or_else(|_| abs.to_string_lossy().into_owned())
        };
        for i in 0..self.character_count() {
            let d = self.character(i).unwrap();
            let source = match &d.origin {
                Some(o) if o.starts_with("demo") => o.clone(),
                Some(o) => relative(Path::new(o)),
                None => anyhow::bail!("{} has no file to come back from", d.name),
            };
            s.add(SceneCharacter {
                name: d.name.clone(),
                source,
                orientation: d.orientation,
                offset: d.offset,
                guides: d.guides.clone(),
                rig: rigs.get(i).cloned().flatten().map(|p| relative(&p)),
            })?;
        }
        s.holds = self.cast.holds.clone();
        s.validate()?;
        Ok(s)
    }

    /// Save the scene: the scene file, and each built character's rig in
    /// `<scene>_rigs/` next to it so its weights come back.
    pub fn save_scene(&mut self, path: &Path) -> bool {
        let result = (|| -> anyhow::Result<Scene> {
            let dir = path
                .parent()
                .filter(|d| !d.as_os_str().is_empty())
                .unwrap_or(Path::new("."))
                .to_path_buf();
            let file = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let stem = file
                .strip_suffix(scene::SCENE_SUFFIX)
                .unwrap_or(&file)
                .to_string();
            self.cast.name = export::sanitize_name(&stem);
            let rigs_dir = dir.join(format!("{}_rigs", self.cast.name));
            let mut rigs = Vec::new();
            for i in 0..self.character_count() {
                let d = self.character(i).unwrap();
                rigs.push(match &d.rig {
                    Some(r) => {
                        let mut r = r.clone();
                        r.name = d.name.clone();
                        export::write_all(&r, &rigs_dir, &[])?;
                        Some(rigs_dir.join(format!("{}.hive.json", d.name)))
                    }
                    None => None,
                });
            }
            let s = self.to_scene(&dir, &rigs)?;
            s.save(path)?;
            Ok(s)
        })();
        match result {
            Ok(s) => {
                self.cast.path = Some(path.to_path_buf());
                self.info(format!(
                    "Saved the scene {} ({} characters) to {}",
                    s.name,
                    s.characters.len(),
                    path.display()
                ));
                true
            }
            Err(e) => {
                self.fail(format!("Couldn't save the scene: {e:#}"));
                false
            }
        }
    }

    /// Open a scene file: every character, its built rig where saved, the holds.
    pub fn open_scene(&mut self, path: &Path) -> anyhow::Result<()> {
        let s = Scene::load(path)?;
        anyhow::ensure!(!s.characters.is_empty(), "the scene has no characters");
        let dir = path
            .parent()
            .filter(|d| !d.as_os_str().is_empty())
            .unwrap_or(Path::new("."))
            .to_path_buf();
        let out_dir = dir.join(format!("{}_rigs", s.name));
        let mut docs = Vec::new();
        for c in &s.characters {
            let saved = scene::saved_rig(c, &dir);
            let mesh = match &saved {
                Some(r) => r.mesh.clone(),
                None => scene::load_mesh(c, &dir)?,
            };
            let mut doc = Doc::new(c.name.clone(), mesh, c.guides.clone(), out_dir.clone());
            doc.orientation = c.orientation;
            doc.offset = c.offset;
            doc.origin = Some(if c.source.starts_with("demo") {
                c.source.clone()
            } else {
                dir.join(&c.source).to_string_lossy().into_owned()
            });
            let mut posing = Posing::default();
            if let Some(rig) = saved
                && serde_json::to_value(&rig.guides).ok() == serde_json::to_value(&c.guides).ok()
            {
                doc.auto_skin = Some(rig.auto_weights(SkinOptions::default()));
                posing.pose = rig.pose.clone();
                doc.rig = Some(rig);
            }
            docs.push((doc, posing));
        }
        let mut docs = docs.into_iter();
        let (first, posing) = docs.next().unwrap();
        self.set_doc(first);
        self.posing = posing;
        self.cast = Cast {
            slots: std::iter::once(None).chain(docs.map(Some)).collect(),
            active: 0,
            holds: s.holds.clone(),
            name: s.name.clone(),
            path: Some(path.to_path_buf()),
        };
        if self.doc.as_ref().is_some_and(|d| d.rig.is_some()) {
            self.mode = Mode::Rig;
        }
        self.info(format!(
            "Opened the scene {} with {} characters and {} held props.",
            s.name,
            s.characters.len(),
            s.holds.len()
        ));
        Ok(())
    }

    /// Export every character into `dir` (building those that need it) and
    /// the scene builders for the chosen programs.
    pub fn export_scene(&mut self, dir: &Path) -> anyhow::Result<Vec<PathBuf>> {
        self.ensure_slots();
        let start = self.cast.active;
        let mut files = Vec::new();
        let mut rigs = Vec::new();
        for i in 0..self.character_count() {
            self.switch_character(i);
            match self.export(dir) {
                Ok(f) => files.extend(f),
                Err(e) => {
                    let name = self
                        .doc
                        .as_ref()
                        .map(|d| d.name.clone())
                        .unwrap_or_default();
                    self.switch_character(start);
                    anyhow::bail!("{name}: {e:#}");
                }
            }
            let name = self.doc.as_ref().unwrap().name.clone();
            rigs.push(Some(dir.join(format!("{name}.hive.json"))));
        }
        self.switch_character(start);
        let s = self.to_scene(dir, &rigs)?;
        for (t, on) in Target::ALL.into_iter().zip(self.targets) {
            if on && t != Target::Fbx {
                let path = dir.join(format!("{}_scene_{}.py", s.name, t.key()));
                std::fs::write(&path, scene::script(&s, t)?)?;
                files.push(path);
            }
        }
        Ok(files)
    }

    /// The other characters in the viewport, beside the edited one, with their
    /// controls in the Rig stage and a line for each held prop.
    pub(super) fn paint_cast(&self, painter: &egui::Painter, pr: &Projector) {
        let Some(doc) = &self.doc else { return };
        let here = doc.offset;
        for i in 0..self.character_count() {
            if i == self.cast.active {
                continue;
            }
            let Some(o) = self.character(i) else { continue };
            let shift = o.offset - here;
            if self.show_mesh {
                let positions: Vec<Vec3> = o.mesh.positions.iter().map(|p| *p + shift).collect();
                view::draw_mesh(
                    painter,
                    pr,
                    &o.mesh,
                    &positions,
                    Color32::from_rgb(120, 126, 140),
                    None,
                    None,
                    if self.xray { 0.25 } else { 0.85 },
                    &[],
                );
            }
            if let Ok(built) = &o.preview {
                if self.mode == Mode::Rig {
                    for c in &built.controls.controls {
                        let lines: Vec<Vec<Vec3>> = c
                            .rest_lines()
                            .into_iter()
                            .map(|l| l.into_iter().map(|p| p + shift).collect())
                            .collect();
                        let [r, g, b] = c.color.map(|v| (v * 255.0) as u8);
                        view::strokes(
                            painter,
                            pr,
                            &lines,
                            Stroke::new(
                                1.3 * c.width(),
                                Color32::from_rgb(r, g, b).gamma_multiply(0.55),
                            ),
                        );
                    }
                } else {
                    for b in built.skeleton.bones.iter().filter(|b| b.deform) {
                        if let (Some(h), Some(t)) = (pr.pos(b.head + shift), pr.pos(b.tail + shift))
                        {
                            painter.line_segment([h, t], Stroke::new(1.2, TEXT_FAINT));
                        }
                    }
                }
            }
            let (_, max) = o.mesh.bounds();
            let (min, _) = o.mesh.bounds();
            let top = Vec3::new((min.x + max.x) * 0.5, max.y, (min.z + max.z) * 0.5) + shift;
            if let Some(at) = pr.pos(top) {
                name_chip(painter, at + Vec2::new(0.0, -14.0), &o.name);
            }
        }
        // Held props: a line from the prop's control to the hand holding it.
        for h in &self.cast.holds {
            let place = |name: &str| -> Option<(&Doc, Vec3)> {
                let i = self.character_index(name)?;
                let d = self.character(i)?;
                Some((d, d.offset - here))
            };
            let (Some((a, sa)), Some((b, sb))) =
                (place(&h.prop.character), place(&h.holder.character))
            else {
                continue;
            };
            let prop = a
                .preview
                .as_ref()
                .ok()
                .and_then(|x| x.controls.control(&h.prop.control))
                .map(|c| c.position + sa);
            let hand = b
                .preview
                .as_ref()
                .ok()
                .and_then(|x| x.skeleton.bone(&h.holder.bone))
                .map(|bone| bone.head + sb);
            if let (Some(p), Some(q)) = (prop.and_then(|p| pr.pos(p)), hand.and_then(|q| pr.pos(q)))
            {
                painter.extend(egui::Shape::dashed_line(
                    &[p, q],
                    Stroke::new(1.6, ACCENT.gamma_multiply(0.85)),
                    6.0,
                    4.0,
                ));
                painter.circle_stroke(q, 6.0, Stroke::new(1.6, ACCENT));
                painter.circle_filled(p, 3.5, ACCENT);
                name_chip(
                    painter,
                    p.lerp(q, 0.5) + Vec2::new(0.0, -12.0),
                    &format!("held by {}", h.holder.character),
                );
            }
        }
    }

    /// The outliner's list of characters, with adding and removing.
    pub(super) fn characters_section(&mut self, ui: &mut Ui) {
        let names = self.character_names();
        let active = self.cast.active.min(names.len().saturating_sub(1));
        let mut switch = None;
        let mut remove = None;
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("CHARACTERS  {}", names.len()))
                    .size(10.5)
                    .strong()
                    .color(TEXT_FAINT),
            );
        });
        for (i, name) in names.iter().enumerate() {
            let on = i == active;
            let built = self.character(i).is_some_and(|d| d.rig.is_some());
            let holds = self
                .cast
                .holds
                .iter()
                .filter(|h| &h.prop.character == name || &h.holder.character == name)
                .count();
            let row = character_row(ui, name, on, built, holds);
            if row.clicked() && !on {
                switch = Some(i);
            }
            row.context_menu(|ui| {
                if ui.button("Edit this character").clicked() {
                    switch = Some(i);
                    ui.close();
                }
                if ui
                    .add_enabled(names.len() > 1, egui::Button::new("Take out of the scene"))
                    .clicked()
                {
                    remove = Some(i);
                    ui.close();
                }
            });
        }
        ui.horizontal(|ui| {
            ui.menu_button(
                RichText::new("+ Add character").size(12.5).color(ACCENT),
                |ui| {
                    if ui.button("From a file…").clicked() {
                        ui.close();
                        if let Some(p) = rfd::FileDialog::new()
                            .add_filter(
                                "Characters and rigs",
                                &["glb", "gltf", "fbx", "obj", "json"],
                            )
                            .pick_file()
                        {
                            self.add_character_file(&p);
                        }
                    }
                    if ui.button("Demo character").clicked() {
                        ui.close();
                        self.add_demo_character(None);
                    }
                    for c in demo::Creature::ALL {
                        if ui.button(format!("Demo {}", c.name())).clicked() {
                            ui.close();
                            self.add_demo_character(Some(c));
                        }
                    }
                },
            );
            if names.len() > 1
                && ui
                    .small_button("Build all")
                    .on_hover_text("Build every character's rig")
                    .clicked()
            {
                self.build_all();
            }
        });
        if let Some(i) = switch {
            self.switch_character(i);
        }
        if let Some(i) = remove {
            self.remove_character(i);
        }
        ui.add_space(2.0);
        let r = ui.available_rect_before_wrap();
        ui.painter().line_segment(
            [Pos2::new(r.left(), r.top()), Pos2::new(r.right(), r.top())],
            Stroke::new(1.0, BORDER),
        );
        ui.add_space(6.0);
    }

    /// Properties: let another character hold this control.
    pub(super) fn hold_section(&mut self, ui: &mut Ui, control: &str) {
        let me = self
            .doc
            .as_ref()
            .map(|d| d.name.clone())
            .unwrap_or_default();
        let others: Vec<String> = self
            .character_names()
            .into_iter()
            .filter(|n| *n != me)
            .collect();
        if others.is_empty() {
            return;
        }
        crate::theme::label_caps(ui, "Held by another character");
        let current = self.hold_of(control).cloned();
        let id = ui.make_persistent_id(("hold", control));
        let (mut who, mut bone, mut held) = ui
            .data(|d| d.get_temp::<(String, String, f32)>(id))
            .unwrap_or_else(|| match &current {
                Some(h) => (h.holder.character.clone(), h.holder.bone.clone(), h.held),
                None => (others[0].clone(), "hand_l".to_string(), 1.0),
            });
        let bones: Vec<String> = self
            .character_index(&who)
            .and_then(|i| self.character(i))
            .and_then(|d| d.preview.as_ref().ok())
            .map(|b| {
                b.skeleton
                    .bones
                    .iter()
                    .filter(|b| b.deform)
                    .map(|b| b.name.clone())
                    .collect()
            })
            .unwrap_or_default();
        egui::ComboBox::from_id_salt(("hold_who", control))
            .selected_text(&who)
            .show_ui(ui, |ui| {
                for o in &others {
                    ui.selectable_value(&mut who, o.clone(), o);
                }
            });
        egui::ComboBox::from_id_salt(("hold_bone", control))
            .selected_text(&bone)
            .height(260.0)
            .show_ui(ui, |ui| {
                for b in &bones {
                    ui.selectable_value(&mut bone, b.clone(), b);
                }
            });
        ui.add(egui::Slider::new(&mut held, 0.0..=1.0).text("held"));
        ui.horizontal(|ui| {
            let label = if current.is_some() { "Update" } else { "Hold" };
            if ui.button(label).clicked() {
                self.hold_prop(control, &who, &bone, held);
            }
            if current.is_some() && ui.button("Release").clicked() {
                self.release_prop(control);
            }
        });
        ui.data_mut(|d| d.insert_temp(id, (who, bone, held)));
        if let Some(h) = &current {
            ui.label(
                RichText::new(format!(
                    "Follows {}'s {} (held {:.2}) in the scene builders.",
                    h.holder.character, h.holder.bone, h.held
                ))
                .size(11.5)
                .color(TEXT_FAINT),
            );
        }
        ui.add_space(8.0);
    }
}

fn name_chip(painter: &egui::Painter, at: Pos2, text: &str) {
    let galley = painter.layout_no_wrap(text.into(), FontId::proportional(12.0), TEXT);
    let rect = egui::Rect::from_center_size(at, galley.size() + Vec2::new(14.0, 6.0));
    painter.rect_filled(rect, 8.0, Color32::from_rgba_unmultiplied(20, 23, 30, 220));
    painter.rect_stroke(
        rect,
        8.0,
        Stroke::new(1.0, BORDER),
        egui::StrokeKind::Inside,
    );
    painter.galley(rect.center() - galley.size() / 2.0, galley, TEXT);
}

fn character_row(ui: &mut Ui, name: &str, on: bool, built: bool, holds: usize) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::click());
    let painter = ui.painter();
    if on {
        painter.rect_filled(rect, 8, ACCENT.gamma_multiply(0.14));
        painter.rect_stroke(
            rect,
            8,
            Stroke::new(1.0, ACCENT.gamma_multiply(0.6)),
            egui::StrokeKind::Inside,
        );
    } else if response.hovered() {
        painter.rect_filled(rect, 8, CARD_HI);
    }
    let y = rect.center().y;
    // A little figure for a character.
    let x = rect.left() + 14.0;
    let ink = if on { ACCENT } else { TEXT_DIM };
    painter.circle_stroke(Pos2::new(x, y - 5.0), 3.0, Stroke::new(1.3, ink));
    painter.line_segment(
        [Pos2::new(x, y - 2.0), Pos2::new(x, y + 4.0)],
        Stroke::new(1.3, ink),
    );
    painter.line_segment(
        [Pos2::new(x - 4.0, y + 8.0), Pos2::new(x, y + 4.0)],
        Stroke::new(1.3, ink),
    );
    painter.line_segment(
        [Pos2::new(x + 4.0, y + 8.0), Pos2::new(x, y + 4.0)],
        Stroke::new(1.3, ink),
    );
    painter.text(
        Pos2::new(rect.left() + 28.0, y),
        Align2::LEFT_CENTER,
        name,
        FontId::proportional(13.5),
        if on {
            TEXT
        } else {
            Color32::from_rgb(205, 209, 218)
        },
    );
    let mut note = if built {
        "built".to_string()
    } else {
        "guides".to_string()
    };
    if holds > 0 {
        note = format!("{note} · {holds} held");
    }
    painter.text(
        Pos2::new(rect.right() - 10.0, y),
        Align2::RIGHT_CENTER,
        note,
        FontId::proportional(11.5),
        TEXT_FAINT,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}
