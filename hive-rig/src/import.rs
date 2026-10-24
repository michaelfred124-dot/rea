//! Mesh importers: Wavefront OBJ (with its .mtl), glTF 2.0 (.gltf / .glb) and
//! binary FBX 7.x ([`fbx`]).
//!
//! All keep what the character looks like: every object and material piece,
//! normals, UV sets, tangents and vertex colours, materials with their texture
//! images, and (glTF, FBX) morph targets; glTF and FBX also bring their skeleton. Images are not decoded: embedded ones are
//! held as their encoded bytes and referenced ones by path, until an export
//! writes them next to the rig.

use std::path::Path;

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result, bail};
use glam::{Mat3, Mat4, Vec2, Vec3, Vec4};

use crate::joints::Joint;
use crate::mesh::{AlphaMode, Image, Material, Mesh, Part, TextureRef, smooth_normals};
use crate::orient::{self, Choice, Detection};
use crate::shapes::Shape;

pub mod fbx;

/// A character as read from its file and turned into hive-rig's frame (Y up,
/// facing +Z, its left on +X).
#[derive(Debug, Clone)]
pub struct Character {
    pub mesh: Mesh,
    /// The skeleton it came with (empty for none), turned the same way.
    pub joints: Vec<Joint>,
    /// How it stood in the file, the body kind, and why.
    pub detection: Detection,
}

/// Load a character: its mesh and skeleton, its orientation (detected, or as
/// `choice` says) and its body kind, and turn it into hive-rig's frame.
pub fn load_character(path: &Path, choice: Choice) -> Result<Character> {
    let (mesh, joints) = load_mesh_and_joints(path)?;
    Ok(orient_character(mesh, joints, choice))
}

/// A file's mesh ([`load_mesh`]) and skeleton ([`load_joints`]), reading the
/// file once.
pub fn load_mesh_and_joints(path: &Path) -> Result<(Mesh, Vec<Joint>)> {
    if extension(path) == "fbx" {
        let scene = load_fbx(path)?;
        return Ok((scene.mesh, scene.joints));
    }
    Ok((load_mesh(path)?, load_joints(path)?))
}

fn extension(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default()
}

/// Read a binary FBX and check it has a mesh.
fn load_fbx(path: &Path) -> Result<fbx::FbxScene> {
    let scene = fbx::load(path)?;
    if scene.mesh.positions.is_empty() {
        bail!("{} contains no meshes", path.display());
    }
    scene.mesh.validate()?;
    Ok(scene)
}

/// Detect (or take from `choice`) how a mesh and skeleton in a file's frame
/// stand, and turn them into hive-rig's frame.
pub fn orient_character(mut mesh: Mesh, mut joints: Vec<Joint>, choice: Choice) -> Character {
    let detection = orient::detect(&mesh, &joints, choice);
    let o = detection.orientation;
    if !o.is_standard() {
        orient::turn_mesh(&mut mesh, o.to_standard());
        orient::turn_joints(&mut joints, o.to_standard());
    }
    Character {
        mesh,
        joints,
        detection,
    }
}

/// Load a mesh, picking the importer from the file extension.
pub fn load_mesh(path: &Path) -> Result<Mesh> {
    let mesh = match extension(path).as_str() {
        "obj" => {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("reading {}", path.display()))?;
            parse_obj_with(&text, path.parent())?
        }
        "gltf" | "glb" => load_gltf(path)?,
        "fbx" => load_fbx(path)?.mesh,
        other => bail!("unsupported mesh format '.{other}' (use .obj, .gltf, .glb or .fbx)"),
    };
    mesh.validate()?;
    Ok(mesh)
}

/// Parse OBJ text with no material library. Polygons are fan-triangulated.
pub fn parse_obj(text: &str) -> Result<Mesh> {
    parse_obj_with(text, None)
}

