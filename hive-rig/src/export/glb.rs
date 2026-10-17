//! Binary glTF 2.0 writer for the skinned mesh and skeleton.
//!
//! glTF is used as the interchange format because Blender and Unreal import it
//! natively and, unlike FBX, it is an open specification that can be written
//! without a proprietary SDK.

use glam::{Mat4, Vec3};
use serde_json::{Value, json};

use crate::rig::Rig;

const ARRAY_BUFFER: u32 = 34962;
const ELEMENT_ARRAY_BUFFER: u32 = 34963;
const FLOAT: u32 = 5126;
const UNSIGNED_SHORT: u32 = 5123;
const UNSIGNED_INT: u32 = 5125;

struct Builder {
    bin: Vec<u8>,
    views: Vec<Value>,
    accessors: Vec<Value>,
}

impl Builder {
    fn push(&mut self, bytes: &[u8], target: Option<u32>) -> usize {
        while !self.bin.len().is_multiple_of(4) {
            self.bin.push(0);
        }
        let mut view = json!({
            "buffer": 0,
            "byteOffset": self.bin.len(),
            "byteLength": bytes.len(),
        });
        if let Some(t) = target {
            view["target"] = json!(t);
        }
        self.bin.extend_from_slice(bytes);
        self.views.push(view);
        self.views.len() - 1
    }

    fn accessor(
        &mut self,
        bytes: &[u8],
        target: Option<u32>,
        component: u32,
        count: usize,
        kind: &str,
        extra: Value,
    ) -> usize {
        let view = self.push(bytes, target);
        let mut acc = json!({
            "bufferView": view,
            "componentType": component,
            "count": count,
            "type": kind,
        });
        if let (Value::Object(a), Value::Object(e)) = (&mut acc, extra) {
            a.extend(e);
        }
        self.accessors.push(acc);
        self.accessors.len() - 1
    }
}

fn f32_bytes(values: impl IntoIterator<Item = f32>) -> Vec<u8> {
    values.into_iter().flat_map(f32::to_le_bytes).collect()
}

fn vec3_bytes(v: &[Vec3]) -> Vec<u8> {
    f32_bytes(v.iter().flat_map(|p| p.to_array()))
}

