use std::path::{Path, PathBuf};

use anyhow::Result;
use clap::{Parser, Subcommand};

use hive_rig::{RigOptions, Target, auto_rig, demo, export, import, mesh::Mesh};

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

fn run(mesh: Mesh, name: &str, out: &Path, common: &Common) -> Result<()> {
    let mut opts = RigOptions::default();
    opts.skin.smooth_iterations = common.smooth;
    let rig = auto_rig(mesh, name, opts)?;
    let d = &rig.landmarks.detected;
    for (found, what) in [(d.crotch, "legs"), (d.arms, "arms"), (d.neck, "neck")] {
        if !found {
            eprintln!("warning: could not detect the {what}; used average proportions instead");
        }
    }
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

fn main() -> Result<()> {
    match Cli::parse().command {
        Command::Rig {
            input,
            out,
            name,
            common,
        } => {
            let mesh = import::load_mesh(&input)?;
            let name = name.unwrap_or_else(|| {
                input
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "character".into())
            });
            run(mesh, &name, &out, &common)
        }
        Command::Demo {
            out,
            arm_drop,
            common,
        } => run(demo::humanoid(arm_drop), "hive_demo", &out, &common),
    }
}
