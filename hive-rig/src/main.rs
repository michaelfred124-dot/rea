use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};

use hive_rig::components::{self, AddOptions, Guides, Kind, Preset};
use hive_rig::rig::detect_guides;
use hive_rig::skeleton::Side;
use hive_rig::template::{self, Template};
use hive_rig::{Rig, RigOptions, Target, build_rig, demo, export, import, mesh::Mesh, naming, rig};

#[derive(Parser)]
#[command(
    name = "hive-rig",
    version,
    about = "Automatic humanoid rigger with controls for Blender, Maya and Unreal"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Rig a humanoid mesh (.obj, .gltf or .glb; Y-up, facing +Z, T- or A-pose).
    Rig {
        input: PathBuf,
        /// Output directory.
        #[arg(short, long, default_value = "hive_out")]
        out: PathBuf,
        /// Rig name (defaults to the input file name).
        #[arg(short, long)]
        name: Option<String>,
        /// Build from this guides file instead of detecting the body.
        #[arg(short, long)]
        guides: Option<PathBuf>,
        /// Detect the body even when the file has a skeleton (by default its
        /// joints become the guides).
        #[arg(long)]
        ignore_joints: bool,
        #[command(flatten)]
        common: Common,
    },
    /// Rig the built-in test character to try the pipeline without an asset.
    Demo {
        #[arg(short, long, default_value = "hive_demo")]
        out: PathBuf,
        /// Lower the demo character's arms by this many degrees (45 = A-pose).
        #[arg(long, default_value_t = 0.0, conflicts_with = "quadruped")]
        arm_drop: f32,
        /// Rig the built-in dog instead: spine, neck and head, tail, two front
        /// legs and two three-segment hind legs (written as `hive_dog`).
        #[arg(long)]
        quadruped: bool,
        #[command(flatten)]
        common: Common,
    },
    /// Rebuild a rig from its (edited) guides, reusing the mesh in its .hive.json.
    Rebuild {
        /// The `<name>.hive.json` of the rig to rebuild.
        rig: PathBuf,
        /// Guides to build from (default: `<name>.guides.json` next to the rig).
        #[arg(short, long)]
        guides: Option<PathBuf>,
        /// Output directory (default: the rig's directory).
        #[arg(short, long)]
        out: Option<PathBuf>,
        #[command(flatten)]
        common: Common,
    },
    /// Add a premade component to a guides file.
    Add {
        guides: PathBuf,
        preset: Preset,
        /// Which hands get fingers (default: both).
        #[arg(long)]
        side: Option<SideArg>,
        /// Bone to attach a tail or chain to.
        #[arg(long)]
        parent: Option<String>,
        /// Bones in a tail or chain.
        #[arg(long)]
        joints: Option<usize>,
        /// Component id for a tail or chain.
        #[arg(long)]
        id: Option<String>,
        /// Curve controls on a tentacle (ends included, default 4).
        #[arg(long)]
        controls: Option<usize>,
    },
    /// Change the spine: its joint count, and whether a curve between the hips
    /// and chest controls drives it (a spline spine) instead of FK.
    Spine {
        guides: PathBuf,
        /// Spine joints (1 to 16).
        #[arg(long)]
        joints: Option<usize>,
        /// Drive the spine with a curve.
        #[arg(long, action = clap::ArgAction::Set)]
        spline: Option<bool>,
        /// Curve controls of a spline spine (ends included, default 3).
        #[arg(long)]
        controls: Option<usize>,
    },
    /// Hang a component from another bone (its guides stay where they are).
    Attach {
        guides: PathBuf,
        /// Component id.
        id: String,
        /// Bone to hang it from.
        bone: String,
    },
    /// Import the premade components of another guides file, hung from a bone.
    Import {
        guides: PathBuf,
        /// The guides file to take components from.
        from: PathBuf,
        /// Bone to hang the imported components from.
        #[arg(long)]
        parent: String,
    },
    /// Remove a component (and everything attached to it) from a guides file.
    Remove { guides: PathBuf, id: String },
    /// List the components in a guides file.
    Components { guides: PathBuf },
    /// Save a layout as a template, load one onto a layout, or list them.
    Template {
        #[command(subcommand)]
        action: TemplateCommand,
    },
    /// Show or set how a guides file names joints and controls on export.
    Naming {
        guides: PathBuf,
        /// game (Unreal mannequin), prefix (L_arm_jnt) or suffix (arm_L_JNT).
        preset: Option<String>,
        /// Joint pattern using {name}, {side} and {desc}.
        #[arg(long)]
        joint: Option<String>,
        /// Control pattern using {name}, {side} and {desc}.
        #[arg(long)]
        control: Option<String>,
        #[arg(long)]
        left: Option<String>,
        #[arg(long)]
        right: Option<String>,
        /// Centre side token (empty to leave it out).
        #[arg(long)]
        center: Option<String>,
    },
}

