//! Hive Rig: the desktop rigging app. Double-click it, open a character, adjust
//! the guides, build and export.
//!
//! `HiveRig [character, rig or guides file] [--demo [dog|bird|spider|snake|horse|dinosaur]] [--build] [--weights BONE] [--paint] [--dab add|subtract|replace|scale|smooth|mend]
//! [--lock BONE] [--bend DEGREES] [--bend-axis x|y|z] [--tools] [--vertices] [--hover] [--library] [--templates] [--export]
//! [--select COMPONENT[,COMPONENT...]] [--pick NAME] [--pose BONE[=DEGREES[@x|y|z]]] [--shape NAME=WEIGHT]...
//! [--screenshot OUT.ppm]`
//! (the flags exist so the window can be checked automatically).

// No console window behind the app on Windows.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod theme;
mod view;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use eframe::egui;

#[derive(Default)]
struct Launch {
    file: Option<PathBuf>,
    demo: bool,
    /// `--demo dog|bird|spider|snake|horse|dinosaur`: open that creature as the demo.
    creature: Option<hive_rig::demo::Creature>,
    build: bool,
    weights: Option<String>,
    /// Turn the weight brush on and show it over the `--weights` bone.
    paint: bool,
    /// Paint one dab there: add, subtract, replace, scale, smooth or mend.
    dab: Option<String>,
    /// Lock these bones' weights.
    lock: Vec<String>,
    /// Bend the `--weights` bone this many degrees (the paint preview).
    bend: Option<f32>,
    /// About this world axis (0..2) instead of the one across the bone.
    bend_axis: Option<usize>,
    /// Scroll Properties down to the weight tools (or a picked control's look).
    tools: bool,
    /// Turn the select tool on and select the vertices around the brush spot.
    vertices: bool,
    /// Show the influences of the vertex at the brush spot, as on hover.
    hover: bool,
    library: bool,
    templates: bool,
    export: bool,
    select: Option<String>,
    pick: Option<String>,
    pose: Option<String>,
    /// `--shape NAME=WEIGHT`: set a shape's weight and pick it (repeatable).
    shapes: Vec<String>,
    screenshot: Option<PathBuf>,
}

fn parse_args() -> Launch {
    let mut launch = Launch::default();
    let mut args = std::env::args_os().skip(1).peekable();
    while let Some(a) = args.next() {
        match a.to_str() {
            Some("--demo") => {
                launch.demo = true;
                // `--demo bird`: a built-in creature instead of the humanoid.
                launch.creature = args
                    .peek()
                    .and_then(|n| hive_rig::demo::Creature::from_name(n.to_str()?));
                if launch.creature.is_some() {
                    args.next();
                }
            }
            Some("--build") => launch.build = true,
            Some("--weights") => {
                launch.weights = args.next().map(|s| s.to_string_lossy().into_owned())
            }
            Some("--paint") => launch.paint = true,
            Some("--dab") => {
                launch.paint = true;
                launch.dab = args.next().map(|s| s.to_string_lossy().into_owned());
            }
            Some("--lock") => {
                if let Some(b) = args.next() {
                    launch.lock.push(b.to_string_lossy().into_owned());
                }
            }
            Some("--bend") => launch.bend = args.next().and_then(|s| s.to_str()?.parse().ok()),
            Some("--bend-axis") => {
                launch.bend_axis = args
                    .next()
                    .and_then(|s| ["x", "y", "z"].iter().position(|a| s.to_str() == Some(a)))
            }
            Some("--tools") => launch.tools = true,
            Some("--vertices") => launch.vertices = true,
            Some("--hover") => launch.hover = true,
            Some("--library") => launch.library = true,
            Some("--templates") => {
                launch.library = true;
                launch.templates = true;
            }
            Some("--export") => launch.export = true,
            Some("--select") => {
                launch.select = args.next().map(|s| s.to_string_lossy().into_owned())
            }
            Some("--pick") => launch.pick = args.next().map(|s| s.to_string_lossy().into_owned()),
            Some("--pose") => launch.pose = args.next().map(|s| s.to_string_lossy().into_owned()),
            Some("--screenshot") => launch.screenshot = args.next().map(PathBuf::from),
            Some("--shape") => launch
                .shapes
                .extend(args.next().map(|s| s.to_string_lossy().into_owned())),
            _ => launch.file = Some(PathBuf::from(a)),
        }
    }
    launch
}

struct HiveRig {
    studio: app::Studio,
    screenshot: Option<PathBuf>,
    frames: u32,
    shot_requested: bool,
    shot_done: Arc<AtomicBool>,
}

