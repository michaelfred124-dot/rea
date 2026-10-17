use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use clap::{Parser, Subcommand, ValueEnum};

use hive_rig::components::{self, AddOptions, Guides, Kind, Preset};
use hive_rig::rig::detect_guides;
use hive_rig::skeleton::Side;
use hive_rig::{Rig, RigOptions, Target, build_rig, demo, export, import, mesh::Mesh};

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
        #[command(flatten)]
        common: Common,
    },
    /// Rig the built-in test character to try the pipeline without an asset.
    Demo {
        #[arg(short, long, default_value = "hive_demo")]
        out: PathBuf,
        /// Lower the demo character's arms by this many degrees (45 = A-pose).
        #[arg(long, default_value_t = 0.0)]
        arm_drop: f32,
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
    },
    /// Remove a component (and everything attached to it) from a guides file.
    Remove { guides: PathBuf, id: String },
    /// List the components in a guides file.
    Components { guides: PathBuf },
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
    for (found, what) in [(d.crotch, "legs"), (d.arms, "arms"), (d.neck, "neck")] {
        if !found {
            eprintln!("warning: could not detect the {what}; used average proportions instead");
        }
    }
    guides
}

fn save_guides(path: &Path, g: &Guides) -> Result<()> {
    std::fs::write(path, serde_json::to_vec_pretty(g)?)
        .with_context(|| format!("writing {}", path.display()))
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Rig {
            input,
            out,
            name,
            guides,
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
                None => detect(&mesh),
            };
            run(mesh, &name, guides, &out, &common)
        }
        Command::Demo {
            out,
            arm_drop,
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
            };
            let added = components::add_preset(&mut g, preset, &opts)?;
            save_guides(&guides, &g)?;
            println!("Added {}", added.join(", "));
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
                    Kind::Spine => format!(
                        "{} joints",
                        c.options.joints.unwrap_or(components::SPINE_JOINTS)
                    ),
                    _ => String::new(),
                };
                println!("{:<12} {:<6} on {:<12} {extra}", c.id, kind, parent);
            }
            Ok(())
        }
    }
}
