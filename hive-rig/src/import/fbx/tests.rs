use super::*;
use crate::{demo, rig};

/// A rig written by hive-rig's FBX writer reads back as the same mesh,
/// pieces, materials, shapes, skeleton and skin.
fn round_trip(r: &rig::Rig) -> FbxScene {
    let back = read(&crate::export::fbx::write(r), None).unwrap();
    let (m, want) = (&back.mesh, &r.mesh);
    back.mesh.validate().unwrap();
    assert_eq!(m.vertex_count(), want.vertex_count());
    for v in 0..m.vertex_count() {
        assert!(
            m.positions[v].distance(want.positions[v]) < 1e-5,
            "vertex {v}"
        );
        assert!(m.normals[v].dot(want.normals[v]) > 0.9999, "normal {v}");
    }
    assert_eq!(m.indices, want.indices);
    assert_eq!(m.uvs.len(), want.uvs.len());
    for (a, b) in m.uvs.iter().zip(&want.uvs) {
        for (x, y) in a.iter().zip(b) {
            assert!(x.distance(*y) < 1e-6);
        }
    }
    assert_eq!(m.colors.len(), want.colors.len());
    assert_eq!(m.objects.len(), want.object_groups(&r.name).len());
    assert_eq!(m.pieces().len(), want.pieces().len());
    for (a, b) in m.pieces().iter().zip(want.pieces().iter()) {
        assert_eq!(
            (a.first_vertex, a.vertex_count),
            (b.first_vertex, b.vertex_count)
        );
        let name = |mesh: &Mesh, p: &Part| p.material.map(|i| mesh.materials[i].name.clone());
        if b.material.is_some() {
            assert_eq!(name(m, a), name(want, b));
        }
    }
    assert_eq!(m.shapes.len(), want.shapes.len());
    for (a, b) in m.shapes.iter().zip(&want.shapes) {
        assert_eq!(a.name, b.name);
        assert_eq!(a.vertices, b.vertices);
        for (x, y) in a.offsets.iter().zip(&b.offsets) {
            assert!(x.distance(*y) < 1e-6);
        }
    }
    // The skeleton: names, parents and heads.
    let sk = &r.skeleton;
    assert_eq!(back.joints.len(), sk.bones.len());
    for (j, b) in back.joints.iter().zip(&sk.bones) {
        assert_eq!(j.name, b.name);
        assert!(j.position.distance(b.head) < 1e-5, "{}", b.name);
        let parent = j.parent.map(|p| back.joints[p].name.as_str());
        assert_eq!(parent, b.parent.map(|p| sk.bones[p].name.as_str()));
    }
    // Every influence of the skin.
    for v in 0..m.vertex_count() {
        let mut got = back.skin[v].clone();
        let mut want: Vec<(usize, f32)> = r.skin.influences(v).collect();
        got.sort_by_key(|x| x.0);
        want.sort_by_key(|x| x.0);
        assert_eq!(got.len(), want.len(), "vertex {v}");
        for (g, w) in got.iter().zip(&want) {
            assert_eq!(g.0, w.0);
            assert!((g.1 - w.1).abs() < 1e-5);
        }
    }
    back
}

#[test]
fn reads_back_the_fbx_writer() {
    let r = rig::auto_rig(demo::demo_humanoid(), "demo", Default::default()).unwrap();
    round_trip(&r);
    let mesh = demo::demo_humanoid();
    let (mut guides, _) = rig::detect_guides(&mesh);
    guides.skin.max_influences = 8;
    let r8 = rig::build_rig(mesh, "demo", guides, Default::default()).unwrap();
    assert!(r8.skin.width() > 4);
    round_trip(&r8);
}

