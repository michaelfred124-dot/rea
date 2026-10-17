//! Layout and drawing of the Hive Rig window. Everything here calls the
//! operations in the parent module; nothing here changes the rig directly.

use std::path::{Path, PathBuf};

use eframe::egui::{
    self, Align, Align2, Color32, FontId, Key, Layout, Margin, PointerButton, Pos2, Rect, RichText,
    Sense, Stroke, Ui, UiBuilder, Vec2,
};
use glam::Vec3;

use hive_rig::components::{self, Guides, Kind};
use hive_rig::skeleton::Side;

use super::{Doc, Drag, FingerSides, Mode, Studio, Tab, push_undo};
use crate::theme::{self, ACCENT, BORDER, CARD, CARD_HI, ERR, OK, TEXT, TEXT_DIM, TEXT_FAINT};
use crate::view::{self, Projector};

const PICK_RADIUS: f32 = 10.0;
const TOAST_SECONDS: f64 = 5.0;

fn kind_label(k: Kind) -> &'static str {
    match k {
        Kind::Root => "Root",
        Kind::Spine => "Spine",
        Kind::Head => "Head",
        Kind::Arm => "Arm",
        Kind::Leg => "Leg",
        Kind::Chain => "Chain",
    }
}

fn side_color(side: Side) -> Color32 {
    match side {
        Side::Center => theme::CENTER,
        Side::Left => theme::LEFT,
        Side::Right => theme::RIGHT,
    }
}

