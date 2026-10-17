//! Writers for the interchange mesh and the per-DCC control-rig builders.
//!
//! Skeleton and skin travel as glTF (open, imported natively by Blender and
//! Unreal). Controls do not survive any generic format, so each DCC also gets a
//! generated Python script that rebuilds the control rig with native tools.

pub mod glb;

use std::path::{Path, PathBuf};
use std::str::FromStr;

use anyhow::{Context, Result};
use serde_json::json;

use crate::rig::Rig;

const BLENDER_TEMPLATE: &str = include_str!("../../templates/blender_rig.py");
const MAYA_TEMPLATE: &str = include_str!("../../templates/maya_rig.py");
const UNREAL_TEMPLATE: &str = include_str!("../../templates/unreal_rig.py");

const JSON_SLOT: &str = "__HIVE_RIG_JSON__";
const GLB_SLOT: &str = "__HIVE_GLB_PATH__";
const EXE_SLOT: &str = "__HIVE_EXE__";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Blender,
    Maya,
    Unreal,
}

impl Target {
    pub const ALL: [Target; 3] = [Target::Blender, Target::Maya, Target::Unreal];

    pub fn key(self) -> &'static str {
        match self {
            Target::Blender => "blender",
            Target::Maya => "maya",
            Target::Unreal => "unreal",
        }
    }
}

impl FromStr for Target {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "blender" => Ok(Target::Blender),
            "maya" => Ok(Target::Maya),
            "unreal" | "ue" | "ue5" => Ok(Target::Unreal),
            other => anyhow::bail!("unknown target '{other}' (expected blender, maya or unreal)"),
        }
    }
}

/// Make a name safe for file names, node names and Python string literals.
pub fn sanitize_name(name: &str) -> String {
    let mut out: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if out.is_empty() || out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert_str(0, "rig_");
    }
    out
}

/// The command-line `hive-rig`, so builder scripts can call it back to rebuild
/// from edited guides. When the scripts are written by the desktop app, that's
/// the `hive-rig` program next to it.
fn exe_path() -> String {
    let Ok(exe) = std::env::current_exe() else {
        return String::new();
    };
    let cli = format!("hive-rig{}", std::env::consts::EXE_SUFFIX);
    let path = if exe.file_name().is_some_and(|n| n == cli.as_str()) {
        exe
    } else {
        exe.with_file_name(cli)
    };
    path.to_string_lossy().replace('"', "")
}

fn fill(template: &str, json: &str, glb_path: &str) -> String {
    // The data is placed inside a raw triple-quoted Python string. Names are
    // sanitised and the rest is numbers, so the data cannot contain `"""`.
    debug_assert!(!json.contains("\"\"\""));
    template
        .replace(JSON_SLOT, json)
        .replace(GLB_SLOT, glb_path)
        .replace(EXE_SLOT, &exe_path())
}

/// Generate the builder script for one DCC.
pub fn script(rig: &Rig, target: Target, glb_path: &Path) -> Result<String> {
    let glb = glb_path.to_string_lossy().replace('"', "");
    Ok(match target {
        Target::Blender => fill(BLENDER_TEMPLATE, &serde_json::to_string(rig)?, &glb),
        Target::Maya => fill(MAYA_TEMPLATE, &serde_json::to_string(rig)?, &glb),
        Target::Unreal => {
            // Unreal imports mesh and skin from the .glb; the script only needs
            // bone names and the control description.
            let bones: Vec<_> = rig
                .skeleton
                .bones
                .iter()
                .map(|b| json!({ "name": b.name, "parent": b.parent, "deform": b.deform, "head": b.head, "translate": b.translate }))
                .collect();
            // Bones that recover the import's axes; the first also places the COG.
            let basis = ["pelvis", "hand_l", "head", "ball_l"].map(|b| rig.guides.naming.bone(b));
            let data = json!({
                "name": rig.name,
                "height": rig.guides.body.height,
                "bones": bones,
                "controls": rig.controls.controls,
                "ik_chains": rig.controls.ik_chains,
                "basis": basis,
            });
            fill(UNREAL_TEMPLATE, &serde_json::to_string(&data)?, &glb)
        }
    })
}

