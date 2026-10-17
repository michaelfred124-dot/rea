//! Skeleton data: bones as head/tail segments in model space.
//!
//! Bone names follow the Unreal mannequin convention (`upperarm_l`, `calf_r`, ...)
//! so the result retargets cleanly in Unreal and reads naturally in Blender and Maya.

use glam::Vec3;
use serde::{Deserialize, Serialize};

/// Which body region a bone belongs to. Used to keep skin weights from bleeding
/// between limbs that happen to be close together (inner thighs, arms at the hips).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Region {
    Center,
    ArmL,
    ArmR,
    LegL,
    LegR,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
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
        });
        self.bones.len() - 1
    }

    pub fn children(&self, index: usize) -> impl Iterator<Item = usize> + '_ {
        self.bones
            .iter()
            .enumerate()
            .filter(move |(_, b)| b.parent == Some(index))
            .map(|(i, _)| i)
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
