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

/// Height of the demo dog, floor to the top of the head.
pub const DOG_HEIGHT: f32 = 0.8;

/// A procedural dog standing on all fours, facing +Z, built around the default
/// quadruped guides (`components::quadruped(DOG_HEIGHT)`) so every bone sits
/// inside the body: a barrel torso, neck, head and muzzle, tail and four legs.
pub fn demo_quadruped() -> Mesh {
    let g = crate::components::quadruped(DOG_HEIGHT);
    let h = DOG_HEIGHT;
    let guide = |comp: &str, name: &str| g.component(comp).unwrap().position(name).unwrap();
    let mut m = Mesh::default();
    let step = 0.02;
    let pelvis = guide("spine", "pelvis");
    let chest = guide("spine", "chest");
    let back = vec3(0.0, pelvis.y + 0.02 * h, pelvis.z - 0.1 * h);
    let front = vec3(0.0, chest.y, chest.z + 0.1 * h);
    m.append(&cylinder(back, front, (0.11 * h, 0.12 * h), 20, step));
    let head = guide("head", "head");
    m.append(&cylinder(
        guide("head", "neck"),
        head,
        (0.06 * h, 0.06 * h),
        14,
        step,
    ));
    m.append(&sphere(head, 0.09 * h, 16, 10));
    m.append(&cylinder(
        head,
        guide("head", "head_top"),
        (0.045 * h, 0.04 * h),
        12,
        step,
    ));
    let tail = g.component("tail").unwrap();
    m.append(&cylinder(
        tail.guides[0].position,
        tail.guides.last().unwrap().position,
        (0.025 * h, 0.025 * h),
        10,
        step,
    ));
    for s in ["_l", "_r"] {
        let arm = format!("front_leg{s}");
        let leg = format!("hind_leg{s}");
        let limbs: [(&str, &[&str], f32); 2] = [
            (&arm, &["shoulder", "elbow", "wrist", "hand_tip"], 0.045),
            (&leg, &["hip", "knee", "hock", "ankle", "toe"], 0.05),
        ];
        for (comp, points, r) in limbs {
            for pair in points.windows(2) {
                let (a, b) = (guide(comp, pair[0]), guide(comp, pair[1]));
                m.append(&cylinder(a, b, (r * h, r * h), 12, step));
            }
        }
    }
    m
}

/// Height of the demo bird, floor to the top of the head.
pub const BIRD_HEIGHT: f32 = 0.5;
/// Height of the demo spider, floor to the top of its knees.
pub const SPIDER_HEIGHT: f32 = 0.4;
/// Size unit of the demo snake, which is about four of them long.
pub const SNAKE_HEIGHT: f32 = 0.5;

/// The built-in creatures: a layout (`components::quadruped`, `bird`, `spider`
/// or `snake`) and a procedural demo body built around its guides.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Creature {
    /// Spine, neck and head, tail, two front legs and two three-segment hind legs.
    Dog,
    /// Wings (arm components), two legs with hocks, a neck, head and tail.
    Bird,
    /// A head, an abdomen and eight legs (FK chains).
    Spider,
    /// A spline spine with a raised head and a spline tail.
    Snake,
}

impl Creature {
    pub const ALL: [Creature; 4] = [
        Creature::Dog,
        Creature::Bird,
        Creature::Spider,
        Creature::Snake,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Creature::Dog => "dog",
            Creature::Bird => "bird",
            Creature::Spider => "spider",
            Creature::Snake => "snake",
        }
    }

    pub fn from_name(name: &str) -> Option<Creature> {
        Creature::ALL.into_iter().find(|c| c.name() == name)
    }

    /// What the creature's layout holds, for template lists.
    pub fn description(self) -> &'static str {
        match self {
            Creature::Dog => "Four legs with three-segment hind legs, a neck, head and tail.",
            Creature::Bird => {
                "Wings with IK/FK like arms, two legs with hocks, a neck, head and tail."
            }
            Creature::Spider => "Eight four-bone FK legs, a head and an abdomen.",
            Creature::Snake => "A spline spine and spline tail with a raised neck and head.",
        }
    }

    /// The demo creature's size: floor to the top of the head (the spider's
    /// knees), or the snake's size unit.
    pub fn height(self) -> f32 {
        match self {
            Creature::Dog => DOG_HEIGHT,
            Creature::Bird => BIRD_HEIGHT,
            Creature::Spider => SPIDER_HEIGHT,
            Creature::Snake => SNAKE_HEIGHT,
        }
    }

    /// The creature's default layout, at the demo size.
    pub fn guides(self) -> crate::components::Guides {
        let h = self.height();
        match self {
            Creature::Dog => crate::components::quadruped(h),
            Creature::Bird => crate::components::bird(h),
            Creature::Spider => crate::components::spider(h),
            Creature::Snake => crate::components::snake(h),
        }
    }

    /// The demo body, built around [`Creature::guides`].
    pub fn mesh(self) -> Mesh {
        match self {
            Creature::Dog => demo_quadruped(),
            Creature::Bird => demo_bird(),
            Creature::Spider => demo_spider(),
            Creature::Snake => demo_snake(),
        }
    }

    /// The name its demo rig is written as: `hive_dog`, `hive_bird`, ...
    pub fn rig_name(self) -> String {
        format!("hive_{}", self.name())
    }
}

