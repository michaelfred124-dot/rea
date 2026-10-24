"""Build a rig with spline chains (a tentacle and a spline spine) in real Blender
and check that the joints follow the curve its controls bend.

Make the rig with:
    hive-rig demo -o out
    hive-rig add out/hive_demo.guides.json tentacle
    hive-rig spine out/hive_demo.guides.json --spline true
    hive-rig rebuild out/hive_demo.hive.json

Run with the `bpy` module (pip install bpy) or Blender itself:
    python blender_spline_check.py <name>_blender.py <name>.glb
    blender --background --python blender_spline_check.py -- <name>_blender.py <name>.glb
"""

import json
import os
import runpy
import sys

import bpy
from mathutils import Matrix, Vector

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import hive_names  # noqa: E402

args = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else sys.argv[1:]
script, glb = args[0], args[1]

bpy.ops.wm.read_factory_settings(use_empty=True)
saved_argv = sys.argv
sys.argv = ["blender"]
ns = runpy.run_path(script, run_name="__main__")
sys.argv = saved_argv

RIG = json.loads(open(script).read().split('r"""', 1)[1].split('"""', 1)[0])
NAMING = RIG["guides"].get("naming")
SPLINES = {s["name"]: s for s in RIG["controls"]["splines"]}

rig = next(o for o in bpy.data.objects if o.type == "ARMATURE")
geo = next(o for o in bpy.data.objects if o.type == "MESH" and not o.name.startswith("WGT_"))
pose = rig.pose.bones


def final(internal):
    """The exported name of an internal bone or control name."""
    return hive_names.final(NAMING, internal)


def update():
    rig.update_tag()
    bpy.context.view_layer.update()


def head(name):
    update()
    return rig.matrix_world @ pose[name].head


def tail(name):
    update()
    return rig.matrix_world @ pose[name].tail


def move(ctrl, delta):
    m = pose[ctrl].matrix.copy()
    m.translation += Vector(delta)
    pose[ctrl].matrix = m
    update()


def deformed():
    deps = bpy.context.evaluated_depsgraph_get()
    deps.update()
    ev = geo.evaluated_get(deps)
    me = ev.to_mesh()
    out = [v.co.copy() for v in me.vertices]
    ev.to_mesh_clear()
    return out


def reset():
    ns["reset_pose"](rig)
    update()


def check(cond, msg):
    if not cond:
        raise SystemExit("FAIL: " + msg)
    print("ok:", msg)


def curve_points(sp):
    """The evaluated curve the chain rides on, as a dense polyline in world space."""
    obj = bpy.data.objects[RIG["name"] + "_" + sp["name"] + "_curve"]
    deps = bpy.context.evaluated_depsgraph_get()
    deps.update()
    ev = obj.evaluated_get(deps)
    me = ev.to_mesh()
    pts = [obj.matrix_world @ v.co for v in me.vertices]
    ev.to_mesh_clear()
    return pts


def distance_to(polyline, p):
    best = float("inf")
    for a, b in zip(polyline, polyline[1:]):
        ab = b - a
        t = max(0.0, min(1.0, (p - a).dot(ab) / max(ab.length_squared, 1e-12)))
        best = min(best, (a + ab * t - p).length)
    return best


def chain_length(sp):
    """Length of the joints on the curve (all but the last, which hangs off its end)."""
    return sum((tail(b) - head(b)).length for b in sp["bones"][:-1])


def on_curve(sp):
    """Largest distance from a joint (heads and the tip) to the curve."""
    line = curve_points(sp)
    joints = [head(b) for b in sp["bones"]]
    return max(distance_to(line, j) for j in joints), line


rest = [v.co.copy() for v in geo.data.vertices]
check(max((a - b).length for a, b in zip(rest, deformed())) < 1e-4, "rig adds no offset in the rest pose")
drivers = rig.animation_data.drivers
check(all(fc.driver.is_valid for fc in drivers), "all %d drivers are valid" % len(drivers))
check(all(fc.driver.type != "SCRIPTED" or fc.driver.is_simple_expression for fc in drivers),
      "drivers use simple expressions (no Python auto-run needed)")
check(set(SPLINES) == {"tentacle", "spine"}, "the rig has a tentacle and a spline spine")