fn side_label(side: Side) -> &'static str {
    match side {
        Side::Center => "Centre",
        Side::Left => "Left",
        Side::Right => "Right",
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

/// Whether two layouts have identical components and guide positions.
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

/// Toast state kept in egui memory: which status it shows and since when.
#[derive(Clone, Copy, Default)]
struct Toast {
    serial: u64,
    since: f64,
}

impl Studio {
    pub fn ui(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        theme_once(&ctx);
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
        if ctx
            .input_mut(|i| i.consume_key(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, Key::Z))
            || ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, Key::Y))
        {
            self.redo();
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, Key::Z)) {
            self.undo();
        }

        egui::Panel::top("appbar")
            .exact_size(58.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::BG)
                    .inner_margin(Margin::symmetric(16, 0)),
            )
            .show(ui, |ui| self.app_bar(ui));
        egui::Panel::bottom("statusbar")
            .exact_size(28.0)
            .frame(
                egui::Frame::new()
                    .fill(theme::BG)
                    .inner_margin(Margin::symmetric(16, 0)),
            )
            .show(ui, |ui| self.status_bar(ui));
        if self.doc.is_some() {
            let side_frame = egui::Frame::new().fill(theme::BG).inner_margin(Margin {
                left: 12,
                right: 12,
                top: 4,
                bottom: 12,
            });
            egui::Panel::left("sidebar")
                .resizable(true)
                .default_size(318.0)
                .min_size(280.0)
                .frame(side_frame)
                .show(ui, |ui| self.sidebar(ui));
            egui::Panel::right("inspector")
                .resizable(true)
                .default_size(300.0)
                .min_size(270.0)
                .frame(side_frame)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink(false)
                        .show(ui, |ui| self.rig_panel(ui));
                });
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::BG).inner_margin(Margin {
                left: 0,
                right: 0,
                top: 4,
                bottom: 12,
            }))
            .show(ui, |ui| {
                if self.doc.is_some() {
                    self.viewport(ui);
                } else {
                    self.welcome(ui);
                }
            });
        self.toast(&ctx);
        self.drop_overlay(&ctx);
    }

    // ----------------------------------------------------------------------- app bar

    fn app_bar(&mut self, ui: &mut Ui) {
        ui.horizontal_centered(|ui| {
            let (r, _) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::hover());
            theme::logo(ui.painter(), r.center(), 14.0);
            ui.label(RichText::new("Hive Rig").size(18.0).strong().color(TEXT));
            if let Some(doc) = &self.doc {
                ui.label(RichText::new("/").size(18.0).color(TEXT_FAINT));
                ui.label(RichText::new(&doc.name).size(15.0).color(TEXT_DIM));
            }
            ui.add_space(12.0);
            self.file_menu(ui);

            // Right side, laid out right to left.
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                let has_doc = self.doc.is_some();
                if has_doc {
                    let out_dir = self
                        .doc
                        .as_ref()
                        .map(|d| d.out_dir.clone())
                        .unwrap_or_default();
                    if theme::secondary(ui, "Export", 0.0).clicked() {
                        self.do_export(out_dir);
                    }
                    let built = self.doc.as_ref().is_some_and(|d| d.rig.is_some());
                    let label = if built { "Rebuild Rig" } else { "Build Rig" };
                    if theme::primary(ui, label, 120.0, true).clicked() {
                        self.build();
                    }
                    ui.add_space(6.0);
                    if theme::icon(ui, "⟳", "Redo (Ctrl+Y)", self.can_redo()).clicked() {
                        self.redo();
                    }
                    if theme::icon(ui, "⟲", "Undo (Ctrl+Z)", self.can_undo()).clicked() {
                        self.undo();
                    }
                    ui.add_space(12.0);
                    // Centre: the steps from character to export.
                    let remaining = ui.available_width();
                    ui.allocate_ui_with_layout(
                        Vec2::new(remaining, 40.0),
                        Layout::centered_and_justified(egui::Direction::LeftToRight),
                        |ui| {
                            ui.horizontal_centered(|ui| self.steps(ui));
                        },
                    );
                }
            });
        });
    }

    fn file_menu(&mut self, ui: &mut Ui) {
        let has_doc = self.doc.is_some();
        ui.menu_button(RichText::new("File").color(TEXT_DIM), |ui| {
            ui.set_min_width(200.0);
            if ui.button("Open character or rig…").clicked() {
                ui.close();
                if let Some(p) = character_dialog().pick_file() {
                    self.open(&p);
                }
            }
            if ui.button("Demo character").clicked() {
                ui.close();
                self.load_demo();
            }
            ui.separator();
            if ui
                .add_enabled(has_doc, egui::Button::new("Load guides…"))
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
                .add_enabled(has_doc, egui::Button::new("Save guides…"))
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
            if ui
                .add_enabled(has_doc, egui::Button::new("Reset guides to the body"))
                .clicked()
            {
                ui.close();
                self.redetect();
            }
            ui.separator();
            if ui.button("Quit").clicked() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
    }

    fn steps(&mut self, ui: &mut Ui) {
        let Some(doc) = &self.doc else { return };
        let built = doc.rig.is_some();
        let exported = doc.exported;
        let steps = [
            ("Character", true, false),
            ("Guides", true, !built),
            ("Build", built, false),
            ("Export", exported, built && !exported),
        ];
        let mut clicked = None;
        for (i, (name, done, active)) in steps.into_iter().enumerate() {
            if i > 0 {
                let (r, _) = ui.allocate_exact_size(Vec2::new(22.0, 2.0), Sense::hover());
                let c = if done {
                    ACCENT.gamma_multiply(0.6)
                } else {
                    BORDER
                };
                ui.painter()
                    .line_segment([r.left_center(), r.right_center()], Stroke::new(2.0, c));
            }
            let (fill, text, ring) = if active {
                (ACCENT.gamma_multiply(0.15), ACCENT, ACCENT)
            } else if done {
                (Color32::TRANSPARENT, TEXT, ACCENT.gamma_multiply(0.5))
            } else {
                (Color32::TRANSPARENT, TEXT_FAINT, BORDER)
            };
            let mark = if done && !active {
                "✔".to_string()
            } else {
                (i + 1).to_string()
            };
            let b = egui::Button::new(
                RichText::new(format!("{mark}  {name}"))
                    .size(13.0)
                    .color(text),
            )
            .fill(fill)
            .stroke(Stroke::new(1.0, ring))
            .corner_radius(14)
            .min_size(Vec2::new(0.0, 28.0));
            if ui.add(b).clicked() {
                clicked = Some(i);
            }
        }
        match clicked {
            Some(0) => {
                if let Some(p) = character_dialog().pick_file() {
                    self.open(&p);
                }
            }
            Some(1) => self.mode = Mode::Guides,
            Some(2) => {
                self.build();
            }
            Some(3) => {
                let dir = self
                    .doc
                    .as_ref()
                    .map(|d| d.out_dir.clone())
                    .unwrap_or_default();
                self.do_export(dir);
            }
            _ => {}
        }
    }

    fn status_bar(&mut self, ui: &mut Ui) {
        ui.horizontal_centered(|ui| {
            theme::dot(ui, if self.error { ERR } else { OK });
            ui.add(
                egui::Label::new(RichText::new(&self.status).size(12.0).color(TEXT_DIM)).truncate(),
            );
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new("Ctrl+Z undo  ·  Ctrl+Y redo  ·  F frame  ·  Del remove")
                        .size(12.0)
                        .color(TEXT_FAINT),
                );
            });
        });
    }

    fn toast(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let id = egui::Id::unique("toast");
        let mut t: Toast = ctx.data(|d| d.get_temp(id)).unwrap_or_default();
        if t.serial != self.status_serial {
            t = Toast {
                serial: self.status_serial,
                since: now,
            };
            ctx.data_mut(|d| d.insert_temp(id, t));
        }
        let age = now - t.since;
        if self.status_serial == 0 || age > TOAST_SECONDS {
            return;
        }
        ctx.request_repaint_after(std::time::Duration::from_millis(200));
        let color = if self.error { ERR } else { OK };
        egui::Area::new(id)
            .anchor(Align2::CENTER_TOP, Vec2::new(0.0, 124.0))
            .order(egui::Order::Foreground)
            .interactable(false)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(CARD_HI)
                    .stroke(Stroke::new(1.0, color.gamma_multiply(0.6)))
                    .corner_radius(12)
                    .shadow(ui.visuals().popup_shadow)
                    .inner_margin(Margin::symmetric(14, 10))
                    .show(ui, |ui| {
                        ui.set_max_width(420.0);
                        ui.horizontal(|ui| {
                            theme::dot(ui, color);
                            ui.add(
                                egui::Label::new(RichText::new(&self.status).color(TEXT)).wrap(),
                            );
                        });
                    });
            });
    }

    fn drop_overlay(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.raw.hovered_files.is_empty()) {
            return;
        }
        let screen = ctx.content_rect();
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::unique("drop"),
        ));
        painter.rect_filled(screen, 0, Color32::from_black_alpha(170));
        let inner = screen.shrink(40.0);
        dashed_rect(&painter, inner, Stroke::new(2.0, ACCENT));
        painter.text(
            inner.center(),
            Align2::CENTER_CENTER,
            "Drop to open",
            FontId::proportional(30.0),
            TEXT,
        );
    }

    // ----------------------------------------------------------------------- welcome

    fn welcome(&mut self, ui: &mut Ui) {
        let rect = ui.max_rect();
        paint_backdrop(ui.painter(), rect);
        let width = 760f32.min(rect.width() - 48.0);
        let top = rect.top() + (rect.height() * 0.14).max(24.0);
        let area = Rect::from_min_size(
            Pos2::new(rect.center().x - width / 2.0, top),
            Vec2::new(width, rect.bottom() - top),
        );
        ui.scope_builder(UiBuilder::new().max_rect(area), |ui| {
            ui.vertical_centered(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::splat(72.0), Sense::hover());
                theme::logo(ui.painter(), r.center(), 34.0);
                ui.add_space(6.0);
                ui.label(RichText::new("Hive Rig").size(38.0).strong().color(TEXT));
                ui.label(
                    RichText::new(
                        "Place guides, build a game-ready rig, export to Blender, Maya and Unreal.",
                    )
                    .size(16.0)
                    .color(TEXT_DIM),
                );
            });
            ui.add_space(28.0);
            let gap = 16.0;
            let card_w = (width - gap) / 2.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                if big_card(
                    ui,
                    card_w,
                    Glyph::Folder,
                    "Open a character",
                    "Drop a .glb, .gltf or .obj file here, or click to browse.",
                    true,
                )
                .clicked()
                    && let Some(p) = character_dialog().pick_file()
                {
                    self.open(&p);
                }
                if big_card(
                    ui,
                    card_w,
                    Glyph::Logo,
                    "Try the demo",
                    "Rig the built-in test character to see how it works.",
                    false,
                )
                .clicked()
                {
                    self.load_demo();
                }
            });
            ui.add_space(30.0);
            theme::label_caps(ui, "How it works");
            ui.add_space(2.0);
            let step_w = (width - 3.0 * 12.0) / 4.0;
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                for (n, title, text) in [
                    (
                        "1",
                        "Load",
                        "A character in a T- or A-pose, facing forward.",
                    ),
                    (
                        "2",
                        "Place guides",
                        "Drag the joints. Add fingers, tails, ears, props.",
                    ),
                    ("3", "Build", "Bones, IK/FK controls and skin weights."),
                    ("4", "Export", "Blender, Maya and Unreal, with controls."),
                ] {
                    step_card(ui, step_w, n, title, text);
                }
            });
        });
    }

    // ----------------------------------------------------------------------- left sidebar

    fn sidebar(&mut self, ui: &mut Ui) {
        ui.add_space(4.0);
        let mut tab = self.tab;
        ui.horizontal(|ui| {
            theme::segmented(
                ui,
                &mut tab,
                &[
                    (Tab::Components, "Components"),
                    (Tab::Library, "Add components"),
                ],
            );
        });
        self.tab = tab;
        ui.add_space(6.0);
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| match self.tab {
                Tab::Components => self.components_tab(ui),
                Tab::Library => self.library_tab(ui),
            });
    }

    fn components_tab(&mut self, ui: &mut Ui) {
        let Some(doc) = &self.doc else { return };
        // Depth of each component in the attachment tree, for indenting.
        let owner = |bone: &str| -> Option<&str> {
            doc.preview
                .as_ref()
                .ok()?
                .skeleton
                .bone(bone)
                .map(|b| b.component.as_str())
        };
        let ids: Vec<&str> = doc
            .guides
            .components
            .iter()
            .map(|c| c.id.as_str())
            .collect();
        let mut depth = vec![0usize; ids.len()];
        for (i, c) in doc.guides.components.iter().enumerate() {
            if let Some(p) = c.parent.as_deref().and_then(owner)
                && let Some(j) = ids.iter().position(|id| *id == p)
                && j < i
            {
                depth[i] = depth[j] + 1;
            }
        }
        let rows: Vec<(String, Kind, Side, usize, usize)> = doc
            .guides
            .components
            .iter()
            .zip(&depth)
            .map(|(c, d)| (c.id.clone(), c.kind, c.side, c.guides.len(), *d))
            .collect();

        theme::card(ui, |ui| {
            theme::card_title(
                ui,
                "Rig components",
                Some(RichText::new(format!("{}", rows.len())).color(TEXT_FAINT)),
            );
            ui.add_space(2.0);
            ui.spacing_mut().item_spacing.y = 2.0;
            for (i, (id, kind, side, _, depth)) in rows.iter().enumerate() {
                let selected = self.selected.is_some_and(|(c, _)| c == i);
                if component_row(ui, id, *kind, *side, (*depth).min(4), selected).clicked() {
                    self.selected = Some((i, None));
                    self.mode = Mode::Guides;
                }
            }
        });
        ui.add_space(4.0);
        theme::card(ui, |ui| {
            theme::card_title(ui, "Symmetry", None);
            ui.horizontal(|ui| {
                theme::pill(ui, &mut self.symmetry, "Mirror moves")
                    .on_hover_text("Moving a left or right guide moves its twin too");
            });
            ui.horizontal(|ui| {
                let w = (ui.available_width() - 8.0) / 2.0;
                if theme::secondary(ui, "Left to right", w).clicked() {
                    self.mirror(Side::Left);
                }
                if theme::secondary(ui, "Right to left", w).clicked() {
                    self.mirror(Side::Right);
                }
            });
        });
        if let Some((c, guide)) = self.selected {
            ui.add_space(4.0);
            self.component_inspector(ui, c, guide);
        } else {
            ui.add_space(8.0);
            ui.label(
                RichText::new("Select a component, or click a guide in the view, to edit it.")
                    .color(TEXT_FAINT),
            );
        }
    }

    fn component_inspector(&mut self, ui: &mut Ui, c: usize, guide: Option<usize>) {
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

        theme::card(ui, |ui| {
            ui.horizontal(|ui| {
                theme::dot(ui, side_color(comp.side));
                ui.label(RichText::new(&comp.id).size(16.0).strong().color(TEXT));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    theme::badge(ui, kind_label(comp.kind), ACCENT);
                    theme::badge(ui, side_label(comp.side), side_color(comp.side));
                });
            });
            ui.label(RichText::new(format!("{} bones", own.len())).color(TEXT_DIM));
            ui.add_space(4.0);

            if let Some(parent) = &comp.parent {
                let mut chosen = parent.clone();
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Attached to").color(TEXT_DIM));
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
                        ui.label(RichText::new("Spine joints").color(TEXT_DIM));
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
                    ui.horizontal(|ui| {
                        if theme::pill(ui, &mut twist, "Twist bones").clicked() {
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
                    });
                }
                _ => {}
            }

            ui.add_space(6.0);
            theme::label_caps(ui, "Guides");
            egui::Grid::new("guides")
                .num_columns(4)
                .spacing([4.0, 4.0])
                .min_col_width(0.0)
                .show(ui, |ui| {
                    ui.label("");
                    for axis in ["X", "Y", "Z"] {
                        ui.label(RichText::new(axis).size(11.0).color(TEXT_FAINT));
                    }
                    ui.end_row();
                    for (gi, g) in comp.guides.iter().enumerate() {
                        let is_sel = guide == Some(gi);
                        if ui
                            .selectable_label(is_sel, RichText::new(&g.name).size(13.0))
                            .clicked()
                        {
                            self.selected = Some((c, Some(gi)));
                            self.mode = Mode::Guides;
                        }
                        let mut p = g.position;
                        let mut changed = false;
                        for v in [&mut p.x, &mut p.y, &mut p.z] {
                            changed |= ui
                                .add_sized(
                                    [62.0, 24.0],
                                    egui::DragValue::new(v).speed(0.002).max_decimals(3),
                                )
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
            if comp.kind != Kind::Root {
                ui.add_space(6.0);
                let remove = egui::Button::new(RichText::new("Remove component").color(ERR))
                    .fill(ERR.gamma_multiply(0.1))
                    .stroke(Stroke::new(1.0, ERR.gamma_multiply(0.4)))
                    .corner_radius(10)
                    .min_size(Vec2::new(ui.available_width(), 32.0));
                if ui.add(remove).clicked() {
                    self.remove_selected();
                }
            }
        });
    }

    /// Record an undo step for a guide move unless one is already in progress.
    fn begin_change(&mut self) {
        if self.drag.is_none()
            && let Some(doc) = &mut self.doc
            && doc.undo.last().is_none_or(|u| !same_layout(u, &doc.guides))
        {
            push_undo(&mut doc.undo, &doc.guides);
            doc.redo.clear();
        }
    }

    fn library_tab(&mut self, ui: &mut Ui) {
        let w = ((ui.available_width() - 14.0) / 2.0).floor();
        let tiles: [LibraryItem; 6] = [
            (
                TileIcon::Fingers,
                "Fingers",
                "Five fingers per hand",
                Studio::add_fingers,
            ),
            (
                TileIcon::Tail,
                "Tail",
                "Behind the pelvis",
                Studio::add_tail,
            ),
            (
                TileIcon::Ears,
                "Ears",
                "A pair on the head",
                Studio::add_ears,
            ),
            (
                TileIcon::Ponytail,
                "Ponytail",
                "Hair from the head",
                Studio::add_ponytail,
            ),
            (TileIcon::Prop, "Prop", "Weapon socket", Studio::add_prop),
            (
                TileIcon::Chain,
                "Chain",
                "FK chain on any bone",
                Studio::add_chain,
            ),
        ];
        theme::label_caps(ui, "Premade components");
        for row in tiles.chunks(2) {
            ui.horizontal(|ui| {
                for (icon, title, text, add) in row {
                    if library_tile(ui, w, *icon, title, text).clicked() {
                        add(self);
                        self.mode = Mode::Guides;
                    }
                }
            });
        }
        ui.add_space(6.0);
        theme::card(ui, |ui| {
            theme::card_title(ui, "Options", None);
            ui.horizontal(|ui| {
                ui.label(RichText::new("Fingers on").color(TEXT_DIM));
                theme::segmented(
                    ui,
                    &mut self.finger_sides,
                    &[
                        (FingerSides::Both, "Both"),
                        (FingerSides::Left, "Left"),
                        (FingerSides::Right, "Right"),
                    ],
                );
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("Tail bones").color(TEXT_DIM));
                ui.add(egui::DragValue::new(&mut self.tail_joints).range(2..=24));
            });
        });
        ui.add_space(4.0);
        let bones = self.doc.as_ref().map(Doc::bone_names).unwrap_or_default();
        if !bones.contains(&self.chain_parent) {
            self.chain_parent = bones
                .iter()
                .find(|b| b.as_str() == "head")
                .or(bones.first())
                .cloned()
                .unwrap_or_default();
        }
        theme::card(ui, |ui| {
            theme::card_title(ui, "Custom chain", None);
            egui::Grid::new("chain")
                .num_columns(2)
                .spacing([10.0, 8.0])
                .show(ui, |ui| {
                    ui.label(RichText::new("On bone").color(TEXT_DIM));
                    egui::ComboBox::from_id_salt("chain_parent")
                        .selected_text(self.chain_parent.as_str())
                        .height(320.0)
                        .show_ui(ui, |ui| {
                            for b in &bones {
                                ui.selectable_value(&mut self.chain_parent, b.clone(), b);
                            }
                        });
                    ui.end_row();
                    ui.label(RichText::new("Bones").color(TEXT_DIM));
                    ui.add(egui::DragValue::new(&mut self.chain_joints).range(1..=24));
                    ui.end_row();
                    ui.label(RichText::new("Name").color(TEXT_DIM));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.chain_id)
                            .hint_text("automatic")
                            .desired_width(150.0),
                    );
                    ui.end_row();
                });
            ui.add_space(4.0);
            if theme::primary(ui, "Add chain", ui.available_width(), true).clicked() {
                self.add_chain();
                self.mode = Mode::Guides;
            }
        });
    }

    // ----------------------------------------------------------------------- right panel

    fn rig_panel(&mut self, ui: &mut Ui) {
        ui.add_space(4.0);
        let Some(doc) = &self.doc else { return };
        let built = doc.rig.is_some();
        let summary = doc.preview.as_ref().ok().map(|b| {
            (
                b.skeleton.bones.len(),
                b.controls.controls.len(),
                b.controls.ik_chains.len(),
            )
        });
        let preview_error = doc.preview.as_ref().err().cloned();
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

        theme::card(ui, |ui| {
            let badge = if preview_error.is_some() {
                RichText::new("Guides need fixing").color(ERR)
            } else if built {
                RichText::new("Built").color(OK)
            } else {
                RichText::new("Not built").color(TEXT_FAINT)
            };
            theme::card_title(ui, "Rig", Some(badge.size(12.0)));
            if let Some(e) = &preview_error {
                ui.label(RichText::new(e).color(ERR));
            }
            if let Some((bones, controls, ik)) = summary {
                let w = (ui.available_width() - 16.0) / 3.0;
                ui.horizontal(|ui| {
                    theme::stat(ui, bones, "bones", w);
                    theme::stat(ui, controls, "controls", w);
                    theme::stat(ui, ik, "IK limbs", w);
                });
            }
            ui.add_space(4.0);
            let label = if built { "Rebuild rig" } else { "Build rig" };
            if theme::primary(ui, label, ui.available_width(), preview_error.is_none()).clicked() {
                self.build();
            }
            if !built {
                ui.label(
                    RichText::new("Builds the bones, controls and skin weights.")
                        .size(12.0)
                        .color(TEXT_FAINT),
                );
            }
        });

        ui.add_space(4.0);
        theme::card(ui, |ui| {
            theme::card_title(ui, "View", None);
            let mut mode = self.mode;
            theme::segmented(
                ui,
                &mut mode,
                &[
                    (Mode::Guides, "Guides"),
                    (Mode::Skeleton, "Skeleton"),
                    (Mode::Rig, "Controls"),
                ],
            );
            self.mode = mode;
            ui.horizontal(|ui| {
                theme::pill(ui, &mut self.show_mesh, "Mesh");
                theme::pill(ui, &mut self.xray, "X-ray");
            });
        });

        ui.add_space(4.0);
        theme::card(ui, |ui| {
            theme::card_title(ui, "Skin weights", None);
            if deform.is_empty() {
                ui.label(
                    RichText::new("Build the rig to paint each bone's weights on the mesh.")
                        .color(TEXT_FAINT),
                );
            } else {
                egui::ComboBox::from_id_salt("weights")
                    .width(ui.available_width() - 8.0)
                    .selected_text(self.weights_bone.as_deref().unwrap_or("Off"))
                    .height(360.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.weights_bone, None, "Off");
                        for b in &deform {
                            ui.selectable_value(&mut self.weights_bone, Some(b.clone()), b);
                        }
                    });
                weight_legend(ui);
            }
        });

        ui.add_space(4.0);
        let Some(doc) = &mut self.doc else { return };
        let mut export_dir = None;
        let mut choose = false;
        theme::card(ui, |ui| {
            theme::card_title(ui, "Export", None);
            let w = (ui.available_width() - 16.0) / 3.0;
            ui.horizontal(|ui| {
                for (i, (name, note)) in
                    [("Blender", "3.6+"), ("Maya", "2022+"), ("Unreal", "5.2+")]
                        .into_iter()
                        .enumerate()
                {
                    if target_tile(ui, w, name, note, self.targets[i]).clicked() {
                        self.targets[i] = !self.targets[i];
                    }
                }
            });
            ui.horizontal(|ui| {
                ui.label(RichText::new("Name").color(TEXT_DIM));
                let mut name = doc.name.clone();
                if ui
                    .add(egui::TextEdit::singleline(&mut name).desired_width(f32::INFINITY))
                    .changed()
                {
                    doc.name = hive_rig::export::sanitize_name(&name);
                }
            });
            ui.add(
                egui::Label::new(
                    RichText::new(doc.out_dir.display().to_string())
                        .size(11.5)
                        .color(TEXT_FAINT),
                )
                .truncate(),
            )
            .on_hover_text(doc.out_dir.display().to_string());
            let any = self.targets.iter().any(|t| *t);
            if theme::primary(ui, "Export rig", ui.available_width(), any).clicked() {
                export_dir = Some(doc.out_dir.clone());
            }
            ui.horizontal(|ui| {
                let w = (ui.available_width() - 8.0) / 2.0;
                if theme::secondary(ui, "Choose folder…", w).clicked() {
                    choose = true;
                }
                let dir = doc.out_dir.clone();
                if ui
                    .add_enabled(
                        dir.is_dir(),
                        egui::Button::new("Open folder")
                            .corner_radius(10)
                            .min_size(Vec2::new(w, 34.0)),
                    )
                    .clicked()
                {
                    open_folder(&dir);
                }
            });
            if doc.exported {
                ui.label(RichText::new("Exported").color(OK));
            }
        });
        if choose {
            let start = self
                .doc
                .as_ref()
                .map(|d| d.out_dir.clone())
                .unwrap_or_default();
            if let Some(dir) = rfd::FileDialog::new()
                .set_directory(start.parent().unwrap_or(&start))
                .pick_folder()
            {
                export_dir = Some(dir);
            }
        }
        if let Some(dir) = export_dir {
            self.do_export(dir);
        }

        ui.add_space(4.0);
        theme::card(ui, |ui| {
            egui::CollapsingHeader::new(RichText::new("Using the export").strong()).show(ui, |ui| {
                for (app, how) in [
                    ("Blender", "Scripting tab > Open the _blender.py > Run. Edit guides and rebuild later in the Hive tab (N panel)."),
                    ("Maya", "Script Editor > Python > load the _maya.py > Execute. The Hive Rig window rebuilds it."),
                    ("Unreal", "Enable the Python and Control Rig plugins, then Tools > Execute Python Script > the _unreal.py."),
                ] {
                    ui.label(RichText::new(app).strong().color(TEXT));
                    ui.label(RichText::new(how).color(TEXT_DIM));
                }
                ui.label(RichText::new("Keep hive-rig.exe next to HiveRig.exe: Blender and Maya call it to rebuild.").color(TEXT_FAINT));
            });
        });
    }

    // ----------------------------------------------------------------------- viewport

    fn frame_camera(&mut self) {
        if let Some(doc) = &self.doc {
            let (min, max) = doc.mesh.bounds();
            self.camera.frame(min, max);
        }
    }

    fn viewport(&mut self, ui: &mut Ui) {
        let outer = ui.max_rect();
        let rect = outer;
        let (response, painter) = ui.allocate_painter(rect.size(), Sense::click_and_drag());
        let rect = response.rect;
        let painter = painter.with_clip_rect(rect);

        let typing = ui.ctx().egui_wants_keyboard_input();
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta().y);
            if scroll != 0.0 {
                self.camera.zoom(scroll);
            }
        }
        if !typing {
            if ui.input(|i| i.key_pressed(Key::F)) {
                self.frame_camera();
            }
            if ui.input(|i| i.key_pressed(Key::Delete)) {
                self.remove_selected();
            }
        }
        let pr = self.camera.projector(rect);
        let pointer = response.interact_pointer_pos().or(response.hover_pos());
        let press = ui.input(|i| i.pointer.press_origin());

        // Pick where the button went down: a drag only starts once the pointer has moved.
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
        if response.double_clicked() {
            self.frame_camera();
        }

        let pr = self.camera.projector(rect);
        paint_backdrop(&painter, rect);
        if let Some(doc) = &self.doc {
            let (min, max) = doc.mesh.bounds();
            paint_grid(&painter, &pr, min.y, (max - min).max_element().max(0.5));
        }
        self.paint(&painter, &pr, pointer);
        paint_axes(
            &painter,
            &pr,
            Pos2::new(rect.left() + 44.0, rect.bottom() - 44.0),
        );

        // Floating toolbars over the view.
        let bar = Rect::from_min_size(
            rect.left_top() + Vec2::new(14.0, 12.0),
            Vec2::new(rect.width() - 28.0, 40.0),
        );
        ui.scope_builder(UiBuilder::new().max_rect(bar), |ui| {
            ui.horizontal(|ui| {
                floating(ui, |ui| {
                    let mut mode = self.mode;
                    theme::segmented(
                        ui,
                        &mut mode,
                        &[
                            (Mode::Guides, "Guides"),
                            (Mode::Skeleton, "Skeleton"),
                            (Mode::Rig, "Controls"),
                        ],
                    );
                    self.mode = mode;
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    floating(ui, |ui| {
                        if theme::icon(ui, "⛶", "Frame the character (F)", true).clicked() {
                            self.frame_camera();
                        }
                        for (name, yaw, pitch) in [
                            ("Persp", 0.55, 0.22),
                            ("Side", std::f32::consts::FRAC_PI_2, 0.05),
                            ("Front", 0.0, 0.05),
                        ] {
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new(name).size(13.0).color(TEXT_DIM),
                                    )
                                    .frame_when_inactive(false)
                                    .corner_radius(8),
                                )
                                .clicked()
                            {
                                self.camera.yaw = yaw;
                                self.camera.pitch = pitch;
                            }
                        }
                    });
                });
            });
        });
        let hint = match self.mode {
            Mode::Guides => {
                "Drag a guide to move a joint  ·  left-drag to orbit  ·  right-drag to pan  ·  scroll to zoom"
            }
            _ => {
                "Left-drag to orbit  ·  right-drag to pan  ·  scroll to zoom  ·  switch to Guides to edit"
            }
        };
        painter.text(
            Pos2::new(rect.left() + 92.0, rect.bottom() - 16.0),
            Align2::LEFT_BOTTOM,
            hint,
            FontId::proportional(12.0),
            TEXT_FAINT,
        );
    }

    fn pick_guide(&self, pr: &Projector, at: Pos2) -> Option<(usize, usize)> {
        if self.mode != Mode::Guides {
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
            let alpha = if self.xray { 0.3 } else { 1.0 };
            let base = Color32::from_rgb(168, 172, 182);
            view::draw_mesh(painter, pr, &doc.mesh, base, colors.as_deref(), alpha);
        }
        let Ok(built) = &doc.preview else { return };
        let bones = &built.skeleton.bones;

        if self.mode == Mode::Rig {
            for c in &built.controls.controls {
                view::ring(
                    painter,
                    pr,
                    c.position,
                    c.axis,
                    c.size.max(0.01),
                    Stroke::new(2.0, rgb(c.color)),
                );
            }
        }
        let bone_alpha = if self.mode == Mode::Guides { 0.45 } else { 1.0 };
        for b in bones {
            if self.mode == Mode::Rig && !b.deform {
                continue;
            }
            let (Some(h), Some(t)) = (pr.pos(b.head), pr.pos(b.tail)) else {
                continue;
            };
            let color = if weights.as_deref() == Some(b.name.as_str()) {
                ACCENT
            } else if b.deform {
                Color32::from_rgb(226, 228, 236)
            } else {
                Color32::from_rgb(130, 134, 150)
            };
            bone_shape(painter, h, t, color.gamma_multiply(bone_alpha));
        }
        if self.mode != Mode::Guides {
            return;
        }
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
                painter.extend(egui::Shape::dashed_line(
                    &[ph, *first],
                    Stroke::new(1.0, TEXT_FAINT),
                    4.0,
                    4.0,
                ));
            }
            let line = Stroke::new(
                if comp_sel { 2.5 } else { 1.5 },
                color.gamma_multiply(if comp_sel { 1.0 } else { 0.65 }),
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
                    7.5
                } else if comp_sel {
                    6.0
                } else {
                    5.0
                };
                if sel || hot {
                    painter.circle_filled(*p, r + 5.0, color.gamma_multiply(0.25));
                }
                painter.circle_filled(*p, r, color);
                painter.circle_stroke(
                    *p,
                    r,
                    Stroke::new(
                        if sel { 2.0 } else { 1.2 },
                        if sel {
                            Color32::WHITE
                        } else {
                            Color32::from_black_alpha(200)
                        },
                    ),
                );
                if sel || hot {
                    label_chip(
                        painter,
                        *p + Vec2::new(12.0, -12.0),
                        &format!("{}  ·  {}", c.id, c.guides[gi].name),
                    );
                }
            }
        }
    }
}

