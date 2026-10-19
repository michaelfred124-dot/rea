"""Measure how well a rig's skin weights deform, in real Blender.

    python blender_skin_eval.py <name>.hive.json [--source model.glb] [--json out.json]
        [--max-leg-leak X] [--max-arm-leak X] [--min-thickness X] [--max-spikes X]
        [--max-piece-wrong X] [--max-artist-diff X]

The rig's mesh, bones and weights are rebuilt in Blender as a plain armature
with an Armature modifier (the control rig isn't needed to judge the weights).
With `--source`, the glTF the rig was made from (`hive-rig rig model.glb`) is
read for the artist's weights, which are mapped onto the rig's bones and judged
the same way, as a reference row.

Measured, for the rig's weights and (with --source) the artist's:

* leg_leak / arm_leak: mean weight a vertex takes from bones of a limb it isn't
  on (the other leg; arm bones on the torso and torso bones on the arm), and
  the share of those vertices with more than 0.05 of it. A vertex is "on" the
  part of its artist's strongest bone, or without artist weights the part of
  the nearest bone (when clearly nearest).
* pieces: on loose mesh pieces (eyes, teeth, buttons...), the share of vertices
  whose strongest bone is neither the artist's nor one across a joint from it
  (or, without artist weights, on pieces under 2% of the mesh, isn't the
  piece's majority bone, so the piece would tear).
* thickness: elbows and knees are bent 90 degrees in Blender; for the skin near
  each joint, the mean distance to the bent bones over the mean distance at
  rest. Below 1 the joint collapses (the candy-wrapper / pinched knee).
* spikes: per vertex, how far its weights are from its neighbours' average
  (sum over bones, mean and 99th percentile), and the share of vertices whose
  strongest bone differs from every neighbour's (isolated islands).
* artist_diff: half the summed absolute difference from the artist's weights
  per vertex (0 = identical, 1 = disjoint), and how often the strongest bone
  agrees. Twist and bend joints count as the bone they belong to.

Downloaded models are untrusted data: run this with `python -I`.
"""

import argparse
import json
import math
import struct
import sys

import bpy
import numpy as np
from mathutils import Matrix, Vector

ap = argparse.ArgumentParser()
ap.add_argument("rig")
ap.add_argument("--source")
ap.add_argument("--json")
for flag in ("max-leg-leak", "max-arm-leak", "min-thickness", "max-spikes", "max-piece-wrong", "max-artist-diff"):
    ap.add_argument("--" + flag, type=float)
argv = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else sys.argv[1:]
args = ap.parse_args(argv)

rig = json.load(open(args.rig))
P = np.array(rig["mesh"]["positions"], dtype=np.float64)
TRI = np.array(rig["mesh"]["indices"], dtype=np.int64).reshape(-1, 3)
BONES = rig["skeleton"]["bones"]
NV, NB = len(P), len(BONES)
HEAD = np.array([b["head"] for b in BONES], dtype=np.float64)
TAIL = np.array([b["tail"] for b in BONES], dtype=np.float64)
SIZE = float((P.max(0) - P.min(0)).max())


def dense(skin):
    w = np.zeros((NV, NB))
    for v, (js, ws) in enumerate(zip(skin["joints"], skin["weights"])):
        for j, x in zip(js, ws):
            w[v, j] += x
    return w


OURS = dense(rig["skin"])

# ------------------------------------------------------------------ bone parts


def canonical(b):
    """Twist helpers and bend joints count as the bone they belong to."""
    bone = BONES[b]
    p = bone.get("parent")
    if p is None:
        return b
    parent = BONES[p]
    if bone.get("twist") or (not parent["deform"] and parent.get("component") == bone.get("component")):
        return canonical(p)
    return b


CANON = [canonical(b) for b in range(NB)]


def part(b):
    name, region = BONES[b]["name"].lower(), BONES[b].get("region", "center")
    if region.startswith("leg"):
        return region
    if region.startswith("arm"):
        return None if "clavicle" in name else region
    if any(k in name for k in ("spine", "pelvis", "chest", "hips")):
        return "torso"
    return None


PART = [part(b) for b in range(NB)]


def fold(w):
    out = np.zeros_like(w)
    for b in range(NB):
        out[:, CANON[b]] += w[:, b]
    return out


def seg_dist(p, a, b):
    ab = b - a
    t = np.clip(((p - a) @ ab) / max(ab @ ab, 1e-30), 0.0, 1.0)
    return np.linalg.norm(p - (a + np.outer(t, ab)), axis=1)


