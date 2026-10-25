"""Send edits back from an FBX another program exported, the way the Unreal
tool does, with real Blender standing in for the other program.

    python -I blender_fbx_edits_check.py <name>.hive.json <name>.glb <hive-rig> <work dir>

Blender imports the rig's .glb with its glTF importer (as Unreal does), so the
mesh comes in with its own vertex order and frame. One vertex is repainted onto
the head and one shape key point is pushed out, and Blender's FBX exporter
writes the mesh (Y up, its own axes and unit scale). Then `hive-rig edits`
turns that FBX into an edits file, `weights import` and `shape import` take it,
and the check reads the rig back: the repainted vertex (and its seam copies)
and nothing else changed weights, the pushed point moved by the push, and every
other shape offset is exactly as it was. A rig without shapes checks weights only.
"""

import json
import os
import shutil
import subprocess
import sys

import bpy
from mathutils import Vector

args = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else sys.argv[1:]
rig_path, glb_path, exe, work = args[:4]
os.makedirs(work, exist_ok=True)
# Work on a copy of the rig so the run can repeat.
for f in os.listdir(os.path.dirname(os.path.abspath(rig_path))):
    if f.endswith((".hive.json", ".guides.json", ".glb")):
        shutil.copy(os.path.join(os.path.dirname(os.path.abspath(rig_path)), f), work)
rig_copy = os.path.join(work, os.path.basename(rig_path))
rig = json.load(open(rig_copy))
positions = rig["mesh"]["positions"]
bones = [b["name"] for b in rig["skeleton"]["bones"]]
shapes = {s["name"]: s for s in rig["mesh"].get("shapes", [])}
failures = []

bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=glb_path)
mesh = next(o for o in bpy.data.objects if o.type == "MESH" and (o.data.shape_keys or not shapes))
arm = next(o for o in bpy.data.objects if o.type == "ARMATURE")


def blender(p):
    """hive-rig model space (Y-up, facing +Z, metres) to Blender's (Z-up, facing -Y)."""
    return Vector((p[0], -p[2], p[1]))


def at(p):
    """Every Blender vertex at hive-rig position `p` (a seam splits one)."""
    want = blender(p)
    return [v.index for v in mesh.data.vertices if (mesh.matrix_world @ v.co - want).length < 1e-5]


# Repaint one vertex well inside the torso onto the head; push one shape point.
painted = len(positions) // 3
copies = at(positions[painted])
assert copies, "no Blender vertex at rig vertex %d" % painted
for g in mesh.vertex_groups:
    g.remove(copies)
mesh.vertex_groups["head"].add(copies, 1.0, "REPLACE")
shape_name = pushed = None
if shapes:
    shape_name = sorted(shapes)[0]
    shape = shapes[shape_name]
    pushed = shape["vertices"][len(shape["vertices"]) // 2]
    push = Vector((0.0, 0.0, 0.01))  # Blender's +Z: hive-rig's +Y
    key = mesh.data.shape_keys.key_blocks[shape_name]
    for i in at(positions[pushed]):
        key.data[i].co += push
fbx = os.path.join(work, "exported.fbx")
bpy.ops.object.select_all(action="SELECT")
bpy.ops.export_scene.fbx(filepath=fbx, use_selection=True, add_leaf_bones=False, bake_anim=False)


def run(*a):
    done = subprocess.run([exe] + [str(x) for x in a], capture_output=True, text=True)
    if done.returncode != 0:
        raise SystemExit("hive-rig %s failed: %s" % (a[0], done.stderr or done.stdout))
    print(done.stdout.strip().splitlines()[0])
    return done.stdout


edits = os.path.join(work, "exported.edits.json")
said = run("edits", rig_copy, fbx, "-o", edits, "--source", "blender")
if "%d of %d vertices matched" % (len(positions), len(positions)) not in said:
    failures.append("not every vertex matched: " + said)
run("weights", "import", rig_copy, edits, "-o", work)
if shapes:
    run("shape", rig_copy, "import", edits)
after = json.load(open(rig_copy))


def weights(r, v):
    s = r["skin"]
    return {r["skeleton"]["bones"][j]["name"]: w for j, w in zip(s["joints"][v], s["weights"][v]) if w > 0}


same_place = {v for v, p in enumerate(positions) if (Vector(p) - Vector(positions[painted])).length < 1e-6}
changed = {v for v in range(len(positions)) if weights(after, v) != weights(rig, v)}
if changed != same_place:
    failures.append("weights changed on %s, expected %s" % (sorted(changed)[:10], sorted(same_place)))
if abs(weights(after, painted).get("head", 0.0) - 1.0) > 1e-6:
    failures.append("vertex %d: %s, expected all on the head" % (painted, weights(after, painted)))
for name, s in shapes.items():
    got = next(x for x in after["mesh"]["shapes"] if x["name"] == name)
    before = dict(zip(s["vertices"], s["offsets"]))
    now = dict(zip(got["vertices"], got["offsets"]))
    for v in set(before) | set(now):
        want = list(before.get(v, [0.0, 0.0, 0.0]))
        if name == shape_name and (Vector(positions[v]) - Vector(positions[pushed])).length < 1e-6:
            want[1] += 0.01
            if (Vector(now.get(v, [0, 0, 0])) - Vector(want)).length > 1e-5:
                failures.append("%s: vertex %d at %s, expected %s" % (name, v, now.get(v), want))
        elif now.get(v, [0.0, 0.0, 0.0]) != want:
            failures.append("%s: vertex %d moved from %s to %s" % (name, v, want, now.get(v)))
    if got.get("driver") != s.get("driver") or len(got.get("inbetweens", [])) != len(s.get("inbetweens", [])):
        failures.append("%s lost its driver or in-betweens" % name)

if failures:
    print("\n".join(failures[:20]))
    raise SystemExit("FBX edits check: %d failures" % len(failures))
print("FBX edits check: one repainted vertex (%d copies)%s came back from Blender's FBX, "
      "nothing else moved: ok" % (len(same_place), " and one pushed shape point" if shapes else ""))
