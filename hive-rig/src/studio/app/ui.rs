//! Layout and drawing of the Hive Rig window. Everything here calls the
//! operations in the parent module; nothing here changes the rig directly.
//!
//! The 3D canvas fills the window. Everything else floats over it: the title and
//! camera pills at the top, the outliner and properties panels at the sides, and
//! the dock along the bottom with the honeycomb Build button in its middle. The
//! rigging window (Shift+A) and the export window open as dialogs.

use std::path::{Path, PathBuf};

use eframe::egui::{
    self, Align, Align2, Color32, FontId, Key, Layout, Margin, Modifiers, PointerButton, Pos2,
    Rect, RichText, Sense, Shadow, Stroke, Ui, UiBuilder, Vec2,
};
use glam::Vec3;

use hive_rig::components::{self, Guides, Kind};
use hive_rig::naming::{Naming, Preset};
use hive_rig::skeleton::Side;

use super::{Category, Doc, Drag, FingerSides, Mode, Studio, push_undo};
use crate::theme::{self, ACCENT, BORDER, CARD, CARD_HI, ERR, OK, TEXT, TEXT_DIM, TEXT_FAINT};
use crate::view::{self, Projector};

const PICK_RADIUS: f32 = 10.0;
const TOAST_SECONDS: f64 = 5.0;
const EDGE: f32 = 16.0;
/// Where the side panels start, below the top pills.
const PANEL_TOP: f32 = 72.0;
/// Space the dock takes at the bottom of the window.
const DOCK_CLEARANCE: f32 = 118.0;
const OUTLINER_WIDTH: f32 = 252.0;
const PROPERTIES_WIDTH: f32 = 300.0;