DEFORM = [b for b in range(NB) if BONES[b]["deform"]]
EUCLID = np.stack([seg_dist(P, HEAD[b], TAIL[b]) for b in range(NB)], axis=1)

# ------------------------------------------------------------------ welded mesh

_, WELD = np.unique(P.round(9), axis=0, return_inverse=True)
WELD = WELD.reshape(-1)
NW = WELD.max() + 1
edges = np.concatenate([TRI[:, [0, 1]], TRI[:, [1, 2]], TRI[:, [2, 0]]])
edges = WELD[edges]
edges = edges[edges[:, 0] != edges[:, 1]]
edges = np.unique(np.sort(edges, axis=1), axis=0)


def pieces():
    parent = np.arange(NW)

    def find(x):
        while parent[x] != x:
            parent[x] = parent[parent[x]]
            x = parent[x]
        return x

    for a, b in edges:
        ra, rb = find(a), find(b)
        if ra != rb:
            parent[ra] = rb
    roots = np.array([find(x) for x in range(NW)])
    return roots[WELD]


PIECE = pieces()

# ------------------------------------------------------------------ the artist's weights


def read_glb(path):
    data = open(path, "rb").read()
    magic, _, _ = struct.unpack_from("<III", data, 0)
    assert magic == 0x46546C67, "not a .glb"
    off, gltf, binary = 12, None, b""
    while off < len(data):
        n, kind = struct.unpack_from("<II", data, off)
        chunk = data[off + 8 : off + 8 + n]
        if kind == 0x4E4F534A:
            gltf = json.loads(chunk)
        elif kind == 0x004E4942:
            binary = chunk
        off += 8 + n
    return gltf, binary


COMP = {5120: "b", 5121: "B", 5122: "h", 5123: "H", 5125: "I", 5126: "f"}
NCOMP = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4, "MAT4": 16}


def accessor(g, binary, i):
    a = g["accessors"][i]
    view = g["bufferViews"][a["bufferView"]]
    n, c = a["count"], NCOMP[a["type"]]
    fmt = COMP[a["componentType"]]
    size = struct.calcsize(fmt)
    stride = view.get("byteStride", size * c)
    base = view.get("byteOffset", 0) + a.get("byteOffset", 0)
    out = np.empty((n, c))
    for k in range(n):
        out[k] = struct.unpack_from("<" + fmt * c, binary, base + k * stride)
    if a.get("normalized"):
        out /= float({"b": 127, "B": 255, "h": 32767, "H": 65535}[fmt])
    return out


def node_matrix(n):
    if "matrix" in n:
        return Matrix([n["matrix"][i::4] for i in range(4)])
    t = Matrix.Translation(n.get("translation", [0, 0, 0]))
    q = n.get("rotation", [0, 0, 0, 1])
    r = Matrix.Identity(4)
    from mathutils import Quaternion

    r = Quaternion((q[3], q[0], q[1], q[2])).to_matrix().to_4x4()
    s = Matrix.Diagonal(n.get("scale", [1, 1, 1]) + [1])
    return t @ r @ s


def artist_weights(path):
    """The source file's weights in the order hive-rig loads its vertices, with
    each joint as a segment to its children, in the same space."""
    g, binary = read_glb(path)
    scene = g["scenes"][g.get("scene", 0)]
    world = {}

    def walk(i, parent):
        m = parent @ node_matrix(g["nodes"][i])
        world[i] = m
        for c in g["nodes"][i].get("children", []):
            walk(c, m)

    for i in scene["nodes"]:
        walk(i, Matrix.Identity(4))
    rows, joint_pos, joint_children = [], {}, {}
    positions = []

    def collect(i):
        n = g["nodes"][i]
        if "mesh" in n:
            skin = g["skins"][n["skin"]] if "skin" in n else None
            if skin is not None:
                ibm = accessor(g, binary, skin["inverseBindMatrices"]) if "inverseBindMatrices" in skin else None
                for k, jn in enumerate(skin["joints"]):
                    if ibm is not None:
                        m = Matrix([ibm[k][c::4] for c in range(4)]).inverted()
                        joint_pos[jn] = np.array(world[i] @ m.translation)
                    else:
                        joint_pos[jn] = np.array(world[jn].translation)
            for prim in g["meshes"][n["mesh"]]["primitives"]:
                if prim.get("mode", 4) != 4:
                    continue
                attrs = prim["attributes"]
                pos = accessor(g, binary, attrs["POSITION"])
                positions.extend(np.array(world[i] @ Vector(p)) for p in pos)
                if skin is None or "JOINTS_0" not in attrs:
                    rows.extend([{}] * len(pos))
                    continue
                sets = []
                k = 0
                while f"JOINTS_{k}" in attrs:
                    sets.append((accessor(g, binary, attrs[f"JOINTS_{k}"]), accessor(g, binary, attrs[f"WEIGHTS_{k}"])))
                    k += 1
                for v in range(len(pos)):
                    row = {}
                    for js, ws in sets:
                        for j, w in zip(js[v], ws[v]):
                            if w > 0:
                                jn = skin["joints"][int(j)]
                                row[jn] = row.get(jn, 0.0) + w
                    rows.append(row)
        for c in n.get("children", []):
            collect(c)

    for i in scene["nodes"]:
        collect(i)
    for jn in joint_pos:
        joint_children[jn] = [c for c in g["nodes"][jn].get("children", []) if c in joint_pos]
    names = {jn: g["nodes"][jn].get("name", f"joint_{jn}") for jn in joint_pos}
    return np.array(positions), rows, joint_pos, joint_children, names


