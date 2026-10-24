"""Check the animator tools of a generated Blender rig in real Blender.

IK to FK and FK to IK matching (no pop), pose mirroring, reset, select all and
space switching without a pop.

Run with the `bpy` module (pip install bpy) or Blender itself:
    python blender_anim_check.py <name>_blender.py
    blender --background --python blender_anim_check.py -- <name>_blender.py
"""

import json
import math
import runpy
import sys

import bpy
from mathutils import Matrix, Quaternion, Vector

args = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else sys.argv[1:]
script = args[0]

bpy.ops.wm.read_factory_settings(use_empty=True)
saved_argv = sys.argv
sys.argv = ["blender"]
ns = runpy.run_path(script, run_name="__main__")
sys.argv = saved_argv

rig = next(o for o in bpy.data.objects if o.type == "ARMATURE")
geo = next(o for o in bpy.data.objects if o.type == "MESH" and not o.name.startswith("WGT_"))
data = json.loads(rig["hive_anim"])
pose, bones = rig.pose.bones, rig.data.bones
center_x = json.loads(rig["hive_guides"])["body"]["center_x"]
limbs = {l["name"]: l for l in data["limbs"]}
POS_TOL, ROT_TOL = 2e-3, math.radians(0.5)


def check(cond, msg):
    if not cond:
        raise SystemExit("FAIL: " + msg)
    print("ok:", msg)


def update():
    rig.update_tag()
    bpy.context.view_layer.update()


def world(name):
    update()
    return (rig.matrix_world @ pose[name].matrix).normalized()


def differs(a, b):
    """Largest position (m) and rotation (rad) difference between two world matrices."""
    rot = a.to_quaternion().rotation_difference(b.to_quaternion()).angle
    return (a.translation - b.translation).length, min(rot, math.tau - rot)


def same(a, b):
    d, r = differs(a, b)
    return d < POS_TOL and r < ROT_TOL, "%.5f m, %.3f deg" % (d, math.degrees(r))


def chain_bones(limb):
    """The limb's bones from the top down (a three-segment leg includes its hock)."""
    return limb["bones"]


def turn(ctrl, rot):
    pose[ctrl].rotation_mode = "QUATERNION"
    pose[ctrl].rotation_quaternion = Matrix.Rotation(rot[0], 3, "X").to_quaternion() @ \
        Matrix.Rotation(rot[1], 3, "Y").to_quaternion() @ Matrix.Rotation(rot[2], 3, "Z").to_quaternion()
    pose[ctrl].rotation_mode = "XYZ"


def deformed():
    deps = bpy.context.evaluated_depsgraph_get()
    deps.update()
    ev = geo.evaluated_get(deps)
    me = ev.to_mesh()
    out = [v.co.copy() for v in me.vertices]
    ev.to_mesh_clear()
    return out


def bend_axis(limb):
    """Hinge axis of the middle joint (the normal of the limb's rest plane) in its
    own rest frame. Positive angles bend the joint further the way it already bends."""
    upper, lower = bones[limb["upper"]], bones[limb["lower"]]
    n = (lower.head_local - upper.head_local).cross(bones[limb["reach"]].head_local - lower.head_local)
    return lower.matrix_local.to_3x3().inverted() @ n.normalized()


rest = [v.co.copy() for v in geo.data.vertices]
ns["reset_pose"](rig)
check(max((a - b).length for a, b in zip(rest, deformed())) < 1e-4, "reset leaves the rest pose unchanged")

# Creatures without arms or legs (a spider's FK legs, a snake's splines) skip
# the limb checks and get the mirror and space checks on the controls they have.
has_limbs = all(n in limbs for n in ("arm_l", "arm_r", "leg_l", "leg_r"))
print("limbs:", ", ".join(sorted(limbs)) or "none")