// --------------------------------------------------------------------------- widgets

fn theme_once(ctx: &egui::Context) {
    let id = egui::Id::unique("hive_theme");
    if !ctx.data(|d| d.get_temp::<bool>(id).unwrap_or(false)) {
        theme::apply(ctx);
        ctx.data_mut(|d| d.insert_temp(id, true));
    }
}

fn component_row(
    ui: &mut Ui,
    id: &str,
    kind: Kind,
    side: Side,
    depth: usize,
    selected: bool,
) -> egui::Response {
    let height = 30.0;
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::click());
    let painter = ui.painter();
    if selected {
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
    let x = rect.left() + 10.0 + depth as f32 * 14.0;
    if depth > 0 {
        let c = BORDER;
        painter.line_segment(
            [
                Pos2::new(x - 8.0, rect.top()),
                Pos2::new(x - 8.0, rect.center().y),
            ],
            Stroke::new(1.0, c),
        );
        painter.line_segment(
            [
                Pos2::new(x - 8.0, rect.center().y),
                Pos2::new(x - 2.0, rect.center().y),
            ],
            Stroke::new(1.0, c),
        );
    }
    painter.circle_filled(Pos2::new(x + 4.0, rect.center().y), 4.0, side_color(side));
    painter.text(
        Pos2::new(x + 16.0, rect.center().y),
        Align2::LEFT_CENTER,
        id,
        FontId::proportional(14.0),
        if selected {
            TEXT
        } else {
            Color32::from_rgb(205, 209, 218)
        },
    );
    painter.text(
        Pos2::new(rect.right() - 10.0, rect.center().y),
        Align2::RIGHT_CENTER,
        kind_label(kind),
        FontId::proportional(12.0),
        TEXT_FAINT,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn library_tile(
    ui: &mut Ui,
    width: f32,
    icon: TileIcon,
    title: &str,
    text: &str,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 100.0), Sense::click());
    let hovered = response.hovered();
    let painter = ui.painter();
    painter.rect_filled(rect, 12, if hovered { CARD_HI } else { CARD });
    painter.rect_stroke(
        rect,
        12,
        Stroke::new(
            1.0,
            if hovered {
                ACCENT.gamma_multiply(0.7)
            } else {
                BORDER
            },
        ),
        egui::StrokeKind::Inside,
    );
    let icon_center = rect.left_top() + Vec2::new(28.0, 28.0);
    painter.circle_filled(icon_center, 17.0, ACCENT.gamma_multiply(0.14));
    tile_icon(painter, icon_center, icon);
    painter.text(
        rect.right_top() + Vec2::new(-12.0, 14.0),
        Align2::RIGHT_TOP,
        "+",
        FontId::proportional(20.0),
        if hovered { ACCENT } else { TEXT_FAINT },
    );
    painter.text(
        rect.left_top() + Vec2::new(12.0, 54.0),
        Align2::LEFT_TOP,
        title,
        FontId::proportional(15.0),
        TEXT,
    );
    let galley = painter.layout(
        text.to_string(),
        FontId::proportional(11.5),
        TEXT_DIM,
        width - 24.0,
    );
    painter.galley(rect.left_top() + Vec2::new(12.0, 74.0), galley, TEXT_DIM);
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(format!("Add {title}"))
}

