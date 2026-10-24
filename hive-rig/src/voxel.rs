//! A solid voxel model of a mesh, and distances measured through it.
//!
//! The automatic skin weights (see [`crate::skin`]) need to know how far each
//! vertex is from each bone *through the body*, so an inner thigh is far from the
//! other leg and an arm hanging against the ribs is far from the spine even when
//! they nearly touch. This follows the idea of geodesic voxel binding (Dionne and
//! de Lasa, "Geodesic Voxel Binding for Production Character Meshes", SCA 2013):
//!
//! 1. Mark every voxel a triangle passes through (an exact triangle/box overlap
//!    test, so the shell has no gaps). Open edges of the mesh are first capped
//!    with a fan, so a neck hole or an eye socket doesn't let the outside in.
//! 2. Flood the outside from the edge of the grid. Whatever the flood can't reach
//!    is solid: the shell and everything it encloses. Separate pieces (eyes,
//!    teeth, clothes) that overlap the body merge into one solid; nested shells
//!    and non-manifold geometry need no special handling.
//! 3. For a bone, seed the solid voxels its segment passes through and grow
//!    shortest paths through the solid (26 neighbours, Dijkstra). A separate
//!    solid piece belongs to the bones that pass through it (an eyeball to its
//!    eye bone, a robot's forearm plate to the forearm); a piece no bone passes
//!    through (teeth, a loose button) takes the straight-line distance to every
//!    bone, so it goes to the bones nearest it.
//!
//! Nothing here knows about bodies, regions or species: the same steps work on a
//! biped, a dog, a spider or a snake.

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};

use glam::Vec3;

use crate::mesh::Mesh;

const OUTSIDE: u32 = u32::MAX;

