"""Check the cone and RBF pose readers in real Blender against hive-rig's own
numbers, pose by pose.

    hive-rig pose out/hive_demo.hive.json --reset --set ... --readers out/p1.json
    hive-rig pose out/hive_demo.hive.json --reset --set ... --readers out/p2.json
    python -I blender_reader_check.py out/hive_demo_blender.py out/p1.json out/p2.json ...

Each answer file is what hive-rig worked out for one pose: every reader's
outputs, every shape's weight, each helper joint's skinning matrix, where the
skin puts the vertices the helpers move, and the pose itself (`studio_pose`,
the controls' moves). The builder builds the rig; for each pose the check
puts the controls there, lets Blender's drivers run, and compares:

* each reader's value on the armature (`hive_reader_<name>`, `_<k>`),
* each reader-driven shape key's value,
* each helper joint's posed transform (as a move of hive-rig's model space),
* the deformed mesh at the helpers' vertices.

The rig must also come back to rest (every reader output as at rest).
"""

import json
import runpy
import sys

import bpy
from mathutils import Matrix, Vector

args = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else sys.argv[1:]
script, answers = args[0], [json.load(open(p)) for p in args[1:]]
TOL = 2e-3  # weights
TOL_M = 1.5e-3  # metres

bpy.ops.wm.read_factory_settings(use_empty=True)
saved = sys.argv
sys.argv = ["blender"]
ns = runpy.run_path(script, run_name="__main__")
sys.argv = saved
RIG = json.loads(open(script).read().split('r"""', 1)[1].split('"""', 1)[0])
rig = next(o for o in bpy.data.objects if o.type == "ARMATURE")
readers = RIG.get("pose_readers", [])
if not readers:
    raise SystemExit("FAIL: the rig has no cone or RBF readers to check")
failures = []
TURN = Matrix(((1, 0, 0, 0), (0, 0, -1, 0), (0, 1, 0, 0), (0, 0, 0, 1)))  # hive-rig axes to Blender's


def fail(msg):
    failures.append(msg)
    print("FAIL: " + msg)


def evaluated():
    rig.update_tag()
    bpy.context.view_layer.update()
    return bpy.context.evaluated_depsgraph_get()


def reader_values(dg):
    ev = rig.evaluated_get(dg)
    out = {}
    for r in readers:
        n = len(r["shapes"]) + len(r["helpers"])
        if r["kind"] == "cone":
            out[r["name"]] = [ev["hive_reader_" + r["name"]]] * n
        else:
            out[r["name"]] = [ev["hive_reader_%s_%d" % (r["name"], k)] for k in range(n)]
    return out


def shape_values(dg):
    out = {}
    for o in bpy.data.objects:
        keys = o.data.shape_keys if o.type == "MESH" else None
        if keys is None:
            continue
        ev = keys.evaluated_get(dg)
        for kb in ev.key_blocks:
            out[kb.name] = kb.value
    return out


def helper_move(dg, name):
    """The helper's transform from rest to posed, in hive-rig's model space."""
    pb = rig.evaluated_get(dg).pose.bones[name]
    m = rig.matrix_world @ pb.matrix @ pb.bone.matrix_local.inverted()
    return TURN.inverted() @ m @ TURN


def mesh_points(dg):
    out = {}
    for o in bpy.data.objects:
        if o.type != "MESH" or o.parent != rig:
            continue
        start = o.get("hive_first_vertex", 0)
        ev = o.evaluated_get(dg)
        for i, v in enumerate(ev.data.vertices):
            co = o.matrix_world @ v.co
            out[start + i] = Vector((co[0], co[2], -co[1]))
    return out


def driven_shapes():
    """Shapes a reader drives that are a single shape key (in-betweens are blended
    from the same weight, which the shape-key drivers of the corrective check cover)."""
    names = set()
    for r in readers:
        for s in r["shapes"]:
            shape = next((x for x in RIG["mesh"]["shapes"] if x["name"] == s), None)
            if shape is not None and not shape.get("inbetweens"):
                names.add(s)
    return names


def helpers_held(hold, saved=None):
    """Mute (or, with `saved`, bring back) the drivers moving the helper joints,
    leaving them at rest under their parents."""
    names = {h["name"] for r in readers for h in r["helpers"]}
    curves = [fc for fc in rig.animation_data.drivers
              if any(fc.data_path.startswith('pose.bones["%s"]' % n) for n in names)]
    for fc in curves:
        fc.mute = hold
    if hold:
        for n in names:
            pb = rig.pose.bones[n]
            pb.location = (0, 0, 0)
            pb.scale = (1, 1, 1)
    return curves


