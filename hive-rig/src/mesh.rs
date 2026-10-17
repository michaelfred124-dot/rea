//! Triangle mesh container shared by the importers, the rigger and the exporters.
//!
//! Convention: Y-up, metres, character facing +Z, character's left on +X
//! (the glTF convention).

use glam::Vec3;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Mesh {
    pub positions: Vec<Vec3>,
    pub normals: Vec<Vec3>,
    pub indices: Vec<u32>,
}

impl Mesh {
    pub fn new(positions: Vec<Vec3>, indices: Vec<u32>) -> Self {
        let mut mesh = Mesh {
            positions,
            normals: Vec::new(),
            indices,
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
        let mut normals = vec![Vec3::ZERO; self.positions.len()];
        for [a, b, c] in self.triangles() {
            let n = (self.positions[b] - self.positions[a])
                .cross(self.positions[c] - self.positions[a]);
            normals[a] += n;
            normals[b] += n;
            normals[c] += n;
        }
        self.normals = normals
            .into_iter()
            .map(|n| n.try_normalize().unwrap_or(Vec3::Y))
            .collect();
    }

    /// Append another mesh, offsetting its indices.
    pub fn append(&mut self, other: &Mesh) {
        let base = self.positions.len() as u32;
        self.positions.extend_from_slice(&other.positions);
        self.normals.extend_from_slice(&other.normals);
        self.indices.extend(other.indices.iter().map(|i| i + base));
    }

    /// Per-vertex neighbour lists built from triangle edges.
    pub fn adjacency(&self) -> Vec<Vec<u32>> {
        let mut adj: Vec<Vec<u32>> = vec![Vec::new(); self.positions.len()];
        for [a, b, c] in self.triangles() {
            for (i, j) in [(a, b), (b, c), (c, a)] {
                adj[i].push(j as u32);
                adj[j].push(i as u32);
            }
        }
        for list in &mut adj {
            list.sort_unstable();
            list.dedup();
        }
        adj
    }

    /// Check the mesh is usable for rigging.
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(!self.positions.is_empty(), "mesh has no vertices");
        anyhow::ensure!(
            self.indices.len() >= 3 && self.indices.len().is_multiple_of(3),
            "mesh has no triangles"
        );
        let n = self.positions.len() as u32;
        anyhow::ensure!(
            self.indices.iter().all(|&i| i < n),
            "mesh has out-of-range indices"
        );
        anyhow::ensure!(
            self.positions.iter().all(|p| p.is_finite()),
            "mesh has non-finite positions"
        );
        Ok(())
    }
}
