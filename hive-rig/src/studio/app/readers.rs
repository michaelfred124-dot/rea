//! Cone and RBF pose readers in the app: make one from the pose that shows
//! (the test pose or the control-rig pose), add that pose to an RBF reader,
//! give a reader a helper joint, and watch every output's weight live as the
//! rig is posed. The readers themselves are `hive_rig::readers`; these keep
//! the document and its built rig in step, undoably with the shapes.

use eframe::egui::{self, RichText, Ui};
use glam::Quat;

use hive_rig::Rig;
use hive_rig::readers::{self, Cone, Kind, Rbf, Reader};
use hive_rig::shapes::Shape;

use super::Studio;
use crate::theme::{self, ACCENT, TEXT, TEXT_DIM, TEXT_FAINT};

/// What a click in the Pose readers card asks for, run after the card is drawn.
enum ReaderAction {
    Pick(String),
    Cone,
    Rbf,
    AddPose,
    AddHelper,
    Remove(String),
    RemoveSample(usize),
    /// A reader changed by a slider (no undo step of its own).
    Tweak(Reader),
}

impl Studio {
    /// The reader's outputs in the pose that shows.
    pub fn reader_outputs_now(&self, r: &Reader, rig: &Rig) -> Vec<f32> {
        let locals = self.pose_locals();
        match rig.skeleton.index_of(&r.bone) {
            Some(i) => r.evaluate(
                locals.get(i).copied().unwrap_or(Quat::IDENTITY),
                rig.skeleton.bones[i].direction(),
            ),
            None => vec![0.0; r.outputs()],
        }
    }

    /// The bone a new reader reads: the test-posed bone, else the picked one.
    fn reader_bone(&self) -> Option<String> {
        if let Some(p) = &self.pose {
            return Some(p.bone.clone());
        }
        match &self.rig_pick {
            Some(super::RigPick::Bone(b)) => Some(b.clone()),
            _ => None,
        }
    }

    /// Put a rig a reader or shape operation changed back on the document:
    /// its shapes and readers, and its skeleton and skin when its helper
    /// joints changed.
    pub(super) fn adopt_reader_rig(&mut self, rig: Rig) {
        let Some(doc) = self.doc.as_mut() else { return };
        let bones_changed = doc.rig.as_ref().is_some_and(|r| {
            let joints = |r: &Rig| -> Vec<(String, glam::Vec3, glam::Vec3)> {
                r.skeleton
                    .bones
                    .iter()
                    .map(|b| (b.name.clone(), b.head, b.tail))
                    .collect()
            };
            joints(r) != joints(&rig) || r.skin.joints != rig.skin.joints
        });
        if let Some(r) = doc.rig.as_mut() {
            if bones_changed {
                r.skeleton = rig.skeleton.clone();
                r.skin = rig.skin.clone();
            }
            r.mesh.shapes = rig.mesh.shapes.clone();
            r.mesh.readers = rig.mesh.readers.clone();
        }
        if bones_changed {
            doc.auto_skin = Some(rig.auto_weights(Default::default()));
            doc.kept = None;
            self.weight_colors = None;
        }
        if let Some((r, _)) = doc.kept.as_mut() {
            r.mesh.shapes = rig.mesh.shapes.clone();
            r.mesh.readers = rig.mesh.readers.clone();
        }
        doc.mesh.shapes = rig.mesh.shapes;
        doc.mesh.readers = rig.mesh.readers;
        doc.place_helpers();
        doc.exported = false;
    }

    /// Undo: the readers as they were (helper joints and skin reworked when
    /// they differ from the built rig's).
    pub(super) fn restore_readers(&mut self, list: Vec<Reader>) {
        let Some(mut rig) = self.shape_rig() else {
            if let Some(doc) = self.doc.as_mut() {
                doc.mesh.readers = list;
                doc.place_helpers();
            }
            return;
        };
        rig.mesh.readers = list;
        if !readers::helpers_in_place(&rig)
            && let Err(e) = readers::reskin(&mut rig)
        {
            self.fail(format!("{e:#}"));
            return;
        }
        self.adopt_reader_rig(rig);
    }

