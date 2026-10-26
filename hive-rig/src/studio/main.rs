//! Hive Rig: the desktop rigging app. Double-click it, open a character, adjust
//! the guides, build and export.
//!
//! `HiveRig [character, rig or guides file] [--demo [dog|bird|spider|snake|horse|dinosaur]] [--build] [--weights BONE] [--paint] [--dab add|subtract|replace|scale|smooth|mend]
//! [--lock BONE] [--bend DEGREES] [--bend-axis x|y|z] [--tools] [--vertices] [--hover] [--library] [--templates] [--export]
//! [--select COMPONENT[,COMPONENT...]] [--pick NAME] [--pose BONE[=DEGREES[@x|y|z]]] [--set CONTROL=TX,TY,TZ,RX,RY,RZ]... [--attr CONTROL.NAME=VALUE]... [--shape NAME=WEIGHT]... [--feathers COUNT[,ROWS]]
//! [--corrective] [--inbetween WEIGHT] [--sculpt grab|smooth|relax|inflate|erase] [--all-bones]
//! [--bone-filter TEXT] [--add-demo [dog|bird|...]] [--build-all] [--character NAME]
//! [--import-shape FILE] [--line-width WIDTH] [--draw-shape] [--mode guides|skeleton|rig] [--joint-size SCALE] [--no-joint-xray] [--joints-menu] [--no-toast] [--no-outliner] [--view front|side|top|persp] [--zoom FACTOR] [--look X,Y,Z] [--xray] [--screenshot OUT.ppm]`
//! (the flags exist so the window can be checked automatically).

// No console window behind the app on Windows.
#![cfg_attr(windows, windows_subsystem = "windows")]