/// One OBJ material from a .mtl file.
#[derive(Default)]
struct ObjMaterial {
    material: Material,
    maps: Vec<(&'static str, String, f32)>,
}

/// Read the materials of a .mtl file (Kd, d, Ke, Pm, Pr, map_Kd, map_Bump/norm,
/// map_Ke, map_d is folded into the base colour's alpha).
fn parse_mtl(text: &str) -> Vec<(String, ObjMaterial)> {
    let mut out: Vec<(String, ObjMaterial)> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let mut parts = line.split_whitespace();
        let Some(key) = parts.next() else { continue };
        let rest: Vec<&str> = parts.collect();
        let nums = |n: usize| -> Option<Vec<f32>> {
            let v: Vec<f32> = rest.iter().take(n).filter_map(|x| x.parse().ok()).collect();
            (v.len() == n).then_some(v)
        };
        if key == "newmtl" {
            let name = rest.join(" ");
            out.push((
                name.clone(),
                ObjMaterial {
                    material: Material {
                        name,
                        roughness: 0.8,
                        ..Default::default()
                    },
                    maps: Vec::new(),
                },
            ));
            continue;
        }
        let Some((_, m)) = out.last_mut() else {
            continue;
        };
        // A map's file name is the last word; options such as `-bm 1` come first.
        let map = || rest.last().map(|s| s.to_string());
        let bump_scale = rest
            .iter()
            .position(|w| *w == "-bm")
            .and_then(|i| rest.get(i + 1)?.parse().ok())
            .unwrap_or(1.0);
        match key {
            "Kd" => {
                if let Some(v) = nums(3) {
                    m.material.base_color[..3].copy_from_slice(&v);
                }
            }
            "d" => {
                if let Some(v) = nums(1) {
                    m.material.base_color[3] = v[0];
                    if v[0] < 1.0 {
                        m.material.alpha_mode = AlphaMode::Blend;
                    }
                }
            }
            "Tr" => {
                if let Some(v) = nums(1) {
                    m.material.base_color[3] = 1.0 - v[0];
                    if v[0] > 0.0 {
                        m.material.alpha_mode = AlphaMode::Blend;
                    }
                }
            }
            "Ke" => {
                if let Some(v) = nums(3) {
                    m.material.emissive.copy_from_slice(&v);
                }
            }
            "Pm" => {
                if let Some(v) = nums(1) {
                    m.material.metallic = v[0];
                }
            }
            "Pr" => {
                if let Some(v) = nums(1) {
                    m.material.roughness = v[0];
                }
            }
            "map_Kd" => m.maps.extend(map().map(|f| ("base_color", f, 1.0))),
            "map_Bump" | "map_bump" | "bump" | "norm" | "map_Kn" => {
                m.maps.extend(map().map(|f| ("normal", f, bump_scale)))
            }
            "map_Ke" => m.maps.extend(map().map(|f| ("emissive", f, 1.0))),
            _ => {}
        }
    }
    out
}