fn kind_label(k: Kind) -> &'static str {
    match k {
        Kind::Root => "Root",
        Kind::Spine => "Spine",
        Kind::Head => "Head",
        Kind::Arm => "Arm",
        Kind::Leg => "Leg",
        Kind::Chain => "Chain",
        Kind::Eyes => "Eyes",
        Kind::Jaw => "Jaw",
        Kind::Face => "Face",
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

/// The naming choices in the export window.
#[derive(Clone, Copy, PartialEq, Eq)]
enum NamingChoice {
    Preset(Preset),
    Custom,
}

/// A premade part in the rigging window.
struct LibraryPart {
    art: Art,
    title: &'static str,
    text: &'static str,
    category: Category,
    add: fn(&mut Studio),
    /// Parts a rig can only have one of.
    once: Option<Kind>,
}

const LIBRARY: [LibraryPart; 9] = [
    LibraryPart {
        art: Art::Face,
        title: "Face",
        text: "Brows, eyelids, cheeks, nose and lips as joints.",
        category: Category::Face,
        add: Studio::add_face,
        once: Some(Kind::Face),
    },
    LibraryPart {
        art: Art::Eyes,
        title: "Eyes",
        text: "Both eyes aim at one look-at control.",
        category: Category::Face,
        add: Studio::add_eyes,
        once: Some(Kind::Eyes),
    },
    LibraryPart {
        art: Art::Jaw,
        title: "Jaw",
        text: "A hinge that opens the mouth.",
        category: Category::Face,
        add: Studio::add_jaw,
        once: Some(Kind::Jaw),
    },
    LibraryPart {
        art: Art::Ears,
        title: "Ears",
        text: "A two-bone chain on each side of the head.",
        category: Category::Face,
        add: Studio::add_ears,
        once: None,
    },
    LibraryPart {
        art: Art::Fingers,
        title: "Fingers",
        text: "Thumb to pinky, three joints each.",
        category: Category::Body,
        add: Studio::add_fingers,
        once: None,
    },
    LibraryPart {
        art: Art::Tail,
        title: "Tail",
        text: "An FK tail from the pelvis.",
        category: Category::Body,
        add: Studio::add_tail,
        once: None,
    },
    LibraryPart {
        art: Art::Ponytail,
        title: "Ponytail",
        text: "Hair that swings from the back of the head.",
        category: Category::Extras,
        add: Studio::add_ponytail,
        once: None,
    },
    LibraryPart {
        art: Art::Prop,
        title: "Prop",
        text: "A weapon socket in the right hand.",
        category: Category::Extras,
        add: Studio::add_prop,
        once: None,
    },
    LibraryPart {
        art: Art::Chain,
        title: "Chain",
        text: "An FK chain on any bone. Set it up below.",
        category: Category::Extras,
        add: Studio::add_chain,
        once: None,
    },
];

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
        self.shortcuts(&ctx);

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::BG))
            .show(ui, |ui| {
                if self.doc.is_some() {
                    self.viewport(ui);
                } else {
                    self.welcome(ui);
                }
            });

        self.title_pill(&ctx);
        if self.doc.is_some() {
            self.steps_pill(&ctx);
            self.camera_pill(&ctx);
            self.outliner(&ctx);
            self.properties(&ctx);
            self.dock(&ctx);
            self.library_window(&ctx);
            self.export_window(&ctx);
        }
        self.toast(&ctx);
        self.drop_overlay(&ctx);
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z))
            || ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Y))
        {
            self.redo();
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::Z)) {
            self.undo();
        }
        if self.doc.is_none() || ctx.egui_wants_keyboard_input() {
            return;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::SHIFT, Key::A)) {
            self.library_open = true;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::E)) {
            self.export_open = true;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::COMMAND, Key::B)) {
            self.build();
        }
        if self.library_open || self.export_open {
            return;
        }
        for (key, mode) in [
            (Key::Num1, Mode::Guides),
            (Key::Num2, Mode::Skeleton),
            (Key::Num3, Mode::Rig),
        ] {
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, key)) {
                self.mode = mode;
            }
        }
    }

    // ----------------------------------------------------------------------- top pills

    fn title_pill(&mut self, ctx: &egui::Context) {
        floating_area("title", Align2::LEFT_TOP, Vec2::new(EDGE, EDGE)).show(ctx, |ui| {
            pill_frame().show(ui, |ui| {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::hover());
                    theme::logo(ui.painter(), r.center(), 12.0);
                    ui.label(RichText::new("Hive Rig").size(15.0).strong().color(TEXT));
                    if let Some(doc) = &self.doc {
                        ui.label(RichText::new("/").color(TEXT_FAINT));
                        ui.label(RichText::new(&doc.name).color(TEXT_DIM));
                    }
                    ui.add_space(2.0);
                    self.file_menu(ui);
                });
            });
        });
    }

    fn file_menu(&mut self, ui: &mut Ui) {
        let has_doc = self.doc.is_some();
        ui.menu_button(RichText::new("File").color(TEXT_DIM), |ui| {
            ui.set_min_width(210.0);
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
            if ui
                .add_enabled(has_doc, egui::Button::new("Export…    Ctrl+E"))
                .clicked()
            {
                ui.close();
                self.export_open = true;
            }
            if ui.button("Quit").clicked() {
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
            }
        });
    }

    fn steps_pill(&mut self, ctx: &egui::Context) {
        floating_area("steps", Align2::CENTER_TOP, Vec2::new(0.0, EDGE)).show(ctx, |ui| {
            pill_frame().show(ui, |ui| {
                ui.horizontal(|ui| self.steps(ui));
            });
        });
    }

    fn steps(&mut self, ui: &mut Ui) {
        let Some(doc) = &self.doc else { return };
        let built = doc.rig.is_some();
        let exported = doc.exported;
        let steps = [
            ("Guides", true, !built),
            ("Build", built, false),
            ("Export", exported, built && !exported),
        ];
        let mut clicked = None;
        ui.spacing_mut().item_spacing.x = 4.0;
        for (i, (name, done, active)) in steps.into_iter().enumerate() {
            if i > 0 {
                let (r, _) = ui.allocate_exact_size(Vec2::new(18.0, 2.0), Sense::hover());
                let c = if done {
                    ACCENT.gamma_multiply(0.6)
                } else {
                    BORDER
                };
                ui.painter()
                    .line_segment([r.left_center(), r.right_center()], Stroke::new(2.0, c));
            }
            let (fill, text) = if active {
                (ACCENT.gamma_multiply(0.18), ACCENT)
            } else if done {
                (Color32::TRANSPARENT, TEXT)
            } else {
                (Color32::TRANSPARENT, TEXT_FAINT)
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
            .stroke(Stroke::NONE)
            .corner_radius(12)
            .min_size(Vec2::new(0.0, 26.0));
            if ui.add(b).clicked() {
                clicked = Some(i);
            }
        }
        match clicked {
            Some(0) => self.mode = Mode::Guides,
            Some(1) => {
                self.build();
            }
            Some(2) => self.export_open = true,
            _ => {}
        }
    }

    fn camera_pill(&mut self, ctx: &egui::Context) {
        floating_area("camera", Align2::RIGHT_TOP, Vec2::new(-EDGE, EDGE)).show(ctx, |ui| {
            pill_frame().show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    for (name, yaw, pitch) in [
                        ("Front", 0.0, 0.05),
                        ("Side", std::f32::consts::FRAC_PI_2, 0.05),
                        ("Persp", 0.55, 0.22),
                    ] {
                        let b = egui::Button::new(RichText::new(name).size(13.0).color(TEXT_DIM))
                            .frame_when_inactive(false)
                            .corner_radius(10)
                            .min_size(Vec2::new(0.0, 26.0));
                        if ui.add(b).clicked() {
                            self.camera.yaw = yaw;
                            self.camera.pitch = pitch;
                        }
                    }
                    if theme::icon(ui, "⛶", "Frame the character (F)", true).clicked() {
                        self.frame_camera();
                    }
                });
            });
        });
    }

    // ----------------------------------------------------------------------- notifications

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
        // Above the dock, or under the top pills while a dialog covers the middle.
        let (anchor, offset) = if self.library_open || self.export_open {
            (Align2::CENTER_TOP, Vec2::new(0.0, PANEL_TOP - 8.0))
        } else if self.doc.is_some() {
            (Align2::CENTER_BOTTOM, Vec2::new(0.0, -DOCK_CLEARANCE))
        } else {
            (Align2::CENTER_BOTTOM, Vec2::new(0.0, -24.0))
        };
        egui::Area::new(id)
            .anchor(anchor, offset)
            .order(egui::Order::Tooltip)
            .interactable(false)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(CARD_HI)
                    .stroke(Stroke::new(1.0, color.gamma_multiply(0.6)))
                    .corner_radius(12)
                    .shadow(ui.visuals().popup_shadow)
                    .inner_margin(Margin::symmetric(14, 10))
                    .show(ui, |ui| {
                        ui.set_max_width(460.0);
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
        let pr = view::Camera::default().projector(rect);
        paint_grid(ui.painter(), &pr, 0.0, 1.0);
        let width = 760f32.min(rect.width() - 48.0);
        let top = rect.top() + (rect.height() * 0.14).max(80.0);
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
                        "Drag the joints. Shift+A adds eyes, fingers, tails.",
                    ),
                    ("3", "Build", "Bones, IK/FK controls and skin weights."),
                    ("4", "Export", "Blender, Maya and Unreal, named your way."),
                ] {
                    step_card(ui, step_w, n, title, text);
                }
            });
        });
    }

    // ----------------------------------------------------------------------- outliner

    fn outliner(&mut self, ctx: &egui::Context) {
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
        let rows: Vec<(String, Kind, Side, usize)> = doc
            .guides
            .components
            .iter()
            .zip(&depth)
            .map(|(c, d)| (c.id.clone(), c.kind, c.side, *d))
            .collect();
        let max_h = ctx.content_rect().height() - PANEL_TOP - DOCK_CLEARANCE - 70.0;

        floating_area("outliner", Align2::LEFT_TOP, Vec2::new(EDGE, PANEL_TOP)).show(ctx, |ui| {
            room(ui, OUTLINER_WIDTH, max_h + 70.0);
            panel_frame().show(ui, |ui| {
                ui.set_width(OUTLINER_WIDTH);
                let mut open = self.outliner_open;
                panel_header(ui, "Outliner", &rows.len().to_string(), &mut open);
                self.outliner_open = open;
                if !open {
                    return;
                }
                egui::ScrollArea::vertical()
                    .max_height(max_h.max(120.0))
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        for (i, (id, kind, side, depth)) in rows.iter().enumerate() {
                            let selected = self.selected.is_some_and(|(c, _)| c == i);
                            let row =
                                component_row(ui, id, *kind, *side, (*depth).min(4), selected);
                            if row.clicked() {
                                self.selected = if selected { None } else { Some((i, None)) };
                                self.mode = Mode::Guides;
                            }
                        }
                    });
                ui.add_space(4.0);
                let add =
                    egui::Button::new(RichText::new("+  Add component").size(13.5).color(ACCENT))
                        .fill(ACCENT.gamma_multiply(0.1))
                        .stroke(Stroke::new(1.0, ACCENT.gamma_multiply(0.35)))
                        .corner_radius(10)
                        .min_size(Vec2::new(ui.available_width(), 32.0));
                if ui
                    .add(add)
                    .on_hover_text("Open the rigging window (Shift+A)")
                    .clicked()
                {
                    self.library_open = true;
                }
            });
        });
    }

    // ----------------------------------------------------------------------- properties

    fn properties(&mut self, ctx: &egui::Context) {
        let max_h = ctx.content_rect().height() - PANEL_TOP - DOCK_CLEARANCE - 50.0;
        let title = match self.selected {
            Some(_) => "Component",
            None => "Rig",
        };
        floating_area("properties", Align2::RIGHT_TOP, Vec2::new(-EDGE, PANEL_TOP)).show(
            ctx,
            |ui| {
                room(ui, PROPERTIES_WIDTH, max_h + 50.0);
                panel_frame().show(ui, |ui| {
                    ui.set_width(PROPERTIES_WIDTH);
                    let mut open = self.properties_open;
                    panel_header(ui, "Properties", title, &mut open);
                    self.properties_open = open;
                    if !open {
                        return;
                    }
                    egui::ScrollArea::vertical()
                        .max_height(max_h.max(120.0))
                        .auto_shrink([false, true])
                        .show(ui, |ui| match self.selected {
                            Some((c, guide)) => self.component_inspector(ui, c, guide),
                            None => self.rig_overview(ui),
                        });
                });
            },
        );
    }

    fn rig_overview(&mut self, ui: &mut Ui) {
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
        let naming = doc.guides.naming.clone();

        ui.horizontal(|ui| {
            let (text, color) = if preview_error.is_some() {
                ("Guides need fixing", ERR)
            } else if built {
                ("Built", OK)
            } else {
                ("Not built yet", TEXT_FAINT)
            };
            theme::badge(ui, text, color);
        });
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
        ui.label(
            RichText::new(
                "Click a component in the outliner, or a guide on the character, to edit it.",
            )
            .size(12.0)
            .color(TEXT_FAINT),
        );

        ui.add_space(8.0);
        theme::label_caps(ui, "Skin weights");
        if deform.is_empty() {
            ui.label(
                RichText::new("Build the rig to paint each bone's weights on the mesh.")
                    .size(12.5)
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
            ui.horizontal(|ui| {
                let mirror = ui
                    .add(egui::Button::new(RichText::new("Mirror L to R").size(12.5)).corner_radius(8))
                    .on_hover_text("Copy the left side's weights onto the right. Right-click to copy right to left.");
                if mirror.clicked() {
                    self.mirror_weights(Side::Left);
                }
                mirror.context_menu(|ui| {
                    if ui.button("Mirror R to L").clicked() {
                        self.mirror_weights(Side::Right);
                        ui.close();
                    }
                });
                let what = if self.weights_bone.is_some() { "the shown bone" } else { "every bone" };
                if ui
                    .add(egui::Button::new(RichText::new("Smooth").size(12.5)).corner_radius(8))
                    .on_hover_text(format!("Blur the weights of {what} with their neighbours."))
                    .clicked()
                {
                    self.smooth_weights();
                }
            });
        }

        ui.add_space(8.0);
        theme::label_caps(ui, "Naming");
        ui.horizontal(|ui| {
            let label = naming.preset().map_or("Custom", |p| p.label());
            ui.label(RichText::new(label).strong().color(TEXT));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add(egui::Button::new(RichText::new("Change").size(12.5)).corner_radius(8))
                    .clicked()
                {
                    self.export_open = true;
                }
            });
        });
        naming_examples(ui, &naming, 3);
    }

    fn component_inspector(&mut self, ui: &mut Ui, c: usize, guide: Option<usize>) {
        let Some(doc) = &self.doc else { return };
        let Some(comp) = doc.guides.components.get(c).cloned() else {
            return;
        };
        let naming = doc.guides.naming.clone();
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

        ui.horizontal(|ui| {
            theme::dot(ui, side_color(comp.side));
            ui.label(RichText::new(&comp.id).size(16.0).strong().color(TEXT));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add(egui::Button::new(RichText::new("Done").size(12.5)).corner_radius(8))
                    .on_hover_text("Back to the rig overview")
                    .clicked()
                {
                    self.selected = None;
                }
            });
        });
        ui.horizontal(|ui| {
            theme::badge(ui, kind_label(comp.kind), ACCENT);
            theme::badge(ui, side_label(comp.side), side_color(comp.side));
            ui.label(
                RichText::new(format!("{} bones", own.len()))
                    .size(12.5)
                    .color(TEXT_DIM),
            );
        });
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
                                [60.0, 24.0],
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

        if !own.is_empty() {
            ui.add_space(6.0);
            egui::CollapsingHeader::new(
                RichText::new(format!("Exports as ({})", own.len()))
                    .size(12.0)
                    .strong()
                    .color(TEXT_FAINT),
            )
            .id_salt("exports_as")
            .show(ui, |ui| {
                for b in &own {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(b).size(12.0).color(TEXT_FAINT));
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(
                                RichText::new(naming.bone(b))
                                    .size(12.5)
                                    .monospace()
                                    .color(TEXT),
                            );
                        });
                    });
                }
            });
        }
        if comp.kind != Kind::Root {
            ui.add_space(6.0);
            let remove = egui::Button::new(RichText::new("Remove component").color(ERR))
                .fill(ERR.gamma_multiply(0.1))
                .stroke(Stroke::new(1.0, ERR.gamma_multiply(0.4)))
                .corner_radius(10)
                .min_size(Vec2::new(ui.available_width(), 32.0));
            if ui.add(remove).on_hover_text("Delete").clicked() {
                self.remove_selected();
            }
        }
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

    // ----------------------------------------------------------------------- dock

    fn dock(&mut self, ctx: &egui::Context) {
        floating_area("dock", Align2::CENTER_BOTTOM, Vec2::new(0.0, -EDGE)).show(ctx, |ui| {
            egui::Frame::new()
                .fill(Color32::from_rgba_unmultiplied(20, 23, 30, 240))
                .stroke(Stroke::new(1.0, Color32::from_white_alpha(16)))
                .corner_radius(20)
                .inner_margin(Margin::symmetric(10, 6))
                .shadow(Shadow {
                    offset: [0, 10],
                    blur: 30,
                    spread: 0,
                    color: Color32::from_black_alpha(130),
                })
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 0.0;
                        self.dock_buttons(ui);
                    });
                });
        });
    }

    fn dock_buttons(&mut self, ui: &mut Ui) {
        let built = self.doc.as_ref().is_some_and(|d| d.rig.is_some());
        let buildable = self.doc.as_ref().is_some_and(|d| d.preview.is_ok());
        let guides = self.mode == Mode::Guides;

        if dock_button(
            ui,
            Icon::Move,
            "Move",
            guides,
            true,
            "Drag guides to place joints (1)",
        )
        .clicked()
        {
            self.mode = Mode::Guides;
        }
        if dock_button(
            ui,
            Icon::Add,
            "Add",
            self.library_open,
            true,
            "Add a component (Shift+A)",
        )
        .clicked()
        {
            self.library_open = true;
        }
        let mirror = dock_button(
            ui,
            Icon::Mirror,
            "Mirror",
            false,
            true,
            "Copy the left side onto the right. Right-click for the other way.",
        );
        if mirror.clicked() {
            self.mirror(Side::Left);
        }
        mirror.context_menu(|ui| {
            if ui.button("Left to right").clicked() {
                self.mirror(Side::Left);
                ui.close();
            }
            if ui.button("Right to left").clicked() {
                self.mirror(Side::Right);
                ui.close();
            }
        });
        if dock_button(
            ui,
            Icon::Symmetry,
            "Symmetry",
            self.symmetry,
            true,
            "Moving a left or right guide moves its twin too",
        )
        .clicked()
        {
            self.symmetry = !self.symmetry;
        }
        dock_separator(ui);
        for (mode, icon, label, tip) in [
            (Mode::Guides, Icon::Guides, "Guides", "Show guides (1)"),
            (
                Mode::Skeleton,
                Icon::Skeleton,
                "Skeleton",
                "Show the skeleton (2)",
            ),
            (
                Mode::Rig,
                Icon::Controls,
                "Controls",
                "Show the animator controls (3)",
            ),
        ] {
            if dock_button(
                ui,
                icon,
                label,
                self.mode == mode && mode != Mode::Guides,
                true,
                tip,
            )
            .clicked()
            {
                self.mode = mode;
            }
        }

        let label = if built { "Rebuild" } else { "Build" };
        if hex_button(ui, label, buildable)
            .on_hover_text("Build the bones, controls and skin weights (Ctrl+B)")
            .clicked()
        {
            self.build();
        }

        if dock_button(
            ui,
            Icon::Mesh,
            "Mesh",
            self.show_mesh,
            true,
            "Show the character mesh",
        )
        .clicked()
        {
            self.show_mesh = !self.show_mesh;
        }
        if dock_button(
            ui,
            Icon::Xray,
            "X-ray",
            self.xray,
            true,
            "See the rig through the mesh",
        )
        .clicked()
        {
            self.xray = !self.xray;
        }
        let weights_on = self.weights_bone.is_some();
        if dock_button(
            ui,
            Icon::Weights,
            "Weights",
            weights_on,
            built,
            "Paint a bone's skin weights on the mesh (build first)",
        )
        .clicked()
        {
            self.weights_bone = if weights_on {
                None
            } else {
                self.doc.as_ref().and_then(|d| {
                    let rig = d.rig.as_ref()?;
                    let first = rig.skeleton.bones.iter().find(|b| b.deform)?;
                    let upper = rig.guides.naming.bone("upperarm_l");
                    let pick = rig
                        .skeleton
                        .bone(&upper)
                        .map_or(first.name.clone(), |b| b.name.clone());
                    Some(pick)
                })
            };
            self.properties_open = true;
            self.selected = None;
        }
        dock_separator(ui);
        if dock_button(
            ui,
            Icon::Undo,
            "Undo",
            false,
            self.can_undo(),
            "Undo (Ctrl+Z)",
        )
        .clicked()
        {
            self.undo();
        }
        if dock_button(
            ui,
            Icon::Redo,
            "Redo",
            false,
            self.can_redo(),
            "Redo (Ctrl+Y)",
        )
        .clicked()
        {
            self.redo();
        }
        if dock_button(
            ui,
            Icon::Frame,
            "Frame",
            false,
            true,
            "Frame the character (F)",
        )
        .clicked()
        {
            self.frame_camera();
        }
        dock_separator(ui);
        if dock_button(
            ui,
            Icon::Export,
            "Export",
            self.export_open,
            true,
            "Export to Blender, Maya and Unreal (Ctrl+E)",
        )
        .clicked()
        {
            self.export_open = true;
        }
    }

    // ----------------------------------------------------------------------- rigging window

    fn library_window(&mut self, ctx: &egui::Context) {
        if !self.library_open {
            return;
        }
        let screen = ctx.content_rect();
        let width = 900f32.min(screen.width() - 60.0);
        let kinds: Vec<Kind> = self
            .doc
            .as_ref()
            .map(|d| d.guides.components.iter().map(|c| c.kind).collect())
            .unwrap_or_default();
        let mut close = false;
        let modal = egui::Modal::new(egui::Id::unique("library"))
            .frame(dialog_frame())
            .backdrop_color(Color32::from_black_alpha(140))
            .show(ctx, |ui| {
                ui.set_width(width);
                close |= dialog_header(
                    ui,
                    "Add a component",
                    "Premade parts land on the guides, ready to adjust. Shift+A opens this window.",
                );
                ui.add_space(4.0);
                let mut cat = self.category;
                theme::segmented(
                    ui,
                    &mut cat,
                    &[
                        (Category::All, "All"),
                        (Category::Face, "Face"),
                        (Category::Body, "Body"),
                        (Category::Extras, "Extras"),
                    ],
                );
                self.category = cat;
                ui.add_space(8.0);
                let parts: Vec<&LibraryPart> = LIBRARY
                    .iter()
                    .filter(|p| self.category == Category::All || p.category == self.category)
                    .collect();
                let gap = 12.0;
                let cols = 5;
                let card_w = ((width - (cols - 1) as f32 * gap) / cols as f32).floor();
                for row in parts.chunks(cols) {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = gap;
                        for part in row {
                            let added = part.once.is_some_and(|k| kinds.contains(&k));
                            if part_card(ui, card_w, part, added).clicked() && !added {
                                let before = self.status_serial;
                                (part.add)(self);
                                self.mode = Mode::Guides;
                                if self.status_serial != before && !self.error {
                                    close = true;
                                }
                            }
                        }
                    });
                    ui.add_space(gap - 8.0);
                }
                ui.add_space(4.0);
                self.library_options(ui);
            });
        if close || modal.should_close() {
            self.library_open = false;
        }
    }

    fn library_options(&mut self, ui: &mut Ui) {
        let bones = self.doc.as_ref().map(Doc::bone_names).unwrap_or_default();
        if !bones.contains(&self.chain_parent) {
            self.chain_parent = bones
                .iter()
                .find(|b| b.as_str() == "head")
                .or(bones.first())
                .cloned()
                .unwrap_or_default();
        }
        egui::Frame::new()
            .fill(Color32::from_rgb(17, 20, 26))
            .stroke(Stroke::new(1.0, BORDER))
            .corner_radius(12)
            .inner_margin(12)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    theme::label_caps(ui, "Options");
                    ui.add_space(8.0);
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
                    ui.add_space(12.0);
                    ui.label(RichText::new("Tail bones").color(TEXT_DIM));
                    ui.add(egui::DragValue::new(&mut self.tail_joints).range(2..=24));
                });
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    theme::label_caps(ui, "Chain");
                    ui.add_space(8.0);
                    ui.label(RichText::new("on").color(TEXT_DIM));
                    egui::ComboBox::from_id_salt("chain_parent")
                        .selected_text(self.chain_parent.as_str())
                        .height(320.0)
                        .show_ui(ui, |ui| {
                            for b in &bones {
                                ui.selectable_value(&mut self.chain_parent, b.clone(), b);
                            }
                        });
                    ui.label(RichText::new("bones").color(TEXT_DIM));
                    ui.add(egui::DragValue::new(&mut self.chain_joints).range(1..=24));
                    ui.label(RichText::new("name").color(TEXT_DIM));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.chain_id)
                            .hint_text("automatic")
                            .desired_width(110.0),
                    );
                });
            });
    }

    // ----------------------------------------------------------------------- export window

    fn export_window(&mut self, ctx: &egui::Context) {
        if !self.export_open || self.doc.is_none() {
            return;
        }
        let mut close = false;
        let mut export_dir = None;
        let mut choose = false;
        let mut apply: Option<Naming> = None;
        let modal = egui::Modal::new(egui::Id::unique("export"))
            .frame(dialog_frame())
            .backdrop_color(Color32::from_black_alpha(140))
            .show(ctx, |ui| {
                ui.set_width(600f32.min(ctx.content_rect().width() - 60.0));
                close |= dialog_header(
                    ui,
                    "Export rig",
                    "Writes the skinned .glb and a builder script per program, with every control.",
                );
                let doc = self.doc.as_mut().unwrap();

                theme::label_caps(ui, "Programs");
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

                ui.add_space(10.0);
                theme::label_caps(ui, "Naming convention");
                let current = doc.guides.naming.clone();
                let mut choice = match (&self.naming_draft, current.preset()) {
                    (None, Some(p)) => NamingChoice::Preset(p),
                    _ => NamingChoice::Custom,
                };
                let before = choice;
                theme::segmented(
                    ui,
                    &mut choice,
                    &[
                        (NamingChoice::Preset(Preset::Game), Preset::Game.label()),
                        (NamingChoice::Preset(Preset::Prefix), Preset::Prefix.label()),
                        (NamingChoice::Preset(Preset::Suffix), Preset::Suffix.label()),
                        (NamingChoice::Custom, "Custom"),
                    ],
                );
                if choice != before {
                    match choice {
                        NamingChoice::Preset(p) => {
                            self.naming_draft = None;
                            apply = Some(p.naming());
                        }
                        NamingChoice::Custom => self.naming_draft = Some(current.clone()),
                    }
                }
                if choice == NamingChoice::Custom && self.naming_draft.is_none() {
                    self.naming_draft = Some(current.clone());
                }
                let shown = self.naming_draft.clone().unwrap_or(current.clone());
                ui.label(
                    RichText::new(match choice {
                        NamingChoice::Preset(Preset::Game) => {
                            "Matches the Unreal mannequin, so animations retarget without remapping."
                        }
                        NamingChoice::Preset(Preset::Prefix) => {
                            "Side first, joints end in _jnt: common in Maya studio pipelines."
                        }
                        NamingChoice::Preset(Preset::Suffix) => {
                            "Side after the name, upper-case type: easy to sort and search."
                        }
                        NamingChoice::Custom => {
                            "Use {name}, {side} and {desc} (fk, ik, pole, chain numbers)."
                        }
                    })
                    .size(12.5)
                    .color(TEXT_FAINT),
                );
                if let Some(draft) = &mut self.naming_draft {
                    egui::Grid::new("naming_fields")
                        .num_columns(4)
                        .spacing([8.0, 6.0])
                        .show(ui, |ui| {
                            ui.label(RichText::new("Joints").color(TEXT_DIM));
                            ui.add(egui::TextEdit::singleline(&mut draft.joint).desired_width(190.0));
                            ui.label(RichText::new("Controls").color(TEXT_DIM));
                            ui.add(
                                egui::TextEdit::singleline(&mut draft.control).desired_width(190.0),
                            );
                            ui.end_row();
                            ui.label(RichText::new("Left / right").color(TEXT_DIM));
                            ui.horizontal(|ui| {
                                ui.add(egui::TextEdit::singleline(&mut draft.left).desired_width(54.0));
                                ui.add(
                                    egui::TextEdit::singleline(&mut draft.right).desired_width(54.0),
                                );
                            });
                            ui.label(RichText::new("Centre").color(TEXT_DIM));
                            ui.add(egui::TextEdit::singleline(&mut draft.center).desired_width(54.0));
                            ui.end_row();
                        });
                    let check = draft.validate();
                    ui.horizontal(|ui| {
                        match &check {
                            Ok(()) if *draft == current => {
                                ui.label(RichText::new("In use").size(12.5).color(OK));
                            }
                            Ok(()) => {
                                if ui.button("Use this naming").clicked() {
                                    apply = Some(draft.clone());
                                }
                            }
                            Err(e) => {
                                ui.label(RichText::new(format!("{e}")).size(12.5).color(ERR));
                            }
                        };
                    });
                }
                naming_examples(ui, &shown, 4);

                ui.add_space(10.0);
                theme::label_caps(ui, "Files");
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
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Folder").color(TEXT_DIM));
                    let dir = doc.out_dir.clone();
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add_enabled(dir.is_dir(), egui::Button::new("Open").corner_radius(8))
                            .clicked()
                        {
                            open_folder(&dir);
                        }
                        if ui
                            .add(egui::Button::new("Choose…").corner_radius(8))
                            .clicked()
                        {
                            choose = true;
                        }
                        ui.add(
                            egui::Label::new(
                                RichText::new(dir.display().to_string())
                                    .size(12.0)
                                    .color(TEXT_FAINT),
                            )
                            .truncate(),
                        )
                        .on_hover_text(dir.display().to_string());
                    });
                });

                ui.add_space(12.0);
                let any = self.targets.iter().any(|t| *t);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if theme::primary(ui, "Export", 140.0, any).clicked() {
                        export_dir = Some(doc.out_dir.clone());
                    }
                    if theme::secondary(ui, "Close", 90.0).clicked() {
                        close = true;
                    }
                    if doc.exported {
                        ui.label(RichText::new("Exported").color(OK));
                    }
                });
            });
        if let Some(n) = apply
            && self.set_naming(n)
            && self.naming_draft.is_some()
        {
            self.naming_draft = self.doc.as_ref().map(|d| d.guides.naming.clone());
        }
        if choose {
            let start = self
                .doc
                .as_ref()
                .map(|d| d.out_dir.clone())
                .unwrap_or_default();
            if let Some(dir) = rfd::FileDialog::new()
                .set_directory(start.parent().unwrap_or(&start))
                .pick_folder()
                && let Some(doc) = &mut self.doc
            {
                doc.out_dir = dir;
            }
        }
        if let Some(dir) = export_dir {
            self.do_export(dir);
            if !self.error {
                close = true;
            }
        }
        if close || modal.should_close() {
            self.export_open = false;
            self.naming_draft = None;
        }
    }

    // ----------------------------------------------------------------------- viewport

    fn frame_camera(&mut self) {
        if let Some(doc) = &self.doc {
            let (min, max) = doc.mesh.bounds();
            self.camera.frame(min, max);
        }
    }

    fn viewport(&mut self, ui: &mut Ui) {
        let rect = ui.max_rect();
        let (response, painter) = ui.allocate_painter(rect.size(), Sense::click_and_drag());
        let rect = response.rect;
        let painter = painter.with_clip_rect(rect);

        let blocked = ui.ctx().egui_wants_keyboard_input() || self.library_open || self.export_open;
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta().y);
            if scroll != 0.0 {
                self.camera.zoom(scroll);
            }
        }
        if !blocked {
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
            Pos2::new(rect.left() + 50.0, rect.bottom() - 50.0),
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
            // The preview uses internal names; the weights list uses exported ones.
            let color = if weights
                .as_deref()
                .is_some_and(|w| w == doc.guides.naming.bone(&b.name))
            {
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
            if c.kind == Kind::Eyes {
                // Sight lines from each eye to the look-at target.
                if let [Some(l), Some(r), Some(t)] = pts[..] {
                    for e in [l, r] {
                        painter.extend(egui::Shape::dashed_line(&[e, t], line, 5.0, 4.0));
                    }
                }
            } else if c.kind != Kind::Face {
                // Face guides are separate joints, not a chain.
                for w in pts.windows(2) {
                    if let [Some(a), Some(b)] = w {
                        painter.line_segment([*a, *b], line);
                    }
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
                } * if c.kind == Kind::Face { 0.6 } else { 1.0 };
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

// --------------------------------------------------------------------------- frames

fn theme_once(ctx: &egui::Context) {
    let id = egui::Id::unique("hive_theme");
    if !ctx.data(|d| d.get_temp::<bool>(id).unwrap_or(false)) {
        theme::apply(ctx);
        ctx.data_mut(|d| d.insert_temp(id, true));
    }
}

fn floating_area(id: &'static str, anchor: Align2, offset: Vec2) -> egui::Area {
    egui::Area::new(egui::Id::unique(id))
        .anchor(anchor, offset)
        .order(egui::Order::Middle)
        .movable(false)
}

/// An area's ui is only as big as its contents were last frame, so a panel that
/// grows (an outliner gaining rows) needs its room set explicitly.
fn room(ui: &mut Ui, width: f32, height: f32) {
    ui.set_max_size(Vec2::new(width + 20.0, height.max(140.0)));
}

fn glass(radius: u8, margin: Margin) -> egui::Frame {
    egui::Frame::new()
        .fill(Color32::from_rgba_unmultiplied(20, 23, 30, 236))
        .stroke(Stroke::new(1.0, Color32::from_white_alpha(14)))
        .corner_radius(radius)
        .inner_margin(margin)
        .shadow(Shadow {
            offset: [0, 8],
            blur: 26,
            spread: 0,
            color: Color32::from_black_alpha(110),
        })
}

fn pill_frame() -> egui::Frame {
    glass(14, Margin::symmetric(8, 4))
}

fn panel_frame() -> egui::Frame {
    glass(16, Margin::same(10))
}

fn dialog_frame() -> egui::Frame {
    egui::Frame::new()
        .fill(Color32::from_rgb(22, 25, 32))
        .stroke(Stroke::new(1.0, Color32::from_white_alpha(18)))
        .corner_radius(20)
        .inner_margin(Margin::same(20))
        .shadow(Shadow {
            offset: [0, 16],
            blur: 48,
            spread: 0,
            color: Color32::from_black_alpha(160),
        })
}

/// A panel title that collapses the panel when clicked.
fn panel_header(ui: &mut Ui, title: &str, note: &str, open: &mut bool) {
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 24.0), Sense::click());
    let painter = ui.painter();
    if response.hovered() {
        painter.rect_filled(rect.expand(2.0), 8, Color32::from_white_alpha(6));
    }
    painter.text(
        rect.left_center() + Vec2::new(4.0, 0.0),
        Align2::LEFT_CENTER,
        title,
        FontId::proportional(14.0),
        TEXT,
    );
    painter.text(
        rect.right_center() - Vec2::new(26.0, 0.0),
        Align2::RIGHT_CENTER,
        note,
        FontId::proportional(12.0),
        TEXT_FAINT,
    );
    let c = rect.right_center() - Vec2::new(10.0, 0.0);
    let pts = if *open {
        vec![
            c + Vec2::new(-4.0, -2.0),
            c + Vec2::new(0.0, 2.0),
            c + Vec2::new(4.0, -2.0),
        ]
    } else {
        vec![
            c + Vec2::new(-2.0, -4.0),
            c + Vec2::new(2.0, 0.0),
            c + Vec2::new(-2.0, 4.0),
        ]
    };
    painter.add(egui::Shape::line(pts, Stroke::new(1.6, TEXT_DIM)));
    if response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
    {
        *open = !*open;
    }
    if *open {
        ui.add_space(2.0);
    }
}

/// Title, subtitle and a close button. Returns whether close was clicked.
fn dialog_header(ui: &mut Ui, title: &str, text: &str) -> bool {
    let mut close = false;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 2.0;
            ui.label(RichText::new(title).size(22.0).strong().color(TEXT));
            ui.label(RichText::new(text).size(13.0).color(TEXT_DIM));
        });
        ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
            let (rect, r) = ui.allocate_exact_size(Vec2::splat(30.0), Sense::click());
            let painter = ui.painter();
            if r.hovered() {
                painter.circle_filled(rect.center(), 15.0, Color32::from_white_alpha(12));
            }
            let s = Stroke::new(1.8, if r.hovered() { TEXT } else { TEXT_DIM });
            let c = rect.center();
            painter.line_segment([c + Vec2::new(-5.0, -5.0), c + Vec2::new(5.0, 5.0)], s);
            painter.line_segment([c + Vec2::new(5.0, -5.0), c + Vec2::new(-5.0, 5.0)], s);
            close = r.on_hover_text("Close (Esc)").clicked();
        });
    });
    ui.add_space(8.0);
    close
}

