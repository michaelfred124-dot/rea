"""Import a hive-rig .fbx with Blender's FBX importer and check it against the rig.

Run with the `bpy` module (pip install bpy) or Blender itself:
    python blender_fbx_check.py <name>.fbx <name>.hive.json
    blender --background --python blender_fbx_check.py -- <name>.fbx <name>.hive.json

Checks the bone count, names, hierarchy and rest positions, the mesh, every
vertex's weights (all influences, however many), and that posing a bone
deforms the skin it weighs on and nothing else.
"""

import json
import math
import sys

import bpy
from mathutils import Vector

args = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else sys.argv[1:]
fbx_path, rig_path = args[0], args[1]
rig = json.load(open(rig_path))
bones = rig["skeleton"]["bones"]
positions = rig["mesh"]["positions"]
skin = rig["skin"]

bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.fbx(filepath=fbx_path)

arm = next(o for o in bpy.data.objects if o.type == "ARMATURE")
mesh = next(o for o in bpy.data.objects if o.type == "MESH")
failures = []


def fail(msg):
    failures.append(msg)


def blender(p):
    """hive-rig model space (Y-up, facing +Z, metres) to Blender's (Z-up, facing -Y)."""
    return Vector((p[0], -p[2], p[1]))


TOL = 1e-4  # metres

# Skeleton: count, names, hierarchy, rest positions.
names = [b["name"] for b in bones]
got = {b.name: b for b in arm.data.bones}
if len(got) != len(bones):
    fail(f"{len(got)} bones imported, the rig has {len(bones)}")
missing = [n for n in names if n not in got]
extra = [n for n in got if n not in names]
if missing or extra:
    fail(f"bone names differ: missing {missing}, extra {extra}")
for b in bones:
    eb = got.get(b["name"])
    if eb is None:
        continue
    want_parent = bones[b["parent"]]["name"] if b["parent"] is not None else None
    have_parent = eb.parent.name if eb.parent else None
    if want_parent != have_parent:
        fail(f"{b['name']}: parent {have_parent}, expected {want_parent}")
    head = arm.matrix_world @ eb.head_local
    if (head - blender(b["head"])).length > TOL:
        fail(f"{b['name']}: rest head {tuple(head)}, expected {tuple(blender(b['head']))}")

# Mesh: vertex count and positions, and that it is skinned to the armature.
verts = mesh.data.vertices
if len(verts) != len(positions):
    fail(f"{len(verts)} vertices imported, the rig has {len(positions)}")
if not any(m.type == "ARMATURE" and m.object == arm for m in mesh.modifiers):
    fail("mesh has no Armature modifier on the imported armature")
worst = max((mesh.matrix_world @ v.co - blender(p)).length for v, p in zip(verts, positions))
if worst > TOL:
    fail(f"vertex positions off by up to {worst:.6f} m")

# Weights: every influence of every vertex, by bone name.
groups = {g.index: g.name for g in mesh.vertex_groups}
widest = 0
bad = 0
for i, v in enumerate(verts):
    have = {groups[g.group]: g.weight for g in v.groups if g.weight > 0}
    want = {}
    for j, w in zip(skin["joints"][i], skin["weights"][i]):
        if w > 0:
            want[names[j]] = want.get(names[j], 0.0) + w
    widest = max(widest, len(want))
    if set(have) != set(want) or any(abs(have[n] - want[n]) > 1e-5 for n in want):
        bad += 1
        if bad <= 5:
            fail(f"vertex {i}: weights {have}, expected {want}")
if bad > 5:
    fail(f"... {bad} vertices with wrong weights in all")

# Bind pose: posing a bone moves the skin it weighs on, and only that skin.
def deformed():
    bpy.context.view_layer.update()
    dg = bpy.context.evaluated_depsgraph_get()
    ev = mesh.evaluated_get(dg)
    return [mesh.matrix_world @ v.co for v in ev.data.vertices]


rest = deformed()
if max((a - mesh.matrix_world @ v.co).length for a, v in zip(rest, verts)) > TOL:
    fail("the skin moves at rest: bind pose and rest pose disagree")
# Probe the bone weighing on the most vertices whose parent is not the root.
counts = [0] * len(bones)
for js, ws in zip(skin["joints"], skin["weights"]):
    for j, w in zip(js, ws):
        if w > 0:
            counts[j] += 1
probe = max(
    (b for b in bones if b["parent"] is not None and bones[b["parent"]]["parent"] is not None),
    key=lambda b: counts[names.index(b["name"])],
)
pb = arm.pose.bones[probe["name"]]
pb.rotation_mode = "XYZ"
pb.rotation_euler = (math.radians(40), 0, 0)
posed = deformed()
idx = names.index(probe["name"])
moved_free = 0
moved_weighted = 0


def under(k):
    while k is not None:
        if k == idx:
            return True
        k = bones[k]["parent"]
    return False


for i in range(len(verts)):
    moves = (posed[i] - rest[i]).length > 1e-5
    influenced = any(w > 0 and under(j) for j, w in zip(skin["joints"][i], skin["weights"][i]))
    if moves and not influenced:
        moved_free += 1
    if influenced and moves:
        moved_weighted += 1
if moved_weighted == 0:
    fail(f"posing {probe['name']} moved none of its skin")
if moved_free:
    fail(f"posing {probe['name']} moved {moved_free} vertices it doesn't weigh on")
pb.rotation_euler = (0, 0, 0)

if failures:
    print("FBX CHECK FAILED")
    for f in failures:
        print("  " + f)
    sys.exit(1)
print(
    f"FBX OK: {len(bones)} bones (names, parents, rest heads), {len(verts)} vertices, "
    f"weights up to {widest} influences, posing {probe['name']} moved {moved_weighted} skinned vertices only"
)