for name, sp in SPLINES.items():
    bones, ctrls, fk = sp["bones"], sp["controls"], sp["fk_controls"]
    settings = pose[sp["settings"]]
    reset()
    rest_heads = [head(b) for b in bones]
    rest_len = chain_length(sp)
    gap, _ = on_curve(sp)
    check(gap < 1e-4, "%s: the joints sit on the curve at rest" % name)
    check(settings["stretch"] == 1.0 and settings["volume"] == 0.0, "%s: stretch starts on, volume off" % name)

    # Bend: push the middle curve control(s) sideways. The joints follow a smooth
    # curve through the bend, staying on it.
    along = (rest_heads[-1] - rest_heads[0]).normalized()
    side = along.cross(Vector((0, 0, 1)))
    if side.length < 0.1:
        side = Vector((1, 0, 0))
    side.normalize()
    push = 0.15 * rest_len
    for c in ctrls[1:-1]:
        move(c, side * push)
    gap, _ = on_curve(sp)
    check(gap < 0.002 * rest_len + 1e-4, "%s: bent, every joint stays on the curve (%.4f m off)" % (name, gap))
    mid = len(bones) // 2
    swing = (head(bones[mid]) - rest_heads[mid]).dot(side)
    check(swing > 0.3 * push, "%s: the middle of the chain follows the bend (%.3f m)" % (name, swing))
    check((head(bones[0]) - rest_heads[0]).length < 1e-3, "%s: the base stays put" % name)
    # Neighbouring joints turn by small, even steps: a smooth curve, not a kink.
    dirs = [(tail(b) - head(b)).normalized() for b in bones[:-1]]
    steps = [a.angle(b) for a, b in zip(dirs, dirs[1:])]
    check(not steps or max(steps) < 0.6, "%s: the chain bends smoothly (largest turn %.2f rad)" % (name, max(steps or [0])))

    # Stretch on: pull the end control out along the chain; the chain reaches
    # the end of the curve, where the last joint starts.
    reset()
    pull = along * 0.3 * rest_len
    move(ctrls[-1], pull)
    curve_end = curve_points(sp)[-1]
    check((head(bones[-1]) - curve_end).length < 2e-3 * rest_len + 1e-4,
          "%s: with stretch on the chain reaches the end of the curve" % name)
    check((head(bones[-1]) - (rest_heads[-1] + pull)).length < 2e-3 * rest_len + 1e-4,
          "%s: the last joint follows the end control" % name)
    check(chain_length(sp) > 1.2 * rest_len, "%s: the chain stretched (%.3f -> %.3f m)" % (name, rest_len, chain_length(sp)))

    # Volume: the stretched joints thin out only with volume on.
    scale_x = pose[bones[mid]].matrix.to_scale().x
    check(abs(scale_x - 1.0) < 1e-3, "%s: without volume the joints keep their thickness" % name)
    settings["volume"] = 1.0
    update()
    scale_x = pose[bones[mid]].matrix.to_scale().x
    check(scale_x < 0.95, "%s: with volume on, stretched joints thin out (%.3f)" % (name, scale_x))
    settings["volume"] = 0.0

    # Stretch off: the joints keep their length and slide along the curve.
    settings["stretch"] = 0.0
    update()
    check(abs(chain_length(sp) - rest_len) < 1e-3 * rest_len + 1e-5,
          "%s: with stretch off the joints keep their length" % name)
    check((head(bones[-1]) - curve_points(sp)[-1]).length > 0.1 * rest_len,
          "%s: with stretch off the chain stops short of a far end control" % name)
    gap, _ = on_curve(sp)
    check(gap < 0.002 * rest_len + 1e-4, "%s: unstretched joints still ride the curve" % name)

    # The FK layer: turning the first FK control swings the rest of the chain.
    reset()
    tip0 = tail(bones[-1])
    pose[fk[0]].rotation_euler = (0.0, 0.0, 0.5)
    pose[fk[0]].rotation_euler = (0.5, 0.3, 0.0)
    update()
    check((tail(bones[-1]) - tip0).length > 0.1 * rest_len, "%s: the first FK control swings the chain" % name)
    check((head(bones[0]) - rest_heads[0]).length < 1e-3, "%s: the base stays on the start control" % name)

    # Turning the end control turns the last joint.
    reset()
    before = pose[bones[-1]].matrix.to_3x3().copy()
    pose[ctrls[-1]].rotation_euler = (0.0, 0.6, 0.0)
    update()
    turned = (pose[bones[-1]].matrix.to_3x3() @ before.inverted()).to_quaternion().angle
    check(abs(turned - 0.6) < 0.05, "%s: the end control turns the last joint (%.2f rad)" % (name, turned))

    # Twist: turning the end control about the chain's length twists the joints
    # along it, a little at the base and most of the way at the tip, without
    # moving them off the curve.
    for ctrl, at_end in ((ctrls[-1], True), (ctrls[0], False)):
        reset()
        chain_bones = bones[:-1]
        before = [pose[b].matrix.to_3x3().normalized() for b in chain_bones]
        heads0 = [head(b) for b in bones]
        axis = (before[-1 if at_end else 0] @ Vector((0.0, 1.0, 0.0))).normalized()
        m = pose[ctrl].matrix.copy()
        spin = Matrix.Rotation(0.8, 4, axis)
        pose[ctrl].matrix = Matrix.Translation(m.translation) @ spin @ Matrix.Translation(-m.translation) @ m
        update()
        twists = []
        for b, m0 in zip(chain_bones, before):
            d = (m0.inverted() @ pose[b].matrix.to_3x3().normalized()).to_quaternion()
            swing_twist = d.to_swing_twist("Y")[1]
            twists.append(swing_twist)
        which = "end" if at_end else "start"
        far, near = (twists[-1], twists[0]) if at_end else (twists[0], twists[-1])
        check(abs(far) > 0.5 and abs(near) < abs(far),
              "%s: twisting the %s control twists the chain toward that end (%.2f near it, %.2f far)"
              % (name, which, far, near))
        steps = [abs(a - b) for a, b in zip(twists, twists[1:])]
        check(not steps or max(steps) < 0.45, "%s: the %s twist spreads evenly (largest step %.2f rad)"
              % (name, which, max(steps or [0])))
        moved = max((head(b) - h).length for b, h in zip(bones[:-1], heads0[:-1]))
        if at_end:
            check(moved < 2e-3 * rest_len + 1e-4, "%s: twisting keeps the joints on the curve (%.4f m)" % (name, moved))

