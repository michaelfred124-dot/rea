//! The complete auto-rig: mesh, skeleton, skin weights and controls.

use serde::{Deserialize, Serialize};

use crate::controls::{self, ControlRig};
use crate::landmarks::{self, Landmarks};
use crate::mesh::Mesh;
use crate::skeleton::Skeleton;
use crate::skin::{self, SkinOptions, SkinWeights};
use crate::{biped, export};

pub const FORMAT: &str = "hive-rig";
pub const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rig {
    pub format: String,
    pub version: u32,
    pub name: String,
    /// Always "Y": model space is Y-up, metres, facing +Z (glTF convention).
    pub up_axis: String,
    pub units: String,
    pub landmarks: Landmarks,
    pub mesh: Mesh,
    pub skeleton: Skeleton,
    pub skin: SkinWeights,
    pub controls: ControlRig,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct RigOptions {
    pub skin: SkinOptions,
}

/// Run the full pipeline on a humanoid mesh.
pub fn auto_rig(mesh: Mesh, name: &str, opts: RigOptions) -> anyhow::Result<Rig> {
    mesh.validate()?;
    let name = export::sanitize_name(name);
    let lm = landmarks::detect(&mesh);
    let skeleton = biped::build(&lm);
    let skin = skin::compute(&mesh, &skeleton, &lm, opts.skin);
    let controls = controls::build(&skeleton, &lm);
    Ok(Rig {
        format: FORMAT.into(),
        version: FORMAT_VERSION,
        name,
        up_axis: "Y".into(),
        units: "m".into(),
        landmarks: lm,
        mesh,
        skeleton,
        skin,
        controls,
    })
}
