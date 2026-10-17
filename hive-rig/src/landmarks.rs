//! Body landmark detection for a humanoid in T-pose or A-pose.
//!
//! The mesh is cut with horizontal planes. Each cut yields a set of intervals along
//! X (the silhouette seen from the front); the number and width of those intervals
//! tell us where the legs split (crotch), where the arms leave the torso and where
//! the neck is narrowest. Joints are then pulled to the centroid of the surrounding
//! geometry so they sit inside the limbs rather than on the surface.

use glam::Vec3;
use serde::{Deserialize, Serialize};

use crate::mesh::Mesh;

/// Proportions of an average adult, as fractions of standing height. Used to place
/// joints along a limb once its start and end are known, and as fallbacks.
mod ratio {
    pub const CROTCH: f32 = 0.47;
    pub const KNEE: f32 = 0.285;
    pub const ANKLE: f32 = 0.045;
    pub const SHOULDER: f32 = 0.81;
    pub const NECK: f32 = 0.87;
    /// Elbow and wrist positions along the shoulder-to-fingertip line.
    pub const ELBOW_ALONG_ARM: f32 = 0.42;
    pub const WRIST_ALONG_ARM: f32 = 0.76;
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Interval {
    pub min: f32,
    pub max: f32,
}

impl Interval {
    pub fn center(&self) -> f32 {
        (self.min + self.max) * 0.5
    }
    pub fn width(&self) -> f32 {
        self.max - self.min
    }
    pub fn contains(&self, x: f32) -> bool {
        x >= self.min && x <= self.max
    }
}

/// Key joint positions found on the mesh, all in model space.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Landmarks {
    pub floor: f32,
    pub height: f32,
    pub center_x: f32,
    pub pelvis: Vec3,
    pub chest: Vec3,
    pub neck: Vec3,
    pub head_top: Vec3,
    pub clavicle_l: Vec3,
    pub shoulder_l: Vec3,
    pub elbow_l: Vec3,
    pub wrist_l: Vec3,
    pub hand_tip_l: Vec3,
    pub hip_l: Vec3,
    pub knee_l: Vec3,
    pub ankle_l: Vec3,
    pub ball_l: Vec3,
    pub toe_l: Vec3,
    /// Half-width of the torso, used to classify arm vs. body geometry.
    pub torso_half_width: f32,
    pub crotch_y: f32,
    /// True for each landmark found from geometry rather than from fallback ratios.
    pub detected: Detected,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Detected {
    pub crotch: bool,
    pub arms: bool,
    pub neck: bool,
}

/// X intervals of the mesh cross-section at height `y`.
pub fn slice_x(mesh: &Mesh, y: f32, merge_tolerance: f32) -> Vec<Interval> {
    let mut spans: Vec<Interval> = Vec::new();
    for [a, b, c] in mesh.triangles() {
        let pts = [mesh.positions[a], mesh.positions[b], mesh.positions[c]];
        let mut hits = [0.0f32; 3];
        let mut n = 0;
        for i in 0..3 {
            let (p, q) = (pts[i], pts[(i + 1) % 3]);
            if (p.y <= y && q.y > y) || (q.y <= y && p.y > y) {
                let t = (y - p.y) / (q.y - p.y);
                hits[n] = p.x + (q.x - p.x) * t;
                n += 1;
            }
        }
        if n >= 2 {
            let (lo, hi) = (hits[0].min(hits[1]), hits[0].max(hits[1]));
            spans.push(Interval { min: lo, max: hi });
        }
    }
    merge(spans, merge_tolerance)
}

fn merge(mut spans: Vec<Interval>, tol: f32) -> Vec<Interval> {
    spans.sort_by(|a, b| a.min.total_cmp(&b.min));
    let mut out: Vec<Interval> = Vec::new();
    for s in spans {
        match out.last_mut() {
            Some(last) if s.min <= last.max + tol => last.max = last.max.max(s.max),
            _ => out.push(s),
        }
    }
    out
}

/// Centroid of the vertices inside a box around `p`, falling back to `p`.
fn centroid_near(mesh: &Mesh, p: Vec3, half: Vec3, filter: impl Fn(Vec3) -> bool) -> Option<Vec3> {
    let mut sum = Vec3::ZERO;
    let mut n = 0u32;
    for &v in &mesh.positions {
        let d = (v - p).abs();
        if d.x <= half.x && d.y <= half.y && d.z <= half.z && filter(v) {
            sum += v;
            n += 1;
        }
    }
    (n > 0).then(|| sum / n as f32)
}

/// Pull a joint to the middle of the limb in the axes listed in `axes` (x, y, z).
fn recenter(
    mesh: &Mesh,
    p: Vec3,
    half: Vec3,
    axes: [bool; 3],
    filter: impl Fn(Vec3) -> bool,
) -> Vec3 {
    match centroid_near(mesh, p, half, filter) {
        Some(c) => Vec3::new(
            if axes[0] { c.x } else { p.x },
            if axes[1] { c.y } else { p.y },
            if axes[2] { c.z } else { p.z },
        ),
        None => p,
    }
}

pub fn detect(mesh: &Mesh) -> Landmarks {
    let (min, max) = mesh.bounds();
    let floor = min.y;
    let h = (max.y - min.y).max(1e-4);
    let at = |r: f32| floor + r * h;
    let tol = 0.004 * h;
    let mid_z = (min.z + max.z) * 0.5;

    // Body centre: midpoint of the widest-spread slice near the hips is unreliable on
    // asymmetric props, so use the centre of the bounding box in X.
    let center_x = (min.x + max.x) * 0.5;

    // --- Crotch: highest cut (searching downward) where two leg intervals sit on
    // either side of the centre line.
    let mut crotch_y = None;
    let steps = 120;
    for i in 0..steps {
        let y = at(0.62) - (0.62 - 0.20) * h * i as f32 / steps as f32;
        let iv = slice_x(mesh, y, tol);
        let left = iv.iter().any(|s| s.min > center_x);
        let right = iv.iter().any(|s| s.max < center_x);
        let center_gap = !iv.iter().any(|s| s.contains(center_x));
        if left && right && center_gap {
            crotch_y = Some(y);
            break;
        }
    }
    let detected_crotch = crotch_y.is_some();
    let crotch_y = crotch_y.unwrap_or(at(ratio::CROTCH));

    // --- Torso half-width at the waist (between crotch and shoulders), measured on
    // the interval containing the centre line.
    let waist_y = crotch_y + 0.12 * h;
    let torso_half_width = slice_x(mesh, waist_y, tol)
        .iter()
        .find(|s| s.contains(center_x))
        .map(|s| (s.max - center_x).max(center_x - s.min))
        .unwrap_or(0.12 * h);

    // --- Arms: geometry clearly outside the torso and above the crotch.
    let arm_threshold = torso_half_width * 1.15;
    let arm_verts = |sign: f32| -> Vec<Vec3> {
        mesh.positions
            .iter()
            .copied()
            .filter(|v| v.y > crotch_y + 0.05 * h && (v.x - center_x) * sign > arm_threshold)
            .collect()
    };
    let left_arm = arm_verts(1.0);
    let detected_arms = left_arm.len() >= 8;

    let (shoulder_l, hand_tip_l) = if detected_arms {
        let tip = left_arm
            .iter()
            .copied()
            .max_by(|a, b| a.x.total_cmp(&b.x))
            .unwrap();
        // Upper-arm root: vertices just outside the torso give the arm's height there.
        let near: Vec<Vec3> = left_arm
            .iter()
            .copied()
            .filter(|v| v.x - center_x < arm_threshold + 0.08 * h)
            .collect();
        let root = if near.is_empty() {
            Vec3::new(center_x + arm_threshold, at(ratio::SHOULDER), mid_z)
        } else {
            near.iter().copied().sum::<Vec3>() / near.len() as f32
        };
        // The shoulder joint sits a little inside the torso silhouette.
        let shoulder = Vec3::new(
            center_x + torso_half_width * 0.85,
            root.y + (root.y - tip.y) * 0.1,
            root.z,
        );
        (shoulder, tip)
    } else {
        let shoulder = Vec3::new(
            center_x + torso_half_width * 0.85,
            at(ratio::SHOULDER),
            mid_z,
        );
        (shoulder, shoulder + Vec3::new(0.44 * h, 0.0, 0.0))
    };

    let arm_filter = |v: Vec3| (v.x - center_x) > arm_threshold * 0.9 && v.y > crotch_y;
    let along = |a: Vec3, b: Vec3, t: f32| a + (b - a) * t;
    let slab = Vec3::new(0.02 * h, 0.06 * h, 0.06 * h);
    let mut elbow_l = along(shoulder_l, hand_tip_l, ratio::ELBOW_ALONG_ARM);
    let mut wrist_l = along(shoulder_l, hand_tip_l, ratio::WRIST_ALONG_ARM);
    if detected_arms {
        elbow_l = recenter(mesh, elbow_l, slab, [false, true, true], arm_filter);
        wrist_l = recenter(mesh, wrist_l, slab, [false, true, true], arm_filter);
    }
    // A slight backward bend tells IK solvers which way the elbow folds.
    elbow_l.z -= 0.006 * h;

    // --- Neck: narrowest centre interval between shoulders and the top of the head.
    let mut neck_y = None;
    let mut narrowest = f32::INFINITY;
    let shoulder_line = shoulder_l.y.max(at(0.70));
    for i in 0..60 {
        let y = shoulder_line + (at(0.95) - shoulder_line) * i as f32 / 60.0;
        if let Some(s) = slice_x(mesh, y, tol).iter().find(|s| s.contains(center_x))
            && s.width() < narrowest
        {
            narrowest = s.width();
            neck_y = Some(y);
        }
    }
    let detected_neck = neck_y.is_some() && narrowest < torso_half_width * 1.4;
    let neck_y = if detected_neck {
        neck_y.unwrap()
    } else {
        at(ratio::NECK)
    };

    let center_filter = |v: Vec3| (v.x - center_x).abs() < torso_half_width;
    let torso_slab = Vec3::new(torso_half_width, 0.03 * h, h);
    let center_point = |y: f32| {
        recenter(
            mesh,
            Vec3::new(center_x, y, mid_z),
            torso_slab,
            [false, false, true],
            center_filter,
        )
    };

    let pelvis = center_point(crotch_y + 0.06 * h);
    let neck = center_point(neck_y);
    let chest = center_point(shoulder_l.y - 0.04 * h);
    let head_top = Vec3::new(center_x, max.y, neck.z);
    let clavicle_l = Vec3::new(
        center_x + 0.02 * h,
        shoulder_l.y - 0.02 * h,
        chest.z + 0.01 * h,
    );

    // --- Legs.
    let leg_filter = |v: Vec3| (v.x - center_x) > 0.0 && v.y < crotch_y;
    let leg_x = slice_x(mesh, crotch_y - 0.05 * h, tol)
        .iter()
        .filter(|s| s.min > center_x)
        .map(|s| s.center())
        .next()
        .unwrap_or(center_x + torso_half_width * 0.5);
    let leg_slab = Vec3::new(0.08 * h, 0.03 * h, 0.15 * h);
    let hip_l = recenter(
        mesh,
        Vec3::new(leg_x, crotch_y + 0.02 * h, pelvis.z),
        leg_slab,
        [false, false, true],
        leg_filter,
    );
    let mut knee_l = recenter(
        mesh,
        Vec3::new(leg_x, at(ratio::KNEE), hip_l.z),
        leg_slab,
        [true, false, true],
        leg_filter,
    );
    // Knees bend forward.
    knee_l.z += 0.008 * h;
    let ankle_l = recenter(
        mesh,
        Vec3::new(knee_l.x, at(ratio::ANKLE), knee_l.z),
        Vec3::new(0.06 * h, 0.02 * h, 0.04 * h),
        [true, false, true],
        leg_filter,
    );
    let toe_z = mesh
        .positions
        .iter()
        .filter(|v| v.y < at(0.06) && (v.x - center_x) > 0.0)
        .map(|v| v.z)
        .fold(ankle_l.z + 0.1 * h, f32::max);
    let toe_l = Vec3::new(ankle_l.x, floor + 0.01 * h, toe_z);
    let ball_l = Vec3::new(
        ankle_l.x,
        floor + 0.02 * h,
        ankle_l.z + (toe_z - ankle_l.z) * 0.7,
    );

    Landmarks {
        floor,
        height: h,
        center_x,
        pelvis,
        chest,
        neck,
        head_top,
        clavicle_l,
        shoulder_l,
        elbow_l,
        wrist_l,
        hand_tip_l,
        hip_l,
        knee_l,
        ankle_l,
        ball_l,
        toe_l,
        torso_half_width,
        crotch_y,
        detected: Detected {
            crotch: detected_crotch,
            arms: detected_arms,
            neck: detected_neck,
        },
    }
}

/// Mirror a left-side point to the right across the body centre line.
pub fn mirror(p: Vec3, center_x: f32) -> Vec3 {
    Vec3::new(2.0 * center_x - p.x, p.y, p.z)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo::demo_humanoid;

    #[test]
    fn slicing_a_humanoid_at_the_shins_finds_two_legs() {
        let mesh = demo_humanoid();
        let iv = slice_x(&mesh, 0.4, 0.005);
        assert_eq!(iv.len(), 2, "{iv:?}");
    }

    #[test]
    fn detects_a_pose_arms() {
        let mesh = crate::demo::humanoid(40.0);
        let lm = detect(&mesh);
        assert!(lm.detected.arms && lm.detected.crotch, "{:?}", lm.detected);
        // Arms slope down: elbow below shoulder, wrist below elbow, all outside the torso.
        assert!(lm.elbow_l.y < lm.shoulder_l.y && lm.wrist_l.y < lm.elbow_l.y);
        assert!(lm.wrist_l.x > lm.elbow_l.x && lm.elbow_l.x > lm.shoulder_l.x);
        // Hand tip lies on the arm line: 0.74 m from the shoulder root at 40 degrees.
        let expected = glam::vec3(0.12, 1.40, 0.0)
            + glam::vec3(40f32.to_radians().cos(), -40f32.to_radians().sin(), 0.0) * 0.74;
        assert!(
            lm.hand_tip_l.distance(expected) < 0.06,
            "tip {} vs {}",
            lm.hand_tip_l,
            expected
        );
    }

    #[test]
    fn detects_demo_landmarks() {
        let mesh = demo_humanoid();
        let lm = detect(&mesh);
        assert!(
            lm.detected.crotch && lm.detected.arms && lm.detected.neck,
            "{:?}",
            lm.detected
        );
        // The demo character is 1.8 m tall with its crotch at 0.85 m.
        assert!((lm.height - 1.8).abs() < 0.02);
        assert!((lm.crotch_y - 0.85).abs() < 0.05, "crotch {}", lm.crotch_y);
        // Hands extend to x = 0.86 in T-pose.
        assert!((lm.hand_tip_l.x - 0.86).abs() < 0.02);
        assert!(
            lm.shoulder_l.y > 1.35 && lm.shoulder_l.y < 1.55,
            "shoulder {}",
            lm.shoulder_l
        );
        assert!(
            lm.neck.y > lm.shoulder_l.y && lm.neck.y < 1.65,
            "neck {}",
            lm.neck
        );
        assert!(lm.knee_l.y < lm.hip_l.y && lm.ankle_l.y < lm.knee_l.y);
        assert!(lm.elbow_l.x > lm.shoulder_l.x && lm.wrist_l.x > lm.elbow_l.x);
        assert!(lm.toe_l.z > lm.ankle_l.z);
    }
}