/// Parse OBJ text. `dir` is the folder the .obj is in, where its `mtllib`
/// material libraries and their texture files are looked up. Each `o` (or `g`
/// when there are no `o` lines) becomes an object, and each run of faces with
/// one material a piece; a vertex used with different UVs or normals is split
/// into copies, as glTF stores them.
pub fn parse_obj_with(text: &str, dir: Option<&Path>) -> Result<Mesh> {
    let mut v: Vec<Vec3> = Vec::new();
    let mut vt: Vec<Vec2> = Vec::new();
    let mut vn: Vec<Vec3> = Vec::new();
    let mut vc: Vec<Option<Vec3>> = Vec::new();
    let mut libraries: Vec<(String, ObjMaterial)> = Vec::new();
    let uses_o = text.lines().any(|l| l.trim_start().starts_with("o "));

    // Faces grouped into pieces: (object name, material name, corner triples).
    struct Piece {
        object: String,
        material: Option<String>,
        corners: Vec<[i64; 3]>,
    }
    let mut pieces: Vec<Piece> = Vec::new();
    let mut object = String::from("mesh");
    let mut material: Option<String> = None;

    for (line_no, line) in text.lines().enumerate() {
        let mut parts = line.split_whitespace();
        let bad = || format!("bad line {}", line_no + 1);
        match parts.next() {
            Some("v") => {
                let coords: Vec<f32> = parts
                    .map(str::parse)
                    .collect::<Result<_, _>>()
                    .with_context(|| format!("bad vertex on line {}", line_no + 1))?;
                if coords.len() < 3 {
                    bail!("vertex on line {} needs 3 coordinates", line_no + 1);
                }
                v.push(Vec3::new(coords[0], coords[1], coords[2]));
                // `v x y z r g b`: a common extension for vertex colours.
                vc.push((coords.len() >= 6).then(|| Vec3::new(coords[3], coords[4], coords[5])));
            }
            Some("vt") => {
                let c: Vec<f32> = parts
                    .take(2)
                    .map(str::parse)
                    .collect::<Result<_, _>>()
                    .with_context(bad)?;
                // OBJ puts the origin at the bottom left; the mesh keeps glTF's top left.
                vt.push(Vec2::new(
                    c.first().copied().unwrap_or(0.0),
                    1.0 - c.get(1).copied().unwrap_or(0.0),
                ));
            }
            Some("vn") => {
                let c: Vec<f32> = parts
                    .take(3)
                    .map(str::parse)
                    .collect::<Result<_, _>>()
                    .with_context(bad)?;
                if c.len() != 3 {
                    bail!("normal on line {} needs 3 coordinates", line_no + 1);
                }
                vn.push(Vec3::new(c[0], c[1], c[2]));
            }
            Some("o") => object = parts.collect::<Vec<_>>().join(" "),
            Some("g") if !uses_o => object = parts.collect::<Vec<_>>().join(" "),
            Some("usemtl") => material = Some(parts.collect::<Vec<_>>().join(" ")),
            Some("mtllib") => {
                let name = parts.collect::<Vec<_>>().join(" ");
                if let Some(d) = dir {
                    match std::fs::read_to_string(d.join(&name)) {
                        Ok(t) => libraries.extend(parse_mtl(&t)),
                        Err(e) => eprintln!("warning: can't read material library {name}: {e}"),
                    }
                }
            }
            Some("f") => {
                let face: Vec<[i64; 3]> = parts
                    .map(|token| {
                        let mut out = [-1i64; 3];
                        for (k, field) in token.split('/').take(3).enumerate() {
                            if field.is_empty() {
                                continue;
                            }
                            let raw: i64 = field
                                .parse()
                                .with_context(|| format!("bad face on line {}", line_no + 1))?;
                            let len = [v.len(), vt.len(), vn.len()][k] as i64;
                            // OBJ indices are 1-based; negative values count from the end.
                            let idx = if raw < 0 { len + raw } else { raw - 1 };
                            if idx < 0 || idx >= len {
                                if k > 0 {
                                    // A missing UV or normal: leave it out.
                                    continue;
                                }
                                bail!("face index out of range on line {}", line_no + 1);
                            }
                            out[k] = idx;
                        }
                        if out[0] < 0 {
                            bail!("face without a vertex on line {}", line_no + 1);
                        }
                        Ok(out)
                    })
                    .collect::<Result<_>>()?;
                let same = pieces
                    .last()
                    .is_some_and(|p| p.object == object && p.material == material);
                if !same {
                    pieces.push(Piece {
                        object: object.clone(),
                        material: material.clone(),
                        corners: Vec::new(),
                    });
                }
                let piece = pieces.last_mut().unwrap();
                for i in 1..face.len().saturating_sub(1) {
                    piece
                        .corners
                        .extend_from_slice(&[face[0], face[i], face[i + 1]]);
                }
            }
            _ => {}
        }
    }

    // Keep each object's pieces together, in first-seen order.
    let mut object_order: Vec<String> = Vec::new();
    for p in &pieces {
        if !object_order.contains(&p.object) {
            object_order.push(p.object.clone());
        }
    }
    pieces.sort_by_key(|p| object_order.iter().position(|o| *o == p.object));

    let mut mesh = Mesh::default();
    let any_uv = !vt.is_empty();
    let any_color = vc.iter().any(Option::is_some);
    let mut material_index: HashMap<String, usize> = HashMap::new();
    let mut image_index: HashMap<String, usize> = HashMap::new();
    let mut normals_given: Vec<bool> = Vec::new();
    for piece in &pieces {
        let first_vertex = mesh.positions.len();
        let first_index = mesh.indices.len();
        let mut corner_vertex: HashMap<[i64; 3], u32> = HashMap::new();
        for c in &piece.corners {
            let next = mesh.positions.len() as u32;
            let idx = *corner_vertex.entry(*c).or_insert_with(|| {
                mesh.positions.push(v[c[0] as usize]);
                normals_given.push(c[2] >= 0);
                mesh.normals.push(if c[2] >= 0 {
                    vn[c[2] as usize]
                } else {
                    Vec3::ZERO
                });
                if any_uv {
                    if mesh.uvs.is_empty() {
                        mesh.uvs.push(Vec::new());
                    }
                    mesh.uvs[0].push(if c[1] >= 0 {
                        vt[c[1] as usize]
                    } else {
                        Vec2::ZERO
                    });
                }
                if any_color {
                    mesh.colors
                        .push(vc[c[0] as usize].map_or(Vec4::ONE, |rgb| rgb.extend(1.0)));
                }
                next
            });
            mesh.indices.push(idx);
        }
        let object = object_order
            .iter()
            .position(|o| *o == piece.object)
            .unwrap();
        let material = match &piece.material {
            None => None,
            Some(name) => Some(match material_index.get(name) {
                Some(&i) => i,
                None => {
                    let found = libraries.iter().find(|(n, _)| n == name);
                    let mut m = found.map_or_else(
                        || Material {
                            name: name.clone(),
                            ..Default::default()
                        },
                        |(_, m)| m.material.clone(),
                    );
                    for (kind, file, scale) in found.map(|f| f.1.maps.as_slice()).unwrap_or(&[]) {
                        let image = *image_index.entry(file.clone()).or_insert_with(|| {
                            let path = dir.map_or_else(|| file.into(), |d| d.join(file));
                            mesh.images.push(Image {
                                name: Path::new(file)
                                    .file_stem()
                                    .map(|s| s.to_string_lossy().into_owned())
                                    .unwrap_or_default(),
                                uri: file.replace('\\', "/"),
                                mime: String::new(),
                                data: None,
                                path: Some(path),
                            });
                            mesh.images.len() - 1
                        });
                        let t = Some(TextureRef {
                            image,
                            uv_set: 0,
                            scale: *scale,
                        });
                        match *kind {
                            "base_color" => m.base_color_texture = t,
                            "normal" => m.normal_texture = t,
                            _ => m.emissive_texture = t,
                        }
                    }
                    mesh.materials.push(m);
                    material_index.insert(name.clone(), mesh.materials.len() - 1);
                    mesh.materials.len() - 1
                }
            }),
        };
        mesh.parts.push(Part {
            object,
            material,
            first_vertex,
            vertex_count: mesh.positions.len() - first_vertex,
            first_index,
            index_count: mesh.indices.len() - first_index,
        });
    }
    // Smooth normals where the file gave none.
    if normals_given.iter().any(|g| !g) {
        let smooth = smooth_normals(&mesh.positions, &mesh.indices);
        for (k, n) in mesh.normals.iter_mut().enumerate() {
            if !normals_given[k] || n.length_squared() < 1e-12 {
                *n = smooth[k];
            } else {
                *n = n.normalize();
            }
        }
    }
    // A plain OBJ (one object, no materials) stays a plain mesh.
    if object_order.len() <= 1 && mesh.materials.is_empty() {
        mesh.parts.clear();
    } else {
        mesh.objects = object_order;
    }
    Ok(mesh)
}