/// A premade component in the library: icon, title, description, and how to add it.
type LibraryItem = (TileIcon, &'static str, &'static str, fn(&mut Studio));

#[derive(Clone, Copy)]
enum TileIcon {
    Fingers,
    Tail,
    Ears,
    Ponytail,
    Prop,
    Chain,
}

/// Small line icons for the component library, drawn so they look the same everywhere.
fn tile_icon(painter: &egui::Painter, c: Pos2, icon: TileIcon) {
    let s = Stroke::new(2.0, ACCENT);
    let p = |x: f32, y: f32| c + Vec2::new(x, y);
    match icon {
        TileIcon::Fingers => {
            for (x, top) in [(-4.5, -8.0), (-1.5, -9.5), (1.5, -9.0), (4.5, -7.0)] {
                painter.line_segment([p(x, 1.0), p(x, top)], s);
            }
            painter.line_segment([p(-7.5, 2.0), p(-5.0, 7.0)], s);
            painter.line_segment([p(-5.0, 7.0), p(5.0, 7.0)], s);
            painter.line_segment([p(-5.5, 1.0), p(5.5, 1.0)], s);
        }
        TileIcon::Tail => {
            let pts: Vec<Pos2> = (0..=16)
                .map(|i| {
                    let t = i as f32 / 16.0;
                    p(
                        -8.0 + 16.0 * t,
                        5.0 - 10.0 * t * t + 3.0 * (t * 9.0).sin() * t,
                    )
                })
                .collect();
            painter.add(egui::Shape::line(pts, s));
        }
        TileIcon::Ears => {
            for sx in [-1.0f32, 1.0] {
                painter.add(egui::Shape::closed_line(
                    vec![p(2.0 * sx, 6.0), p(8.0 * sx, -8.0), p(9.0 * sx, 4.0)],
                    s,
                ));
            }
        }
        TileIcon::Ponytail => {
            painter.circle_stroke(p(-3.0, -3.0), 5.0, s);
            let pts: Vec<Pos2> = (0..=12)
                .map(|i| {
                    let t = i as f32 / 12.0;
                    p(1.0 + 7.0 * t, -1.0 + 10.0 * t + 1.5 * (t * 12.0).sin())
                })
                .collect();
            painter.add(egui::Shape::line(pts, s));
        }
        TileIcon::Prop => {
            painter.line_segment([p(-7.0, 7.0), p(8.0, -8.0)], s);
            painter.line_segment([p(-6.0, 0.0), p(0.0, 6.0)], s);
        }
        TileIcon::Chain => {
            for x in [-6.0f32, 0.0, 6.0] {
                painter.circle_stroke(p(x, 0.0), 3.5, s);
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Glyph {
    Folder,
    Logo,
}

fn folder_icon(painter: &egui::Painter, c: Pos2, color: Color32) {
    let body = Rect::from_center_size(c + Vec2::new(0.0, 2.0), Vec2::new(26.0, 18.0));
    let tab = Rect::from_min_size(body.left_top() - Vec2::new(0.0, 4.0), Vec2::new(11.0, 6.0));
    painter.rect_filled(tab, 2, color.gamma_multiply(0.8));
    painter.rect_filled(body, 3, color);
    painter.line_segment(
        [
            Pos2::new(c.x, body.top() + 5.0),
            Pos2::new(c.x, body.bottom() - 4.0),
        ],
        Stroke::new(2.0, theme::ACCENT_TEXT),
    );
    painter.line_segment(
        [
            Pos2::new(c.x - 4.0, body.top() + 9.0),
            Pos2::new(c.x, body.top() + 5.0),
        ],
        Stroke::new(2.0, theme::ACCENT_TEXT),
    );
    painter.line_segment(
        [
            Pos2::new(c.x + 4.0, body.top() + 9.0),
            Pos2::new(c.x, body.top() + 5.0),
        ],
        Stroke::new(2.0, theme::ACCENT_TEXT),
    );
}

fn target_tile(ui: &mut Ui, width: f32, name: &str, note: &str, on: bool) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 58.0), Sense::click());
    let painter = ui.painter();
    let (fill, stroke) = if on {
        (ACCENT.gamma_multiply(0.12), ACCENT)
    } else if response.hovered() {
        (CARD_HI, BORDER)
    } else {
        (Color32::from_rgb(20, 23, 30), BORDER)
    };
    painter.rect_filled(rect, 10, fill);
    painter.rect_stroke(rect, 10, Stroke::new(1.0, stroke), egui::StrokeKind::Inside);
    let check = rect.right_top() + Vec2::new(-12.0, 12.0);
    if on {
        painter.circle_filled(check, 6.0, ACCENT);
    } else {
        painter.circle_stroke(check, 6.0, Stroke::new(1.0, TEXT_FAINT));
    }
    painter.text(
        rect.left_top() + Vec2::new(10.0, 12.0),
        Align2::LEFT_TOP,
        name,
        FontId::proportional(14.0),
        if on { TEXT } else { TEXT_DIM },
    );
    painter.text(
        rect.left_top() + Vec2::new(10.0, 34.0),
        Align2::LEFT_TOP,
        note,
        FontId::proportional(11.5),
        TEXT_FAINT,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn big_card(
    ui: &mut Ui,
    width: f32,
    glyph: Glyph,
    title: &str,
    text: &str,
    drop_zone: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, 170.0), Sense::click());
    let hovered = response.hovered();
    let painter = ui.painter();
    painter.rect_filled(rect, 16, if hovered { CARD_HI } else { CARD });
    if drop_zone {
        dashed_rect(
            painter,
            rect.shrink(8.0),
            Stroke::new(
                1.5,
                if hovered {
                    ACCENT
                } else {
                    Color32::from_rgb(64, 72, 88)
                },
            ),
        );
    } else {
        painter.rect_stroke(
            rect,
            16,
            Stroke::new(
                1.0,
                if hovered {
                    ACCENT.gamma_multiply(0.7)
                } else {
                    BORDER
                },
            ),
            egui::StrokeKind::Inside,
        );
    }
    let c = Pos2::new(rect.center().x, rect.top() + 52.0);
    painter.circle_filled(c, 26.0, ACCENT.gamma_multiply(0.15));
    match glyph {
        Glyph::Folder => folder_icon(painter, c, ACCENT),
        Glyph::Logo => theme::logo(painter, c, 15.0),
    }
    painter.text(
        Pos2::new(rect.center().x, rect.top() + 100.0),
        Align2::CENTER_TOP,
        title,
        FontId::proportional(18.0),
        TEXT,
    );
    let galley = painter.layout(
        text.to_string(),
        FontId::proportional(13.0),
        TEXT_DIM,
        width - 60.0,
    );
    let pos = Pos2::new(rect.center().x - galley.size().x / 2.0, rect.top() + 128.0);
    painter.galley(pos, galley, TEXT_DIM);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn step_card(ui: &mut Ui, width: f32, n: &str, title: &str, text: &str) {
    egui::Frame::new()
        .fill(Color32::from_rgb(19, 22, 28))
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(12)
        .inner_margin(12)
        .show(ui, |ui| {
            ui.set_width(width - 26.0);
            ui.set_min_height(70.0);
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    theme::badge(ui, n, ACCENT);
                    ui.label(RichText::new(title).strong().color(TEXT));
                });
                ui.label(RichText::new(text).size(12.0).color(TEXT_DIM));
            });
        });
}

