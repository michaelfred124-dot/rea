//! Joints and bones the way a rigger reads them: a shaded ball at each joint and
//! a tapered four-sided (octahedral) bone from the joint to the next, lit from the
//! camera's upper left and depth-sorted together, so a skeleton reads as solid
//! 3D at any angle. Built as a list of screen-space pieces, so the same pieces
//! are painted, sorted into the mesh (when the mesh may hide them) and picked.

use eframe::egui::{self, Color32, Pos2, Shape, Stroke, Vec2};
use glam::Vec3;

use crate::view::Projector;

/// How a bone is highlighted.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum State {
    #[default]
    Normal,
    Hover,
    Picked,
}

/// One bone to draw, from its head joint to its tail.
#[derive(Clone, Copy, Debug)]
pub struct BoneSpec {
    pub head: Vec3,
    pub tail: Vec3,
    /// World radius of the joint ball at the head.
    pub radius: f32,
    /// Draw the joint ball at the head.
    pub joint: bool,
    /// Draw a smaller ball at the tail: an end joint, with no child there.
    pub end: bool,
    pub color: Color32,
    pub state: State,
    /// 0..1; below 1 for ghosted bones.
    pub alpha: f32,
    /// Factor on the bone body's width (helper bones are slimmer).
    pub width: f32,
}

/// A piece of a drawn skeleton: one face of a bone or one joint ball, with the
/// depth it sorts at (0 near .. 1 far) and the bone it belongs to.
#[derive(Clone, Debug)]
pub struct Prim {
    pub depth: f32,
    pub owner: usize,
    kind: Kind,
}

#[derive(Clone, Debug)]
enum Kind {
    Facet {
        pts: [Pos2; 3],
        fill: Color32,
        edge: Stroke,
    },
    Ball {
        at: Pos2,
        r: f32,
        color: Color32,
        outline: Stroke,
        halo: Option<Color32>,
    },
}

/// The picked colour's outline and halo.
const PICKED_EDGE: Color32 = Color32::from_rgb(255, 196, 72);
/// Where the bone is widest, as a fraction of its length, at least.
const WAIST: f32 = 0.12;
/// End joints are this much smaller than the joint they end.
const END_BALL: f32 = 0.7;

/// The light, in the camera's screen space (x right, y up, z toward the viewer).
fn light() -> Vec3 {
    Vec3::new(-0.42, 0.58, 0.70).normalize()
}

/// Lambert plus a soft highlight; `n` and the light in the same space.
fn lit(color: Color32, n: Vec3, l: Vec3, view: Vec3, alpha: f32) -> Color32 {
    let diffuse = n.dot(l).max(0.0);
    let h = (l + view).normalize();
    let spec = n.dot(h).max(0.0).powi(28) * 0.38;
    let k = 0.42 + 0.62 * diffuse;
    let ch = |c: u8| (c as f32 * k + 255.0 * spec).clamp(0.0, 255.0) as u8;
    Color32::from_rgba_unmultiplied(
        ch(color.r()),
        ch(color.g()),
        ch(color.b()),
        (alpha.clamp(0.0, 1.0) * 255.0) as u8,
    )
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let m = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(m(a.r(), b.r()), m(a.g(), b.g()), m(a.b(), b.b()))
}

fn darker(c: Color32, k: f32, alpha: f32) -> Color32 {
    let ch = |x: u8| (x as f32 * k) as u8;
    Color32::from_rgba_unmultiplied(ch(c.r()), ch(c.g()), ch(c.b()), (alpha * 255.0) as u8)
}

/// The body colour for a highlight state.
fn body(color: Color32, state: State) -> Color32 {
    match state {
        State::Normal => color,
        State::Hover => mix(color, Color32::WHITE, 0.38),
        State::Picked => mix(color, Color32::WHITE, 0.72),
    }
}

