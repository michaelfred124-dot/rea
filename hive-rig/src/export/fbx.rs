//! Binary FBX 7.4 writer for the skinned mesh, skeleton, skin weights and bind
//! pose, for game engines (Unreal and Unity import it directly).
//!
//! Written from the publicly described binary layout: a fixed header, a tree of
//! named records whose properties are typed scalars, strings and arrays, and a
//! footer. Arrays are stored uncompressed (encoding 0), so no zlib is needed.
//!
//! The scene is Y-up, right-handed, facing +Z (the rig's model space), in
//! centimetres (`UnitScaleFactor` 1), the unit Unreal and Maya work in. Unity
//! and Blender scale it to metres on import.
//!
//! * One `Mesh` model with its geometry (triangles, smooth normals, one material).
//! * One `LimbNode` model per bone, named with the rig's naming convention and
//!   parented as in the skeleton. Joints carry translation only, like the .glb.
//! * A `Skin` deformer with one `Cluster` per bone that weighs on the mesh,
//!   holding every influence the skin stores (no limit of four).
//! * A `BindPose` with the mesh and every bone.

use glam::{Mat4, Vec3};

use crate::rig::Rig;

const VERSION: u32 = 7400;
/// Model space is metres; FBX files here are centimetres.
const CM: f32 = 100.0;

const MAGIC: &[u8; 23] = b"Kaydara FBX Binary  \x00\x1a\x00";
/// A fixed file id and creation time, so the output is reproducible. Readers
/// pair the file id with the footer code below.
const FILE_ID: [u8; 16] = [
    0x28, 0xb3, 0x2a, 0xeb, 0xb6, 0x24, 0xcc, 0xc2, 0xbf, 0xc8, 0xb0, 0x2a, 0xa9, 0x2b, 0xfc, 0xf1,
];
const CREATION_TIME: &str = "1970-01-01 10:00:00:000";
const FOOTER_CODE: [u8; 16] = [
    0xfa, 0xbc, 0xab, 0x09, 0xd0, 0xc8, 0xd4, 0x66, 0xb1, 0x76, 0xfb, 0x83, 0x1c, 0xf7, 0x26, 0x7e,
];
const FOOTER_MAGIC: [u8; 16] = [
    0xf8, 0x5a, 0x8c, 0x6a, 0xde, 0xf5, 0xd9, 0x7e, 0xec, 0xe9, 0x0c, 0xe3, 0x75, 0x8f, 0x29, 0x0b,
];

/// A typed property of a record.
#[derive(Debug, Clone, PartialEq)]
enum Prop {
    Bool(bool),
    I32(i32),
    I64(i64),
    F64(f64),
    Str(String),
    Raw(Vec<u8>),
    I32s(Vec<i32>),
    F64s(Vec<f64>),
}

impl From<&str> for Prop {
    fn from(s: &str) -> Self {
        Prop::Str(s.to_string())
    }
}
impl From<String> for Prop {
    fn from(s: String) -> Self {
        Prop::Str(s)
    }
}
impl From<i32> for Prop {
    fn from(v: i32) -> Self {
        Prop::I32(v)
    }
}
impl From<i64> for Prop {
    fn from(v: i64) -> Self {
        Prop::I64(v)
    }
}
impl From<f64> for Prop {
    fn from(v: f64) -> Self {
        Prop::F64(v)
    }
}
impl From<bool> for Prop {
    fn from(v: bool) -> Self {
        Prop::Bool(v)
    }
}

#[derive(Debug, Clone, Default)]
struct Node {
    name: &'static str,
    props: Vec<Prop>,
    children: Vec<Node>,
}

impl Node {
    fn new(name: &'static str, props: impl IntoIterator<Item = Prop>) -> Self {
        Node {
            name,
            props: props.into_iter().collect(),
            children: Vec::new(),
        }
    }

    fn with(mut self, children: impl IntoIterator<Item = Node>) -> Self {
        self.children.extend(children);
        self
    }

    /// A record holding one value, such as `Version: 100`.
    fn one(name: &'static str, value: impl Into<Prop>) -> Self {
        Node::new(name, [value.into()])
    }