    /// Run a reader operation on the built rig, undoably.
    fn reader_edit(&mut self, f: impl FnOnce(&mut Rig) -> anyhow::Result<String>) -> bool {
        let Some(mut rig) = self.shape_rig() else {
            self.warning("Build the rig first: readers read the built rig's bones.");
            return false;
        };
        match f(&mut rig).and_then(|msg| rig.mesh.validate().map(|_| msg)) {
            Ok(msg) => {
                self.push_shape_undo();
                self.adopt_reader_rig(rig);
                self.info(msg);
                true
            }
            Err(e) => {
                self.fail(format!("{e:#}"));
                false
            }
        }
    }

    /// The picked reader.
    fn picked_reader(&self) -> Option<&Reader> {
        let name = self.shape_tools.reader.as_deref()?;
        self.doc
            .as_ref()?
            .mesh
            .readers
            .iter()
            .find(|r| r.name == name)
    }

    fn unique(&self, base: &str) -> String {
        let taken = |n: &str| {
            self.doc.as_ref().is_some_and(|d| {
                d.mesh.shapes.iter().any(|s| s.name == n)
                    || d.mesh.readers.iter().any(|r| r.name == n)
                    || d.rig
                        .as_ref()
                        .is_some_and(|r| r.skeleton.index_of(n).is_some())
            })
        };
        let mut name = base.to_string();
        let mut k = 2;
        while taken(&name) {
            name = format!("{base}{k}");
            k += 1;
        }
        name
    }

    /// A cone reader on the posed bone, centred where it points now (fully on
    /// within 15 degrees, off 30 further), driving a new shape to sculpt.
    pub fn new_cone_reader(&mut self) -> bool {
        let Some(bone) = self.reader_bone() else {
            self.warning("Pose a bone (or pick one) first: a cone reader reads where it points.");
            return false;
        };
        let locals = self.pose_locals();
        let (name, shape) = (
            self.unique(&format!("{bone}_cone")),
            self.unique(&format!("{bone}_cone_fix")),
        );
        let made = (name.clone(), shape.clone());
        let ok = self.reader_edit(|rig| {
            let i = rig
                .skeleton
                .index_of(&bone)
                .ok_or_else(|| anyhow::anyhow!("the rig has no bone '{bone}'"))?;
            let dir = locals.get(i).copied().unwrap_or(Quat::IDENTITY) * rig.skeleton.bones[i].direction();
            rig.mesh.shapes.push(Shape::from_dense(&made.1, &[]));
            readers::set_reader(
                rig,
                Reader {
                    name: made.0.clone(),
                    bone: bone.clone(),
                    kind: Kind::Cone(Cone {
                        direction: dir,
                        angle: 15.0,
                        falloff: 30.0,
                        away: false,
                    }),
                    shapes: vec![made.1.clone()],
                    helpers: Vec::new(),
                },
            )?;
            Ok(format!(
                "Made cone reader '{}' on {bone}: '{}' is on while it points this way. Sculpt it in this pose.",
                made.0, made.1
            ))
        });
        if ok {
            self.shape_tools.reader = Some(name);
            self.pick_shape(&shape);
            self.set_mesh_tool(super::MeshTool::Sculpt);
        }
        ok
    }