/// Read a glTF file's document and buffers without decoding its images.
fn open_gltf(path: &Path) -> Result<(gltf::Document, Vec<gltf::buffer::Data>)> {
    let gltf::Gltf { document, blob } =
        gltf::Gltf::open(path).with_context(|| format!("reading {}", path.display()))?;
    let buffers = gltf::import_buffers(&document, path.parent(), blob)
        .with_context(|| format!("reading the buffers of {}", path.display()))?;
    Ok((document, buffers))
}

/// Decode standard base64 (the payload of a `data:` URI).
fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for c in text.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            b'=' | b'\n' | b'\r' | b' ' => continue,
            _ => return None,
        } as u32;
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

/// Undo `%xx` escapes in a URI path.
fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Some(v) = std::str::from_utf8(&b[i + 1..i + 3])
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn gltf_images(
    doc: &gltf::Document,
    buffers: &[gltf::buffer::Data],
    base: Option<&Path>,
) -> Vec<Image> {
    doc.images()
        .map(|img| {
            let name = img
                .name()
                .map(|n| {
                    Path::new(n)
                        .file_stem()
                        .map_or_else(|| n.to_string(), |s| s.to_string_lossy().into_owned())
                })
                .unwrap_or_else(|| format!("image_{}", img.index()));
            match img.source() {
                gltf::image::Source::View { view, mime_type } => {
                    let buf = &buffers[view.buffer().index()];
                    let bytes = &buf[view.offset()..view.offset() + view.length()];
                    Image {
                        name,
                        uri: String::new(),
                        mime: mime_type.to_string(),
                        data: Some(Arc::from(bytes)),
                        path: None,
                    }
                }
                gltf::image::Source::Uri { uri, mime_type } => {
                    if let Some(rest) = uri.strip_prefix("data:") {
                        let (head, payload) = rest.split_once(',').unwrap_or(("", ""));
                        let mime = head.split(';').next().unwrap_or("").to_string();
                        Image {
                            name,
                            uri: String::new(),
                            mime: mime_type.map_or(mime, str::to_string),
                            data: base64_decode(payload).map(Arc::from),
                            path: None,
                        }
                    } else {
                        let rel = percent_decode(uri);
                        Image {
                            name: Path::new(&rel)
                                .file_stem()
                                .map_or(name, |s| s.to_string_lossy().into_owned()),
                            uri: rel.clone(),
                            mime: mime_type.unwrap_or("").to_string(),
                            data: None,
                            path: Some(base.map_or_else(|| rel.clone().into(), |b| b.join(&rel))),
                        }
                    }
                }
            }
        })
        .collect()
}

fn texture_ref(info: Option<gltf::texture::Info>) -> Option<TextureRef> {
    info.map(|t| TextureRef {
        image: t.texture().source().index(),
        uv_set: t.tex_coord() as usize,
        scale: 1.0,
    })
}

fn gltf_material(m: &gltf::Material) -> Material {
    let pbr = m.pbr_metallic_roughness();
    Material {
        name: m.name().map_or_else(
            || format!("material_{}", m.index().unwrap_or(0)),
            str::to_string,
        ),
        base_color: pbr.base_color_factor(),
        metallic: pbr.metallic_factor(),
        roughness: pbr.roughness_factor(),
        emissive: m.emissive_factor(),
        alpha_mode: match m.alpha_mode() {
            gltf::material::AlphaMode::Opaque => AlphaMode::Opaque,
            gltf::material::AlphaMode::Mask => AlphaMode::Mask,
            gltf::material::AlphaMode::Blend => AlphaMode::Blend,
        },
        alpha_cutoff: m.alpha_cutoff().unwrap_or(0.5),
        double_sided: m.double_sided(),
        base_color_texture: texture_ref(pbr.base_color_texture()),
        metallic_roughness_texture: texture_ref(pbr.metallic_roughness_texture()),
        normal_texture: m.normal_texture().map(|t| TextureRef {
            image: t.texture().source().index(),
            uv_set: t.tex_coord() as usize,
            scale: t.scale(),
        }),
        occlusion_texture: m.occlusion_texture().map(|t| TextureRef {
            image: t.texture().source().index(),
            uv_set: t.tex_coord() as usize,
            scale: t.strength(),
        }),
        emissive_texture: texture_ref(m.emissive_texture()),
    }
}