#[derive(Subcommand)]
enum TemplateCommand {
    /// Save the whole layout, or only some premade parts, as `<name>.template.json`.
    Save {
        guides: PathBuf,
        /// The template file (`<name>.template.json`), or a folder to save it in.
        out: PathBuf,
        /// Only these components (premade parts such as fingers, a tail or eyes).
        #[arg(long, value_delimiter = ',')]
        only: Option<Vec<String>>,
        /// Template name (default: the file name).
        #[arg(long)]
        name: Option<String>,
        /// A line describing the template.
        #[arg(long)]
        description: Option<String>,
    },
    /// Load a template onto a guides file: a whole layout replaces the guides
    /// (scaled to this body), parts are added (ids in use get a number).
    Apply {
        guides: PathBuf,
        /// A template file, or the name of a built-in or saved template.
        template: String,
        /// Bone to hang parts from whose parent isn't in the template.
        #[arg(long)]
        parent: Option<String>,
    },
    /// List the built-in templates and the ones in the template folders.
    List,
}

#[derive(Clone, Copy, ValueEnum)]
enum SideArg {
    Left,
    Right,
}

#[derive(clap::Args)]
struct Common {
    /// Which DCC builder scripts to write.
    #[arg(
        short,
        long,
        value_delimiter = ',',
        default_value = "blender,maya,unreal"
    )]
    targets: Vec<Target>,
    /// Skin weight smoothing passes.
    #[arg(long, default_value_t = 3)]
    smooth: usize,
}

fn run(mesh: Mesh, name: &str, guides: Guides, out: &Path, common: &Common) -> Result<()> {
    let mut opts = RigOptions::default();
    opts.skin.smooth_iterations = common.smooth;
    let rig = build_rig(mesh, name, guides, opts)?;
    let files = export::write_all(&rig, out, &common.targets)?;
    println!(
        "Rigged '{}': {} vertices, {} bones, {} controls, {} IK limbs",
        rig.name,
        rig.mesh.vertex_count(),
        rig.skeleton.bones.len(),
        rig.controls.controls.len(),
        rig.controls.ik_chains.len()
    );
    for f in files {
        println!("  {}", f.display());
    }
    Ok(())
}

fn detect(mesh: &Mesh) -> Guides {
    let (guides, d) = detect_guides(mesh);
    warn_undetected(&d);
    guides
}

fn warn_undetected(d: &hive_rig::landmarks::Detected) {
    for (found, what) in [(d.crotch, "legs"), (d.arms, "arms"), (d.neck, "neck")] {
        if !found {
            eprintln!("warning: could not detect the {what}; used average proportions instead");
        }
    }
}

/// Guides for a character file: from its joints when it has a skeleton,
/// otherwise detected on the mesh.
fn character(mesh: &Mesh, path: &Path) -> Result<Guides> {
    let joints = import::load_joints(path)?;
    let (guides, d, used) = rig::character_guides(mesh, &joints);
    match used {
        None => {
            warn_undetected(&d);
        }
        Some(Ok(report)) => {
            println!(
                "Read {} guides from the file's {} joints{}.",
                report.placed.len(),
                joints.len(),
                if report.added.is_empty() {
                    String::new()
                } else {
                    format!(" and added {}", report.added.join(", "))
                }
            );
            if !report.unused.is_empty() {
                println!(
                    "  not used: {}",
                    report
                        .unused
                        .iter()
                        .take(12)
                        .cloned()
                        .collect::<Vec<_>>()
                        .join(", ")
                        + if report.unused.len() > 12 {
                            ", ..."
                        } else {
                            ""
                        }
                );
            }
        }
        Some(Err(e)) => {
            eprintln!("warning: couldn't use the file's joints ({e:#}); detected the body instead");
            warn_undetected(&d);
        }
    }
    Ok(guides)
}

