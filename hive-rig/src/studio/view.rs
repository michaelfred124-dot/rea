//! The 3D viewport: an orbit camera and CPU-projected drawing with egui shapes.
//!
//! Characters are a few thousand to a few hundred thousand triangles, which egui
//! draws fine when projected and depth-sorted on the CPU, and this keeps the app
//! free of custom GPU code, so it runs on any OpenGL 2 machine.

use eframe::egui::{self, Color32, Pos2, Rect, Shape, Stroke};
use glam::camera::rh::{proj::directx, view::look_at_mat4};
use glam::{Mat4, Vec3, Vec4Swizzles, vec3};

use hive_rig::mesh::Mesh;

pub struct Camera {
    pub target: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub dist: f32,
    pub fov_y: f32,
}

impl Default for Camera {
    fn default() -> Self {
        Camera {
            target: vec3(0.0, 0.9, 0.0),
            yaw: 0.35,
            pitch: 0.15,
            dist: 3.0,
            fov_y: 40f32.to_radians(),
        }
    }
}

impl Camera {
    pub fn eye(&self) -> Vec3 {
        let (sy, cy) = self.yaw.sin_cos();
        let (sp, cp) = self.pitch.sin_cos();
        self.target + self.dist * vec3(cp * sy, sp, cp * cy)
    }

    /// Fit a bounding box in view, looking at the character's front.
    pub fn frame(&mut self, min: Vec3, max: Vec3) {
        self.target = (min + max) * 0.5;
        let radius = ((max - min).length() * 0.5).max(1e-3);
        self.dist = radius / (self.fov_y * 0.5).sin() * 1.05;
    }

    pub fn orbit(&mut self, delta: egui::Vec2) {
        self.yaw -= delta.x * 0.008;
        self.pitch = (self.pitch + delta.y * 0.008).clamp(-1.5, 1.5);
    }

    pub fn pan(&mut self, delta: egui::Vec2, viewport_height: f32) {
        let forward = (self.target - self.eye()).normalize();
        let right = forward.cross(Vec3::Y).normalize();
        let up = right.cross(forward);
        let per_pixel = 2.0 * self.dist * (self.fov_y * 0.5).tan() / viewport_height.max(1.0);
        self.target += (-right * delta.x + up * delta.y) * per_pixel;
    }

    pub fn zoom(&mut self, scroll: f32) {
        self.dist = (self.dist * (-scroll * 0.0025).exp()).clamp(0.05, 500.0);
    }

    pub fn projector(&self, rect: Rect) -> Projector {
        let aspect = rect.width().max(1.0) / rect.height().max(1.0);
        let proj = directx::perspective(self.fov_y, aspect, self.dist * 0.01, self.dist * 100.0);
        let view = look_at_mat4(self.eye(), self.target, Vec3::Y);
        let vp = proj * view;
        Projector {
            vp,
            inv: vp.inverse(),
            rect,
            eye: self.eye(),
            forward: (self.target - self.eye()).normalize(),
        }
    }
}

pub struct Projector {
    vp: Mat4,
    inv: Mat4,
    pub rect: Rect,
    pub eye: Vec3,
    pub forward: Vec3,
}

impl Projector {
    /// Screen position and depth (0 near .. 1 far), or None behind the camera.
    pub fn project(&self, p: Vec3) -> Option<(Pos2, f32)> {
        let clip = self.vp * p.extend(1.0);
        if clip.w <= 1e-6 {
            return None;
        }
        let ndc = clip.xyz() / clip.w;
        let x = self.rect.left() + (ndc.x + 1.0) * 0.5 * self.rect.width();
        let y = self.rect.top() + (1.0 - ndc.y) * 0.5 * self.rect.height();
        Some((Pos2::new(x, y), ndc.z))
    }

    pub fn pos(&self, p: Vec3) -> Option<Pos2> {
        self.project(p).map(|(s, _)| s)
    }

    /// The world-space ray under a screen position.
    pub fn ray(&self, s: Pos2) -> (Vec3, Vec3) {
        let x = (s.x - self.rect.left()) / self.rect.width() * 2.0 - 1.0;
        let y = 1.0 - (s.y - self.rect.top()) / self.rect.height() * 2.0;
        let near = self.inv.project_point3(vec3(x, y, 0.0));
        let far = self.inv.project_point3(vec3(x, y, 1.0));
        (near, (far - near).normalize())
    }

    /// Where the ray under `s` meets the plane through `p` facing the camera, so a
    /// dragged point stays at the same depth.
    pub fn drag_point(&self, s: Pos2, p: Vec3) -> Vec3 {
        let (o, d) = self.ray(s);
        let denom = d.dot(self.forward);
        if denom.abs() < 1e-6 {
            return p;
        }
        o + d * ((p - o).dot(self.forward) / denom)
    }
}