/// Write `<name>.glb`, `<name>.hive.json`, `<name>.guides.json` and one builder
/// script per target.
pub fn write_all(rig: &Rig, out_dir: &Path, targets: &[Target]) -> Result<Vec<PathBuf>> {
    // A bare file name's parent is the empty path: that's the current folder.
    let out_dir = if out_dir.as_os_str().is_empty() {
        Path::new(".")
    } else {
        out_dir
    };
    std::fs::create_dir_all(out_dir).with_context(|| format!("creating {}", out_dir.display()))?;
    // `absolute` rather than `canonicalize`: on Windows the latter adds a `\\?\` prefix
    // that DCC importers such as Unreal's may not accept in the embedded .glb path.
    let out_dir = std::path::absolute(out_dir)?;
    let mut written = Vec::new();

    let glb_path = out_dir.join(format!("{}.glb", rig.name));
    std::fs::write(&glb_path, glb::write(rig))?;
    written.push(glb_path.clone());

    let json_path = out_dir.join(format!("{}.hive.json", rig.name));
    std::fs::write(&json_path, serde_json::to_vec_pretty(rig)?)?;
    written.push(json_path);

    let guides_path = out_dir.join(format!("{}.guides.json", rig.name));
    std::fs::write(&guides_path, serde_json::to_vec_pretty(&rig.guides)?)?;
    written.push(guides_path);

    for &t in targets {
        let path = out_dir.join(format!("{}_{}.py", rig.name, t.key()));
        std::fs::write(&path, script(rig, t, &glb_path)?)?;
        written.push(path);
    }
    Ok(written)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{demo::demo_humanoid, rig};

    #[test]
    fn sanitizes_names() {
        assert_eq!(sanitize_name("my hero-01"), "my_hero_01");
        assert_eq!(sanitize_name("3d"), "rig_3d");
        assert_eq!(sanitize_name(""), "rig_");
    }

    #[test]
    fn parses_targets() {
        assert_eq!("UE5".parse::<Target>().unwrap(), Target::Unreal);
        assert!("houdini".parse::<Target>().is_err());
    }

    #[test]
    fn scripts_embed_valid_json_and_no_placeholders() {
        let r = rig::auto_rig(demo_humanoid(), "demo", Default::default()).unwrap();
        for t in Target::ALL {
            let s = script(&r, t, Path::new("/tmp/demo.glb")).unwrap();
            assert!(
                !s.contains(JSON_SLOT) && !s.contains(GLB_SLOT) && !s.contains(EXE_SLOT),
                "{t:?}"
            );
            let start = s.find("r\"\"\"").unwrap() + 4;
            let end = start + s[start..].find("\"\"\"").unwrap();
            let v: serde_json::Value = serde_json::from_str(&s[start..end]).unwrap();
            assert_eq!(v["name"], "demo");
        }
    }

    #[test]
    fn every_shape_reaches_every_builder() {
        use crate::controls::Shape;
        // Blender and Maya draw the outline hive-rig writes, so they have no
        // shape table of their own to fall behind.
        for template in [BLENDER_TEMPLATE, MAYA_TEMPLATE] {
            assert!(template.contains("[\"outline\"]"));
            assert!(!template.contains("\"circle\""));
        }
        // Unreal maps every shape to a library shape.
        for shape in Shape::ALL {
            let key = serde_json::to_string(&shape).unwrap();
            assert!(
                UNREAL_TEMPLATE.contains(&format!("    {key}: \"")),
                "{key} missing from Unreal's SHAPES"
            );
        }
        // The embedded rig carries each control's outline.
        let r = rig::auto_rig(demo_humanoid(), "demo", Default::default()).unwrap();
        let s = script(&r, Target::Maya, Path::new("/tmp/demo.glb")).unwrap();
        let start = s.find("r\"\"\"").unwrap() + 4;
        let end = start + s[start..].find("\"\"\"").unwrap();
        let v: serde_json::Value = serde_json::from_str(&s[start..end]).unwrap();
        for c in v["controls"]["controls"].as_array().unwrap() {
            assert!(
                !c["outline"].as_array().unwrap().is_empty(),
                "{}",
                c["name"]
            );
        }
    }
}