/// Morph target names from a glTF mesh's `extras.targetNames`, the usual place.
fn target_names(mesh: &gltf::Mesh) -> Vec<String> {
    mesh.extras()
        .as_ref()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw.get()).ok())
        .and_then(|v| {
            v.get("targetNames")?.as_array().map(|a| {
                a.iter()
                    .map(|n| n.as_str().unwrap_or("").to_string())
                    .collect()
            })
        })
        .unwrap_or_default()
}

/// What the glTF walk gathers before it becomes a [`Mesh`].
#[derive(Default)]
struct Gathered {
    mesh: Mesh,
    /// Per vertex: whether the source gave its normal, and its tangent.
    has_normal: Vec<bool>,
    tangents: Vec<Option<Vec4>>,
    colors: Vec<Option<Vec4>>,
    uvs: Vec<Vec<Option<Vec2>>>,
    /// Morph targets merged by name across meshes.
    shapes: Vec<GatheredShape>,
}

/// A morph target as the glTF walk builds it up.
struct GatheredShape {
    name: String,
    /// Vertex to (position offset, normal offset), world space.
    offsets: HashMap<u32, (Vec3, Vec3)>,
    weight: f32,
    /// Some primitive gave normal offsets for it.
    has_normals: bool,
}

/// Load every triangle primitive in the default scene, baked into world space.
/// Each mesh node becomes an object and each primitive a piece with its
/// material; morph targets with the same name on several meshes become one shape.
pub fn load_gltf(path: &Path) -> Result<Mesh> {
    let (doc, buffers) = open_gltf(path)?;
    let scene = doc
        .default_scene()
        .or_else(|| doc.scenes().next())
        .context("glTF file has no scene")?;
    let mut g = Gathered::default();
    g.mesh.materials = doc.materials().map(|m| gltf_material(&m)).collect();
    g.mesh.images = gltf_images(&doc, &buffers, path.parent());
    let world = scene_world(&doc, &scene).0;
    for node in scene.nodes() {
        collect_node(&node, Mat4::IDENTITY, &buffers, &world, &mut g);
    }
    if g.mesh.positions.is_empty() {
        bail!("glTF file contains no triangle meshes");
    }
    let mut mesh = g.mesh;
    // Normals: the source's, or smooth ones for pieces that had none.
    if g.has_normal.iter().any(|h| !h) {
        let smooth = smooth_normals(&mesh.positions, &mesh.indices);
        for ((normal, smooth), given) in mesh.normals.iter_mut().zip(smooth).zip(&g.has_normal) {
            if !given {
                *normal = smooth;
            }
        }
    }
    mesh.uvs = g
        .uvs
        .into_iter()
        .map(|set| set.into_iter().map(|t| t.unwrap_or(Vec2::ZERO)).collect())
        .collect();
    if g.tangents.iter().all(Option::is_some) {
        mesh.tangents = g.tangents.into_iter().flatten().collect();
    }
    if g.colors.iter().any(Option::is_some) {
        mesh.colors = g
            .colors
            .into_iter()
            .map(|c| c.unwrap_or(Vec4::ONE))
            .collect();
    }
    mesh.shapes = g
        .shapes
        .into_iter()
        .map(|s| {
            let GatheredShape {
                name,
                offsets,
                weight,
                has_normals: normals,
            } = s;
            let mut keys: Vec<u32> = offsets.keys().copied().collect();
            keys.sort_unstable();
            Shape {
                name,
                offsets: keys.iter().map(|k| offsets[k].0).collect(),
                normal_offsets: if normals {
                    keys.iter().map(|k| offsets[k].1).collect()
                } else {
                    Vec::new()
                },
                vertices: keys,
                weight,
                driver: None,
            }
        })
        .collect();
    // Unused images and materials stay: they're cheap and keep indices stable.
    Ok(mesh)
}

/// The world matrix of every node of `scene` at its rest transform, and each
/// node's parent.
fn scene_world(
    doc: &gltf::Document,
    scene: &gltf::Scene,
) -> (Vec<Option<Mat4>>, Vec<Option<usize>>) {
    let mut world = vec![None; doc.nodes().len()];
    let mut parent = vec![None; doc.nodes().len()];
    fn walk(
        node: &gltf::Node,
        m: Mat4,
        up: Option<usize>,
        world: &mut [Option<Mat4>],
        parent: &mut [Option<usize>],
    ) {
        let m = m * Mat4::from_cols_array_2d(&node.transform().matrix());
        world[node.index()] = Some(m);
        parent[node.index()] = up;
        for child in node.children() {
            walk(&child, m, Some(node.index()), world, parent);
        }
    }
    for node in scene.nodes() {
        walk(&node, Mat4::IDENTITY, None, &mut world, &mut parent);
    }
    (world, parent)
}

