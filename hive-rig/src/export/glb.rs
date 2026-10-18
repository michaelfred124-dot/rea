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
    // glTF carries influences in sets of four (JOINTS_0/WEIGHTS_0, JOINTS_1/
    // WEIGHTS_1, ...); unused slots are joint 0 with weight 0.
    let sets = rig.skin.width().div_ceil(4).max(1);
    let mut skin_attributes = serde_json::Map::new();
    for set in 0..sets {
        let mut joints: Vec<u8> = Vec::with_capacity(mesh.vertex_count() * 8);
        let mut weights: Vec<f32> = Vec::with_capacity(mesh.vertex_count() * 4);
        for v in 0..mesh.vertex_count() {
            let pairs: Vec<(usize, f32)> = if v < rig.skin.vertex_count() {
                rig.skin.influences(v).collect()
            } else {
                Vec::new()
            };
            for slot in set * 4..set * 4 + 4 {
                let (j, w) = pairs.get(slot).copied().unwrap_or((0, 0.0));
                joints.extend_from_slice(&(j as u16).to_le_bytes());
                weights.push(w);
            }
        }
        let jnt = b.accessor(
            &joints,
            Some(ARRAY_BUFFER),
            UNSIGNED_SHORT,
            mesh.vertex_count(),
            "VEC4",
            json!({}),
        );
        let wgt = b.accessor(
            &f32_bytes(weights),
            Some(ARRAY_BUFFER),
            FLOAT,
            mesh.vertex_count(),
            "VEC4",
            json!({}),
        );
        skin_attributes.insert(format!("JOINTS_{set}"), json!(jnt));
        skin_attributes.insert(format!("WEIGHTS_{set}"), json!(wgt));
    }
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

    let mut attributes = serde_json::Map::new();
    attributes.insert("POSITION".into(), json!(pos));
    attributes.insert("NORMAL".into(), json!(nrm));
    attributes.extend(skin_attributes);

    let doc = json!({
        "asset": { "version": "2.0", "generator": concat!("hive-rig ", env!("CARGO_PKG_VERSION")) },
        "scene": 0,
        "scenes": [{ "name": rig.name, "nodes": roots }],
        "nodes": nodes,
        "meshes": [{
            "name": rig.name,
            "primitives": [{
                "attributes": attributes,
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

    #[test]
    fn more_than_four_influences_go_into_a_second_set() {
        let mesh = demo_humanoid();
        let (mut guides, _) = rig::detect_guides(&mesh);
        guides.skin.max_influences = 8;
        let r = rig::build_rig(mesh, "demo", guides, Default::default()).unwrap();
        assert!(r.skin.width() > 4);
        let bytes = write(&r);
        let g = gltf::Gltf::from_slice(&bytes).expect("valid glb");
        let blob = g.blob.as_deref().unwrap();
        let prim = g.meshes().next().unwrap().primitives().next().unwrap();
        let reader = prim.reader(|_| Some(blob));
        let j0: Vec<[u16; 4]> = reader.read_joints(0).unwrap().into_u16().collect();
        let j1: Vec<[u16; 4]> = reader.read_joints(1).unwrap().into_u16().collect();
        let w0: Vec<[f32; 4]> = reader.read_weights(0).unwrap().into_f32().collect();
        let w1: Vec<[f32; 4]> = reader.read_weights(1).unwrap().into_f32().collect();
        assert!(reader.read_joints(2).is_none());
        for v in 0..r.mesh.vertex_count() {
            let sum: f32 = w0[v].iter().chain(&w1[v]).sum();
            assert!((sum - 1.0).abs() < 1e-4, "vertex {v} sums to {sum}");
            for (k, (j, w)) in r.skin.influences(v).enumerate() {
                let (gj, gw) = if k < 4 {
                    (j0[v][k], w0[v][k])
                } else {
                    (j1[v][k - 4], w1[v][k - 4])
                };
                assert_eq!((gj as usize, gw), (j, w));
            }
        }
        // Four influences need only the first set.
        let four = rig::auto_rig(demo_humanoid(), "demo", Default::default()).unwrap();
        let g = gltf::Gltf::from_slice(&write(&four)).unwrap();
        let blob = g.blob.as_deref().unwrap();
        let prim = g.meshes().next().unwrap().primitives().next().unwrap();
        assert!(prim.reader(|_| Some(blob)).read_joints(1).is_none());
    }
}
