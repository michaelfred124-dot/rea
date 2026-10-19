//! Binary glTF 2.0 writer for the skinned mesh and skeleton.
//!
//! glTF is used as the interchange format because Blender and Unreal import it
//! natively and, unlike FBX, it is an open specification that can be written
//! without a proprietary SDK.

use glam::{Mat4, Vec2, Vec3, Vec4};
use serde_json::{Value, json};

use crate::mesh::{AlphaMode, Material, TextureRef};
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

fn vec2_bytes(v: &[Vec2]) -> Vec<u8> {
    f32_bytes(v.iter().flat_map(|p| p.to_array()))
}

fn vec4_bytes(v: &[Vec4]) -> Vec<u8> {
    f32_bytes(v.iter().flat_map(|p| p.to_array()))
}

fn bounds(v: &[Vec3]) -> Value {
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for p in v {
        min = min.min(*p);
        max = max.max(*p);
    }
    if v.is_empty() {
        min = Vec3::ZERO;
        max = Vec3::ZERO;
    }
    json!({ "min": min.to_array(), "max": max.to_array() })
}

fn texture_info(t: &TextureRef, textures: &mut Vec<Value>, key: Option<&str>) -> Value {
    textures.push(json!({ "source": t.image, "sampler": 0 }));
    let mut info = json!({ "index": textures.len() - 1 });
    if t.uv_set != 0 {
        info["texCoord"] = json!(t.uv_set);
    }
    if let Some(k) = key {
        info[k] = json!(t.scale);
    }
    info
}

fn material_json(m: &Material, textures: &mut Vec<Value>) -> Value {
    let mut pbr = json!({
        "baseColorFactor": m.base_color,
        "metallicFactor": m.metallic,
        "roughnessFactor": m.roughness,
    });
    if let Some(t) = &m.base_color_texture {
        pbr["baseColorTexture"] = texture_info(t, textures, None);
    }
    if let Some(t) = &m.metallic_roughness_texture {
        pbr["metallicRoughnessTexture"] = texture_info(t, textures, None);
    }
    let mut out = json!({ "name": m.name, "pbrMetallicRoughness": pbr });
    if m.emissive != [0.0; 3] {
        out["emissiveFactor"] = json!(m.emissive);
    }
    if let Some(t) = &m.normal_texture {
        out["normalTexture"] = texture_info(t, textures, Some("scale"));
    }
    if let Some(t) = &m.occlusion_texture {
        out["occlusionTexture"] = texture_info(t, textures, Some("strength"));
    }
    if let Some(t) = &m.emissive_texture {
        out["emissiveTexture"] = texture_info(t, textures, None);
    }
    match m.alpha_mode {
        AlphaMode::Opaque => {}
        AlphaMode::Mask => {
            out["alphaMode"] = json!("MASK");
            out["alphaCutoff"] = json!(m.alpha_cutoff);
        }
        AlphaMode::Blend => out["alphaMode"] = json!("BLEND"),
    }
    if m.double_sided {
        out["doubleSided"] = json!(true);
    }
    out
}