/// Distance from `p` to the segment `a`-`b`.
pub fn segment_distance(p: Vec3, a: Vec3, b: Vec3) -> f32 {
    let ab = b - a;
    let len2 = ab.length_squared();
    let t = if len2 > 0.0 {
        ((p - a).dot(ab) / len2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    p.distance(a + ab * t)
}

/// The solid voxels of a mesh.
pub struct Volume {
    origin: Vec3,
    /// Edge length of one voxel.
    h: f32,
    dims: [usize; 3],
    /// Dense grid: the solid index of each voxel, or [`OUTSIDE`].
    index: Vec<u32>,
    /// Each solid voxel's dense grid index.
    cells: Vec<u32>,
    /// Each solid voxel's connected piece (26-connected).
    piece: Vec<u32>,
}

impl Volume {
    /// Voxelize `mesh` with `resolution` voxels along its longest side.
    pub fn new(mesh: &Mesh, resolution: usize) -> Volume {
        let (min, max) = mesh.bounds();
        let extent = (max - min).max_element().max(1e-6);
        let h = extent / resolution.max(8) as f32;
        // Two voxels of air all round, so the flood can go round everything and
        // every solid voxel has all 26 neighbours inside the grid.
        let pad = 2.0 * h;
        let origin = min - Vec3::splat(pad);
        let size = (max + Vec3::splat(pad)) - origin;
        let dims = [
            (size.x / h).ceil() as usize + 1,
            (size.y / h).ceil() as usize + 1,
            (size.z / h).ceil() as usize + 1,
        ];
        let mut vol = Volume {
            origin,
            h,
            dims,
            index: Vec::new(),
            cells: Vec::new(),
            piece: Vec::new(),
        };
        let mut shell = vec![false; dims[0] * dims[1] * dims[2]];
        let tris: Vec<[Vec3; 3]> = mesh
            .triangles()
            .map(|t| t.map(|i| mesh.positions[i]))
            .chain(caps(mesh))
            .collect();
        for tri in &tris {
            vol.mark_triangle(tri, &mut shell);
        }
        let outside = vol.flood_outside(&shell);
        vol.index = vec![OUTSIDE; shell.len()];
        for (i, out) in outside.iter().enumerate() {
            if !out {
                vol.index[i] = vol.cells.len() as u32;
                vol.cells.push(i as u32);
            }
        }
        vol.piece = vol.label_pieces();
        vol
    }

    pub fn voxel_size(&self) -> f32 {
        self.h
    }

    pub fn solid_count(&self) -> usize {
        self.cells.len()
    }

    fn dense(&self, x: usize, y: usize, z: usize) -> usize {
        (z * self.dims[1] + y) * self.dims[0] + x
    }

    fn coords(&self, dense: usize) -> [usize; 3] {
        let x = dense % self.dims[0];
        let y = (dense / self.dims[0]) % self.dims[1];
        let z = dense / (self.dims[0] * self.dims[1]);
        [x, y, z]
    }

    fn key(&self, p: Vec3) -> Option<[usize; 3]> {
        let k = ((p - self.origin) / self.h).floor();
        let in_grid = k.x >= 0.0
            && k.y >= 0.0
            && k.z >= 0.0
            && (k.x as usize) < self.dims[0]
            && (k.y as usize) < self.dims[1]
            && (k.z as usize) < self.dims[2];
        in_grid.then_some([k.x as usize, k.y as usize, k.z as usize])
    }

    fn center_of(&self, dense: usize) -> Vec3 {
        let [x, y, z] = self.coords(dense);
        self.origin + (Vec3::new(x as f32, y as f32, z as f32) + 0.5) * self.h
    }

    /// The centre of solid voxel `cell`.
    pub fn center(&self, cell: u32) -> Vec3 {
        self.center_of(self.cells[cell as usize] as usize)
    }

    /// The solid voxel holding `p`, if it is solid.
    pub fn cell_of(&self, p: Vec3) -> Option<u32> {
        let [x, y, z] = self.key(p)?;
        let i = self.index[self.dense(x, y, z)];
        (i != OUTSIDE).then_some(i)
    }

    /// The solid voxel for a mesh vertex at `p`: the one holding it, or else the
    /// solid neighbour whose centre is nearest (a vertex used by no triangle).
    pub fn cell_near(&self, p: Vec3) -> Option<u32> {
        if let Some(c) = self.cell_of(p) {
            return Some(c);
        }
        let [x, y, z] = self.key(p)?;
        let mut best: Option<(f32, u32)> = None;
        for dz in 0..3usize {
            for dy in 0..3usize {
                for dx in 0..3usize {
                    let (Some(nx), Some(ny), Some(nz)) = (
                        (x + dx).checked_sub(1),
                        (y + dy).checked_sub(1),
                        (z + dz).checked_sub(1),
                    ) else {
                        continue;
                    };
                    if nx >= self.dims[0] || ny >= self.dims[1] || nz >= self.dims[2] {
                        continue;
                    }
                    let d = self.dense(nx, ny, nz);
                    let i = self.index[d];
                    let dist = self.center_of(d).distance(p);
                    if i != OUTSIDE && best.is_none_or(|b| dist < b.0) {
                        best = Some((dist, i));
                    }
                }
            }
        }
        best.map(|b| b.1)
    }

    /// Mark the voxels triangle `tri` overlaps.
    fn mark_triangle(&self, tri: &[Vec3; 3], shell: &mut [bool]) {
        let lo = tri[0].min(tri[1]).min(tri[2]);
        let hi = tri[0].max(tri[1]).max(tri[2]);
        let (Some(a), Some(b)) = (self.key(lo), self.key(hi)) else {
            return;
        };
        // A hair larger than the voxel, so a vertex exactly on a voxel face
        // marks the voxel it is filed under.
        let half = Vec3::splat(self.h * 0.501);
        for z in a[2]..=b[2] {
            for y in a[1]..=b[1] {
                for x in a[0]..=b[0] {
                    let d = self.dense(x, y, z);
                    if shell[d] {
                        continue;
                    }
                    let c = self.center_of(d);
                    if triangle_box_overlap(tri, c, half) {
                        shell[d] = true;
                    }
                }
            }
        }
    }

    /// The voxels reachable from the grid's edge without crossing the shell
    /// (6-connected, which a gap-free shell can't leak through).
    fn flood_outside(&self, shell: &[bool]) -> Vec<bool> {
        let mut out = vec![false; shell.len()];
        let mut stack = vec![0usize];
        out[0] = true;
        let [nx, ny, nz] = self.dims;
        while let Some(d) = stack.pop() {
            let [x, y, z] = self.coords(d);
            let mut visit = |x: usize, y: usize, z: usize, stack: &mut Vec<usize>| {
                let n = self.dense(x, y, z);
                if !out[n] && !shell[n] {
                    out[n] = true;
                    stack.push(n);
                }
            };
            if x > 0 {
                visit(x - 1, y, z, &mut stack);
            }
            if x + 1 < nx {
                visit(x + 1, y, z, &mut stack);
            }
            if y > 0 {
                visit(x, y - 1, z, &mut stack);
            }
            if y + 1 < ny {
                visit(x, y + 1, z, &mut stack);
            }
            if z > 0 {
                visit(x, y, z - 1, &mut stack);
            }
            if z + 1 < nz {
                visit(x, y, z + 1, &mut stack);
            }
        }
        out
    }

    /// The 26 neighbour offsets in the dense grid, with their lengths.
    fn neighbours(&self) -> Vec<(isize, f32)> {
        let (sx, sy) = (1isize, self.dims[0] as isize);
        let sz = sy * self.dims[1] as isize;
        let mut out = Vec::with_capacity(26);
        for dz in -1isize..=1 {
            for dy in -1isize..=1 {
                for dx in -1isize..=1 {
                    if dx == 0 && dy == 0 && dz == 0 {
                        continue;
                    }
                    let len = ((dx * dx + dy * dy + dz * dz) as f32).sqrt() * self.h;
                    out.push((dx * sx + dy * sy + dz * sz, len));
                }
            }
        }
        out
    }

    fn solid_neighbour(&self, cell: usize, offset: isize) -> Option<usize> {
        // Solid voxels are at least two voxels from the grid's edge.
        let d = (self.cells[cell] as isize + offset) as usize;
        let i = self.index[d];
        (i != OUTSIDE).then_some(i as usize)
    }

    fn label_pieces(&self) -> Vec<u32> {
        let offsets = self.neighbours();
        let mut piece = vec![u32::MAX; self.cells.len()];
        let mut next = 0u32;
        let mut stack = Vec::new();
        for start in 0..self.cells.len() {
            if piece[start] != u32::MAX {
                continue;
            }
            piece[start] = next;
            stack.push(start);
            while let Some(c) = stack.pop() {
                for &(o, _) in &offsets {
                    if let Some(n) = self.solid_neighbour(c, o)
                        && piece[n] == u32::MAX
                    {
                        piece[n] = next;
                        stack.push(n);
                    }
                }
            }
            next += 1;
        }
        piece
    }

    /// How many separate solid pieces the mesh makes.
    pub fn piece_count(&self) -> usize {
        self.piece.iter().max().map_or(0, |m| *m as usize + 1)
    }

    /// The piece solid voxel `cell` belongs to.
    pub fn piece_of(&self, cell: u32) -> usize {
        self.piece[cell as usize] as usize
    }

    /// Where the segment `a`-`b` starts its paths: the solid voxels it passes
    /// through, each at its distance from the segment. A segment entirely outside
    /// the mesh starts from the solid voxels nearest it.
    pub fn seeds(&self, a: Vec3, b: Vec3) -> Vec<(u32, f32)> {
        let mut out: HashMap<u32, f32> = HashMap::new();
        let steps = ((a.distance(b) / (self.h * 0.25)).ceil() as usize).max(1);
        for s in 0..=steps {
            let p = a.lerp(b, s as f32 / steps as f32);
            if let Some(c) = self.cell_of(p) {
                out.entry(c)
                    .or_insert_with(|| segment_distance(self.center(c), a, b));
            }
        }
        if out.is_empty() {
            let near: Vec<f32> = (0..self.cells.len())
                .map(|c| segment_distance(self.center(c as u32), a, b))
                .collect();
            let best = near.iter().copied().fold(f32::INFINITY, f32::min);
            for (c, d) in near.into_iter().enumerate() {
                if d <= best + self.h {
                    out.insert(c as u32, d);
                }
            }
        }
        let mut out: Vec<(u32, f32)> = out.into_iter().collect();
        out.sort_unstable_by_key(|s| s.0);
        out
    }

    /// Distance from the segment `a`-`b` to every solid voxel, through the solid,
    /// starting from its `seeds`. A piece the segment doesn't reach keeps an
    /// infinite distance (it belongs to the bones inside it), unless `orphan`
    /// marks it as reached by no bone at all: then it takes the straight-line
    /// distance, so a loose piece goes to the bones nearest it. `dist` is reused
    /// scratch space and comes back holding the result.
    pub fn distances(
        &self,
        a: Vec3,
        b: Vec3,
        seeds: &[(u32, f32)],
        orphan: &[bool],
        dist: &mut Vec<f32>,
    ) {
        dist.clear();
        dist.resize(self.cells.len(), f32::INFINITY);
        let offsets = self.neighbours();
        let mut heap: BinaryHeap<Reverse<(u32, u32)>> = BinaryHeap::new();
        for &(c, d) in seeds {
            if d < dist[c as usize] {
                dist[c as usize] = d;
                heap.push(Reverse((d.to_bits(), c)));
            }
        }
        while let Some(Reverse((bits, c))) = heap.pop() {
            let d = f32::from_bits(bits);
            let c = c as usize;
            if d > dist[c] {
                continue;
            }
            for &(o, len) in &offsets {
                if let Some(n) = self.solid_neighbour(c, o) {
                    let nd = d + len;
                    if nd < dist[n] {
                        dist[n] = nd;
                        heap.push(Reverse((nd.to_bits(), n as u32)));
                    }
                }
            }
        }
        // In a piece no bone reaches the straight line is the shortest path (a
        // path through the piece can't be shorter).
        if orphan.iter().any(|o| *o) {
            for (c, d) in dist.iter_mut().enumerate() {
                if orphan.get(self.piece[c] as usize).copied().unwrap_or(false) {
                    *d = segment_distance(self.center(c as u32), a, b);
                }
            }
        }
    }
}

/// Fans that close each open edge loop of the mesh, so holes (a neck opening, eye
/// sockets, the end of a sleeve) don't let the outside flood into the body.
/// Vertices at the same position count as one, so UV seams aren't holes.
fn caps(mesh: &Mesh) -> Vec<[Vec3; 3]> {
    let weld = weld_positions(&mesh.positions);
    let mut edges: HashMap<(u32, u32), u32> = HashMap::new();
    for [a, b, c] in mesh.triangles() {
        let t = [weld[a], weld[b], weld[c]];
        for (i, j) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
            if i == j {
                continue;
            }
            *edges.entry((i.min(j), i.max(j))).or_insert(0) += 1;
        }
    }
    let open: Vec<(u32, u32)> = edges
        .iter()
        .filter(|(_, n)| **n == 1)
        .map(|(k, _)| *k)
        .collect();
    if open.is_empty() {
        return Vec::new();
    }
    // Group the open edges into connected loops (union-find over their ends).
    let mut parent: HashMap<u32, u32> = HashMap::new();
    fn find(parent: &mut HashMap<u32, u32>, v: u32) -> u32 {
        let mut r = v;
        while let Some(&p) = parent.get(&r) {
            if p == r {
                break;
            }
            r = p;
        }
        let mut at = v;
        while at != r {
            let next = parent[&at];
            parent.insert(at, r);
            at = next;
        }
        r
    }
    for &(a, b) in &open {
        parent.entry(a).or_insert(a);
        parent.entry(b).or_insert(b);
        let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
        if ra != rb {
            parent.insert(ra, rb);
        }
    }
    let mut loops: HashMap<u32, Vec<(u32, u32)>> = HashMap::new();
    for &(a, b) in &open {
        let r = find(&mut parent, a);
        loops.entry(r).or_default().push((a, b));
    }
    let mut out = Vec::new();
    for edges in loops.into_values() {
        let mut verts: Vec<u32> = edges.iter().flat_map(|&(a, b)| [a, b]).collect();
        verts.sort_unstable();
        verts.dedup();
        let center = verts
            .iter()
            .map(|&v| mesh.positions[v as usize])
            .sum::<Vec3>()
            / verts.len() as f32;
        for (a, b) in edges {
            out.push([
                mesh.positions[a as usize],
                mesh.positions[b as usize],
                center,
            ]);
        }
    }
    out
}

/// For each vertex, the first vertex at exactly the same position.
pub fn weld_positions(positions: &[Vec3]) -> Vec<u32> {
    let mut first: HashMap<[u32; 3], u32> = HashMap::with_capacity(positions.len());
    positions
        .iter()
        .enumerate()
        .map(|(i, p)| {
            // -0.0 and 0.0 are the same place.
            let key = (*p + Vec3::ZERO).to_array().map(f32::to_bits);
            *first.entry(key).or_insert(i as u32)
        })
        .collect()
}

/// Whether triangle `tri` overlaps the axis-aligned box at `center` with half
/// extents `half`, by the separating axis test: the box's three axes, the
/// triangle's normal, and the nine cross products of their edges.
fn triangle_box_overlap(tri: &[Vec3; 3], center: Vec3, half: Vec3) -> bool {
    let v = [tri[0] - center, tri[1] - center, tri[2] - center];
    let e = [v[1] - v[0], v[2] - v[1], v[0] - v[2]];
    // The box's axes.
    for k in 0..3 {
        let (lo, hi) = (
            v[0][k].min(v[1][k]).min(v[2][k]),
            v[0][k].max(v[1][k]).max(v[2][k]),
        );
        if lo > half[k] || hi < -half[k] {
            return false;
        }
    }
    // Edge cross products.
    for edge in e {
        for axis in [Vec3::X, Vec3::Y, Vec3::Z] {
            let a = axis.cross(edge);
            if a.length_squared() < 1e-20 {
                continue;
            }
            let p = [a.dot(v[0]), a.dot(v[1]), a.dot(v[2])];
            let r = half.x * a.x.abs() + half.y * a.y.abs() + half.z * a.z.abs();
            let (lo, hi) = (p[0].min(p[1]).min(p[2]), p[0].max(p[1]).max(p[2]));
            if lo > r || hi < -r {
                return false;
            }
        }
    }
    // The triangle's plane.
    let n = e[0].cross(e[1]);
    let r = half.x * n.x.abs() + half.y * n.y.abs() + half.z * n.z.abs();
    n.dot(v[0]).abs() <= r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;

    #[test]
    fn a_closed_shell_is_solid_inside() {
        let ball = demo::sphere(Vec3::ZERO, 1.0, 24, 16);
        let vol = Volume::new(&ball, 32);
        assert!(vol.cell_of(Vec3::ZERO).is_some(), "the centre is solid");
        assert!(vol.cell_of(Vec3::new(0.0, 0.5, 0.3)).is_some());
        assert!(vol.cell_of(Vec3::splat(0.95)).is_none(), "outside the ball");
        // Roughly the ball's volume.
        let v = vol.solid_count() as f32 * vol.voxel_size().powi(3);
        let exact = 4.0 / 3.0 * std::f32::consts::PI;
        assert!((v - exact).abs() < 0.3 * exact, "{v} vs {exact}");
    }

    #[test]
    fn a_hole_is_capped() {
        // A ball with its top cap of triangles removed is still solid inside.
        let mut ball = demo::sphere(Vec3::ZERO, 1.0, 24, 16);
        let keep: Vec<u32> = ball
            .indices
            .chunks(3)
            .filter(|t| t.iter().all(|&i| ball.positions[i as usize].y < 0.8))
            .flatten()
            .copied()
            .collect();
        ball.indices = keep;
        let vol = Volume::new(&ball, 32);
        assert!(vol.cell_of(Vec3::ZERO).is_some());
    }

    #[test]
    fn distance_goes_round_a_gap_not_across_it() {
        // Two parallel bars joined only at one end, like legs at the crotch.
        let mut mesh = demo::block(Vec3::new(-0.6, 0.0, -0.1), Vec3::new(-0.4, 2.0, 0.1));
        mesh.append(&demo::block(
            Vec3::new(0.4, 0.0, -0.1),
            Vec3::new(0.6, 2.0, 0.1),
        ));
        mesh.append(&demo::block(
            Vec3::new(-0.6, 1.8, -0.1),
            Vec3::new(0.6, 2.0, 0.1),
        ));
        let vol = Volume::new(&mesh, 64);
        let mut dist = Vec::new();
        // A bone down the left bar.
        let (a, b) = (Vec3::new(-0.5, 0.1, 0.0), Vec3::new(-0.5, 1.7, 0.0));
        vol.distances(a, b, &vol.seeds(a, b), &[], &mut dist);
        let at = |p: Vec3| dist[vol.cell_of(p).unwrap() as usize];
        // The bottom of the right bar is 1 m away in a straight line but has
        // to go up and over: about 0.9 + 1 + 1.7 m.
        let far = at(Vec3::new(0.5, 0.1, 0.0));
        assert!(far > 2.4, "{far}");
        assert!(at(Vec3::new(-0.45, 1.0, 0.0)) < 0.1);
    }

    #[test]
    fn a_loose_piece_belongs_to_its_own_bones() {
        let mut mesh = demo::block(Vec3::new(-0.1, 0.0, -0.1), Vec3::new(0.1, 1.0, 0.1));
        mesh.append(&demo::sphere(Vec3::new(0.6, 0.5, 0.0), 0.1, 12, 8));
        let vol = Volume::new(&mesh, 64);
        let mut dist = Vec::new();
        let (a, b) = (Vec3::new(0.0, 0.1, 0.0), Vec3::new(0.0, 0.9, 0.0));
        let ball = vol.cell_of(Vec3::new(0.6, 0.5, 0.0)).unwrap();
        let seeds = vol.seeds(a, b);
        assert!(
            seeds
                .iter()
                .all(|s| vol.piece_of(s.0) != vol.piece_of(ball))
        );
        // While no bone reaches the ball it takes the straight line...
        let mut orphan = vec![false; vol.piece_count()];
        orphan[vol.piece_of(ball)] = true;
        vol.distances(a, b, &seeds, &orphan, &mut dist);
        let d = dist[ball as usize];
        assert!((d - 0.6).abs() < 0.05, "{d}");
        // ...and once a bone of its own does, the bar's bone keeps out of it.
        vol.distances(a, b, &seeds, &[], &mut dist);
        assert!(dist[ball as usize].is_infinite());
    }
}
