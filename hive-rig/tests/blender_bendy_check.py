"""Check bendy limbs and soft IK of a generated Blender rig in real Blender.

Build the rig with both on (`hive-rig demo --bendy --soft 0.5`), then:
    python blender_bendy_check.py <name>_blender.py
    blender --background --python blender_bendy_check.py -- <name>_blender.py

Bendy: moving a bend control arcs its segment's bend joints along a smooth curve
while the shoulder (hip), elbow (knee) and wrist (ankle) stay put; the pin moves
the elbow or knee of the curves. Soft IK: as the goal moves out through full
reach the middle joint opens smoothly instead of snapping straight, the end
follows hive-rig's soft_reach curve, stretch still reaches the goal, and FK to
IK matching doesn't pop on a nearly straight limb.
"""

import json
import math
import runpy
import sys

import bpy
from mathutils import Matrix, Quaternion, Vector

args = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else sys.argv[1:]
script = args[0]

bpy.ops.wm.read_factory_settings(use_empty=True)
saved_argv = sys.argv
sys.argv = ["blender"]
ns = runpy.run_path(script, run_name="__main__")
sys.argv = saved_argv

RIG = ns["RIG_DATA"]
rig = next(o for o in bpy.data.objects if o.type == "ARMATURE")
geo = next(o for o in bpy.data.objects if o.type == "MESH" and not o.name.startswith("WGT_"))
pose, bones = rig.pose.bones, rig.data.bones
chains = {c["name"]: c for c in RIG["controls"]["ik_chains"]}
limbs = {l["name"]: l for l in json.loads(rig["hive_anim"])["limbs"]}


def check(cond, msg):
    if not cond:
        raise SystemExit("FAIL: " + msg)
    print("ok:", msg)


def update():
    rig.update_tag()
    bpy.context.view_layer.update()


def deformed():
    deps = bpy.context.evaluated_depsgraph_get()
    deps.update()
    ev = geo.evaluated_get(deps)
    me = ev.to_mesh()
    out = [v.co.copy() for v in me.vertices]
    ev.to_mesh_clear()
    return out


def soft_reach(distance, length, soft, zone):
    """src/controls.rs soft_reach, which every builder implements."""
    ease = max(0.0, min(1.0, soft)) * zone * length
    if ease <= 0.0:
        return min(distance, length)
    start = length - ease
    if distance <= start:
        return distance
    return start + ease * (1.0 - math.exp(-(distance - start) / ease))


check(all(c.get("bendy") for c in chains.values()), "every limb is bendy")
check(all(c["default_soft"] == 0.5 for c in chains.values()), "every limb starts with soft 0.5")
drivers = rig.animation_data.drivers
check(all(fc.driver.is_valid for fc in drivers), "all %d drivers are valid" % len(drivers))
check(all(fc.driver.type != "SCRIPTED" or fc.driver.is_simple_expression for fc in drivers),
      "drivers use simple expressions (no Python auto-run needed)")

groups = {vg.name for vg in geo.vertex_groups}
for c in chains.values():
    b = c["bendy"]
    for seg in (b["upper"], b["lower"]):
        check(not bones[seg["bone"]].use_deform and seg["bone"] not in groups,
              "%s: the segment bone %s no longer deforms" % (c["name"], seg["bone"]))
        check(len(seg["joints"]) >= 3 and all(bones[j].use_deform and j in groups for j in seg["joints"]),
              "%s: %d bend joints on %s take the skin" % (c["name"], len(seg["joints"]), seg["bone"]))



def reset(soft=0.0):
    """Bind pose. Soft IK eases a limb that is nearly straight at bind (as these
    arms and legs are), so the bendy checks switch it off."""
    ns["reset_pose"](rig)
    for limb in limbs.values():
        if "soft" in pose[limb["ik"]].keys():
            pose[limb["ik"]]["soft"] = soft
    update()


rest = [v.co.copy() for v in geo.data.vertices]
ns["reset_pose"](rig)
check(all(pose[l["ik"]]["soft"] == 0.5 for l in limbs.values()), "reset restores the default soft")
check(max((a - b).length for a, b in zip(rest, deformed())) > 1e-3,
      "soft 0.5 eases the nearly straight limbs of the bind pose")
reset()
check(max((a - b).length for a, b in zip(rest, deformed())) < 1e-4, "with soft off the bendy rig rests on the bind mesh")


def heads(names):
    update()
    return [pose[n].head.copy() for n in names]


def curve_points(seg):
    """The bend joints' heads plus the last joint's tail: the curve from start to end."""
    update()
    return [pose[j].head.copy() for j in seg["joints"]] + [pose[seg["joints"][-1]].tail.copy()]


def offsets_from_chord(points):
    a, b = points[0], points[-1]
    chord = (b - a).normalized()
    out = []
    for p in points:
        v = p - a
        out.append(v - chord * v.dot(chord))
    return out


