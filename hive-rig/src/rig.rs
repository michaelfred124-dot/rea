//! The complete auto-rig: mesh, guides, skeleton, skin weights and controls.

use serde::{Deserialize, Serialize};

use crate::components::{self, Guides};
use crate::controls::ControlRig;
use crate::export;
use crate::joints::{self, Joint, JointReport};
use crate::landmarks::{self, Detected};
use crate::mesh::Mesh;
use crate::skeleton::Skeleton;
use crate::skin::{self, SkinOptions, SkinWeights};

pub const FORMAT: &str = "hive-rig";
pub const FORMAT_VERSION: u32 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rig {
    pub format: String,
    pub version: u32,
    /// The hive-rig that wrote the file (absent before 0.1.0 stamped it).
    /// Writing always stamps this program's version.
    #[serde(default, rename = "hive_rig", serialize_with = "crate::version::stamp")]
    pub written_by: Option<String>,
    pub name: String,
    /// Always "Y": model space is Y-up, metres, facing +Z (glTF convention).
    pub up_axis: String,
    pub units: String,
    /// The component layout the rig was built from.
    pub guides: Guides,
    pub mesh: Mesh,
    pub skeleton: Skeleton,
    pub skin: SkinWeights,
    pub controls: ControlRig,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RigOptions {
    pub skin: SkinOptions,
}

/// Detect the body on a humanoid mesh and place the default biped guides on it.
/// `Detected` says which landmarks came from the geometry rather than proportions.
pub fn detect_guides(mesh: &Mesh) -> (Guides, Detected) {
    let lm = landmarks::detect(mesh);
    let detected = lm.detected.clone();
    (components::biped(&lm), detected)
}

/// Guides for a character: detected on the mesh, then moved onto the joints of
/// the skeleton it came with, if any. The third value is `None` without joints,
/// and an error (with the detected guides returned) when they couldn't be used.
pub fn character_guides(
    mesh: &Mesh,
    joints: &[Joint],
) -> (Guides, Detected, Option<anyhow::Result<JointReport>>) {
    let (guides, detected) = detect_guides(mesh);
    if joints.is_empty() {
        return (guides, detected, None);
    }
    match joints::guides_from_joints(guides.clone(), joints) {
        Ok((g, report)) => (g, detected, Some(Ok(report))),
        Err(e) => (guides, detected, Some(Err(e))),
    }
}

/// Run the full pipeline on a humanoid mesh with the default biped layout.
pub fn auto_rig(mesh: Mesh, name: &str, opts: RigOptions) -> anyhow::Result<Rig> {
    mesh.validate()?;
    let (guides, _) = detect_guides(&mesh);
    build_rig(mesh, name, guides, opts)
}

/// Build the rig described by `guides` on `mesh`.
pub fn build_rig(mesh: Mesh, name: &str, guides: Guides, opts: RigOptions) -> anyhow::Result<Rig> {
    mesh.validate()?;
    let name = export::sanitize_name(name);
    let mut built = components::build(&guides)?;
    guides
        .naming
        .apply(&mut built.skeleton, &mut built.controls)?;
    let skin = skin::compute(&mesh, &built.skeleton, &guides.body, opts.skin);
    Ok(Rig {
        format: FORMAT.into(),
        version: FORMAT_VERSION,
        written_by: None,
        name,
        up_axis: "Y".into(),
        units: "m".into(),
        guides,
        mesh,
        skeleton: built.skeleton,
        skin,
        controls: built.controls,
    })
}

impl Rig {
    /// Read a `<name>.hive.json` written by `export::write_all`.
    pub fn load(path: &std::path::Path) -> anyhow::Result<Rig> {
        use anyhow::Context;
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let rig: Rig = serde_json::from_str(&text).with_context(|| {
            format!(
                "{} is not a hive-rig file from this version",
                path.display()
            )
        })?;
        anyhow::ensure!(
            rig.format == FORMAT,
            "{} is not a hive-rig file",
            path.display()
        );
        Ok(rig)
    }

    /// A warning when the rig file came from a newer hive-rig, or holds guides
    /// from one.
    pub fn newer_warning(&self) -> Option<String> {
        if self.version > FORMAT_VERSION {
            return Some(format!(
                "this rig file is format version {} from a newer hive-rig; this {} reads up to version {FORMAT_VERSION}, so parts of it may be ignored",
                self.version,
                crate::version::CURRENT
            ));
        }
        crate::version::newer_note("this rig file", self.written_by.as_deref())
            .or_else(|| self.guides.newer_warning())
    }
}