def map_artist(path):
    pos, rows, joint_pos, kids, names = artist_weights(path)
    if len(pos) != NV or np.abs(pos - P).max() > 1e-3 * SIZE:
        print(f"  source vertices don't line up with the rig's ({len(pos)} vs {NV}); no artist reference")
        return None
    targets = sorted(set(CANON[b] for b in DEFORM))
    to_bone = {}
    for jn, head in joint_pos.items():
        tail = np.mean([joint_pos[c] for c in kids[jn]], axis=0) if kids[jn] else head
        pts = np.stack([head, (head + tail) / 2, tail])
        cost = [seg_dist(pts, HEAD[b], TAIL[b]).mean() for b in targets]
        to_bone[jn] = targets[int(np.argmin(cost))]
    w = np.zeros((NV, NB))
    for v, row in enumerate(rows):
        for jn, x in row.items():
            w[v, to_bone[jn]] += x
    s = w.sum(1, keepdims=True)
    unweighted = (s[:, 0] <= 0).sum()
    w = np.where(s > 0, w / np.maximum(s, 1e-12), 0)
    print(f"  artist joints mapped to rig bones: " + ", ".join(
        f"{names[j]}->{BONES[b]['name']}" for j, b in list(to_bone.items())[:12]) + (" ..." if len(to_bone) > 12 else ""))
    return w, unweighted


# ------------------------------------------------------------------ truth parts per vertex


def truth_parts(artist):
    if artist is not None:
        dom = artist.argmax(1)
        ok = artist.sum(1) > 0
        return [PART[b] if ok[v] else None for v, b in enumerate(dom)]
    # Without artist weights: the part of the clearly nearest bone.
    out = []
    order = np.argsort(np.where(np.isin(np.arange(NB), DEFORM), EUCLID, np.inf), axis=1)
    for v in range(NV):
        b0 = order[v, 0]
        p0 = PART[b0]
        rival = next((b for b in order[v, 1:] if PART[b] != p0), None)
        clear = rival is None or EUCLID[v, b0] < 0.7 * EUCLID[v, rival]
        out.append(p0 if clear else None)
    return out


def leaks(w, truth):
    groups = {
        "leg_leak": [("leg_l", ["leg_r"]), ("leg_r", ["leg_l"])],
        "arm_leak": [("torso", ["arm_l", "arm_r"]), ("arm_l", ["torso"]), ("arm_r", ["torso"])],
    }
    bones_of = {p: [b for b in range(NB) if PART[b] == p] for p in ("leg_l", "leg_r", "arm_l", "arm_r", "torso")}
    out = {}
    for key, pairs in groups.items():
        vals = []
        for on, others in pairs:
            vs = [v for v in range(NV) if truth[v] == on]
            cols = [b for o in others for b in bones_of[o]]
            if vs and cols:
                vals.extend(w[vs][:, cols].sum(1))
        vals = np.array(vals)
        out[key] = (float(vals.mean()), float((vals > 0.05).mean())) if len(vals) else None
    return out