for name in ("arm_l", "leg_l", "arm_r", "leg_r"):
    chain, limb = chains[name], limbs[name]
    b = chain["bendy"]
    upper, lower = b["upper"], b["lower"]
    reset()
    # Pose the limb first (FK for arms, IK for legs) so the bend works on a posed limb.
    if limb["fk"] and pose[limb["ik"]]["ik_fk"] < 0.5:
        pose[limb["fk"][0]].rotation_euler = (0.2, 0.1, -0.15)
        pose[limb["fk"][1]].rotation_euler = (0.0, 0.0, 0.0)
    else:
        m = pose[limb["ik"]].matrix.copy()
        m.translation += Vector((0.0, -0.08, 0.1))
        pose[limb["ik"]].matrix = m
    update()
    before_upper = curve_points(upper)
    before_lower = curve_points(lower)
    ends = heads([limb["upper"], limb["lower"], limb["end"]])
    mesh_before = deformed()

    # Push the upper segment's bend control sideways, square to the segment.
    seg_dir = (before_upper[-1] - before_upper[0]).normalized()
    side = seg_dir.cross(Vector((0.0, 0.0, 1.0)))
    if side.length < 0.1:
        side = seg_dir.cross(Vector((1.0, 0.0, 0.0)))
    side.normalize()
    length = (before_upper[-1] - before_upper[0]).length
    push = side * 0.2 * length
    ctrl = pose[upper["control"]]
    m = ctrl.matrix.copy()
    m.translation += push
    ctrl.matrix = m
    update()
    after = curve_points(upper)
    k = len(upper["joints"])
    off = offsets_from_chord(after)
    # The ends stay put: shoulder or hip, elbow or knee, wrist or ankle, and the lower segment.
    moved_ends = max((a - b).length for a, b in zip(ends, heads([limb["upper"], limb["lower"], limb["end"]])))
    check(moved_ends < 1e-4, "%s: bending the upper segment leaves the joints in place (%.6f m)" % (name, moved_ends))
    check((after[0] - before_upper[0]).length < 1e-4 and (after[-1] - before_upper[-1]).length < 1e-4,
          "%s: the curve's ends stay on the shoulder/hip and the elbow/knee (%.6f, %.6f m)"
          % (name, (after[0] - before_upper[0]).length, (after[-1] - before_upper[-1]).length))
    lower_moved = max((a - b).length for a, b in zip(before_lower, curve_points(lower)))
    check(lower_moved < 1e-4, "%s: the lower segment's joints don't move (%.6f m)" % (name, lower_moved))
    # The middle point moves by the full push, the others by 4t(1-t) of it: a smooth arc.
    for i, o in enumerate(off):
        t = i / k
        want = 4.0 * t * (1.0 - t) * push.length
        check(abs(o.length - want) < 0.03 * push.length and (want < 1e-6 or o.dot(push) > 0),
              "%s: curve point %d/%d bows out %.4f m (expected %.4f)" % (name, i, k, o.length, want))
    # Each joint runs from its point to the next one, so the skin follows the arc.
    for i, j in enumerate(upper["joints"]):
        update()
        gap = (pose[j].tail - after[i + 1]).length
        check(gap < 1e-4, "%s: %s reaches the next curve point (%.6f m)" % (name, j, gap))
    mesh_after = deformed()
    moved = [(a - b).length for a, b in zip(mesh_before, mesh_after)]
    check(max(moved) > 0.5 * push.length, "%s: the skin arcs with the bend (%.3f m)" % (name, max(moved)))

    # The pin moves the elbow or knee of both curves; the shoulder and wrist stay put.
    reset()
    pin = pose[b["pin"]]
    m = pin.matrix.copy()
    m.translation += Vector((0.0, 0.03, 0.04))
    pin.matrix = m
    update()
    up_pts, lo_pts = curve_points(upper), curve_points(lower)
    check((up_pts[-1] - pin.head).length < 1e-4 and (lo_pts[0] - pin.head).length < 1e-4,
          "%s: both curves meet at the moved pin" % name)
    check((up_pts[0] - bones[limb["upper"]].head_local).length < 1e-4
          and (lo_pts[-1] - bones[limb["end"]].head_local).length < 1e-4,
          "%s: the pin leaves the shoulder/hip and wrist/ankle in place" % name)

reset()
check(max((a - b).length for a, b in zip(rest, deformed())) < 1e-4, "reset puts the bendy limbs back at rest")

# --------------------------------------------------------------------------- soft IK
leg = limbs["leg_l"]
chain = chains["leg_l"]
soft = leg["soft"]
L, zone = soft["length"], soft["zone"]
ik = pose[leg["ik"]]


