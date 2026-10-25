//! Binary FBX (7.x) reader: meshes with their normals, UV sets, vertex
//! colours, materials and textures, blend shapes, the skeleton and its skin.
//!
//! Written from the publicly described binary layout (no FBX SDK): a header
//! with the version, a tree of named records whose properties are typed
//! scalars, strings and arrays (arrays optionally zlib-compressed), and a
//! footer. From 7.5 the record offsets are 64-bit. ASCII FBX isn't read.
//!
//! The scene is read the way the format describes it: objects (models,
//! geometry, materials, textures, deformers, poses) joined by connections, each
//! model placed by its translation, rotation (in its rotation order, with pre-
//! and post-rotation and pivots), scaling and geometric transform, and the
//! whole scene turned from the file's axes (`GlobalSettings`) into Y up,
//! front +Z, and scaled from its unit (`UnitScaleFactor`, centimetres per
//! unit) into metres: the frame a .glb is read in. A skinned vertex sits where
//! its clusters put it at the file's current pose (the bind pose, when the
//! skeleton wasn't moved), like a skinned glTF vertex.
//!
//! The file is untrusted: every offset, length and index is checked, arrays
//! inflate only to the size they declare, and the record tree is depth-limited.

use std::collections::{HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context, Result, bail, ensure};
use glam::{DMat3, DMat4, DVec3, Vec2, Vec3, Vec4};

use crate::joints::Joint;
use crate::mesh::{AlphaMode, Image, Material, Mesh, Part, TextureRef, smooth_normals};
use crate::shapes::Shape;

/// `Kaydara FBX Binary  ` and a NUL, then 0x1a 0x00 and the version.
const MAGIC: &[u8; 21] = b"Kaydara FBX Binary  \x00";
/// The deepest record nesting read (real files use under ten).
const MAX_DEPTH: usize = 64;
/// The largest array read, in bytes once inflated.
const MAX_ARRAY: usize = 1 << 31;

/// What an FBX file holds, in the file's frame turned Y up and in metres.
#[derive(Debug, Clone, Default)]
pub struct FbxScene {
    pub mesh: Mesh,
    /// Every bone (LimbNode, or a model a skin cluster links to), in file order.
    pub joints: Vec<Joint>,
    /// Per mesh vertex, its skin as (index into `joints`, weight), summing to 1;
    /// empty for a vertex no cluster weighs.
    pub skin: Vec<Vec<(usize, f32)>>,
}

/// Read a binary FBX file.
pub fn load(path: &Path) -> Result<FbxScene> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    read(&bytes, path.parent()).with_context(|| format!("reading {}", path.display()))
}

// --------------------------------------------------------------------------- records

/// A typed property of a record.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Value {
    Bool(bool),
    I16(i16),
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
    Str(String),
    Raw(Vec<u8>),
    Bools(Vec<bool>),
    I32s(Vec<i32>),
    I64s(Vec<i64>),
    F32s(Vec<f32>),
    F64s(Vec<f64>),
}

impl Value {
    fn as_i64(&self) -> Option<i64> {
        match *self {
            Value::Bool(v) => Some(i64::from(v)),
            Value::I16(v) => Some(v.into()),
            Value::I32(v) => Some(v.into()),
            Value::I64(v) => Some(v),
            Value::F32(v) => Some(v as i64),
            Value::F64(v) => Some(v as i64),
            _ => None,
        }
    }

    fn as_f64(&self) -> Option<f64> {
        match *self {
            Value::Bool(v) => Some(f64::from(u8::from(v))),
            Value::I16(v) => Some(v.into()),
            Value::I32(v) => Some(v.into()),
            Value::I64(v) => Some(v as f64),
            Value::F32(v) => Some(v.into()),
            Value::F64(v) => Some(v),
            _ => None,
        }
    }

    fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    /// Any numeric array as doubles.
    fn f64s(&self) -> Option<Vec<f64>> {
        Some(match self {
            Value::F64s(v) => v.clone(),
            Value::F32s(v) => v.iter().map(|&x| x.into()).collect(),
            Value::I32s(v) => v.iter().map(|&x| x.into()).collect(),
            Value::I64s(v) => v.iter().map(|&x| x as f64).collect(),
            _ => return None,
        })
    }

    /// Any integer array as i64.
    fn i64s(&self) -> Option<Vec<i64>> {
        Some(match self {
            Value::I32s(v) => v.iter().map(|&x| x.into()).collect(),
            Value::I64s(v) => v.clone(),
            _ => return None,
        })
    }
}

/// A record: its name, properties and nested records.
#[derive(Debug, Clone, Default)]
pub(crate) struct Node {
    pub name: String,
    pub props: Vec<Value>,
    pub children: Vec<Node>,
}

impl Node {
    fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }

    fn all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }

    /// The first property of the child record `name`.
    fn value(&self, name: &str) -> Option<&Value> {
        self.child(name)?.props.first()
    }

    fn id(&self) -> Option<i64> {
        self.props.first()?.as_i64()
    }

    /// The object's class (`Mesh`, `LimbNode`, `Skin`...).
    fn class(&self) -> &str {
        self.props.get(2).and_then(Value::as_str).unwrap_or("")
    }

    /// The object's name without its `\0\x01Class` (or `Class::`) part.
    fn object_name(&self) -> String {
        let full = self.props.get(1).and_then(Value::as_str).unwrap_or("");
        let name = full.split("\0\u{1}").next().unwrap_or("");
        match name.split_once("::") {
            Some((_, rest)) if !full.contains('\0') => rest.to_string(),
            _ => name.to_string(),
        }
    }
}

struct Reader<'a> {
    b: &'a [u8],
    /// 7.5 and later: 64-bit record offsets.
    wide: bool,
}

