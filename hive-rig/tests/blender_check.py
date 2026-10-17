"""Build a generated Blender rig in real Blender and check that it works.

Run with the `bpy` module (pip install bpy) or Blender itself:
    python blender_check.py <name>_blender.py <name>.glb
    blender --background --python blender_check.py -- <name>_blender.py <name>.glb
"""

import json
import os
import runpy
import sys

import bpy
from mathutils import Vector

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import hive_names  # noqa: E402

args = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else sys.argv[1:]
script, glb = args[0], args[1]

bpy.ops.wm.read_factory_settings(use_empty=True)
saved_argv = sys.argv
sys.argv = ["blender"]
runpy.run_path(script, run_name="__main__")
sys.argv = saved_argv

rig = next(o for o in bpy.data.objects if o.type == "ARMATURE")
geo = next(o for o in bpy.data.objects if o.type == "MESH" and not o.name.startswith("WGT_"))


# The checks use the internal (default) names; a rig exported with another naming
# convention is checked through the same renaming hive-rig applies (src/naming.rs).
NAMING = json.loads(open(script).read().split('r"""', 1)[1].split('"""', 1)[0])["guides"].get("naming")


def final(internal):
    return hive_names.final(NAMING, internal)


class Named:
    def __init__(self, items):
        self.items = items

    def __getitem__(self, internal):
        return self.items[final(internal)]

    def __contains__(self, internal):
        return final(internal) in self.items

    def __iter__(self):
        return iter(self.items)


pb = Named(rig.pose.bones)
rest_bones = Named(rig.data.bones)


def update():
    rig.update_tag()
    bpy.context.view_layer.update()


def head(name):
    update()
    return rig.matrix_world @ pb[name].head


def rest_head(name):
    return rest_bones[name].head_local


def move(ctrl, delta):
    m = pb[ctrl].matrix.copy()
    m.translation += Vector(delta)
    pb[ctrl].matrix = m
    update()


def deformed():
    deps = bpy.context.evaluated_depsgraph_get()
    deps.update()
    ev = geo.evaluated_get(deps)
    me = ev.to_mesh()
    out = [v.co.copy() for v in me.vertices]
    ev.to_mesh_clear()
    return out


def check(cond, msg):
    if not cond:
        raise SystemExit("FAIL: " + msg)
    print("ok:", msg)


rest = [v.co.copy() for v in geo.data.vertices]
check(max((a - b).length for a, b in zip(rest, deformed())) < 1e-4, "rig adds no offset in the rest pose")

for side in ("l", "r"):
    move("foot_%s_ik_ctrl" % side, (0, 0, 0.3))
    check((head("foot_" + side) - (rest_head("foot_" + side) + Vector((0, 0, 0.3)))).length < 1e-3,
          "foot_%s follows its IK control" % side)
    check(head("calf_" + side).y < rest_head("calf_" + side).y - 0.05, "knee_%s bends forward" % side)

    hand_ik = "hand_%s_ik_ctrl" % side
    check(pb[hand_ik]["ik_fk"] == 0.0, "arm_%s starts in FK" % side)
    pb["upperarm_%s_fk_ctrl" % side].rotation_euler = (0.0, 0.0, 0.6)
    check((head("hand_" + side) - rest_head("hand_" + side)).length > 0.1, "arm_%s FK control moves the hand" % side)
    pb["upperarm_%s_fk_ctrl" % side].rotation_euler = (0.0, 0.0, 0.0)

    pb[hand_ik]["ik_fk"] = 1.0
    # Pull the hand a third of the way toward the shoulder: always reachable.
    delta = (rest_head("upperarm_" + side) - rest_head("hand_" + side)) * 0.33
    move(hand_ik, delta)
    check((head("hand_" + side) - (rest_head("hand_" + side) + delta)).length < 1e-3, "hand_%s reaches its IK goal" % side)
    check(head("lowerarm_" + side).y > rest_head("lowerarm_" + side).y + 0.01, "elbow_%s bends backward" % side)