/// Sphere scaled to an ellipsoid with semi-axes `radii`.
fn ellipsoid(center: Vec3, radii: Vec3) -> Mesh {
    let mut m = sphere(Vec3::ZERO, 1.0, 16, 10);
    for p in &mut m.positions {
        *p = center + *p * radii;
    }
    m
}

/// A round tube through `points`, its radius going from `radii[i]` at each
/// point, with a ball at every bend so the joints stay filled.
fn tube(m: &mut Mesh, points: &[Vec3], radii: &[f32], step: f32) {
    for (i, pair) in points.windows(2).enumerate() {
        let r = (radii[i] + radii[i + 1]) * 0.5;
        m.append(&cylinder(pair[0], pair[1], (r, r), 12, step));
        if i > 0 {
            m.append(&sphere(pair[0], radii[i], 10, 6));
        }
    }
}

/// The guides of one component of `g`, in order.
fn points(g: &crate::components::Guides, comp: &str, names: &[&str]) -> Vec<Vec3> {
    let c = g.component(comp).unwrap();
    if names.is_empty() {
        return c.guides.iter().map(|x| x.position).collect();
    }
    names.iter().map(|n| c.position(n).unwrap()).collect()
}

/// A procedural bird standing on two legs, facing +Z, built around
/// `components::bird(BIRD_HEIGHT)`: an egg-shaped body, a neck and round head
/// with a beak, flat wings spread to the sides, a fanned tail and thin legs.
pub fn demo_bird() -> Mesh {
    let h = BIRD_HEIGHT;
    let g = crate::components::bird(h);
    let mut m = Mesh::default();
    let step = 0.015;
    let [pelvis, chest] = points(&g, "spine", &[]).try_into().unwrap();
    let along = (chest - pelvis).normalize();
    m.append(&cylinder(
        pelvis - along * 0.1 * h,
        chest + along * 0.06 * h,
        (0.1 * h, 0.11 * h),
        20,
        step,
    ));
    m.append(&ellipsoid(chest, Vec3::new(0.1, 0.1, 0.08) * h));
    let [neck, head, beak] = points(&g, "head", &[]).try_into().unwrap();
    tube(
        &mut m,
        &[chest, neck, head],
        &[0.07 * h, 0.05 * h, 0.045 * h],
        step,
    );
    m.append(&sphere(head, 0.07 * h, 16, 10));
    m.append(&cylinder(head, beak, (0.025 * h, 0.02 * h), 10, step));
    let tail = points(&g, "tail", &[]);
    m.append(&cylinder(
        tail[0],
        *tail.last().unwrap(),
        (0.07 * h, 0.015 * h),
        12,
        step,
    ));
    for s in ["_l", "_r"] {
        let wing = points(
            &g,
            &format!("wing{s}"),
            &["clavicle", "shoulder", "elbow", "wrist", "hand_tip"],
        );
        // (chord, thickness) of each segment: the hand carries the widest feathers.
        let shapes = [(0.03, 0.03), (0.06, 0.02), (0.07, 0.018), (0.08, 0.012)];
        for (pair, (chord, thick)) in wing.windows(2).zip(shapes) {
            m.append(&cylinder(
                pair[0],
                pair[1],
                (chord * h, thick * h),
                12,
                step,
            ));
        }
        let leg = format!("leg{s}");
        tube(
            &mut m,
            &points(&g, &leg, &["hip", "knee", "hock", "ankle", "ball", "toe"]),
            &[
                0.03 * h,
                0.028 * h,
                0.018 * h,
                0.014 * h,
                0.012 * h,
                0.01 * h,
            ],
            step,
        );
        let [ankle, heel] = points(&g, &leg, &["ankle", "heel"]).try_into().unwrap();
        m.append(&cylinder(ankle, heel, (0.01 * h, 0.01 * h), 8, step));
    }
    m
}