for side in ("l", "r"):
    for kind in ("arm", "leg"):
        name = "%s_%s" % (kind, side)
        if name not in limbs:
            continue
        limb = limbs[name]

        # IK to FK: pose the IK limb, match, and the FK controls hold the IK pose.
        ns["reset_pose"](rig)
        ik = pose[limb["ik"]]
        ik["ik_fk"] = 1.0
        upper, end = bones[limb["upper"]].head_local, bones[limb["end"]].head_local
        m = ik.matrix.copy()
        m.translation += (upper - end) * 0.3 + Vector((0.0, -0.05, 0.03))
        ik.matrix = m @ Matrix.Rotation(0.3, 4, "X") @ Matrix.Rotation(-0.2, 4, "Z")
        pole = pose[limb["pole"]]
        pm = pole.matrix.copy()
        pm.translation += Vector((0.08 if side == "l" else -0.08, 0.0, 0.0))
        pole.matrix = pm
        if kind == "leg":
            ik["roll"] = 15.0
        if limb.get("hock"):
            turn(limb["hock"]["control"], (0.25, 0.0, 0.1))  # swing the hock
        update()
        ik_pose = [world(b) for b in chain_bones(limb)]
        check(differs(ik_pose[0], world(limb["fk"][0]))[1] > 0.05, "%s: IK pose differs from FK before matching" % name)
        ns["ik_to_fk"](rig, name)
        check(pose[limb["ik"]]["ik_fk"] == 0.0, "%s: IK to FK switches the limb to FK" % name)
        for bone, ctrl, want in zip(chain_bones(limb), limb["fk"], ik_pose):
            ok, how = same(world(ctrl), want)
            check(ok, "%s: FK control %s matches the IK pose (%s)" % (name, ctrl, how))
            ok, how = same(world(bone), want)
            check(ok, "%s: %s does not pop switching to FK (%s)" % (name, bone, how))

        # FK to IK: pose the FK limb (bending the middle joint like a hinge), match.
        ns["reset_pose"](rig)
        pose[limb["ik"]]["ik_fk"] = 0.0
        if kind == "leg":
            pose[limb["ik"]]["roll"] = -20.0  # the IK goal sits off the control
        turn(limb["fk"][0], (0.35, 0.2, -0.3))
        turn(limb["fk"][-1], (0.2, -0.15, 0.25))
        if limb.get("hock"):
            turn(limb["fk"][2], (-0.3, 0.1, 0.15))  # the FK hock segment
        pose[limb["fk"][1]].rotation_mode = "QUATERNION"
        pose[limb["fk"][1]].rotation_quaternion = Quaternion(bend_axis(limb), 0.7)
        pose[limb["fk"][1]].rotation_mode = "XYZ"
        update()
        fk_pose = [world(b) for b in chain_bones(limb)]
        ns["fk_to_ik"](rig, name)
        check(pose[limb["ik"]]["ik_fk"] == 1.0, "%s: FK to IK switches the limb to IK" % name)
        for bone, want in zip(chain_bones(limb), fk_pose):
            ok, how = same(world(bone), want)
            check(ok, "%s: %s does not pop switching to IK (%s)" % (name, bone, how))

        # A straight limb has no bend plane; the pole still lands in front of the joint.
        ns["reset_pose"](rig)
        pose[limb["ik"]]["ik_fk"] = 0.0
        pose[limb["fk"][1]].rotation_mode = "QUATERNION"
        pose[limb["fk"][1]].rotation_quaternion = Quaternion(bend_axis(limb), 0.0)
        pose[limb["fk"][1]].rotation_mode = "XYZ"
        update()
        rest_bend = (bones[limb["pole"]].head_local - bones[limb["lower"]].head_local).normalized()
        ns["fk_to_ik"](rig, name)
        bend = (pose[limb["pole"]].head - pose[limb["lower"]].head).normalized()
        check(bend.dot(rest_bend) > 0.99, "%s: FK to IK keeps the pole in front of the joint" % name)

def mirrored(p):
    return Vector((2 * center_x - p.x, p.y, p.z))


def flip(v):
    return Vector((-v.x, v.y, v.z))