#[test]
fn reads_back_objects_uvs_colours_materials_textures_and_shapes() {
    let mut r = rig::auto_rig(demo::dressed_humanoid(), "dressed", Default::default()).unwrap();
    // An in-between goes out as a channel of its own and folds back.
    let name = r.mesh.shapes[0].name.clone();
    crate::shapes::add_inbetween(&mut r, &name, 0.4).unwrap();
    for o in &mut r.mesh.shapes[0].inbetweens[0].offsets {
        *o *= 2.0;
    }
    r.mesh.shapes[0].weight = 0.2;
    let back = round_trip(&r);
    let m = &back.mesh;
    assert_eq!(m.objects.len(), 2);
    assert_eq!(m.uvs.len(), 2);
    assert!(!m.colors.is_empty());
    for (a, b) in m.colors.iter().zip(&r.mesh.colors) {
        assert!(a.distance(*b) < 1e-6);
    }
    assert_eq!(m.shapes.len(), 1);
    let (got, want) = (&m.shapes[0], &r.mesh.shapes[0]);
    assert_eq!(got.inbetweens.len(), 1);
    assert!((got.inbetweens[0].weight - 0.4).abs() < 1e-4);
    assert!((got.weight - 0.2).abs() < 1e-4, "{}", got.weight);
    for (x, y) in got.inbetweens[0]
        .offsets
        .iter()
        .zip(&want.inbetweens[0].offsets)
    {
        assert!(x.distance(*y) < 1e-6);
    }
    let textured = m
        .materials
        .iter()
        .find_map(|x| x.base_color_texture)
        .expect("a textured material");
    assert_eq!(m.images[textured.image].uri, "skin.png");
    for (a, b) in m.materials.iter().zip(&r.mesh.materials) {
        assert_eq!(a.name, b.name);
        for k in 0..4 {
            assert!((a.base_color[k] - b.base_color[k]).abs() < 1e-6);
        }
        assert!((a.roughness - b.roughness).abs() < 1e-5);
    }
}

fn walk(n: &mut Node, f: &mut dyn FnMut(&mut Node)) {
    f(n);
    for c in &mut n.children {
        walk(c, f);
    }
}

fn is_p(n: &Node, name: &str) -> bool {
    n.name == "P" && n.props.first().and_then(Value::as_str) == Some(name)
}

fn model<'a>(top: &'a mut [Node], name: &str) -> &'a mut Node {
    let objects = top.iter_mut().find(|n| n.name == "Objects").unwrap();
    let m = objects
        .children
        .iter_mut()
        .find(|n| n.name == "Model" && n.object_name() == name)
        .unwrap();
    m.children
        .iter_mut()
        .find(|c| c.name == "Properties70")
        .unwrap()
}

fn entry(name: &str, v: [f64; 3]) -> Node {
    Node {
        name: "P".into(),
        props: [
            Value::Str(name.into()),
            Value::Str("Vector3D".into()),
            Value::Str("Vector".into()),
            Value::Str("".into()),
        ]
        .into_iter()
        .chain(v.map(Value::F64))
        .collect(),
        children: Vec::new(),
    }
}

