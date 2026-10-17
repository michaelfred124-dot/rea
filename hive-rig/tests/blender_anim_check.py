"""Check the animator tools of a generated Blender rig in real Blender.

IK to FK and FK to IK matching (no pop), pose mirroring, reset, select all and
space switching without a pop.

Run with the `bpy` module (pip install bpy) or Blender itself:
    python blender_anim_check.py <name>_blender.py
    blender --background --python blender_anim_check.py -- <name>_blender.py
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

rig = next(o for o in bpy.data.objects if o.type == "ARMATURE")
geo = next(o for o in bpy.data.objects if o.type == "MESH" and not o.name.startswith("WGT_"))
data = json.loads(rig["hive_anim"])
pose, bones = rig.pose.bones, rig.data.bones
center_x = json.loads(rig["hive_guides"])["body"]["center_x"]
limbs = {l["name"]: l for l in data["limbs"]}
POS_TOL, ROT_TOL = 2e-3, math.radians(0.5)


def check(cond, msg):
    if not cond:
        raise SystemExit("FAIL: " + msg)
    print("ok:", msg)


def update():
    rig.update_tag()
    bpy.context.view_layer.update()


def world(name):
    update()
    return (rig.matrix_world @ pose[name].matrix).normalized()


def differs(a, b):
    """Largest position (m) and rotation (rad) difference between two world matrices."""
    rot = a.to_quaternion().rotation_difference(b.to_quaternion()).angle
    return (a.translation - b.translation).length, min(rot, math.tau - rot)


def same(a, b):
    d, r = differs(a, b)
    return d < POS_TOL and r < ROT_TOL, "%.5f m, %.3f deg" % (d, math.degrees(r))


def chain_bones(limb):
    return [limb[k] for k in ("upper", "lower", "end")]


def deformed():
    deps = bpy.context.evaluated_depsgraph_get()
    deps.update()
    ev = geo.evaluated_get(deps)
    me = ev.to_mesh()
    out = [v.co.copy() for v in me.vertices]
    ev.to_mesh_clear()
    return out


def bend_axis(limb):
    """Hinge axis of the middle joint (the normal of the limb's rest plane) in its
    own rest frame. Positive angles bend the joint further the way it already bends."""
    upper, lower = bones[limb["upper"]], bones[limb["lower"]]
    n = (lower.head_local - upper.head_local).cross(bones[limb["end"]].head_local - lower.head_local)
    return lower.matrix_local.to_3x3().inverted() @ n.normalized()


rest = [v.co.copy() for v in geo.data.vertices]
ns["reset_pose"](rig)
check(max((a - b).length for a, b in zip(rest, deformed())) < 1e-4, "reset leaves the rest pose unchanged")

for side in ("l", "r"):
    for kind in ("arm", "leg"):
        name = "%s_%s" % (kind, side)
        limb = limbs[name]

        # IK to FK: pose the IK limb, match, and the FK controls hold the IK pose.
        ns["reset_pose"](rig)
        ik = pose[limb["ik"]]
        ik["ik_fk"] = 1.0
        upper, end = bones[limb["upper"]].head_local, bones[limb["end"]].head_local
        m = ik.matrix.copy()
        m.translation += (upper - end) * 0.3 + Vector((0.0, -0.05, 0.03))
        ik.matrix = m @ Matrix.Rotation(0.3, 4, "X") @ Matrix.Rotation(-0.2, 4, "Z")
        pole = pose[limb["pole"]]
        pm = pole.matrix.copy()
        pm.translation += Vector((0.08 if side == "l" else -0.08, 0.0, 0.0))
        pole.matrix = pm
        if kind == "leg":
            ik["roll"] = 15.0
        update()
        ik_pose = [world(b) for b in chain_bones(limb)]
        check(differs(ik_pose[0], world(limb["fk"][0]))[1] > 0.05, "%s: IK pose differs from FK before matching" % name)
        ns["ik_to_fk"](rig, name)
        check(pose[limb["ik"]]["ik_fk"] == 0.0, "%s: IK to FK switches the limb to FK" % name)
        for bone, ctrl, want in zip(chain_bones(limb), limb["fk"], ik_pose):
            ok, how = same(world(ctrl), want)
            check(ok, "%s: FK control %s matches the IK pose (%s)" % (name, ctrl, how))
            ok, how = same(world(bone), want)
            check(ok, "%s: %s does not pop switching to FK (%s)" % (name, bone, how))

        # FK to IK: pose the FK limb (bending the middle joint like a hinge), match.
        ns["reset_pose"](rig)
        pose[limb["ik"]]["ik_fk"] = 0.0
        if kind == "leg":
            pose[limb["ik"]]["roll"] = -20.0  # the IK goal sits off the control
        for ctrl, rot in zip((limb["fk"][0], limb["fk"][2]), ((0.35, 0.2, -0.3), (0.2, -0.15, 0.25))):
            pose[ctrl].rotation_mode = "QUATERNION"
            pose[ctrl].rotation_quaternion = Matrix.Rotation(rot[0], 3, "X").to_quaternion() @ \
                Matrix.Rotation(rot[1], 3, "Y").to_quaternion() @ Matrix.Rotation(rot[2], 3, "Z").to_quaternion()
            pose[ctrl].rotation_mode = "XYZ"
        pose[limb["fk"][1]].rotation_mode = "QUATERNION"
        pose[limb["fk"][1]].rotation_quaternion = Quaternion(bend_axis(limb), 0.7)
        pose[limb["fk"][1]].rotation_mode = "XYZ"
        update()
        fk_pose = [world(b) for b in chain_bones(limb)]
        ns["fk_to_ik"](rig, name)
        check(pose[limb["ik"]]["ik_fk"] == 1.0, "%s: FK to IK switches the limb to IK" % name)
        for bone, want in zip(chain_bones(limb), fk_pose):
            ok, how = same(world(bone), want)
            check(ok, "%s: %s does not pop switching to IK (%s)" % (name, bone, how))

        # A straight limb has no bend plane; the pole still lands in front of the joint.
        ns["reset_pose"](rig)
        pose[limb["ik"]]["ik_fk"] = 0.0
        pose[limb["fk"][1]].rotation_mode = "QUATERNION"
        pose[limb["fk"][1]].rotation_quaternion = Quaternion(bend_axis(limb), 0.0)
        pose[limb["fk"][1]].rotation_mode = "XYZ"
        update()
        rest_bend = (bones[limb["pole"]].head_local - bones[limb["lower"]].head_local).normalized()
        ns["fk_to_ik"](rig, name)
        bend = (pose[limb["pole"]].head - pose[limb["lower"]].head).normalized()
        check(bend.dot(rest_bend) > 0.99, "%s: FK to IK keeps the pole in front of the joint" % name)

# Mirror: pose the left side, copy it to the right, and the right is its mirror image.
ns["reset_pose"](rig)
arm, leg = limbs["arm_l"], limbs["leg_l"]
pose[arm["fk"][0]].rotation_euler = (0.3, -0.2, 0.5)
pose[arm["fk"][1]].rotation_euler = (0.0, 0.3, -0.6)
pose[arm["fk"][2]].rotation_euler = (0.2, 0.4, 0.1)
pose[leg["ik"]].location = (0.04, 0.1, -0.05)
pose[leg["ik"]].rotation_euler = (0.2, 0.3, -0.1)
pose[leg["ik"]]["roll"] = 25.0
pose[leg["pole"]].location = (0.05, 0.0, 0.02)
update()
n = ns["mirror_pose"](rig, None, "left")
check(n == len(data["mirror"]), "mirror with nothing selected copies all %d left controls" % n)


def mirrored(p):
    return Vector((2 * center_x - p.x, p.y, p.z))


for kl, kr in (("arm_l", "arm_r"), ("leg_l", "leg_r")):
    for bl, br in zip(chain_bones(limbs[kl]), chain_bones(limbs[kr])):
        update()
        head_err = (pose[br].head - mirrored(pose[bl].head)).length
        tail_err = (pose[br].tail - mirrored(pose[bl].tail)).length
        check(head_err < POS_TOL and tail_err < POS_TOL,
              "mirror: %s is the mirror image of %s (%.5f, %.5f m)" % (br, bl, head_err, tail_err))
        # The roll mirrors too: each side axis maps onto the twin's (up to its sign).
        def flip(v):
            return Vector((-v.x, v.y, v.z))

        sign = 1.0 if flip(bones[bl].x_axis).dot(bones[br].x_axis) > 0 else -1.0
        err = 1.0 - sign * flip(pose[bl].x_axis).dot(pose[br].x_axis)
        check(err < 1e-4, "mirror: %s turns like %s mirrored (%.6f)" % (br, bl, err))
check(pose[limbs["leg_r"]["ik"]]["roll"] == 25.0, "mirror copies the foot roll")
moved = max((pose[bl].head - bones[bl].head_local).length for bl in chain_bones(arm))
check(moved > 0.05, "the posed left arm moved (%.3f m)" % moved)

# Mirror selected only: just the picked control's twin changes.
ns["reset_pose"](rig)
pose[arm["fk"][0]].rotation_euler = (0.0, 0.0, 0.5)
pose[leg["ik"]].location = (0.0, 0.1, 0.0)
for p in pose:
    ns["set_bone_selected"](p, p.name == arm["fk"][0])
check(ns["selected_controls"](rig) == [arm["fk"][0]], "one control picked")
ns["mirror_pose"](rig, ns["selected_controls"](rig), "right")
right_arm, right_leg = limbs["arm_r"], limbs["leg_r"]
check(pose[right_arm["fk"][0]].matrix_basis != Matrix.Identity(4), "mirror selected copies the picked control")
check(pose[right_leg["ik"]].matrix_basis == Matrix.Identity(4), "mirror selected leaves other controls alone")

# Select all controls.
count = ns["select_controls"](rig)
check(sorted(ns["selected_controls"](rig)) == sorted(data["controls"]) and count == len(data["controls"]),
      "select all picks every control (%d)" % count)
check(not any(ns["bone_selected"](p) for p in pose if p.name not in data["controls"]), "select all picks only controls")

# Space switch without a pop: twist the chest, move the IK hand into chest space.
ns["reset_pose"](rig)
hand_ik = arm["ik"]
spaces = data["spaces"][hand_ik]
pose[hand_ik]["ik_fk"] = 1.0
chest = spaces[2]
pose[chest].rotation_euler = (0.0, 0.4, 0.2)
pose[hand_ik].location = (0.02, -0.05, 0.03)
update()
before = world(hand_ik)
ns["switch_space"](rig, hand_ik, 2)
ok, how = same(world(hand_ik), before)
check(pose[hand_ik]["space"] == 2 and ok, "switching the IK hand to %s does not move it (%s)" % (chest, how))
ns["switch_space"](rig, hand_ik, 0)
ok, how = same(world(hand_ik), before)
check(pose[hand_ik]["space"] == 0 and ok, "switching back to %s does not move it (%s)" % (spaces[0], how))

# The Hive tab draws its Animation box (headless Blender has no UI, so a stand-in layout).
from types import SimpleNamespace  # noqa: E402
from unittest import mock  # noqa: E402

layout = mock.MagicMock()
ns["HIVE_PT_panel"].draw(SimpleNamespace(layout=layout),
                         SimpleNamespace(active_object=rig, active_pose_bone=pose[hand_ik], mode="POSE"))
drawn = [c.args[0] for c in layout.mock_calls if c.args and isinstance(c.args[0], str) and c.args[0].startswith("hive.")]
check(drawn.count("hive.match_limb") == 2 * len(limbs) and "hive.switch_space" in drawn and "hive.mirror_pose" in drawn,
      "the Hive tab shows match buttons for %d limbs, mirror, reset and the space buttons" % len(limbs))

# Reset everything through the panel's operator.
pose[arm["fk"][0]].rotation_euler = (0.4, 0.0, 0.0)
pose[leg["ik"]]["roll"] = 30.0
pose[hand_ik]["space"] = 2
check(bpy.ops.hive.reset_pose(selected=False) == {"FINISHED"}, "Reset All runs from the Hive tab")
update()
check(all(pose[n].matrix_basis == Matrix.Identity(4) for n in data["controls"]), "reset puts every control at bind")
check(all(pose[c][p] == v for c, d in data["defaults"].items() for p, v in d.items()), "reset restores default settings")
check(max((a - b).length for a, b in zip(rest, deformed())) < 1e-4, "after reset the mesh is back at rest")
print("blender animator tools check passed")