    /// An RBF reader on the posed bone with this pose as its first sample,
    /// turning a new shape fully on here.
    pub fn new_rbf_reader(&mut self) -> bool {
        let Some(bone) = self.reader_bone() else {
            self.warning("Pose a bone (or pick one) first: an RBF reader learns from poses.");
            return false;
        };
        let name = self.unique(&format!("{bone}_rbf"));
        let made = name.clone();
        let ok = self.reader_edit(|rig| {
            readers::set_reader(
                rig,
                Reader {
                    name: made.clone(),
                    bone: bone.clone(),
                    kind: Kind::Rbf(Rbf {
                        poses: Vec::new(),
                        radius: 0.0,
                        swing: false,
                    }),
                    shapes: Vec::new(),
                    helpers: Vec::new(),
                },
            )?;
            Ok(format!("Made RBF reader '{made}' on {bone}."))
        });
        if ok {
            self.shape_tools.reader = Some(name);
            self.add_pose_to_reader();
        }
        ok
    }

    /// Add the pose that shows as a sample of the picked RBF reader, with a
    /// new shape that is fully on there (and off at its other samples).
    pub fn add_pose_to_reader(&mut self) -> bool {
        let Some(r) = self.picked_reader().cloned() else {
            self.warning("Pick an RBF reader first.");
            return false;
        };
        if !matches!(r.kind, Kind::Rbf(_)) {
            self.warning("Poses go on RBF readers; a cone reader reads one direction.");
            return false;
        }
        let Some(i) = self
            .shape_rig()
            .and_then(|rig| rig.skeleton.index_of(&r.bone))
        else {
            return false;
        };
        let q = self.pose_locals().get(i).copied().unwrap_or(Quat::IDENTITY);
        if readers::turn_between(q, Quat::IDENTITY) < 0.5 {
            self.warning(format!(
                "Turn {} first: rest is a sample of every RBF reader already.",
                r.bone
            ));
            return false;
        }
        let shape = self.unique(&format!("{}_p", r.name));
        let made = shape.clone();
        let ok = self.reader_edit(|rig| {
            let k = readers::add_sample(rig, &r.name, q, &[(made.clone(), 1.0)])?;
            Ok(format!(
                "'{}' has pose {k}: '{made}' is fully on here. Sculpt it in this pose.",
                r.name
            ))
        });
        if ok {
            self.pick_shape(&shape);
            self.set_mesh_tool(super::MeshTool::Sculpt);
        }
        ok
    }

    /// A helper joint for the picked reader: at its bone's head under the
    /// bone's parent, skinned in its radius, fully moved where the reader is
    /// fully on. Move and scale it with the sliders.
    pub fn add_reader_helper(&mut self) -> bool {
        let Some(r) = self.picked_reader().cloned() else {
            self.warning("Pick a reader first.");
            return false;
        };
        let name = self.unique(&format!("{}_vol", r.bone));
        self.reader_edit(|rig| {
            let mut h = readers::default_helper(rig, &r, &name)?;
            h.across = 1.15;
            let on: Vec<usize> = match &r.kind {
                Kind::Rbf(b) => (1..=b.poses.len()).collect(),
                Kind::Cone(_) => Vec::new(),
            };
            readers::set_helper(rig, &r.name, h, &on)?;
            Ok(format!(
                "Added helper joint '{name}' under {}: skinned with the rest, it swells as '{}' comes on.",
                rig.skeleton.bone(&name).and_then(|b| b.parent).map_or("?".into(), |p| rig.skeleton.bones[p].name.clone()),
                r.name
            ))
        })
    }

    pub fn remove_reader(&mut self, name: &str) -> bool {
        let name = name.to_string();
        let ok = self.reader_edit(|rig| {
            readers::remove_reader(rig, &name)?;
            Ok(format!(
                "Removed reader '{name}'; its shapes keep their own weight."
            ))
        });
        if ok && self.shape_tools.reader.as_deref() == Some(name.as_str()) {
            self.shape_tools.reader = None;
        }
        ok
    }

    /// A reader's settings changed by a slider: applied without an undo step.
    fn tweak_reader(&mut self, r: Reader) {
        let Some(mut rig) = self.shape_rig() else {
            return;
        };
        match readers::set_reader(&mut rig, r) {
            Ok(()) => self.adopt_reader_rig(rig),
            Err(e) => self.fail(format!("{e:#}")),
        }
    }