/// Example internal names and what they export as.
fn naming_examples(ui: &mut Ui, naming: &Naming, rows: usize) {
    let examples = [
        ("upperarm_l", false),
        ("spine_01", false),
        ("hand_l_ik_ctrl", true),
        ("root_ctrl", true),
    ];
    egui::Frame::new()
        .fill(Color32::from_rgb(15, 17, 22))
        .corner_radius(10)
        .inner_margin(Margin::symmetric(10, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 3.0;
            for (internal, control) in examples.into_iter().take(rows) {
                let name = if control {
                    naming.control(internal)
                } else {
                    naming.bone(internal)
                };
                ui.horizontal(|ui| {
                    let kind = if control { "control" } else { "joint" };
                    ui.label(RichText::new(kind).size(11.0).color(TEXT_FAINT));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(name).monospace().size(13.0).color(ACCENT));
                    });
                });
            }
        });
}

// --------------------------------------------------------------------------- dock widgets

#[derive(Clone, Copy)]
enum Icon {
    Move,
    Add,
    Mirror,
    Symmetry,
    Guides,
    Skeleton,
    Controls,
    Mesh,
    Xray,
    Weights,
    Undo,
    Redo,
    Frame,
    Export,
}

fn dock_button(
    ui: &mut Ui,
    icon: Icon,
    label: &str,
    active: bool,
    enabled: bool,
    tip: &str,
) -> egui::Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(60.0, 56.0), sense);
    let hovered = enabled && response.hovered();
    let painter = ui.painter();
    if active {
        painter.rect_filled(rect.shrink(3.0), 12, ACCENT.gamma_multiply(0.16));
    } else if hovered {
        painter.rect_filled(rect.shrink(3.0), 12, Color32::from_white_alpha(10));
    }
    let color = if !enabled {
        TEXT_FAINT.gamma_multiply(0.7)
    } else if active {
        ACCENT
    } else if hovered {
        TEXT
    } else {
        TEXT_DIM
    };
    dock_icon(
        painter,
        rect.center_top() + Vec2::new(0.0, 21.0),
        icon,
        color,
    );
    painter.text(
        rect.center_bottom() - Vec2::new(0.0, 7.0),
        Align2::CENTER_BOTTOM,
        label,
        FontId::proportional(11.0),
        color,
    );
    let response = response.on_hover_text(tip);
    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