def check_mirrored(bl, br):
    """Posed bone `br` is the mirror image of posed bone `bl`, roll included."""
    update()
    head_err = (pose[br].head - mirrored(pose[bl].head)).length
    tail_err = (pose[br].tail - mirrored(pose[bl].tail)).length
    check(head_err < POS_TOL and tail_err < POS_TOL,
          "mirror: %s is the mirror image of %s (%.5f, %.5f m)" % (br, bl, head_err, tail_err))
    # The roll mirrors too: each side axis maps onto the twin's (up to its sign).
    sign = 1.0 if flip(bones[bl].x_axis).dot(bones[br].x_axis) > 0 else -1.0
    err = 1.0 - sign * flip(pose[bl].x_axis).dot(pose[br].x_axis)
    check(err < 1e-4, "mirror: %s turns like %s mirrored (%.6f)" % (br, bl, err))


# Mirror: pose the left side, copy it to the right, and the right is its mirror image.
ns["reset_pose"](rig)
twin = {l: r for l, r in data["mirror"]}
if has_limbs:
    arm, leg = limbs["arm_l"], limbs["leg_l"]
    pose[arm["fk"][0]].rotation_euler = (0.3, -0.2, 0.5)
    pose[arm["fk"][1]].rotation_euler = (0.0, 0.3, -0.6)
    pose[arm["fk"][2]].rotation_euler = (0.2, 0.4, 0.1)
    pose[leg["ik"]].location = (0.04, 0.1, -0.05)
    pose[leg["ik"]].rotation_euler = (0.2, 0.3, -0.1)
    pose[leg["ik"]]["roll"] = 25.0
    pose[leg["pole"]].location = (0.05, 0.0, 0.02)
    if leg.get("hock"):
        pose[leg["hock"]["control"]].rotation_euler = (0.2, 0.0, 0.1)
else:
    # Every left control turned (an FK leg bends at each joint).
    for k, left in enumerate(twin):
        pose[left].rotation_euler = (0.2 + 0.05 * k, -0.3, 0.4)
update()
n = ns["mirror_pose"](rig, None, "left")
check(n == len(data["mirror"]), "mirror with nothing selected copies all %d left controls" % n)

if has_limbs:
    for kl, kr in (("arm_l", "arm_r"), ("leg_l", "leg_r")):
        for bl, br in zip(chain_bones(limbs[kl]), chain_bones(limbs[kr])):
            check_mirrored(bl, br)
    check(pose[limbs["leg_r"]["ik"]]["roll"] == 25.0, "mirror copies the foot roll")
    moved = max((pose[bl].head - bones[bl].head_local).length for bl in chain_bones(arm))
    check(moved > 0.05, "the posed left arm moved (%.3f m)" % moved)
else:
    # Each pair of controls ends up mirror images.
    for left, right in data["mirror"]:
        check_mirrored(left, right)
    if data["mirror"]:
        moved = max((pose[l].tail - bones[l].tail_local).length for l in twin)
        check(moved > 0.01, "the posed left controls moved (%.3f m)" % moved)
    else:
        print("no left/right controls to mirror")

# Mirror selected only: just the picked control's twin changes.
ns["reset_pose"](rig)
if has_limbs:
    picked, other, other_twin = arm["fk"][0], leg["ik"], limbs["leg_r"]["ik"]
    pose[picked].rotation_euler = (0.0, 0.0, 0.5)
    pose[other].location = (0.0, 0.1, 0.0)
elif len(twin) > 1:
    picked, other = list(twin)[:2]
    other_twin = twin[other]
    pose[picked].rotation_euler = (0.0, 0.0, 0.5)
    pose[other].rotation_euler = (0.0, 0.0, 0.5)
else:
    picked = None
if picked:
    for p in pose:
        ns["set_bone_selected"](p, p.name == picked)
    check(ns["selected_controls"](rig) == [picked], "one control picked")
    ns["mirror_pose"](rig, ns["selected_controls"](rig), "right")
    check(pose[twin[picked]].matrix_basis != Matrix.Identity(4), "mirror selected copies the picked control")
    check(pose[other_twin].matrix_basis == Matrix.Identity(4), "mirror selected leaves other controls alone")

