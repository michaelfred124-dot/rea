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


def shape_mismatch(c):
    """How far the control's custom shape, as Blender draws it at rest, strays from
    the outline hive-rig wrote for it (metres), or None if the edge count is off."""
    bone = rig.data.bones[c["name"]]
    wgt = rig.pose.bones[c["name"]].custom_shape
    lines = c["outline"]
    want_edges = sum(len(line) - 1 for line in lines)
    if wgt is None or len(wgt.data.edges) != want_edges:
        return None
    drawn = [rig.matrix_world @ bone.matrix_local @ v.co for v in wgt.data.vertices]
    wanted = [Vector((p[0], -p[2], p[1])) for line in lines for p in line]
    return max(min((d - w).length for d in drawn) for w in wanted)


DATA = json.loads(open(script).read().split('r"""', 1)[1].split('"""', 1)[0])
RIG_CONTROLS = DATA["controls"]["controls"]
worst = [shape_mismatch(c) for c in RIG_CONTROLS]
check(all(w is not None and w < 1e-3 for w in worst),
      "every control shape sits on its outline (%d controls, %d shapes)"
      % (len(RIG_CONTROLS), len({c["shape"] for c in RIG_CONTROLS})))
# Custom shapes (drawn or imported in hive-rig) are drawn from the same
# outline, and a control's line width is its wire width (Blender 4.2+).
CUSTOM = [c for c in RIG_CONTROLS if c.get("custom")]
if CUSTOM:
    check(all(shape_mismatch(c) is not None and shape_mismatch(c) < 1e-3 for c in CUSTOM),
          "custom control shapes drawn on their outline (%s)"
          % ", ".join(sorted({c["custom"]["name"] for c in CUSTOM})))
WIDE = [c for c in RIG_CONTROLS if c.get("line_width") is not None]
if WIDE and hasattr(rig.pose.bones[WIDE[0]["name"]], "custom_shape_wire_width"):
    check(all(abs(rig.pose.bones[c["name"]].custom_shape_wire_width - max(c["line_width"], 1.0)) < 1e-4
              for c in WIDE),
          "line width set as the wire width on %d controls" % len(WIDE))

if hasattr(rig.pose.bones[RIG_CONTROLS[0]["name"]], "color"):
    off = max(max(abs(a - b) for a, b in zip(rig.pose.bones[c["name"]].color.custom.normal, c["color"]))
              for c in RIG_CONTROLS)
    # Blender keeps bone colours at 8 bits a channel.
    check(off < 1.0 / 255 + 1e-4, "every control is drawn in its colour (%d colours)" % len({tuple(c["color"]) for c in RIG_CONTROLS}))

rest = [v.co.copy() for v in geo.data.vertices]

_bones = [b["name"] for b in DATA["skeleton"]["bones"]]
_groups = {vg.index: vg.name for vg in geo.vertex_groups}
_off = 0
for v, vert in enumerate(geo.data.vertices):
    got = {_groups[g.group]: g.weight for g in vert.groups if g.weight > 0.0}
    want = {}
    for j, w in zip(DATA["skin"]["joints"][v], DATA["skin"]["weights"][v]):
        if w > 0.0:
            want[_bones[j]] = want.get(_bones[j], 0.0) + w
    if any(abs(got.get(n, 0.0) - want.get(n, 0.0)) > 1e-4 for n in set(got) | set(want)):
        _off += 1
check(_off == 0, "the builder script gives every vertex its weights (%d vertices)" % len(rest))
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
# A bendy forearm grades the twist over several joints; the far one takes the usual half.
_twists = [n for n in ("lowerarm_twist_%02d_l" % i for i in range(1, 13)) if n in pb]
_angles = [(pb["lowerarm_l"].matrix.inverted() @ pb[n].matrix).to_euler().y for n in _twists]
twist_angle = _angles[-1]
check(abs(twist_angle - 0.5) < 0.05, "forearm twist bone takes half the hand twist (%.2f rad)" % twist_angle)
check(all(a < b + 1e-4 for a, b in zip(_angles, _angles[1:])), "forearm twist grows towards the hand")

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