fn dock_separator(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(13.0, 56.0), Sense::hover());
    ui.painter().line_segment(
        [
            rect.center_top() + Vec2::new(0.0, 12.0),
            rect.center_bottom() - Vec2::new(0.0, 12.0),
        ],
        Stroke::new(1.0, BORDER),
    );
}

fn hexagon(c: Pos2, r: f32) -> Vec<Pos2> {
    (0..6)
        .map(|i| {
            let a = std::f32::consts::FRAC_PI_3 * i as f32 + std::f32::consts::FRAC_PI_6;
            c + Vec2::angled(a) * r
        })
        .collect()
}

/// The Build button: a honeycomb cell that rises out of the dock.
fn hex_button(ui: &mut Ui, label: &str, enabled: bool) -> egui::Response {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(96.0, 56.0), sense);
    let hovered = enabled && response.hovered();
    let pressed = enabled && response.is_pointer_button_down_on();
    let painter = ui.painter();
    let c = rect.center() - Vec2::new(0.0, if pressed { 10.0 } else { 12.0 });
    let r = 40.0;
    painter.add(egui::Shape::convex_polygon(
        hexagon(c + Vec2::new(0.0, 5.0), r),
        Color32::from_black_alpha(110),
        Stroke::NONE,
    ));
    let fill = if !enabled {
        Color32::from_rgb(92, 80, 52)
    } else if hovered {
        Color32::from_rgb(255, 202, 96)
    } else {
        ACCENT
    };
    painter.add(egui::Shape::convex_polygon(
        hexagon(c, r),
        fill,
        Stroke::new(3.0, Color32::from_rgb(20, 23, 30)),
    ));
    painter.add(egui::Shape::closed_line(
        hexagon(c, r - 7.0),
        Stroke::new(1.0, theme::ACCENT_TEXT.gamma_multiply(0.25)),
    ));
    // A three-joint chain, the logo's mark.
    let ink = theme::ACCENT_TEXT;
    let (a, b, d) = (
        c + Vec2::new(-9.0, 2.0),
        c + Vec2::new(0.0, -10.0),
        c + Vec2::new(9.0, 2.0),
    );
    painter.line_segment([a, b], Stroke::new(2.5, ink));
    painter.line_segment([b, d], Stroke::new(2.5, ink));
    for p in [a, b, d] {
        painter.circle_filled(p, 3.0, ink);
    }
    painter.text(
        c + Vec2::new(0.0, 17.0),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(12.5),
        ink,
    );
    if enabled {
        response.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        response
    }
}

