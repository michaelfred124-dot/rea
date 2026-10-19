//! The complete auto-rig: mesh, guides, skeleton, skin weights and controls.

use serde::{Deserialize, Serialize};

use crate::components::{self, Guides};
use crate::controls::ControlRig;
use crate::export;
use crate::joints::{self, Joint, JointReport};
use crate::landmarks::{self, Detected};
use crate::mesh::Mesh;
use crate::skeleton::Skeleton;
use crate::skin::{self, SkinOptions, SkinWeights, Skinned};

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
    guides.skin.validate()?;
    let skin = skin::compute(&mesh, &built.skeleton, opts.skin, guides.skin);
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
        let mut rig: Rig = serde_json::from_str(&text).with_context(|| {
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
        // Texture images are named relative to the rig file.
        let dir = path.parent().unwrap_or(std::path::Path::new("."));
        for image in &mut rig.mesh.images {
            if !image.uri.is_empty() {
                image.path = Some(dir.join(&image.uri));
            }
        }
        rig.mesh
            .validate()
            .with_context(|| format!("{} has a broken mesh", path.display()))?;
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

impl Rig {
    /// Bone names before the naming convention was applied (`upperarm_l` under
    /// any convention), so rigs with different naming can be matched. Falls back
    /// to the exported names when the guides no longer build the same bones.
    pub fn internal_bone_names(&self) -> Vec<String> {
        match components::build(&self.guides) {
            Ok(b) if b.skeleton.bones.len() == self.skeleton.bones.len() => {
                b.skeleton.bones.into_iter().map(|b| b.name).collect()
            }
            _ => self.skeleton.bones.iter().map(|b| b.name.clone()).collect(),
        }
    }

    /// The automatic weights this rig's guides give, before any hand edits.
    pub fn auto_weights(&self, opts: SkinOptions) -> SkinWeights {
        skin::compute(&self.mesh, &self.skeleton, opts, self.guides.skin)
    }
}

/// Carry the hand-edited weights of `old` over to `new`, rebuilt from edited
/// guides on the same mesh (see [`skin::keep_edits`]). `old_auto` is the
/// automatic weights `old` was built with; without it every vertex keeps its
/// weights where the bones allow. Returns how many vertices kept theirs.
pub fn keep_weights(old: &Rig, old_auto: Option<&SkinWeights>, new: &mut Rig) -> usize {
    let (skin, kept) = skin::keep_edits(
        &old.skin,
        &old.internal_bone_names(),
        old_auto,
        &new.skin,
        &new.internal_bone_names(),
    );
    new.skin = skin::limit(&skin, new.guides.skin);
    kept
}

/// For each bone of `to`, the bone of `from` it corresponds to: the same
/// internal name, or else the same name once side tokens, prefixes and joint
/// suffixes are read the way joints-to-guides reads them (`L_upperarm_jnt`,
/// `upperarm_l` and `mixamorig:LeftArm`-style names).
pub fn bone_map(from: &Rig, to: &Rig) -> Vec<Option<usize>> {
    let from_internal = from.internal_bone_names();
    let to_internal = to.internal_bone_names();
    let by_internal: std::collections::HashMap<&str, usize> = from_internal
        .iter()
        .enumerate()
        .map(|(i, n)| (n.as_str(), i))
        .collect();
    let key = |name: &str| {
        let (side, names) = joints::candidates(name);
        names.first().map(|n| (side, n.clone()))
    };
    let mut by_key = std::collections::HashMap::new();
    for (i, b) in from.skeleton.bones.iter().enumerate() {
        if let Some(k) = key(&b.name) {
            by_key.entry(k).or_insert(i);
        }
    }
    to_internal
        .iter()
        .zip(&to.skeleton.bones)
        .map(|(internal, bone)| {
            by_internal
                .get(internal.as_str())
                .copied()
                .or_else(|| key(&bone.name).and_then(|k| by_key.get(&k).copied()))
        })
        .collect()
}

/// What [`copy_weights`] matched.
#[derive(Debug, Clone, Default)]
pub struct CopyReport {
    /// Target bones that found a source bone.
    pub matched: usize,
    /// Target deform bones with no source bone; their skin comes from the
    /// nearest matched bones.
    pub unmatched: Vec<String>,
}

/// Copy `from`'s skin weights onto `to`'s mesh, which may be another character
/// with the same or a similar skeleton, in another pose or size. The result
/// follows `to`'s influence limits.
pub fn copy_weights(from: &Rig, to: &Rig) -> (SkinWeights, CopyReport) {
    let map = bone_map(from, to);
    let skin = skin::transfer(
        &Skinned {
            mesh: &from.mesh,
            skeleton: &from.skeleton,
            skin: &from.skin,
        },
        &Skinned {
            mesh: &to.mesh,
            skeleton: &to.skeleton,
            skin: &to.skin,
        },
        &map,
    );
    let report = CopyReport {
        matched: map.iter().filter(|m| m.is_some()).count(),
        unmatched: to
            .skeleton
            .bones
            .iter()
            .zip(&map)
            .filter(|(b, m)| b.deform && m.is_none())
            .map(|(b, _)| b.name.clone())
            .collect(),
    };
    (skin::limit(&skin, to.guides.skin), report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{demo, naming};

    #[test]
    fn copy_weights_across_naming_and_pose() {
        let t = auto_rig(demo::humanoid(0.0), "t", RigOptions::default()).unwrap();
        let mesh = demo::humanoid(45.0);
        let (mut guides, _) = detect_guides(&mesh);
        guides.naming = naming::Preset::Suffix.naming();
        guides.skin.max_influences = 8;
        let a = build_rig(mesh, "a", guides, RigOptions::default()).unwrap();
        let map = bone_map(&t, &a);
        // Every bone matches through the internal names despite the renaming.
        assert!(map.iter().all(Option::is_some));
        let i = a
            .skeleton
            .index_of(&a.guides.naming.bone("upperarm_l"))
            .unwrap();
        assert_eq!(t.skeleton.bones[map[i].unwrap()].name, "upperarm_l");

        let (skin, report) = copy_weights(&t, &a);
        assert!(report.unmatched.is_empty());
        assert_eq!(skin.vertex_count(), a.mesh.vertex_count());
        assert!(skin.width() <= 8);
        // Most vertices keep the bone the A-pose rig gives them itself.
        let top = |s: &SkinWeights, v: usize| {
            s.influences(v)
                .max_by(|x, y| x.1.total_cmp(&y.1))
                .map(|p| p.0)
        };
        let n = skin.vertex_count();
        let agree = (0..n).filter(|&v| top(&skin, v) == top(&a.skin, v)).count();
        assert!(agree as f32 > 0.9 * n as f32, "{agree} of {n}");
    }

    #[test]
    fn rebuilding_keeps_hand_edits() {
        let mut old = auto_rig(demo::humanoid(0.0), "k", RigOptions::default()).unwrap();
        let base = old.auto_weights(SkinOptions::default());
        assert_eq!(base, old.skin);
        let head = old.skeleton.index_of("head").unwrap() as u16;
        old.skin.joints[0] = vec![head];
        old.skin.weights[0] = vec![1.0];
        let mut guides = old.guides.clone();
        guides.skin.max_influences = 8;
        let mut new = build_rig(old.mesh.clone(), "k", guides, RigOptions::default()).unwrap();
        let fresh = new.skin.clone();
        assert_eq!(keep_weights(&old, Some(&base), &mut new), 1);
        assert_eq!(new.skin.joints[0], vec![head]);
        assert_eq!(new.skin.joints[1], fresh.joints[1]);
    }
}
