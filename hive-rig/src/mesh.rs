//! Triangle mesh container shared by the importers, the rigger and the exporters.
//!
//! Convention: Y-up, metres, character facing +Z, character's left on +X
//! (the glTF convention).
//!
//! The vertices of every piece of the character sit in one array, so skinning
//! weighs the whole character as one surface. What makes the pieces look the
//! way they did in the source file rides along per vertex (normals, UV sets,
//! tangents, colours) and per piece ([`Part`]: which object it belongs to and
//! its material), so the exporters can write the pieces back out. UVs use the
//! glTF convention (origin at the top left of the image); exporters to
//! programs with the origin at the bottom left flip V.

use std::borrow::Cow;
use std::collections::HashMap;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use glam::{Vec2, Vec3, Vec4};
use serde::{Deserialize, Serialize};

use crate::shapes::Shape;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Mesh {
    pub positions: Vec<Vec3>,
    /// One per vertex: the source file's normals (split at hard edges, since a
    /// hard edge is two vertices), or smooth normals when it had none.
    pub normals: Vec<Vec3>,
    pub indices: Vec<u32>,
    /// Texture coordinate sets (glTF `TEXCOORD_n`), each one per vertex.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub uvs: Vec<Vec<Vec2>>,
    /// Tangents (xyz and the bitangent sign in w), one per vertex, when every
    /// piece of the source had them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tangents: Vec<Vec4>,
    /// Linear RGBA vertex colours, one per vertex (white where a piece had none).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub colors: Vec<Vec4>,
    /// Names of the separate objects the character is made of, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub objects: Vec<String>,
    /// The pieces (one per object and material), in vertex order. Empty means
    /// one piece holding everything.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<Part>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub materials: Vec<Material>,
    /// Texture images the materials use. The files are kept next to the rig,
    /// not inside it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<Image>,
    /// Blendshapes (morph targets): the source file's and the ones made in hive-rig.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shapes: Vec<Shape>,
    /// Cone and RBF pose readers: what they read, the shapes they drive and
    /// the helper joints they move (see [`crate::readers`]).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub readers: Vec<crate::readers::Reader>,
}

/// One piece of the mesh: a run of vertices and triangles of one object with
/// one material (a glTF primitive, an OBJ object's faces of one material).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Part {
    /// Index into [`Mesh::objects`].
    pub object: usize,
    /// Index into [`Mesh::materials`], or none for the default material.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub material: Option<usize>,
    pub first_vertex: usize,
    pub vertex_count: usize,
    /// The piece's triangles: `index_count` entries of [`Mesh::indices`] from
    /// `first_index`, each naming a vertex of this piece.
    pub first_index: usize,
    pub index_count: usize,
}

impl Part {
    pub fn vertices(&self) -> Range<usize> {
        self.first_vertex..self.first_vertex + self.vertex_count
    }
    pub fn index_range(&self) -> Range<usize> {
        self.first_index..self.first_index + self.index_count
    }
}

/// How a material's alpha is used (glTF `alphaMode`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AlphaMode {
    #[default]
    Opaque,
    Mask,
    Blend,
}

/// A metal-roughness surface (the glTF material model, which Blender's
/// Principled BSDF, Maya's standardSurface and Unreal read directly).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Material {
    pub name: String,
    /// Linear RGBA multiplied with the base colour texture.
    pub base_color: [f32; 4],
    pub metallic: f32,
    pub roughness: f32,
    pub emissive: [f32; 3],
    pub alpha_mode: AlphaMode,
    pub alpha_cutoff: f32,
    pub double_sided: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_color_texture: Option<TextureRef>,
    /// Roughness in G, metalness in B.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metallic_roughness_texture: Option<TextureRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub normal_texture: Option<TextureRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub occlusion_texture: Option<TextureRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub emissive_texture: Option<TextureRef>,
}

impl Default for Material {
    /// The plain grey hive-rig gives a mesh without materials.
    fn default() -> Self {
        Material {
            name: String::new(),
            base_color: [0.8, 0.8, 0.8, 1.0],
            metallic: 0.0,
            roughness: 0.6,
            emissive: [0.0; 3],
            alpha_mode: AlphaMode::Opaque,
            alpha_cutoff: 0.5,
            double_sided: false,
            base_color_texture: None,
            metallic_roughness_texture: None,
            normal_texture: None,
            occlusion_texture: None,
            emissive_texture: None,
        }
    }
}

