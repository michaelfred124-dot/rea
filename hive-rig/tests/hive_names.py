"""Exported names for internal bone and control names, as src/naming.rs renders them.

The DCC checks are written against the internal (default) names; this lets them
check a rig exported with another naming convention too, including the Unity
Humanoid and Mixamo skeleton maps.
"""

# (internal name, ending _l for a sided bone; Unity human bone; Mixamo bone),
# as HUMAN_BONES in src/naming.rs.
HUMAN_BONES = [
    ("pelvis", "Hips", "Hips"),
    ("spine_01", "Spine", "Spine"),
    ("spine_02", "Chest", "Spine1"),
    ("spine_03", "UpperChest", "Spine2"),
    ("neck_01", "Neck", "Neck"),
    ("head", "Head", "Head"),
    ("jaw", "Jaw", None),
    ("eye_l", "Eye", "Eye"),
    ("clavicle_l", "Shoulder", "Shoulder"),
    ("upperarm_l", "UpperArm", "Arm"),
    ("lowerarm_l", "LowerArm", "ForeArm"),
    ("hand_l", "Hand", "Hand"),
    ("thigh_l", "UpperLeg", "UpLeg"),
    ("calf_l", "LowerLeg", "Leg"),
    ("foot_l", "Foot", "Foot"),
    ("ball_l", "Toes", "ToeBase"),
]
for finger, unity, mixamo in (("thumb", "Thumb", "HandThumb"), ("index", "Index", "HandIndex"),
                              ("middle", "Middle", "HandMiddle"), ("ring", "Ring", "HandRing"),
                              ("pinky", "Little", "HandPinky")):
    for k, part in enumerate(("Proximal", "Intermediate", "Distal")):
        HUMAN_BONES.append(("%s_%02d_l" % (finger, k + 1), "%s %s" % (unity, part), "%s%d" % (mixamo, k + 1)))

MIXAMO_PREFIX = "mixamorig:"


def human_entry(internal):
    """(side word, Unity name, Mixamo name) of an internal bone, or None."""
    for name, unity, mixamo in HUMAN_BONES:
        if not name.endswith("_l"):
            if name == internal:
                return "", unity, mixamo
            continue
        base = name[:-2]
        if internal in (base + "_l", base + "_r"):
            return ("Left" if internal.endswith("_l") else "Right"), unity, mixamo
    return None


def mapped(skeleton, internal):
    """The engine map's name for an internal bone, or None."""
    entry = human_entry(internal)
    if entry is None:
        return None
    side, unity, mixamo = entry
    if skeleton == "unity":
        return (side + unity).replace(" ", "")
    if skeleton == "mixamo" and mixamo:
        return MIXAMO_PREFIX + side + mixamo
    return None


def final(naming, internal):
    if not naming:
        return internal
    control = internal.endswith("_ctrl")
    skeleton = None if control else naming.get("skeleton")
    if skeleton:
        name = mapped(skeleton, internal)
        if name:
            return name
    tokens = internal.split("_")[:-1] if control else internal.split("_")
    at = max((i for i, t in enumerate(tokens) if t in ("l", "r") and i > 0), default=None)
    if at is None:
        name, side, desc = tokens, naming["center"], []
    else:
        name, side, desc = tokens[:at], naming["left" if tokens[at] == "l" else "right"], tokens[at + 1:]
    text = (naming["control"] if control else naming["joint"]).replace("{name}", "_".join(name))
    text = text.replace("{side}", side).replace("{desc}", "_".join(desc))
    prefix = MIXAMO_PREFIX if skeleton == "mixamo" else ""
    return prefix + "_".join(t for t in text.split("_") if t)