fn save_guides(path: &Path, g: &Guides) -> Result<()> {
    std::fs::write(path, serde_json::to_vec_pretty(g)?)
        .with_context(|| format!("writing {}", path.display()))
}

/// Opened by double-clicking (no arguments) or by dropping a file on the program:
/// ask for a character, rig it, show the output folder, and wait so the console
/// window doesn't close before it can be read.
fn guided(dropped: Option<PathBuf>) -> Result<()> {
    let result = guided_rig(dropped);
    if let Err(e) = &result {
        eprintln!("\nSomething went wrong: {e:#}");
    }
    println!("\nPress Enter to close this window.");
    let _ = std::io::stdin().read_line(&mut String::new());
    result
}

fn guided_rig(dropped: Option<PathBuf>) -> Result<()> {
    println!(
        "hive-rig {}: automatic rigger for Blender, Maya and Unreal\n",
        env!("CARGO_PKG_VERSION")
    );
    let input = match dropped {
        Some(p) => Some(p),
        None => {
            println!(
                "Drag your character file (.glb, .gltf or .obj) into this window and press Enter,"
            );
            println!("or just press Enter to rig the built-in demo character.");
            print!("> ");
            std::io::Write::flush(&mut std::io::stdout())?;
            let mut line = String::new();
            std::io::stdin().read_line(&mut line)?;
            // Dropping a file into a console pastes its path, quoted when it has spaces.
            let path = line.trim().trim_matches(|c| c == '"' || c == '\'');
            (!path.is_empty()).then(|| PathBuf::from(path))
        }
    };
    let common = Common {
        targets: Target::ALL.to_vec(),
        smooth: 3,
    };
    let out = match input {
        Some(input) => {
            let mesh = import::load_mesh(&input)?;
            mesh.validate()?;
            let stem = input
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "character".into());
            let out = input
                .parent()
                .unwrap_or(Path::new("."))
                .join(format!("{stem}_rig"));
            println!();
            let guides = character(&mesh, &input)?;
            run(mesh, &stem, guides, &out, &common)?;
            out
        }
        None => {
            let here = std::env::current_exe()?;
            let out = here.parent().unwrap_or(Path::new(".")).join("demo_rig");
            println!();
            let mesh = demo::humanoid(0.0);
            let guides = detect(&mesh);
            run(mesh, "hive_demo", guides, &out, &common)?;
            out
        }
    };
    println!(
        "\nDone. Open the _blender.py, _maya.py or _unreal.py file from this folder in that program:"
    );
    println!("  {}", out.display());
    if cfg!(windows) {
        let _ = std::process::Command::new("explorer").arg(&out).spawn();
    }
    Ok(())
}

