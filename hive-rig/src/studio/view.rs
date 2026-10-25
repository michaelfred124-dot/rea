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
        let forward = (self.target - self.eye()).normalize();
        let right = forward.cross(Vec3::Y).normalize_or(Vec3::X);
        Projector {
            vp,
            inv: vp.inverse(),
            rect,
            eye: self.eye(),
            forward,
            right,
            up: right.cross(forward),
            focal: rect.height().max(1.0) / (2.0 * (self.fov_y * 0.5).tan()),
        }
    }
}

pub struct Projector {
    vp: Mat4,
    inv: Mat4,
    pub rect: Rect,
    pub eye: Vec3,
    pub forward: Vec3,
    /// The camera's screen right and up, in world space.
    pub right: Vec3,
    pub up: Vec3,
    /// Pixels per world unit at one unit in front of the camera.
    focal: f32,
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

    /// Pixels per world unit at `p`'s depth (0 behind the camera), for sizing
    /// things drawn on screen at a world size.
    pub fn scale_at(&self, p: Vec3) -> f32 {
        let depth = (p - self.eye).dot(self.forward);
        if depth <= 1e-6 {
            0.0
        } else {
            self.focal / depth
        }
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
/// A bone's own colour in the all-bones weights view: hues spread by the
/// golden angle, so neighbouring bones (which come one after another in the
/// skeleton) differ clearly.
pub fn bone_color(index: usize) -> Color32 {
    let h = (index as f32 * 0.618_034).fract() * 6.0;
    let (s, v) = (0.62, 0.92);
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    let to = |t: f32| ((t + m) * 255.0).round() as u8;
    Color32::from_rgb(to(r), to(g), to(b))
}

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

/// Linear 0..1 to an sRGB byte, roughly (a 2.2 gamma is plenty for a viewport).
fn srgb(x: f32) -> f32 {
    x.clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0
}

/// What a triangle is drawn with: the texture of its piece's material (if
/// loaded) with the UV set that maps it, and the colour it's multiplied by.
#[derive(Clone, Copy)]
struct TriLook {
    texture: Option<(egui::TextureId, usize)>,
    tint: [f32; 4],
}

/// Shaded, depth-sorted triangles. `vertex_colors` overrides the colours per
/// vertex (for the weight view). Otherwise each piece takes its material's
/// colour, vertex colours and, where `textures` has its image loaded (one
/// entry per mesh image), the base colour texture; pieces without a material
/// are `base`. `alpha` below 1 lets the bones show through. `inside` are skeleton
/// pieces (sorted far to near) to sort in among the triangles, so the mesh in
/// front of a joint hides it.
#[allow(clippy::too_many_arguments)]
pub fn draw_mesh(
    painter: &egui::Painter,
    pr: &Projector,
    mesh: &Mesh,
    positions: &[Vec3],
    base: Color32,
    vertex_colors: Option<&[Color32]>,
    textures: Option<&[Option<egui::TextureId>]>,
    alpha: f32,
    inside: &[crate::joints::Prim],
) {
    let screen: Vec<Option<(Pos2, f32)>> = positions.iter().map(|&p| pr.project(p)).collect();
    // Each triangle's look, from its piece's material.
    let plain = TriLook {
        texture: None,
        tint: [1.0; 4],
    };
    let mut looks: Vec<TriLook> = Vec::new();
    if vertex_colors.is_none() && !mesh.materials.is_empty() {
        looks = vec![plain; mesh.indices.len() / 3];
        for part in mesh.pieces().iter() {
            let Some(m) = part.material.and_then(|m| mesh.materials.get(m)) else {
                continue;
            };
            let texture = m.base_color_texture.and_then(|t| {
                let id = textures?.get(t.image).copied().flatten()?;
                (t.uv_set < mesh.uvs.len()).then_some((id, t.uv_set))
            });
            let look = TriLook {
                texture,
                tint: m.base_color,
            };
            for t in part.index_range().step_by(3) {
                if let Some(l) = looks.get_mut(t / 3) {
                    *l = look;
                }
            }
        }
    }
    let mut tris: Vec<(f32, usize, [usize; 3])> = Vec::with_capacity(mesh.indices.len() / 3);
    for (k, t) in mesh.triangles().enumerate() {
        let (Some(a), Some(b), Some(c)) = (screen[t[0]], screen[t[1]], screen[t[2]]) else {
            continue;
        };
        let depth = a.1.max(b.1).max(c.1);
        if depth > 1.0 || a.1.min(b.1).min(c.1) < 0.0 {
            continue;
        }
        tris.push((depth, k, t));
    }
    // Far to near, so nearer triangles paint over farther ones.
    tris.sort_unstable_by(|a, b| b.0.total_cmp(&a.0));

    // Consecutive triangles with the same texture share one egui mesh.
    let mut out = egui::Mesh::default();
    let flush = |out: &mut egui::Mesh| {
        if !out.is_empty() {
            painter.add(Shape::mesh(std::mem::take(out)));
        }
    };
    let mut inside = inside.iter().peekable();
    for (depth, k, t) in tris {
        // Skeleton pieces farther than this triangle go first.
        if inside.peek().is_some_and(|p| p.depth > depth) {
            flush(&mut out);
            while let Some(p) = inside.next_if(|p| p.depth > depth) {
                crate::joints::paint_one(painter, p);
            }
        }
        let look = looks.get(k).copied().unwrap_or(plain);
        let texture_id = look.texture.map_or(egui::TextureId::default(), |t| t.0);
        if out.texture_id != texture_id {
            flush(&mut out);
            out.texture_id = texture_id;
        }
        let [p0, p1, p2] = t.map(|i| positions[i]);
        let n = (p1 - p0).cross(p2 - p0).normalize_or_zero();
        let to_eye = (pr.eye - (p0 + p1 + p2) / 3.0).normalize_or_zero();
        // Head light, lit from both sides so inconsistent winding still reads.
        let shade = 0.35 + 0.65 * n.dot(to_eye).abs();
        let base_index = out.vertices.len() as u32;
        for &i in &t {
            let c = match vertex_colors {
                Some(cols) => {
                    let c = cols[i];
                    [c.r() as f32, c.g() as f32, c.b() as f32]
                }
                None if looks.is_empty() => [base.r() as f32, base.g() as f32, base.b() as f32],
                None => {
                    let vc = mesh.colors.get(i).copied().unwrap_or(glam::Vec4::ONE);
                    [0, 1, 2].map(|k| srgb(look.tint[k] * vc[k]))
                }
            };
            let a = alpha * look.tint[3] * mesh.colors.get(i).map_or(1.0, |c| c.w);
            let color = Color32::from_rgba_unmultiplied(
                (c[0] * shade) as u8,
                (c[1] * shade) as u8,
                (c[2] * shade) as u8,
                (a.clamp(0.0, 1.0) * 255.0) as u8,
            );
            let uv = match look.texture {
                Some((_, set)) => {
                    let uv = mesh.uvs[set][i];
                    Pos2::new(uv.x, uv.y)
                }
                None => egui::epaint::WHITE_UV,
            };
            out.vertices.push(egui::epaint::Vertex {
                pos: screen[i].unwrap().0,
                uv,
                color,
            });
        }
        out.add_triangle(base_index, base_index + 1, base_index + 2);
    }
    flush(&mut out);
    for p in inside {
        crate::joints::paint_one(painter, p);
    }
}

/// Model-space line strips, such as a control's outline. A strip that crosses
/// behind the camera is split there rather than drawn through it.
pub fn strokes(painter: &egui::Painter, pr: &Projector, lines: &[Vec<Vec3>], stroke: Stroke) {
    for line in lines {
        let mut run: Vec<Pos2> = Vec::with_capacity(line.len());
        for &p in line {
            match pr.pos(p) {
                Some(s) => run.push(s),
                None => flush(painter, &mut run, stroke),
            }
        }
        flush(painter, &mut run, stroke);
    }
}

fn flush(painter: &egui::Painter, run: &mut Vec<Pos2>, stroke: Stroke) {
    if run.len() > 1 {
        painter.add(Shape::line(std::mem::take(run), stroke));
    }
    run.clear();
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
