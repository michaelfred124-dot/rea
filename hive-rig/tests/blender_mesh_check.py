"""Check that a rigged character still looks like its source file, in real Blender.

Run with the `bpy` module (pip install bpy), isolated from the folder the
files came from:
    python -I blender_mesh_check.py <name>_blender.py --source <source.glb> [--fbx <name>.fbx]
    python -I blender_mesh_check.py <name>_blender.py --corrective SHAPE CONTROL [--fbx <name>.fbx]

The source .glb is read here with a small reader of its own (not hive-rig's
importer, not Blender's), as data only. After building the rig with the
generated script, it checks against the source (turned upright as the script
builds it, when the rig records that the source stood otherwise; the FBX is
checked against the source as it stands):

* the mesh objects: one per mesh node, with the source's material pieces;
* UVs (every set), per object, matched by vertex position (order-free);
* materials by name, and that every texture the source uses is loaded;
* every morph target as a shape key with the same offsets.

With --fbx, the same UV, material and shape checks run on the .fbx imported
with Blender's FBX importer. With --corrective, the shape's driver must stay
off at rest and come on as the control bends the joint it reads.
"""

import json
import math
import runpy
import struct
import sys

import bpy
from mathutils import Matrix, Quaternion, Vector

args = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else sys.argv[1:]
script = args[0]


def opt(name, n=1):
    if name not in args:
        return None
    i = args.index(name)
    return args[i + 1] if n == 1 else args[i + 1 : i + 1 + n]


source_path, fbx_path, corrective = opt("--source"), opt("--fbx"), opt("--corrective", 2)
failures = []


def fail(msg):
    failures.append(msg)
    print("FAIL: " + msg)


def ok(msg):
    print("ok: " + msg)


# --------------------------------------------------------------------------- a small glTF reader
COMPONENTS = {5120: ("b", 1), 5121: ("B", 1), 5122: ("h", 2), 5123: ("H", 2), 5125: ("I", 4), 5126: ("f", 4)}
WIDTH = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4, "MAT4": 16}
NORMALIZE = {5120: 127.0, 5121: 255.0, 5122: 32767.0, 5123: 65535.0}


def read_glb(path):
    data = open(path, "rb").read()
    assert data[:4] == b"glTF", "not a .glb"
    n = struct.unpack_from("<I", data, 12)[0]
    doc = json.loads(data[20 : 20 + n])
    rest = 20 + n
    blob = b""
    if rest + 8 <= len(data):
        m = struct.unpack_from("<I", data, rest)[0]
        blob = data[rest + 8 : rest + 8 + m]
    return doc, blob


def accessor(doc, blob, index):
    acc = doc["accessors"][index]
    fmt, size = COMPONENTS[acc["componentType"]]
    width = WIDTH[acc["type"]]
    count = acc["count"]
    out = [[0.0] * width for _ in range(count)]
    if "bufferView" in acc:
        view = doc["bufferViews"][acc["bufferView"]]
        assert view.get("buffer", 0) == 0, "external buffers aren't read here"
        stride = view.get("byteStride") or size * width
        base = view.get("byteOffset", 0) + acc.get("byteOffset", 0)
        for i in range(count):
            out[i] = list(struct.unpack_from("<" + fmt * width, blob, base + i * stride))
    sparse = acc.get("sparse")
    if sparse:
        iv = sparse["indices"]
        ifmt, isize = COMPONENTS[iv["componentType"]]
        iview = doc["bufferViews"][iv["bufferView"]]
        vview = doc["bufferViews"][sparse["values"]["bufferView"]]
        for k in range(sparse["count"]):
            at = struct.unpack_from("<" + ifmt, blob, iview.get("byteOffset", 0) + iv.get("byteOffset", 0) + k * isize)[0]
            off = vview.get("byteOffset", 0) + sparse["values"].get("byteOffset", 0) + k * size * width
            out[at] = list(struct.unpack_from("<" + fmt * width, blob, off))
    if acc.get("normalized") and acc["componentType"] in NORMALIZE:
        out = [[max(x / NORMALIZE[acc["componentType"]], -1.0) for x in row] for row in out]
    return out