mod app;
mod joints;
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
    /// `--feathers COUNT[,ROWS]`: feathers on the demo creature's wings.
    feathers: Option<(usize, usize)>,
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
    /// `--set CONTROL=tx,ty,tz,rx,ry,rz`: pose a control and pick it (repeatable).
    set: Vec<String>,
    /// `--attr CONTROL.NAME=VALUE`: set a control's attribute (repeatable).
    attrs: Vec<String>,
    /// `--tool move|rotate|scale`: the manipulator shown on the picked controls.
    tool: Option<String>,
    /// `--shape NAME=WEIGHT`: set a shape's weight and pick it (repeatable).
    shapes: Vec<String>,
    /// Make a corrective for the `--pose` and push its skin out a little.
    corrective: bool,
    /// Give the picked shape an in-between at this weight and sculpt it.
    inbetween: Option<f32>,
    /// Turn the sculpt tool on with this brush, shown on the picked shape.
    sculpt: Option<String>,
    /// Colour the mesh by every bone at once.
    all_bones: bool,
    /// Search the bone list for this.
    bone_filter: Option<String>,
    /// `--mode guides|skeleton|rig`: the stage the viewport shows.
    mode: Option<String>,
    /// `--joint-size SCALE`: the joint display size.
    joint_size: Option<f32>,
    /// `--no-joint-xray`: let the mesh hide the joints.
    no_joint_xray: bool,
    /// Open the joint display menu.
    joints_menu: bool,
    /// Leave the status notification out of the screenshot.
    no_toast: bool,
    /// `--no-outliner`: start with the outliner folded away.
    no_outliner: bool,
    /// `--view front|side|top|persp`: turn the camera as the camera pill does.
    view: Option<String>,
    /// `--zoom FACTOR`: move the camera in by this factor after framing.
    zoom: Option<f32>,
    /// `--look X,Y,Z`: aim the camera at this point (model space, metres).
    look: Option<[f32; 3]>,
    /// See through the mesh.
    xray: bool,
    /// `--add-demo [creature]`: add a demo character to the scene (repeatable).
    add_demo: Vec<Option<hive_rig::demo::Creature>>,
    /// Build every character of the scene.
    build_all: bool,
    /// `--character NAME`: edit this character of the scene.
    character: Option<String>,
    /// `--import-shape FILE`: import a control shape and put it on the `--pick` control.
    import_shape: Option<PathBuf>,
    /// `--line-width WIDTH`: the `--pick` control's line width.
    line_width: Option<f32>,
    /// Open the shape editor with a shape half drawn, for the `--pick` control.
    draw_shape: bool,
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
            Some("--feathers") => {
                launch.feathers = args.next().and_then(|s| {
                    let s = s.to_str()?.to_string();
                    let (n, rows) = s.split_once(',').unwrap_or((&s, "1"));
                    Some((n.trim().parse().ok()?, rows.trim().parse().ok()?))
                })
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
            Some("--set") => launch
                .set
                .extend(args.next().map(|s| s.to_string_lossy().into_owned())),
            Some("--attr") => launch
                .attrs
                .extend(args.next().map(|s| s.to_string_lossy().into_owned())),
            Some("--tool") => launch.tool = args.next().map(|s| s.to_string_lossy().into_owned()),
            Some("--screenshot") => launch.screenshot = args.next().map(PathBuf::from),
            Some("--corrective") => launch.corrective = true,
            Some("--inbetween") => {
                launch.inbetween = args.next().and_then(|s| s.to_str()?.parse().ok())
            }
            Some("--sculpt") => {
                launch.sculpt = args.next().map(|s| s.to_string_lossy().into_owned())
            }
            Some("--all-bones") => launch.all_bones = true,
            Some("--bone-filter") => {
                launch.bone_filter = args.next().map(|s| s.to_string_lossy().into_owned())
            }
            Some("--mode") => launch.mode = args.next().map(|s| s.to_string_lossy().into_owned()),
            Some("--joint-size") => {
                launch.joint_size = args.next().and_then(|s| s.to_str()?.parse().ok())
            }
            Some("--no-joint-xray") => launch.no_joint_xray = true,
            Some("--joints-menu") => launch.joints_menu = true,
            Some("--no-toast") => launch.no_toast = true,
            Some("--no-outliner") => launch.no_outliner = true,
            Some("--xray") => launch.xray = true,
            Some("--zoom") => launch.zoom = args.next().and_then(|s| s.to_str()?.parse().ok()),
            Some("--look") => {
                launch.look = args.next().and_then(|s| {
                    let v: Vec<f32> = s
                        .to_str()?
                        .split(',')
                        .filter_map(|x| x.trim().parse().ok())
                        .collect();
                    (v.len() == 3).then(|| [v[0], v[1], v[2]])
                })
            }
            Some("--view") => launch.view = args.next().map(|s| s.to_string_lossy().into_owned()),
            Some("--add-demo") => {
                let creature = args
                    .peek()
                    .and_then(|n| hive_rig::demo::Creature::from_name(n.to_str()?));
                if creature.is_some() {
                    args.next();
                }
                launch.add_demo.push(creature);
            }
            Some("--build-all") => launch.build_all = true,
            Some("--character") => {
                launch.character = args.next().map(|s| s.to_string_lossy().into_owned())
            }
            Some("--import-shape") => launch.import_shape = args.next().map(PathBuf::from),
            Some("--line-width") => {
                launch.line_width = args.next().and_then(|s| s.to_str()?.parse().ok())
            }
            Some("--draw-shape") => launch.draw_shape = true,
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
        Some(c) => match launch.feathers {
            Some((n, rows)) => studio.load_feathered_creature(c, n, rows),
            None => studio.load_creature(c),
        },
        None if launch.demo => studio.load_demo(),
        None => {}
    }
    if let Some(file) = &launch.file {
        studio.open(file);
    }
    for creature in &launch.add_demo {
        studio.add_demo_character(*creature);
    }
    if launch.build_all {
        studio.build_all();
    }
    if let Some(name) = &launch.character
        && let Some(i) = studio.character_index(name)
    {
        studio.switch_character(i);
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
        studio.skin_tools.inspect = spot;
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
    if launch.corrective && studio.new_corrective() {
        // Push the skin around the posed joint out, so the shape shows.
        let joint = studio.pose.as_ref().and_then(|p| {
            let rig = studio.doc.as_ref()?.rig.as_ref()?;
            let b = rig.skeleton.bone(&p.bone)?;
            Some((b.head, rig.guides.body.height))
        });
        if let Some((at, height)) = joint
            && studio.begin_sculpt()
        {
            studio.sculpt_at(at, glam::Vec3::new(0.0, 0.0, 0.012 * height));
        }
    }
    if let Some(w) = launch.inbetween {
        studio.add_inbetween_here_at(w);
    }
    if let Some(brush) = &launch.sculpt {
        use hive_rig::shapes::SculptBrush;
        studio.shape_tools.brush = SculptBrush::ALL
            .into_iter()
            .find(|b| b.label().eq_ignore_ascii_case(brush))
            .unwrap_or(SculptBrush::Grab);
        studio.set_mesh_tool(app::MeshTool::Sculpt);
        studio.show_sculpt_brush();
        studio.shape_tools.scroll_to = 4;
    }
    studio.skin_tools.color_all = launch.all_bones;
    if let Some(text) = launch.bone_filter {
        studio.skin_tools.bone_filter = text;
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
    if let Some(app::RigPick::Control(name)) = studio.rig_pick.clone() {
        if let Some(path) = &launch.import_shape {
            studio.import_custom_shape(path, Some(&name));
        }
        if let Some(w) = launch.line_width {
            studio.set_line_width(&name, w);
        }
        if launch.draw_shape {
            // A crown drawn half way: the left half is the mirror image.
            let mut draft = app::ShapeDraft::new(Some(name.clone()));
            draft.name = "crown".into();
            for p in [
                [0.0, -0.5],
                [0.75, -0.5],
                [0.875, 0.5],
                [0.5, 0.125],
                [0.25, 0.625],
                [0.0, 0.25],
            ] {
                draft.add_point(p);
            }
            studio.shape_draft = Some(draft);
        }
        if launch.build && (launch.import_shape.is_some() || launch.line_width.is_some()) {
            studio.build();
        }
        studio.mode = app::Mode::Rig;
        studio.rig_pick = Some(app::RigPick::Control(name));
    }
    for text in &launch.set {
        let (name, values) = text.split_once('=').unwrap_or((text, ""));
        let mut ch = studio.posing.pose.channels(name);
        for (i, v) in values.split(',').enumerate().take(9) {
            if let Ok(v) = v.trim().parse() {
                ch.set(i, v);
            }
        }
        studio.mode = app::Mode::Rig;
        studio.set_control_channels(name, ch);
        studio.select_control(name, true);
    }
    for text in &launch.attrs {
        let (target, value) = text.split_once('=').unwrap_or((text, "1"));
        if let Some((control, name)) = target.rsplit_once('.')
            && let Ok(v) = value.trim().parse()
            && !studio.set_control_attribute(control, name, v)
        {
            eprintln!("no attribute {name} on {control}");
        }
    }
    if let Some(tool) = &launch.tool {
        studio.mode = app::Mode::Rig;
        studio.posing.tool = match tool.as_str() {
            "move" => app::Tool::Move,
            "rotate" => app::Tool::Rotate,
            "scale" => app::Tool::Scale,
            _ => app::Tool::Select,
        };
    }
    if let Some(mode) = &launch.mode {
        studio.mode = match mode.as_str() {
            "guides" => app::Mode::Guides,
            "skeleton" => app::Mode::Skeleton,
            _ => app::Mode::Rig,
        };
    }
    if let Some(size) = launch.joint_size {
        studio.joints.size = size.clamp(app::JointDisplay::MIN_SIZE, app::JointDisplay::MAX_SIZE);
    }
    if launch.no_joint_xray {
        studio.joints.xray = false;
    }
    studio.display_open = launch.joints_menu;
    if let Some(view) = &launch.view {
        let (yaw, pitch) = match view.as_str() {
            "front" => (0.0, 0.05),
            "side" => (std::f32::consts::FRAC_PI_2, 0.05),
            "top" => (0.35, 1.1),
            _ => (0.55, 0.22),
        };
        studio.camera.yaw = yaw;
        studio.camera.pitch = pitch;
    }
    studio.xray |= launch.xray;
    if let Some(zoom) = launch.zoom.filter(|z| *z > 0.0) {
        studio.camera.dist /= zoom;
    }
    if let Some([x, y, z]) = launch.look {
        studio.camera.target = glam::Vec3::new(x, y, z);
    }
    // Several ids (comma separated) are picked in order, as with Shift+click.
    for id in launch.select.iter().flat_map(|s| s.split(',')) {
        if let Some(c) = studio.component_index(id) {
            studio.toggle_pick(c, None);
        }
    }

    if launch.no_outliner {
        studio.outliner_open = false;
    }
    if launch.no_toast {
        studio.status_serial = 0;
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
