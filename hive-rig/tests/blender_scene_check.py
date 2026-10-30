"""Build a generated hive-rig scene in real Blender and check it: every
character built in one file at its place, and each held prop following the
other character's hand by its `held` setting.

Run with the `bpy` module (pip install bpy) or Blender itself:
    python blender_scene_check.py <out>/<scene>_scene_blender.py
after `hive-rig scene build <scene>.scene.json -o <out>`.
"""

import json
import runpy
import sys

import bpy
from mathutils import Matrix, Vector

args = sys.argv[sys.argv.index("--") + 1:] if "--" in sys.argv else sys.argv[1:]
script = args[0]
SCENE = json.loads(open(script).read().split('r"""', 1)[1].split('"""', 1)[0])


def check(cond, msg):
    if not cond:
        raise SystemExit("FAIL: " + msg)
    print("ok:", msg)


def to_blender(p):
    return Vector((p[0], -p[2], p[1]))


def update():
    # Drivers read custom properties only when the rigs are tagged and the
    # frame is evaluated again, as a user's own edit in Blender would do.
    for o in bpy.data.objects:
        o.update_tag()
    bpy.context.scene.frame_set(bpy.context.scene.frame_current)
    bpy.context.view_layer.update()


bpy.ops.wm.read_factory_settings(use_empty=True)
saved = sys.argv
sys.argv = ["blender"]
runpy.run_path(script, run_name="__main__")
sys.argv = saved
update()

rigs = {}
for c in SCENE["characters"]:
    rig = bpy.data.objects.get(c["name"] + "_rig")
    check(rig is not None and rig.type == "ARMATURE", "%s is built" % c["name"])
    rigs[c["name"]] = rig
    check((rig.matrix_world.translation - to_blender(c["offset"])).length < 1e-5,
          "%s stands at %s" % (c["name"], c["offset"]))
    # Its skinned meshes come with it: their middle is near the rig's place.
    meshes = [o for o in bpy.data.objects if o.type == "MESH" and o.parent == rig]
    check(meshes, "%s has its meshes" % c["name"])
    xs = [(o.matrix_world @ v.co).x for o in meshes for v in o.data.vertices]
    middle = (min(xs) + max(xs)) / 2.0
    check(abs(middle - to_blender(c["offset"]).x) < 0.2,
          "%s's mesh stands with it (middle x %.3f)" % (c["name"], middle))
check(len({r.name for r in rigs.values()}) == len(SCENE["characters"]), "every character has its own rig")


def world(rig, bone):
    return rig.matrix_world @ rig.pose.bones[bone].matrix


def turn(rig, bone, degrees):
    pb = rig.pose.bones[bone]
    pb.rotation_mode = "XYZ"
    pb.rotation_euler = (0.0, 0.0, degrees * 3.14159265 / 180.0)
    update()


def reset(rig):
    for pb in rig.pose.bones:
        pb.location = (0, 0, 0)
        pb.rotation_euler = (0, 0, 0)
        pb.rotation_quaternion = (1, 0, 0, 0)
    update()


for h in SCENE["holds"]:
    prop_rig, holder = rigs[h["prop_character"]], rigs[h["holder_character"]]
    ctrl, hand = h["prop_control"], h["holder_bone"]
    pb = prop_rig.pose.bones[ctrl]
    con = pb.constraints.get("Held by " + h["holder_character"])
    check(con is not None and con.type == "CHILD_OF" and con.subtarget == hand,
          "%s's %s has a Child Of on %s's %s" % (h["prop_character"], ctrl, h["holder_character"], hand))
    check(abs(pb["held"] - h["held"]) < 1e-6, "held starts at %s" % h["held"])
    # Rest: the hold doesn't move the prop.
    pb["held"] = 1.0
    update()
    rest = world(prop_rig, ctrl).copy()
    pb["held"] = 0.0
    update()
    check((world(prop_rig, ctrl).translation - rest.translation).length < 1e-4,
          "holding it doesn't move it at rest")
    # Find a bone up the holder's arm that moves the hand: the hand's parent chain.
    chain = []
    b = holder.pose.bones[hand]
    while b.parent is not None:
        chain.append(b.parent.name)
        b = b.parent
    mover = None
    for name in [hand] + chain:
        # The holder's control for that bone, or the bone itself when nothing drives it.
        for cand in (name + "_fk_ctrl", name + "_ctrl"):
            if cand in holder.pose.bones:
                mover = cand
                break
        if mover:
            break
    hand_before = world(holder, hand).translation.copy()
    # Move the holder's whole rig: the hand moves for sure, IK or FK.
    holder.location = holder.location + Vector((0.3, 0.2, 0.1))
    update()
    moved = world(holder, hand).translation - hand_before
    check(moved.length > 0.1, "the holder's hand moves (%.3f)" % moved.length)
    pb["held"] = 1.0
    update()
    got = world(prop_rig, ctrl).translation - rest.translation
    check((got - moved).length < 1e-4, "held 1: the prop follows the holder's hand (off by %.5f)" % (got - moved).length)
    pb["held"] = 0.0
    update()
    still = world(prop_rig, ctrl).translation - rest.translation
    check(still.length < 1e-4, "held 0: it stays on its own socket (%.5f)" % still.length)
    pb["held"] = 0.5
    update()
    half = world(prop_rig, ctrl).translation - rest.translation
    check((half - moved * 0.5).length < 2e-3, "held 0.5: half way (off by %.5f)" % (half - moved * 0.5).length)
    # Turning the holder's hand (or its arm) carries the prop round with it.
    holder.location = holder.location - Vector((0.3, 0.2, 0.1))
    pb["held"] = 1.0
    update()
    if mover:
        turn(holder, mover, 35.0)
        rel_rest = world(holder, hand).inverted() @ world(prop_rig, ctrl)
        turn(holder, mover, -20.0)
        rel_now = world(holder, hand).inverted() @ world(prop_rig, ctrl)
        diff = max(abs(a - b) for ra, rb in zip(rel_rest, rel_now) for a, b in zip(ra, rb))
        check(diff < 1e-4, "held 1: turning %s keeps the prop fixed in the hand (%.6f)" % (mover, diff))
        reset(holder)
    # The prop's own character still carries it when not held.
    pb["held"] = 0.0
    update()
    socket_parent = prop_rig.data.bones[ctrl].parent
    if socket_parent is not None:
        before = (world(prop_rig, socket_parent.name).inverted() @ world(prop_rig, ctrl))
        prop_rig.location = prop_rig.location + Vector((0.0, 0.4, 0.0))
        update()
        after = (world(prop_rig, socket_parent.name).inverted() @ world(prop_rig, ctrl))
        diff = max(abs(a - b) for ra, rb in zip(before, after) for a, b in zip(ra, rb))
        check(diff < 1e-4, "held 0: it moves with its own character (%.6f)" % diff)
        prop_rig.location = prop_rig.location - Vector((0.0, 0.4, 0.0))
    pb["held"] = h["held"]
    update()
    _ = Matrix  # keep the import for readers adding matrix checks

print("scene %s checked: %d characters, %d held props" % (SCENE["name"], len(rigs), len(SCENE["holds"])))