/// Where a skinned primitive's vertices sit in the scene. glTF ignores a skinned
/// mesh node's own transform: each vertex is placed by its joints, as
/// `sum(weight * joint_world * inverse_bind)`. Usually every joint agrees (one
/// matrix for the whole primitive); when the rest pose differs from the bind
/// pose each vertex gets its own blend, which is what a viewer shows at rest.
enum Placement {
    One(Mat4),
    PerVertex(Vec<Mat4>),
}

impl Placement {
    fn at(&self, i: usize) -> Mat4 {
        match self {
            Placement::One(m) => *m,
            Placement::PerVertex(v) => v[i],
        }
    }
}

fn skinned_placement(
    node: &gltf::Node,
    prim: &gltf::Primitive,
    count: usize,
    buffers: &[gltf::buffer::Data],
    world: &[Option<Mat4>],
) -> Option<Placement> {
    let skin = node.skin()?;
    let reader = skin.reader(|b| Some(&buffers[b.index()]));
    let ibms: Vec<Mat4> = reader
        .read_inverse_bind_matrices()
        .map(|it| it.map(|m| Mat4::from_cols_array_2d(&m)).collect())
        .unwrap_or_default();
    let joints: Vec<Mat4> = skin
        .joints()
        .enumerate()
        .map(|(k, j)| {
            world[j.index()].unwrap_or(Mat4::IDENTITY)
                * ibms.get(k).copied().unwrap_or(Mat4::IDENTITY)
        })
        .collect();
    let first = *joints.first()?;
    let scale = first
        .abs()
        .to_cols_array()
        .iter()
        .fold(1e-6f32, |a, b| a.max(*b));
    if joints.iter().all(|m| {
        (*m - first)
            .abs()
            .to_cols_array()
            .iter()
            .all(|d| *d <= 1e-4 * scale)
    }) {
        return Some(Placement::One(first));
    }
    let r = prim.reader(|b| Some(&buffers[b.index()]));
    let mut acc = vec![(Mat4::ZERO, 0.0f32); count];
    let mut set = 0;
    while let (Some(js), Some(ws)) = (r.read_joints(set), r.read_weights(set)) {
        for (v, (j, w)) in js.into_u16().zip(ws.into_f32()).enumerate().take(count) {
            for k in 0..4 {
                if w[k] > 0.0
                    && let Some(m) = joints.get(j[k] as usize)
                {
                    acc[v].0 += *m * w[k];
                    acc[v].1 += w[k];
                }
            }
        }
        set += 1;
    }
    if set == 0 {
        return Some(Placement::One(first));
    }
    Some(Placement::PerVertex(
        acc.into_iter()
            .map(|(m, w)| if w > 0.0 { m * (1.0 / w) } else { first })
            .collect(),
    ))
}

