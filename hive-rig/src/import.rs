//! Mesh importers: Wavefront OBJ and glTF 2.0 (.gltf / .glb).

use std::path::Path;

use anyhow::{Context, Result, bail};
use glam::{Mat4, Vec3};

use crate::mesh::Mesh;

/// Load a mesh, picking the importer from the file extension.
pub fn load_mesh(path: &Path) -> Result<Mesh> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    let mesh = match ext.as_str() {
        "obj" => {
            let text = std::fs::read_to_string(path)
                .with_context(|| format!("reading {}", path.display()))?;
            parse_obj(&text)?
        }
        "gltf" | "glb" => load_gltf(path)?,
        other => bail!("unsupported mesh format '.{other}' (use .obj, .gltf or .glb)"),
    };
    mesh.validate()?;
    Ok(mesh)
}

/// Parse OBJ text. Polygons are fan-triangulated; groups and materials are ignored.
pub fn parse_obj(text: &str) -> Result<Mesh> {
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    for (line_no, line) in text.lines().enumerate() {
        let mut parts = line.split_whitespace();
        match parts.next() {
            Some("v") => {
                let coords: Vec<f32> = parts
                    .take(3)
                    .map(str::parse)
                    .collect::<Result<_, _>>()
                    .with_context(|| format!("bad vertex on line {}", line_no + 1))?;
                if coords.len() != 3 {
                    bail!("vertex on line {} needs 3 coordinates", line_no + 1);
                }
                positions.push(Vec3::new(coords[0], coords[1], coords[2]));
            }
            Some("f") => {
                let face: Vec<u32> = parts
                    .map(|token| {
                        let raw: i64 = token
                            .split('/')
                            .next()
                            .unwrap_or("")
                            .parse()
                            .with_context(|| format!("bad face on line {}", line_no + 1))?;
                        // OBJ indices are 1-based; negative values count from the end.
                        let idx = if raw < 0 {
                            positions.len() as i64 + raw
                        } else {
                            raw - 1
                        };
                        if idx < 0 || idx >= positions.len() as i64 {
                            bail!("face index out of range on line {}", line_no + 1);
                        }
                        Ok(idx as u32)
                    })
                    .collect::<Result<_>>()?;
                for i in 1..face.len().saturating_sub(1) {
                    indices.extend_from_slice(&[face[0], face[i], face[i + 1]]);
                }
            }
            _ => {}
        }
    }
    Ok(Mesh::new(positions, indices))
}

/// Load every mesh primitive in the default scene, baked into world space and merged.
pub fn load_gltf(path: &Path) -> Result<Mesh> {
    let (doc, buffers, _) =
        gltf::import(path).with_context(|| format!("reading {}", path.display()))?;
    let scene = doc
        .default_scene()
        .or_else(|| doc.scenes().next())
        .context("glTF file has no scene")?;
    let mut out = Mesh::default();
    for node in scene.nodes() {
        collect_node(&node, Mat4::IDENTITY, &buffers, &mut out);
    }
    if out.positions.is_empty() {
        bail!("glTF file contains no triangle meshes");
    }
    out.compute_normals();
    Ok(out)
}

fn collect_node(node: &gltf::Node, parent: Mat4, buffers: &[gltf::buffer::Data], out: &mut Mesh) {
    let world = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
    if let Some(mesh) = node.mesh() {
        for prim in mesh.primitives() {
            if prim.mode() != gltf::mesh::Mode::Triangles {
                continue;
            }
            let reader = prim.reader(|b| Some(&buffers[b.index()]));
            let Some(pos) = reader.read_positions() else {
                continue;
            };
            let base = out.positions.len() as u32;
            out.positions
                .extend(pos.map(|p| world.transform_point3(Vec3::from(p))));
            let count = out.positions.len() as u32 - base;
            match reader.read_indices() {
                Some(idx) => out.indices.extend(idx.into_u32().map(|i| i + base)),
                None => out.indices.extend(base..base + count),
            }
        }
    }
    for child in node.children() {
        collect_node(&child, world, buffers, out);
    }
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
}