def reset():
    for p in pb:
        p.location = (0, 0, 0)
        p.rotation_euler = (0, 0, 0)
        p.rotation_quaternion = (1, 0, 0, 0)
        p.scale = (1, 1, 1)
        for prop in ("roll", "stretch", "space"):
            if prop in p.keys():
                p[prop] = 0
    update()


drivers = rig.animation_data.drivers
check(all(fc.driver.is_valid for fc in drivers), "all %d drivers are valid" % len(drivers))
check(all(fc.driver.type != "SCRIPTED" or fc.driver.is_simple_expression for fc in drivers),
      "drivers use simple expressions (no Python auto-run needed)")

# Reverse foot (legs start in IK).
reset()
ankle0, ball0, toe0 = rest_head("foot_l"), rest_head("ball_l"), rest_bones["ball_l"].tail_local
pb["foot_l_ik_ctrl"]["roll"] = -25.0
check(head("ball_l").z > ball0.z + 0.02 and abs(head("foot_l").z - ankle0.z) < 0.05, "negative roll rocks back on the heel")
pb["foot_l_ik_ctrl"]["roll"] = 25.0
check(head("foot_l").z > ankle0.z + 0.02 and (head("ball_l") - ball0).length < 0.01, "positive roll lifts the heel around the ball")
pb["foot_l_ik_ctrl"]["roll"] = 70.0
update()
toe_now = rig.matrix_world @ pb["ball_l"].tail
check(head("ball_l").z > ball0.z + 0.02 and (toe_now - toe0).length < 0.02, "past the break angle the foot pivots on the toe tip")

# Stretchy IK.
reset()
pb["hand_l_ik_ctrl"]["ik_fk"] = 1.0
far = (rest_head("hand_l") - rest_head("upperarm_l")).normalized() * 0.25
move("hand_l_ik_ctrl", far)
goal = rest_head("hand_l") + far
check((head("hand_l") - goal).length > 0.1, "without stretch the arm stops short of a far goal")
pb["hand_l_ik_ctrl"]["stretch"] = 1.0
check((head("hand_l") - goal).length < 0.02, "with stretch the arm reaches a far goal")

# Twist helpers.
reset()
pb["hand_l_ik_ctrl"]["ik_fk"] = 0.0
pb["hand_l_fk_ctrl"].rotation_euler = (0.0, 1.0, 0.0)  # 1 rad about the hand's length
update()
rel = pb["lowerarm_l"].matrix.inverted() @ pb["lowerarm_twist_01_l"].matrix
twist_angle = rel.to_euler().y
check(abs(twist_angle - 0.5) < 0.05, "forearm twist bone takes half the hand twist (%.2f rad)" % twist_angle)

# Space switching.
reset()
pb["hand_l_ik_ctrl"]["ik_fk"] = 1.0
before = head("hand_l_ik_ctrl")
pb["hand_l_ik_ctrl"]["space"] = 2  # chest
check((head("hand_l_ik_ctrl") - before).length < 1e-4, "switching space at rest causes no pop")
pb["spine_03_ctrl"].rotation_euler = (0.0, 0.4, 0.0)  # twist the chest
moved = (head("hand_l_ik_ctrl") - before).length
pb["hand_l_ik_ctrl"]["space"] = 0
check(moved > 0.05 and (head("hand_l_ik_ctrl") - before).length < 1e-4,
      "IK hand follows the chest only in chest space (moved %.3f, back to %.4f)" % (moved, (head("hand_l_ik_ctrl") - before).length))

reset()
move("cog_ctrl", (0, 0, -0.1))
check(abs(head("head").z - rest_head("head").z + 0.1) < 1e-3, "COG moves the upper body")
check(max((a - b).length for a, b in zip(rest, deformed())) > 0.1, "skin follows the skeleton")