def node_matrix(node):
    if "matrix" in node:
        m = node["matrix"]
        return Matrix([[m[c * 4 + r] for c in range(4)] for r in range(4)])
    t = Matrix.Translation(Vector(node.get("translation", [0, 0, 0])))
    x, y, z, w = node.get("rotation", [0, 0, 0, 1])
    r = Quaternion((w, x, y, z)).to_matrix().to_4x4()
    s = Matrix.Diagonal(Vector(node.get("scale", [1, 1, 1]) + [1]))
    return t @ r @ s


def source_objects(path):
    """Each mesh node of the default scene: name, triangle pieces with their
    material names, vertices (position, UVs per set) and morph offsets, in
    hive-rig's model space (glTF's), baked into world space."""
    doc, blob = read_glb(path)
    images_of = {}
    out = []

    def walk(i, parent):
        node = doc["nodes"][i]
        world = parent @ node_matrix(node)
        if "mesh" in node:
            mesh = doc["meshes"][node["mesh"]]
            names = (mesh.get("extras") or {}).get("targetNames", [])
            lin = world.to_3x3()
            obj = {"name": node.get("name") or mesh.get("name"), "materials": [], "verts": [], "targets": {}, "textures": set()}
            for prim in mesh["primitives"]:
                if prim.get("mode", 4) != 4:
                    continue
                attrs = prim["attributes"]
                pos = accessor(doc, blob, attrs["POSITION"])
                used = sorted(set(int(i[0]) for i in accessor(doc, blob, prim["indices"]))) if "indices" in prim else list(range(len(pos)))
                uvs = []
                k = 0
                while "TEXCOORD_%d" % k in attrs:
                    uvs.append(accessor(doc, blob, attrs["TEXCOORD_%d" % k]))
                    k += 1
                mat = prim.get("material")
                obj["materials"].append(doc["materials"][mat].get("name") if mat is not None else None)
                if mat is not None:
                    m = doc["materials"][mat]
                    for key, tex in list(m.get("pbrMetallicRoughness", {}).items()) + list(m.items()):
                        if key.endswith("Texture") and isinstance(tex, dict):
                            obj["textures"].add(doc["textures"][tex["index"]]["source"])
                for v in used:
                    p = world @ Vector(pos[v])
                    obj["verts"].append((tuple(p), [tuple(u[v]) for u in uvs]))
                for t, target in enumerate(prim.get("targets", [])):
                    name = names[t] if t < len(names) and names[t] else "target_%d" % t
                    deltas = accessor(doc, blob, target["POSITION"]) if "POSITION" in target else []
                    lst = obj["targets"].setdefault(name, [])
                    for v in used:
                        if deltas and Vector(deltas[v]).length > 1e-7:
                            lst.append((tuple(world @ Vector(pos[v])), tuple(lin @ Vector(deltas[v]))))
            out.append(obj)
        for c in node.get("children", []):
            walk(c, world)

    scene = doc["scenes"][doc.get("scene", 0)]
    for i in scene["nodes"]:
        walk(i, Matrix.Identity(4))
    return [o for o in out if o["verts"]], doc


def blender(p):
    """glTF / hive-rig space (Y-up, facing +Z) to Blender's (Z-up, facing -Y)."""
    return Vector((p[0], -p[2], p[1]))


def key(p, digits=4):
    return tuple(round(x, digits) + 0.0 for x in p)


# --------------------------------------------------------------------------- what Blender has
def object_vertices(obj):
    """(position in hive-rig space, [uv per set in glTF's convention]) per vertex."""
    me = obj.data
    world = obj.matrix_world
    per_vertex = [None] * len(me.vertices)
    layers = list(me.uv_layers)
    for loop in me.loops:
        v = loop.vertex_index
        uvs = [tuple((l.data[loop.index].uv[0], 1.0 - l.data[loop.index].uv[1])) for l in layers]
        if per_vertex[v] is None:
            per_vertex[v] = []
        per_vertex[v].append(uvs)
    out = []
    for v in me.vertices:
        p = world @ v.co
        hive = (p.x, p.z, -p.y)
        for uvs in per_vertex[v.index] or [[]]:
            out.append((hive, uvs))
    return out