/// The pieces of a skeleton, sorted far to near.
pub fn build(pr: &Projector, bones: &[BoneSpec]) -> Vec<Prim> {
    let l_screen = light();
    // The light in world space, fixed to the camera.
    let l_world =
        (pr.right * l_screen.x + pr.up * l_screen.y - pr.forward * l_screen.z).normalize();
    let mut out = Vec::with_capacity(bones.len() * 6);
    let mut balls: Vec<(Vec3, f32)> = Vec::with_capacity(bones.len());
    for (i, b) in bones.iter().enumerate() {
        let color = body(b.color, b.state);
        let mut ball = |at: Vec3, radius: f32, out: &mut Vec<Prim>| {
            // Joints that share a place (siblings from one parent) draw once.
            if balls
                .iter()
                .any(|(p, r)| p.distance(at) < 0.05 * radius && *r >= radius)
            {
                return;
            }
            balls.push((at, radius));
            let Some((s, _)) = pr.project(at) else { return };
            let scale = pr.scale_at(at);
            if scale <= 0.0 {
                return;
            }
            let Some((_, depth)) = pr.project(at - pr.forward * radius) else {
                return;
            };
            if !(0.0..=1.0).contains(&depth) {
                return;
            }
            let (outline, halo) = match b.state {
                State::Normal => (Stroke::new(1.2, Color32::from_black_alpha(215)), None),
                State::Hover => (Stroke::new(1.4, Color32::from_white_alpha(200)), None),
                State::Picked => (
                    Stroke::new(2.0, PICKED_EDGE),
                    Some(PICKED_EDGE.gamma_multiply(0.28)),
                ),
            };
            out.push(Prim {
                depth,
                owner: i,
                kind: Kind::Ball {
                    at: s,
                    r: (radius * scale).max(2.0),
                    color: color.gamma_multiply(b.alpha),
                    outline: Stroke::new(outline.width, outline.color.gamma_multiply(b.alpha)),
                    halo,
                },
            });
        };
        if b.joint {
            ball(b.head, b.radius, &mut out);
        }
        if b.end {
            ball(b.tail, b.radius * END_BALL, &mut out);
        }

        let d = b.tail - b.head;
        let len = d.length();
        if len < b.radius * 0.5 {
            continue;
        }
        let axis = d / len;
        let side = if axis.z.abs() < 0.9 { Vec3::Z } else { Vec3::X };
        let u = axis.cross(side).normalize();
        let v = axis.cross(u);
        // Widest just outside the joint ball, so the four faces show.
        let w = (0.11 * len).min(b.radius) * b.width;
        let waist = b.head + axis * (1.25 * b.radius).max(WAIST * len).min(0.4 * len);
        let ring = [u, v, -u, -v].map(|e| waist + e * w);
        let Some(ends) = [b.head, b.tail]
            .into_iter()
            .chain(ring)
            .map(|p| pr.project(p))
            .collect::<Option<Vec<_>>>()
        else {
            continue;
        };
        if ends.iter().any(|e| !(0.0..=1.0).contains(&e.1)) {
            continue;
        }
        let edge = match b.state {
            State::Picked => Stroke::new(1.3, PICKED_EDGE.gamma_multiply(b.alpha)),
            State::Hover => {
                Stroke::new(1.0, Color32::from_white_alpha(150).gamma_multiply(b.alpha))
            }
            State::Normal => Stroke::new(0.8, darker(color, 0.3, 0.6 * b.alpha)),
        };
        for k in 0..4 {
            let (r0, r1) = (k, (k + 1) % 4);
            // Head and tail faces, wound outward.
            for (apex, a, c) in [(0, r1, r0), (1, r0, r1)] {
                let p = [[b.head, b.tail][apex], ring[a], ring[c]];
                let mut n = (p[1] - p[0]).cross(p[2] - p[0]).normalize_or_zero();
                let centroid = (p[0] + p[1] + p[2]) / 3.0;
                // Face outward, away from the bone's axis.
                let on_axis = b.head + axis * (centroid - b.head).dot(axis);
                if n.dot(centroid - on_axis) < 0.0 {
                    n = -n;
                }
                let to_eye = (pr.eye - centroid).normalize_or_zero();
                if n.dot(to_eye) <= 0.0 {
                    continue;
                }
                let s = [ends[apex], ends[2 + a], ends[2 + c]];
                let depth = s[0].1.max(s[1].1).max(s[2].1);
                out.push(Prim {
                    depth,
                    owner: i,
                    kind: Kind::Facet {
                        pts: [s[0].0, s[1].0, s[2].0],
                        fill: lit(color, n, l_world, to_eye, b.alpha),
                        edge,
                    },
                });
            }
        }
    }
    out.sort_by(|a, b| b.depth.total_cmp(&a.depth));
    out
}

