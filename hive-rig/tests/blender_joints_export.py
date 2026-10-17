"""Make a skinned character the way another tool would hand it over: import a
hive-rig .glb into real Blender, rename its bones Mixamo-style, move one face
joint, and export it again. `hive-rig rig` on the result must read those joints
back as guides (checked by the CI step that runs this).

    python blender_joints_export.py <in>.glb <out>.glb
"""

import sys

import bpy
from mathutils import Vector

args = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else sys.argv[1:]
src, dst = args[0], args[1]

MIXAMO = {
    "pelvis": "Hips",
    "clavicle_l": "LeftShoulder",
    "upperarm_l": "LeftArm",
    "lowerarm_l": "LeftForeArm",
    "hand_l": "LeftHand",
    "clavicle_r": "RightShoulder",
    "upperarm_r": "RightArm",
    "lowerarm_r": "RightForeArm",
    "hand_r": "RightHand",
    "thigh_l": "LeftUpLeg",
    "calf_l": "LeftLeg",
    "foot_l": "LeftFoot",
    "ball_l": "LeftToeBase",
    "thigh_r": "RightUpLeg",
    "calf_r": "RightLeg",
    "foot_r": "RightFoot",
    "ball_r": "RightToeBase",
    "spine_01": "Spine",
    "spine_02": "Spine1",
    "spine_03": "Spine2",
    "neck_01": "Neck",
    "head": "Head",
}

bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=src)
arm = next(o for o in bpy.data.objects if o.type == "ARMATURE")
bpy.context.view_layer.objects.active = arm
bpy.ops.object.mode_set(mode="EDIT")
eb = arm.data.edit_bones
# Raise the left inner brow 1 cm, as an artist adjusting the face would.
brow = eb["brow_in_l"]
brow.head += Vector((0.0, 0.0, 0.01))
brow.tail += Vector((0.0, 0.0, 0.01))
bpy.ops.object.mode_set(mode="OBJECT")
for old, new in MIXAMO.items():
    arm.data.bones[old].name = "mixamorig:" + new
# Vertex groups follow bone renames in Blender, so the skin stays intact.
bpy.ops.export_scene.gltf(filepath=dst, export_format="GLB")
print("wrote", dst)