    /// The Pose readers card, under Blendshapes.
    pub(super) fn readers_card(&mut self, ui: &mut Ui) {
        let Some(doc) = &self.doc else { return };
        let Some(rig) = doc.rig.clone() else { return };
        let list = doc.mesh.readers.clone();
        theme::label_caps(ui, "Pose readers");
        let mut action: Option<ReaderAction> = None;
        if list.is_empty() {
            ui.label(
                RichText::new(
                    "Cone readers turn shapes on as a bone points one way; RBF readers learn from poses. \
                     Pose a bone, then make one.",
                )
                .size(12.0)
                .color(TEXT_FAINT),
            );
        }
        for r in &list {
            let picked = self.shape_tools.reader.as_deref() == Some(r.name.as_str());
            let out = self.reader_outputs_now(r, &rig);
            ui.horizontal(|ui| {
                let label = RichText::new(format!("{}  ·{} on {}", r.name, r.kind_label(), r.bone))
                    .size(12.5)
                    .color(if picked { ACCENT } else { TEXT });
                if ui
                    .add(egui::Button::selectable(picked, label).truncate())
                    .clicked()
                {
                    action = Some(ReaderAction::Pick(r.name.clone()));
                }
            });
            let names = r
                .shapes
                .iter()
                .cloned()
                .chain(r.helpers.iter().map(|h| h.name.clone()));
            for (k, n) in names.enumerate() {
                let w = out.get(k).copied().unwrap_or(0.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&n).size(11.5).color(TEXT_DIM));
                    ui.add(
                        egui::ProgressBar::new(w)
                            .desired_width(90.0)
                            .text(format!("{w:.2}")),
                    );
                });
            }
        }
        ui.horizontal_wrapped(|ui| {
            let small = |t: &str| egui::Button::new(RichText::new(t).size(12.5)).corner_radius(8);
            if ui
                .add(small("Cone from pose"))
                .on_hover_text("A reader on the posed bone, on while it points this way, with a shape to sculpt.")
                .clicked()
            {
                action = Some(ReaderAction::Cone);
            }
            if ui
                .add(small("RBF from pose"))
                .on_hover_text("A reader that learns from poses of the posed bone; this pose is its first.")
                .clicked()
            {
                action = Some(ReaderAction::Rbf);
            }
        });
        if let Some(r) = self.picked_reader().cloned() {
            self.reader_section(ui, &r, &mut action);
        }
        match action {
            Some(ReaderAction::Pick(n)) => self.shape_tools.reader = Some(n),
            Some(ReaderAction::Cone) => {
                self.new_cone_reader();
            }
            Some(ReaderAction::Rbf) => {
                self.new_rbf_reader();
            }
            Some(ReaderAction::AddPose) => {
                self.add_pose_to_reader();
            }
            Some(ReaderAction::AddHelper) => {
                self.add_reader_helper();
            }
            Some(ReaderAction::Remove(n)) => {
                self.remove_reader(&n);
            }
            Some(ReaderAction::RemoveSample(k)) => {
                if let Some(name) = self.shape_tools.reader.clone() {
                    self.reader_edit(|rig| {
                        readers::remove_sample(rig, &name, k)?;
                        Ok(format!("Took pose {k} off '{name}'."))
                    });
                }
            }
            Some(ReaderAction::Tweak(r)) => self.tweak_reader(r),
            None => {}
        }
    }

    /// The picked reader's settings, samples and helpers.
    fn reader_section(&self, ui: &mut Ui, r: &Reader, action: &mut Option<ReaderAction>) {
        ui.add_space(4.0);
        theme::label_caps(ui, &format!("Reader {}", r.name));
        let mut edited = r.clone();
        let mut changed = false;
        match &mut edited.kind {
            Kind::Cone(c) => {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Angle").size(12.0).color(TEXT_DIM));
                    changed |= ui
                        .add(egui::Slider::new(&mut c.angle, 0.0..=90.0).suffix("°"))
                        .changed();
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Falloff").size(12.0).color(TEXT_DIM));
                    changed |= ui
                        .add(egui::Slider::new(&mut c.falloff, 1.0..=120.0).suffix("°"))
                        .changed();
                });
                changed |= ui
                    .checkbox(&mut c.away, "Away: on as it swings away")
                    .on_hover_text(
                        "Off inside the cone, on once the bone swings out of it in any direction.",
                    )
                    .changed();
            }
            Kind::Rbf(b) => {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Radius").size(12.0).color(TEXT_DIM));
                    changed |= ui
                        .add(egui::Slider::new(&mut b.radius, 0.0..=120.0).suffix("°"))
                        .on_hover_text("How far each pose reaches; 0 picks it from how far apart the poses are.")
                        .changed();
                });
                changed |= ui.checkbox(&mut b.swing, "Ignore twist").changed();
                let outputs: Vec<String> = r
                    .shapes
                    .iter()
                    .cloned()
                    .chain(r.helpers.iter().map(|h| h.name.clone()))
                    .collect();
                for (k, p) in b.poses.iter_mut().enumerate() {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(format!(
                                "Pose {} ({:.0}°)",
                                k + 1,
                                readers::turn_between(p.rotation, Quat::IDENTITY)
                            ))
                            .size(12.0)
                            .color(TEXT),
                        );
                        if ui.small_button("Remove").clicked() {
                            *action = Some(ReaderAction::RemoveSample(k + 1));
                        }
                    });
                    for (o, name) in outputs.iter().enumerate() {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(name).size(11.5).color(TEXT_DIM));
                            changed |= ui
                                .add(egui::Slider::new(&mut p.values[o], 0.0..=1.0))
                                .changed();
                        });
                    }
                }
            }
        }
        for h in &mut edited.helpers {
            ui.label(
                RichText::new(format!("Helper {} (under {})", h.name, h.parent))
                    .size(12.0)
                    .color(TEXT),
            );
            let mut cm = h.offset * 100.0;
            ui.horizontal(|ui| {
                ui.label(RichText::new("Move").size(11.5).color(TEXT_DIM));
                for v in [&mut cm.x, &mut cm.y, &mut cm.z] {
                    changed |= ui
                        .add(egui::DragValue::new(v).speed(0.1).suffix(" cm"))
                        .changed();
                }
            });
            h.offset = cm / 100.0;
            ui.horizontal(|ui| {
                ui.label(RichText::new("Along").size(11.5).color(TEXT_DIM));
                changed |= ui.add(egui::Slider::new(&mut h.along, 0.5..=2.0)).changed();
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("Across").size(11.5).color(TEXT_DIM));
                changed |= ui
                    .add(egui::Slider::new(&mut h.across, 0.5..=2.0))
                    .changed();
            });
        }
        ui.horizontal_wrapped(|ui| {
            let small = |t: &str| egui::Button::new(RichText::new(t).size(12.5)).corner_radius(8);
            if matches!(r.kind, Kind::Rbf(_))
                && ui
                    .add(small("Add pose to reader"))
                    .on_hover_text("This pose of the bone as a sample, with a new shape on here.")
                    .clicked()
            {
                *action = Some(ReaderAction::AddPose);
            }
            if ui
                .add(small("Add helper joint"))
                .on_hover_text("A joint at the bone's head that the reader moves and scales (volume at a bend).")
                .clicked()
            {
                *action = Some(ReaderAction::AddHelper);
            }
            if ui.add(small("Remove reader")).clicked() {
                *action = Some(ReaderAction::Remove(r.name.clone()));
            }
        });
        if changed && action.is_none() {
            *action = Some(ReaderAction::Tweak(edited));
        }
    }
}