/// Paint pieces in the order given (far to near from `build`).
pub fn paint(painter: &egui::Painter, prims: &[Prim]) {
    for p in prims {
        paint_one(painter, p);
    }
}

/// Paint one piece.
pub fn paint_one(painter: &egui::Painter, p: &Prim) {
    match &p.kind {
        Kind::Facet { pts, fill, edge } => {
            painter.add(Shape::convex_polygon(pts.to_vec(), *fill, Stroke::NONE));
            // Edges as separate segments: a closed outline on a thin face would
            // spike out at its sharp corners.
            if edge.width > 0.0 {
                let [a, b, c] = *pts;
                for seg in [[a, b], [b, c], [c, a]] {
                    painter.line_segment(seg, *edge);
                }
            }
        }
        Kind::Ball {
            at,
            r,
            color,
            outline,
            halo,
        } => {
            if let Some(h) = halo {
                painter.circle_filled(*at, r + 4.0, *h);
            }
            if *r < 3.0 {
                painter.circle_filled(*at, *r, *color);
            } else {
                painter.add(Shape::mesh(ball_mesh(*at, *r, *color)));
            }
            painter.circle_stroke(*at, *r, *outline);
        }
    }
}

/// A lit sphere as a disc of rings, shaded per vertex.
fn ball_mesh(at: Pos2, r: f32, color: Color32) -> egui::Mesh {
    let l = light();
    let view = Vec3::Z;
    let alpha = color.a() as f32 / 255.0;
    // Undo the premultiplied alpha for lighting.
    let base = if color.a() == 0 {
        color
    } else {
        let un = |c: u8| ((c as f32 / alpha).min(255.0)) as u8;
        Color32::from_rgb(un(color.r()), un(color.g()), un(color.b()))
    };
    let segments = ((r * 1.4) as usize).clamp(14, 40);
    const RINGS: [f32; 5] = [0.35, 0.62, 0.82, 0.94, 1.0];
    let mut mesh = egui::Mesh::default();
    let shade = |x: f32, y: f32| {
        let z = (1.0 - x * x - y * y).max(0.0).sqrt();
        lit(base, Vec3::new(x, y, z), l, view, alpha)
    };
    mesh.colored_vertex(at, shade(0.0, 0.0));
    for &rho in &RINGS {
        for s in 0..segments {
            let a = s as f32 / segments as f32 * std::f32::consts::TAU;
            let (sin, cos) = a.sin_cos();
            // Screen y runs down; the normal's y runs up.
            let p = at + Vec2::new(cos, -sin) * (r * rho);
            mesh.colored_vertex(p, shade(cos * rho, sin * rho));
        }
    }
    let n = segments as u32;
    for s in 0..n {
        mesh.add_triangle(0, 1 + s, 1 + (s + 1) % n);
    }
    for ring in 0..RINGS.len() as u32 - 1 {
        let (a0, b0) = (1 + ring * n, 1 + (ring + 1) * n);
        for s in 0..n {
            let s1 = (s + 1) % n;
            mesh.add_triangle(a0 + s, b0 + s, b0 + s1);
            mesh.add_triangle(a0 + s, b0 + s1, a0 + s1);
        }
    }
    mesh
}