/// A translucent floating group of controls over the viewport.
fn floating(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(22, 25, 32, 230))
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(12)
        .inner_margin(Margin::same(4))
        .shadow(egui::Shadow {
            offset: [0, 4],
            blur: 14,
            spread: 0,
            color: Color32::from_black_alpha(90),
        })
        .show(ui, |ui| {
            ui.horizontal(add);
        });
}

fn weight_legend(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 10.0), Sense::hover());
    let mut mesh = egui::Mesh::default();
    let n = 24;
    for i in 0..=n {
        let t = i as f32 / n as f32;
        let x = rect.left() + rect.width() * t;
        let c = view::weight_color(t);
        mesh.colored_vertex(Pos2::new(x, rect.top()), c);
        mesh.colored_vertex(Pos2::new(x, rect.bottom()), c);
        if i > 0 {
            let k = (i as u32) * 2;
            mesh.add_triangle(k - 2, k - 1, k);
            mesh.add_triangle(k - 1, k, k + 1);
        }
    }
    ui.painter().add(egui::Shape::mesh(mesh));
    ui.horizontal(|ui| {
        ui.label(RichText::new("0").size(11.0).color(TEXT_FAINT));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(RichText::new("1").size(11.0).color(TEXT_FAINT));
        });
    });
}

fn label_chip(painter: &egui::Painter, at: Pos2, text: &str) {
    let galley = painter.layout_no_wrap(text.to_string(), FontId::proportional(12.5), TEXT);
    let rect = Rect::from_min_size(
        at - Vec2::new(0.0, galley.size().y + 8.0),
        galley.size() + Vec2::new(16.0, 8.0),
    );
    painter.rect_filled(rect, 7, Color32::from_rgba_unmultiplied(14, 16, 20, 235));
    painter.rect_stroke(rect, 7, Stroke::new(1.0, BORDER), egui::StrokeKind::Inside);
    painter.galley(rect.min + Vec2::new(8.0, 4.0), galley, TEXT);
}