# Flip: pose both sides and the centre differently, flip, and every deforming
# bone lands where its twin was, mirrored (a centre bone on its own mirror).
def bone_twin_map():
    """Deforming bones paired by mirrored rest positions; centre bones pair with themselves."""
    deform = [b for b in bones if b.use_deform]
    out = {}
    for b in deform:
        want_h, want_t = mirrored(b.head_local), mirrored(b.tail_local)
        best = min(deform, key=lambda o: (o.head_local - want_h).length + (o.tail_local - want_t).length)
        if (best.head_local - want_h).length + (best.tail_local - want_t).length < 1e-3:
            out[b.name] = best.name
    return out


def pose_some_of_everything():
    ns["reset_pose"](rig)
    names = data["controls"]
    for k, n in enumerate(names):
        pb = pose[n]
        if k % 3 == 0:
            continue
        pb.rotation_mode = "XYZ"
        pb.rotation_euler = (0.05 * (k % 5) - 0.1, 0.07 * (k % 4) - 0.1, 0.06 * (k % 3) + 0.02)
        if not all(pb.lock_location) and k % 2 == 0:
            pb.location = (0.01 * (k % 3), -0.008 * (k % 2 + 1), 0.006)
    if has_limbs:
        pose[limbs["leg_l"]["ik"]]["roll"] = 20.0
        pose[limbs["arm_r"]["ik"]]["ik_fk"] = 1.0
    update()


bone_twin = bone_twin_map()
centre_bones = [b for b, t in bone_twin.items() if b == t]
check(len(centre_bones) > 0 and (len(bone_twin) > len(centre_bones) or not data["mirror"]), "flip: %d deforming bones paired, %d in the centre"
      % (len(bone_twin), len(centre_bones)))
pose_some_of_everything()
before_pose = {n: pose[n].matrix_basis.copy() for n in data["controls"]}
before = {b: (pose[b].head.copy(), pose[b].tail.copy(), pose[b].x_axis.copy()) for b in bone_twin}
settings_before = {n: ns["control_settings"](pose[n]) for n in data["controls"]}
n = ns["flip_pose"](rig)
check(n == len(data["controls"]), "flip with nothing selected flips all %d controls" % n)
update()
worst = 0.0
for b, t in bone_twin.items():
    h, tl, _ = before[t]
    worst = max(worst, (pose[b].head - mirrored(h)).length, (pose[b].tail - mirrored(tl)).length)
check(worst < POS_TOL, "flip: every deforming bone is its twin's mirror image (%.5f m)" % worst)
moved = max((before[b][0] - pose[b].head).length for b in centre_bones)
check(moved > 1e-3 or not any(before_pose[n] != Matrix.Identity(4) for n in data["controls"]),
      "flip moves the centre (%.4f m)" % moved)
twin = {l: r for l, r in data["mirror"]}
twin.update({r: l for l, r in data["mirror"]})
check(all(ns["control_settings"](pose[twin.get(n, n)]) == settings_before[n] for n in data["controls"]),
      "flip swaps ik_fk, roll and the other settings between twins")
flipped_pose = {n: pose[n].matrix_basis.copy() for n in data["controls"]}
ns["flip_pose"](rig)
err = max(max(abs(a - b) for ra, rb in zip(pose[n].matrix_basis, before_pose[n]) for a, b in zip(ra, rb))
          for n in data["controls"])
check(err < 1e-4, "flipping twice gives the pose back (%.6f)" % err)

# Pose library: save, reset, apply; apply mirrored is the flip; selected only; delete.
ns["save_pose"](rig, "test pose")
check(sorted(ns["pose_library"](rig)) == ["test pose"], "the pose is in the library")
ns["reset_pose"](rig)
ns["apply_pose"](rig, "test pose")
err = max(max(abs(a - b) for ra, rb in zip(pose[n].matrix_basis, before_pose[n]) for a, b in zip(ra, rb))
          for n in data["controls"])
