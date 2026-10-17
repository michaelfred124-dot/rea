# hive-rig

An automatic humanoid rigger written in Rust. Give it a character mesh and it
produces a skinned skeleton plus an animator control rig for **Blender**,
**Maya** and **Unreal Engine**.

```bash
cd hive-rig
cargo run --release -- rig path/to/character.glb -o out/
# or try it without an asset:
cargo run --release -- demo -o out/            # T-pose test character
cargo run --release -- demo -o out/ --arm-drop 45   # A-pose
```

## What you get

| File | Use it in | What it contains |
| --- | --- | --- |
| `<name>.glb` | Blender, Unreal, any glTF tool | Mesh, skeleton and skin weights |
| `<name>_blender.py` | Blender 3.6+ | Mesh, armature, weights **and controls** |
| `<name>_maya.py` | Maya 2022+ | Mesh, oriented joints, skinCluster **and controls** |
| `<name>_unreal.py` | Unreal Engine 5.2+ | Imports the .glb and builds a **Control Rig Blueprint** |
| `<name>.hive.json` | Your own tools | The full rig description |

Controls don't survive generic formats like FBX or glTF, so each application
gets a generated Python script that rebuilds the control rig with native
tools. The skin and skeleton are written as glTF because it's an open format
that Blender and Unreal import natively.

### The control rig

* `root_ctrl` (world placement) and `cog_ctrl` (moves the body, feet stay planted)
* FK controls for pelvis, three spine joints, neck, head, clavicles and toes
* Arms and legs have FK controls, an IK control, a pole-vector control and an
  `ik_fk` blend (0 = FK, 1 = IK). Legs start in IK and arms start in FK.
* `stretch` on every limb's IK control (0 = rigid, 1 = reaches any goal)
* Reverse foot: `roll` on each foot's IK control. Negative values rock back on
  the heel. Positive values lift the heel around the ball, then around the toe
  tip once the roll passes 30°. The toe control stays usable in both modes.
* Space switching via `space` on the IK hands (world, body, chest or head), the
  IK feet (world or body), the poles (world or their IK goal) and the head
  (neck, chest, body or world). Unreal uses Control Rig's built-in control
  spaces (UE 5.4+).
* Twist bones (`upperarm_twist_01_l`, `lowerarm_twist_01_l`, `thigh_twist_01_l`,
  `calf_twist_01_l`…) take the far half of each limb's skin. Upper twists cancel
  half of their own twist and lower twists carry half of the hand or foot twist.
  Blender and Maya drive them. In Unreal they are skinned but follow their
  parents, so drive them in a post-process Anim Blueprint as you would for the mannequin.
* Colour coding: left limbs are blue, right limbs are red and the centre is yellow.

Bone names follow the Unreal mannequin (`pelvis`, `spine_01`, `upperarm_l`,
`calf_r`, `ball_l`…), so the skeleton retargets cleanly in Unreal.

### Running the builder scripts

* **Blender:** Scripting tab > Open > Run Script. Headless:
  `blender --background --python out/<name>_blender.py -- --save out/<name>.blend`
* **Maya:** Script Editor > Python tab > File > Load Script > Execute.
  Headless: `mayapy out/<name>_maya.py --save out/<name>.ma`
* **Unreal:** enable the *Python Editor Script Plugin* and *Control Rig*, then
  use Tools > Execute Python Script. The script imports the `.glb` into
  `/Game/HiveRig/<name>` and creates the Control Rig next to the mesh. Set
  `SKELETAL_MESH_PATH` at the top of the script to reuse a mesh you've already imported.

## Input requirements

* `.obj`, `.gltf` or `.glb`
* One humanoid standing on the ground in T-pose or A-pose
* Y-up, facing +Z (the glTF convention, and the default for Blender's glTF
  and OBJ exporters)
* Arms clear of the body and legs apart

If the legs, arms or neck can't be found from the geometry, the rigger falls
back to average human proportions and prints a warning.

## How it works

1. **Landmarks** (`src/landmarks.rs`): the mesh is cut with horizontal planes.
   The front silhouette intervals of each cut show where the legs split
   (crotch), where the arms leave the torso, and where the neck is narrowest.
   Joints are then pulled to the centroid of the nearby geometry so they sit
   inside the limbs. Elbows get a slight backward bend and knees a slight
   forward bend, which tells IK solvers which way to fold.
2. **Skeleton** (`src/biped.rs`): 31 bones, including 8 twist helpers, built from
   the landmarks and mirrored for the right side.
3. **Skin** (`src/skin.rs`): every vertex is assigned to a body region (torso,
   left/right arm or leg), so weights can't bleed between limbs that are close
   together. Weights fall off with inverse distance to each bone, are limited
   to 4 influences, and are smoothed over the surface.
4. **Controls** (`src/controls.rs`): a DCC-neutral description of every control,
   its parent, shape, colour and driven bone, plus the IK chains.
5. **Export** (`src/export/`): the glTF writer and the three builder templates
   in `templates/`, with the rig data embedded so each script is self-contained.

## Testing

```bash
cargo test                       # landmarks, skeleton, skin, controls, glTF round trip
python3 tests/dcc_stub_run.py maya out/hive_demo_maya.py      # Python-level check
python3 tests/dcc_stub_run.py unreal out/hive_demo_unreal.py  # Python-level check
pip install bpy && python tests/blender_check.py out/hive_demo_blender.py out/hive_demo.glb
```

`blender_check.py` builds the rig in real Blender and checks that it works as
a rig: the rest pose is unchanged, feet and hands reach their IK goals, knees
bend forward and elbows bend back, and FK rotates the limbs. It checks the heel,
ball and toe roll stages, stretchy IK, the forearm twist share, and space
switching (no pop at rest, and the IK hand follows the chest only in chest
space). It also checks that every driver uses a simple expression, so the rig
works without enabling Python auto-run. Finally, the COG moves the body, the
mesh deforms, and the `.glb` re-imports with weights on every bone.
CI runs all of this (`.github/workflows/hive-rig.yml`).

The Maya and Unreal builders are checked against stub APIs, which catches
script errors but doesn't prove the application calls behave as intended.
They haven't been run inside Maya or Unreal yet.

## Not yet included

Fingers, facial bones, editable guides with rebuilds, quadruped and creature
templates, IK/FK and space *matching* (snapping without a pop when switching
mid-animation), and FBX output.
