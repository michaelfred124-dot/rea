//! Hive Rig: the desktop rigging app. Double-click it, open a character, adjust
//! the guides, build and export.
//!
//! `HiveRig [character, rig or guides file] [--demo [dog|bird|spider|snake]] [--build] [--weights BONE] [--paint] [--dab add|subtract|smooth] [--library] [--templates] [--export]
//! [--select COMPONENT[,COMPONENT...]] [--pick NAME] [--pose BONE] [--screenshot OUT.ppm]`
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
    /// `--demo dog|bird|spider|snake`: open that creature as the demo.
    creature: Option<hive_rig::demo::Creature>,
    build: bool,
    weights: Option<String>,
    /// Turn the weight brush on and show it over the `--weights` bone.
    paint: bool,
    /// Paint one dab there: add, subtract or smooth.
    dab: Option<String>,
    library: bool,
    templates: bool,
    export: bool,
    select: Option<String>,
    pick: Option<String>,
    pose: Option<String>,
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
    if launch.paint {
        studio.show_brush();
    }
    if let Some(mode) = &launch.dab {
        studio.brush.mode = match mode.as_str() {
            "subtract" => hive_rig::skin::BrushMode::Subtract,
            "smooth" => hive_rig::skin::BrushMode::Smooth,
            _ => hive_rig::skin::BrushMode::Add,
        };
        studio.brush.strength = 0.8;
        if let Some(at) = studio.brush_at
            && studio.begin_stroke()
        {
            studio.paint_at(at);
        }
    }
    studio.library_open = launch.library;
    if launch.templates {
        studio.category = app::Category::Templates;
    }
    studio.export_open = launch.export;
    if let Some(bone) = &launch.pose {
        studio.start_pose(bone);
        if let Some(p) = studio.pose.as_mut() {
            p.degrees = 40.0;
        }
    }
    if let Some(name) = &launch.pick {
        studio.rig_pick = Some(if name.starts_with("ctrl") || name.ends_with("_ctrl") {
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