fn dashed_rect(painter: &egui::Painter, rect: Rect, stroke: Stroke) {
    let pts = [
        rect.left_top(),
        rect.right_top(),
        rect.right_bottom(),
        rect.left_bottom(),
        rect.left_top(),
    ];
    painter.extend(egui::Shape::dashed_line(&pts, stroke, 8.0, 6.0));
}

/// A soft vertical gradient behind the character.
fn paint_backdrop(painter: &egui::Painter, rect: Rect) {
    let top = Color32::from_rgb(34, 38, 47);
    let bottom = Color32::from_rgb(17, 19, 24);
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(rect.left_top(), top);
    mesh.colored_vertex(rect.right_top(), top);
    mesh.colored_vertex(rect.right_bottom(), bottom);
    mesh.colored_vertex(rect.left_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}

/// A floor grid under the character, fading towards its edge.
fn paint_grid(painter: &egui::Painter, pr: &Projector, floor: f32, size: f32) {
    let step = (size / 8.0).max(0.05);
    let n: i32 = 10;
    let extent = step * n as f32;
    for i in -n..=n {
        let t = i as f32 * step;
        let fade = 1.0 - (i.abs() as f32 / n as f32);
        let color = if i == 0 {
            Color32::from_rgba_unmultiplied(120, 128, 145, (110.0 * fade + 40.0) as u8)
        } else {
            Color32::from_rgba_unmultiplied(90, 98, 115, (70.0 * fade) as u8)
        };
        for (a, b) in [
            (Vec3::new(t, floor, -extent), Vec3::new(t, floor, extent)),
            (Vec3::new(-extent, floor, t), Vec3::new(extent, floor, t)),
        ] {
            let pts: Vec<Pos2> = (0..=8)
                .filter_map(|k| pr.pos(a.lerp(b, k as f32 / 8.0)))
                .collect();
            if pts.len() > 1 {
                painter.add(egui::Shape::line(pts, Stroke::new(1.0, color)));
            }
        }
    }
}

/// A small XYZ orientation gizmo.
fn paint_axes(painter: &egui::Painter, pr: &Projector, at: Pos2) {
    let right = pr.forward.cross(Vec3::Y).normalize_or_zero();
    let up = right.cross(pr.forward);
    painter.circle_filled(at, 30.0, Color32::from_rgba_unmultiplied(14, 16, 20, 160));
    let mut axes = [
        (Vec3::X, "X", Color32::from_rgb(240, 90, 90)),
        (Vec3::Y, "Y", Color32::from_rgb(110, 210, 120)),
        (Vec3::Z, "Z", Color32::from_rgb(90, 150, 250)),
    ];
    // Draw the axes pointing away first.
    axes.sort_by(|a, b| b.0.dot(pr.forward).total_cmp(&a.0.dot(pr.forward)));
    for (axis, name, color) in axes {
        let d = Vec2::new(axis.dot(right), -axis.dot(up)) * 22.0;
        painter.line_segment([at, at + d], Stroke::new(2.0, color));
        painter.circle_filled(at + d, 7.0, color);
        painter.text(
            at + d,
            Align2::CENTER_CENTER,
            name,
            FontId::proportional(9.5),
            Color32::from_rgb(14, 16, 20),
        );
    }
}

/// A bone as a thin diamond from head to tail.
fn bone_shape(painter: &egui::Painter, head: Pos2, tail: Pos2, color: Color32) {
    let d = tail - head;
    let len = d.length();
    if len < 1.0 {
        painter.circle_filled(head, 2.0, color);
        return;
    }
    let n = Vec2::new(-d.y, d.x) / len * (len * 0.1).clamp(1.5, 6.0);
    let mid = head + d * 0.2;
    let outline = vec![head, mid + n, tail, mid - n];
    painter.add(egui::Shape::convex_polygon(
        outline,
        color.gamma_multiply(0.3),
        Stroke::new(1.2, color),
    ));
    painter.circle_filled(head, 2.5, color);
}