impl eframe::App for HiveRig {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.studio.ui(ui);
        let Some(path) = &self.screenshot else { return };
        let ctx = ui.ctx().clone();
        self.frames += 1;
        if self.frames == 5 && !self.shot_requested {
            self.shot_requested = true;
            let path = path.clone();
            let done = self.shot_done.clone();
            let ctx2 = ctx.clone();
            ctx.request_screenshot(move |image| {
                if let Err(e) = write_ppm(&path, &image) {
                    eprintln!("screenshot failed: {e}");
                }
                done.store(true, Ordering::SeqCst);
                ctx2.request_repaint();
            });
        }
        if self.shot_done.load(Ordering::SeqCst) {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        ctx.request_repaint();
    }
}

fn write_ppm(path: &std::path::Path, image: &egui::ColorImage) -> std::io::Result<()> {
    let [w, h] = image.size;
    let mut out = format!("P6\n{w} {h}\n255\n").into_bytes();
    for p in &image.pixels {
        out.extend_from_slice(&[p.r(), p.g(), p.b()]);
    }
    std::fs::write(path, out)
}

fn main() -> eframe::Result {
    let launch = parse_args();
    let mut studio = app::Studio::default();
    // `--demo layout.guides.json` opens the demo, then the guides onto it.
    match launch.creature {
        Some(c) => studio.load_creature(c),
        None if launch.demo => studio.load_demo(),
        None => {}
    }
    if let Some(file) = &launch.file {
        studio.open(file);
    }
    if launch.build {
        studio.build();
    }
    studio.weights_bone = launch.weights;
    for bone in &launch.lock {
        studio.toggle_lock(bone);
    }
    if let Some(degrees) = launch.bend {
        studio.bend_paint_bone(degrees, launch.bend_axis);
    }
    if launch.paint || launch.vertices || launch.hover {
        studio.show_brush();
    }
    if let Some(mode) = &launch.dab {
        use hive_rig::skin::BrushMode;
        studio.brush.mode = match mode.as_str() {
            "subtract" => BrushMode::Subtract,
            "replace" => BrushMode::Replace,
            "scale" => BrushMode::Scale,
            "smooth" => BrushMode::Smooth,
            "mend" => BrushMode::Mend,
            _ => BrushMode::Add,
        };
        studio.brush.strength = 0.8;
        if let Some(at) = studio.brush_at
            && studio.begin_stroke()
        {
            studio.paint_at(at);
        }
    }
    // The vertex nearest the brush spot: selected with its neighbours, or hovered.
    let spot = studio.brush_at.and_then(|at| {
        let m = &studio.doc.as_ref()?.mesh.positions;
        (0..m.len()).min_by(|a, b| m[*a].distance(at).total_cmp(&m[*b].distance(at)))
    });
    if launch.vertices
        && let Some(v) = spot
    {
        studio.painting = false;
        studio.skin_tools.selecting = true;
        studio.select_vertices(&[v], app::SelectOp::Replace);
        studio.grow_selection();
    }
    if launch.hover {
        studio.painting = launch.paint;
        studio.skin_tools.hover = spot;
    }
    if launch.tools {
        studio.skin_tools.scroll_to_tools = 4;
    }
    studio.library_open = launch.library;
    if launch.templates {
        studio.category = app::Category::Templates;
    }
    studio.export_open = launch.export;
    if let Some(text) = &launch.pose {
        // BONE, BONE=DEGREES or BONE=DEGREES@x|y|z.
        let (bone, turn) = text.split_once('=').unwrap_or((text, "40"));
        let (degrees, axis) = turn.split_once('@').unwrap_or((turn, "x"));
        studio.start_pose(bone);
        if let Some(p) = studio.pose.as_mut() {
            p.degrees = degrees.trim().parse().unwrap_or(40.0);
            p.axis = match axis.trim() {
                "y" | "Y" => 1,
                "z" | "Z" => 2,
                _ => 0,
            };
        }
    }
    for text in &launch.shapes {
        let (name, weight) = text.split_once('=').unwrap_or((text, "1"));
        studio.set_shape_weight(name, weight.trim().parse().unwrap_or(1.0));
        studio.shape_pick = Some(name.to_string());
    }
    if let Some(name) = &launch.pick {
        studio.rig_pick = Some(if name.starts_with("ctrl") || name.ends_with("_ctrl") {
            // Controls are picked in the controls stage.
            studio.mode = app::Mode::Rig;
            app::RigPick::Control(name.clone())
        } else {
            app::RigPick::Bone(name.clone())
        });
    }
    // Several ids (comma separated) are picked in order, as with Shift+click.
    for id in launch.select.iter().flat_map(|s| s.split(',')) {
        if let Some(c) = studio.component_index(id) {
            studio.toggle_pick(c, None);
        }
    }

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Hive Rig")
            .with_inner_size([1400.0, 860.0])
            .with_min_inner_size([900.0, 560.0])
            .with_drag_and_drop(true),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };
    eframe::run_native(
        "Hive Rig",
        options,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx);
            Ok(Box::new(HiveRig {
                studio,
                screenshot: launch.screenshot,
                frames: 0,
                shot_requested: false,
                shot_done: Arc::new(AtomicBool::new(false)),
            }))
        }),
    )
}