/// Compressed arrays, 64-bit records (7.5), a Z-up file in inches, and a model
/// turned by a rotation its pre-rotation undoes (about a pivot) all place the
/// mesh and bones where the format says.
#[test]
fn reads_compressed_wide_records_axes_units_and_transforms() {
    let r = rig::auto_rig(demo::demo_humanoid(), "demo", Default::default()).unwrap();
    let (_, mut top) = parse_records(&crate::export::fbx::write(&r)).unwrap();
    // Z up, front -Y, 2.54 cm per unit: the file's points are the rig's
    // (in centimetres) turned and scaled the other way.
    let turn = DMat4::from_cols(
        DVec3::X.extend(0.0),
        DVec3::Z.extend(0.0),
        DVec3::NEG_Y.extend(0.0),
        glam::DVec4::W,
    );
    let into = turn * DMat4::from_scale(DVec3::splat(1.0 / 2.54));
    for n in &mut top {
        walk(n, &mut |n| {
            for (name, v) in [
                ("UpAxis", Value::I32(2)),
                ("FrontAxis", Value::I32(1)),
                ("FrontAxisSign", Value::I32(-1)),
                ("UnitScaleFactor", Value::F64(2.54)),
            ] {
                if is_p(n, name) {
                    n.props[4] = v;
                }
            }
            if is_p(n, "Lcl Translation") {
                let v = DVec3::new(
                    n.props[4].as_f64().unwrap(),
                    n.props[5].as_f64().unwrap(),
                    n.props[6].as_f64().unwrap(),
                );
                let q = into.transform_vector3(v);
                n.props[4..7].clone_from_slice(&q.to_array().map(Value::F64));
            }
            let Some(Value::F64s(v)) = n.props.first_mut() else {
                return;
            };
            match n.name.as_str() {
                "Vertices" if v.len() % 3 == 0 => {
                    for c in v.as_chunks_mut::<3>().0 {
                        *c = into.transform_point3(DVec3::from_array(*c)).to_array();
                    }
                }
                "Normals" => {
                    for c in v.as_chunks_mut::<3>().0 {
                        *c = turn.transform_vector3(DVec3::from_array(*c)).to_array();
                    }
                }
                "Transform" | "TransformLink" | "Matrix" => {
                    let m = DMat4::from_cols_array(&v.clone().try_into().unwrap());
                    *v = (into * m * into.inverse()).to_cols_array().to_vec();
                }
                _ => {}
            }
        });
    }
    let p70 = model(&mut top, "pelvis");
    p70.children
        .retain(|c| c.props[0].as_str() != Some("Lcl Rotation"));
    p70.children.push(entry("Lcl Rotation", [0.0, 0.0, 30.0]));
    p70.children.push(entry("PreRotation", [0.0, 0.0, -30.0]));
    p70.children.push(entry("RotationPivot", [1.0, 2.0, 3.0]));
    let back = read(&encode(&top, 7500, true), None).unwrap();
    for (a, b) in back.mesh.positions.iter().zip(&r.mesh.positions) {
        assert!(a.distance(*b) < 1e-4, "{a} {b}");
    }
    for (a, b) in back.mesh.normals.iter().zip(&r.mesh.normals) {
        assert!(a.dot(*b) > 0.999);
    }
    for (j, b) in back.joints.iter().zip(&r.skeleton.bones) {
        assert!(j.position.distance(b.head) < 1e-4, "{}", b.name);
    }
    // Without the pre-rotation the pelvis turns and its legs move with it, and
    // so does their skin.
    let p70 = model(&mut top, "pelvis");
    p70.children
        .retain(|c| c.props[0].as_str() != Some("PreRotation"));
    let turned = read(&encode(&top, 7400, false), None).unwrap();
    let i = r.skeleton.index_of("thigh_l").unwrap();
    assert!(turned.joints[i].position.distance(r.skeleton.bones[i].head) > 0.01);
    let moved = (0..r.mesh.vertex_count())
        .filter(|&v| turned.mesh.positions[v].distance(r.mesh.positions[v]) > 0.01)
        .count();
    assert!(moved > r.mesh.vertex_count() / 10, "{moved}");
}

#[test]
fn refuses_damaged_and_ascii_files() {
    let r = rig::auto_rig(demo::demo_humanoid(), "demo", Default::default()).unwrap();
    let bytes = crate::export::fbx::write(&r);
    for cut in [10, 27, 100, bytes.len() / 2] {
        assert!(read(&bytes[..cut], None).is_err(), "cut at {cut}");
    }
    let mut bad = bytes.clone();
    bad[27..31].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(read(&bad, None).is_err());
    let old = b"; FBX 6.1.0 project file\nFBXHeaderExtension:  {\n\tFBXVersion: 6100\n}\n";
    let e = read(old, None).unwrap_err().to_string();
    assert!(e.contains("ASCII FBX version 6100"), "{e}");
    let empty = b"; FBX 7.4.0 project file\nFBXHeaderExtension:  {\n}\n";
    let e = read(empty, None).unwrap_err().to_string();
    assert!(e.contains("no objects"), "{e}");
    assert!(read(b"hello", None).is_err());
    // A changed byte anywhere either reads or fails cleanly, never panics.
    for at in (27..bytes.len()).step_by(331) {
        let mut b = bytes.clone();
        b[at] ^= 0xa5;
        let _ = read(&b, None);
    }
}

/// A Blender-style cluster stores its mesh transform relative to the bone;
/// the format's own reading stores the mesh's: both place the skin the same.
#[test]
fn clusters_with_either_mesh_transform() {
    let r = rig::auto_rig(demo::demo_humanoid(), "demo", Default::default()).unwrap();
    let (_, mut top) = parse_records(&crate::export::fbx::write(&r)).unwrap();
    // hive-rig writes Transform = TransformLink⁻¹; make it the mesh's (identity).
    for n in &mut top {
        walk(n, &mut |n| {
            if n.name == "Transform"
                && let Some(Value::F64s(v)) = n.props.first_mut()
            {
                *v = DMat4::IDENTITY.to_cols_array().to_vec();
            }
        });
    }
    let back = read(&encode(&top, 7400, false), None).unwrap();
    for (a, b) in back.mesh.positions.iter().zip(&r.mesh.positions) {
        assert!(a.distance(*b) < 1e-5, "{a} {b}");
    }
}