def sweep(soft_value, stretch=0.0, lo=0.85, hi=1.15, steps=61):
    """Move the IK goal out along the rest reach line; return (goal distance, knee
    angle, end distance) at each step."""
    reset()
    ik["ik_fk"] = 1.0
    ik["soft"] = soft_value
    ik["stretch"] = stretch
    update()
    root = pose[soft["root"]].head.copy()
    goal0 = pose[leg["goal"]].head.copy()
    direction = (goal0 - root).normalized()
    base = ik.matrix.copy()
    out = []
    for s in range(steps):
        d = L * (lo + (hi - lo) * s / (steps - 1))
        m = base.copy()
        m.translation = base.translation + direction * (d - (goal0 - root).length)
        ik.matrix = m
        update()
        a, k, e = (pose[n].head.copy() for n in (leg["upper"], leg["lower"], leg["end"]))
        angle = (k - a).angle(e - k)
        out.append((d, angle, (e - root).length, (e - pose[leg["goal"]].head).length))
    return out


hard = sweep(0.0)
eased = sweep(0.5)
for h, e in list(zip(hard, eased))[::6]:
    print("    goal at %.2fx reach: knee %.3f rad without soft, %.3f with" % (h[0] / L, h[1], e[1]))


def max_slope(rows):
    return max(abs(rows[i + 1][1] - rows[i][1]) / (rows[i + 1][0] - rows[i][0]) for i in range(len(rows) - 1))


check(hard[-1][1] < 0.02, "without soft the knee snaps straight past full reach (%.4f rad)" % hard[-1][1])
check(all(eased[i + 1][1] <= eased[i][1] + 1e-6 for i in range(len(eased) - 1)),
      "with soft the knee opens monotonically as the goal moves out")
check(min(r[1] for r in eased) > 0.01, "with soft the knee never snaps fully straight (%.4f rad at 1.15x reach)"
      % eased[-1][1])
hard_slope, soft_slope = max_slope(hard), max_slope(eased)
check(soft_slope < 0.35 * hard_slope,
      "the knee's fastest opening rate drops from %.1f to %.1f rad/m with soft" % (hard_slope, soft_slope))
# No jumps: the change between steps varies smoothly (second difference stays small).
jerk = max(abs(eased[i + 1][1] - 2 * eased[i][1] + eased[i - 1][1]) for i in range(1, len(eased) - 1))
hard_jerk = max(abs(hard[i + 1][1] - 2 * hard[i][1] + hard[i - 1][1]) for i in range(1, len(hard) - 1))
check(jerk < 0.25 * hard_jerk, "the soft angle curve is smooth (%.4f vs %.4f rad without soft)" % (jerk, hard_jerk))
worst = max(abs(r[2] - soft_reach(r[0], L, 0.5, zone)) for r in eased)
check(worst < 2e-3, "the foot follows hive-rig's soft_reach curve (within %.5f m)" % worst)
check(all(r[3] < 1e-3 for r in eased if r[0] < L * (1 - 0.5 * zone) - 1e-3),
      "below the eased zone the foot stays on the goal")

stretchy = sweep(0.5, stretch=1.0)
check(max(r[3] for r in stretchy) < 2e-3, "soft with stretch reaches the goal at every distance (%.5f m)"
      % max(r[3] for r in stretchy))
at_reach = min(stretchy, key=lambda r: abs(r[0] - L))
check(at_reach[1] > 0.05, "soft with stretch keeps the knee bent at full reach (%.3f rad)" % at_reach[1])
check(all(stretchy[i + 1][1] <= stretchy[i][1] + 1e-6 for i in range(len(stretchy) - 1)),
      "soft with stretch still opens the knee monotonically")

# FK to IK on a nearly straight leg: the goal is pushed out so the softened reach lands on FK.
reset()
ik["soft"] = 0.5
ik["ik_fk"] = 0.0


def bend_axis(limb):
    upper, lower = bones[limb["upper"]], bones[limb["lower"]]
    n = (lower.head_local - upper.head_local).cross(bones[limb["reach"]].head_local - lower.head_local)
    return lower.matrix_local.to_3x3().inverted() @ n.normalized()


fk_mid = pose[leg["fk"][1]]
fk_mid.rotation_mode = "QUATERNION"
fk_mid.rotation_quaternion = Quaternion(bend_axis(leg), -0.02)
fk_mid.rotation_mode = "XYZ"
pose[leg["fk"][0]].rotation_euler = (0.1, 0.0, 0.05)
update()
want = [(rig.matrix_world @ pose[n].matrix).normalized() for n in leg["bones"]]
ns["fk_to_ik"](rig, "leg_l")
update()
gap = (pose[leg["goal"]].head - pose[leg["end"]].head).length
check(gap > 1e-3, "FK to IK pushes the goal %.4f m past the foot to make up for soft" % gap)
for n, w in zip(leg["bones"], want):
    got = (rig.matrix_world @ pose[n].matrix).normalized()
    err = (got.translation - w.translation).length
    check(err < 2e-3, "FK to IK with soft on: %s does not pop (%.5f m)" % (n, err))

reset()
check(max((a - b).length for a, b in zip(rest, deformed())) < 1e-4, "after the soft checks the mesh is back at rest")
print("blender bendy and soft IK check passed")
