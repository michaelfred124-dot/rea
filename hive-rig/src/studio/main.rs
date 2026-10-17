//! Hive Rig: the desktop rigging app. Double-click it, open a character, adjust
//! the guides, build and export.
//!
//! `HiveRig [character file] [--demo] [--build] [--weights BONE] [--screenshot OUT.ppm]`
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
    build: bool,
    weights: Option<String>,
    screenshot: Option<PathBuf>,
}

fn parse_args() -> Launch {
    let mut launch = Launch::default();
    let mut args = std::env::args_os().skip(1);
    while let Some(a) = args.next() {
        match a.to_str() {
            Some("--demo") => launch.demo = true,
            Some("--build") => launch.build = true,
            Some("--weights") => {
                launch.weights = args.next().map(|s| s.to_string_lossy().into_owned())
            }
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
    if let Some(file) = &launch.file {
        studio.open(file);
    } else if launch.demo {
        studio.load_demo();
    }
    if launch.build {
        studio.build();
    }
    studio.weights_bone = launch.weights;

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
