//! Build a humanoid skeleton from detected landmarks.

use glam::Vec3;

use crate::landmarks::{Landmarks, mirror};
use crate::skeleton::{Region, Side, Skeleton, Twist};

pub const SPINE_COUNT: usize = 3;

/// Bone chain names for a limb, used by the control builder.
pub struct LimbNames {
    pub upper: String,
    pub lower: String,
    pub end: String,
}

pub fn arm_names(side: Side) -> LimbNames {
    let s = side.suffix();
    LimbNames {
        upper: format!("upperarm{s}"),
        lower: format!("lowerarm{s}"),
        end: format!("hand{s}"),
    }
}

pub fn leg_names(side: Side) -> LimbNames {
    let s = side.suffix();
    LimbNames {
        upper: format!("thigh{s}"),
        lower: format!("calf{s}"),
        end: format!("foot{s}"),
    }
}

pub fn spine_name(i: usize) -> String {
    format!("spine_{:02}", i + 1)
}

pub fn build(lm: &Landmarks) -> Skeleton {
    let mut sk = Skeleton::default();
    let h = lm.height;
    let floor_center = Vec3::new(lm.center_x, lm.floor, lm.pelvis.z);

    let root = sk.add(
        "root",
        None,
        floor_center,
        floor_center + Vec3::Z * 0.15 * h,
        Side::Center,
        Region::Center,
    );
    sk.bones[root].deform = false;

    // Pelvis points up into the first spine joint.
    let spine_start = lm.pelvis + Vec3::Y * 0.04 * h;
    sk.add(
        "pelvis",
        Some("root"),
        lm.pelvis,
        spine_start,
        Side::Center,
        Region::Center,
    );

    let mut parent = "pelvis".to_string();
    for i in 0..SPINE_COUNT {
        let t0 = i as f32 / SPINE_COUNT as f32;
        let t1 = (i + 1) as f32 / SPINE_COUNT as f32;
        let name = spine_name(i);
        sk.add(
            name.clone(),
            Some(&parent),
            spine_start.lerp(lm.neck, t0),
            spine_start.lerp(lm.neck, t1),
            Side::Center,
            Region::Center,
        );
        parent = name;
    }
    let head_base = lm.neck.lerp(lm.head_top, 0.25);
    sk.add(
        "neck_01",
        Some(&parent),
        lm.neck,
        head_base,
        Side::Center,
        Region::Center,
    );
    sk.add(
        "head",
        Some("neck_01"),
        head_base,
        lm.head_top,
        Side::Center,
        Region::Center,
    );

    let top_spine = spine_name(SPINE_COUNT - 1);
    for side in [Side::Left, Side::Right] {
        let m = |p: Vec3| {
            if side == Side::Left {
                p
            } else {
                mirror(p, lm.center_x)
            }
        };
        let s = side.suffix();
        let (arm_region, leg_region) = match side {
            Side::Left => (Region::ArmL, Region::LegL),
            _ => (Region::ArmR, Region::LegR),
        };

        let arm = arm_names(side);
        let clavicle = format!("clavicle{s}");
        sk.add(
            clavicle.clone(),
            Some(&top_spine),
            m(lm.clavicle_l),
            m(lm.shoulder_l),
            side,
            Region::Center,
        );
        sk.add(
            arm.upper.clone(),
            Some(&clavicle),
            m(lm.shoulder_l),
            m(lm.elbow_l),
            side,
            arm_region,
        );
        sk.add(
            arm.lower.clone(),
            Some(&arm.upper),
            m(lm.elbow_l),
            m(lm.wrist_l),
            side,
            arm_region,
        );
        sk.add(
            arm.end.clone(),
            Some(&arm.lower),
            m(lm.wrist_l),
            m(lm.hand_tip_l),
            side,
            arm_region,
        );

        let leg = leg_names(side);
        sk.add(
            leg.upper.clone(),
            Some("pelvis"),
            m(lm.hip_l),
            m(lm.knee_l),
            side,
            leg_region,
        );
        sk.add(
            leg.lower.clone(),
            Some(&leg.upper),
            m(lm.knee_l),
            m(lm.ankle_l),
            side,
            leg_region,
        );
        sk.add(
            leg.end.clone(),
            Some(&leg.lower),
            m(lm.ankle_l),
            m(lm.ball_l),
            side,
            leg_region,
        );
        sk.add(
            format!("ball{s}"),
            Some(&leg.end),
            m(lm.ball_l),
            m(lm.toe_l),
            side,
            leg_region,
        );

        // Twist helpers on the far half of each limb segment (mannequin names).
        // Upper segments counter half of their own twist so the shoulder and hip
        // don't candy-wrap; lower segments carry half of the hand or foot twist.
        let twists = [
            (&arm.upper, -0.5, arm.upper.clone(), arm_region),
            (&arm.lower, 0.5, arm.end.clone(), arm_region),
            (&leg.upper, -0.5, leg.upper.clone(), leg_region),
            (&leg.lower, 0.5, leg.end.clone(), leg_region),
        ];
        for (seg, factor, source, region) in twists {
            let b = sk.bone(seg).expect("limb bone").clone();
            let name = twist_name(seg);
            let i = sk.add(
                name,
                Some(seg),
                b.head.lerp(b.tail, 0.5),
                b.tail,
                side,
                region,
            );
            sk.bones[i].twist = Some(Twist { source, factor });
        }
    }
    sk
}

/// `upperarm_l` -> `upperarm_twist_01_l`.
pub fn twist_name(bone: &str) -> String {
    match bone.rsplit_once('_') {
        Some((base, side)) => format!("{base}_twist_01_{side}"),
        None => format!("{bone}_twist_01"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{demo::demo_humanoid, landmarks::detect};

    #[test]
    fn builds_a_symmetric_connected_skeleton() {
        let sk = build(&detect(&demo_humanoid()));
        assert_eq!(sk.bones.len(), 1 + 1 + SPINE_COUNT + 2 + 2 * (4 + 4 + 4));
        let t = sk.bone("lowerarm_twist_01_l").unwrap();
        assert_eq!(t.twist.as_ref().unwrap().source, "hand_l");
        assert_eq!(twist_name("calf_r"), "calf_twist_01_r");
        assert_eq!(sk.bones.iter().filter(|b| b.parent.is_none()).count(), 1);
        let l = sk.bone("hand_l").unwrap();
        let r = sk.bone("hand_r").unwrap();
        assert!((l.head.x + r.head.x).abs() < 1e-4 && (l.head.y - r.head.y).abs() < 1e-5);
        for b in &sk.bones {
            assert!(b.length() > 1e-3, "{} is degenerate", b.name);
        }
    }
}