/// Encode the rig as a .glb file: one node per object of the mesh (each a mesh
/// with a primitive per piece and material), sharing one skin; normals, UV
/// sets, tangents and colours; the materials with their images embedded; the
/// blendshapes as morph targets named in `extras.targetNames`.
pub fn write(rig: &Rig) -> Vec<u8> {
    let mesh = &rig.mesh;
    let sk = &rig.skeleton;
    let mut b = Builder {
        bin: Vec::new(),
        views: Vec::new(),
        accessors: Vec::new(),
    };

    // Images, embedded so the file stands on its own (Unreal imports it as is).
    let mut images = Vec::new();
    for (i, image) in mesh.images.iter().enumerate() {
        match image.bytes() {
            Some(bytes) => {
                let view = b.push(&bytes, None);
                images.push(json!({ "name": image.name, "bufferView": view, "mimeType": image.mime_type() }));
            }
            None => {
                // Not readable: name the file, so a reader next to it can find it.
                eprintln!(
                    "warning: image {i} ('{}') isn't in the .glb: its file can't be read",
                    image.name
                );
                images.push(json!({ "name": image.name, "uri": image.uri }));
            }
        }
    }
    let mut textures = Vec::new();
    let mut materials: Vec<Value> = mesh
        .materials
        .iter()
        .map(|m| material_json(m, &mut textures))
        .collect();
    let pieces = mesh.pieces();
    let default_material = if materials.is_empty() || pieces.iter().any(|p| p.material.is_none()) {
        materials.push(json!({
            "name": format!("{}_mat", rig.name),
            "pbrMetallicRoughness": { "baseColorFactor": [0.8, 0.8, 0.8, 1.0], "metallicFactor": 0.0, "roughnessFactor": 0.6 },
        }));
        materials.len() - 1
    } else {
        0
    };

    let sets = rig.skin.width().div_ceil(4).max(1);
    let shapes_dense: Vec<Vec<Vec3>> = mesh
        .shapes
        .iter()
        .map(|s| s.dense(mesh.vertex_count()))
        .collect();
    let shape_normals: Vec<Option<Vec<Vec3>>> = mesh
        .shapes
        .iter()
        .map(|s| (!s.normal_offsets.is_empty()).then(|| s.dense_normals(mesh.vertex_count())))
        .collect();

    let groups = mesh.object_groups(&rig.name);
    let mut meshes = Vec::new();
    for (gi, g) in groups.iter().enumerate() {
        let r = g.vertices.clone();
        let count = r.len();
        let mut attributes = serde_json::Map::new();
        let pos = &mesh.positions[r.clone()];
        let a = b.accessor(
            &vec3_bytes(pos),
            Some(ARRAY_BUFFER),
            FLOAT,
            count,
            "VEC3",
            bounds(pos),
        );
        attributes.insert("POSITION".into(), json!(a));
        let a = b.accessor(
            &vec3_bytes(&mesh.normals[r.clone()]),
            Some(ARRAY_BUFFER),
            FLOAT,
            count,
            "VEC3",
            json!({}),
        );
        attributes.insert("NORMAL".into(), json!(a));
        if mesh.tangents.len() == mesh.vertex_count() {
            let a = b.accessor(
                &vec4_bytes(&mesh.tangents[r.clone()]),
                Some(ARRAY_BUFFER),
                FLOAT,
                count,
                "VEC4",
                json!({}),
            );
            attributes.insert("TANGENT".into(), json!(a));
        }
        for (k, set) in mesh.uvs.iter().enumerate() {
            let a = b.accessor(
                &vec2_bytes(&set[r.clone()]),
                Some(ARRAY_BUFFER),
                FLOAT,
                count,
                "VEC2",
                json!({}),
            );
            attributes.insert(format!("TEXCOORD_{k}"), json!(a));
        }
        if mesh.colors.len() == mesh.vertex_count() {
            let a = b.accessor(
                &vec4_bytes(&mesh.colors[r.clone()]),
                Some(ARRAY_BUFFER),
                FLOAT,
                count,
                "VEC4",
                json!({}),
            );
            attributes.insert("COLOR_0".into(), json!(a));
        }
        // glTF carries influences in sets of four (JOINTS_0/WEIGHTS_0, JOINTS_1/
        // WEIGHTS_1, ...); unused slots are joint 0 with weight 0.
        for set in 0..sets {
            let mut joints: Vec<u8> = Vec::with_capacity(count * 8);
            let mut weights: Vec<f32> = Vec::with_capacity(count * 4);
            for v in r.clone() {
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
                count,
                "VEC4",
                json!({}),
            );
            let wgt = b.accessor(
                &f32_bytes(weights),
                Some(ARRAY_BUFFER),
                FLOAT,
                count,
                "VEC4",
                json!({}),
            );
            attributes.insert(format!("JOINTS_{set}"), json!(jnt));
            attributes.insert(format!("WEIGHTS_{set}"), json!(wgt));
        }
        // Morph targets: the shapes that move this object (a shape that moves
        // nothing at all is kept on the first object, so it isn't lost).
        let mine: Vec<usize> = (0..mesh.shapes.len())
            .filter(|&s| {
                let sh = &mesh.shapes[s];
                sh.vertices.iter().any(|&v| r.contains(&(v as usize)))
                    || (gi == 0 && sh.vertices.is_empty())
            })
            .collect();
        let mut targets = Vec::new();
        for &s in &mine {
            let d = &shapes_dense[s][r.clone()];
            let pa = b.accessor(
                &vec3_bytes(d),
                Some(ARRAY_BUFFER),
                FLOAT,
                count,
                "VEC3",
                bounds(d),
            );
            let mut t = json!({ "POSITION": pa });
            if let Some(n) = &shape_normals[s] {
                let na = b.accessor(
                    &vec3_bytes(&n[r.clone()]),
                    Some(ARRAY_BUFFER),
                    FLOAT,
                    count,
                    "VEC3",
                    json!({}),
                );
                t["NORMAL"] = json!(na);
            }
            targets.push(t);
        }
        let primitives: Vec<Value> = g
            .parts
            .iter()
            .map(|p| {
                let local: Vec<u8> = mesh.indices[p.index_range()]
                    .iter()
                    .flat_map(|&i| (i - r.start as u32).to_le_bytes())
                    .collect();
                let idx = b.accessor(
                    &local,
                    Some(ELEMENT_ARRAY_BUFFER),
                    UNSIGNED_INT,
                    p.index_count,
                    "SCALAR",
                    json!({}),
                );
                let mut prim = json!({
                    "attributes": attributes,
                    "indices": idx,
                    "material": p.material.unwrap_or(default_material),
                });
                if !targets.is_empty() {
                    prim["targets"] = json!(targets);
                }
                prim
            })
            .collect();
        let mut m = json!({ "name": g.name, "primitives": primitives });
        if !mine.is_empty() {
            m["weights"] = json!(
                mine.iter()
                    .map(|&s| mesh.shapes[s].weight)
                    .collect::<Vec<_>>()
            );
            m["extras"] = json!({
                "targetNames": mine.iter().map(|&s| mesh.shapes[s].name.as_str()).collect::<Vec<_>>()
            });
        }
        meshes.push(m);
    }

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
    let first_mesh_node = nodes.len();
    for (i, g) in groups.iter().enumerate() {
        // A single object keeps the rig's name, as before objects were kept.
        let name = if groups.len() == 1 {
            rig.name.clone()
        } else {
            g.name.clone()
        };
        nodes.push(json!({ "name": name, "mesh": i, "skin": 0 }));
    }
    let roots: Vec<usize> = sk
        .bones
        .iter()
        .enumerate()
        .filter(|(_, b)| b.parent.is_none())
        .map(|(i, _)| i)
        .chain(first_mesh_node..first_mesh_node + groups.len())
        .collect();

    let mut doc = json!({
        "asset": { "version": "2.0", "generator": concat!("hive-rig ", env!("CARGO_PKG_VERSION")) },
        "scene": 0,
        "scenes": [{ "name": rig.name, "nodes": roots }],
        "nodes": nodes,
        "meshes": meshes,
        "materials": materials,
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
    if !images.is_empty() {
        doc["images"] = json!(images);
        doc["textures"] = json!(textures);
        doc["samplers"] = json!([{ "wrapS": 10497, "wrapT": 10497 }]);
    }

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

    #[test]
    fn glb_keeps_objects_materials_attributes_and_shapes() {
        let r = rig::auto_rig(
            crate::demo::dressed_humanoid(),
            "dressed",
            Default::default(),
        )
        .unwrap();
        let bytes = write(&r);
        let g = gltf::Gltf::from_slice(&bytes).expect("valid glb");
        // One mesh per object, sharing one skin; a primitive per piece.
        assert_eq!(g.meshes().count(), 2);
        let skinned: Vec<_> = g.nodes().filter(|n| n.mesh().is_some()).collect();
        assert_eq!(skinned.len(), 2);
        assert!(
            skinned
                .iter()
                .all(|n| n.skin().map(|s| s.index()) == Some(0))
        );
        assert_eq!(g.materials().count(), 2);
        assert_eq!(g.images().count(), 1);
        let img = g.images().next().unwrap();
        assert!(matches!(img.source(), gltf::image::Source::View { .. }));
        let gem = g
            .meshes()
            .find(|m| m.name() == Some("gem"))
            .expect("gem mesh");
        let names = gem
            .extras()
            .as_ref()
            .map(|e| e.get().to_string())
            .unwrap_or_default();
        assert!(names.contains("gem_puff"), "{names}");

        // Back through our own loader: the same pieces, attributes and shape.
        let dir = std::env::temp_dir().join(format!("hive-rig-glb-dressed-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("dressed.glb");
        std::fs::write(&path, &bytes).unwrap();
        let back = crate::import::load_mesh(&path).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        let m = &r.mesh;
        assert_eq!(back.objects, vec!["body".to_string(), "gem".to_string()]);
        assert_eq!(back.parts.len(), 2);
        assert_eq!(back.vertex_count(), m.vertex_count());
        assert_eq!(back.indices, m.indices);
        assert_eq!(back.uvs.len(), 2);
        for (a, b) in back.uvs.iter().flatten().zip(m.uvs.iter().flatten()) {
            assert!((*a - *b).length() < 1e-6);
        }
        assert_eq!(back.colors.len(), m.colors.len());
        for (a, b) in back.colors.iter().zip(&m.colors) {
            assert!((*a - *b).length() < 1e-6);
        }
        assert_eq!(
            back.materials[0].base_color_texture.map(|t| t.image),
            Some(0)
        );
        assert_eq!(back.materials[1].metallic, 1.0);
        assert_eq!(
            back.images[0].bytes().as_deref(),
            m.images[0].bytes().as_deref()
        );
        let (a, b) = (&back.shapes[0], &m.shapes[0]);
        assert_eq!(a.name, "gem_puff");
        assert_eq!(a.vertices, b.vertices);
        for (x, y) in a.offsets.iter().zip(&b.offsets) {
            assert!((*x - *y).length() < 1e-5);
        }
    }
}