fn arrow_head(painter: &egui::Painter, tip: Pos2, dir: Vec2, stroke: Stroke) {
    let d = dir.normalized() * 4.5;
    let n = Vec2::new(-d.y, d.x);
    painter.line_segment([tip, tip - d + n], stroke);
    painter.line_segment([tip, tip - d - n], stroke);
}

fn arc(c: Pos2, r: f32, from: f32, to: f32) -> Vec<Pos2> {
    (0..=16)
        .map(|i| {
            let a = from + (to - from) * i as f32 / 16.0;
            c + Vec2::new(a.cos(), -a.sin()) * r
        })
        .collect()
}

/// The dock's line icons, drawn so they look the same on every system.
fn dock_icon(painter: &egui::Painter, c: Pos2, icon: Icon, color: Color32) {
    let s = Stroke::new(1.8, color);
    let p = |x: f32, y: f32| c + Vec2::new(x, y);
    match icon {
        Icon::Move => {
            for d in [Vec2::X, -Vec2::X, Vec2::Y, -Vec2::Y] {
                painter.line_segment([c, c + d * 10.0], s);
                arrow_head(painter, c + d * 10.0, d, s);
            }
        }
        Icon::Add => {
            painter.add(egui::Shape::closed_line(hexagon(c, 10.0), s));
            painter.line_segment([p(-4.0, 0.0), p(4.0, 0.0)], s);
            painter.line_segment([p(0.0, -4.0), p(0.0, 4.0)], s);
        }
        Icon::Mirror => {
            painter.add(egui::Shape::convex_polygon(
                vec![p(-3.0, -8.0), p(-3.0, 8.0), p(-11.0, 8.0)],
                color,
                Stroke::NONE,
            ));
            painter.add(egui::Shape::closed_line(
                vec![p(3.0, -8.0), p(3.0, 8.0), p(11.0, 8.0)],
                s,
            ));
            painter.extend(egui::Shape::dashed_line(
                &[p(0.0, -11.0), p(0.0, 11.0)],
                Stroke::new(1.2, color),
                2.5,
                2.0,
            ));
        }
        Icon::Symmetry => {
            painter.extend(egui::Shape::dashed_line(
                &[p(0.0, -11.0), p(0.0, 11.0)],
                Stroke::new(1.2, color),
                2.5,
                2.0,
            ));
            painter.circle_filled(p(-7.0, 2.0), 3.2, color);
            painter.circle_filled(p(7.0, 2.0), 3.2, color);
            painter.add(egui::Shape::line(arc(p(0.0, 2.0), 7.0, 0.35, 2.8), s));
        }
        Icon::Guides => {
            let pts = [p(-9.0, 7.0), p(-2.0, -1.0), p(8.0, -7.0)];
            painter.add(egui::Shape::line(pts.to_vec(), s));
            for q in pts {
                painter.circle_filled(q, 3.0, color);
            }
        }
        Icon::Skeleton => bone_shape(painter, p(-8.0, 8.0), p(8.0, -8.0), color),
        Icon::Controls => {
            painter.circle_stroke(c, 9.5, s);
            painter.circle_stroke(c, 4.5, Stroke::new(1.2, color));
            painter.circle_filled(c, 1.8, color);
        }
        Icon::Mesh => {
            painter.circle_stroke(c, 10.0, s);
            let ellipse = |rx: f32, ry: f32| -> Vec<Pos2> {
                (0..=24)
                    .map(|i| {
                        let a = std::f32::consts::TAU * i as f32 / 24.0;
                        p(a.cos() * rx, a.sin() * ry)
                    })
                    .collect()
            };
            painter.add(egui::Shape::line(
                ellipse(10.0, 3.5),
                Stroke::new(1.2, color),
            ));
            painter.add(egui::Shape::line(
                ellipse(4.0, 10.0),
                Stroke::new(1.2, color),
            ));
        }
        Icon::Xray => {
            painter.circle_stroke(c, 10.0, s);
            let half: Vec<Pos2> = arc(
                c,
                10.0,
                std::f32::consts::FRAC_PI_2,
                1.5 * std::f32::consts::PI,
            );
            painter.add(egui::Shape::convex_polygon(
                half,
                color.gamma_multiply(0.45),
                Stroke::NONE,
            ));
            painter.line_segment([p(0.0, -10.0), p(0.0, 10.0)], Stroke::new(1.2, color));
        }
        Icon::Weights => {
            let mut drop = arc(p(0.0, 3.0), 7.0, -0.45, std::f32::consts::PI + 0.45);
            drop.reverse();
            drop.push(p(0.0, -11.0));
            painter.add(egui::Shape::closed_line(drop, s));
            painter.circle_filled(p(-2.5, 4.0), 2.0, color);
        }
        Icon::Undo | Icon::Redo => {
            let flip = if matches!(icon, Icon::Undo) {
                1.0
            } else {
                -1.0
            };
            let pts: Vec<Pos2> = arc(p(0.0, 3.0), 8.0, -0.4, std::f32::consts::PI)
                .into_iter()
                .map(|q| Pos2::new(c.x + (q.x - c.x) * flip, q.y))
                .collect();
            let tip = *pts.last().unwrap();
            painter.add(egui::Shape::line(pts, s));
            arrow_head(painter, tip + Vec2::new(0.0, 1.0), Vec2::new(0.0, 1.0), s);
        }
        Icon::Frame => {
            for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                let corner = p(10.0 * sx, 9.0 * sy);
                painter.line_segment([corner, corner - Vec2::new(5.0 * sx, 0.0)], s);
                painter.line_segment([corner, corner - Vec2::new(0.0, 5.0 * sy)], s);
            }
            painter.circle_filled(c, 2.5, color);
        }
        Icon::Export => {
            painter.add(egui::Shape::line(
                vec![p(-9.0, 1.0), p(-9.0, 9.0), p(9.0, 9.0), p(9.0, 1.0)],
                s,
            ));
            painter.line_segment([p(0.0, 4.0), p(0.0, -10.0)], s);
            arrow_head(painter, p(0.0, -10.0), Vec2::new(0.0, -1.0), s);
        }
    }
}