    fn encode(&self, out: &mut Vec<u8>) {
        let start = out.len();
        // End offset, property count and property bytes are patched once known.
        out.extend_from_slice(&[0; 12]);
        out.push(self.name.len() as u8);
        out.extend_from_slice(self.name.as_bytes());
        let props_start = out.len();
        for p in &self.props {
            encode_prop(p, out);
        }
        let props_len = (out.len() - props_start) as u32;
        for c in &self.children {
            c.encode(out);
        }
        // A nested list (and an empty record) ends with a null record.
        if !self.children.is_empty() || self.props.is_empty() {
            out.extend_from_slice(&[0; 13]);
        }
        let end = out.len() as u32;
        out[start..start + 4].copy_from_slice(&end.to_le_bytes());
        out[start + 4..start + 8].copy_from_slice(&(self.props.len() as u32).to_le_bytes());
        out[start + 8..start + 12].copy_from_slice(&props_len.to_le_bytes());
    }
}

fn encode_prop(p: &Prop, out: &mut Vec<u8>) {
    fn array(out: &mut Vec<u8>, code: u8, count: usize, bytes: impl Iterator<Item = u8>) {
        out.push(code);
        out.extend_from_slice(&(count as u32).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // uncompressed
        let len_at = out.len();
        out.extend_from_slice(&0u32.to_le_bytes());
        let data_start = out.len();
        out.extend(bytes);
        let len = (out.len() - data_start) as u32;
        out[len_at..len_at + 4].copy_from_slice(&len.to_le_bytes());
    }
    match p {
        Prop::Bool(v) => {
            out.push(b'C');
            out.push(u8::from(*v));
        }
        Prop::I32(v) => {
            out.push(b'I');
            out.extend_from_slice(&v.to_le_bytes());
        }
        Prop::I64(v) => {
            out.push(b'L');
            out.extend_from_slice(&v.to_le_bytes());
        }
        Prop::F64(v) => {
            out.push(b'D');
            out.extend_from_slice(&v.to_le_bytes());
        }
        Prop::Str(s) => {
            out.push(b'S');
            out.extend_from_slice(&(s.len() as u32).to_le_bytes());
            out.extend_from_slice(s.as_bytes());
        }
        Prop::Raw(b) => {
            out.push(b'R');
            out.extend_from_slice(&(b.len() as u32).to_le_bytes());
            out.extend_from_slice(b);
        }
        Prop::I32s(v) => array(out, b'i', v.len(), v.iter().flat_map(|x| x.to_le_bytes())),
        Prop::F64s(v) => array(out, b'd', v.len(), v.iter().flat_map(|x| x.to_le_bytes())),
    }
}

/// `Name\0\x01Class`, the binary form of an object's `Class::Name`.
fn object_name(name: &str, class: &str) -> String {
    format!("{name}\0\x01{class}")
}

/// One `P` entry of a `Properties70` block.
fn p(
    name: &str,
    kind: &str,
    label: &str,
    flags: &str,
    values: impl IntoIterator<Item = Prop>,
) -> Node {
    Node::new(
        "P",
        [name.into(), kind.into(), label.into(), flags.into()]
            .into_iter()
            .chain(values),
    )
}

fn p_int(name: &str, v: i32) -> Node {
    p(name, "int", "Integer", "", [v.into()])
}

fn p_double(name: &str, v: f64) -> Node {
    p(name, "double", "Number", "", [v.into()])
}

fn p_vec(name: &str, kind: &str, v: Vec3) -> Node {
    p(
        name,
        kind,
        "",
        "A",
        v.to_array().map(|x| Prop::F64(x as f64)),
    )
}

fn properties(entries: impl IntoIterator<Item = Node>) -> Node {
    Node::new("Properties70", []).with(entries)
}

fn matrix(m: Mat4) -> Prop {
    // FBX stores 4x4 matrices with the translation in elements 12-14, the same
    // order as glam's column array.
    Prop::F64s(m.to_cols_array().iter().map(|&x| x as f64).collect())
}

fn connect(child: i64, parent: i64) -> Node {
    Node::new("C", ["OO".into(), child.into(), parent.into()])
}

/// Encode the rig's mesh, skeleton, skin weights and bind pose as a binary FBX.
pub fn write(rig: &Rig) -> Vec<u8> {
    let mesh = &rig.mesh;
    let sk = &rig.skeleton;

    // Object ids: any distinct non-zero 64-bit numbers (0 is the scene root).
    let geometry_id: i64 = 1000;
    let mesh_id: i64 = 1001;
    let material_id: i64 = 1002;
    let skin_id: i64 = 1003;
    let pose_id: i64 = 1004;
    let bone_id = |i: usize| 2000 + 3 * i as i64;
    let attribute_id = |i: usize| 2001 + 3 * i as i64;
    let cluster_id = |i: usize| 2002 + 3 * i as i64;

    // Every bone's influences, gathered per bone for its cluster.
    let mut clusters: Vec<(Vec<i32>, Vec<f64>)> = vec![(Vec::new(), Vec::new()); sk.bones.len()];
    for v in 0..mesh.vertex_count().min(rig.skin.vertex_count()) {
        for (j, w) in rig.skin.influences(v) {
            if let Some(c) = clusters.get_mut(j) {
                c.0.push(v as i32);
                c.1.push(w as f64);
            }
        }
    }
    let skinned: Vec<usize> = (0..sk.bones.len())
        .filter(|&i| !clusters[i].0.is_empty())
        .collect();

    let mut objects = Vec::new();

    // Geometry: triangles, with the last corner of each polygon stored as !index.
    let vertices: Vec<f64> = mesh
        .positions
        .iter()
        .flat_map(|p| (*p * CM).to_array())
        .map(|x| x as f64)
        .collect();
    let polygons: Vec<i32> = mesh
        .triangles()
        .flat_map(|[a, b, c]| [a as i32, b as i32, !(c as i32)])
        .collect();
    let normals: Vec<f64> = mesh
        .normals
        .iter()
        .flat_map(|n| n.to_array())
        .map(|x| x as f64)
        .collect();
    objects.push(
        Node::new(
            "Geometry",
            [
                geometry_id.into(),
                object_name(&rig.name, "Geometry").into(),
                "Mesh".into(),
            ],
        )
        .with([
            Node::one("GeometryVersion", 124),
            Node::one("Vertices", Prop::F64s(vertices)),
            Node::one("PolygonVertexIndex", Prop::I32s(polygons)),
            Node::one("LayerElementNormal", 0).with([
                Node::one("Version", 101),
                Node::one("Name", ""),
                Node::one("MappingInformationType", "ByVertice"),
                Node::one("ReferenceInformationType", "Direct"),
                Node::one("Normals", Prop::F64s(normals)),
            ]),
            Node::one("LayerElementMaterial", 0).with([
                Node::one("Version", 101),
                Node::one("Name", ""),
                Node::one("MappingInformationType", "AllSame"),
                Node::one("ReferenceInformationType", "IndexToDirect"),
                Node::one("Materials", Prop::I32s(vec![0])),
            ]),
            Node::one("Layer", 0).with([
                Node::one("Version", 100),
                Node::new("LayerElement", []).with([
                    Node::one("Type", "LayerElementNormal"),
                    Node::one("TypedIndex", 0),
                ]),
                Node::new("LayerElement", []).with([
                    Node::one("Type", "LayerElementMaterial"),
                    Node::one("TypedIndex", 0),
                ]),
            ]),
        ]),
    );

    let model = |id: i64, name: &str, kind: &str, translation: Vec3| {
        Node::new(
            "Model",
            [id.into(), object_name(name, "Model").into(), kind.into()],
        )
        .with([
            Node::one("Version", 232),
            properties([
                p_vec("Lcl Translation", "Lcl Translation", translation),
                p_vec("Lcl Rotation", "Lcl Rotation", Vec3::ZERO),
                p_vec("Lcl Scaling", "Lcl Scaling", Vec3::ONE),
            ]),
            Node::one("Shading", true),
            Node::one("Culling", "CullingOff"),
        ])
    };
    objects.push(model(mesh_id, &rig.name, "Mesh", Vec3::ZERO));

    objects.push(
        Node::new(
            "Material",
            [
                material_id.into(),
                object_name(&format!("{}_mat", rig.name), "Material").into(),
                "".into(),
            ],
        )
        .with([
            Node::one("Version", 102),
            Node::one("ShadingModel", "lambert"),
            Node::one("MultiLayer", 0),
            properties([p_vec("DiffuseColor", "Color", Vec3::splat(0.8))]),
        ]),
    );

    // Bones: a LimbNode model and its skeleton attribute each.
    for (i, bone) in sk.bones.iter().enumerate() {
        objects.push(model(
            bone_id(i),
            &bone.name,
            "LimbNode",
            sk.local_offset(i) * CM,
        ));
        objects.push(
            Node::new(
                "NodeAttribute",
                [
                    attribute_id(i).into(),
                    object_name(&bone.name, "NodeAttribute").into(),
                    "LimbNode".into(),
                ],
            )
            .with([
                properties([p_double("Size", (bone.length() * CM) as f64)]),
                Node::one("TypeFlags", "Skeleton"),
            ]),
        );
    }

    // Skin: one cluster per weighted bone. The bind pose is the rest pose, with
    // the mesh at the origin: each link sits at its head, and `Transform` is the
    // mesh's bind transform relative to the link.
    let bind = |i: usize| Mat4::from_translation(sk.bones[i].head * CM);
    objects.push(
        Node::new(
            "Deformer",
            [
                skin_id.into(),
                object_name(&format!("{}_skin", rig.name), "Deformer").into(),
                "Skin".into(),
            ],
        )
        .with([
            Node::one("Version", 101),
            Node::one("Link_DeformAcuracy", 50.0),
        ]),
    );
    for &i in &skinned {
        let (indexes, weights) = std::mem::take(&mut clusters[i]);
        objects.push(
            Node::new(
                "Deformer",
                [
                    cluster_id(i).into(),
                    object_name(&sk.bones[i].name, "SubDeformer").into(),
                    "Cluster".into(),
                ],
            )
            .with([
                Node::one("Version", 100),
                Node::new("UserData", ["".into(), "".into()]),
                Node::one("Indexes", Prop::I32s(indexes)),
                Node::one("Weights", Prop::F64s(weights)),
                Node::one("Transform", matrix(bind(i).inverse())),
                Node::one("TransformLink", matrix(bind(i))),
            ]),
        );
    }

    let pose_node = |id: i64, m: Mat4| {
        Node::new("PoseNode", []).with([Node::one("Node", id), Node::one("Matrix", matrix(m))])
    };
    objects.push(
        Node::new(
            "Pose",
            [
                pose_id.into(),
                object_name(&format!("{}_bind", rig.name), "Pose").into(),
                "BindPose".into(),
            ],
        )
        .with(
            [
                Node::one("Type", "BindPose"),
                Node::one("Version", 100),
                Node::one("NbPoseNodes", (sk.bones.len() + 1) as i32),
                pose_node(mesh_id, Mat4::IDENTITY),
            ]
            .into_iter()
            .chain((0..sk.bones.len()).map(|i| pose_node(bone_id(i), bind(i)))),
        ),
    );

    let mut connections = vec![
        connect(mesh_id, 0),
        connect(geometry_id, mesh_id),
        connect(material_id, mesh_id),
        connect(skin_id, geometry_id),
    ];
    for (i, bone) in sk.bones.iter().enumerate() {
        connections.push(connect(bone_id(i), bone.parent.map_or(0, bone_id)));
        connections.push(connect(attribute_id(i), bone_id(i)));
    }
    for &i in &skinned {
        connections.push(connect(cluster_id(i), skin_id));
        connections.push(connect(bone_id(i), cluster_id(i)));
    }

    let definitions = {
        let types: [(&str, usize); 7] = [
            ("GlobalSettings", 1),
            ("Model", sk.bones.len() + 1),
            ("Geometry", 1),
            ("Material", 1),
            ("NodeAttribute", sk.bones.len()),
            ("Deformer", skinned.len() + 1),
            ("Pose", 1),
        ];
        let total: usize = types.iter().map(|t| t.1).sum();
        Node::new("Definitions", []).with(
            [Node::one("Version", 100), Node::one("Count", total as i32)]
                .into_iter()
                .chain(types.iter().map(|&(name, count)| {
                    Node::one("ObjectType", name).with([Node::one("Count", count as i32)])
                })),
        )
    };

    let creator = concat!("hive-rig ", env!("CARGO_PKG_VERSION"));
    let top = [
        Node::new("FBXHeaderExtension", []).with([
            Node::one("FBXHeaderVersion", 1003),
            Node::one("FBXVersion", VERSION as i32),
            Node::one("EncryptionType", 0),
            Node::new("CreationTimeStamp", []).with([
                Node::one("Version", 1000),
                Node::one("Year", 1970),
                Node::one("Month", 1),
                Node::one("Day", 1),
                Node::one("Hour", 10),
                Node::one("Minute", 0),
                Node::one("Second", 0),
                Node::one("Millisecond", 0),
            ]),
            Node::one("Creator", creator),
            Node::new(
                "SceneInfo",
                [
                    object_name("GlobalInfo", "SceneInfo").into(),
                    "UserData".into(),
                ],
            )
            .with([
                Node::one("Type", "UserData"),
                Node::one("Version", 100),
                Node::new("MetaData", []).with([
                    Node::one("Version", 100),
                    Node::one("Title", rig.name.as_str()),
                    Node::one("Subject", ""),
                    Node::one("Author", ""),
                    Node::one("Keywords", ""),
                    Node::one("Revision", ""),
                    Node::one("Comment", ""),
                ]),
            ]),
        ]),
        Node::one("FileId", Prop::Raw(FILE_ID.to_vec())),
        Node::one("CreationTime", CREATION_TIME),
        Node::one("Creator", creator),
        Node::new("GlobalSettings", []).with([
            Node::one("Version", 1000),
            properties([
                p_int("UpAxis", 1),
                p_int("UpAxisSign", 1),
                p_int("FrontAxis", 2),
                p_int("FrontAxisSign", 1),
                p_int("CoordAxis", 0),
                p_int("CoordAxisSign", 1),
                p_int("OriginalUpAxis", 1),
                p_int("OriginalUpAxisSign", 1),
                p_double("UnitScaleFactor", 1.0),
                p_double("OriginalUnitScaleFactor", 1.0),
            ]),
        ]),
        Node::new("Documents", []).with([
            Node::one("Count", 1),
            Node::new("Document", [9000i64.into(), "Scene".into(), "Scene".into()]).with([
                properties([
                    p("SourceObject", "object", "", "", []),
                    p("ActiveAnimStackName", "KString", "", "", ["".into()]),
                ]),
                Node::one("RootNode", 0i64),
            ]),
        ]),
        Node::new("References", []),
        definitions,
        Node::new("Objects", []).with(objects),
        Node::new("Connections", []).with(connections),
        Node::new("Takes", []).with([Node::one("Current", "")]),
    ];

    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&VERSION.to_le_bytes());
    for node in &top {
        node.encode(&mut out);
    }
    out.extend_from_slice(&[0; 13]);
    // Footer: code, zero padding to a 16-byte boundary, version, 120 zero bytes,
    // closing magic.
    out.extend_from_slice(&FOOTER_CODE);
    let pad = 16 - out.len() % 16;
    out.extend(std::iter::repeat_n(0u8, pad));
    out.extend_from_slice(&VERSION.to_le_bytes());
    out.extend_from_slice(&[0; 120]);
    out.extend_from_slice(&FOOTER_MAGIC);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{demo::demo_humanoid, rig};

    /// A record read back from the file.
    #[derive(Debug)]
    struct Read {
        name: String,
        props: Vec<Prop>,
        children: Vec<Read>,
    }

    impl Read {
        fn child(&self, name: &str) -> &Read {
            self.children
                .iter()
                .find(|c| c.name == name)
                .unwrap_or_else(|| panic!("no {name} under {}", self.name))
        }
        fn all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Read> + 'a {
            self.children.iter().filter(move |c| c.name == name)
        }
    }

    fn u32_at(b: &[u8], at: usize) -> u32 {
        u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
    }

    /// A strict reader for the subset the writer produces: every offset and
    /// length must agree.
    fn read_node(b: &[u8], at: &mut usize) -> Option<Read> {
        let end = u32_at(b, *at) as usize;
        let count = u32_at(b, *at + 4) as usize;
        let props_len = u32_at(b, *at + 8) as usize;
        let name_len = b[*at + 12] as usize;
        if end == 0 {
            assert_eq!((count, props_len, name_len), (0, 0, 0));
            *at += 13;
            return None;
        }
        let name = String::from_utf8(b[*at + 13..*at + 13 + name_len].to_vec()).unwrap();
        *at += 13 + name_len;
        let props_end = *at + props_len;
        let mut props = Vec::new();
        for _ in 0..count {
            let code = b[*at];
            *at += 1;
            let take = |at: &mut usize, n: usize| {
                let s = &b[*at..*at + n];
                *at += n;
                s
            };
            props.push(match code {
                b'C' => Prop::Bool(take(at, 1)[0] != 0),
                b'I' => Prop::I32(i32::from_le_bytes(take(at, 4).try_into().unwrap())),
                b'L' => Prop::I64(i64::from_le_bytes(take(at, 8).try_into().unwrap())),
                b'D' => Prop::F64(f64::from_le_bytes(take(at, 8).try_into().unwrap())),
                b'S' | b'R' => {
                    let n = u32_at(b, *at) as usize;
                    *at += 4;
                    let s = take(at, n).to_vec();
                    if code == b'S' {
                        Prop::Str(String::from_utf8(s).unwrap())
                    } else {
                        Prop::Raw(s)
                    }
                }
                b'i' | b'd' => {
                    let n = u32_at(b, *at) as usize;
                    assert_eq!(u32_at(b, *at + 4), 0, "uncompressed");
                    let len = u32_at(b, *at + 8) as usize;
                    assert_eq!(len, n * if code == b'i' { 4 } else { 8 });
                    *at += 12;
                    let s = take(at, len);
                    if code == b'i' {
                        Prop::I32s(
                            s.chunks(4)
                                .map(|c| i32::from_le_bytes(c.try_into().unwrap()))
                                .collect(),
                        )
                    } else {
                        Prop::F64s(
                            s.chunks(8)
                                .map(|c| f64::from_le_bytes(c.try_into().unwrap()))
                                .collect(),
                        )
                    }
                }
                other => panic!("unexpected property type {}", other as char),
            });
        }
        assert_eq!(*at, props_end, "{name}: property length");
        let mut children = Vec::new();
        if *at < end {
            while let Some(c) = read_node(b, at) {
                children.push(c);
            }
        }
        assert_eq!(*at, end, "{name}: end offset");
        Some(Read {
            name,
            props,
            children,
        })
    }

    fn parse(bytes: &[u8]) -> Read {
        assert_eq!(&bytes[..23], MAGIC);
        assert_eq!(u32_at(bytes, 23), VERSION);
        let mut at = 27;
        let mut children = Vec::new();
        while let Some(n) = read_node(bytes, &mut at) {
            children.push(n);
        }
        assert_eq!(&bytes[at..at + 16], &FOOTER_CODE);
        assert_eq!(&bytes[bytes.len() - 16..], &FOOTER_MAGIC);
        assert_eq!(u32_at(bytes, bytes.len() - 16 - 120 - 4), VERSION);
        assert_eq!((bytes.len() - 16 - 120 - 4) % 16, 0);
        Read {
            name: String::new(),
            props: Vec::new(),
            children,
        }
    }

    fn id(r: &Read) -> i64 {
        match r.props[0] {
            Prop::I64(v) => v,
            _ => panic!("{} has no id", r.name),
        }
    }

    fn text(p: &Prop) -> &str {
        match p {
            Prop::Str(s) => s,
            _ => panic!("not a string: {p:?}"),
        }
    }

    fn check(r: &rig::Rig) {
        let doc = parse(&write(r));
        let objects = doc.child("Objects");
        let links: Vec<(i64, i64)> = doc
            .child("Connections")
            .all("C")
            .map(|c| match (&c.props[1], &c.props[2]) {
                (Prop::I64(a), Prop::I64(b)) => (*a, *b),
                _ => panic!(),
            })
            .collect();
        let parent_of = |child: i64, among: &[i64]| {
            links
                .iter()
                .find(|(c, p)| *c == child && (among.contains(p) || *p == 0))
                .map(|l| l.1)
        };

        // Bones: names, hierarchy and rest positions.
        let limbs: Vec<&Read> = objects
            .all("Model")
            .filter(|m| text(&m.props[2]) == "LimbNode")
            .collect();
        assert_eq!(limbs.len(), r.skeleton.bones.len());
        let limb_ids: Vec<i64> = limbs.iter().map(|m| id(m)).collect();
        let names: Vec<&str> = limbs
            .iter()
            .map(|m| text(&m.props[1]).split('\0').next().unwrap())
            .collect();
        for (i, bone) in r.skeleton.bones.iter().enumerate() {
            assert_eq!(names[i], bone.name);
            let parent = parent_of(limb_ids[i], &limb_ids).unwrap();
            let expected = bone.parent.map_or(0, |p| limb_ids[p]);
            assert_eq!(parent, expected, "{}", bone.name);
            let t = limbs[i]
                .child("Properties70")
                .all("P")
                .find(|p| text(&p.props[0]) == "Lcl Translation")
                .unwrap();
            let got: Vec<f64> = t.props[4..7]
                .iter()
                .map(|p| match p {
                    Prop::F64(v) => *v,
                    _ => panic!(),
                })
                .collect();
            let want = r.skeleton.local_offset(i) * CM;
            for (g, w) in got.iter().zip(want.to_array()) {
                assert!((g - w as f64).abs() < 1e-4, "{}", bone.name);
            }
        }

        // Skin: every stored influence, in clusters linked to their bones.
        let geometry = objects.child("Geometry");
        let Prop::F64s(verts) = &geometry.child("Vertices").props[0] else {
            panic!()
        };
        assert_eq!(verts.len(), r.mesh.vertex_count() * 3);
        let Prop::I32s(polys) = &geometry.child("PolygonVertexIndex").props[0] else {
            panic!()
        };
        assert_eq!(polys.len(), r.mesh.indices.len());
        assert_eq!(polys.iter().filter(|&&i| i < 0).count() * 3, polys.len());
        let mut weights = vec![Vec::new(); r.mesh.vertex_count()];
        let mut clusters = 0;
        for d in objects.all("Deformer") {
            if text(&d.props[2]) != "Cluster" {
                continue;
            }
            clusters += 1;
            let bone = links
                .iter()
                .find(|(c, p)| *p == id(d) && limb_ids.contains(c))
                .map(|l| limb_ids.iter().position(|b| *b == l.0).unwrap())
                .expect("cluster has a bone");
            let (Prop::I32s(ix), Prop::F64s(ws)) =
                (&d.child("Indexes").props[0], &d.child("Weights").props[0])
            else {
                panic!()
            };
            assert_eq!(ix.len(), ws.len());
            for (&v, &w) in ix.iter().zip(ws) {
                weights[v as usize].push((bone, w as f32));
            }
            let Prop::F64s(link) = &d.child("TransformLink").props[0] else {
                panic!()
            };
            let head = r.skeleton.bones[bone].head * CM;
            assert!((link[12] - head.x as f64).abs() < 1e-4);
            assert!((link[13] - head.y as f64).abs() < 1e-4);
        }
        assert!(clusters > 0);
        for (v, got) in weights.iter_mut().enumerate() {
            let mut want: Vec<(usize, f32)> = r.skin.influences(v).collect();
            want.sort_by_key(|x| x.0);
            got.sort_by_key(|x| x.0);
            assert_eq!(*got, want, "vertex {v}");
        }

        // Bind pose: the mesh and every bone.
        let pose = objects.child("Pose");
        assert_eq!(pose.all("PoseNode").count(), r.skeleton.bones.len() + 1);
    }

    #[test]
    fn fbx_carries_skeleton_skin_and_bind_pose() {
        check(&rig::auto_rig(demo_humanoid(), "demo", Default::default()).unwrap());
    }

    #[test]
    fn fbx_keeps_every_influence() {
        let mesh = demo_humanoid();
        let (mut guides, _) = rig::detect_guides(&mesh);
        guides.skin.max_influences = 8;
        let r = rig::build_rig(mesh, "demo", guides, Default::default()).unwrap();
        assert!(r.skin.width() > 4);
        check(&r);
    }

    #[test]
    fn fbx_is_reproducible() {
        let r = rig::auto_rig(demo_humanoid(), "demo", Default::default()).unwrap();
        assert_eq!(write(&r), write(&r));
    }
}