def compare_uvs(label, src, objs):
    """Every (position, UV sets) of the source object appears in Blender's and
    the other way round: the same surface with the same mapping."""
    want = set((key(p), tuple(key(u, 3) for u in uvs)) for p, uvs in src["verts"])
    have = set()
    for obj in objs:
        for p, uvs in object_vertices(obj):
            have.add((key(p), tuple(key(u, 3) for u in uvs)))
    sets = len(src["verts"][0][1]) if src["verts"] else 0
    layers = {len(o.data.uv_layers) for o in objs}
    if layers != {sets}:
        fail("%s: %s UV sets in Blender, %d in the source" % (label, sorted(layers), sets))
    missing = want - have
    extra = have - want
    if missing and extra:
        # Rounding can put a point on either side of a boundary (FBX stores
        # centimetres): pair what's left by distance instead.
        size = max(max(abs(c) for c in p) for p, _ in src["verts"]) or 1.0
        left = list(extra)
        for m in list(missing):
            for e in left:
                close = (Vector(m[0]) - Vector(e[0])).length < 1e-4 * size + 2e-4
                same_uv = len(m[1]) == len(e[1]) and all(
                    abs(a - b) < 2e-3 for u, w in zip(m[1], e[1]) for a, b in zip(u, w))
                if close and same_uv:
                    missing.discard(m)
                    left.remove(e)
                    break
        extra = set(left)
    if missing or extra:
        fail("%s: UVs differ: %d of %d source corners missing, %d extra (e.g. %s)"
             % (label, len(missing), len(want), len(extra), sorted(missing)[:2]))
    else:
        ok("%s: UVs match the source (%d vertex corners, %d set(s))" % (label, len(want), sets))


def compare_materials(label, src, obj, doc, texture_check=True):
    want = [m for m in src["materials"]]
    have = [s.material.name if s.material else None for s in obj.material_slots]
    want_names = sorted(set(m for m in want if m))
    have_names = sorted(set(h.split(".")[0] for h in have if h and not h.endswith("_mat")))
    if want_names != sorted(set(n for n in have_names)):
        fail("%s: materials %s, expected %s" % (label, have_names, want_names))
    else:
        ok("%s: materials %s" % (label, have_names))
    if not texture_check:
        return
    loaded = set()
    for s in obj.material_slots:
        m = s.material
        if m is None or not m.use_nodes:
            continue
        for n in m.node_tree.nodes:
            if n.type == "TEX_IMAGE" and n.image is not None and n.image.size[0] > 0:
                loaded.add(n.image)
    if len(loaded) < len(src["textures"]):
        fail("%s: %d textures loaded with pixels, the source uses %d" % (label, len(loaded), len(src["textures"])))
    else:
        ok("%s: %d texture image(s) loaded (%s)" % (label, len(loaded), ", ".join("%dx%d" % tuple(i.size) for i in loaded)))


def compare_shapes(label, src, objs, scale=1.0):
    for name, want in src["targets"].items():
        have = []
        found = False
        for obj in objs:
            keys = obj.data.shape_keys
            if keys is None or name not in keys.key_blocks:
                continue
            found = True
            basis = keys.reference_key
            kb = keys.key_blocks[name]
            lin = obj.matrix_world.to_3x3()
            for b, k in zip(basis.data, kb.data):
                d = lin @ (k.co - b.co)
                if d.length > 1e-6:
                    p = obj.matrix_world @ b.co
                    have.append(((p.x, p.z, -p.y), (d.x, d.z, -d.y)))
        if not found:
            fail("%s: no shape key '%s'" % (label, name))
            continue
        w = sorted((key(p), key(d)) for p, d in want)
        h = sorted((key(p), key(d)) for p, d in have)
        if w != h:
            bad = len(set(w) ^ set(h))
            fail("%s: shape '%s' offsets differ (%d of %d vertices)" % (label, name, bad, len(w)))
        else:
            ok("%s: shape '%s' keeps its %d vertex offsets" % (label, name, len(w)))