// --------------------------------------------------------------------------- rigging window art

#[derive(Clone, Copy)]
enum Art {
    Face,
    Eyes,
    Jaw,
    Ears,
    Fingers,
    Tail,
    Ponytail,
    Prop,
    Chain,
}

fn part_card(ui: &mut Ui, width: f32, part: &LibraryPart, added: bool) -> egui::Response {
    let height = 204.0;
    let sense = if added {
        Sense::hover()
    } else {
        Sense::click()
    };
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), sense);
    let hovered = response.hovered() && !added;
    let painter = ui.painter();
    painter.rect_filled(rect, 14, if hovered { CARD_HI } else { CARD });
    painter.rect_stroke(
        rect,
        14,
        Stroke::new(
            if hovered { 1.5 } else { 1.0 },
            if hovered {
                ACCENT.gamma_multiply(0.8)
            } else {
                BORDER
            },
        ),
        egui::StrokeKind::Inside,
    );
    let art = Rect::from_min_size(rect.min + Vec2::splat(6.0), Vec2::new(width - 12.0, 128.0));
    paint_art_backdrop(painter, art, hovered);
    illustration(&painter.with_clip_rect(art), art, part.art, hovered);
    if added {
        let chip = Rect::from_min_size(
            art.right_top() + Vec2::new(-60.0, 8.0),
            Vec2::new(52.0, 20.0),
        );
        painter.rect_filled(chip, 10, OK.gamma_multiply(0.2));
        painter.text(
            chip.center(),
            Align2::CENTER_CENTER,
            "Added",
            FontId::proportional(11.0),
            OK,
        );
    } else {
        let plus = art.right_top() + Vec2::new(-16.0, 16.0);
        painter.circle_filled(
            plus,
            11.0,
            if hovered {
                ACCENT
            } else {
                Color32::from_black_alpha(90)
            },
        );
        let ink = if hovered {
            theme::ACCENT_TEXT
        } else {
            TEXT_DIM
        };
        painter.line_segment(
            [plus - Vec2::new(4.5, 0.0), plus + Vec2::new(4.5, 0.0)],
            Stroke::new(1.8, ink),
        );
        painter.line_segment(
            [plus - Vec2::new(0.0, 4.5), plus + Vec2::new(0.0, 4.5)],
            Stroke::new(1.8, ink),
        );
    }
    painter.text(
        Pos2::new(rect.left() + 14.0, art.bottom() + 12.0),
        Align2::LEFT_TOP,
        part.title,
        FontId::proportional(15.5),
        if added { TEXT_DIM } else { TEXT },
    );
    let galley = painter.layout(
        part.text.to_string(),
        FontId::proportional(12.0),
        TEXT_DIM,
        width - 28.0,
    );
    painter.galley(
        Pos2::new(rect.left() + 14.0, art.bottom() + 34.0),
        galley,
        TEXT_DIM,
    );
    if added {
        response.on_hover_text(format!("The rig already has {}", part.title.to_lowercase()))
    } else {
        response
            .on_hover_cursor(egui::CursorIcon::PointingHand)
            .on_hover_text(format!("Add {}", part.title.to_lowercase()))
    }
}