/// Blender's leaf bones (`head_end` under `head`) are left out; other end
/// joints stay.
#[test]
fn leaves_out_exporter_leaf_bones() {
    let r = rig::auto_rig(demo::demo_humanoid(), "demo", Default::default()).unwrap();
    let (_, mut top) = parse_records(&crate::export::fbx::write(&r)).unwrap();
    let objects = top.iter_mut().find(|n| n.name == "Objects").unwrap();
    let head = objects
        .children
        .iter()
        .find(|n| n.name == "Model" && n.object_name() == "head")
        .unwrap()
        .id()
        .unwrap();
    let limb = |id: i64, name: &str| Node {
        name: "Model".into(),
        props: vec![
            Value::I64(id),
            Value::Str(format!("{name}\0\u{1}Model")),
            Value::Str("LimbNode".into()),
        ],
        children: vec![],
    };
    objects.children.push(limb(900_001, "head_end"));
    objects.children.push(limb(900_002, "head_tip"));
    let links = top.iter_mut().find(|n| n.name == "Connections").unwrap();
    for id in [900_001i64, 900_002] {
        links.children.push(Node {
            name: "C".into(),
            props: vec![Value::Str("OO".into()), Value::I64(id), Value::I64(head)],
            children: vec![],
        });
    }
    let back = read(&encode(&top, 7400, false), None).unwrap();
    let names: Vec<&str> = back.joints.iter().map(|j| j.name.as_str()).collect();
    assert_eq!(names.len(), r.skeleton.bones.len() + 1);
    assert!(names.contains(&"head_tip") && !names.contains(&"head_end"));
}

/// Write records back out (in the 7.4 or 7.5 layout), compressing every array.
fn encode(top: &[Node], version: u32, wide: bool) -> Vec<u8> {
    fn size(out: &mut Vec<u8>, v: usize, wide: bool) {
        if wide {
            out.extend_from_slice(&(v as u64).to_le_bytes());
        } else {
            out.extend_from_slice(&(v as u32).to_le_bytes());
        }
    }
    fn array(out: &mut Vec<u8>, code: u8, n: usize, raw: Vec<u8>) {
        out.push(code);
        out.extend_from_slice(&(n as u32).to_le_bytes());
        out.extend_from_slice(&1u32.to_le_bytes());
        let z = miniz_oxide::deflate::compress_to_vec_zlib(&raw, 6);
        out.extend_from_slice(&(z.len() as u32).to_le_bytes());
        out.extend_from_slice(&z);
    }
    fn scalar(out: &mut Vec<u8>, code: u8, bytes: &[u8]) {
        out.push(code);
        out.extend_from_slice(bytes);
    }
    fn value(out: &mut Vec<u8>, v: &Value) {
        match v {
            Value::Bool(b) => scalar(out, b'C', &[u8::from(*b)]),
            Value::I16(x) => scalar(out, b'Y', &x.to_le_bytes()),
            Value::I32(x) => scalar(out, b'I', &x.to_le_bytes()),
            Value::I64(x) => scalar(out, b'L', &x.to_le_bytes()),
            Value::F32(x) => scalar(out, b'F', &x.to_le_bytes()),
            Value::F64(x) => scalar(out, b'D', &x.to_le_bytes()),
            Value::Str(s) => {
                scalar(out, b'S', &(s.len() as u32).to_le_bytes());
                out.extend_from_slice(s.as_bytes());
            }
            Value::Raw(s) => {
                scalar(out, b'R', &(s.len() as u32).to_le_bytes());
                out.extend_from_slice(s);
            }
            Value::Bools(a) => array(out, b'b', a.len(), a.iter().map(|&x| u8::from(x)).collect()),
            Value::I32s(a) => array(
                out,
                b'i',
                a.len(),
                a.iter().flat_map(|x| x.to_le_bytes()).collect(),
            ),
            Value::I64s(a) => array(
                out,
                b'l',
                a.len(),
                a.iter().flat_map(|x| x.to_le_bytes()).collect(),
            ),
            Value::F32s(a) => array(
                out,
                b'f',
                a.len(),
                a.iter().flat_map(|x| x.to_le_bytes()).collect(),
            ),
            Value::F64s(a) => array(
                out,
                b'd',
                a.len(),
                a.iter().flat_map(|x| x.to_le_bytes()).collect(),
            ),
        }
    }
    fn node(out: &mut Vec<u8>, n: &Node, wide: bool) {
        let w = if wide { 8 } else { 4 };
        let start = out.len();
        out.extend(std::iter::repeat_n(0u8, 3 * w));
        out.push(n.name.len() as u8);
        out.extend_from_slice(n.name.as_bytes());
        let p0 = out.len();
        for p in &n.props {
            value(out, p);
        }
        let props_len = out.len() - p0;
        for c in &n.children {
            node(out, c, wide);
        }
        if !n.children.is_empty() || n.props.is_empty() {
            out.extend(std::iter::repeat_n(0u8, 3 * w + 1));
        }
        let mut head = Vec::new();
        size(&mut head, out.len(), wide);
        size(&mut head, n.props.len(), wide);
        size(&mut head, props_len, wide);
        out[start..start + 3 * w].copy_from_slice(&head);
    }
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&[0x1a, 0]);
    out.extend_from_slice(&version.to_le_bytes());
    for n in top {
        node(&mut out, n, wide);
    }
    let w = if wide { 8 } else { 4 };
    out.extend(std::iter::repeat_n(0u8, 3 * w + 1));
    out
}

