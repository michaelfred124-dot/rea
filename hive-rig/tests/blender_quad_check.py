"""Build a generated quadruped rig (`hive-rig demo --quadruped`) in real Blender
and check that its legs work.

Run with the `bpy` module (pip install bpy) or Blender itself:
    python blender_quad_check.py <name>_blender.py <name>.glb
    blender --background --python blender_quad_check.py -- <name>_blender.py <name>.glb
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

# Checks use the internal names; another naming convention is mapped the way hive-rig does.
NAMING = json.loads(open(script).read().split('r"""', 1)[1].split('"""', 1)[0])["guides"].get("naming")


class Named:
    def __init__(self, items):
        self.items = items

    def __getitem__(self, internal):
        return self.items[hive_names.final(NAMING, internal)]

    def __iter__(self):
        return iter(self.items)


pb = Named(rig.pose.bones)
rest_bones = Named(rig.data.bones)
# Blender is Z-up; the character faces -Y (hive +Z), so "forward" is -Y.
FORWARD = Vector((0, -1, 0))


def update():
    rig.update_tag()
    bpy.context.view_layer.update()


def head(name):
    update()
    return rig.matrix_world @ pb[name].head


def tail(name):
    update()
    return rig.matrix_world @ pb[name].tail


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


def reset():
    for p in pb:
        p.location = (0, 0, 0)
        p.rotation_euler = (0, 0, 0)
        p.rotation_quaternion = (1, 0, 0, 0)
        p.scale = (1, 1, 1)
        for prop in ("roll", "stretch", "space"):
            if prop in p.keys():
                p[prop] = 0
        if "ik_fk" in p.keys():
            p["ik_fk"] = 1.0
    update()


def check(cond, msg):
    if not cond:
        raise SystemExit("FAIL: " + msg)
    print("ok:", msg)


rest = [v.co.copy() for v in geo.data.vertices]
check(max((a - b).length for a, b in zip(rest, deformed())) < 1e-4, "rig adds no offset in the rest pose")

drivers = rig.animation_data.drivers
check(all(fc.driver.is_valid for fc in drivers), "all %d drivers are valid" % len(drivers))
check(all(fc.driver.type != "SCRIPTED" or fc.driver.is_simple_expression for fc in drivers),
      "drivers use simple expressions (no Python auto-run needed)")

for s in ("l", "r"):
    reset()
    foot_ik = "foot_%s_ik_ctrl" % s
    check(pb[foot_ik]["ik_fk"] == 1.0, "hind leg %s starts in IK" % s)
    # The pole stands straight ahead of the knee.
    off = head("calf_%s_pole_ctrl" % s) - rest_head("calf_" + s)
    check(off.dot(FORWARD) > 0.9 * off.length and abs(off.x) < 0.05 * off.length,
          "hind %s pole is straight ahead of the knee" % s)

    # Lift the paw: it follows, the hock stays on the paw and the knee folds forward.
    lift = Vector((0, 0, 0.08))
    move(foot_ik, lift)
    check((head("foot_" + s) - (rest_head("foot_" + s) + lift)).length < 1e-3, "paw %s follows its IK control" % s)
    check((tail("hock_" + s) - head("foot_" + s)).length < 1e-3, "hock %s stays joined to the paw" % s)
    check((head("hock_" + s) - (rest_head("hock_" + s) + lift)).length < 1e-3,
          "hock %s rides up with the paw (the hock control keeps its angle)" % s)
    knee_fwd = (head("calf_" + s) - rest_head("calf_" + s)).dot(FORWARD)
    check(knee_fwd > 0.005, "knee %s folds forward (%.3f m)" % (s, knee_fwd))

    # The hock control swings the hock about the ankle; the paw stays put.
    reset()
    hock0, paw0 = head("hock_" + s), head("foot_" + s)
    pb["hock_%s_ik_ctrl" % s].rotation_euler = (0.3, 0.0, 0.0)
    moved = (head("hock_" + s) - hock0).length
    check(moved > 0.02 and (head("foot_" + s) - paw0).length < 1e-3,
          "hock %s control swings the hock (%.3f m) and the paw stays planted" % (s, moved))
    check((tail("calf_" + s) - head("hock_" + s)).length < 1e-3, "knee solve follows the hock %s" % s)

    # FK: the hock FK control turns everything below it.
    reset()
    pb[foot_ik]["ik_fk"] = 0.0
    pb["hock_%s_fk_ctrl" % s].rotation_euler = (0.4, 0.0, 0.0)
    check((head("foot_" + s) - rest_head("foot_" + s)).length > 0.02, "hock %s FK control moves the paw" % s)
    pb["hock_%s_fk_ctrl" % s].rotation_euler = (0.0, 0.0, 0.0)
    pb["thigh_%s_fk_ctrl" % s].rotation_euler = (0.4, 0.0, 0.0)
    check((head("hock_" + s) - rest_head("hock_" + s)).length > 0.03, "thigh %s FK control swings the leg" % s)

    # Front legs start in IK and their elbows fold backward.
    reset()
    hand_ik = "hand_%s_ik_ctrl" % s
    check(pb[hand_ik]["ik_fk"] == 1.0, "front leg %s starts in IK" % s)
    move(hand_ik, (0, 0, 0.06))
    check((head("hand_" + s) - (rest_head("hand_" + s) + Vector((0, 0, 0.06)))).length < 1e-3,
          "front paw %s follows its IK control" % s)
    elbow_back = -(head("lowerarm_" + s) - rest_head("lowerarm_" + s)).dot(FORWARD)
    check(elbow_back > 0.005, "front elbow %s folds backward (%.3f m)" % (s, elbow_back))

# Reverse foot on the hind paw.
reset()
ankle0, ball0, toe0 = rest_head("foot_l"), rest_head("ball_l"), rest_bones["ball_l"].tail_local
pb["foot_l_ik_ctrl"]["roll"] = -25.0
check(head("ball_l").z > ball0.z + 0.005 and abs(head("foot_l").z - ankle0.z) < 0.02,
      "negative roll rocks the paw back on its heel")
pb["foot_l_ik_ctrl"]["roll"] = 25.0
check(head("foot_l").z > ankle0.z + 0.005 and (head("ball_l") - ball0).length < 0.005,
      "positive roll lifts the paw around the pads")
check((tail("hock_l") - head("foot_l")).length < 1e-3, "the hock follows the rolled paw")
pb["foot_l_ik_ctrl"]["roll"] = 70.0
check(head("ball_l").z > ball0.z + 0.005 and (tail("ball_l") - toe0).length < 0.01,
      "past the break angle the paw pivots on the toe tip")

# Stretch: pull the paw out of reach, straight away from the hip.
reset()
away = (rest_head("hock_l") - rest_head("thigh_l")).normalized() * 0.2
move("foot_l_ik_ctrl", away)
goal = rest_head("foot_l") + away
check((head("foot_l") - goal).length > 0.03, "without stretch the hind leg stops short of a far goal")
pb["foot_l_ik_ctrl"]["stretch"] = 1.0
check((head("foot_l") - goal).length < 0.01, "with stretch the hind leg reaches a far goal")

# Skin: a hind paw moves its own leg's skin and not the front leg on the same side.
reset()
front_paw = rig.matrix_world @ rest_head("hand_l")
hind_paw = rig.matrix_world @ rest_head("foot_l")
near_front = [i for i, v in enumerate(rest) if (geo.matrix_world @ v - front_paw).length < 0.05]
near_hind = [i for i, v in enumerate(rest) if (geo.matrix_world @ v - hind_paw).length < 0.05]
move("foot_l_ik_ctrl", (0, 0, 0.08))
after = deformed()
check(near_front and max((after[i] - rest[i]).length for i in near_front) < 1e-4,
      "lifting the hind paw leaves the front paw's skin alone")
check(near_hind and min((after[i] - rest[i]).length for i in near_hind) > 0.04,
      "lifting the hind paw carries its skin")

reset()
move("cog_ctrl", (0, 0, -0.05))
check(abs(head("head").z - rest_head("head").z + 0.05) < 1e-3, "COG moves the body")
check((head("foot_l") - rest_head("foot_l")).length < 1e-3, "the paws stay planted when the body moves")

bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=glb)
armatures = [o for o in bpy.data.objects if o.type == "ARMATURE"]
meshes = [o for o in bpy.data.objects if o.type == "MESH" and o.parent in armatures]
check(len(armatures) == 1 and len(meshes) == 1, "glb imports as one armature and one skinned mesh")
check(len(meshes[0].vertex_groups) == len(armatures[0].data.bones), "glb carries skin weights for every bone")
print("blender quadruped check passed")
