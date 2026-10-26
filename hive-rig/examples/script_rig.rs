//! Build a rig from a script with the hive-rig library instead of the CLI.
//!
//! ```text
//! cargo run --example script_rig -- [MESH] [OUT_DIR]
//! ```
//!
//! With no mesh (or `-`) it uses the built-in test character. It detects the guides,
//! adds fingers, a tail and eyes, checks the layout, builds the skeleton and
//! controls on their own (`components::build`), then the whole rig with its
//! skin (`build_rig`), and writes the .glb, .hive.json, .guides.json, the
//! Blender, Maya and Unreal builder scripts and an .fbx (`export::write_all`).

use std::path::PathBuf;

use anyhow::Result;
use hive_rig::components::{self, AddOptions, Preset};
use hive_rig::{RigOptions, Target, build_rig, checks, demo, export, import, rig};

fn main() -> Result<()> {
    let mut args = std::env::args_os().skip(1);
    let mesh_path = args.next().filter(|a| a != "-").map(PathBuf::from);
    let out = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("hive_script_rig"));

    // 1. A mesh, in hive-rig's frame (Y up, metres, facing +Z).
    let (mesh, name) = match &mesh_path {
        Some(p) => {
            let ch = import::load_character(p, Default::default())?;
            let name = p.file_stem().unwrap().to_string_lossy().into_owned();
            (ch.mesh, name)
        }
        None => (demo::demo_humanoid(), "scripted".to_string()),
    };

    // 2. Guides: detected on the mesh, then premade parts added.
    let (mut guides, _detected) = rig::detect_guides(&mesh);
    components::add_preset(&mut guides, Preset::Fingers, &AddOptions::default())?;
    components::add_preset(
        &mut guides,
        Preset::Tail,
        &AddOptions {
            joints: Some(6),
            ..Default::default()
        },
    )?;
    components::add_preset(&mut guides, Preset::Eyes, &AddOptions::default())?;

    // 3. The pre-build checks, as `hive-rig check` runs them.
    let mut surface = checks::Surface::new(&mesh);
    let issues = checks::check(&guides, Some(&mut surface));
    for i in &issues {
        println!("check: {} ({})", i.message, i.suggestion);
    }
    anyhow::ensure!(checks::errors(&issues) == 0, "the guides don't build");

    // 4. The skeleton and control rig alone, without skinning.
    let built = components::build(&guides)?;
    println!(
        "{} bones, {} controls",
        built.skeleton.bones.len(),
        built.controls.controls.len()
    );

    // 5. The whole rig, skinned, and every output.
    let r = build_rig(mesh, &name, guides, RigOptions::default())?;
    for path in export::write_all(&r, &out, &Target::ALL)? {
        println!("wrote {}", path.display());
    }
    Ok(())
}