check(err < 1e-5 and all(ns["control_settings"](pose[n]) == settings_before[n] for n in data["controls"]),
      "applying the saved pose puts every control back (%.6f)" % err)
ns["reset_pose"](rig)
ns["apply_pose"](rig, "test pose", None, True)
err = max(max(abs(a - b) for ra, rb in zip(pose[n].matrix_basis, flipped_pose[n]) for a, b in zip(ra, rb))
          for n in data["controls"])
check(err < 1e-4, "applying it mirrored gives the flipped pose (%.6f)" % err)
ns["reset_pose"](rig)
one = next(n for n in data["controls"] if before_pose[n] != Matrix.Identity(4))
check(ns["apply_pose"](rig, "test pose", [one]) == 1, "applying to the selected controls only")
check(all(pose[n].matrix_basis == Matrix.Identity(4) for n in data["controls"] if n != one),
      "the other controls stay put")
ns["save_pose"](rig, "second", [one])
check(len(json.loads(rig["hive_poses"])["second"]) == 1, "a pose of the selected controls holds only them")
ns["delete_pose"](rig, "second")
check(sorted(ns["pose_library"](rig)) == ["test pose"], "delete takes a pose out")

# Select all controls.
count = ns["select_controls"](rig)
check(sorted(ns["selected_controls"](rig)) == sorted(data["controls"]) and count == len(data["controls"]),
      "select all picks every control (%d)" % count)
check(not any(ns["bone_selected"](p) for p in pose if p.name not in data["controls"]), "select all picks only controls")

# Space switch without a pop: twist the chest, move the IK hand into chest space
# (without arms, the first control that has spaces, into its last space).
ns["reset_pose"](rig)
if has_limbs:
    hand_ik = arm["ik"]
    pose[hand_ik]["ik_fk"] = 1.0
    target = 2
else:
    hand_ik = next(c for c in data["controls"] if c in data["spaces"])
    target = len(data["spaces"][hand_ik]) - 1
spaces = data["spaces"][hand_ik]
chest = spaces[target]
pose[chest].rotation_euler = (0.0, 0.4, 0.2)
pose[hand_ik].location = (0.02, -0.05, 0.03)
update()
before = world(hand_ik)
ns["switch_space"](rig, hand_ik, target)
ok, how = same(world(hand_ik), before)
check(pose[hand_ik]["space"] == target and ok, "switching %s to %s does not move it (%s)" % (hand_ik, chest, how))
ns["switch_space"](rig, hand_ik, 0)
ok, how = same(world(hand_ik), before)
check(pose[hand_ik]["space"] == 0 and ok, "switching back to %s does not move it (%s)" % (spaces[0], how))

# The Hive tab draws its Animation box (headless Blender has no UI, so a stand-in layout).
from types import SimpleNamespace  # noqa: E402
from unittest import mock  # noqa: E402

layout = mock.MagicMock()
ns["HIVE_PT_panel"].draw(SimpleNamespace(layout=layout),
                         SimpleNamespace(active_object=rig, active_pose_bone=pose[hand_ik], mode="POSE"))
drawn = [c.args[0] for c in layout.mock_calls if c.args and isinstance(c.args[0], str) and c.args[0].startswith("hive.")]
check(drawn.count("hive.match_limb") == 2 * len(limbs) and "hive.switch_space" in drawn and "hive.mirror_pose" in drawn,
      "the Hive tab shows match buttons for %d limbs, mirror, reset and the space buttons" % len(limbs))
check("hive.flip_pose" in drawn and "hive.save_pose" in drawn and drawn.count("hive.apply_pose") == 2
      and "hive.delete_pose" in drawn, "the Hive tab shows Flip Pose and the pose library with its pose")
for p in pose:
    ns["set_bone_selected"](p, False)
check(bpy.ops.hive.flip_pose() == {"FINISHED"} and bpy.ops.hive.apply_pose(name="test pose", mirrored=True) == {"FINISHED"},
      "Flip Pose and Apply Pose run from the Hive tab")
