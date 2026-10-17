//! A procedurally built 1.8 m T-pose test character, used by the `demo` command
//! and by the tests so the pipeline can be exercised without an asset.

use glam::{Vec3, vec3};

use crate::mesh::Mesh;

/// Capped cylinder between `a` and `b` with an elliptical cross-section.
/// `radii` are along the two axes perpendicular to the cylinder.
pub fn cylinder(a: Vec3, b: Vec3, radii: (f32, f32), segments: usize, ring_spacing: f32) -> Mesh {
    prism(a, b, radii, segments, ring_spacing, 0.0)
}

fn prism(
    a: Vec3,
    b: Vec3,
    radii: (f32, f32),
    segments: usize,
    ring_spacing: f32,
    phase: f32,
) -> Mesh {
    let axis = (b - a).normalize();
    let helper = if axis.y.abs() > 0.9 { Vec3::X } else { Vec3::Y };
    let u = axis.cross(helper).normalize();
    let v = axis.cross(u).normalize();
    let rings = ((a.distance(b) / ring_spacing).ceil() as usize).max(1);
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    for r in 0..=rings {
        let c = a.lerp(b, r as f32 / rings as f32);
        for s in 0..segments {
            let t = phase + s as f32 / segments as f32 * std::f32::consts::TAU;
            positions.push(c + u * t.cos() * radii.0 + v * t.sin() * radii.1);
        }
    }
    let ring = |r: usize, s: usize| (r * segments + s % segments) as u32;
    for r in 0..rings {
        for s in 0..segments {
            indices.extend_from_slice(&[ring(r, s), ring(r + 1, s), ring(r + 1, s + 1)]);
            indices.extend_from_slice(&[ring(r, s), ring(r + 1, s + 1), ring(r, s + 1)]);
        }
    }
    for (end, r) in [(a, 0), (b, rings)] {
        let center = positions.len() as u32;
        positions.push(end);
        for s in 0..segments {
            if r == 0 {
                indices.extend_from_slice(&[center, ring(r, s + 1), ring(r, s)]);
            } else {
                indices.extend_from_slice(&[center, ring(r, s), ring(r, s + 1)]);
            }
        }
    }
    Mesh::new(positions, indices)
}

pub fn sphere(center: Vec3, radius: f32, segments: usize, rings: usize) -> Mesh {
    let mut positions = Vec::new();
    let mut indices = Vec::new();
    for r in 0..=rings {
        let phi = r as f32 / rings as f32 * std::f32::consts::PI;
        for s in 0..segments {
            let theta = s as f32 / segments as f32 * std::f32::consts::TAU;
            positions.push(
                center + radius * vec3(phi.sin() * theta.cos(), phi.cos(), phi.sin() * theta.sin()),
            );
        }
    }
    let idx = |r: usize, s: usize| (r * segments + s % segments) as u32;
    for r in 0..rings {
        for s in 0..segments {
            indices.extend_from_slice(&[idx(r, s), idx(r, s + 1), idx(r + 1, s + 1)]);
            indices.extend_from_slice(&[idx(r, s), idx(r + 1, s + 1), idx(r + 1, s)]);
        }
    }
    Mesh::new(positions, indices)
}

/// Axis-aligned box, built as a four-sided prism along Z so its faces are subdivided.
pub fn block(min: Vec3, max: Vec3) -> Mesh {
    let c = (min + max) * 0.5;
    let half = (max - min) * 0.5;
    let r = std::f32::consts::SQRT_2;
    prism(
        vec3(c.x, c.y, min.z),
        vec3(c.x, c.y, max.z),
        (half.x * r, half.y * r),
        4,
        0.03,
        std::f32::consts::FRAC_PI_4,
    )
}

/// The default demo character: T-pose.
pub fn demo_humanoid() -> Mesh {
    humanoid(0.0)
}

/// Test character with the arms lowered by `arm_drop_degrees` (45 gives an A-pose).
pub fn humanoid(arm_drop_degrees: f32) -> Mesh {
    let arm_drop = arm_drop_degrees.to_radians();
    let mut m = Mesh::default();
    let seg = 16;
    let step = 0.03;
    for sx in [1.0f32, -1.0] {
        m.append(&cylinder(
            vec3(0.1 * sx, 0.06, 0.0),
            vec3(0.1 * sx, 0.88, 0.0),
            (0.065, 0.065),
            seg,
            step,
        ));
        m.append(&block(
            vec3(0.1 * sx - 0.045, 0.0, -0.05),
            vec3(0.1 * sx + 0.045, 0.08, 0.2),
        ));
        // Arm and hand, dropped `arm_drop_degrees` below horizontal.
        let dir = vec3(sx * arm_drop.cos(), -arm_drop.sin(), 0.0);
        let shoulder = vec3(0.12 * sx, 1.40, 0.0);
        let wrist = shoulder + dir * 0.64;
        m.append(&cylinder(shoulder, wrist, (0.045, 0.045), seg, step));
        m.append(&cylinder(wrist, wrist + dir * 0.10, (0.035, 0.03), 8, step));
    }
    m.append(&cylinder(
        vec3(0.0, 0.85, 0.0),
        vec3(0.0, 1.45, 0.0),
        (0.16, 0.10),
        24,
        step,
    ));
    m.append(&cylinder(
        vec3(0.0, 1.45, 0.0),
        vec3(0.0, 1.57, 0.0),
        (0.05, 0.05),
        seg,
        step,
    ));
    m.append(&sphere(vec3(0.0, 1.68, 0.0), 0.12, seg, 12));
    m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn demo_is_valid_and_1_8m_tall() {
        let m = demo_humanoid();
        m.validate().unwrap();
        let (min, max) = m.bounds();
        assert!(min.y.abs() < 1e-5 && (max.y - 1.8).abs() < 1e-5);
        assert!((max.x - 0.86).abs() < 1e-5 && (min.x + 0.86).abs() < 1e-5);
    }
}