rest_values = reader_values(evaluated())
worst = {"reader": 0.0, "shape": 0.0, "helper": 0.0, "mesh": 0.0}
moved = 0.0
for k, ans in enumerate(answers):
    label = "pose %d" % (k + 1)
    if ns["add_studio_pose"](rig, ans, label) == 0:
        fail("%s: the answer has no studio pose" % label)
        continue
    ns["apply_pose"](rig, label)
    dg = evaluated()
    got = reader_values(dg)
    on = 0.0
    for name, want in ans["readers"].items():
        for o, (g, w) in enumerate(zip(got[name], want)):
            d = abs(g - w)
            worst["reader"] = max(worst["reader"], d)
            on = max(on, w)
            if d > TOL:
                fail("%s: reader %s output %d is %.4f in Blender, %.4f in hive-rig" % (label, name, o, g, w))
    shapes = shape_values(dg)
    for s in driven_shapes():
        d = abs(shapes.get(s, -1.0) - ans["shapes"][s])
        worst["shape"] = max(worst["shape"], d)
        if d > TOL:
            fail("%s: shape key %s is %.4f in Blender, %.4f in hive-rig" % (label, s, shapes.get(s, -1.0), ans["shapes"][s]))
    for name, h in ans["helpers"].items():
        want = Matrix([[h["matrix"][c * 4 + r] for c in range(4)] for r in range(4)])
        got_m = helper_move(dg, name)
        # Compare where the move puts points around the helper's rest head.
        bone = next(b for b in RIG["skeleton"]["bones"] if b["name"] == name)
        head = Vector(bone["head"])
        for p in (head, head + Vector((0.05, 0, 0)), head + Vector((0, 0.05, 0)), head + Vector((0, 0, 0.05))):
            d = ((got_m @ p.to_4d()).to_3d() - (want @ p.to_4d()).to_3d()).length
            worst["helper"] = max(worst["helper"], d)
            if d > TOL_M:
                fail("%s: helper %s puts a point %.4f m from where hive-rig does" % (label, name, d))
                break
    # What the helpers do to the skin: the mesh with them driven, less the mesh
    # with them riding on their parents (both programs' other bones, twist
    # helpers among them, cancel out of the difference).
    points = mesh_points(dg)
    held = helpers_held(True)
    still = mesh_points(evaluated())
    helpers_held(False, held)
    for v, p in ans.get("vertices", {}).items():
        want = Vector(p["with"]) - Vector(p["without"])
        got_d = points[int(v)] - still[int(v)]
        d = (got_d - want).length
        worst["mesh"] = max(worst["mesh"], d)
        moved = max(moved, want.length)
        if d > TOL_M:
            fail("%s: the helpers move vertex %s %.4f m away from how hive-rig's skin moves it" % (label, v, d))
            break
    print("ok: %s: readers within %.4f, %d shape keys, %d helpers, %d helper vertices (largest output %.2f)"
          % (label, worst["reader"], len(driven_shapes()), len(ans["helpers"]), len(ans.get("vertices", {})), on))
    ns["reset_pose"](rig)

back = reader_values(evaluated())
for name, values in rest_values.items():
    if any(abs(a - b) > 1e-4 for a, b in zip(values, back[name])):
        fail("reader %s didn't come back to its rest value" % name)
if not any(max(a["readers"][r["name"]] or [0.0]) > 0.5 for a in answers for r in readers):
    fail("no pose turned a reader on past 0.5: are these poses of the read bones?")
if any(r["helpers"] for r in readers) and moved < 2e-3:
    fail("no pose moved a vertex through a helper joint by 2 mm (largest %.4f m)" % moved)
if failures:
    raise SystemExit("FAIL: pose reader check (%d)" % len(failures))
print("Cone and RBF readers in Blender match hive-rig over %d poses: outputs within %.4f, shape keys within %.4f, "
      "helpers within %.2f mm, their moves of the skin (up to %.1f mm) within %.2f mm: ok"
      % (len(answers), worst["reader"], worst["shape"], worst["helper"] * 1000, moved * 1000, worst["mesh"] * 1000))
