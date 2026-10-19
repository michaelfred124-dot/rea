//! Body parts of an existing skeleton read from its shape, not its names.
//!
//! Joint names differ between every tool and artist (`leg_joint_L_1`,
//! `b_LeftLeg01_015`, or no names at all), but a skeleton's tree says what it
//! is: limbs are chains that end at the floor (legs) or far out to the sides
//! (arms), the pelvis is where the two legs and the spine branch, the chest
//! where the two arms and the neck branch, and the head is the chain that
//! reaches the top. Left and right come from position, in hive-rig's frame
//! (left is +X), so the skeleton must already be turned into it.
//!
//! An animal on four legs has two pairs of floor-reaching limbs, at the front
//! and the back of the body: the hind pair branches at the pelvis, the front
//! pair at the shoulders, and the tail is the chain from the pelvis that reaches
//! furthest back.

use crate::components::Body;
use crate::joints::Joint;
use crate::orient::BodyKind;

/// The parts found. Chains run from the joint after their branch point to their
/// far end; `[0]` is the left side, `[1]` the right.
#[derive(Debug, Clone, Default)]
pub struct BodyMap {
    pub kind: BodyKind,
    pub pelvis: usize,
    /// From the joint above the pelvis up to the chest (the arms' branch point).
    pub spine: Vec<usize>,
    pub chest: Option<usize>,
    /// From the joint above the chest to the head's end, on the mid-line.
    pub head: Vec<usize>,
    /// Arms, or a quadruped's front legs.
    pub arms: [Vec<usize>; 2],
    /// Legs, or a quadruped's hind legs.
    pub legs: [Vec<usize>; 2],
    pub tail: Vec<usize>,
}

impl BodyMap {
    /// The joints of a part, for checking that a name points into it.
    pub fn part(&self, part: Part) -> Vec<usize> {
        match part {
            Part::Spine => {
                let mut v = vec![self.pelvis];
                v.extend(&self.spine);
                v
            }
            Part::Head => {
                let mut v: Vec<usize> = self.chest.into_iter().collect();
                v.extend(&self.head);
                v
            }
            Part::Arm(s) => self.arms[s].clone(),
            Part::Leg(s) => self.legs[s].clone(),
        }
    }
}

/// A part of the body a guide belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    Spine,
    Head,
    /// 0 left, 1 right.
    Arm(usize),
    Leg(usize),
}

struct Tree<'a> {
    joints: &'a [Joint],
    children: Vec<Vec<usize>>,
}

