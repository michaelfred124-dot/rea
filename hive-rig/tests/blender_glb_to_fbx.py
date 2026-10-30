"""Make a binary FBX test file from a .glb with real Blender's importer and
exporter (no hive-rig code involved), so hive-rig's FBX reader can be checked
on a file written by another program.

    python -I blender_glb_to_fbx.py model.glb model.fbx [--leaf-bones]

Blender writes FBX 7.4 with zlib-compressed arrays, Y up, in its own unit
scale, its bones as LimbNodes (with `_end` leaf bones when --leaf-bones), the
skin as clusters and the shape keys as blend shapes. Downloaded models are
untrusted data: run this with `python -I`.
"""

import argparse
import json
import struct
import sys

import bpy

ap = argparse.ArgumentParser()
ap.add_argument("source")
ap.add_argument("out")
ap.add_argument("--leaf-bones", action="store_true")
argv = sys.argv[sys.argv.index("--") + 1 :] if "--" in sys.argv else sys.argv[1:]
args = ap.parse_args(argv)

def scene_nodes(path):
    """Names of the nodes in the .glb's default scene (Blender also brings in
    nodes no scene uses, which a viewer, and hive-rig, leave out)."""
    data = open(path, "rb").read()
    n, kind = struct.unpack_from("<II", data, 12)
    assert kind == 0x4E4F534A, "not a .glb"
    g = json.loads(data[20 : 20 + n])
    scene = g["scenes"][g.get("scene", 0)]
    names, stack = set(), list(scene["nodes"])
    while stack:
        node = g["nodes"][stack.pop()]
        names.add(node.get("name", ""))
        stack.extend(node.get("children", []))
    return names


bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=args.source)
keep = scene_nodes(args.source)
for ob in list(bpy.data.objects):
    if ob.type == "MESH" and ob.name not in keep:
        bpy.data.objects.remove(ob)
# The glTF importer leaves the first animation on; FBX gets the rest pose.
for ob in bpy.data.objects:
    if ob.animation_data:
        ob.animation_data.action = None
    if ob.type == "ARMATURE":
        for pb in ob.pose.bones:
            pb.matrix_basis.identity()
bpy.ops.export_scene.fbx(
    filepath=args.out,
    use_selection=False,
    add_leaf_bones=args.leaf_bones,
    bake_anim=False,
    path_mode="COPY",
    embed_textures=True,
    mesh_smooth_type="OFF",
)
data = open(args.out, "rb").read(27)
assert data.startswith(b"Kaydara FBX Binary  \x00"), "not a binary FBX"
print(f"wrote {args.out}: FBX {int.from_bytes(data[23:27], 'little')}, "
      f"{sum(1 for o in bpy.data.objects if o.type == 'MESH')} meshes, "
      f"{sum(len(o.data.bones) for o in bpy.data.objects if o.type == 'ARMATURE')} bones")