if "eyes_ctrl" in pb:
    # Eyes look at their control, follow the head, and the gaze can stay put in world space.
    reset()

    def gaze(eye):
        update()
        b = pb[eye]
        return ((rig.matrix_world @ b.tail) - (rig.matrix_world @ b.head)).normalized()

    rest_gaze = gaze("eye_l")
    move("eyes_ctrl", (0.3, 0, 0))
    to_target = (head("eye_l_aim_ctrl") - head("eye_l")).normalized()
    check(gaze("eye_l").dot(to_target) > 0.9999 and gaze("eye_l").dot(rest_gaze) < 0.98,
          "eyes turn to look at the moved eye control")
    check(gaze("eye_r").x > 0.1, "both eyes follow the master eye control")
    reset()
    pb["head_ctrl"].rotation_euler = (0.0, 0.0, 0.5)
    followed = (head("eyes_ctrl") - pb["eyes_ctrl"].bone.head_local).length
    check(followed > 0.01, "the eye control follows the head in head space")
    reset()
    pb["eyes_ctrl"]["space"] = 2  # root: the gaze stays fixed while the head turns
    pb["head_ctrl"].rotation_euler = (0.0, 0.0, 0.5)
    look = (head("eye_l_aim_ctrl") - head("eye_l")).normalized()
    check(gaze("eye_l").dot(look) > 0.9999 and (head("eyes_ctrl") - pb["eyes_ctrl"].bone.head_local).length < 1e-3,
          "in world space the eyes keep looking at a fixed target")

if "jaw_ctrl" in pb:
    reset()
    chin = pb["jaw"].tail.copy()
    pb["jaw_ctrl"].rotation_euler = (0.4, 0.0, 0.0)
    update()
    check((pb["jaw"].tail - chin).length > 0.01, "the jaw control opens the jaw")
    reset()

if "brow_in_l_ctrl" in pb:
    # Face joints move with their controls and only deform the skin near them.
    reset()
    brow0 = head("brow_in_l")
    before = deformed()
    move("brow_in_l_ctrl", (0, 0, 0.01))
    check((head("brow_in_l") - brow0 - Vector((0, 0, 0.01))).length < 1e-4, "a face control moves its joint")
    after = deformed()
    moved = [(b - a).length for a, b in zip(before, after)]
    near = [i for i, m in enumerate(moved) if m > 1e-4]
    world_brow = rig.matrix_world @ rest_head("brow_in_l")
    reach = max((geo.matrix_world @ before[i] - world_brow).length for i in near) if near else 0.0
    check(max(moved) > 0.002 and reach < 0.08, "moving a brow deforms only the skin near it (%d vertices within %.3f m)" % (len(near), reach))
    reset()
    lid = pb["lid_up_l"]
    tip0 = (rig.matrix_world @ lid.tail).copy()
    pb["lid_up_l_ctrl"].rotation_euler = (0.4, 0.0, 0.0)
    update()
    check(((rig.matrix_world @ lid.tail) - tip0).length > 0.002, "rotating an eyelid control swings the lid")
    reset()
    if "jaw_ctrl" in pb:
        lip0 = head("lip_low")
        pb["jaw_ctrl"].rotation_euler = (0.4, 0.0, 0.0)
        check((head("lip_low") - lip0).length > 0.002, "the lower lip rides on the jaw")
        reset()

bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=glb)
armatures = [o for o in bpy.data.objects if o.type == "ARMATURE"]
# Recent importers also add an "Icosphere" bone-display shape; only skinned meshes count.
meshes = [o for o in bpy.data.objects if o.type == "MESH" and o.parent in armatures]
check(len(armatures) == 1 and len(meshes) == 1, "glb imports as one armature and one skinned mesh")
check(len(meshes[0].vertex_groups) == len(armatures[0].data.bones), "glb carries skin weights for every bone")
print("blender check passed")