fn collect_node(
    node: &gltf::Node,
    parent: Mat4,
    buffers: &[gltf::buffer::Data],
    scene: &[Option<Mat4>],
    g: &mut Gathered,
) {
    let world = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
    if let Some(gmesh) = node.mesh() {
        let object = g.mesh.objects.len();
        let mut object_used = false;
        let names = target_names(&gmesh);
        let default_weights: Vec<f32> = gmesh.weights().map(<[f32]>::to_vec).unwrap_or_default();
        for prim in gmesh.primitives() {
            if prim.mode() != gltf::mesh::Mode::Triangles {
                continue;
            }
            let reader = prim.reader(|b| Some(&buffers[b.index()]));
            let Some(pos) = reader.read_positions() else {
                continue;
            };
            let pos: Vec<Vec3> = pos.map(Vec3::from).collect();
            let count = pos.len();
            let place = skinned_placement(node, &prim, count, buffers, scene)
                .unwrap_or(Placement::One(world));
            let flips = Mat3::from_mat4(place.at(0)).determinant() < 0.0;
            // Per vertex: the point matrix, its linear part and the normal matrix.
            let frame = |i: usize| {
                let m = place.at(i);
                let linear = Mat3::from_mat4(m);
                (m, linear, linear.inverse().transpose())
            };
            let tris: Vec<u32> = match reader.read_indices() {
                Some(idx) => idx.into_u32().collect(),
                None => (0..count as u32).collect(),
            };
            let tris: Vec<u32> = tris.as_chunks::<3>().0.iter().flatten().copied().collect();
            if tris.iter().any(|&i| i as usize >= count) {
                continue;
            }
            // Only the vertices the triangles use (primitives may share one
            // big vertex buffer), in their original order.
            let mut used = vec![false; count];
            for &i in &tris {
                used[i as usize] = true;
            }
            let mut remap = vec![u32::MAX; count];
            let base = g.mesh.positions.len();
            let mut kept: Vec<usize> = Vec::new();
            for (i, u) in used.iter().enumerate() {
                if *u {
                    remap[i] = (base + kept.len()) as u32;
                    kept.push(i);
                }
            }
            if kept.is_empty() {
                continue;
            }
            let first_index = g.mesh.indices.len();
            g.mesh
                .positions
                .extend(kept.iter().map(|&i| frame(i).0.transform_point3(pos[i])));
            match reader.read_normals() {
                Some(nr) => {
                    let nr: Vec<Vec3> = nr.map(Vec3::from).collect();
                    g.mesh.normals.extend(
                        kept.iter()
                            .map(|&i| (frame(i).2 * nr[i]).try_normalize().unwrap_or(Vec3::Y)),
                    );
                    g.has_normal.extend(std::iter::repeat_n(true, kept.len()));
                }
                None => {
                    g.mesh
                        .normals
                        .extend(std::iter::repeat_n(Vec3::Y, kept.len()));
                    g.has_normal.extend(std::iter::repeat_n(false, kept.len()));
                }
            }
            match reader.read_tangents() {
                Some(t) => {
                    let t: Vec<[f32; 4]> = t.collect();
                    g.tangents.extend(kept.iter().map(|&i| {
                        let [x, y, z, w] = t[i];
                        let d = (frame(i).1 * Vec3::new(x, y, z)).normalize_or_zero();
                        Some(d.extend(if flips { -w } else { w }))
                    }));
                }
                None => g.tangents.extend(std::iter::repeat_n(None, kept.len())),
            }
            match reader.read_colors(0) {
                Some(c) => {
                    let c: Vec<[f32; 4]> = c.into_rgba_f32().collect();
                    g.colors
                        .extend(kept.iter().map(|&i| Some(Vec4::from(c[i]))));
                }
                None => g.colors.extend(std::iter::repeat_n(None, kept.len())),
            }
            let mut set = 0;
            while let Some(tc) = reader.read_tex_coords(set) {
                let tc: Vec<[f32; 2]> = tc.into_f32().collect();
                if g.uvs.len() <= set as usize {
                    g.uvs.push(vec![None; base]);
                }
                g.uvs[set as usize].extend(kept.iter().map(|&i| Some(Vec2::from(tc[i]))));
                set += 1;
            }
            let total = g.mesh.positions.len();
            for s in &mut g.uvs {
                s.resize(total, None);
            }
            // Triangles, kept facing the same way when the node mirrors them.
            for t in tris.as_chunks::<3>().0 {
                let [a, b, c] = t.map(|i| remap[i as usize]);
                if flips {
                    g.mesh.indices.extend_from_slice(&[a, c, b]);
                } else {
                    g.mesh.indices.extend_from_slice(&[a, b, c]);
                }
            }
            // Morph targets: offsets transform like directions.
            for (k, (dp, dn, _)) in reader.read_morph_targets().enumerate() {
                let name = names
                    .get(k)
                    .filter(|n| !n.is_empty())
                    .cloned()
                    .unwrap_or_else(|| format!("target_{k}"));
                let weight = default_weights.get(k).copied().unwrap_or(0.0);
                let slot = match g.shapes.iter().position(|s| s.name == name) {
                    Some(i) => i,
                    None => {
                        g.shapes.push(GatheredShape {
                            name,
                            offsets: HashMap::new(),
                            weight,
                            has_normals: false,
                        });
                        g.shapes.len() - 1
                    }
                };
                let dp: Vec<Vec3> = dp
                    .map(|it| it.map(Vec3::from).collect())
                    .unwrap_or_default();
                let dn: Vec<Vec3> = dn
                    .map(|it| it.map(Vec3::from).collect())
                    .unwrap_or_default();
                let shape = &mut g.shapes[slot];
                shape.has_normals |= !dn.is_empty();
                for (j, &i) in kept.iter().enumerate() {
                    let (_, linear, normal_m) = frame(i);
                    let p = dp.get(i).map_or(Vec3::ZERO, |d| linear * *d);
                    let nn = dn.get(i).map_or(Vec3::ZERO, |d| normal_m * *d);
                    if p.length_squared() > crate::shapes::EPSILON.powi(2) || nn != Vec3::ZERO {
                        shape.offsets.insert((base + j) as u32, (p, nn));
                    }
                }
            }
            g.mesh.parts.push(Part {
                object,
                material: prim.material().index(),
                first_vertex: base,
                vertex_count: kept.len(),
                first_index,
                index_count: g.mesh.indices.len() - first_index,
            });
            object_used = true;
        }
        if object_used {
            g.mesh.objects.push(
                node.name()
                    .or(gmesh.name())
                    .map_or_else(|| format!("mesh_{}", node.index()), str::to_string),
            );
        }
    }
    for child in node.children() {
        collect_node(&child, world, buffers, scene, g);
    }
}