/// The record tree written as ASCII FBX, the way exporters write it: objects
/// named `Class::Name`, arrays as `*count { a: ... }` broken over lines, bare
/// `T`/`F`, comments, and an embedded file as base64 text after an empty value.
pub(crate) fn to_ascii(top: &[Node], version: u32) -> String {
    fn text(s: &str) -> String {
        let s = match s.split_once("\0\u{1}") {
            Some((name, class)) => format!("{class}::{name}"),
            None => s.to_string(),
        };
        format!("\"{}\"", s.replace('"', "&quot;"))
    }
    fn b64(b: &[u8]) -> String {
        const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for c in b.chunks(3) {
            let n = c
                .iter()
                .enumerate()
                .fold(0u32, |n, (i, &x)| n | (u32::from(x) << (16 - 8 * i)));
            for i in 0..4 {
                out.push(if i <= c.len() {
                    A[((n >> (18 - 6 * i)) & 63) as usize] as char
                } else {
                    '='
                });
            }
        }
        out
    }
    fn array(out: &mut String, pad: &str, items: Vec<String>) {
        out.push_str(&format!("*{} {{\n{pad}\ta: ", items.len()));
        for (i, chunk) in items.chunks(16).enumerate() {
            if i > 0 {
                out.push_str(",\n");
            }
            out.push_str(&chunk.join(","));
        }
        out.push_str(&format!("\n{pad}}}"));
    }
    fn floats(v: impl Iterator<Item = f64>) -> Vec<String> {
        v.map(|x| format!("{x:?}")).collect()
    }
    fn node(out: &mut String, n: &Node, depth: usize) {
        let pad = "\t".repeat(depth);
        out.push_str(&format!("{pad}{}: ", n.name));
        for (i, p) in n.props.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            match p {
                Value::Bool(b) => out.push(if *b { 'T' } else { 'F' }),
                Value::I16(x) => out.push_str(&x.to_string()),
                Value::I32(x) => out.push_str(&x.to_string()),
                Value::I64(x) => out.push_str(&x.to_string()),
                Value::F32(x) => out.push_str(&format!("{:?}", f64::from(*x))),
                Value::F64(x) => out.push_str(&format!("{x:?}")),
                Value::Str(s) => out.push_str(&text(s)),
                Value::Raw(b) => {
                    let s = b64(b);
                    let (a, rest) = s.split_at(s.len().min(8));
                    out.push_str(&format!(",\n{pad}\t\"{a}\", \"{rest}\""));
                }
                Value::Bools(v) => array(
                    out,
                    &pad,
                    v.iter().map(|&b| u8::from(b).to_string()).collect(),
                ),
                Value::I32s(v) => array(out, &pad, v.iter().map(i32::to_string).collect()),
                Value::I64s(v) => array(out, &pad, v.iter().map(i64::to_string).collect()),
                Value::F32s(v) => array(out, &pad, floats(v.iter().map(|&x| x.into()))),
                Value::F64s(v) => array(out, &pad, floats(v.iter().copied())),
            }
        }
        if !n.children.is_empty() {
            out.push_str(" {\n");
            for c in &n.children {
                node(out, c, depth + 1);
            }
            out.push_str(&format!("{pad}}}"));
        }
        out.push('\n');
    }
    let mut out = format!(
        "; FBX {}.{}.0 project file\n; ------------------------------------------\n\n",
        version / 1000,
        version / 100 % 10
    );
    for n in top {
        out.push_str(&format!(
            "\n; {} section\n;-------------------------------\n\n",
            n.name
        ));
        node(&mut out, n, 0);
    }
    out
}