fn template_command(action: TemplateCommand) -> Result<()> {
    match action {
        TemplateCommand::Save {
            guides,
            out,
            only,
            name,
            description,
        } => {
            let g = Guides::load(&guides)?;
            let name = name.unwrap_or_else(|| {
                if out.is_dir() {
                    guides
                        .file_name()
                        .map(|f| f.to_string_lossy().into_owned())
                        .unwrap_or_default()
                        .trim_end_matches(".json")
                        .trim_end_matches(".guides")
                        .to_string()
                } else {
                    template::name_from_path(&out)
                }
            });
            let path = if out.is_dir() {
                template::file_for(&out, &name)
            } else {
                anyhow::ensure!(
                    out.to_string_lossy().ends_with(template::TEMPLATE_SUFFIX),
                    "name the template file <name>{}",
                    template::TEMPLATE_SUFFIX
                );
                out
            };
            let mut t = Template::from_guides(&g, only.as_deref(), &name)?;
            t.description = description.unwrap_or_default();
            t.save(&path)?;
            let what = t
                .components
                .iter()
                .map(|p| p.component.id.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            println!("Saved template '{}' ({what}) to {}", t.name, path.display());
            Ok(())
        }
        TemplateCommand::Apply {
            guides,
            template: spec,
            parent,
        } => {
            let mut g = Guides::load(&guides)?;
            let t = template::find(&spec)?;
            let applied = t.apply(&mut g, parent.as_deref())?;
            save_guides(&guides, &g)?;
            match t.scope {
                template::Scope::Layout => println!(
                    "Loaded the '{}' layout ({} components)",
                    t.name,
                    applied.added.len()
                ),
                template::Scope::Parts => {
                    println!("Added {} from '{}'", applied.summary(), t.name)
                }
            }
            Ok(())
        }
        TemplateCommand::List => {
            let (listed, problems) = template::list();
            for l in &listed {
                let scope = match l.template.scope {
                    template::Scope::Layout => "layout",
                    template::Scope::Parts => "parts",
                };
                let from = match &l.source {
                    template::Source::BuiltIn => "built in".to_string(),
                    template::Source::File(p) => p.display().to_string(),
                };
                println!("{:<22} {scope:<7} {from}", l.template.name);
            }
            for p in problems {
                eprintln!("warning: {p}");
            }
            let folders: Vec<String> = template::folders()
                .iter()
                .map(|d| d.display().to_string())
                .collect();
            println!("Template folders: {}", folders.join(", "));
            Ok(())
        }
    }
}

fn main() -> Result<()> {
    let args: Vec<std::ffi::OsString> = std::env::args_os().skip(1).collect();
    match args.as_slice() {
        [] => return guided(None),
        // A character file dropped onto the program.
        [file] if Path::new(file).is_file() => return guided(Some(PathBuf::from(file))),
        _ => {}
    }
    match Cli::parse().command {
        Command::Rig {
            input,
            out,
            name,
            guides,
            ignore_joints,
            common,
        } => {
            let mesh = import::load_mesh(&input)?;
            mesh.validate()?;
            let name = name.unwrap_or_else(|| {
                input
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "character".into())
            });
            let guides = match guides {
                Some(path) => Guides::load(&path)?,
                None if ignore_joints => detect(&mesh),
                None => character(&mesh, &input)?,
            };
            run(mesh, &name, guides, &out, &common)
        }
        Command::Demo {
            out,
            arm_drop: _,
            quadruped: true,
            common,
        } => {
            let guides = components::quadruped(demo::DOG_HEIGHT);
            run(demo::demo_quadruped(), "hive_dog", guides, &out, &common)
        }
        Command::Demo {
            out,
            arm_drop,
            quadruped: false,
            common,
        } => {
            let mesh = demo::humanoid(arm_drop);
            let guides = detect(&mesh);
            run(mesh, "hive_demo", guides, &out, &common)
        }
        Command::Rebuild {
            rig,
            guides,
            out,
            common,
        } => {
            let old = Rig::load(&rig)?;
            let dir = rig.parent().unwrap_or(Path::new("."));
            let guides_path =
                guides.unwrap_or_else(|| dir.join(format!("{}.guides.json", old.name)));
            let guides = Guides::load(&guides_path)?;
            let out = out.unwrap_or_else(|| dir.to_path_buf());
            run(old.mesh, &old.name, guides, &out, &common)
        }
        Command::Add {
            guides,
            preset,
            side,
            parent,
            joints,
            id,
            controls,
        } => {
            let mut g = Guides::load(&guides)?;
            let opts = AddOptions {
                side: side.map(|s| match s {
                    SideArg::Left => Side::Left,
                    SideArg::Right => Side::Right,
                }),
                parent,
                joints,
                id,
                controls,
            };
            let added = components::add_preset(&mut g, preset, &opts)?;
            save_guides(&guides, &g)?;
            println!("Added {}", added.join(", "));
            Ok(())
        }
        Command::Spine {
            guides,
            joints,
            spline,
            controls,
        } => {
            let mut g = Guides::load(&guides)?;
            let id = g
                .components
                .iter()
                .find(|c| c.kind == Kind::Spine)
                .map(|c| c.id.clone())
                .context("the layout has no spine")?;
            if let Some(n) = joints {
                components::set_spine_joints(&mut g, &id, n)?;
            }
            if spline.is_some() || controls.is_some() {
                let on = spline.unwrap_or_else(|| g.component(&id).unwrap().options.spline);
                components::set_spine_spline(&mut g, &id, on, controls)?;
            }
            save_guides(&guides, &g)?;
            let o = &g.component(&id).unwrap().options;
            let joints = o.joints.unwrap_or(components::SPINE_JOINTS);
            if o.spline {
                println!(
                    "{id}: {joints} joints on a curve with {} controls",
                    o.curve_controls(true)
                );
            } else {
                println!("{id}: {joints} joints with FK controls");
            }
            Ok(())
        }
        Command::Attach { guides, id, bone } => {
            let mut g = Guides::load(&guides)?;
            components::attach(&mut g, &id, &bone)?;
            save_guides(&guides, &g)?;
            println!("{id} now hangs from {bone}");
            Ok(())
        }
        Command::Import {
            guides,
            from,
            parent,
        } => {
            let mut g = Guides::load(&guides)?;
            let src = Guides::load(&from)?;
            let added = components::import(&mut g, &src, &parent)?;
            save_guides(&guides, &g)?;
            println!("Imported {} from {from:?} onto {parent}", added.join(", "));
            Ok(())
        }
        Command::Remove { guides, id } => {
            let mut g = Guides::load(&guides)?;
            let removed = components::remove(&mut g, &id)?;
            save_guides(&guides, &g)?;
            println!("Removed {}", removed.join(", "));
            Ok(())
        }
        Command::Components { guides } => {
            let g = Guides::load(&guides)?;
            for c in &g.components {
                let kind = format!("{:?}", c.kind).to_lowercase();
                let parent = c.parent.as_deref().unwrap_or("-");
                let extra = match c.kind {
                    Kind::Chain => format!("{} bones", c.guides.len() - 1),
                    Kind::Spline => format!(
                        "{} bones, {} curve controls",
                        c.options.joints.unwrap_or(c.guides.len() - 1),
                        c.options.curve_controls(false)
                    ),
                    Kind::Spine if c.options.spline => format!(
                        "{} joints on a curve ({} controls)",
                        c.options.joints.unwrap_or(components::SPINE_JOINTS),
                        c.options.curve_controls(true)
                    ),
                    Kind::Spine => format!(
                        "{} joints",
                        c.options.joints.unwrap_or(components::SPINE_JOINTS)
                    ),
                    _ => String::new(),
                };
                println!("{:<12} {:<7} on {:<12} {extra}", c.id, kind, parent);
            }
            Ok(())
        }
        Command::Template { action } => template_command(action),
        Command::Naming {
            guides,
            preset,
            joint,
            control,
            left,
            right,
            center,
        } => {
            let mut g = Guides::load(&guides)?;
            let mut n = match &preset {
                Some(p) => p.parse::<naming::Preset>()?.naming(),
                None => g.naming.clone(),
            };
            let mut changed = preset.is_some();
            for (field, value) in [
                (&mut n.joint, joint),
                (&mut n.control, control),
                (&mut n.left, left),
                (&mut n.right, right),
                (&mut n.center, center),
            ] {
                if let Some(v) = value {
                    *field = v;
                    changed = true;
                }
            }
            if changed {
                n.validate()?;
                g.naming = n.clone();
                save_guides(&guides, &g)?;
            }
            let label = n.preset().map_or("custom", |p| p.key());
            println!("naming: {label}");
            println!(
                "  joints:   {}  (upperarm_l -> {})",
                n.joint,
                n.bone("upperarm_l")
            );
            println!(
                "  controls: {}  (upperarm_l_fk_ctrl -> {})",
                n.control,
                n.control("upperarm_l_fk_ctrl")
            );
            Ok(())
        }
    }
}