BOARD = DATA["controls"].get("face_board")
if BOARD:
    # The face board: each slider moves and turns the face controls it drives by
    # hive-rig's offsets (worked out here from the rig file the way hive-rig's
    # pose evaluator does), sliders stay in their tracks, and the lip seal closes
    # the lips. Names in the board data are already the exported ones.
    import math
    from mathutils import Euler, Matrix

    bones = rig.pose.bones
    sliders = {s["part"]: s for s in BOARD["sliders"]}

    def slide(part, x, y):
        s = sliders[part]
        bones[s["control"]].location = (x * s["travel"], y * s["travel"], 0.0)
        update()

    def values():
        out = {}
        for part, s in sliders.items():
            loc = bones[s["control"]].location
            out[part] = (min(max(loc[0] / s["travel"], s["x"][0]), s["x"][1]),
                         min(max(loc[1] / s["travel"], s["y"][0]), s["y"][1]))
        return out

    def amount(v, d):
        axis, sign = (0 if d[0] == "x" else 1), (1.0 if d[1] == "+" else -1.0)
        return max(v[axis] * sign, 0.0)

    def offset(control):
        """hive-rig's board offset of a control: (move, turn matrix, pivot), hive space."""
        o = next(o for o in BOARD["offsets"] if o["control"] == control)
        vals = values()
        t, r = Vector((0, 0, 0)), Vector((0, 0, 0))
        for d in o["drives"]:
            a = amount(vals.get(d["part"], (0.0, 0.0)), d["dir"])
            t += Vector(d.get("translate", (0, 0, 0))) * a
            r += Vector(d.get("rotate", (0, 0, 0))) * a
        turn = Euler([math.radians(x) for x in r], "XYZ").to_matrix()
        return t, turn, Vector(o["pivot"])

    def expected(control, point):
        """Where the board puts a rest point (hive space) carried by `control`."""
        t, turn, pivot = offset(control)
        return pivot + t + turn @ (point - pivot)

    def hive_pos(v):
        return Vector((v[0], v[2], -v[1]))

    def all_rest():
        reset()
        for s in BOARD["sliders"]:
            bones[s["control"]].location = (0, 0, 0)
        update()

    jaw_ctrl = final("jaw_ctrl")
    all_rest()
    rest_pts = {b.name: (hive_pos(b.head.copy()), hive_pos(b.tail.copy())) for b in bones}
    if "jaw" in sliders and any(o["control"] == jaw_ctrl for o in BOARD["offsets"]):
        slide("jaw", 0.0, -1.0)
        want = expected(jaw_ctrl, rest_pts[final("jaw")][1])
        got = hive_pos(bones[final("jaw")].tail)
        check((got - want).length < 1e-4 and want.y < rest_pts[final("jaw")][1].y - 0.01,
              "the board's jaw slider opens the jaw as hive-rig does (%.6f m off)" % (got - want).length)
        # Past the end of the track is the end of the track.
        slide("jaw", 0.0, -4.0)
        check((hive_pos(bones[final("jaw")].tail) - want).length < 1e-4, "a slider stops at the end of its track")
        all_rest()
    slide("lid_l", 0.0, -1.0)
    lid = final("lid_up_l")
    want = expected(final("lid_up_l_ctrl"), rest_pts[lid][1])
    got = hive_pos(bones[lid].tail)
    check((got - want).length < 1e-4 and want.y < rest_pts[lid][1].y - 0.002,
          "the board's lid slider closes the upper lid as hive-rig does (%.6f m off)" % (got - want).length)
    all_rest()
    slide("brow_l", -1.0, 1.0)
    slide("corner_r", 0.6, 0.8)
    for bone in ("brow_in_l", "brow_out_l", "lip_corner_r", "lip_up_1_r"):
        if final(bone) not in bones:
            continue
        want = expected(final(bone + "_ctrl"), rest_pts[final(bone)][0])
        got = hive_pos(bones[final(bone)].head)
        check((got - want).length < 1e-4 and (want - rest_pts[final(bone)][0]).length > 1e-3,
              "the board moves %s as hive-rig does (%.6f m off)" % (bone, (got - want).length))
    # The face control still moves its joint on top of the board.
    before = hive_pos(bones[final("brow_in_l")].head)
    bones[final("brow_in_l_ctrl")].location = (0.0, 0.0, 0.0)
    move("brow_in_l_ctrl", (0, -0.01, 0))
    check((hive_pos(bones[final("brow_in_l")].head) - before - Vector((0, 0, 0.01))).length < 1e-4,
          "a face control moves its joint on top of the board")
    all_rest()
    # The panel carries its sliders and leaves the face alone.
    frame = BOARD["frame"]
    slider0 = BOARD["sliders"][0]["control"]
    s0 = (rig.matrix_world @ bones[slider0].head).copy()
    lip0 = (rig.matrix_world @ bones[final("lip_up")].head).copy()
    m = bones[frame].matrix.copy()
    m.translation += Vector((0.05, 0, 0))
    bones[frame].matrix = m
    update()
    check(((rig.matrix_world @ bones[slider0].head) - s0 - Vector((0.05, 0, 0))).length < 1e-4
          and ((rig.matrix_world @ bones[final("lip_up")].head) - lip0).length < 1e-6,
          "moving the panel carries its sliders and leaves the face alone")
    all_rest()
    # Lip seal: with the jaw open, a zipper all the way closes every lip pair to
    # its rest gap, meeting between where the controls put the lips; half way
    # from the left it closes only the left.
    seal = BOARD.get("seal") or {"pairs": []}

    def gap(pair):
        update()
        return (bones[pair["upper"]].head - bones[pair["lower"]].head).length

    if "jaw" in sliders and seal["pairs"]:
        slide("jaw", 0.0, -1.0)
        opened = {p["upper"]: gap(p) for p in seal["pairs"]}
        check(all(opened[p["upper"]] > p["rest_gap"] + 0.005 for p in seal["pairs"]), "the open jaw parts the lips")
        slide("zip_r", 1.0, 0.0)
        for p in seal["pairs"]:
            mid = (bones[p["upper_control"]].head + bones[p["lower_control"]].head) * 0.5
            got = (bones[p["upper"]].head + bones[p["lower"]].head) * 0.5
            check(abs(gap(p) - p["rest_gap"]) < 1e-4 and (got - mid).length < 1e-4,
                  "the right zipper closes %s and %s between their controls" % (p["upper"], p["lower"]))
        slide("zip_r", 0.0, 0.0)
        slide("zip_l", -0.5, 0.0)
        left = [p for p in seal["pairs"] if p["along"] < 0.3]
        right = [p for p in seal["pairs"] if p["along"] > 0.7]
        check(all(abs(gap(p) - p["rest_gap"]) < 1e-4 for p in left)
              and all(gap(p) > p["rest_gap"] + 0.005 for p in right),
              "the left zipper half way closes the left of the mouth only")
        all_rest()
        slide("jaw", 0.0, -0.08)
        middle = min(seal["pairs"], key=lambda p: abs(p["along"] - 0.5))
        apart = gap(middle)
        slide("sticky", 1.0, 0.0)
        check(apart > middle["rest_gap"] + 1e-3 and abs(gap(middle) - middle["rest_gap"]) < 1e-4,
              "sticky lips hold the middle of the mouth shut as the jaw starts to open")
        slide("jaw", 0.0, -1.0)
        check(gap(middle) > middle["rest_gap"] + 0.005, "sticky lips let go when the jaw opens wide")
        all_rest()
    # Shapes that follow a board slider.
    linked = [s for s in DATA["mesh"].get("shapes", []) if s.get("board") and not s.get("inbetweens")]
    for s in linked:
        key = geo.data.shape_keys.key_blocks[s["name"]]
        link = s["board"]
        sl = sliders[link["part"]]
        axis, sign = (0 if link["dir"][0] == "x" else 1), (1.0 if link["dir"][1] == "+" else -1.0)
        v = [0.0, 0.0]
        v[axis] = 0.5 * sign
        slide(link["part"], v[0], v[1])
        bpy.context.evaluated_depsgraph_get().update()
        got = key.value
        want = min(max(0.5 * float(link.get("weight", 1.0)), 0.0), 1.0)
        check(abs(got - want) < 1e-4, "shape %s follows board slider %s %s (%.3f)" % (s["name"], link["part"], link["dir"], got))
        all_rest()
        check(abs(key.value) < 1e-6, "shape %s is off with the board at rest" % s["name"])
    print("face board checked: %d sliders, %d offsets, %d lip pairs, %d linked shapes"
          % (len(BOARD["sliders"]), len(BOARD["offsets"]), len(seal["pairs"]), len(linked)))

bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=glb)
armatures = [o for o in bpy.data.objects if o.type == "ARMATURE"]
# Recent importers also add an "Icosphere" bone-display shape; only skinned meshes count.
meshes = [o for o in bpy.data.objects if o.type == "MESH" and o.parent in armatures]
check(len(armatures) == 1 and len(meshes) == 1, "glb imports as one armature and one skinned mesh")
check(len(meshes[0].vertex_groups) == len(armatures[0].data.bones), "glb carries skin weights for every bone")


def same_weights(got, want):
    return all(abs(got.get(n, 0.0) - want.get(n, 0.0)) < 1e-3 for n in set(got) | set(want))


# Every vertex's weights, all of its influences (JOINTS_1/WEIGHTS_1 past four),
# arrive as hive-rig wrote them. The importer may reorder vertices, so they are
# matched by position (glTF Y-up is Blender Z-up).
SKIN = DATA["skin"]
BONE_NAMES = [b["name"] for b in DATA["skeleton"]["bones"]]
POSITIONS = DATA["mesh"]["positions"]


def wanted(v):
    out = {}
    for j, w in zip(SKIN["joints"][v], SKIN["weights"][v]):
        if w > 0.0:
            out[BONE_NAMES[j]] = out.get(BONE_NAMES[j], 0.0) + w
    return out


glb_mesh = meshes[0]
names = {vg.index: vg.name for vg in glb_mesh.vertex_groups}
by_place = {}
for v, p in enumerate(POSITIONS):
    by_place.setdefault(tuple(round(c, 4) for c in (p[0], -p[2], p[1])), []).append(v)
mismatched, most = 0, 0
for vert in glb_mesh.data.vertices:
    co = glb_mesh.matrix_world @ vert.co
    got = {names[g.group]: g.weight for g in vert.groups if g.weight > 0.0}
    most = max(most, len(got))
    candidates = by_place.get(tuple(round(c, 4) for c in co), [])
    if not any(same_weights(got, wanted(v)) for v in candidates):
        mismatched += 1
limit = DATA["guides"].get("skin", {}).get("max_influences", 4)
check(mismatched == 0 and len(glb_mesh.data.vertices) == len(POSITIONS),
      "glb weights match the rig on all %d vertices (up to %d influences, limit %d)"
      % (len(POSITIONS), most, limit))
check(most <= limit, "no vertex has more than %d influences" % limit)
print("blender check passed")