def piece_errors(w, artist):
    sizes = np.bincount(PIECE)
    main = sizes.argmax()
    loose = np.where(PIECE != main)[0]
    if len(loose) == 0:
        return None
    dom = fold(w).argmax(1)

    def parent(b):
        p = BONES[b].get("parent")
        return None if p is None else CANON[p]

    def wrong(d, r):
        # Not the expected bone, nor the bone across a joint from it (where to
        # cut between two bones is a matter of taste).
        return d != r and parent(d) != r and parent(r) != d

    if artist is not None:
        ref = fold(artist).argmax(1)
        ok = artist.sum(1) > 0
        sel = loose[ok[loose]]
        bad = [wrong(d, r) for d, r in zip(dom[sel], ref[sel])]
        return float(np.mean(bad)) if len(sel) else None, len(set(PIECE[loose]))
    # Without artist weights only small pieces (eyes, buttons, under 2% of the
    # mesh) are judged: each should follow one bone.
    small = [p for p in set(PIECE[loose]) if sizes[p] < 0.02 * NV]
    if not small:
        return None
    bad = total = 0
    for p in small:
        vs = np.where(PIECE == p)[0]
        major = np.bincount(dom[vs]).argmax()
        bad += sum(wrong(d, major) for d in dom[vs])
        total += len(vs)
    return float(bad / total), len(small)


def spikes(w):
    wf = fold(w)
    ww = np.zeros((NW, NB))
    ww[WELD] = wf
    nsum = np.zeros((NW, NB))
    deg = np.zeros(NW)
    np.add.at(nsum, edges[:, 0], ww[edges[:, 1]])
    np.add.at(nsum, edges[:, 1], ww[edges[:, 0]])
    np.add.at(deg, edges[:, 0], 1)
    np.add.at(deg, edges[:, 1], 1)
    has = deg > 0
    lap = np.abs(ww[has] - nsum[has] / deg[has, None]).sum(1)
    dom = ww.argmax(1)
    same = np.zeros(NW, dtype=bool)
    same[edges[:, 0][dom[edges[:, 0]] == dom[edges[:, 1]]]] = True
    same[edges[:, 1][dom[edges[:, 0]] == dom[edges[:, 1]]]] = True
    islands = (~same & has).sum() / max(has.sum(), 1)
    return float(lap.mean()), float(np.percentile(lap, 99)), float(islands)


def artist_diff(w, artist):
    a, b = fold(w), fold(artist)
    ok = artist.sum(1) > 0
    d = 0.5 * np.abs(a - b).sum(1)
    return float(d[ok].mean()), float((a.argmax(1) == b.argmax(1))[ok].mean())


# ------------------------------------------------------------------ bending in Blender


def blender_rig(w, tag):
    for o in list(bpy.data.objects):
        bpy.data.objects.remove(o)
    me = bpy.data.meshes.new(tag)
    me.from_pydata(P.tolist(), [], TRI.tolist())
    obj = bpy.data.objects.new(tag, me)
    arm = bpy.data.armatures.new(tag + "_arm")
    arm_obj = bpy.data.objects.new(tag + "_arm", arm)
    bpy.context.scene.collection.objects.link(obj)
    bpy.context.scene.collection.objects.link(arm_obj)
    bpy.context.view_layer.objects.active = arm_obj
    bpy.ops.object.mode_set(mode="EDIT")
    for b in BONES:
        eb = arm.edit_bones.new(b["name"])
        eb.head, eb.tail = b["head"], b["tail"]
        if (eb.tail - eb.head).length < 1e-9:
            eb.tail = eb.head + Vector((0, 1e-4 * SIZE, 0))
    for b in BONES:
        if b.get("parent") is not None:
            arm.edit_bones[b["name"]].parent = arm.edit_bones[BONES[b["parent"]]["name"]]
    bpy.ops.object.mode_set(mode="OBJECT")
    for bi, b in enumerate(BONES):
        vg = obj.vertex_groups.new(name=b["name"])
        for v in np.nonzero(w[:, bi] > 0)[0]:
            vg.add([int(v)], float(w[v, bi]), "REPLACE")
    mod = obj.modifiers.new("arm", "ARMATURE")
    mod.object = arm_obj
    return obj, arm_obj


def find(name):
    return next((i for i, b in enumerate(BONES) if b["name"] == name), None)