# Spine specifics: the chest carries the head and arms, the hips the pelvis.
reset()
spine = SPLINES["spine"]
chest = spine["controls"][-1]
head0, hand0, foot0 = head(final("head")), head(final("hand_l")), head(final("foot_l"))
move(chest, (0.0, 0.05, 0.03))
check((head(final("head")) - head0 - Vector((0.0, 0.05, 0.03))).length < 2e-3,
      "moving the chest control carries the head")
check((head(final("hand_l")) - hand0).length > 0.02, "the arms (in FK) ride on the chest")
check((head(final("foot_l")) - foot0).length < 1e-4, "the IK feet stay planted")
reset()
pelvis0 = tail(final("pelvis"))
pose[spine["controls"][0]].rotation_euler = (0.3, 0.0, 0.0)
update()
check((tail(final("pelvis")) - pelvis0).length > 1e-3, "the hips control turns the pelvis")
gap, _ = on_curve(spine)
check(gap < 1e-3, "turning the hips keeps the spine on its curve")
spaces = next(c for c in RIG["controls"]["controls"] if c["name"] == final("hand_l_ik_ctrl"))["spaces"]
check(chest in spaces, "the IK hand can follow the chest control")

# The skin follows a bent tentacle.
reset()
tentacle = SPLINES["tentacle"]
for c in tentacle["controls"][1:]:
    move(c, (0.0, 0.0, 0.1))
after = deformed()
check(max((a - b).length for a, b in zip(rest, after)) > 0.02, "bending the tentacle deforms the mesh")
reset()
check(max((a - b).length for a, b in zip(rest, deformed())) < 1e-4, "reset puts the mesh back at rest")

bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=glb)
armatures = [o for o in bpy.data.objects if o.type == "ARMATURE"]
meshes = [o for o in bpy.data.objects if o.type == "MESH" and o.parent in armatures]
check(len(armatures) == 1 and len(meshes) == 1, "glb imports as one armature and one skinned mesh")
check(len(meshes[0].vertex_groups) == len(armatures[0].data.bones), "glb carries skin weights for every bone")
names = {b.name for b in armatures[0].data.bones}
check(all(b in names for sp in SPLINES.values() for b in sp["bones"]), "glb has every spline joint")
print("blender spline check passed")