/// A soft spotlight behind an illustration.
fn paint_art_backdrop(painter: &egui::Painter, r: Rect, hot: bool) {
    painter.rect_filled(r, 10, Color32::from_rgb(29, 33, 42));
    let glow = if hot { 16 } else { 9 };
    for (k, a) in [(0.62f32, glow / 2), (0.45, glow), (0.3, glow + 4)] {
        painter.circle_filled(
            r.center() + Vec2::new(0.0, r.height() * 0.08),
            r.height() * k,
            Color32::from_rgba_unmultiplied(255, 214, 140, a),
        );
    }
    // A floor line under the figure.
    let y = r.bottom() - r.height() * 0.07;
    painter.line_segment(
        [
            Pos2::new(r.left() + 20.0, y),
            Pos2::new(r.right() - 20.0, y),
        ],
        Stroke::new(1.0, Color32::from_white_alpha(14)),
    );
}

const SKIN: Color32 = Color32::from_rgb(68, 76, 92);
const BONE: Color32 = Color32::from_rgb(140, 148, 166);

/// A joint dot coloured by side, like the guides in the view.
fn joint(painter: &egui::Painter, at: Pos2, x: f32, r: f32) {
    let color = if x.abs() < 0.5 {
        theme::CENTER
    } else if x > 0.0 {
        theme::LEFT
    } else {
        theme::RIGHT
    };
    painter.circle_filled(at, r + 1.0, Color32::from_black_alpha(160));
    painter.circle_filled(at, r, color.gamma_multiply(0.85));
}

/// A highlighted chain: glow, honey line and joint rings.
fn hot_chain(painter: &egui::Painter, pts: &[Pos2], width: f32, hot: bool) {
    let honey = if hot {
        Color32::from_rgb(255, 205, 100)
    } else {
        ACCENT
    };
    painter.add(egui::Shape::line(
        pts.to_vec(),
        Stroke::new(width + 6.0, ACCENT.gamma_multiply(0.18)),
    ));
    painter.add(egui::Shape::line(pts.to_vec(), Stroke::new(width, honey)));
    for p in pts {
        painter.circle_filled(*p, width + 1.6, Color32::from_rgb(29, 33, 42));
        painter.circle_stroke(*p, width + 1.6, Stroke::new(1.6, honey));
    }
}

/// Illustrations for the rigging window: a small mannequin, or a bust for face
/// parts, with the part the component adds picked out in honey.
fn illustration(painter: &egui::Painter, r: Rect, art: Art, hot: bool) {
    match art {
        Art::Face | Art::Eyes | Art::Jaw | Art::Ears | Art::Ponytail => bust(painter, r, art, hot),
        Art::Fingers | Art::Tail | Art::Prop | Art::Chain => figure(painter, r, art, hot),
    }
}

fn figure(painter: &egui::Painter, r: Rect, art: Art, hot: bool) {
    // A 100-unit-tall mannequin facing the viewer: its left is on screen right.
    let k = r.height() * 0.84 / 100.0;
    let origin = Pos2::new(r.center().x, r.top() + r.height() * 0.06);
    let pt = |x: f32, y: f32| origin + Vec2::new(x * k, y * k);
    let limb = Stroke::new(5.5 * k, SKIN);
    // Body.
    painter.add(egui::Shape::convex_polygon(
        vec![
            pt(-12.0, 20.0),
            pt(12.0, 20.0),
            pt(9.5, 38.0),
            pt(8.0, 53.0),
            pt(-8.0, 53.0),
            pt(-9.5, 38.0),
        ],
        SKIN,
        Stroke::NONE,
    ));
    painter.circle_filled(pt(0.0, 9.0), 7.5 * k, SKIN);
    painter.line_segment([pt(0.0, 14.0), pt(0.0, 21.0)], Stroke::new(4.0 * k, SKIN));
    let arm = |s: f32| [pt(12.0 * s, 21.0), pt(23.0 * s, 34.0), pt(32.0 * s, 46.0)];
    let leg = |s: f32| {
        [
            pt(5.5 * s, 52.0),
            pt(6.5 * s, 74.0),
            pt(7.0 * s, 94.0),
            pt(9.5 * s, 98.0),
        ]
    };
    for s in [-1.0f32, 1.0] {
        let a = arm(s);
        painter.add(egui::Shape::line(a.to_vec(), limb));
        painter.circle_filled(a[2], 3.2 * k, SKIN);
        let l = leg(s);
        painter.add(egui::Shape::line(l.to_vec(), Stroke::new(6.5 * k, SKIN)));
    }
    // Skeleton.
    let bone = Stroke::new(1.2, BONE);
    let spine = [
        pt(0.0, 52.0),
        pt(0.0, 37.0),
        pt(0.0, 22.0),
        pt(0.0, 16.0),
        pt(0.0, 9.0),
    ];
    painter.add(egui::Shape::line(spine.to_vec(), bone));
    for s in [-1.0f32, 1.0] {
        let a = arm(s);
        painter.add(egui::Shape::line(
            vec![pt(0.0, 22.0), a[0], a[1], a[2]],
            bone,
        ));
        let l = leg(s);
        painter.add(egui::Shape::line(
            vec![pt(0.0, 52.0), l[0], l[1], l[2], l[3]],
            bone,
        ));
    }
    let dot = 2.4;
    for (x, y) in [
        (0.0, 52.0),
        (0.0, 37.0),
        (0.0, 22.0),
        (0.0, 16.0),
        (0.0, 9.0),
    ] {
        joint(painter, pt(x, y), x, dot);
    }
    for s in [-1.0f32, 1.0] {
        for (x, y) in [
            (12.0, 21.0),
            (23.0, 34.0),
            (32.0, 46.0),
            (5.5, 52.0),
            (6.5, 74.0),
            (7.0, 94.0),
        ] {
            joint(painter, pt(x * s, y), x * s, dot);
        }
    }

    match art {
        Art::Fingers => {
            for s in [-1.0f32, 1.0] {
                let wrist = pt(32.0 * s, 46.0);
                painter.circle_filled(
                    wrist + Vec2::new(2.0 * s * k, 5.0 * k),
                    9.0 * k,
                    ACCENT.gamma_multiply(0.16),
                );
                for (dx, dy) in [(-1.5, 6.5), (1.0, 8.5), (3.2, 8.8), (5.2, 8.0), (6.8, 6.2)] {
                    let base = wrist + Vec2::new(1.5 * s * k, 2.5 * k);
                    let tip = wrist + Vec2::new(dx * s * k, dy * k);
                    let mid = base.lerp(tip, 0.5);
                    hot_chain(painter, &[base, mid, tip], 1.0, hot);
                }
            }
        }
        Art::Tail => {
            let pts: Vec<Pos2> = [
                (3.0, 55.0),
                (9.0, 63.0),
                (16.0, 68.0),
                (23.0, 68.0),
                (28.0, 64.0),
                (31.0, 58.0),
            ]
            .into_iter()
            .map(|(x, y)| pt(x, y))
            .collect();
            hot_chain(painter, &pts, 2.2, hot);
        }
        Art::Prop => {
            let hand = pt(-32.0, 46.0);
            let tip = pt(-46.0, 14.0);
            let dir = (tip - hand).normalized();
            let n = Vec2::new(-dir.y, dir.x);
            let guard = hand + dir * 5.0 * k;
            painter.add(egui::Shape::line(
                vec![hand, tip],
                Stroke::new(8.0, ACCENT.gamma_multiply(0.18)),
            ));
            painter.add(egui::Shape::convex_polygon(
                vec![guard + n * 2.2 * k, tip, guard - n * 2.2 * k],
                if hot {
                    Color32::from_rgb(255, 205, 100)
                } else {
                    ACCENT
                },
                Stroke::NONE,
            ));
            painter.line_segment(
                [guard + n * 5.0 * k, guard - n * 5.0 * k],
                Stroke::new(2.5, ACCENT),
            );
            painter.circle_stroke(hand, 4.5, Stroke::new(1.8, ACCENT));
        }
        Art::Chain => {
            let pts: Vec<Pos2> = [
                (13.0, 22.0),
                (17.0, 29.0),
                (19.5, 36.5),
                (20.5, 44.0),
                (20.0, 51.0),
            ]
            .into_iter()
            .map(|(x, y)| pt(x + 4.0, y))
            .collect();
            hot_chain(painter, &pts, 1.8, hot);
        }
        _ => {}
    }
}