def thickness(w, tag):
    """Bend each elbow forward and each knee back 90 degrees together; for each
    joint, the posed over rest mean distance of nearby skin to its two bones."""
    joints = []
    for side in ("l", "r"):
        for upper, lower, forward in (("upperarm", "lowerarm", 1.0), ("thigh", "calf", -1.0)):
            u, lo = find(f"{upper}_{side}"), find(f"{lower}_{side}")
            if u is not None and lo is not None:
                joints.append((u, lo, forward))
    if not joints:
        return None
    obj, arm_obj = blender_rig(w, tag)
    pose = arm_obj.pose.bones
    turned = {}
    for u, lo, forward in joints:
        d = TAIL[lo] - HEAD[lo]
        d /= np.linalg.norm(d)
        axis = np.cross(d, [0, 0, forward])
        if np.linalg.norm(axis) < 1e-6:
            axis = np.array([1.0, 0, 0])
        axis /= np.linalg.norm(axis)
        rot = Matrix.Rotation(math.radians(90), 4, Vector(axis))
        piv = Vector(HEAD[lo])
        pb = pose[BONES[lo]["name"]]
        pb.matrix = Matrix.Translation(piv) @ rot @ Matrix.Translation(-piv) @ pb.matrix
        bpy.context.view_layer.update()
        turned[lo] = (np.array(rot.to_3x3()), HEAD[lo])
    dg = bpy.context.evaluated_depsgraph_get()
    ev = obj.evaluated_get(dg)
    posed = np.array([v.co for v in ev.to_mesh().vertices])
    ev.to_mesh_clear()
    ratios = []
    nearest = np.where(np.isin(np.arange(NB), DEFORM), EUCLID, np.inf).argmin(1)
    for u, lo, _ in joints:
        r = 0.6 * np.linalg.norm(TAIL[lo] - HEAD[lo])
        chain = {u, lo} | {b for b in range(NB) if CANON[b] in (u, lo)}
        sel = np.where((np.linalg.norm(P - HEAD[lo], axis=1) < r) & np.isin(nearest, list(chain)))[0]
        if len(sel) == 0:
            continue
        rot, piv = turned[lo]
        ptail = piv + rot @ (TAIL[lo] - piv)
        rest = np.minimum(seg_dist(P[sel], HEAD[u], HEAD[lo]), seg_dist(P[sel], HEAD[lo], TAIL[lo]))
        bent = np.minimum(seg_dist(posed[sel], HEAD[u], HEAD[lo]), seg_dist(posed[sel], piv, ptail))
        ratios.append(float(bent.mean() / max(rest.mean(), 1e-12)))
    return (min(ratios), float(np.mean(ratios))) if ratios else None


def evaluate(w, artist, truth, tag):
    out = {}
    out.update(leaks(w, truth))
    out["pieces"] = piece_errors(w, artist)
    out["thickness"] = thickness(w, tag)
    out["spikes"] = spikes(w)
    out["artist_diff"] = artist_diff(w, artist) if artist is not None else None
    return out


def fmt(x):
    if x is None:
        return "n/a"
    if isinstance(x, tuple | list):
        return " / ".join(f"{v:.3f}" if isinstance(v, float) else str(v) for v in x)
    return f"{x:.3f}"


artist = None
if args.source:
    mapped = map_artist(args.source)
    if mapped is not None:
        artist, unweighted = mapped
        if unweighted:
            print(f"  {unweighted} source vertices have no artist weights; they are left out")
truth = truth_parts(artist)
rows = {"rig": evaluate(OURS, artist, truth, "rig")}
if artist is not None:
    rows["artist"] = evaluate(artist, artist, truth, "artist")

print(f"{rig['name']}: {NV} vertices, {NB} bones, {len(set(PIECE))} mesh pieces")
labels = {
    "leg_leak": "leg leak (mean / share>0.05)",
    "arm_leak": "arm-torso leak (mean / share>0.05)",
    "pieces": "loose pieces wrong bone (share / pieces)",
    "thickness": "bent joint thickness (min / mean)",
    "spikes": "spikes (mean / p99 / islands)",
    "artist_diff": "vs artist (diff / same top bone)",
}
for k, label in labels.items():
    print(f"  {label:42s}" + "".join(f"  {name}: {fmt(r[k]):28s}" for name, r in rows.items()))
if args.json:
    json.dump(rows, open(args.json, "w"), indent=1)

r = rows["rig"]
fail = []
checks = [
    ("max_leg_leak", lambda: r["leg_leak"] and r["leg_leak"][0], max),
    ("max_arm_leak", lambda: r["arm_leak"] and r["arm_leak"][0], max),
    ("min_thickness", lambda: r["thickness"] and r["thickness"][0], min),
    ("max_spikes", lambda: r["spikes"][1], max),
    ("max_piece_wrong", lambda: r["pieces"] and r["pieces"][0], max),
    ("max_artist_diff", lambda: r["artist_diff"] and r["artist_diff"][0], max),
]
for key, get, kind in checks:
    limit = getattr(args, key)
    value = get()
    if limit is None or value is None:
        continue
    if (kind is max and value > limit) or (kind is min and value < limit):
        fail.append(f"{key.replace('_', ' ')}: {value:.3f} (limit {limit})")
if fail:
    sys.exit("skin weights fail: " + "; ".join(fail))
print("skin weights OK")