impl Reader<'_> {
    fn bytes(&self, at: usize, n: usize) -> Result<&[u8]> {
        at.checked_add(n)
            .and_then(|end| self.b.get(at..end))
            .context("the file ends inside a record")
    }

    fn u32_at(&self, at: usize) -> Result<u32> {
        Ok(u32::from_le_bytes(self.bytes(at, 4)?.try_into().unwrap()))
    }

    fn u64_at(&self, at: usize) -> Result<u64> {
        Ok(u64::from_le_bytes(self.bytes(at, 8)?.try_into().unwrap()))
    }

    fn size_at(&self, at: usize) -> Result<usize> {
        let v = if self.wide {
            self.u64_at(at)?
        } else {
            self.u32_at(at)?.into()
        };
        usize::try_from(v).context("an offset doesn't fit in memory")
    }

    /// The record at `at`, or `None` for the null record that ends a list.
    fn node(&self, at: &mut usize, depth: usize) -> Result<Option<Node>> {
        ensure!(depth < MAX_DEPTH, "records nest too deeply");
        let w = if self.wide { 8 } else { 4 };
        let start = *at;
        let end = self.size_at(start)?;
        let count = self.size_at(start + w)?;
        let props_len = self.size_at(start + 2 * w)?;
        let name_len = self.bytes(start + 3 * w, 1)?[0] as usize;
        if end == 0 {
            *at = start + 3 * w + 1;
            return Ok(None);
        }
        ensure!(
            end > start && end <= self.b.len(),
            "a record ends outside the file"
        );
        let name = String::from_utf8_lossy(self.bytes(start + 3 * w + 1, name_len)?).into_owned();
        *at = start + 3 * w + 1 + name_len;
        let props_end = at
            .checked_add(props_len)
            .filter(|&e| e <= end)
            .with_context(|| format!("{name}: properties run past the record"))?;
        ensure!(count <= props_len, "{name}: more properties than bytes");
        let mut props = Vec::new();
        for _ in 0..count {
            props.push(self.value(at, props_end).with_context(|| name.clone())?);
        }
        ensure!(*at == props_end, "{name}: property lengths don't add up");
        let mut children = Vec::new();
        while *at < end {
            match self.node(at, depth + 1)? {
                Some(c) => children.push(c),
                None => break,
            }
        }
        ensure!(*at <= end, "{name}: a nested record runs past it");
        *at = end;
        Ok(Some(Node {
            name,
            props,
            children,
        }))
    }

    fn value(&self, at: &mut usize, limit: usize) -> Result<Value> {
        let code = self.bytes(*at, 1)?[0];
        *at += 1;
        let mut take = |n: usize| -> Result<&[u8]> {
            ensure!(
                at.checked_add(n).is_some_and(|e| e <= limit),
                "a property runs past its record"
            );
            let s = self.bytes(*at, n)?;
            *at += n;
            Ok(s)
        };
        Ok(match code {
            b'C' => Value::Bool(take(1)?[0] != 0),
            b'Y' => Value::I16(i16::from_le_bytes(take(2)?.try_into().unwrap())),
            b'I' => Value::I32(i32::from_le_bytes(take(4)?.try_into().unwrap())),
            b'L' => Value::I64(i64::from_le_bytes(take(8)?.try_into().unwrap())),
            b'F' => Value::F32(f32::from_le_bytes(take(4)?.try_into().unwrap())),
            b'D' => Value::F64(f64::from_le_bytes(take(8)?.try_into().unwrap())),
            b'S' | b'R' => {
                let n = u32::from_le_bytes(take(4)?.try_into().unwrap()) as usize;
                let s = take(n)?;
                if code == b'S' {
                    Value::Str(String::from_utf8_lossy(s).into_owned())
                } else {
                    Value::Raw(s.to_vec())
                }
            }
            b'f' | b'd' | b'l' | b'i' | b'b' => {
                let head = take(12)?;
                let count = u32::from_le_bytes(head[0..4].try_into().unwrap()) as usize;
                let encoding = u32::from_le_bytes(head[4..8].try_into().unwrap());
                let stored = u32::from_le_bytes(head[8..12].try_into().unwrap()) as usize;
                let size = match code {
                    b'd' | b'l' => 8,
                    b'f' | b'i' => 4,
                    _ => 1,
                };
                let len = count
                    .checked_mul(size)
                    .filter(|&l| l <= MAX_ARRAY)
                    .context("an array is too large")?;
                let data = take(stored)?;
                let raw: std::borrow::Cow<[u8]> = match encoding {
                    0 => {
                        ensure!(stored == len, "an array's length doesn't match its count");
                        data.into()
                    }
                    1 => {
                        let out =
                            miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(data, len)
                                .map_err(|e| {
                                    anyhow::anyhow!("a compressed array is damaged: {e:?}")
                                })?;
                        ensure!(out.len() == len, "a compressed array has the wrong length");
                        out.into()
                    }
                    e => bail!("unknown array encoding {e}"),
                };
                match code {
                    b'f' => Value::F32s(
                        raw.as_chunks::<4>()
                            .0
                            .iter()
                            .map(|c| f32::from_le_bytes(*c))
                            .collect(),
                    ),
                    b'd' => Value::F64s(
                        raw.as_chunks::<8>()
                            .0
                            .iter()
                            .map(|c| f64::from_le_bytes(*c))
                            .collect(),
                    ),
                    b'l' => Value::I64s(
                        raw.as_chunks::<8>()
                            .0
                            .iter()
                            .map(|c| i64::from_le_bytes(*c))
                            .collect(),
                    ),
                    b'i' => Value::I32s(
                        raw.as_chunks::<4>()
                            .0
                            .iter()
                            .map(|c| i32::from_le_bytes(*c))
                            .collect(),
                    ),
                    _ => Value::Bools(raw.iter().map(|&x| x != 0).collect()),
                }
            }
            other => bail!("unknown property type {:?}", other as char),
        })
    }
}

/// The top-level records of a binary FBX file, and its version.
pub(crate) fn parse_records(bytes: &[u8]) -> Result<(u32, Vec<Node>)> {
    if !bytes.starts_with(MAGIC) {
        let head = &bytes[..bytes.len().min(4096)];
        if head.windows(18).any(|w| w == b"FBXHeaderExtension") {
            bail!("this is an ASCII FBX; hive-rig reads binary FBX (7.x): save it as binary");
        }
        bail!("not a binary FBX file");
    }
    ensure!(bytes.len() >= 27, "the FBX header is cut short");
    let version = u32::from_le_bytes(bytes[23..27].try_into().unwrap());
    ensure!(
        (7000..8000).contains(&version),
        "FBX version {version} isn't supported (binary 7.x is)"
    );
    let r = Reader {
        b: bytes,
        wide: version >= 7500,
    };
    let mut at = 27;
    let mut top = Vec::new();
    while at < bytes.len() {
        match r.node(&mut at, 0)? {
            Some(n) => top.push(n),
            None => break,
        }
    }
    Ok((version, top))
}

// --------------------------------------------------------------------------- scene