/// The bone (index into the specs) whose drawn piece is front-most under `at`,
/// among the pieces nearer than `max_depth` (where the mesh hides the rest).
pub fn hit(prims: &[Prim], at: Pos2, max_depth: f32) -> Option<usize> {
    prims
        .iter()
        .rev()
        .find(|p| p.depth <= max_depth && contains(p, at))
        .map(|p| p.owner)
}

fn contains(p: &Prim, at: Pos2) -> bool {
    match &p.kind {
        Kind::Facet { pts, .. } => {
            let [a, b, c] = *pts;
            let cross = |p: Pos2, q: Pos2| (q - p).x * (at - p).y - (q - p).y * (at - p).x;
            let (d0, d1, d2) = (cross(a, b), cross(b, c), cross(c, a));
            // Thin faces seen edge-on get a pixel of slack.
            let slack = 1.0;
            let near_edge =
                |d: f32, p: Pos2, q: Pos2| d.abs() / (q - p).length().max(1e-3) <= slack;
            (d0 >= 0.0 && d1 >= 0.0 && d2 >= 0.0)
                || (d0 <= 0.0 && d1 <= 0.0 && d2 <= 0.0)
                || (near_edge(d0, a, b) && near_edge(d1, b, c) && near_edge(d2, c, a))
        }
        Kind::Ball { at: c, r, .. } => c.distance(at) <= r + 1.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::Camera;
    use eframe::egui::{Rect, vec2};

    fn spec(head: Vec3, tail: Vec3) -> BoneSpec {
        BoneSpec {
            head,
            tail,
            radius: 0.03,
            joint: true,
            end: true,
            color: Color32::from_rgb(200, 180, 90),
            state: State::Normal,
            alpha: 1.0,
            width: 1.0,
        }
    }

    fn projector() -> Projector {
        let mut cam = Camera::default();
        cam.frame(Vec3::new(-0.5, 0.0, -0.2), Vec3::new(0.5, 1.8, 0.2));
        cam.yaw = 0.0;
        cam.pitch = 0.0;
        cam.projector(Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)))
    }

    #[test]
    fn bones_show_their_front_faces_and_balls_far_to_near() {
        let pr = projector();
        let prims = build(
            &pr,
            &[spec(Vec3::new(0.0, 1.0, 0.0), Vec3::new(0.0, 1.4, 0.0))],
        );
        let facets = prims
            .iter()
            .filter(|p| matches!(p.kind, Kind::Facet { .. }))
            .count();
        let balls = prims.len() - facets;
        // A bone seen side-on shows half its eight faces, plus head and end balls.
        assert_eq!(balls, 2);
        assert!((2..=6).contains(&facets), "{facets} faces");
        assert!(prims.windows(2).all(|w| w[0].depth >= w[1].depth));
    }

    #[test]
    fn picking_hits_the_drawn_shapes_and_the_front_one_wins() {
        let pr = projector();
        let head = Vec3::new(0.0, 1.0, 0.0);
        let tail = Vec3::new(0.0, 1.4, 0.0);
        // A second bone straight behind the first.
        let behind = Vec3::new(0.0, 0.0, -0.3);
        let bones = [spec(head + behind, tail + behind), spec(head, tail)];
        let prims = build(&pr, &bones);
        // On the joint ball, on the body at its waist, and off the bone.
        assert_eq!(hit(&prims, pr.pos(head).unwrap(), 1.0), Some(1));
        let waist = head + (tail - head) * 0.15;
        assert_eq!(hit(&prims, pr.pos(waist).unwrap(), 1.0), Some(1));
        let off = pr.pos(waist).unwrap() + vec2(30.0, 0.0);
        assert_eq!(hit(&prims, off, 1.0), None);
        // The ball reaches its drawn edge and no further.
        let r = 0.03 * pr.scale_at(head);
        assert!(r > 3.0);
        let edge = pr.pos(head).unwrap() + vec2(r - 0.5, 0.0);
        assert_eq!(hit(&prims, edge, 1.0), Some(1));
    }
}
