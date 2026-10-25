//! The complete auto-rig: mesh, guides, skeleton, skin weights and controls.

use serde::{Deserialize, Serialize};

use crate::components::{self, Guides};
use crate::controls::ControlRig;
use crate::export;
use crate::joints::{self, Joint, JointReport};
use crate::landmarks::{self, Detected};
use crate::mesh::Mesh;
use crate::orient::{BodyKind, Orientation};
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
    /// How the character stood in its source file. The mesh, skeleton and
    /// guides here are turned into hive-rig's frame (Y up, facing +Z); the .glb
    /// and .fbx turn it back. Absent means the source was already in it.
    #[serde(default, skip_serializing_if = "Orientation::is_standard")]
    pub orientation: Orientation,
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

/// Guides for a character (turned into hive-rig's frame): detected on the mesh,
/// or for a four-legged `body` the dog layout fitted to it, then moved onto the
/// joints of the skeleton it came with, if any. The third value is `None`
/// without joints, and an error (with the detected guides returned) when they
/// couldn't be used.
pub fn character_guides(
    mesh: &Mesh,
    joints: &[Joint],
    body: BodyKind,
) -> (Guides, Detected, Option<anyhow::Result<JointReport>>) {
    if body == BodyKind::Quadruped {
        let fitted = joints::fit_quadruped(mesh);
        let found = Detected {
            crotch: true,
            arms: true,
            neck: true,
        };
        if joints.is_empty() {
            return (fitted, found, None);
        }
        return match joints::quad_guides_from_joints(fitted.clone(), joints) {
            Ok(Some((g, report))) => (g, found, Some(Ok(report))),
            Ok(None) => (
                fitted,
                found,
                Some(Err(anyhow::anyhow!(
                    "the skeleton doesn't show four legs, so the dog layout was fitted to the body"
                ))),
            ),
            Err(e) => (fitted, found, Some(Err(e))),
        };
    }
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
    let mut rig = Rig {
        format: FORMAT.into(),
        version: FORMAT_VERSION,
        written_by: None,
        name,
        up_axis: "Y".into(),
        orientation: Orientation::STANDARD,
        units: "m".into(),
        guides,
        mesh,
        skeleton: built.skeleton,
        skin,
        controls: built.controls,
    };
    // Shapes named like a board move (blink_l, jawOpen) follow its slider.
    crate::shapes::link_board(&mut rig);
    Ok(rig)
}

impl Rig {
    /// The mesh and skeleton turned back to how the character stood in its
    /// source file, for the formats that carry no controls (.glb and .fbx):
    /// joints carry translation only, so turning them is turning their heads.
    /// The guides and controls stay in hive-rig's frame.
    pub fn in_source_frame(&self) -> std::borrow::Cow<'_, Rig> {
        if self.orientation.is_standard() {
            return std::borrow::Cow::Borrowed(self);
        }
        let m = self.orientation.to_source();
        let mut r = self.clone();
        crate::orient::turn_mesh(&mut r.mesh, m);
        for b in &mut r.skeleton.bones {
            b.head = m * b.head;
            b.tail = m * b.tail;
        }
        std::borrow::Cow::Owned(r)
    }

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
    /// Target bones that found a source bone (by name or by position).
    pub matched: usize,
    /// Of those, the bones of extra chains (tails, ears, hair named
    /// differently) matched by where they lie rather than by name.
    pub by_position: Vec<String>,
    /// Target deform bones with no source bone; their skin comes from the
    /// nearest matched bones.
    pub unmatched: Vec<String>,
}

