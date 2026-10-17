# hive-rig

An automatic humanoid rigger written in Rust. Give it a character mesh and it
produces a skinned skeleton plus an animator control rig for **Blender**,
**Maya** and **Unreal Engine**. The rig is built from premade components placed
by editable guides, so you can move guides, add fingers or a tail, and rebuild
without losing your skin weights.

```bash
cd hive-rig
cargo run --release -- rig path/to/character.glb -o out/
# or try it without an asset:
cargo run --release -- demo -o out/            # T-pose test character
cargo run --release -- demo -o out/ --arm-drop 45   # A-pose
```

On Windows you can also just double-click `hive-rig.exe`: it asks for a
character file (drag it into the window, or press Enter for the demo), rigs it
and opens the output folder. Dropping a file onto the exe does the same.

## What you get

| File | Use it in | What it contains |
| --- | --- | --- |
| `<name>.glb` | Blender, Unreal, any glTF tool | Mesh, skeleton and skin weights |
| `<name>_blender.py` | Blender 3.6+ | Mesh, armature, weights **and controls** |
| `<name>_maya.py` | Maya 2022+ | Mesh, oriented joints, skinCluster **and controls** |
| `<name>_unreal.py` | Unreal Engine 5.2+ | Imports the .glb and builds a **Control Rig Blueprint** |
| `<name>.guides.json` | hive-rig, Blender, Maya | The editable component layout |
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

## Components and guides

A rig is a list of components, each placed by a few guide points:

| Component | Guides | Builds |
| --- | --- | --- |
| root | root | `root` bone, `root_ctrl` |
| spine | pelvis, chest | `pelvis`, `spine_01`… (3 joints by default), COG, pelvis and spine controls |
| head | neck, head, head_top | `neck_01`, `head`, controls with space switching |
| arm | clavicle, shoulder, elbow, wrist, hand_tip | clavicle, IK/FK arm, stretch, spaces, twist bones |
| leg | hip, knee, ankle, ball, toe, heel | IK/FK leg, reverse foot, stretch, spaces, twist bones |
| chain | two or more | an FK chain: fingers, tails, ears, hair, props |

The first rig detects the body and places the default biped. From then on you
can change the layout and rebuild:

```bash
hive-rig add out/hero.guides.json fingers           # five fingers on each hand
hive-rig add out/hero.guides.json tail --joints 6
hive-rig add out/hero.guides.json chain --parent head --joints 2 --id ear_l
hive-rig remove out/hero.guides.json tail           # also removes anything attached to it
hive-rig components out/hero.guides.json            # list the layout
hive-rig rebuild out/hero.hive.json                 # rebuild from the edited guides
hive-rig rig other_character.glb --guides out/hero.guides.json   # reuse a layout
```

`guides.json` is plain JSON, so you can also edit guide positions, the spine's
`joints` option, or `twist: false` on a limb by hand.

### Editing guides in Blender and Maya

The Blender script adds a **Hive** tab to the 3D view sidebar (press N). The
Maya script opens a **Hive Rig** window. Both have the same tools:

* **Edit Guides** puts out a sphere (Blender) or locator (Maya) for every guide
  and hides the controls. Move the guides where you want the joints.
* **Mirror Left to Right** copies the left side's guides onto the right.
* **Add Fingers / Add Tail** and **Remove Component** (select one of its guides first).
* **Rebuild Rig** rebuilds the skeleton and controls from the guides. The mesh
  object is kept, along with its materials and modifiers. With **Keep Weights**
  on, every vertex keeps its current weights unless the new rig gives it to a
  bone that didn't exist before, such as a newly added finger. Turn it off to
  get fresh automatic weights everywhere.

Rebuilding runs the `hive-rig` program that generated the script. If you've
moved it since, set `HIVE_EXE` at the top of the script. Running the script
again in a file that already has the rig only brings back the tab or window.

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
2. **Guides** (`src/components.rs`): the landmarks place the default biped
   layout, mirrored for the right side. Each component builds its bones and
   controls from its guides: 31 bones for the biped, including 8 twist helpers.
3. **Skin** (`src/skin.rs`): every vertex is assigned to a body region (torso,
   left/right arm or leg), so weights can't bleed between limbs that are close
   together. Weights fall off with inverse distance to each bone, are limited
   to 4 influences, and are smoothed over the surface.
4. **Controls** (`src/controls.rs`): the DCC-neutral description of every control
   (its parent, shape, colour and driven bone) and the IK chains, as built by the components.
5. **Export** (`src/export/`): the glTF writer and the three builder templates
   in `templates/`, with the rig data embedded so each script is self-contained.

## Testing

```bash
cargo test                       # landmarks, skeleton, skin, controls, glTF round trip
python3 tests/dcc_stub_run.py maya out/hive_demo_maya.py      # Python-level check
python3 tests/dcc_stub_run.py unreal out/hive_demo_unreal.py  # Python-level check
pip install bpy && python tests/blender_check.py out/hive_demo_blender.py out/hive_demo.glb
python tests/blender_guides_check.py out/hive_demo_blender.py           # guides and rebuild
python3 tests/dcc_stub_run.py maya out/hive_demo_maya.py --guides       # Maya guide tools
```

`blender_check.py` builds the rig in real Blender and checks that it works as
a rig: the rest pose is unchanged, feet and hands reach their IK goals, knees
bend forward and elbows bend back, and FK rotates the limbs. It checks the heel,
ball and toe roll stages, stretchy IK, the forearm twist share, and space
switching (no pop at rest, and the IK hand follows the chest only in chest
space). It also checks that every driver uses a simple expression, so the rig
works without enabling Python auto-run. Finally, the COG moves the body, the
mesh deforms, and the `.glb` re-imports with weights on every bone.
`blender_guides_check.py` edits the guides in real Blender. It moves the elbow,
mirrors it, paints a weight, adds fingers and a tail, and rebuilds. Then it
checks that the bones moved, the painted weight survived, the new fingers got
weights and controls, and a finger control bends the finger.
CI runs all of this (`.github/workflows/hive-rig.yml`).

The Maya and Unreal builders, and the Maya guide tools, are checked against stub
APIs. That catches script errors but doesn't prove the application calls behave
as intended. They haven't been run inside Maya or Unreal yet.

## Not yet included

Facial bones, quadruped and creature templates, extra arms or legs on the same
side, guide editing in Unreal (rebuild with the CLI and rerun the script), IK/FK
and space *matching* (snapping without a pop when switching mid-animation), and
FBX output.