def by_name(objs, name):
    return next((o for o in objs if o.name.split(".")[0] in (name, name + "_geo") or o.name == name), None)


# --------------------------------------------------------------------------- build the rig
bpy.ops.wm.read_factory_settings(use_empty=True)
saved_argv = sys.argv
sys.argv = ["blender"]
runpy.run_path(script, run_name="__main__")
sys.argv = saved_argv
rig = next(o for o in bpy.data.objects if o.type == "ARMATURE")
built = [o for o in bpy.data.objects if o.type == "MESH" and o.parent == rig]
ok("built %d mesh object(s): %s" % (len(built), ", ".join(o.name for o in built)))

AXES = {"+x": (1, 0, 0), "-x": (-1, 0, 0), "+y": (0, 1, 0), "-y": (0, -1, 0), "+z": (0, 0, 1), "-z": (0, 0, -1)}


def upright(objs, orientation):
    """The source objects turned into hive-rig's frame (Y up, facing +Z), where
    the Blender script builds the character; the .glb and .fbx keep the
    source's own placement."""
    if not orientation:
        return objs
    up, fwd = Vector(AXES[orientation["up"]]), Vector(AXES[orientation["forward"]])
    turn = Matrix((up.cross(fwd), up, fwd))  # rows: source to standard

    def t(p):
        return tuple(turn @ Vector(p))

    out = []
    for o in objs:
        o = dict(o)
        o["verts"] = [(t(p), uvs) for p, uvs in o["verts"]]
        o["targets"] = {n: [(t(p), t(d)) for p, d in lst] for n, lst in o["targets"].items()}
        out.append(o)
    return out


if source_path:
    src, doc = source_objects(source_path)
    rig_data = json.loads(open(script).read().split('r"""', 1)[1].split('"""', 1)[0])
    if rig_data.get("orientation"):
        ok("the source stands up %(up)s, facing %(forward)s; the script builds it upright" % rig_data["orientation"])
    src_upright = upright(src, rig_data.get("orientation"))
    if len(built) != len(src):
        fail("%d mesh objects built, the source has %d" % (len(built), len(src)))
    else:
        ok("object count kept (%d)" % len(src))
    for s in src_upright:
        obj = by_name(built, s["name"]) if len(src) > 1 else built[0]
        if obj is None:
            fail("no object for the source's '%s'" % s["name"])
            continue
        pieces = len(set(s["materials"]))
        if len(obj.material_slots) != pieces:
            fail("%s: %d material slots, the source has %d pieces" % (s["name"], len(obj.material_slots), pieces))
        compare_uvs(s["name"], s, [obj])
        compare_materials(s["name"], s, obj, doc)
        compare_shapes(s["name"], s, [obj])
    # Skinned: the armature moves every object.
    for obj in built:
        if not any(m.type == "ARMATURE" and m.object == rig for m in obj.modifiers):
            fail("%s is not skinned to the rig" % obj.name)

