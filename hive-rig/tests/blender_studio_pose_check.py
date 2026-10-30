"""Check that a pose posed in hive-rig (the app's Rig stage, or `hive-rig pose`)
reaches Blender's pose library as "Studio pose" and, applied, puts the bones
where hive-rig's own evaluator puts them.

    hive-rig pose out/hive_demo.hive.json --set ... --bones out/posed.json
    python blender_studio_pose_check.py out/hive_demo_blender.py out/posed.json

`posed.json` is hive-rig's answer: each bone's head and tail in the pose
(metres, Y up). The builder builds the rig at rest with the pose in its
library; the check applies it and compares every deforming bone, then resets.
"""

import json
import runpy
import sys

import bpy
from mathutils import Vector

args = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else sys.argv[1:]
script, answer = args[0], json.load(open(args[1]))
TOL = float(args[2]) if len(args) > 2 else 2e-3  # metres

bpy.ops.wm.read_factory_settings(use_empty=True)
saved = sys.argv
sys.argv = ["blender"]
ns = runpy.run_path(script, run_name="__main__")
sys.argv = saved
rig = next(o for o in bpy.data.objects if o.type == "ARMATURE")
failures = []


def to_blender(p):
    return Vector((p[0], -p[2], p[1]))


def update():
    rig.update_tag()
    bpy.context.view_layer.update()


def heads():
    update()
    return {b.name: rig.matrix_world @ b.head for b in rig.pose.bones if rig.data.bones[b.name].use_deform}


rest = heads()
library = ns["pose_library"](rig)
if "Studio pose" not in library:
    raise SystemExit("FAIL: no Studio pose in the library (has %s)" % sorted(library))
# Built at rest: the pose is only in the library.
moved = [n for n, h in rest.items() if (h - rig.data.bones[n].head_local).length > 1e-5]
if moved:
    failures.append("built posed, not at rest: %s" % moved[:5])
ns["apply_pose"](rig, "Studio pose")
posed = heads()
worst, where = 0.0, None
moved_any = False
for name, h in posed.items():
    want = to_blender(answer[name]["head"])
    d = (h - want).length
    if d > worst:
        worst, where = d, name
    if (want - rest[name]).length > 0.01:
        moved_any = True
if worst > TOL:
    failures.append("%s: %.4f m from hive-rig's pose" % (where, worst))
if not moved_any:
    failures.append("the pose moved no bone: is it a pose at all?")
ns["reset_pose"](rig)
back = heads()
if any((back[n] - rest[n]).length > 1e-5 for n in rest):
    failures.append("reset didn't bring the rest pose back")
if failures:
    print("\n".join(failures))
    raise SystemExit("FAIL: studio pose check")
print("Studio pose in Blender matches hive-rig's evaluator on %d deforming bones (within %.2f mm): ok"
      % (len(posed), worst * 1000))
