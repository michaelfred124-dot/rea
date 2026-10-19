//! Skeleton data: bones as head/tail segments in model space.
//!
//! Bone names follow the Unreal mannequin convention (`upperarm_l`, `calf_r`, ...)
//! so the result retargets cleanly in Unreal and reads naturally in Blender and Maya.

use glam::Vec3;
use serde::{Deserialize, Serialize};

/// Which body region a bone belongs to (a label written to the rig file). The
/// automatic skin weights no longer read it: they keep limbs apart by measuring
/// through the body (see `crate::voxel`), which works for any creature.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Region {
    Center,
    ArmL,
    ArmR,
    LegL,
    LegR,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    #[default]
    Center,
    Left,
    Right,
}

impl Side {
    pub fn suffix(self) -> &'static str {
        match self {
            Side::Center => "",
            Side::Left => "_l",
            Side::Right => "_r",
        }
    }

    /// +1 for left (+X), -1 for right, 0 for centre.
    pub fn sign(self) -> f32 {
        match self {
            Side::Center => 0.0,
            Side::Left => 1.0,
            Side::Right => -1.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bone {
    pub name: String,
    pub parent: Option<usize>,
    pub head: Vec3,
    pub tail: Vec3,
    /// Deforming bones receive skin weights; helper bones (root) do not.
    pub deform: bool,
    pub side: Side,
    pub region: Region,
    /// Other regions this bone borders (the upper arm the torso). A label kept
    /// in rig files; the automatic skin weights don't read it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Region>,
    /// Id of the component that built this bone.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub component: String,
    /// Twist helper: rotates about its own length by `factor` times the twist of `source`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub twist: Option<Twist>,
    /// The bone's control moves it as well as rotating it: face joints, and bones
    /// built under a non-deforming helper (the root). Without it the control only
    /// turns the bone. Decided when the rig is built, so the builders never guess
    /// it from other flags (a bendy limb's segment bones stop deforming but still
    /// carry their children by rotation).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub translate: bool,
    /// Only vertices within this distance of the bone take its weight (face
    /// joints, so a brow doesn't pull the whole forehead).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reach: Option<f32>,
}

/// Twist distribution for forearm / upper-arm style helper bones.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Twist {
    /// Bone whose twist (local rotation about its length) is copied.
    pub source: String,
    /// Fraction of that twist to apply; negative counter-rotates.
    pub factor: f32,
}

impl Bone {
    pub fn length(&self) -> f32 {
        self.head.distance(self.tail)
    }

    pub fn direction(&self) -> Vec3 {
        (self.tail - self.head).try_normalize().unwrap_or(Vec3::Y)
    }

    /// Closest distance from `p` to the bone segment.
    pub fn distance_to(&self, p: Vec3) -> f32 {
        let seg = self.tail - self.head;
        let len2 = seg.length_squared();
        let t = if len2 > 0.0 {
            ((p - self.head).dot(seg) / len2).clamp(0.0, 1.0)
        } else {
            0.0
        };
        p.distance(self.head + seg * t)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Skeleton {
    pub bones: Vec<Bone>,
}

impl Skeleton {
    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.bones.iter().position(|b| b.name == name)
    }

    pub fn bone(&self, name: &str) -> Option<&Bone> {
        self.index_of(name).map(|i| &self.bones[i])
    }

    /// Add a bone whose parent is looked up by name. Parents must be added first.
    pub fn add(
        &mut self,
        name: impl Into<String>,
        parent: Option<&str>,
        head: Vec3,
        tail: Vec3,
        side: Side,
        region: Region,
    ) -> usize {
        let parent = parent.map(|p| {
            self.index_of(p)
                .unwrap_or_else(|| panic!("parent bone '{p}' must be added before its children"))
        });
        self.bones.push(Bone {
            name: name.into(),
            parent,
            head,
            tail,
            deform: true,
            side,
            region,
            links: Vec::new(),
            component: String::new(),
            twist: None,
            translate: false,
            reach: None,
        });
        self.bones.len() - 1
    }

    /// True when `bone` is `ancestor` or hangs below it.
    pub fn within(&self, bone: usize, ancestor: usize) -> bool {
        let mut at = Some(bone);
        while let Some(i) = at {
            if i == ancestor {
                return true;
            }
            at = self.bones.get(i).and_then(|b| b.parent);
        }
        false
    }

    pub fn children(&self, index: usize) -> impl Iterator<Item = usize> + '_ {
        self.bones
            .iter()
            .enumerate()
            .filter(move |(_, b)| b.parent == Some(index))
            .map(|(i, _)| i)
    }

    /// The part of the bone that receives skin weights: up to its twist helper if it
    /// has one, so the helper takes over the far half of the limb.
    pub fn skin_segment(&self, index: usize) -> (Vec3, Vec3) {
        let bone = &self.bones[index];
        let twist_child = self
            .children(index)
            .find(|&c| self.bones[c].twist.is_some() && self.bones[c].head != bone.head);
        match twist_child {
            Some(c) => (bone.head, self.bones[c].head),
            None => (bone.head, bone.tail),
        }
    }

    /// Bone head relative to its parent's head (the glTF / Maya joint translation).
    pub fn local_offset(&self, index: usize) -> Vec3 {
        let bone = &self.bones[index];
        match bone.parent {
            Some(p) => bone.head - self.bones[p].head,
            None => bone.head,
        }
    }
}

/// `point` turned by `radians` about `axis` through `pivot` (right-handed).
pub fn turn_about(pivot: Vec3, axis: Vec3, radians: f32, point: Vec3) -> Vec3 {
    let axis = axis.try_normalize().unwrap_or(Vec3::Y);
    pivot + glam::Quat::from_axis_angle(axis, radians) * (point - pivot)
}