fn bust(painter: &egui::Painter, r: Rect, art: Art, hot: bool) {
    let rad = r.height() * 0.26;
    let hc = r.center() - Vec2::new(0.0, r.height() * 0.1);
    let p = |x: f32, y: f32| hc + Vec2::new(x * rad, y * rad);
    // Shoulders, neck and head.
    painter.add(egui::Shape::convex_polygon(
        vec![p(-1.7, 1.55), p(1.7, 1.55), p(2.6, 2.6), p(-2.6, 2.6)],
        SKIN,
        Stroke::NONE,
    ));
    painter.rect_filled(Rect::from_min_max(p(-0.38, 0.6), p(0.38, 1.7)), 4, SKIN);
    painter.circle_filled(hc, rad, SKIN);
    // Neck and head bones.
    let bone = Stroke::new(1.2, BONE);
    painter.line_segment([p(0.0, 1.6), p(0.0, 0.25)], bone);
    painter.line_segment([p(0.0, 0.25), p(0.0, -0.95)], bone);
    joint(painter, p(0.0, 1.6), 0.0, 2.6);
    joint(painter, p(0.0, 0.25), 0.0, 2.6);

    match art {
        Art::Face => {
            // A joint on each brow, lid, cheek and lip, like the face guides.
            for sx in [-1.0f32, 1.0] {
                let brow = [p(0.18 * sx, -0.42), p(0.4 * sx, -0.5), p(0.62 * sx, -0.42)];
                hot_chain(painter, &brow, 1.4, hot);
                painter.circle_stroke(
                    p(0.4 * sx, -0.14),
                    0.15 * rad,
                    Stroke::new(1.0, Color32::from_rgb(28, 30, 36)),
                );
                for y in [-0.3, 0.02] {
                    joint(painter, p(0.4 * sx, y), sx, 2.4);
                }
                joint(painter, p(0.55 * sx, 0.3), sx, 2.6);
            }
            joint(painter, p(0.0, 0.18), 0.0, 2.6);
            let mouth = [p(-0.3, 0.6), p(0.0, 0.53), p(0.3, 0.6)];
            painter.add(egui::Shape::line(
                mouth.to_vec(),
                Stroke::new(1.4, Color32::from_rgb(28, 30, 36)),
            ));
            hot_chain(painter, &mouth, 1.4, hot);
            hot_chain(painter, &[p(0.0, 0.68)], 1.4, hot);
        }
        Art::Eyes => {
            let target = p(1.6, -1.15);
            for sx in [-1.0f32, 1.0] {
                let eye = p(0.36 * sx, -0.08);
                painter.extend(egui::Shape::dashed_line(
                    &[eye, target],
                    Stroke::new(1.2, ACCENT.gamma_multiply(0.75)),
                    4.0,
                    3.0,
                ));
                painter.circle_filled(eye, 0.19 * rad, Color32::from_rgb(222, 226, 235));
                painter.circle_filled(
                    eye + (target - eye).normalized() * 0.07 * rad,
                    0.09 * rad,
                    Color32::from_rgb(28, 30, 36),
                );
                painter.circle_stroke(
                    eye,
                    0.19 * rad + 2.0,
                    Stroke::new(
                        1.6,
                        if hot {
                            Color32::from_rgb(255, 205, 100)
                        } else {
                            ACCENT
                        },
                    ),
                );
            }
            let d = 6.0;
            painter.add(egui::Shape::convex_polygon(
                vec![
                    target + Vec2::new(0.0, -d),
                    target + Vec2::new(d, 0.0),
                    target + Vec2::new(0.0, d),
                    target + Vec2::new(-d, 0.0),
                ],
                ACCENT,
                Stroke::new(1.0, theme::ACCENT_TEXT),
            ));
        }
        Art::Jaw => {
            let outline: Vec<Pos2> = [
                (-0.86, 0.05),
                (-0.62, 0.72),
                (0.0, 1.05),
                (0.62, 0.72),
                (0.86, 0.05),
            ]
            .into_iter()
            .map(|(x, y)| p(x, y + 0.08))
            .collect();
            painter.add(egui::Shape::line(
                outline.clone(),
                Stroke::new(7.0, ACCENT.gamma_multiply(0.16)),
            ));
            painter.add(egui::Shape::line(outline, Stroke::new(2.0, ACCENT)));
            painter.add(egui::Shape::line(
                arc(p(0.0, 0.55), 0.32 * rad, -2.6, -0.54),
                Stroke::new(1.6, Color32::from_rgb(28, 30, 36)),
            ));
            hot_chain(painter, &[p(0.0, 0.12), p(0.0, 1.05)], 2.0, hot);
        }
        Art::Ears => {
            for sx in [-1.0f32, 1.0] {
                let shape = vec![p(0.88 * sx, 0.2), p(1.55 * sx, -0.95), p(1.0 * sx, -0.45)];
                painter.add(egui::Shape::convex_polygon(
                    shape,
                    SKIN,
                    Stroke::new(1.0, ACCENT.gamma_multiply(0.6)),
                ));
                hot_chain(
                    painter,
                    &[
                        p(0.92 * sx, -0.05),
                        p(1.18 * sx, -0.48),
                        p(1.45 * sx, -0.85),
                    ],
                    1.6,
                    hot,
                );
            }
        }
        Art::Ponytail => {
            // A hair cap, then the tail swinging down one side.
            painter.add(egui::Shape::line(
                arc(hc, rad * 1.02, 0.2, std::f32::consts::PI - 0.2),
                Stroke::new(4.0, Color32::from_rgb(86, 94, 112)),
            ));
            let pts = [
                p(0.35, -1.0),
                p(0.95, -0.85),
                p(1.3, -0.3),
                p(1.42, 0.4),
                p(1.36, 1.1),
                p(1.5, 1.75),
            ];
            hot_chain(painter, &pts, 2.0, hot);
        }
        _ => {}
    }
}

// --------------------------------------------------------------------------- other widgets

fn component_row(
    ui: &mut Ui,
    id: &str,
    kind: Kind,
    side: Side,
    depth: usize,
    selected: bool,
) -> egui::Response {
    let height = 28.0;
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
        FontId::proportional(13.5),
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
        FontId::proportional(11.5),
        TEXT_FAINT,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
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
    let ink = Stroke::new(2.0, theme::ACCENT_TEXT);
    let top = Pos2::new(c.x, body.top() + 5.0);
    painter.line_segment([top, Pos2::new(c.x, body.bottom() - 4.0)], ink);
    painter.line_segment([Pos2::new(c.x - 4.0, body.top() + 9.0), top], ink);
    painter.line_segment([Pos2::new(c.x + 4.0, body.top() + 9.0), top], ink);
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
        .fill(Color32::from_rgba_unmultiplied(19, 22, 28, 230))
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
    let top = Color32::from_rgb(38, 42, 52);
    let bottom = Color32::from_rgb(15, 17, 21);
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
    let n: i32 = 12;
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
    painter.circle_filled(at, 32.0, Color32::from_rgba_unmultiplied(14, 16, 20, 150));
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
