"""Build a generated Blender rig in real Blender and check that it works.

Run with the `bpy` module (pip install bpy) or Blender itself:
    python blender_check.py <name>_blender.py <name>.glb
    blender --background --python blender_check.py -- <name>_blender.py <name>.glb
"""

import runpy
import sys

import bpy
from mathutils import Vector

args = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else sys.argv[1:]
script, glb = args[0], args[1]

bpy.ops.wm.read_factory_settings(use_empty=True)
saved_argv = sys.argv
sys.argv = ["blender"]
runpy.run_path(script, run_name="__main__")
sys.argv = saved_argv

rig = next(o for o in bpy.data.objects if o.type == "ARMATURE")
geo = next(o for o in bpy.data.objects if o.type == "MESH" and not o.name.startswith("WGT_"))
pb = rig.pose.bones
rest_bones = rig.data.bones


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

move("cog_ctrl", (0, 0, -0.1))
check(abs(head("head").z - rest_head("head").z + 0.1) < 1e-3, "COG moves the upper body")
check(max((a - b).length for a, b in zip(rest, deformed())) > 0.1, "skin follows the skeleton")

bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=glb)
armatures = [o for o in bpy.data.objects if o.type == "ARMATURE"]
# Recent importers also add an "Icosphere" bone-display shape; only skinned meshes count.
meshes = [o for o in bpy.data.objects if o.type == "MESH" and o.parent in armatures]
check(len(armatures) == 1 and len(meshes) == 1, "glb imports as one armature and one skinned mesh")
check(len(meshes[0].vertex_groups) == len(armatures[0].data.bones), "glb carries skin weights for every bone")
print("blender check passed")