impl Material {
    /// Every texture the material uses, with what it's for.
    pub fn textures(&self) -> impl Iterator<Item = (&'static str, &TextureRef)> {
        [
            ("base_color", &self.base_color_texture),
            ("metallic_roughness", &self.metallic_roughness_texture),
            ("normal", &self.normal_texture),
            ("occlusion", &self.occlusion_texture),
            ("emissive", &self.emissive_texture),
        ]
        .into_iter()
        .filter_map(|(k, t)| t.as_ref().map(|t| (k, t)))
    }
}

/// A material's use of an image.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TextureRef {
    /// Index into [`Mesh::images`].
    pub image: usize,
    /// Which UV set ([`Mesh::uvs`]) maps it.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub uv_set: usize,
    /// Normal map strength or occlusion strength (1 for other textures).
    #[serde(default = "one", skip_serializing_if = "is_one")]
    pub scale: f32,
}

fn is_zero(v: &usize) -> bool {
    *v == 0
}
fn one() -> f32 {
    1.0
}
fn is_one(v: &f32) -> bool {
    *v == 1.0
}

/// A texture image. Its pixels stay in an image file: the rig file only names
/// it, relative to the rig file, so a 4K texture doesn't swell the JSON.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Image {
    pub name: String,
    /// The file, relative to the `.hive.json` once written (`<name>_textures/…`).
    pub uri: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub mime: String,
    /// The encoded file, for an image that was embedded in the source (a .glb
    /// or a data URI) and hasn't been written out yet.
    #[serde(skip)]
    pub data: Option<Arc<[u8]>>,
    /// Where the file is on disk, when known.
    #[serde(skip)]
    pub path: Option<PathBuf>,
}

impl Image {
    /// The encoded image file, from memory or disk.
    pub fn bytes(&self) -> Option<Cow<'_, [u8]>> {
        if let Some(d) = &self.data {
            return Some(Cow::Borrowed(d));
        }
        std::fs::read(self.path.as_ref()?).ok().map(Cow::Owned)
    }

    /// File extension for the image's type.
    pub fn extension(&self) -> &str {
        match self.mime.as_str() {
            "image/png" => "png",
            "image/jpeg" => "jpg",
            "image/webp" => "webp",
            _ => Path::new(&self.uri)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("png"),
        }
    }

    /// The MIME type, from the file name when the source didn't say.
    pub fn mime_type(&self) -> &str {
        if !self.mime.is_empty() {
            return &self.mime;
        }
        match self.extension().to_ascii_lowercase().as_str() {
            "jpg" | "jpeg" => "image/jpeg",
            "webp" => "image/webp",
            _ => "image/png",
        }
    }
}

impl Mesh {
    pub fn new(positions: Vec<Vec3>, indices: Vec<u32>) -> Self {
        let mut mesh = Mesh {
            positions,
            indices,
            ..Default::default()
        };
        mesh.compute_normals();
        mesh
    }

    pub fn vertex_count(&self) -> usize {
        self.positions.len()
    }