/// Copy `from`'s skin weights onto `to`'s mesh, which may be another character
/// with the same or a similar skeleton, in another pose or size. Bones match
/// by name ([`bone_map`]), then chains left over on both sides by position
/// ([`skin::match_bones`]). The result follows `to`'s influence limits.
pub fn copy_weights(from: &Rig, to: &Rig) -> (SkinWeights, CopyReport) {
    let src = Skinned {
        mesh: &from.mesh,
        skeleton: &from.skeleton,
        skin: &from.skin,
    };
    let dst = Skinned {
        mesh: &to.mesh,
        skeleton: &to.skeleton,
        skin: &to.skin,
    };
    let m = skin::match_bones(&src, &dst, &bone_map(from, to));
    let skin = skin::transfer_matched(&src, &dst, &m);
    let report = CopyReport {
        matched: m.dst_to_src.iter().filter(|m| m.is_some()).count(),
        by_position: m
            .by_position
            .iter()
            .map(|&b| to.skeleton.bones[b].name.clone())
            .collect(),
        unmatched: to
            .skeleton
            .bones
            .iter()
            .zip(&m.dst_to_src)
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

    /// A tail and ears named differently, with other joint counts, still take
    /// the source's tail and ear weights, matched by where they lie.
    #[test]
    fn copy_weights_matches_extra_chains_by_position() {
        use crate::components::{self, Component};
        use crate::skeleton::Side;
        let guides = components::quadruped(demo::DOG_HEIGHT);
        let mut src_guides = guides.clone();
        let mut dst_guides = guides;
        let h = demo::DOG_HEIGHT;
        // Ears on the head: `ear_l`/`ear_r` (three joints) on the source,
        // `flap_l`/`flap_r` (two) on the target, the tail renamed `brush`
        // with fewer joints.
        let built = components::build(&src_guides).unwrap();
        let head = built.skeleton.bone("head").unwrap().clone();
        let ear = |id: &str, s: f32, joints: usize| {
            let mut c = components::chain(
                id.to_string(),
                if s > 0.0 { Side::Left } else { Side::Right },
                "head",
                (0..=joints).map(|k| {
                    let t = k as f32 / joints as f32;
                    head.tail + glam::Vec3::new(s * (0.03 + 0.04 * t), 0.06 * t, -0.03) * h
                }),
            );
            c.options = Default::default();
            c
        };
        src_guides.components.push(ear("ear_l", 1.0, 3));
        src_guides.components.push(ear("ear_r", -1.0, 3));
        dst_guides.components.push(ear("flap_l", 1.0, 2));
        dst_guides.components.push(ear("flap_r", -1.0, 2));
        let tail: &mut Component = dst_guides
            .components
            .iter_mut()
            .find(|c| c.id == "tail")
            .unwrap();
        tail.id = "brush".into();
        let n = tail.guides.len();
        let keep = [0, n / 2, n - 1];
        tail.guides = keep
            .iter()
            .enumerate()
            .map(|(k, &i)| components::Guide {
                name: format!("j{}", k + 1),
                position: tail.guides[i].position,
            })
            .collect();
        let src = build_rig(
            demo::demo_quadruped(),
            "src",
            src_guides,
            RigOptions::default(),
        )
        .unwrap();
        let dst = build_rig(
            demo::demo_quadruped(),
            "dst",
            dst_guides,
            RigOptions::default(),
        )
        .unwrap();
        let (skin, report) = copy_weights(&src, &dst);
        for prefix in ["brush_", "flap_"] {
            assert!(
                report.by_position.iter().any(|b| b.starts_with(prefix)),
                "{:?}",
                report.by_position
            );
            assert!(!report.unmatched.iter().any(|b| b.starts_with(prefix)));
        }
        // Where the source's tail and ear bones weigh most, the copy's tail and
        // ear bones do, on the same side.
        let top = |s: &SkinWeights, sk: &crate::skeleton::Skeleton, v: usize| {
            let b = s
                .influences(v)
                .max_by(|x, y| x.1.total_cmp(&y.1))
                .unwrap()
                .0;
            sk.bones[b].name.clone()
        };
        let (mut tail_n, mut tail_ok, mut ear_n, mut ear_ok) = (0, 0, 0, 0);
        for v in 0..skin.vertex_count() {
            let was = top(&src.skin, &src.skeleton, v);
            let got = top(&skin, &dst.skeleton, v);
            if was.starts_with("tail_") {
                tail_n += 1;
                tail_ok += usize::from(got.starts_with("brush_"));
            }
            if was.starts_with("ear_") {
                ear_n += 1;
                ear_ok +=
                    usize::from(got.starts_with("flap_") && got.ends_with(&was[was.len() - 2..]));
            }
        }
        assert!(
            tail_n > 20 && tail_ok as f32 > 0.95 * tail_n as f32,
            "{tail_ok} of {tail_n}"
        );
        assert!(
            ear_n > 0 && ear_ok as f32 > 0.95 * ear_n as f32,
            "{ear_ok} of {ear_n}"
        );
        // By name alone the tail's skin would go to the pelvis.
        let m = skin::BoneMatch {
            dst_to_src: bone_map(&src, &dst),
            src_to_dst: {
                let mut s2d = vec![None; src.skeleton.bones.len()];
                for (d, s) in bone_map(&src, &dst).iter().enumerate() {
                    if let Some(s) = s {
                        s2d[*s] = s2d[*s].or(Some(d));
                    }
                }
                s2d
            },
            by_position: Vec::new(),
        };
        let by_name = skin::transfer_matched(
            &Skinned {
                mesh: &src.mesh,
                skeleton: &src.skeleton,
                skin: &src.skin,
            },
            &Skinned {
                mesh: &dst.mesh,
                skeleton: &dst.skeleton,
                skin: &dst.skin,
            },
            &m,
        );
        let brush = (0..skin.vertex_count())
            .filter(|&v| top(&by_name, &dst.skeleton, v).starts_with("brush_"))
            .count();
        assert!(brush < tail_n / 10, "{brush}");
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

    /// A character lying Z-up and facing -Y in its file: the .glb and .fbx put
    /// it back that way, and opening the .glb again finds that orientation,
    /// turns it upright onto the same mesh, and its joints become the guides on
    /// the right sides.
    #[test]
    fn orientation_round_trip() {
        use crate::orient::{Axis, Choice};
        let o = Orientation::new(Axis::PosZ, Axis::NegY).unwrap();
        let mut rig = auto_rig(demo::humanoid(0.0), "turned", RigOptions::default()).unwrap();
        rig.orientation = o;
        let dir = std::env::temp_dir().join(format!("hive-rig-orient-{}", std::process::id()));
        let files = crate::export::write_all(&rig, &dir, &[crate::Target::Fbx]).unwrap();
        assert!(
            files
                .iter()
                .any(|f| f.extension().is_some_and(|e| e == "fbx"))
        );
        let glb = dir.join("turned.glb");
        // In the file it stands along +Z.
        let raw = crate::import::load_mesh(&glb).unwrap();
        let (lo, hi) = raw.bounds();
        assert!(hi.z - lo.z > 2.0 * (hi.y - lo.y), "{lo} {hi}");
        let ch = crate::import::load_character(&glb, Choice::default()).unwrap();
        assert_eq!(ch.detection.orientation, o, "{:?}", ch.detection);
        assert_eq!(ch.detection.body, BodyKind::Biped);
        for (a, b) in ch.mesh.positions.iter().zip(&rig.mesh.positions) {
            assert!(a.distance(*b) < 1e-4);
        }
        let (g, _, report) = character_guides(&ch.mesh, &ch.joints, ch.detection.body);
        assert!(report.unwrap().unwrap().placed.len() > 10);
        let want = rig.skeleton.bone("upperarm_l").unwrap().head;
        let got = g.component("arm_l").unwrap().position("shoulder").unwrap();
        assert!(got.distance(want) < 1e-4, "{got} {want}");
        assert!(got.x > g.body.center_x);
        // The .fbx reads back the same way: lying in the file, found, stood up
        // onto the same mesh, its bones the same guides.
        let fbx = dir.join("turned.fbx");
        let raw = crate::import::load_mesh(&fbx).unwrap();
        let (lo, hi) = raw.bounds();
        assert!(hi.z - lo.z > 2.0 * (hi.y - lo.y), "{lo} {hi}");
        let ch = crate::import::load_character(&fbx, Choice::default()).unwrap();
        assert_eq!(ch.detection.orientation, o, "{:?}", ch.detection);
        assert_eq!(ch.mesh.vertex_count(), rig.mesh.vertex_count());
        for (a, b) in ch.mesh.positions.iter().zip(&rig.mesh.positions) {
            assert!(a.distance(*b) < 1e-4);
        }
        let (g, _, report) = character_guides(&ch.mesh, &ch.joints, ch.detection.body);
        assert!(report.unwrap().unwrap().placed.len() > 10);
        let got = g.component("arm_l").unwrap().position("shoulder").unwrap();
        assert!(got.distance(want) < 1e-4, "{got} {want}");
        // The rig file keeps the orientation.
        let back = Rig::load(&dir.join("turned.hive.json")).unwrap();
        assert_eq!(back.orientation, o);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The dog's skeleton with no names is read by its shape into the dog layout.
    #[test]
    fn a_nameless_dog_skeleton_gives_the_dog_layout() {
        let rig = build_rig(
            demo::demo_quadruped(),
            "dog",
            crate::components::quadruped(demo::DOG_HEIGHT),
            RigOptions::default(),
        )
        .unwrap();
        let mut joints: Vec<Joint> = rig
            .skeleton
            .bones
            .iter()
            .enumerate()
            .map(|(i, b)| Joint {
                name: format!("bone{i}"),
                position: b.head,
                parent: b.parent,
            })
            .collect();
        // Paw ends at the floor.
        for (i, b) in rig.skeleton.bones.iter().enumerate() {
            if b.name.starts_with("ball_") || b.name.starts_with("hand_") {
                joints.push(Joint {
                    name: format!("end{i}"),
                    position: b.tail,
                    parent: Some(i),
                });
            }
        }
        let ch = crate::import::orient_character(
            rig.mesh.clone(),
            joints,
            crate::orient::Choice::default(),
        );
        assert_eq!(ch.detection.body, BodyKind::Quadruped);
        let (g, _, report) = character_guides(&ch.mesh, &ch.joints, ch.detection.body);
        let report = report.unwrap().unwrap();
        assert!(!report.by_shape.is_empty());
        assert_eq!(crate::orient::layout_name(&g), "dog");
        let want = rig.skeleton.bone("calf_l").unwrap().head;
        let got = g.component("hind_leg_l").unwrap().position("knee").unwrap();
        assert!(got.distance(want) < 1e-4, "{got} {want}");
    }
}