struct Link<'a> {
    child: i64,
    parent: i64,
    /// The parent's property it connects to (a texture onto a material slot).
    property: Option<&'a str>,
}

struct Doc<'a> {
    objects: HashMap<i64, &'a Node>,
    /// Object ids in file order.
    order: Vec<i64>,
    links: Vec<Link<'a>>,
    /// Link indices by parent and by child, in file order.
    by_parent: HashMap<i64, Vec<usize>>,
    by_child: HashMap<i64, Vec<usize>>,
    /// Default properties per object kind (`Model`, `Material`...).
    templates: HashMap<String, &'a Node>,
}

impl<'a> Doc<'a> {
    fn new(top: &'a [Node]) -> Doc<'a> {
        let find = |name: &str| top.iter().find(|n| n.name == name);
        let mut objects = HashMap::new();
        let mut order = Vec::new();
        if let Some(o) = find("Objects") {
            for n in &o.children {
                if let Some(id) = n.id()
                    && objects.insert(id, n).is_none()
                {
                    order.push(id);
                }
            }
        }
        let mut links = Vec::new();
        if let Some(c) = find("Connections") {
            for n in c.all("C") {
                let kind = n.props.first().and_then(Value::as_str).unwrap_or("");
                let (Some(child), Some(parent)) = (
                    n.props.get(1).and_then(Value::as_i64),
                    n.props.get(2).and_then(Value::as_i64),
                ) else {
                    continue;
                };
                let property = (kind == "OP")
                    .then(|| n.props.get(3).and_then(Value::as_str))
                    .flatten();
                links.push(Link {
                    child,
                    parent,
                    property,
                });
            }
        }
        let mut by_parent: HashMap<i64, Vec<usize>> = HashMap::new();
        let mut by_child: HashMap<i64, Vec<usize>> = HashMap::new();
        for (i, l) in links.iter().enumerate() {
            by_parent.entry(l.parent).or_default().push(i);
            by_child.entry(l.child).or_default().push(i);
        }
        let mut templates = HashMap::new();
        if let Some(d) = find("Definitions") {
            for t in d.all("ObjectType") {
                let kind = t.props.first().and_then(Value::as_str).unwrap_or("");
                if let Some(p) = t
                    .child("PropertyTemplate")
                    .and_then(|p| p.child("Properties70"))
                {
                    templates.insert(kind.to_string(), p);
                }
            }
        }
        Doc {
            objects,
            order,
            links,
            by_parent,
            by_child,
            templates,
        }
    }

    /// Objects connected under `parent` (as objects, not onto a property), in order.
    fn children(&self, parent: i64) -> impl Iterator<Item = (&'a Node, i64)> + '_ {
        self.by_parent
            .get(&parent)
            .into_iter()
            .flatten()
            .filter(|&&i| self.links[i].property.is_none())
            .filter_map(|&i| {
                let id = self.links[i].child;
                self.objects.get(&id).map(|n| (*n, id))
            })
    }

    /// Objects `child` connects to as an object, in order (0, the scene, has none).
    fn parents(&self, child: i64) -> impl Iterator<Item = (&'a Node, i64)> + '_ {
        self.by_child
            .get(&child)
            .into_iter()
            .flatten()
            .filter(|&&i| self.links[i].property.is_none())
            .filter_map(|&i| {
                let id = self.links[i].parent;
                self.objects.get(&id).map(|n| (*n, id))
            })
    }

    /// A `Properties70` entry's values: the object's own, or its kind's default.
    fn prop(&self, obj: &'a Node, name: &str) -> Option<&'a [Value]> {
        let find = |p70: &'a Node| {
            p70.all("P")
                .find(|p| p.props.first().and_then(Value::as_str) == Some(name))
                .map(|p| p.props.get(4..).unwrap_or(&[]))
        };
        obj.child("Properties70")
            .and_then(find)
            .or_else(|| self.templates.get(&obj.name).and_then(|t| find(t)))
    }

    fn prop_f64(&self, obj: &'a Node, name: &str) -> Option<f64> {
        self.prop(obj, name)?.first()?.as_f64()
    }

    fn prop_vec(&self, obj: &'a Node, name: &str) -> Option<DVec3> {
        let v = self.prop(obj, name)?;
        Some(DVec3::new(
            v.first()?.as_f64()?,
            v.get(1)?.as_f64()?,
            v.get(2)?.as_f64()?,
        ))
    }

    fn prop_str(&self, obj: &'a Node, name: &str) -> Option<&'a str> {
        self.prop(obj, name)?.first()?.as_str()
    }
}

/// A rotation from Euler degrees applied in `order` (FBX `RotationOrder`:
/// 0 XYZ, 1 XZY, 2 YZX, 3 YXZ, 4 ZXY, 5 ZYX; XYZ turns about X first).
fn euler(deg: DVec3, order: i64) -> DMat4 {
    let (x, y, z) = (
        DMat4::from_rotation_x(deg.x.to_radians()),
        DMat4::from_rotation_y(deg.y.to_radians()),
        DMat4::from_rotation_z(deg.z.to_radians()),
    );
    match order {
        1 => y * z * x,
        2 => x * z * y,
        3 => z * x * y,
        4 => y * x * z,
        5 => x * y * z,
        _ => z * y * x,
    }
}

impl<'a> Doc<'a> {
    /// A model's transform relative to its parent model.
    fn local(&self, m: &'a Node) -> DMat4 {
        let v = |name: &str, d: DVec3| self.prop_vec(m, name).unwrap_or(d);
        let t = DMat4::from_translation(v("Lcl Translation", DVec3::ZERO));
        let order = self
            .prop(m, "RotationOrder")
            .and_then(|p| p.first()?.as_i64())
            .unwrap_or(0);
        let r = euler(v("Lcl Rotation", DVec3::ZERO), order);
        let s = DMat4::from_scale(v("Lcl Scaling", DVec3::ONE));
        let pre = euler(v("PreRotation", DVec3::ZERO), 0);
        let post = euler(v("PostRotation", DVec3::ZERO), 0);
        let r_off = DMat4::from_translation(v("RotationOffset", DVec3::ZERO));
        let r_piv = DMat4::from_translation(v("RotationPivot", DVec3::ZERO));
        let s_off = DMat4::from_translation(v("ScalingOffset", DVec3::ZERO));
        let s_piv = DMat4::from_translation(v("ScalingPivot", DVec3::ZERO));
        t * r_off
            * r_piv
            * pre
            * r
            * post.inverse()
            * r_piv.inverse()
            * s_off
            * s_piv
            * s
            * s_piv.inverse()
    }

    /// A model's geometric transform: moves its geometry only, not its children.
    fn geometric(&self, m: &'a Node) -> DMat4 {
        let v = |name: &str, d: DVec3| self.prop_vec(m, name).unwrap_or(d);
        DMat4::from_translation(v("GeometricTranslation", DVec3::ZERO))
            * euler(v("GeometricRotation", DVec3::ZERO), 0)
            * DMat4::from_scale(v("GeometricScaling", DVec3::ONE))
    }

    /// The model a model hangs from, if not the scene.
    fn parent_model(&self, id: i64) -> Option<i64> {
        self.parents(id)
            .find(|(n, _)| n.name == "Model")
            .map(|(_, p)| p)
    }

    /// A model's transform in the file's scene.
    fn global(&self, id: i64, memo: &mut HashMap<i64, DMat4>) -> DMat4 {
        if let Some(m) = memo.get(&id) {
            return *m;
        }
        // Walk up iteratively (a cycle stops at the first repeat).
        let mut chain = vec![id];
        let mut seen = HashSet::from([id]);
        let mut at = id;
        while let Some(p) = self.parent_model(at) {
            if memo.contains_key(&p) || !seen.insert(p) {
                if memo.contains_key(&p) {
                    chain.push(p);
                }
                break;
            }
            chain.push(p);
            at = p;
        }
        let mut m = DMat4::IDENTITY;
        for &c in chain.iter().rev() {
            if let Some(known) = memo.get(&c) {
                m = *known;
                continue;
            }
            m *= self.local(self.objects[&c]);
            memo.insert(c, m);
        }
        memo[&id]
    }
}

/// The turn from the file's axes (up, front and the third, "coord" axis, each
/// with a sign) onto Y up, front +Z and +X.
fn axes(doc: &Doc, settings: Option<&Node>) -> DMat4 {
    let Some(s) = settings else {
        return DMat4::IDENTITY;
    };
    let get = |name: &str, d: i64| {
        doc.prop(s, name)
            .and_then(|p| p.first()?.as_i64())
            .unwrap_or(d)
    };
    let axis = |index: i64, sign: i64| {
        let mut v = DVec3::ZERO;
        v[index.clamp(0, 2) as usize] = if sign < 0 { -1.0 } else { 1.0 };
        v
    };
    let up = axis(get("UpAxis", 1), get("UpAxisSign", 1));
    let front = axis(get("FrontAxis", 2), get("FrontAxisSign", 1));
    let coord = axis(get("CoordAxis", 0), get("CoordAxisSign", 1));
    // Rows: the file's coord, up and front axes become X, Y and Z.
    let m = DMat3::from_cols(coord, up, front).transpose();
    if m.determinant().abs() < 0.5 {
        return DMat4::IDENTITY;
    }
    DMat4::from_mat3(m)
}

/// A geometry layer element (normals, UVs, colours): values by polygon
/// vertex, control point or polygon, directly or through an index.
struct Layer {
    mapping: String,
    indexed: bool,
    data: Vec<f64>,
    index: Vec<i64>,
    width: usize,
}

impl Layer {
    fn read(e: &Node, data: &str, index: &str, width: usize) -> Option<Layer> {
        let mapping = e
            .value("MappingInformationType")
            .and_then(Value::as_str)
            .unwrap_or("ByPolygonVertex")
            .to_string();
        let reference = e
            .value("ReferenceInformationType")
            .and_then(Value::as_str)
            .unwrap_or("Direct");
        let values = e.value(data)?.f64s()?;
        let index = e.value(index).and_then(Value::i64s).unwrap_or_default();
        Some(Layer {
            mapping,
            indexed: reference != "Direct" && !index.is_empty(),
            data: values,
            index,
            width,
        })
    }

    /// The value at a polygon corner: polygon `poly`, corner `corner` (counted
    /// over the whole mesh), control point `cp`.
    fn at(&self, poly: usize, corner: usize, cp: usize) -> Option<&[f64]> {
        let i = match self.mapping.as_str() {
            "ByPolygonVertex" => corner,
            "ByVertice" | "ByVertex" | "ByControlPoint" => cp,
            "ByPolygon" => poly,
            "AllSame" => 0,
            _ => return None,
        };
        let i = if self.indexed {
            usize::try_from(*self.index.get(i)?).ok()?
        } else {
            i
        };
        self.data.get(i * self.width..(i + 1) * self.width)
    }
}

/// A cluster's influence: the joint and the matrix taking a geometry point to
/// the scene at the current pose.
struct Influence {
    joint: usize,
    weight: f64,
    matrix: DMat4,
}

fn matrix_of(v: Option<&Value>) -> Option<DMat4> {
    let a = v?.f64s()?;
    (a.len() == 16).then(|| DMat4::from_cols_array(&a.try_into().unwrap()))
}

fn close(a: &DMat4, b: &DMat4) -> f64 {
    let scale = b.to_cols_array().iter().fold(1.0f64, |m, x| m.max(x.abs()));
    (*a - *b)
        .to_cols_array()
        .iter()
        .fold(0.0f64, |m, x| m.max(x.abs()))
        / scale
}

/// A safe relative path for a texture the file names: under the file's
/// folder, never above it or absolute (an absolute name keeps its file name).
fn texture_path(relative: &str, absolute: &str) -> Option<PathBuf> {
    let clean = |s: &str| -> Option<PathBuf> {
        let p = PathBuf::from(s.replace('\\', "/"));
        let ok = !s.is_empty()
            && p.components()
                .all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
        ok.then_some(p)
    };
    clean(relative).or_else(|| {
        let name = absolute.replace('\\', "/");
        let base = name.rsplit('/').next().unwrap_or("");
        clean(base)
    })
}

fn mime_for(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        "image/jpeg"
    } else if lower.ends_with(".webp") {
        "image/webp"
    } else if lower.ends_with(".png") {
        "image/png"
    } else {
        ""
    }
}

/// A leaf bone an exporter added only to show where its parent bone ends
/// (Blender's `<parent>_end`, on by default): it has no children, no skin,
/// and the parent's name with `_end`. Its place comes from the exporter's
/// bone length, not the source skeleton, so it is left out.
fn is_leaf_marker(doc: &Doc, id: i64) -> bool {
    let name = doc.objects[&id].object_name();
    let Some(parent) = doc.parent_model(id) else {
        return false;
    };
    doc.children(id).all(|(n, _)| n.name != "Model")
        && name.strip_suffix("_end") == Some(doc.objects[&parent].object_name().as_str())
}

/// A morph target as the walk builds it up.
struct GatheredShape {
    name: String,
    offsets: HashMap<u32, (Vec3, Vec3)>,
    weight: f32,
    has_normals: bool,
}

/// Read a binary FBX held in memory. `dir` is where texture files are looked for.
pub fn read(bytes: &[u8], dir: Option<&Path>) -> Result<FbxScene> {
    let (_, top) = parse_records(bytes)?;
    let doc = Doc::new(&top);
    ensure!(
        top.iter().any(|n| n.name == "Objects"),
        "the FBX file has no objects"
    );
    let settings = top.iter().find(|n| n.name == "GlobalSettings");
    let unit = settings
        .and_then(|s| doc.prop_f64(s, "UnitScaleFactor"))
        .filter(|u| u.is_finite() && *u > 0.0)
        .unwrap_or(1.0);
    let root = axes(&doc, settings) * DMat4::from_scale(DVec3::splat(unit / 100.0));
    let mut memo = HashMap::new();

    // Models from the scene down, parents first, in connection order.
    let is_model = |id: &i64| doc.objects.get(id).is_some_and(|n| n.name == "Model");
    let mut models: Vec<i64> = Vec::new();
    let mut seen = HashSet::new();
    let roots: Vec<i64> = doc
        .order
        .iter()
        .copied()
        .filter(|id| is_model(id) && doc.parent_model(*id).is_none())
        .collect();
    for r in roots {
        let mut stack = vec![r];
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            models.push(id);
            let kids: Vec<i64> = doc
                .children(id)
                .filter(|(n, _)| n.name == "Model")
                .map(|(_, c)| c)
                .collect();
            stack.extend(kids.into_iter().rev());
        }
    }

    // Joints: LimbNodes and anything a cluster links to.
    let mut linked: HashSet<i64> = HashSet::new();
    for id in &doc.order {
        let n = doc.objects[id];
        if n.name == "Deformer" && n.class() == "Cluster" {
            linked.extend(
                doc.children(*id)
                    .filter(|(m, _)| m.name == "Model")
                    .map(|(_, m)| m),
            );
        }
    }
    let in_scene: HashSet<i64> = models.iter().copied().collect();
    let joint_ids: Vec<i64> = doc
        .order
        .iter()
        .copied()
        .filter(|id| in_scene.contains(id))
        .filter(|id| {
            matches!(doc.objects[id].class(), "LimbNode" | "Limb" | "Root") || linked.contains(id)
        })
        .filter(|id| linked.contains(id) || !is_leaf_marker(&doc, *id))
        .collect();
    let slot: HashMap<i64, usize> = joint_ids
        .iter()
        .enumerate()
        .map(|(i, id)| (*id, i))
        .collect();
    let joints: Vec<Joint> = joint_ids
        .iter()
        .map(|&id| {
            let mut up = doc.parent_model(id);
            let mut guard = 0;
            while let Some(p) = up {
                if slot.contains_key(&p) || guard > models.len() {
                    break;
                }
                up = doc.parent_model(p);
                guard += 1;
            }
            let position = (root * doc.global(id, &mut memo)).w_axis.truncate();
            Joint {
                name: doc.objects[&id].object_name(),
                position: position.as_vec3(),
                parent: up.and_then(|p| slot.get(&p).copied()),
            }
        })
        .collect();

    // Materials and the images their textures use.
    let mut mesh = Mesh::default();
    let mut material_index: HashMap<i64, usize> = HashMap::new();
    let mut image_index: HashMap<String, usize> = HashMap::new();
    let mut uv_names: Vec<String> = Vec::new();
    // UV set names, so a texture's `UVSet` finds its layer.
    for id in &doc.order {
        let n = doc.objects[id];
        if n.name == "Geometry" {
            for uv in n.all("LayerElementUV") {
                let k = uv.id().unwrap_or(0).max(0) as usize;
                let name = uv
                    .value("Name")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if uv_names.len() <= k {
                    uv_names.resize(k + 1, String::new());
                }
                if uv_names[k].is_empty() {
                    uv_names[k] = name;
                }
            }
        }
    }
    for &id in &doc.order {
        let n = doc.objects[&id];
        if n.name != "Material" {
            continue;
        }
        let color = |name: &str| doc.prop_vec(n, name);
        let factor = |name: &str| doc.prop_f64(n, name);
        let diffuse = color("DiffuseColor")
            .or_else(|| color("Diffuse"))
            .unwrap_or(DVec3::splat(0.8));
        let alpha = factor("Opacity")
            .or_else(|| factor("TransparencyFactor").map(|t| 1.0 - t))
            .unwrap_or(1.0)
            .clamp(0.0, 1.0);
        let emissive =
            color("EmissiveColor").unwrap_or(DVec3::ZERO) * factor("EmissiveFactor").unwrap_or(1.0);
        let mut m = Material {
            name: n.object_name(),
            base_color: [
                diffuse.x as f32,
                diffuse.y as f32,
                diffuse.z as f32,
                alpha as f32,
            ],
            metallic: factor("ReflectionFactor").unwrap_or(0.0).clamp(0.0, 1.0) as f32,
            roughness: factor("Shininess")
                .map_or(0.6, |s| 1.0 - s / 100.0)
                .clamp(0.0, 1.0) as f32,
            emissive: emissive.as_vec3().to_array(),
            alpha_mode: if alpha < 1.0 {
                AlphaMode::Blend
            } else {
                AlphaMode::Opaque
            },
            ..Default::default()
        };
        for &li in doc.by_parent.get(&id).into_iter().flatten() {
            let link = &doc.links[li];
            let Some(slot) = link.property else { continue };
            let Some(tex) = doc.objects.get(&link.child).filter(|t| t.name == "Texture") else {
                continue;
            };
            let Some(image) =
                texture_image(&doc, tex, link.child, dir, &mut mesh, &mut image_index)
            else {
                continue;
            };
            let uv_set = doc
                .prop_str(tex, "UVSet")
                .and_then(|u| uv_names.iter().position(|n| n == u))
                .unwrap_or(0);
            let t = Some(TextureRef {
                image,
                uv_set,
                scale: 1.0,
            });
            match slot {
                "DiffuseColor" | "Diffuse" | "Maya|baseColor" | "base_color_map" => {
                    m.base_color_texture = m.base_color_texture.or(t)
                }
                "NormalMap" | "Bump" | "Maya|normalCamera" => {
                    m.normal_texture = m.normal_texture.or(t)
                }
                "EmissiveColor" | "Emissive" | "Maya|emissionColor" => {
                    m.emissive_texture = m.emissive_texture.or(t)
                }
                _ => {}
            }
        }
        material_index.insert(id, mesh.materials.len());
        mesh.materials.push(m);
    }

    // Bind-pose matrices, to tell how the clusters store their mesh transform.
    let mut bind: HashMap<i64, DMat4> = HashMap::new();
    for id in &doc.order {
        let n = doc.objects[id];
        if n.name == "Pose" && n.class() == "BindPose" {
            for pn in n.all("PoseNode") {
                if let (Some(node), Some(m)) = (
                    pn.value("Node").and_then(Value::as_i64),
                    matrix_of(pn.value("Matrix")),
                ) {
                    bind.entry(node).or_insert(m);
                }
            }
        }
    }

    let mut skin: Vec<Vec<(usize, f32)>> = Vec::new();
    let mut has_normal: Vec<bool> = Vec::new();
    let mut uvs: Vec<Vec<Option<Vec2>>> = Vec::new();
    let mut colors: Vec<Option<Vec4>> = Vec::new();
    let mut shapes: Vec<GatheredShape> = Vec::new();

    for &model_id in &models {
        let model = doc.objects[&model_id];
        if model.class() != "Mesh" {
            continue;
        }
        let Some((geom, geom_id)) = doc
            .children(model_id)
            .find(|(n, _)| n.name == "Geometry" && n.class() == "Mesh")
        else {
            continue;
        };
        let Some(cps) = geom.value("Vertices").and_then(Value::f64s) else {
            continue;
        };
        let cps: Vec<DVec3> = cps
            .as_chunks::<3>()
            .0
            .iter()
            .map(|c| DVec3::from_array(*c))
            .collect();
        let Some(pvi) = geom.value("PolygonVertexIndex").and_then(Value::i64s) else {
            continue;
        };
        // Polygons as (first corner, corner count); a negative index ends one.
        let mut polygons: Vec<(usize, usize)> = Vec::new();
        let mut corner_cp: Vec<usize> = Vec::with_capacity(pvi.len());
        let mut start = 0;
        for (k, &raw) in pvi.iter().enumerate() {
            let cp = if raw < 0 { !raw } else { raw };
            let cp = usize::try_from(cp).ok().filter(|&c| c < cps.len());
            let Some(cp) = cp else {
                bail!(
                    "{}: a polygon names a vertex it doesn't have",
                    model.object_name()
                );
            };
            corner_cp.push(cp);
            if raw < 0 {
                polygons.push((start, k + 1 - start));
                start = k + 1;
            }
        }
        let normals = geom
            .child("LayerElementNormal")
            .and_then(|e| Layer::read(e, "Normals", "NormalsIndex", 3));
        let mut uv_layers: Vec<(usize, Layer)> = geom
            .all("LayerElementUV")
            .filter_map(|e| {
                Some((
                    e.id().unwrap_or(0).max(0) as usize,
                    Layer::read(e, "UV", "UVIndex", 2)?,
                ))
            })
            .collect();
        uv_layers.sort_by_key(|l| l.0);
        let color_layer = geom
            .child("LayerElementColor")
            .and_then(|e| Layer::read(e, "Colors", "ColorIndex", 4));
        let material_layer = geom
            .child("LayerElementMaterial")
            .and_then(|e| Layer::read(e, "Materials", "", 1));
        let slots: Vec<usize> = doc
            .children(model_id)
            .filter_map(|(_, id)| material_index.get(&id).copied())
            .collect();

        // Where each control point sits: through its clusters when skinned.
        let mesh_global = doc.global(model_id, &mut memo);
        let geometric = doc.geometric(model);
        let mut influences: Vec<Vec<Influence>> = (0..cps.len()).map(|_| Vec::new()).collect();
        let skins: Vec<i64> = doc
            .by_parent
            .get(&geom_id)
            .into_iter()
            .flatten()
            .map(|&i| doc.links[i].child)
            .filter(|id| {
                doc.objects
                    .get(id)
                    .is_some_and(|n| n.name == "Deformer" && n.class() == "Skin")
            })
            .collect();
        for skin_id in skins {
            let mut clusters = Vec::new();
            for (c, cid) in doc
                .children(skin_id)
                .filter(|(n, _)| n.name == "Deformer" && n.class() == "Cluster")
            {
                let Some((_, bone)) = doc.children(cid).find(|(n, _)| n.name == "Model") else {
                    continue;
                };
                let Some(&joint) = slot.get(&bone) else {
                    continue;
                };
                let indexes = c.value("Indexes").and_then(Value::i64s).unwrap_or_default();
                let weights = c.value("Weights").and_then(Value::f64s).unwrap_or_default();
                let transform = matrix_of(c.value("Transform")).unwrap_or(DMat4::IDENTITY);
                let link = matrix_of(c.value("TransformLink"))
                    .unwrap_or_else(|| doc.global(bone, &mut memo));
                clusters.push((joint, bone, indexes, weights, transform, link));
            }
            // The format describes `Transform` as the mesh's bind transform;
            // some writers store it relative to the link instead. Take the
            // reading that agrees with the mesh's bind pose.
            let mesh_bind = bind.get(&model_id).copied().unwrap_or(mesh_global);
            let (mut relative, mut absolute) = (0.0, 0.0);
            for (_, _, _, _, t, l) in &clusters {
                relative += close(&(*l * *t), &mesh_bind);
                absolute += close(t, &mesh_bind);
            }
            let stored_relative = relative < absolute;
            for (joint, bone, indexes, weights, t, l) in clusters {
                let x = if stored_relative { t } else { l.inverse() * t };
                let matrix = doc.global(bone, &mut memo) * x;
                for (&i, &w) in indexes.iter().zip(&weights) {
                    if let Some(list) = usize::try_from(i).ok().and_then(|i| influences.get_mut(i))
                        && w > 0.0
                        && w.is_finite()
                    {
                        list.push(Influence {
                            joint,
                            weight: w,
                            matrix,
                        });
                    }
                }
            }
        }
        let place: Vec<DMat4> = influences
            .iter()
            .map(|list| {
                let total: f64 = list.iter().map(|i| i.weight).sum();
                let m = if total > 0.0 {
                    list.iter()
                        .fold(DMat4::ZERO, |m, i| m + i.matrix * (i.weight / total))
                } else {
                    mesh_global
                };
                root * m * geometric
            })
            .collect();
        let flips = place
            .first()
            .is_some_and(|m| DMat3::from_mat4(*m).determinant() < 0.0);

        // Vertices: one per distinct control point and corner values, per
        // material piece, in control point order.
        let mut by_material: Vec<(Option<usize>, Vec<usize>)> = Vec::new();
        for (p, &(first, _)) in polygons.iter().enumerate() {
            let s = material_layer
                .as_ref()
                .and_then(|l| l.at(p, first, corner_cp[first]))
                .and_then(|v| v.first().copied())
                .unwrap_or(0.0) as usize;
            let material = slots.get(s).or(slots.first()).copied();
            match by_material.iter_mut().find(|g| g.0 == material) {
                Some(g) => g.1.push(p),
                None => by_material.push((material, vec![p])),
            }
        }
        by_material.sort_by_key(|g| {
            g.0.map_or(usize::MAX, |m| {
                slots.iter().position(|&s| s == m).unwrap_or(0)
            })
        });
        let object = mesh.objects.len();
        let mut cp_vertices: Vec<Vec<u32>> = vec![Vec::new(); cps.len()];
        let mut used = false;
        for (material, polys) in by_material {
            // Corner keys: the control point and the bits of every corner value.
            let mut keys: HashMap<Vec<u64>, usize> = HashMap::new();
            let mut distinct: Vec<(Vec<u64>, usize, usize, usize)> = Vec::new(); // key, cp, poly, corner
            let mut corner_vertex: Vec<(usize, usize)> = Vec::new(); // (poly, distinct index) per corner
            for &p in &polys {
                let (first, n) = polygons[p];
                for (corner, &cp) in corner_cp.iter().enumerate().skip(first).take(n) {
                    let mut key = vec![cp as u64];
                    // Values that agree to `step` are one: exporters write a
                    // vertex's normal per corner, with rounding noise.
                    let mut push = |v: Option<&[f64]>, step: f64| match v {
                        Some(v) => key.extend(v.iter().map(|x| (x / step).round() as i64 as u64)),
                        None => key.push(u64::MAX),
                    };
                    push(normals.as_ref().and_then(|l| l.at(p, corner, cp)), 1e-4);
                    for (_, l) in &uv_layers {
                        push(l.at(p, corner, cp), 1e-7);
                    }
                    push(color_layer.as_ref().and_then(|l| l.at(p, corner, cp)), 1e-5);
                    let d = *keys.entry(key.clone()).or_insert_with(|| {
                        distinct.push((key, cp, p, corner));
                        distinct.len() - 1
                    });
                    corner_vertex.push((p, d));
                }
            }
            if distinct.is_empty() {
                continue;
            }
            // Control point order (then first use), so a file written one
            // vertex per control point reads back in its own order.
            let mut order: Vec<usize> = (0..distinct.len()).collect();
            order.sort_by_key(|&d| (distinct[d].1, d));
            let base = mesh.positions.len();
            let mut new_index = vec![0u32; distinct.len()];
            for (k, &d) in order.iter().enumerate() {
                new_index[d] = (base + k) as u32;
            }
            for &d in &order {
                let (_, cp, p, corner) = distinct[d];
                let m = place[cp];
                let linear = DMat3::from_mat4(m);
                mesh.positions.push(m.transform_point3(cps[cp]).as_vec3());
                match normals.as_ref().and_then(|l| l.at(p, corner, cp)) {
                    Some(n) => {
                        let n = linear.inverse().transpose() * DVec3::new(n[0], n[1], n[2]);
                        mesh.normals
                            .push(n.as_vec3().try_normalize().unwrap_or(Vec3::Y));
                        has_normal.push(true);
                    }
                    None => {
                        mesh.normals.push(Vec3::Y);
                        has_normal.push(false);
                    }
                }
                for (k, (_, l)) in uv_layers.iter().enumerate() {
                    if uvs.len() <= k {
                        uvs.push(vec![None; base]);
                    }
                    // FBX has V up from the bottom; glTF from the top.
                    let t = l
                        .at(p, corner, cp)
                        .map(|t| Vec2::new(t[0] as f32, 1.0 - t[1] as f32));
                    uvs[k].push(t);
                }
                colors.push(
                    color_layer
                        .as_ref()
                        .and_then(|l| l.at(p, corner, cp))
                        .map(|c| Vec4::new(c[0] as f32, c[1] as f32, c[2] as f32, c[3] as f32)),
                );
                let total: f64 = influences[cp].iter().map(|i| i.weight).sum();
                let mut pairs: Vec<(usize, f32)> = Vec::new();
                for i in &influences[cp] {
                    match pairs.iter_mut().find(|x| x.0 == i.joint) {
                        Some(x) => x.1 += (i.weight / total) as f32,
                        None => pairs.push((i.joint, (i.weight / total) as f32)),
                    }
                }
                skin.push(pairs);
                cp_vertices[cp].push(new_index[d]);
            }
            let total = mesh.positions.len();
            for s in &mut uvs {
                s.resize(total, None);
            }
            let first_index = mesh.indices.len();
            let mut at = 0;
            for &p in &polys {
                let (_, n) = polygons[p];
                let corners: Vec<u32> = corner_vertex[at..at + n]
                    .iter()
                    .map(|&(_, d)| new_index[d])
                    .collect();
                at += n;
                for k in 1..n.saturating_sub(1) {
                    let (a, b, c) = (corners[0], corners[k], corners[k + 1]);
                    if flips {
                        mesh.indices.extend_from_slice(&[a, c, b]);
                    } else {
                        mesh.indices.extend_from_slice(&[a, b, c]);
                    }
                }
            }
            mesh.parts.push(Part {
                object,
                material,
                first_vertex: base,
                vertex_count: total - base,
                first_index,
                index_count: mesh.indices.len() - first_index,
            });
            used = true;
        }
        if !used {
            continue;
        }
        mesh.objects.push(model.object_name());

        // Blend shapes: channels of the geometry's BlendShape deformers.
        let blends: Vec<i64> = doc
            .by_parent
            .get(&geom_id)
            .into_iter()
            .flatten()
            .map(|&i| doc.links[i].child)
            .filter(|id| {
                doc.objects
                    .get(id)
                    .is_some_and(|n| n.name == "Deformer" && n.class() == "BlendShape")
            })
            .collect();
        for blend in blends {
            for (channel, channel_id) in doc
                .children(blend)
                .filter(|(n, _)| n.name == "Deformer" && n.class() == "BlendShapeChannel")
            {
                // The full-weight shape (the last of any in-betweens).
                let Some((shape, _)) = doc
                    .children(channel_id)
                    .filter(|(n, _)| n.name == "Geometry" && n.class() == "Shape")
                    .last()
                else {
                    continue;
                };
                let name = channel.object_name();
                let weight = channel
                    .value("DeformPercent")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0) as f32
                    / 100.0;
                let idx = shape
                    .value("Indexes")
                    .and_then(Value::i64s)
                    .unwrap_or_default();
                let dv = shape
                    .value("Vertices")
                    .and_then(Value::f64s)
                    .unwrap_or_default();
                let dn = shape
                    .value("Normals")
                    .and_then(Value::f64s)
                    .unwrap_or_default();
                let slot = match shapes.iter().position(|s| s.name == name) {
                    Some(i) => i,
                    None => {
                        shapes.push(GatheredShape {
                            name,
                            offsets: HashMap::new(),
                            weight,
                            has_normals: false,
                        });
                        shapes.len() - 1
                    }
                };
                let gathered = &mut shapes[slot];
                for (k, &cp) in idx.iter().enumerate() {
                    let Some(cp) = usize::try_from(cp).ok().filter(|&c| c < cps.len()) else {
                        continue;
                    };
                    let get = |a: &[f64]| {
                        a.get(3 * k..3 * k + 3)
                            .map_or(DVec3::ZERO, |v| DVec3::new(v[0], v[1], v[2]))
                    };
                    let linear = DMat3::from_mat4(place[cp]);
                    let d = (linear * get(&dv)).as_vec3();
                    let n = (linear.inverse().transpose() * get(&dn)).as_vec3();
                    if n != Vec3::ZERO {
                        gathered.has_normals = true;
                    }
                    if d.length_squared() <= crate::shapes::EPSILON.powi(2) && n == Vec3::ZERO {
                        continue;
                    }
                    for &v in &cp_vertices[cp] {
                        gathered.offsets.insert(v, (d, n));
                    }
                }
            }
        }
    }

    if has_normal.iter().any(|h| !h) {
        let smooth = smooth_normals(&mesh.positions, &mesh.indices);
        for ((normal, smooth), given) in mesh.normals.iter_mut().zip(smooth).zip(&has_normal) {
            if !given {
                *normal = smooth;
            }
        }
    }
    mesh.uvs = uvs
        .into_iter()
        .map(|set| set.into_iter().map(|t| t.unwrap_or(Vec2::ZERO)).collect())
        .collect();
    if colors.iter().any(Option::is_some) {
        mesh.colors = colors.into_iter().map(|c| c.unwrap_or(Vec4::ONE)).collect();
    }
    mesh.shapes = shapes
        .into_iter()
        .map(|s| {
            let mut keys: Vec<u32> = s.offsets.keys().copied().collect();
            keys.sort_unstable();
            Shape {
                offsets: keys.iter().map(|k| s.offsets[k].0).collect(),
                normal_offsets: if s.has_normals {
                    keys.iter().map(|k| s.offsets[k].1).collect()
                } else {
                    Vec::new()
                },
                vertices: keys,
                name: s.name,
                weight: s.weight,
                driver: None,
                board: None,
                inbetweens: Vec::new(),
            }
        })
        .collect();
    // In-betweens hive-rig wrote as `<shape>_ib<percent>` channels.
    mesh.shapes = crate::shapes::fold_inbetweens(std::mem::take(&mut mesh.shapes));
    Ok(FbxScene { mesh, joints, skin })
}

/// The image a texture shows: the file's embedded copy (its video clip's
/// `Content`), or the file it names next to the .fbx. Shared by textures
/// naming the same file.
fn texture_image(
    doc: &Doc,
    tex: &Node,
    tex_id: i64,
    dir: Option<&Path>,
    mesh: &mut Mesh,
    seen: &mut HashMap<String, usize>,
) -> Option<usize> {
    let text = |n: &Node, k: &str| n.value(k).and_then(Value::as_str).unwrap_or("").to_string();
    let video = doc
        .children(tex_id)
        .find(|(n, _)| n.name == "Video")
        .map(|v| v.0);
    let mut relative = text(tex, "RelativeFilename");
    let mut absolute = text(tex, "FileName");
    if let Some(v) = video {
        if relative.is_empty() {
            relative = text(v, "RelativeFilename");
        }
        if absolute.is_empty() {
            absolute = text(v, "Filename");
        }
    }
    let embedded = video.and_then(|v| match v.value("Content") {
        Some(Value::Raw(b)) if !b.is_empty() => Some(b.clone()),
        _ => None,
    });
    let rel = texture_path(&relative, &absolute);
    let key = match (&rel, &embedded) {
        (Some(r), _) => r.to_string_lossy().into_owned(),
        (None, Some(_)) => format!("embedded:{tex_id}"),
        (None, None) => return None,
    };
    if let Some(&i) = seen.get(&key) {
        return Some(i);
    }
    let file = rel
        .as_ref()
        .map(|r| r.to_string_lossy().into_owned())
        .unwrap_or_default();
    let name = rel
        .as_ref()
        .and_then(|r| r.file_stem())
        .map_or_else(|| tex.object_name(), |s| s.to_string_lossy().into_owned());
    let image = match embedded {
        Some(bytes) => Image {
            name,
            uri: String::new(),
            mime: mime_for(&file).to_string(),
            data: Some(Arc::from(bytes)),
            path: None,
        },
        None => {
            let rel = rel?;
            Image {
                name,
                uri: file.clone(),
                mime: mime_for(&file).to_string(),
                data: None,
                path: Some(dir.map_or_else(|| rel.clone(), |d| d.join(&rel))),
            }
        }
    };
    seen.insert(key, mesh.images.len());
    mesh.images.push(image);
    Some(mesh.images.len() - 1)
}

#[cfg(test)]
mod tests;