if corrective:
    shape, control = corrective
    geo = next(o for o in built if o.data.shape_keys and shape in o.data.shape_keys.key_blocks)
    kb = geo.data.shape_keys.key_blocks[shape]
    reader = next(s for s in json.loads(open(script).read().split('r"""', 1)[1].split('"""', 1)[0])["mesh"]["shapes"]
                  if s["name"] == shape)["driver"]
    bone = reader["bone"]
    pb = rig.pose.bones[control]
    pb.rotation_mode = "XYZ"

    def evaluate():
        bpy.context.view_layer.update()
        dg = bpy.context.evaluated_depsgraph_get()
        return kb.id_data.evaluated_get(dg).key_blocks[shape].value

    def turned_by():
        """The read bone's turn from rest, in degrees, measured from the bones."""
        bpy.context.view_layer.update()
        pose = rig.pose.bones[bone]
        rest = rig.data.bones[bone]
        parent_rest = rest.parent.matrix_local if rest.parent else Matrix.Identity(4)
        parent_pose = pose.parent.matrix if pose.parent else Matrix.Identity(4)
        local = (parent_rest.inverted() @ rest.matrix_local).inverted() @ parent_pose.inverted() @ pose.matrix
        return math.degrees(local.to_quaternion().angle)

    def deformed():
        bpy.context.view_layer.update()
        dg = bpy.context.evaluated_depsgraph_get()
        ev = geo.evaluated_get(dg)
        return [v.co.copy() for v in ev.data.vertices]

    at_rest = evaluate()
    if abs(at_rest) > 1e-3:
        fail("%s is %.3f at rest; it should be off" % (shape, at_rest))
    best = (0.0, None, 0.0)
    for axis in range(3):
        for sign in (1, -1):
            e = [0.0, 0.0, 0.0]
            e[axis] = sign * math.radians(reader["end"])
            pb.rotation_euler = e
            v = evaluate()
            if v > best[0]:
                best = (v, (axis, sign), turned_by())
            pb.rotation_euler = (0, 0, 0)
    value, how, turn = best
    if how is None or value < 0.9:
        fail("bending %s never turned %s on (best %.3f)" % (control, shape, value))
    else:
        ok("bending %s by %.0f degrees (the joint turned %.1f) turns %s on: %.3f" % (control, reader["end"], turn, shape, value))
        # Half the bend: about half on; the other way: off.
        axis, sign = how
        e = [0.0, 0.0, 0.0]
        e[axis] = sign * math.radians(reader["end"] / 2)
        pb.rotation_euler = e
        half = evaluate()
        expected = (turned_by() - reader["start"]) / (reader["end"] - reader["start"])
        if abs(half - expected) > 0.1:
            fail("half bend: %s is %.3f, the bone turned for %.3f" % (shape, half, expected))
        else:
            ok("half bend: %s is %.3f (the bone's turn gives %.3f)" % (shape, half, expected))
        with_shape = deformed()
        kb.mute = True
        without = deformed()
        kb.mute = False
        moved = sum(1 for a, b in zip(with_shape, without) if (a - b).length > 1e-5)
        if moved == 0:
            fail("the corrective shape doesn't move the bent mesh")
        else:
            ok("the corrective moves %d vertices of the bent mesh" % moved)
        e[axis] = -sign * math.radians(reader["end"])
        pb.rotation_euler = e
        other = evaluate()
        if other > 1e-3:
            fail("bending the other way turns %s on (%.3f)" % (shape, other))
        else:
            ok("bending the other way leaves %s off" % shape)
        pb.rotation_euler = (0, 0, 0)

if fbx_path:
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.fbx(filepath=fbx_path)
    imported = [o for o in bpy.data.objects if o.type == "MESH"]
    ok("FBX: %d mesh object(s)" % len(imported))
    hive = json.loads(open(script).read().split('r"""', 1)[1].split('"""', 1)[0])
    if source_path:
        if len(imported) != len(src):
            fail("FBX: %d mesh objects, the source has %d" % (len(imported), len(src)))
        for s in src:
            obj = by_name(imported, s["name"]) if len(src) > 1 else imported[0]
            if obj is None:
                fail("FBX: no object for '%s'" % s["name"])
                continue
            compare_uvs("FBX " + s["name"], s, [obj])
            compare_materials("FBX " + s["name"], s, obj, doc)
            compare_shapes("FBX " + s["name"], s, [obj])
    for s in hive["mesh"].get("shapes", []):
        if not any(o.data.shape_keys and s["name"] in o.data.shape_keys.key_blocks for o in imported):
            fail("FBX: shape '%s' is missing" % s["name"])
    if hive["mesh"].get("shapes"):
        ok("FBX: all %d shapes present as shape keys" % len(hive["mesh"]["shapes"]))

if failures:
    print("MESH CHECK FAILED (%d)" % len(failures))
    sys.exit(1)
print("mesh check passed")
