//! Hive: an automatic humanoid rigger.
//!
//! Give it a character mesh in T-pose or A-pose (OBJ, glTF or GLB). It finds the
//! body landmarks, places premade components with editable guides, builds a
//! game-ready skeleton from them, computes skin weights, designs
//! an animator control rig, and exports it for Blender, Maya and Unreal Engine:
//!
//! * `<name>.glb` – skinned mesh + skeleton (imports natively in Blender and Unreal)
//! * `<name>_blender.py` – rebuilds mesh, armature, weights and controls in Blender
//! * `<name>_maya.py` – rebuilds mesh, joints, skinCluster and controls in Maya
//! * `<name>_unreal.py` – imports the .glb and builds a Control Rig Blueprint
//! * `<name>.guides.json` – the editable component layout, for rebuilding
//! * `<name>.hive.json` – the full rig description for other tools

pub mod components;
pub mod controls;
pub mod demo;
pub mod export;
pub mod import;
pub mod landmarks;
pub mod mesh;
pub mod naming;
pub mod rig;
pub mod skeleton;
pub mod skin;

pub use export::Target;
pub use rig::{Rig, RigOptions, auto_rig, build_rig};