/// Weight 0..1 as a blue-green-yellow-red ramp.
pub fn weight_color(w: f32) -> Color32 {
    const STOPS: [(f32, [f32; 3]); 5] = [
        (0.0, [0.10, 0.12, 0.35]),
        (0.25, [0.10, 0.45, 0.85]),
        (0.5, [0.15, 0.80, 0.45]),
        (0.75, [0.95, 0.85, 0.15]),
        (1.0, [0.95, 0.20, 0.10]),
    ];
    let w = w.clamp(0.0, 1.0);
    let i = STOPS.iter().position(|s| s.0 >= w).unwrap_or(4).max(1);
    let (a, b) = (STOPS[i - 1], STOPS[i]);
    let t = (w - a.0) / (b.0 - a.0);
    let c: Vec<u8> = (0..3)
        .map(|k| ((a.1[k] + (b.1[k] - a.1[k]) * t) * 255.0) as u8)
        .collect();
    Color32::from_rgb(c[0], c[1], c[2])
}

/// Flat-shaded, depth-sorted triangles. `vertex_colors` overrides the base colour
/// per vertex (for the weight view); `alpha` below 1 lets the bones show through.
pub fn draw_mesh(
    painter: &egui::Painter,
    pr: &Projector,
    mesh: &Mesh,
    base: Color32,
    vertex_colors: Option<&[Color32]>,
    alpha: f32,
) {
    let screen: Vec<Option<(Pos2, f32)>> = mesh.positions.iter().map(|&p| pr.project(p)).collect();
    let mut tris: Vec<(f32, [usize; 3])> = Vec::with_capacity(mesh.indices.len() / 3);
    for t in mesh.triangles() {
        let (Some(a), Some(b), Some(c)) = (screen[t[0]], screen[t[1]], screen[t[2]]) else {
            continue;
        };
        let depth = a.1.max(b.1).max(c.1);
        if depth > 1.0 || a.1.min(b.1).min(c.1) < 0.0 {
            continue;
        }
        tris.push((depth, t));
    }
    // Far to near, so nearer triangles paint over farther ones.
    tris.sort_unstable_by(|a, b| b.0.total_cmp(&a.0));

    let mut out = egui::Mesh::default();
    out.reserve_triangles(tris.len());
    out.reserve_vertices(tris.len() * 3);
    for (_, t) in tris {
        let [p0, p1, p2] = t.map(|i| mesh.positions[i]);
        let n = (p1 - p0).cross(p2 - p0).normalize_or_zero();
        let to_eye = (pr.eye - (p0 + p1 + p2) / 3.0).normalize_or_zero();
        // Head light, lit from both sides so inconsistent winding still reads.
        let shade = 0.35 + 0.65 * n.dot(to_eye).abs();
        let base_index = out.vertices.len() as u32;
        for &i in &t {
            let c = vertex_colors.map_or(base, |cols| cols[i]);
            let c = Color32::from_rgba_unmultiplied(
                (c.r() as f32 * shade) as u8,
                (c.g() as f32 * shade) as u8,
                (c.b() as f32 * shade) as u8,
                (alpha * 255.0) as u8,
            );
            out.colored_vertex(screen[i].unwrap().0, c);
        }
        out.add_triangle(base_index, base_index + 1, base_index + 2);
    }
    painter.add(Shape::mesh(out));
}

/// A circle of `radius` around `center`, in the plane facing `axis`.
pub fn ring(
    painter: &egui::Painter,
    pr: &Projector,
    center: Vec3,
    axis: Vec3,
    radius: f32,
    stroke: Stroke,
) {
    let axis = axis.try_normalize().unwrap_or(Vec3::Y);
    let helper = if axis.y.abs() > 0.9 { Vec3::X } else { Vec3::Y };
    let u = axis.cross(helper).normalize();
    let v = axis.cross(u);
    let points: Vec<Pos2> = (0..=32)
        .filter_map(|i| {
            let a = i as f32 / 32.0 * std::f32::consts::TAU;
            pr.pos(center + (u * a.cos() + v * a.sin()) * radius)
        })
        .collect();
    if points.len() > 2 {
        painter.add(Shape::line(points, stroke));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_and_drag_round_trip() {
        let mut cam = Camera::default();
        cam.frame(vec3(-0.5, 0.0, -0.2), vec3(0.5, 1.8, 0.2));
        let pr = cam.projector(Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0)));
        let p = vec3(0.1, 1.2, 0.05);
        let (s, depth) = pr.project(p).unwrap();
        assert!(pr.rect.contains(s) && (0.0..1.0).contains(&depth));
        // Dragging to the point's own screen position leaves it where it is.
        assert!((pr.drag_point(s, p) - p).length() < 1e-3);
        // Dragging right moves it along the camera's right, at the same depth.
        let moved = pr.drag_point(s + egui::vec2(50.0, 0.0), p);
        assert!(((moved - p).dot(pr.forward)).abs() < 1e-3);
        assert!((pr.pos(moved).unwrap().x - (s.x + 50.0)).abs() < 0.5);
    }

    #[test]
    fn weight_ramp_ends() {
        assert_eq!(weight_color(0.0), Color32::from_rgb(25, 30, 89));
        assert_eq!(weight_color(1.0), Color32::from_rgb(242, 50, 25));
    }
}