/// The joints of the first skin in a glTF file at their rest pose, in the same
/// space [`load_gltf`] puts the mesh in (which places skinned vertices by these
/// same joints), each with its nearest ancestor that is also a joint of the
/// skin. Empty for files without a skin. For an FBX, every bone of the file
/// (see [`fbx::FbxScene::joints`]).
pub fn load_joints(path: &Path) -> Result<Vec<Joint>> {
    let ext = extension(path);
    if ext == "fbx" {
        return Ok(fbx::load(path)?.joints);
    }
    if ext != "gltf" && ext != "glb" {
        return Ok(Vec::new());
    }
    let (doc, _) = open_gltf(path)?;
    let scene = doc
        .default_scene()
        .or_else(|| doc.scenes().next())
        .context("glTF file has no scene")?;
    let (world, parent) = scene_world(&doc, &scene);
    let Some(skin) = doc
        .nodes()
        .filter(|n| world[n.index()].is_some())
        .find_map(|n| n.skin())
    else {
        return Ok(Vec::new());
    };
    let nodes: Vec<usize> = skin.joints().map(|n| n.index()).collect();
    let slot: HashMap<usize, usize> = nodes.iter().enumerate().map(|(i, n)| (*n, i)).collect();
    Ok(skin
        .joints()
        .enumerate()
        .map(|(i, node)| {
            let mut up = parent[node.index()];
            while let Some(p) = up {
                if slot.contains_key(&p) {
                    break;
                }
                up = parent[p];
            }
            Joint {
                name: node
                    .name()
                    .map_or_else(|| format!("joint_{i}"), str::to_string),
                position: world[node.index()]
                    .unwrap_or(Mat4::IDENTITY)
                    .w_axis
                    .truncate(),
                parent: up.map(|p| slot[&p]),
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_obj_quads_and_slashes() {
        let obj = "v 0 0 0\nv 1 0 0\nv 1 1 0\nv 0 1 0\nf 1/1/1 2/2/2 3/3/3 4/4/4\n";
        let mesh = parse_obj(obj).unwrap();
        assert_eq!(mesh.vertex_count(), 4);
        assert_eq!(mesh.indices, vec![0, 1, 2, 0, 2, 3]);
    }

    #[test]
    fn parses_negative_obj_indices() {
        let obj = "v 0 0 0\nv 1 0 0\nv 0 1 0\nf -3 -2 -1\n";
        assert_eq!(parse_obj(obj).unwrap().indices, vec![0, 1, 2]);
    }

    #[test]
    fn rejects_out_of_range_faces() {
        assert!(parse_obj("v 0 0 0\nf 1 2 3\n").is_err());
    }

    #[test]
    fn obj_keeps_objects_materials_textures_and_seams() {
        let dir = std::env::temp_dir().join(format!("hive-rig-obj-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("tex")).unwrap();
        std::fs::write(
            dir.join("look.mtl"),
            "newmtl cloth\nKd 0.5 0.25 1\nmap_Kd tex/cloth.png\nnewmtl glass\nKd 1 1 1\nd 0.5\n",
        )
        .unwrap();
        std::fs::write(dir.join("tex").join("cloth.png"), b"png bytes").unwrap();
        // Two quads sharing vertices 2 and 3, but with different UVs there (a
        // seam), in two objects with a material each.
        let obj = "mtllib look.mtl\n\
                   v 0 0 0 1 0 0\nv 1 0 0 0 1 0\nv 1 1 0 0 0 1\nv 0 1 0 1 1 1\nv 1 2 0\nv 0 2 0\n\
                   vt 0 0\nvt 1 0\nvt 1 1\nvt 0 1\nvt 0.5 0.5\n\
                   o shirt\nusemtl cloth\nf 1/1 2/2 3/3 4/4\n\
                   o visor\nusemtl glass\nf 4/5 3/5 5/3 6/4\n";
        let mesh = parse_obj_with(obj, Some(&dir)).unwrap();
        mesh.validate().unwrap();
        assert_eq!(mesh.objects, vec!["shirt".to_string(), "visor".to_string()]);
        assert_eq!(mesh.parts.len(), 2);
        // The seam splits 3 and 4: 8 vertices, not 6.
        assert_eq!(mesh.vertex_count(), 8);
        assert_eq!(mesh.materials.len(), 2);
        assert_eq!(mesh.materials[0].base_color, [0.5, 0.25, 1.0, 1.0]);
        assert_eq!(mesh.materials[1].base_color[3], 0.5);
        let t = mesh.materials[0].base_color_texture.expect("texture");
        assert_eq!(
            mesh.images[t.image].bytes().as_deref(),
            Some(&b"png bytes"[..])
        );
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(mesh.uvs.len(), 1);
        // V is flipped to glTF's top-left origin.
        assert_eq!(mesh.uvs[0][0], Vec2::new(0.0, 1.0));
        assert_eq!(mesh.colors.len(), 8);
        assert_eq!(mesh.colors[1], glam::Vec4::new(0.0, 1.0, 0.0, 1.0));
        // The seam copies weld back into one surface.
        let welded = mesh.welded();
        let distinct: std::collections::HashSet<u32> = welded.into_iter().collect();
        assert_eq!(distinct.len(), 6);
    }
}