impl<'a> Tree<'a> {
    fn new(joints: &'a [Joint]) -> Tree<'a> {
        let mut children = vec![Vec::new(); joints.len()];
        for (i, j) in joints.iter().enumerate() {
            if let Some(p) = j.parent
                && p < joints.len()
                && p != i
            {
                children[p].push(i);
            }
        }
        Tree { joints, children }
    }

    /// `i` and its ancestors, from `i` up.
    fn up(&self, i: usize) -> Vec<usize> {
        let mut out = vec![i];
        let mut at = self.joints[i].parent;
        while let Some(p) = at {
            if out.contains(&p) || out.len() > self.joints.len() {
                break;
            }
            out.push(p);
            at = self.joints[p].parent;
        }
        out
    }

    fn lca(&self, a: usize, b: usize) -> Option<usize> {
        let ua = self.up(a);
        self.up(b).into_iter().find(|x| ua.contains(x))
    }

    /// The chain from below `top` down to `end` (excluding `top`), or `None`
    /// when `top` isn't above `end`.
    fn down(&self, top: usize, end: usize) -> Option<Vec<usize>> {
        let mut up = self.up(end);
        let k = up.iter().position(|&x| x == top)?;
        up.truncate(k);
        up.reverse();
        Some(up)
    }

    fn subtree(&self, i: usize) -> Vec<usize> {
        let mut out = vec![i];
        let mut k = 0;
        while k < out.len() {
            out.extend(&self.children[out[k]]);
            k += 1;
        }
        out
    }

    fn depth(&self, i: usize) -> usize {
        self.up(i).len()
    }

    /// The joint below `i` (or `i` itself) at the end of the longest chain
    /// down from it, measured along the bones.
    fn reach_end(&self, i: usize) -> usize {
        let mut best = (0.0f32, i);
        let mut stack = vec![(i, 0.0f32)];
        while let Some((at, length)) = stack.pop() {
            if length > best.0 {
                best = (length, at);
            }
            let p = self.joints[at].position;
            for &c in &self.children[at] {
                if c != i {
                    stack.push((c, length + p.distance(self.joints[c].position)));
                }
            }
        }
        best.1
    }
}

/// Read the body parts of `joints` (in hive-rig's frame) on a body measured by
/// `body`, of the `kind` the character was found to be (see
/// [`crate::orient::detect`]). `None` when the skeleton doesn't show its legs.
pub fn map(joints: &[Joint], body: &Body, kind: BodyKind) -> Option<BodyMap> {
    if joints.len() < 4 {
        return None;
    }
    let t = Tree::new(joints);
    let h = body.height.max(1e-6);
    let cx = body.center_x;
    let pos = |i: usize| joints[i].position;
    // Limb ends at the floor, grouped by side and, along the body, into limbs.
    let (zmin, zmax) = joints
        .iter()
        .fold((f32::INFINITY, f32::NEG_INFINITY), |a, j| {
            (a.0.min(j.position.z), a.1.max(j.position.z))
        });
    let gap = 0.2 * (zmax - zmin).max(0.2 * h);
    let mut sides: [Vec<usize>; 2] = [Vec::new(), Vec::new()];
    for i in 0..joints.len() {
        let p = pos(i);
        if !t.children[i].is_empty() || p.y - body.floor > 0.12 * h || (p.x - cx).abs() < 0.02 * h {
            continue;
        }
        sides[usize::from(p.x < cx)].push(i);
    }
    let limbs = |v: &mut Vec<usize>| -> Vec<usize> {
        v.sort_by(|a, b| pos(*a).z.total_cmp(&pos(*b).z));
        let mut groups: Vec<Vec<usize>> = Vec::new();
        for &i in v.iter() {
            match groups.last_mut() {
                Some(g) if pos(i).z - pos(*g.last().unwrap()).z <= gap => g.push(i),
                _ => groups.push(vec![i]),
            }
        }
        // The deepest leaf of each limb (the toe tip rather than a heel helper).
        groups
            .into_iter()
            .map(|g| {
                *g.iter()
                    .max_by(|a, b| {
                        t.depth(**a)
                            .cmp(&t.depth(**b))
                            .then(pos(**b).y.total_cmp(&pos(**a).y))
                    })
                    .unwrap()
            })
            .collect()
    };
    let [mut l, mut r] = sides;
    let (left, right) = (limbs(&mut l), limbs(&mut r));
    match kind {
        // Hands hanging near the floor end low too: the feet are the lowest.
        BodyKind::Biped => {
            let foot = |v: &[usize]| {
                v.iter()
                    .copied()
                    .min_by(|a, b| pos(*a).y.total_cmp(&pos(*b).y))
            };
            biped(&t, body, foot(&left)?, foot(&right)?)
        }
        BodyKind::Quadruped if left.len() == 2 && right.len() == 2 => {
            quadruped(&t, body, [left[0], right[0]], [left[1], right[1]])
        }
        BodyKind::Quadruped => None,
    }
}

fn biped(t: &Tree, body: &Body, foot_l: usize, foot_r: usize) -> Option<BodyMap> {
    let h = body.height;
    let cx = body.center_x;
    let pos = |i: usize| t.joints[i].position;
    let high = |p: usize| pos(p).y - body.floor >= 0.25 * h;
    let mut pelvis = t.lca(foot_l, foot_r)?;
    let mut legs = [t.down(pelvis, foot_l)?, t.down(pelvis, foot_r)?];
    if !high(pelvis) {
        // Feet hung from the root (as IK rigs do) rather than from the legs:
        // each belongs to the leg chain ending nearest above it.
        let above = |f: usize| {
            let side = (pos(f).x - cx).signum();
            let ancestors = t.up(f);
            (0..t.joints.len())
                .filter(|&i| t.children[i].is_empty() && !ancestors.contains(&i))
                .filter(|&i| pos(i).y > pos(f).y + 0.02 * h && (pos(i).x - cx) * side > 0.0)
                .min_by(|a, b| {
                    pos(*a)
                        .distance(pos(f))
                        .total_cmp(&pos(*b).distance(pos(f)))
                })
        };
        let (end_l, end_r) = (above(foot_l)?, above(foot_r)?);
        pelvis = t.lca(end_l, end_r)?;
        legs = [t.down(pelvis, end_l)?, t.down(pelvis, end_r)?];
        legs[0].push(foot_l);
        legs[1].push(foot_r);
    }
    if legs.iter().any(|l| l.len() < 2) || !high(pelvis) {
        return None;
    }
    let mut map = BodyMap {
        kind: BodyKind::Biped,
        pelvis,
        legs,
        ..BodyMap::default()
    };
    // Arms: the joints furthest out to each side above the legs, outside them,
    // followed down to the end of their longest chain (arms held down are
    // furthest out at the elbow, with the hands hanging below it).
    let in_legs: Vec<usize> = map.legs.iter().flat_map(|l| t.subtree(l[0])).collect();
    let free = |i: &usize| !in_legs.contains(i) && pos(*i).y - body.floor > 0.35 * h;
    let hand = |sign: f32| {
        (0..t.joints.len())
            .filter(free)
            .filter(|&i| sign * (pos(i).x - cx) > 0.12 * h)
            .max_by(|a, b| (sign * pos(*a).x).total_cmp(&(sign * pos(*b).x)))
            .map(|i| t.reach_end(i))
    };
    if let (Some(hl), Some(hr)) = (hand(1.0), hand(-1.0))
        && let Some(chest) = t.lca(hl, hr)
        && t.up(chest).contains(&pelvis)
        && let (Some(al), Some(ar)) = (t.down(chest, hl), t.down(chest, hr))
    {
        map.spine = if chest == pelvis {
            Vec::new()
        } else {
            t.down(pelvis, chest)?
        };
        map.arms = [al, ar];
        map.chest = Some(chest);
        map.head = head_chain(t, body, chest, &map.arms, |p| p.y);
    }
    Some(map)
}

fn quadruped(t: &Tree, body: &Body, hind: [usize; 2], front: [usize; 2]) -> Option<BodyMap> {
    let pos = |i: usize| t.joints[i].position;
    let pelvis = t.lca(hind[0], hind[1])?;
    let shoulders = t.lca(front[0], front[1])?;
    let legs = [t.down(pelvis, hind[0])?, t.down(pelvis, hind[1])?];
    let arms = [t.down(shoulders, front[0])?, t.down(shoulders, front[1])?];
    let mut map = BodyMap {
        kind: BodyKind::Quadruped,
        pelvis,
        legs,
        arms,
        ..BodyMap::default()
    };
    if t.up(shoulders).contains(&pelvis) && shoulders != pelvis {
        map.spine = t.down(pelvis, shoulders)?;
        map.chest = Some(shoulders);
        map.head = head_chain(t, body, shoulders, &map.arms, |p| p.y + p.z);
    }
    // Tail: from the pelvis, the branch (not a leg, not the spine) reaching
    // furthest back.
    let mut taken: Vec<usize> = map.legs.iter().flatten().copied().collect();
    taken.extend(&map.spine);
    let tail_end = t.children[pelvis]
        .iter()
        .filter(|c| !taken.contains(c))
        .flat_map(|&c| t.subtree(c))
        .min_by(|a, b| pos(*a).z.total_cmp(&pos(*b).z));
    if let Some(end) = tail_end
        && pos(end).z < pos(pelvis).z
    {
        map.tail = t.down(pelvis, end).unwrap_or_default();
    }
    Some(map)
}

/// From `chest`, the branch that isn't an arm reaching furthest by `reach`,
/// as a chain; joints off the mid-line at its end (eyes) are dropped.
fn head_chain(
    t: &Tree,
    body: &Body,
    chest: usize,
    arms: &[Vec<usize>; 2],
    reach: impl Fn(glam::Vec3) -> f32,
) -> Vec<usize> {
    let pos = |i: usize| t.joints[i].position;
    let end = t.children[chest]
        .iter()
        .filter(|c| !arms.iter().any(|a| a.first() == Some(c)))
        .flat_map(|&c| t.subtree(c))
        .max_by(|a, b| reach(pos(*a)).total_cmp(&reach(pos(*b))));
    let Some(end) = end else {
        return Vec::new();
    };
    let mut chain = t.down(chest, end).unwrap_or_default();
    // Face joints (brows, lids, eyes, a jaw) fan out from the head. Past a
    // fan only a leaf on the mid-line can be the head's end, never a brow.
    if let Some(f) = chain.iter().position(|&j| t.children[j].len() >= 3)
        && f + 1 < chain.len()
    {
        let next = chain[f + 1];
        let ends_head = f + 2 == chain.len()
            && t.children[next].is_empty()
            && (pos(next).x - body.center_x).abs() < 0.002 * body.height;
        chain.truncate(if ends_head { f + 2 } else { f + 1 });
    }
    while chain
        .last()
        .is_some_and(|&j| (pos(j).x - body.center_x).abs() > 0.03 * body.height)
    {
        chain.pop();
    }
    chain
}

/// Cumulative lengths along `points`, from the first.
fn along(points: &[glam::Vec3]) -> Vec<f32> {
    let mut out = vec![0.0];
    for w in points.windows(2) {
        out.push(out.last().unwrap() + w[0].distance(w[1]));
    }
    out
}

/// An arm's (or front leg's) joints by role: clavicle, shoulder, elbow, wrist
/// and hand end, as indices into the chain.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ArmRoles {
    pub clavicle: Option<usize>,
    pub shoulder: Option<usize>,
    pub elbow: Option<usize>,
    pub wrist: Option<usize>,
    pub hand_tip: Option<usize>,
}