check(bpy.ops.hive.save_pose(name="from the tab") == {"FINISHED"} and "from the tab" in ns["pose_library"](rig),
      "Save Pose runs from the Hive tab")

# Reset everything through the panel's operator.
if has_limbs:
    pose[arm["fk"][0]].rotation_euler = (0.4, 0.0, 0.0)
    pose[leg["ik"]]["roll"] = 30.0
else:
    for name in data["controls"][1::3]:
        pose[name].rotation_euler = (0.4, 0.0, 0.0)
pose[hand_ik]["space"] = target
check(bpy.ops.hive.reset_pose(selected=False) == {"FINISHED"}, "Reset All runs from the Hive tab")
update()
check(all(pose[n].matrix_basis == Matrix.Identity(4) for n in data["controls"]), "reset puts every control at bind")
check(all(pose[c][p] == v for c, d in data["defaults"].items() for p, v in d.items()), "reset restores default settings")
check(max((a - b).length for a, b in zip(rest, deformed())) < 1e-4, "after reset the mesh is back at rest")

# Matching across a frame range: animate one limb, match it on every frame, and
# each frame keeps its pose with the other side keyed onto it.
if has_limbs:
    name = "arm_l"
    limb = limbs[name]
    scene = bpy.context.scene
    frames = list(range(1, 6))
    ns["reset_pose"](rig)
    ik = pose[limb["ik"]]
    for f in frames:
        scene.frame_set(f)
        ik["ik_fk"] = 0.0
        ik.keyframe_insert('["ik_fk"]')
        for k, ctrl in enumerate(limb["fk"]):
            pb = pose[ctrl]
            pb.rotation_mode = "XYZ"
            pb.rotation_euler = (0.1 * f, -0.05 * f * (k + 1), 0.15 * f) if k != 1 else (0.0, 0.0, 0.0)
            if k == 1:
                pb.rotation_mode = "QUATERNION"
                pb.rotation_quaternion = Quaternion(bend_axis(limb), 0.2 + 0.15 * f)
                pb.rotation_mode = "XYZ"
            pb.keyframe_insert("rotation_euler")
    fk_poses = {}
    for f in frames:
        scene.frame_set(f)
        fk_poses[f] = [world(b) for b in chain_bones(limb)]
    check(differs(fk_poses[1][0], fk_poses[5][0])[1] > 0.1, "range: the FK arm is animated")
    scene.frame_set(3)
    n = ns["match_range"](rig, name, "ik", frames)
    check(n == len(frames) and scene.frame_current == 3, "range: FK to IK visits %d frames and comes back" % n)
    for f in frames:
        scene.frame_set(f)
        check(pose[limb["ik"]]["ik_fk"] == 1.0, "range: frame %d is keyed to IK" % f)
        for bone, want in zip(chain_bones(limb), fk_poses[f]):
            ok, how = same(world(bone), want)
            check(ok, "range: frame %d, %s keeps its FK pose in IK (%s)" % (f, bone, how))
    # And back: the IK keys just made drive the arm; match FK onto them.
    n = ns["match_range"](rig, name, "fk", frames)
    for f in frames:
        scene.frame_set(f)
        check(pose[limb["ik"]]["ik_fk"] == 0.0, "range: frame %d is keyed back to FK" % f)
        for bone, ctrl, want in zip(chain_bones(limb), limb["fk"], fk_poses[f]):
            ok, how = same(world(bone), want)
            check(ok, "range: frame %d, %s keeps its pose back in FK (%s)" % (f, bone, how))
    # The panel's range switch reaches the match buttons.
    rig["hive_match_range"] = True
    layout = mock.MagicMock()
    ns["HIVE_PT_panel"].draw(SimpleNamespace(layout=layout),
                             SimpleNamespace(active_object=rig, active_pose_bone=None, mode="POSE"))
    check(any(c.args[:1] == (rig,) for c in layout.mock_calls if c.args), "range: the Hive tab shows the range switch")
    rig["hive_match_range"] = False
print("blender animator tools check passed")