    pub fn triangles(&self) -> impl Iterator<Item = [usize; 3]> + '_ {
        self.indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|t| t.map(|i| i as usize))
    }

    pub fn bounds(&self) -> (Vec3, Vec3) {
        let mut min = Vec3::splat(f32::INFINITY);
        let mut max = Vec3::splat(f32::NEG_INFINITY);
        for p in &self.positions {
            min = min.min(*p);
            max = max.max(*p);
        }
        (min, max)
    }

    /// Area-weighted smooth vertex normals.
    pub fn compute_normals(&mut self) {
        self.normals = smooth_normals(&self.positions, &self.indices);
    }

    /// The pieces: [`Mesh::parts`], or one piece holding everything.
    pub fn pieces(&self) -> Cow<'_, [Part]> {
        if self.parts.is_empty() {
            Cow::Owned(vec![Part {
                object: 0,
                material: None,
                first_vertex: 0,
                vertex_count: self.positions.len(),
                first_index: 0,
                index_count: self.indices.len(),
            }])
        } else {
            Cow::Borrowed(&self.parts)
        }
    }

    /// Each object's name, vertex range and pieces, in order. `fallback` names
    /// the single object of a mesh that doesn't name its objects.
    pub fn object_groups(&self, fallback: &str) -> Vec<ObjectGroup> {
        let pieces = self.pieces();
        let mut out: Vec<ObjectGroup> = Vec::new();
        for p in pieces.iter() {
            match out.last_mut() {
                Some(g) if g.object == p.object => {
                    g.vertices.end = p.vertices().end;
                    g.parts.push(p.clone());
                }
                _ => out.push(ObjectGroup {
                    object: p.object,
                    name: self
                        .objects
                        .get(p.object)
                        .cloned()
                        .unwrap_or_else(|| fallback.to_string()),
                    vertices: p.vertices(),
                    parts: vec![p.clone()],
                }),
            }
        }
        out
    }

    /// Append another mesh, offsetting its indices, pieces, materials and images.
    /// Shapes of `other` move onto the appended vertices.
    pub fn append(&mut self, other: &Mesh) {
        let base = self.positions.len();
        let index_base = self.indices.len();
        let had_pieces = !self.parts.is_empty() || !other.parts.is_empty();
        let mut parts = if had_pieces {
            self.pieces().into_owned()
        } else {
            Vec::new()
        };
        let objects_base = if had_pieces {
            self.objects.len().max(1)
        } else {
            0
        };
        // Per-vertex attributes: pad whichever side lacks a set.
        let n_uv = self.uvs.len().max(other.uvs.len());
        self.uvs.resize(n_uv, vec![Vec2::ZERO; base]);
        for (k, set) in self.uvs.iter_mut().enumerate() {
            match other.uvs.get(k) {
                Some(o) => set.extend_from_slice(o),
                None => set.resize(base + other.positions.len(), Vec2::ZERO),
            }
        }
        if !self.colors.is_empty() || !other.colors.is_empty() {
            self.colors.resize(base, Vec4::ONE);
            if other.colors.is_empty() {
                self.colors.resize(base + other.positions.len(), Vec4::ONE);
            } else {
                self.colors.extend_from_slice(&other.colors);
            }
        }
        // Tangents only survive when both sides have them.
        if self.tangents.len() == base && other.tangents.len() == other.positions.len() {
            self.tangents.extend_from_slice(&other.tangents);
        } else {
            self.tangents.clear();
        }
        self.positions.extend_from_slice(&other.positions);
        self.normals.extend_from_slice(&other.normals);
        self.indices
            .extend(other.indices.iter().map(|i| i + base as u32));
        if had_pieces {
            if self.objects.is_empty() {
                self.objects.push("mesh".into());
            }
            let materials_base = self.materials.len();
            for p in other.pieces().iter() {
                parts.push(Part {
                    object: objects_base + p.object,
                    material: p.material.map(|m| m + materials_base),
                    first_vertex: p.first_vertex + base,
                    vertex_count: p.vertex_count,
                    first_index: p.first_index + index_base,
                    index_count: p.index_count,
                });
            }
            let n_other = other.parts.iter().map(|p| p.object + 1).max().unwrap_or(1);
            for k in 0..n_other {
                self.objects.push(
                    other
                        .objects
                        .get(k)
                        .cloned()
                        .unwrap_or_else(|| format!("mesh_{}", objects_base + k)),
                );
            }
            self.parts = parts;
        }
        let images_base = self.images.len();
        self.images.extend(other.images.iter().cloned());
        self.materials.extend(other.materials.iter().map(|m| {
            let mut m = m.clone();
            for t in [
                &mut m.base_color_texture,
                &mut m.metallic_roughness_texture,
                &mut m.normal_texture,
                &mut m.occlusion_texture,
                &mut m.emissive_texture,
            ]
            .into_iter()
            .flatten()
            {
                t.image += images_base;
            }
            m
        }));
        for s in &other.shapes {
            let mut s = s.clone();
            for v in s
                .vertices
                .iter_mut()
                .chain(s.inbetweens.iter_mut().flat_map(|b| &mut b.vertices))
            {
                *v += base as u32;
            }
            match self.shapes.iter_mut().find(|x| x.name == s.name) {
                Some(mine) => *mine = crate::shapes::combine(&[mine, &s], &s.name),
                None => self.shapes.push(s),
            }
        }
    }

    /// For each vertex, one index shared by every vertex at the same position
    /// (the copies a UV seam or hard edge makes), so tools can treat the
    /// surface as connected across them.
    pub fn welded(&self) -> Vec<u32> {
        let mut first: HashMap<[u32; 3], u32> = HashMap::with_capacity(self.positions.len());
        self.positions
            .iter()
            .enumerate()
            .map(|(i, p)| {
                // Bit patterns: seam copies are exact copies, and -0.0 == 0.0.
                let key = (*p + Vec3::ZERO).to_array().map(f32::to_bits);
                *first.entry(key).or_insert(i as u32)
            })
            .collect()
    }

    /// Per-vertex neighbour lists built from triangle edges. Vertices at the same
    /// position (seam copies) share their neighbours, so smoothing runs across
    /// UV seams and hard edges as if they weren't split.
    pub fn adjacency(&self) -> Vec<Vec<u32>> {
        let n = self.positions.len();
        let weld = self.welded();
        // Neighbours between welded positions.
        let mut adj: Vec<Vec<u32>> = vec![Vec::new(); n];
        for [a, b, c] in self.triangles() {
            for (i, j) in [(a, b), (b, c), (c, a)] {
                adj[weld[i] as usize].push(weld[j]);
                adj[weld[j] as usize].push(weld[i]);
            }
        }
        for list in &mut adj {
            list.sort_unstable();
            list.dedup();
        }
        if weld.iter().enumerate().all(|(i, &w)| w as usize == i) {
            return adj;
        }
        // Back to vertices: each copy lists every copy of each neighbour.
        let mut copies: Vec<Vec<u32>> = vec![Vec::new(); n];
        for (i, &w) in weld.iter().enumerate() {
            copies[w as usize].push(i as u32);
        }
        (0..n)
            .map(|v| {
                let mut list: Vec<u32> = adj[weld[v] as usize]
                    .iter()
                    .flat_map(|&r| copies[r as usize].iter().copied())
                    .collect();
                list.sort_unstable();
                list
            })
            .collect()
    }

    /// Check the mesh is usable for rigging and its pieces line up.
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(!self.positions.is_empty(), "mesh has no vertices");
        anyhow::ensure!(
            self.indices.len() >= 3 && self.indices.len().is_multiple_of(3),
            "mesh has no triangles"
        );
        let n = self.positions.len();
        anyhow::ensure!(
            self.indices.iter().all(|&i| (i as usize) < n),
            "mesh has out-of-range indices"
        );
        anyhow::ensure!(
            self.positions.iter().all(|p| p.is_finite()),
            "mesh has non-finite positions"
        );
        anyhow::ensure!(self.normals.len() == n, "mesh needs one normal per vertex");
        for (k, set) in self.uvs.iter().enumerate() {
            anyhow::ensure!(set.len() == n, "UV set {k} needs one entry per vertex");
        }
        for (what, len) in [
            ("tangents", self.tangents.len()),
            ("colours", self.colors.len()),
        ] {
            anyhow::ensure!(len == 0 || len == n, "mesh {what} need one per vertex");
        }
        if !self.parts.is_empty() {
            let (mut v, mut i, mut object) = (0, 0, 0);
            for (k, p) in self.parts.iter().enumerate() {
                anyhow::ensure!(
                    p.first_vertex == v && p.first_index == i,
                    "mesh piece {k} doesn't follow the one before it"
                );
                anyhow::ensure!(
                    p.object >= object && p.object < self.objects.len().max(1),
                    "mesh piece {k} has a bad object"
                );
                anyhow::ensure!(
                    p.material.is_none_or(|m| m < self.materials.len()),
                    "mesh piece {k} has a bad material"
                );
                anyhow::ensure!(
                    self.indices[p.index_range()]
                        .iter()
                        .all(|&x| p.vertices().contains(&(x as usize))),
                    "mesh piece {k} uses vertices of another piece"
                );
                v += p.vertex_count;
                i += p.index_count;
                object = p.object;
            }
            anyhow::ensure!(
                v == n && i == self.indices.len(),
                "mesh pieces don't cover the mesh"
            );
        }
        for m in &self.materials {
            for (_, t) in m.textures() {
                anyhow::ensure!(
                    t.image < self.images.len(),
                    "material '{}' names a missing image",
                    m.name
                );
            }
        }
        for s in &self.shapes {
            s.validate(n)?;
        }
        for r in &self.readers {
            r.validate()?;
        }
        Ok(())
    }
}