/// Roles along an arm chain. The wrist is the first joint the fingers branch
/// from (three or more children), or else the last joint unless the last
/// segment is short (a hand end); the elbow is the joint about halfway along;
/// before it, a clavicle and the shoulder.
pub fn arm_roles(joints: &[Joint], chain: &[usize]) -> ArmRoles {
    let mut r = ArmRoles::default();
    if chain.is_empty() {
        return r;
    }
    let pts: Vec<glam::Vec3> = chain.iter().map(|&i| joints[i].position).collect();
    let kids = |i: usize| joints.iter().filter(|j| j.parent == Some(i)).count();
    let n = chain.len();
    let seg = |k: usize| pts[k].distance(pts[k - 1]);
    // Three or more children starting at one point away from their parent (a
    // palm joint per finger) branch at that point: the wrist is the next joint
    // along the chain.
    let fan_ahead = |k: usize| {
        k + 1 < n && {
            let next = pts[k + 1];
            let reach = next.distance(pts[k]);
            joints
                .iter()
                .filter(|j| j.parent == Some(chain[k]) && j.position.distance(next) < 0.1 * reach)
                .count()
                >= 3
        }
    };
    let w = (1..n)
        .find(|&k| kids(chain[k]) >= 3)
        .map(|k| if fan_ahead(k) { k + 1 } else { k })
        .unwrap_or(if n >= 3 && seg(n - 1) < 0.6 * seg(n - 2) {
            n - 2
        } else {
            n - 1
        });
    r.wrist = Some(w);
    if w + 1 < n {
        r.hand_tip = Some(n - 1);
    }
    let cum = along(&pts);
    if w >= 2 {
        let total = cum[w].max(1e-9);
        r.elbow = (1..w).min_by(|a, b| {
            (cum[*a] / total - 0.55)
                .abs()
                .total_cmp(&(cum[*b] / total - 0.55).abs())
        });
    }
    match r.elbow {
        Some(e) if e >= 2 => {
            r.clavicle = Some(0);
            r.shoulder = (1..e).max_by(|a, b| {
                pts[*a]
                    .distance(pts[e])
                    .total_cmp(&pts[*b].distance(pts[e]))
            });
        }
        _ => {
            if w >= 1 {
                r.shoulder = Some(0);
            }
        }
    }
    r
}