/// A procedural spider facing +Z, built around `components::spider(SPIDER_HEIGHT)`:
/// a rounded cephalothorax and head with fangs, a big abdomen and eight
/// tapering legs.
pub fn demo_spider() -> Mesh {
    let h = SPIDER_HEIGHT;
    let g = crate::components::spider(h);
    let mut m = Mesh::default();
    let step = 0.012;
    let [pelvis, chest] = points(&g, "spine", &[]).try_into().unwrap();
    m.append(&ellipsoid(
        (pelvis + chest) * 0.5,
        Vec3::new(0.15, 0.1, 0.24) * h,
    ));
    let [_, head, fangs] = points(&g, "head", &[]).try_into().unwrap();
    m.append(&sphere(head, 0.08 * h, 14, 8));
    m.append(&cylinder(head, fangs, (0.035 * h, 0.025 * h), 10, step));
    let abdomen = points(&g, "abdomen", &[]);
    m.append(&ellipsoid(abdomen[1], Vec3::new(0.2, 0.18, 0.36) * h));
    for s in ["_l", "_r"] {
        for n in 1..=crate::components::SPIDER_LEGS {
            tube(
                &mut m,
                &points(&g, &format!("leg{n}{s}"), &[]),
                &[0.035 * h, 0.03 * h, 0.022 * h, 0.015 * h, 0.01 * h],
                step,
            );
        }
    }
    m
}

/// A procedural snake lying along the floor facing +Z, built around
/// `components::snake(SNAKE_HEIGHT)`: a body tapering to the tail tip, a
/// raised neck and a flat head with a snout.
pub fn demo_snake() -> Mesh {
    let h = SNAKE_HEIGHT;
    let g = crate::components::snake(h);
    let mut m = Mesh::default();
    let step = 0.02;
    let mut tail = points(&g, "tail", &[]);
    tail.reverse();
    let [pelvis, chest] = points(&g, "spine", &[]).try_into().unwrap();
    let [neck, head, snout] = points(&g, "head", &[]).try_into().unwrap();
    let mut body = tail.clone();
    let n = tail.len();
    let mut radii: Vec<f32> = (0..n)
        .map(|i| (0.015 + 0.075 * i as f32 / (n - 1) as f32) * h)
        .collect();
    // The spine guides are far apart: the body is round between them.
    for t in [0.0, 0.33, 0.67, 1.0] {
        body.push(pelvis.lerp(chest, t));
        radii.push(0.09 * h);
    }
    body.extend([neck, head]);
    radii.extend([0.075 * h, 0.06 * h]);
    tube(&mut m, &body, &radii, step);
    m.append(&ellipsoid(
        head + Vec3::Z * 0.06 * h,
        Vec3::new(0.09, 0.055, 0.14) * h,
    ));
    m.append(&cylinder(head, snout, (0.05 * h, 0.035 * h), 12, step));
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

    #[test]
    fn demo_dog_is_valid_and_stands_on_the_floor() {
        let m = demo_quadruped();
        m.validate().unwrap();
        let (min, max) = m.bounds();
        assert!(min.y > -0.05 * DOG_HEIGHT && min.y < 0.02 * DOG_HEIGHT);
        assert!(max.z - min.z > max.y - min.y, "longer than it is tall");
    }

    /// Every creature's body is valid and on the floor, and its layout builds
    /// with no check warnings: every joint inside the body, limbs bending the
    /// way IK folds them, left and right mirror images.
    #[test]
    fn creatures_stand_on_the_floor_around_their_guides() {
        for c in Creature::ALL {
            assert_eq!(Creature::from_name(c.name()), Some(c));
            let m = c.mesh();
            m.validate().unwrap();
            let (min, _) = m.bounds();
            let h = c.height();
            assert!(min.y > -0.05 * h && min.y < 0.02 * h, "{c:?}: {min}");
            let g = c.guides();
            let issues = crate::checks::check(&g, Some(&mut crate::checks::Surface::new(&m)));
            assert!(issues.is_empty(), "{c:?}: {issues:#?}");
            let rig =
                crate::rig::build_rig(m, &c.rig_name(), g, crate::RigOptions::default()).unwrap();
            // Every deforming bone takes some skin.
            let used: std::collections::HashSet<usize> = (0..rig.skin.vertex_count())
                .flat_map(|v| rig.skin.influences(v).map(|(b, _)| b).collect::<Vec<_>>())
                .collect();
            for (i, b) in rig.skeleton.bones.iter().enumerate() {
                assert!(
                    !b.deform || used.contains(&i),
                    "{c:?}: {} has no skin",
                    b.name
                );
            }
        }
    }
}