/// One object of a mesh: its name, its vertices and its pieces.
#[derive(Debug, Clone)]
pub struct ObjectGroup {
    pub object: usize,
    pub name: String,
    pub vertices: Range<usize>,
    pub parts: Vec<Part>,
}

/// Area-weighted smooth normals of a triangle list.
pub fn smooth_normals(positions: &[Vec3], indices: &[u32]) -> Vec<Vec3> {
    let mut normals = vec![Vec3::ZERO; positions.len()];
    for t in indices.as_chunks::<3>().0 {
        let [a, b, c] = t.map(|i| i as usize);
        let n = (positions[b] - positions[a]).cross(positions[c] - positions[a]);
        normals[a] += n;
        normals[b] += n;
        normals[c] += n;
    }
    normals
        .into_iter()
        .map(|n| n.try_normalize().unwrap_or(Vec3::Y))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad(x: f32) -> Mesh {
        Mesh::new(
            vec![
                Vec3::new(x, 0.0, 0.0),
                Vec3::new(x + 1.0, 0.0, 0.0),
                Vec3::new(x + 1.0, 1.0, 0.0),
                Vec3::new(x, 1.0, 0.0),
            ],
            vec![0, 1, 2, 0, 2, 3],
        )
    }

    #[test]
    fn seam_copies_share_neighbours() {
        // Two quads side by side that share an edge only by position, as a UV
        // seam leaves them.
        let mut m = quad(0.0);
        m.append(&quad(1.0));
        let adj = m.adjacency();
        // Vertex 1 (x=1, y=0) and its copy 4 have the same neighbours, which
        // reach into both quads.
        assert_eq!(adj[1], adj[4]);
        assert!(adj[1].contains(&0) && adj[1].contains(&5));
        assert!(!adj[1].contains(&1) && !adj[1].contains(&4));
        m.validate().unwrap();
    }

    #[test]
    fn append_keeps_pieces_and_materials() {
        let mut a = quad(0.0);
        a.objects = vec!["body".into()];
        a.materials = vec![Material {
            name: "skin".into(),
            base_color_texture: Some(TextureRef {
                image: 0,
                uv_set: 0,
                scale: 1.0,
            }),
            ..Default::default()
        }];
        a.images = vec![Image {
            name: "skin".into(),
            uri: "skin.png".into(),
            ..Default::default()
        }];
        a.parts = a.pieces().into_owned();
        a.parts[0].material = Some(0);
        a.uvs = vec![vec![Vec2::ZERO; 4]];
        let mut b = quad(3.0);
        b.objects = vec!["hat".into()];
        b.parts = b.pieces().into_owned();
        b.materials = vec![Material {
            name: "felt".into(),
            base_color_texture: Some(TextureRef {
                image: 0,
                uv_set: 0,
                scale: 1.0,
            }),
            ..Default::default()
        }];
        b.parts[0].material = Some(0);
        b.images = vec![Image {
            name: "felt".into(),
            uri: "felt.png".into(),
            ..Default::default()
        }];
        a.append(&b);
        a.validate().unwrap();
        let groups = a.object_groups("x");
        assert_eq!(
            groups.iter().map(|g| g.name.as_str()).collect::<Vec<_>>(),
            ["body", "hat"]
        );
        assert_eq!(groups[1].vertices, 4..8);
        assert_eq!(a.parts[1].material, Some(1));
        assert_eq!(a.materials[1].base_color_texture.unwrap().image, 1);
        assert_eq!(a.uvs[0].len(), 8);
    }
}