/// An ASCII FBX reads as the same scene as the binary FBX it was written from:
/// mesh, pieces, UV sets, colours, materials, textures (an embedded one too),
/// shapes with in-betweens, skeleton and skin.
#[test]
fn reads_ascii_like_binary() {
    let mut dressed =
        rig::auto_rig(demo::dressed_humanoid(), "dressed", Default::default()).unwrap();
    let name = dressed.mesh.shapes[0].name.clone();
    crate::shapes::add_inbetween(&mut dressed, &name, 0.4).unwrap();
    let plain = rig::auto_rig(demo::demo_humanoid(), "demo", Default::default()).unwrap();
    for r in [&plain, &dressed] {
        let (version, mut top) = parse_records(&crate::export::fbx::write(r)).unwrap();
        let mut clips = 0;
        for n in &mut top {
            walk(n, &mut |n| {
                if n.name == "Video" {
                    clips += 1;
                    n.children.push(Node {
                        name: "Content".into(),
                        props: vec![Value::Raw((0..=255u8).chain(*b"hive").collect())],
                        children: vec![],
                    });
                }
            });
        }
        let binary = read(&encode(&top, version, false), None).unwrap();
        let text = to_ascii(&top, version);
        assert!(text.contains("\"Model::") && text.contains("a: "));
        let (v, back) = parse_records(text.as_bytes()).unwrap();
        assert_eq!(v, version);
        assert_eq!(back.len(), top.len());
        let ascii = read(text.as_bytes(), None).unwrap();
        assert_eq!(format!("{ascii:?}"), format!("{binary:?}"), "{}", r.name);
        assert!(!ascii.joints.is_empty() && !ascii.skin.is_empty());
        if r.name == "dressed" {
            assert!(clips > 0, "the dressed rig has a texture clip");
            assert_eq!(ascii.mesh.shapes.len(), 1);
            assert_eq!(ascii.mesh.shapes[0].inbetweens.len(), 1);
            assert_eq!(ascii.mesh.uvs.len(), 2);
            assert!(!ascii.mesh.images.is_empty());
        }
    }
}

/// An ASCII .fbx on disk goes through the same import as a binary one.
#[test]
fn imports_an_ascii_file() {
    let r = rig::auto_rig(demo::demo_humanoid(), "demo", Default::default()).unwrap();
    let bin = crate::export::fbx::write(&r);
    let (version, top) = parse_records(&bin).unwrap();
    let dir = std::env::temp_dir().join(format!("hive-rig-ascii-fbx-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (a, b) = (dir.join("ascii.fbx"), dir.join("binary.fbx"));
    std::fs::write(&a, to_ascii(&top, version)).unwrap();
    std::fs::write(&b, &bin).unwrap();
    let (ma, ja) = crate::import::load_mesh_and_joints(&a).unwrap();
    let (mb, jb) = crate::import::load_mesh_and_joints(&b).unwrap();
    assert_eq!(ma.positions, mb.positions);
    assert_eq!(format!("{ja:?}"), format!("{jb:?}"));
    std::fs::remove_dir_all(&dir).ok();
}