/// A biped leg's joints by role: hip, knee, ankle, ball and toe, as indices.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LegRoles {
    pub hip: Option<usize>,
    pub knee: Option<usize>,
    pub ankle: Option<usize>,
    pub ball: Option<usize>,
    pub toe: Option<usize>,
}

/// Roles along a leg chain: the ankle is the first joint after which the chain
/// runs more along the floor than down it, or drops to the floor (within 5% of
/// the leg's height); the knee the joint about halfway from the hip to it, and
/// the joints after the ankle the ball and toe.
pub fn leg_roles(joints: &[Joint], chain: &[usize]) -> LegRoles {
    let mut r = LegRoles::default();
    if chain.len() < 2 {
        return r;
    }
    let pts: Vec<glam::Vec3> = chain.iter().map(|&i| joints[i].position).collect();
    let n = pts.len();
    r.hip = Some(0);
    let low = pts.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
    let near_floor = 0.05 * (pts[0].y - low).max(1e-6);
    let a = (1..n - 1)
        .find(|&k| {
            let d = pts[k + 1] - pts[k];
            glam::Vec2::new(d.x, d.z).length() > d.y.abs()
                || (pts[k + 1].y - low < near_floor && pts[k].y - low >= near_floor)
        })
        .unwrap_or(n - 1);
    r.ankle = Some(a);
    let cum = along(&pts);
    if a >= 2 {
        let total = cum[a].max(1e-9);
        r.knee = (1..a).min_by(|x, y| {
            (cum[*x] / total - 0.5)
                .abs()
                .total_cmp(&(cum[*y] / total - 0.5).abs())
        });
    }
    if a + 1 < n {
        r.ball = Some(a + 1);
    }
    if a + 2 < n {
        r.toe = Some(n - 1);
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{components, demo, rig};
    use glam::Vec3;

    /// The demo's own skeleton with every name hidden.
    fn nameless(mesh: &crate::mesh::Mesh) -> (Vec<Joint>, components::Built, Body) {
        let (guides, _) = rig::detect_guides(mesh);
        let built = components::build(&guides).unwrap();
        let joints = built
            .skeleton
            .bones
            .iter()
            .enumerate()
            .map(|(i, b)| Joint {
                name: format!("j{i}"),
                position: b.head,
                parent: b.parent,
            })
            .collect();
        (joints, built, guides.body)
    }

    #[test]
    fn finds_a_biped_by_shape() {
        let mesh = demo::humanoid(0.0);
        let (mut joints, built, body) = nameless(&mesh);
        // Toe ends, as most skeletons have.
        for side in ["l", "r"] {
            let b = built.skeleton.bone(&format!("ball_{side}")).unwrap();
            let i = built.skeleton.index_of(&format!("ball_{side}")).unwrap();
            joints.push(Joint {
                name: "end".into(),
                position: b.tail,
                parent: Some(i),
            });
        }
        let map = map(&joints, &body, BodyKind::Biped).unwrap();
        let name = |i: usize| {
            built
                .skeleton
                .bones
                .get(i)
                .map(|b| b.name.as_str())
                .unwrap_or("end")
        };
        assert_eq!(name(map.pelvis), "pelvis");
        assert_eq!(map.kind, BodyKind::Biped);
        let leg = leg_roles(&joints, &map.legs[0]);
        assert_eq!(name(map.legs[0][leg.hip.unwrap()]), "thigh_l");
        assert_eq!(name(map.legs[0][leg.ankle.unwrap()]), "foot_l");
        assert!(name(map.legs[0][leg.knee.unwrap()]).starts_with("calf_l"));
        let right = leg_roles(&joints, &map.legs[1]);
        assert_eq!(name(map.legs[1][right.ankle.unwrap()]), "foot_r");
        let arm = arm_roles(&joints, &map.arms[1]);
        assert_eq!(name(map.arms[1][arm.clavicle.unwrap()]), "clavicle_r");
        assert_eq!(name(map.arms[1][arm.shoulder.unwrap()]), "upperarm_r");
        assert!(name(map.arms[1][arm.elbow.unwrap()]).starts_with("lowerarm_r"));
        assert_eq!(name(map.arms[1][arm.wrist.unwrap()]), "hand_r");
        assert_eq!(name(*map.head.first().unwrap()), "neck_01");
        assert!(map.chest.is_some());
    }

    #[test]
    fn the_head_ends_at_its_face_joints() {
        // Brows, eyes and a jaw fan out from the head, a brow higher than
        // the head joint; none of them is the head's end.
        let mesh = demo::humanoid(0.0);
        let (mut joints, built, body) = nameless(&mesh);
        let head = built.skeleton.index_of("head").unwrap();
        let at = built.skeleton.bones[head].head;
        let h = body.height;
        for (dx, dy) in [
            (0.008, 0.05),
            (-0.008, 0.05),
            (0.02, 0.03),
            (-0.02, 0.03),
            (0.0, -0.03),
        ] {
            joints.push(Joint {
                name: "face".into(),
                position: at + Vec3::new(dx * h, dy * h, 0.04 * h),
                parent: Some(head),
            });
        }
        let map = map(&joints, &body, BodyKind::Biped).unwrap();
        assert_eq!(*map.head.last().unwrap(), head);
    }

    #[test]
    fn finds_a_dog_by_shape() {
        let mesh = demo::demo_quadruped();
        let guides = components::quadruped(demo::DOG_HEIGHT);
        let built = components::build(&guides).unwrap();
        let mut joints: Vec<Joint> = built
            .skeleton
            .bones
            .iter()
            .map(|b| Joint {
                name: String::new(),
                position: b.head,
                parent: b.parent,
            })
            .collect();
        // Paw ends.
        for (i, b) in built.skeleton.bones.iter().enumerate() {
            if b.tail.y < 0.03 * guides.body.height
                && !built.skeleton.bones.iter().any(|c| c.parent == Some(i))
            {
                joints.push(Joint {
                    name: String::new(),
                    position: b.tail,
                    parent: Some(i),
                });
            }
        }
        let _ = mesh;
        let map = map(&joints, &guides.body, BodyKind::Quadruped).unwrap();
        assert_eq!(map.kind, BodyKind::Quadruped);
        let name = |i: usize| {
            built
                .skeleton
                .bones
                .get(i)
                .map(|b| b.name.as_str())
                .unwrap_or("end")
        };
        assert_eq!(name(map.pelvis), "pelvis");
        assert!(
            name(map.legs[0][0]).starts_with("thigh_l"),
            "{}",
            name(map.legs[0][0])
        );
        assert!(!map.tail.is_empty());
        assert!(!map.head.is_empty());
        let _ = Vec3::ZERO;
    }
}