/// Encode the rig's mesh, skeleton and weights as a .glb file.
pub fn write(rig: &Rig) -> Vec<u8> {
    let mesh = &rig.mesh;
    let sk = &rig.skeleton;
    let mut b = Builder {
        bin: Vec::new(),
        views: Vec::new(),
        accessors: Vec::new(),
    };

    let (min, max) = mesh.bounds();
    let pos = b.accessor(
        &vec3_bytes(&mesh.positions),
        Some(ARRAY_BUFFER),
        FLOAT,
        mesh.vertex_count(),
        "VEC3",
        json!({ "min": min.to_array(), "max": max.to_array() }),
    );
    let nrm = b.accessor(
        &vec3_bytes(&mesh.normals),
        Some(ARRAY_BUFFER),
        FLOAT,
        mesh.vertex_count(),
        "VEC3",
        json!({}),
    );
    let joints: Vec<u8> = rig
        .skin
        .joints
        .iter()
        .flatten()
        .flat_map(|j| j.to_le_bytes())
        .collect();
    let jnt = b.accessor(
        &joints,
        Some(ARRAY_BUFFER),
        UNSIGNED_SHORT,
        mesh.vertex_count(),
        "VEC4",
        json!({}),
    );
    let wgt = b.accessor(
        &f32_bytes(rig.skin.weights.iter().flatten().copied()),
        Some(ARRAY_BUFFER),
        FLOAT,
        mesh.vertex_count(),
        "VEC4",
        json!({}),
    );
    let idx_bytes: Vec<u8> = mesh.indices.iter().flat_map(|i| i.to_le_bytes()).collect();
    let idx = b.accessor(
        &idx_bytes,
        Some(ELEMENT_ARRAY_BUFFER),
        UNSIGNED_INT,
        mesh.indices.len(),
        "SCALAR",
        json!({}),
    );
    // Bind pose: joints carry translation only, so the inverse bind matrix of each
    // joint is the inverse of its model-space head translation.
    let ibm = f32_bytes(
        sk.bones
            .iter()
            .flat_map(|bone| Mat4::from_translation(-bone.head).to_cols_array()),
    );
    let ibm = b.accessor(&ibm, None, FLOAT, sk.bones.len(), "MAT4", json!({}));

    let mut nodes: Vec<Value> = sk
        .bones
        .iter()
        .enumerate()
        .map(|(i, bone)| {
            let mut node = json!({
                "name": bone.name,
                "translation": sk.local_offset(i).to_array(),
            });
            let children: Vec<usize> = sk.children(i).collect();
            if !children.is_empty() {
                node["children"] = json!(children);
            }
            node
        })
        .collect();
    let mesh_node = nodes.len();
    nodes.push(json!({ "name": rig.name, "mesh": 0, "skin": 0 }));
    let roots: Vec<usize> = sk
        .bones
        .iter()
        .enumerate()
        .filter(|(_, b)| b.parent.is_none())
        .map(|(i, _)| i)
        .chain([mesh_node])
        .collect();

    let doc = json!({
        "asset": { "version": "2.0", "generator": concat!("hive-rig ", env!("CARGO_PKG_VERSION")) },
        "scene": 0,
        "scenes": [{ "name": rig.name, "nodes": roots }],
        "nodes": nodes,
        "meshes": [{
            "name": rig.name,
            "primitives": [{
                "attributes": { "POSITION": pos, "NORMAL": nrm, "JOINTS_0": jnt, "WEIGHTS_0": wgt },
                "indices": idx,
                "material": 0,
            }],
        }],
        "materials": [{
            "name": format!("{}_mat", rig.name),
            "pbrMetallicRoughness": { "baseColorFactor": [0.8, 0.8, 0.8, 1.0], "metallicFactor": 0.0, "roughnessFactor": 0.6 },
        }],
        "skins": [{
            "name": format!("{}_skin", rig.name),
            "inverseBindMatrices": ibm,
            "skeleton": roots[0],
            "joints": (0..sk.bones.len()).collect::<Vec<_>>(),
        }],
        "accessors": b.accessors,
        "bufferViews": b.views,
        "buffers": [{ "byteLength": b.bin.len() }],
    });

    let mut json_bytes = serde_json::to_vec(&doc).expect("glTF JSON serialises");
    while !json_bytes.len().is_multiple_of(4) {
        json_bytes.push(b' ');
    }
    let mut bin = b.bin;
    while !bin.len().is_multiple_of(4) {
        bin.push(0);
    }
    let total = 12 + 8 + json_bytes.len() + 8 + bin.len();
    let mut out = Vec::with_capacity(total);
    out.extend_from_slice(&0x4654_6C67u32.to_le_bytes()); // "glTF"
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&(total as u32).to_le_bytes());
    out.extend_from_slice(&(json_bytes.len() as u32).to_le_bytes());
    out.extend_from_slice(&0x4E4F_534Au32.to_le_bytes()); // "JSON"
    out.extend_from_slice(&json_bytes);
    out.extend_from_slice(&(bin.len() as u32).to_le_bytes());
    out.extend_from_slice(&0x004E_4942u32.to_le_bytes()); // "BIN\0"
    out.extend_from_slice(&bin);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{demo::demo_humanoid, rig};

    #[test]
    fn glb_round_trips_through_the_gltf_crate() {
        let r = rig::auto_rig(demo_humanoid(), "demo", Default::default()).unwrap();
        let bytes = write(&r);
        let g = gltf::Gltf::from_slice(&bytes).expect("valid glb");
        let skin = g.skins().next().expect("skin");
        assert_eq!(skin.joints().count(), r.skeleton.bones.len());
        let blob = g.blob.as_deref().unwrap();
        let prim = g.meshes().next().unwrap().primitives().next().unwrap();
        let reader = prim.reader(|_| Some(blob));
        assert_eq!(
            reader.read_positions().unwrap().count(),
            r.mesh.vertex_count()
        );
        let weights: Vec<[f32; 4]> = reader.read_weights(0).unwrap().into_f32().collect();
        assert!(
            weights
                .iter()
                .all(|w| (w.iter().sum::<f32>() - 1.0).abs() < 1e-4)
        );
        // Re-importing the mesh through our own loader gives the same vertex count.
        let dir = std::env::temp_dir().join(format!("hive-rig-glb-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("demo.glb");
        std::fs::write(&path, &bytes).unwrap();
        let back = crate::import::load_mesh(&path).unwrap();
        assert_eq!(back.vertex_count(), r.mesh.vertex_count());
        std::fs::remove_dir_all(&dir).ok();
    }
}
