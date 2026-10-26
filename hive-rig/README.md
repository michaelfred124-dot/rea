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
cargo run --release -- demo -o out/ --quadruped     # a dog: spine, head, tail, four legs
cargo run --release -- demo -o out/ --creature bird # also dog, spider, snake, horse, dinosaur
cargo run --release -- demo -o out/ --bendy --soft 0.3   # bendy arms and legs with soft IK
```

## The Hive Rig app

`HiveRig` (`HiveRig.exe` on Windows) is a desktop rigging app built on the same
library. The 3D view fills the window and everything else floats over it. Open a
character (or the demo) and the guides appear on it as coloured joint dots:
yellow for the centre, blue for the left, red for the right. Drag a guide to move
a joint; with Symmetry on, its twin on the other side follows.

* **The dock** along the bottom holds the tools: Move, Add, Mirror (right-click
  for right to left), Symmetry, the Guides / Skeleton / Controls views, Mesh,
  X-ray, Weights, Undo, Redo, Frame and Export. The honeycomb button in the
  middle builds the rig.
* **The outliner** (left) lists the components as their attachment tree: each
  component sits under the one owning the bone it hangs from (a leg hung from
  the head is listed under the head). **Properties** (right)
  shows the rig summary, the pre-build [checks](#checks-before-building), [blendshapes](#blendshapes-and-corrective-shapes) (a searchable shape list with a slider each, correctives, in-betweens and the sculpt brushes), [skin weights](#skin-weights-in-the-app) (the mesh tool, a searchable bone list, the paint brush, mirror and copy, the vertex inspector, locks, fixes and the vertex selection) and naming, or, with a component
  selected, its attachment, options (the spine's joint count and **Spline (curve)**
  toggle, a tentacle's joint and curve-control counts), exact guide positions and the names its
  bones will export as. Click a panel's title to fold it away.
* **The rigging window** (Add, or Shift+A) is the library of premade parts, each
  with an illustration: face joints, eyes with a look-at control, a jaw, ears, fingers, toes, a
  tail, a curve-driven tentacle, a ponytail, a prop socket in the right hand, three-segment hind legs
  for quadrupeds, and a custom chain. Its **Templates** category lists saved
  setups (see [Rig templates](#rig-templates)).
* **The export window** (Ctrl+E) picks the programs (and **Export FBX** for
  game engines), the naming convention, the file name and the folder.

Every change rebuilds the bone and control preview at once; Ctrl+Z and Ctrl+Y
undo and redo, Ctrl+B builds, 1/2/3 switch views and F frames the character.
Building computes the skin weights, which you can inspect and paint per bone. The app can
also open a `.hive.json` to keep editing an exported rig, or a `.guides.json` to
apply a saved layout.

```bash
cargo run --release --bin HiveRig               # or: HiveRig character.glb / HiveRig --demo [layout.guides.json]
```

The viewport is drawn with egui shapes on the CPU and the window uses OpenGL
(eframe with the glow backend), so it runs without a dedicated GPU. Build
without the app with `--no-default-features`. Keep `hive-rig.exe` next to
`HiveRig.exe`: the scripts it exports call the command line to rebuild.

Double-clicking `hive-rig.exe` (the command line) still works without the app:
it asks for a character file, rigs it with the default guides and opens the
output folder.

## What you get

| File | Use it in | What it contains |
| --- | --- | --- |
| `<name>.glb` | Blender, Unreal, any glTF tool | Mesh (with its objects, UVs, materials, textures and blendshapes), skeleton and skin weights |
| `<name>.fbx` (with `--targets fbx`) | Unreal, Unity, any FBX tool | The same mesh, skeleton, skin weights, blend shapes and bind pose |
| `<name>_textures/` | Everything above | The character's texture images, when it has any |
| `<name>_blender.py` | Blender 3.6+ | Mesh, armature, weights **and controls** |
| `<name>_maya.py` | Maya 2022+ | Mesh, oriented joints, skinCluster **and controls** |
| `<name>_unreal.py` | Unreal Engine 5.2+ | Imports the .glb and builds a **Control Rig Blueprint** |
| `<name>.guides.json` | hive-rig, Blender, Maya | The editable component layout |
| `<name>.hive.json` | Your own tools | The full rig description |

Controls don't survive generic formats like FBX or glTF, so each application
gets a generated Python script that rebuilds the control rig with native
tools. The skin and skeleton are written as glTF because it's an open format
that Blender and Unreal import natively.

### FBX for game engines

`--targets fbx` (on `rig`, `demo` and `rebuild`, alone or with the scripts:
`--targets blender,maya,unreal,fbx`), or **Export FBX** in the app's export
window, also writes `<name>.fbx`: a binary FBX 7.4 that Unreal and Unity import
directly as a skeletal mesh, with no script. It holds the mesh (a mesh object
per source object, with its normals, UV sets, vertex colours, materials with
their texture files, and blend shapes; see [Mesh fidelity](#mesh-fidelity)), one joint per bone named with the rig's
[naming convention](#naming-conventions) and parented as in the skeleton, the
skin weights with every influence the skin keeps (8 for `--engine unreal`, up to
12; FBX has no limit of four), and a bind pose for the mesh and every bone. It
is a Y-up file in centimetres (Unity and Blender scale it to metres), with the
character standing as it stood in the source (see
[Input requirements](#input-requirements)). Joints
carry translation only, like the `.glb`. FBX has no controls: use the Unreal
script for a Control Rig.

```bash
hive-rig demo -o out --targets fbx --engine unreal   # out/hive_demo.fbx with 8 influences
hive-rig rebuild out/hive_demo.hive.json --targets unreal,fbx
```

The writer is a small hand-written one in `src/export/fbx.rs` (no FBX SDK, no new
crates), from the publicly described binary layout. Arrays are written
uncompressed, so files are larger than ones written with compression.

### FBX input

Every command and the app that take a `.glb` also take a binary `.fbx` (7.x,
from Maya, Blender, 3ds Max, Mixamo or hive-rig itself): `hive-rig rig
hero.fbx -o out`, `check --mesh hero.fbx`, `shape --from sculpt.fbx`, or
dropping it on the app. The reader (`src/import/fbx.rs`, inflating with `miniz_oxide`) is written from the
public binary layout too, with no SDK: it reads 32- and 64-bit records (7.4
and 7.5+), zlib-compressed arrays, every mesh with its normals, UV sets,
vertex colours, a piece per material (materials with their colour, opacity and
diffuse, normal and emissive texture files, or the images embedded in the
file), blend shapes, the bones (LimbNodes and anything a skin cluster links
to) and the skin. Models are placed as the format describes: translation,
rotation in its rotation order, pre- and post-rotation, pivots and offsets,
scaling and the geometric transform; the scene is turned from the file's axes
(`GlobalSettings`) to Y up and scaled from its unit to metres, and a skinned
vertex sits where its clusters put it at the file's pose. Bones become guides
as for a `.glb` ([joints to guides](#bring-your-own-skeleton-joints-to-guides)).
A vertex is split only where its corners' normals, UVs or colours really
differ (not by rounding noise), so an FBX that Blender made from a `.glb`
reads back with the `.glb`'s vertices in the same order. Leaf bones an
exporter adds to mark where a bone ends (Blender's `<bone>_end`) are left out.
ASCII FBX isn't read (save it as binary). A texture is looked for only under
the `.fbx`'s folder: an absolute path in the file keeps just its file name.

### Mesh fidelity

A character comes out looking as it went in, only rigged. hive-rig keeps:

* **Objects and pieces**: each mesh node of a glTF (each `o`/`g` of an OBJ)
  stays its own object, and each primitive (each run of faces with one
  `usemtl`) its own piece with its material. Skinning treats the whole
  character as one surface (copies of a vertex along a UV seam or hard edge
  count as joined), but every output keeps the pieces: a `.glb` mesh node per
  object sharing one skin, an FBX mesh per object, and a Blender object or Maya
  mesh per object.
* **Vertex data**: normals (as given, including hard edges), every UV set,
  tangents, and vertex colours (OBJ `v x y z r g b` too). An OBJ vertex used
  with different UVs or normals is split into copies, as glTF stores it.
* **Materials**: glTF metal-roughness materials (base colour, metallic,
  roughness, emissive, alpha mode and cutoff, double-sided) with their base
  colour, metallic-roughness, normal, occlusion and emissive textures and the
  UV set each one uses; OBJ `.mtl` files (`Kd`, `d`/`Tr`, `Ke`, `Pr`, `Pm`,
  `map_Kd`, `bump`/`norm`, `map_Ke`).
* **Textures**: images embedded in a `.glb` or referenced by a `.gltf` or
  `.mtl` are written once into `<name>_textures/` next to the rig, and the
  `.hive.json` names them there (no image data in the JSON). The `.glb`
  embeds them again; the FBX, Blender and Maya scripts read them from the
  folder.
* **Blendshapes**: glTF morph targets (merged by name across meshes, with
  their normal offsets and default weights) become hive-rig shapes, written
  back as `.glb` morph targets (named in `extras.targetNames`), FBX blend shape
  channels, Blender shape keys, a Maya blendShape and Unreal morph targets.

Not kept yet: texture transforms (`KHR_texture_transform`) and sampler
wrap/filter settings, other material extensions (clearcoat, transmission and
so on), tangent offsets of morph targets, and the source file's own skin and
animations (the character is rigged anew). Blender doesn't wire the occlusion
texture into its material (it has no input for it); the image is kept on the
material.

### Blendshapes and corrective shapes

Shapes live on the rig's mesh (`"shapes"` in the `.hive.json`): each moves
some vertices by an offset, has a weight, and may be **driven** by a pose
reader: the turn of one bone about an axis, from `start` degrees (off) to
`end` degrees (fully on). A **corrective** is a shape sculpted in a pose and
driven by it, such as an elbow that keeps its volume as it bends.

A shape can have **in-betweens**: extra targets at weights between 0 and 1
(say 0.5) that the shape passes through on its way to the full target, so an
elbow corrective can bulge one way half bent and another way fully bent. Each
target is fully on at its own weight and blends linearly with its neighbours.

In the app, after Build, the Properties panel's **Blendshapes** card lists the
shapes (with a search box once there are many) with a weight slider each (a
driven shape shows the weight its bone gives it in the test pose, and a dot
count of its in-betweens). **Corrective from pose** makes a shape for the
test-posed bone, driven by it, off at rest and on at the pose's angle; **New
shape** makes one to sculpt at rest. Click a shape to sculpt it: under **Sculpt
<shape>** pick the **Target** (an in-between or **Full 1.00**), add one with
**+ In-between** (at the shape's weight now, or half way; right-click one to
remove it) and pick a brush:

| Brush | What a drag does |
| --- | --- |
| Grab | The vertices under the brush follow the pointer |
| Smooth | Each vertex moves toward its neighbours' average, evening out bumps |
| Relax | Evens the vertices out along the surface, keeping its shape |
| Inflate | Pushes the surface out along its normals (Ctrl: in) |
| Erase | Takes the target's change back off under the brush |

Sculpting works in the current pose with the picked target fully on, and the
change is worked back through the skin so that, in that pose, the target puts
the vertices where the brush left them. When the test pose doesn't show the
target fully on, the card says so and **Pose there** turns the reader's bone to
the target's angle. The brush shows its radius, its falloff (dots on the
vertices it will move, brighter where it moves them more) and its strength (an
arc round the ring) before you press; `[` and `]` resize it. Each shape's `...`
menu mirrors it onto the other side (its driver moves to the twin bone), splits
it into left and right halves, or removes it; **Combine shown** adds the shapes
with a weight above zero into a new one (in-betweens are dropped). Shape edits
have their own **Undo shape**.

On the command line (each command writes the rig and its outputs again):

```bash
hive-rig shape out/hero.hive.json list
# Sculpt in a pose and drive the shape from it: the forearm bent 90 degrees
hive-rig shape out/hero.hive.json sculpt elbow_fix_l --pose lowerarm_l=90 \
  --at 0.33,1.43,-0.01 --radius 0.08 --offset 0,0.03,0 --drive
hive-rig shape out/hero.hive.json inbetween elbow_fix_l 0.5               # a target half way
hive-rig shape out/hero.hive.json sculpt elbow_fix_l --inbetween 0.5 --pose lowerarm_l=45 \
  --at 0.33,1.43,-0.01 --radius 0.06 --offset 0,0.02,0.01
hive-rig shape out/hero.hive.json inbetween elbow_fix_l 0.5 --remove
hive-rig shape out/hero.hive.json sculpt smile --from sculpted_smile.glb   # from a sculpted copy
hive-rig shape out/hero.hive.json mirror elbow_fix_l                       # elbow_fix_r, driven by lowerarm_r
hive-rig shape out/hero.hive.json split smile --width 0.02                 # smile_l and smile_r
hive-rig shape out/hero.hive.json combine smile_l,brow_up_l --as happy_l
hive-rig shape out/hero.hive.json weight smile 0.5
hive-rig shape out/hero.hive.json drive shoulder_fix --bone upperarm_l --toward up --start 10 --end 80
hive-rig shape out/hero.hive.json undrive shoulder_fix
hive-rig shape out/hero.hive.json rename smile grin
hive-rig shape out/hero.hive.json remove grin
```

`--pose BONE=DEGREES@DIRECTION` turns a bone so its tip swings toward
`up`, `down`, `forward`, `back`, `left`, `right` or `x,y,z`; a limb's middle
bone (forearm, shin) bends the way its IK bends without one. `--from` takes a
mesh with the same vertices, sculpted in that pose (or at rest). A rebuild keeps
the shapes and moves their drivers onto the renamed bones.

How each target carries shapes, in-betweens and drivers:

| Target | Shapes and in-betweens | Driver |
| --- | --- | --- |
| `.glb` | Morph targets with names and default weights; an in-between is a morph target of its own, `<shape>_ib50`, at its share of the weight. hive-rig folds them back into one shape when it reads the file | None (glTF has no drivers) |
| `.fbx` | A blend shape channel per target (`<shape>_ib50` too, since Blender's FBX importer refuses FBX in-betweens) | None (FBX has no drivers) |
| Blender | A shape key per target. A shape with in-betweens gets a `hive_shape_<name>` slider on the armature (in its custom properties and the hive-rig panel's **Skin and Shapes** box) and each key follows it through a hat-shaped simple expression | A driver on the key's value (or on the slider, with in-betweens): the bone's local rotation (quaternion) read with a simple expression, so it works without Python auto-run. Checked in real Blender |
| Maya | A blendShape in front of the skinCluster, with in-betweens as native in-between targets (inputTargetItem 5000 + 1000 x weight) | A node network (multMatrix, decomposeMatrix, vectorProduct) and an expression on the target weight. Only against stubs |
| Unreal | Morph targets from the `.glb` import (in-betweens as `<shape>_ib50`) | A curve per morph target in the Control Rig, set by the shape's pose reader or, for a shape nothing drives, by a `<shape>_shape` float control (at its hive-rig weight) so it can be keyed; with in-betweens the curves blend through the targets in order. The curves drive the morph targets when the Control Rig runs in the mesh's Anim Blueprint. Only against stubs |

#### Send edits back to hive-rig

Weights painted and shapes sculpted in Blender, Maya or Unreal come back into the
`.hive.json`, so they survive the next rebuild. The Blender panel's **Skin and
Shapes** box and the Maya window's **Skin and Shapes** section have **Send to
Hive** buttons (all, weights only, shapes only): they write
`<name>.edits.json` next to the rig (each vertex's weights by bone name, and
each shape key or blendShape target with its in-betweens, in hive-rig's units
and frame) and run:

```bash
hive-rig weights import out/hero.hive.json out/hero.edits.json   # only vertices that changed move
hive-rig shape out/hero.hive.json import out/hero.edits.json     # --only NAME for one shape
```

Imported weights follow the rig's influence limits; a shape keeps its driver
(only an undriven shape takes the slider's weight) and a new shape key comes in
as a new shape. **Rebuild** in either tool sends the edits first and keeps the
edited weights. The mesh must keep hive-rig's vertices and their order.
Blender's round trip is checked in real Blender; Maya's only against stubs.

In Unreal, the **Hive Rig** menu has **Send to Hive** (weights and shapes,
weights only, shapes only; from the Output Log `py "<name>_unreal.py"
send_edits [--weights | --shapes]`) for weights painted on the imported
Skeletal Mesh and morph targets edited there. Unreal keeps its own vertex
order, so the tool exports the mesh as a binary FBX (`<name>_unreal_edits.fbx`,
with its morph targets) and has hive-rig turn it into the same edits file:

```bash
hive-rig edits out/hero.hive.json hero_unreal_edits.fbx      # writes out/hero.edits.json
```

`hive-rig edits` takes any binary FBX exported from the rig's mesh in its rest
pose. It finds the file's frame and units from the bones it shares with the rig
(by name; an affine fit, or a turn and scale for a flat skeleton such as the
snake's), gives each rig vertex the weights and shape offsets of the file's
vertex at its place (within 0.1% of the body height; vertices sharing a place,
as at a seam or where pieces meet, are paired off one to one by skin and normal),
and folds `<shape>_ib50` morph targets back into in-betweens. Weights within
0.005 of the rig's count as unchanged (`--weight-tolerance`), since Unreal keeps
8-bit weights unless the mesh has High Precision Skin Weights on, and shape
offsets within 0.01 mm of the rig's stay exactly the rig's. A rig vertex with
no vertex there keeps its weights and shapes, with a warning. The matching is
checked in `cargo test` (another frame, order, doubled vertices, 8-bit weights,
the horse's shared points) and on an FBX written by real Blender's exporter
from the rig's `.glb` (`tests/blender_fbx_edits_check.py`); the Unreal export
itself (`AssetExportTask` with `SkeletalMeshExporterFBX`) only against stubs.

Pose readers measure a bone's turn about one axis since rest (the twist part of
its rotation about that axis, so turns about other axes change it little). Cone and RBF readers, and helper joints driven
by readers, are not built yet.

### The control rig

* `root_ctrl` (world placement) and `cog_ctrl` (moves the body, feet stay planted)
* FK controls for pelvis, three spine joints, neck, head, clavicles and toes,
  or a spline spine (see [Spline chains](#spline-chains-tentacles-and-a-spline-spine))
* Arms and legs have FK controls, an IK control, a pole-vector control and an
  `ik_fk` blend (0 = FK, 1 = IK). Legs start in IK and arms start in FK.
* `stretch` on every limb's IK control (0 = rigid, 1 = reaches any goal)
* `soft` on every arm and leg IK control (0 to 1, default 0 = a plain solve).
  As the goal nears full reach, the last `soft` × 10% of the reach is eased
  along an exponential, so the knee or elbow opens smoothly instead of popping
  straight; the hand or foot then trails the control slightly. With `stretch`
  the softened chain scales along its length to land on the goal, keeping the
  eased bend. Set the starting value with the component's `soft` option. A limb
  that is nearly straight in the bind pose is eased there too when `soft` > 0.
  Blender (drivers and constraints), Maya (a node network) and Unreal (float
  math ahead of a non-stretching two-bone IK, which scales the bones by the
  stretch) build it the same way.
* Bendy limbs (`bendy: true` on an arm or leg, off by default): each segment's
  skin moves from the segment bone onto four bend joints
  (`upperarm_twist_01_l`…`_04_l`), which follow a curve from the shoulder or hip
  through the elbow or knee to the wrist or ankle. `upperarm_l_bend_ctrl` and
  `lowerarm_l_bend_ctrl` at each segment's middle arc it (cartoony bends) with
  the joints at the ends staying put, and `lowerarm_l_pin_ctrl` moves the elbow
  (or knee) where the two curves meet. The bend joints still take the twist
  shares. Blender and Maya aim each joint along the curve; Unreal is a simpler
  approximation that moves the joints onto the curve without turning them.
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

### Naming conventions

By default bone names follow the Unreal mannequin (`pelvis`, `spine_01`,
`upperarm_l`, `calf_r`, `ball_l`…), so the skeleton retargets cleanly in Unreal,
and controls add `_ctrl` (`upperarm_l_fk_ctrl`). Pick another convention in the
app's export window or with the command line:

| Preset | Joint | Control |
| --- | --- | --- |
| `game` (default) | `upperarm_l`, `spine_01` | `hand_l_ik_ctrl`, `root_ctrl` |
| `prefix` | `L_upperarm_jnt`, `C_spine_01_jnt` | `L_hand_ik_ctrl`, `C_root_ctrl` |
| `suffix` | `upperarm_L_JNT`, `spine_01_M_JNT` | `hand_L_ik_CTRL`, `root_M_CTRL` |
| `unity` | `Hips`, `Spine`, `LeftUpperArm`, `LeftIndexProximal`, others as `game` | as `game` |
| `mixamo` | `mixamorig:Hips`, `mixamorig:LeftArm`, `mixamorig:LeftHandIndex1`, others `mixamorig:` + the `game` name | as `game` |

```bash
hive-rig naming out/hero.guides.json prefix
hive-rig demo -o out --naming unity --targets blender,fbx   # --naming on rig, demo and rebuild too
hive-rig naming out/hero.guides.json --joint "{side}_{name}_{desc}_bnd" --left Lf --right Rt --center Ct
hive-rig rebuild out/hero.hive.json
```

Patterns place `{name}` (the part, such as `upperarm` or `index`), `{side}` and
`{desc}` (`fk`, `ik`, `pole`, `aim`, chain numbers); empty parts and their
underscores drop out. The convention is stored in the guides file, so rebuilds
from Blender, Maya or the command line keep it, and the guides themselves keep
the internal names. Names that would collide are refused.

**Engine presets.** `unity` and `mixamo` carry a skeleton map (`"skeleton":
"unity"` in the naming): the humanoid bones (hips, the first three spine joints,
neck, head, jaw, eyes, clavicles, arms, hands, the three joints of each finger,
legs, feet and the balls of the feet) take that engine's names, and every other
bone (twist and bend joints, extra spine joints, tails, the face) and every
control keeps the game names. With `unity` Unity's Humanoid avatar setup finds
every bone by itself; with `mixamo` animations downloaded from Mixamo play on
the rig by bone name in Blender and Unity (the rig should stand in Mixamo's
T-pose; the demo does). Spine joints past the third, a quad leg's hock and any
extra limbs have no engine slot. Mixamo's colon puts Maya's joints in a
`mixamorig` namespace, which the Maya builder creates; for Unreal keep `game`,
the mannequin's own names.

**Unity avatar mapping.** With **Export FBX** (`--targets fbx`), a biped (legs
and arms with every bone Unity's Humanoid needs) also gets `<name>.ht`, a Unity
Human Template: in the model's Rig tab pick Humanoid, Configure, then Mapping >
Load and choose the file, and every human bone maps to the rig's bone whatever
the naming convention. It is written from the template format Unity saves and
checked in `cargo test`, not yet loaded in Unity. The skeleton maps are checked
in real Blender: the builder, the animator tools and the FBX through Blender's
importer, with `unity` and `mixamo` names (Maya and Unreal only against stubs),
and a rig with either set of names reads back as the same guides.

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
| arm | clavicle, shoulder, elbow, wrist, hand_tip | clavicle, IK/FK arm, stretch, soft IK, spaces, twist bones; bend and pin controls with `bendy` |
| leg | hip, knee, ankle, ball, toe, heel | IK/FK leg, reverse foot, stretch, soft IK, spaces, twist bones; bend and pin controls with `bendy` |
| quad_leg | hip, knee, hock, ankle, ball, toe, heel | three-segment hind leg (`thigh`, `calf`, `hock`, `foot`, `ball`): IK with a hock control, reverse paw roll, FK, stretch, spaces, twist bones; `knee_up` for a spider's leg |
| chain | two or more | an FK chain: fingers, tails, ears, hair, props |
| spline | two or more (`j1`…) | a curve-driven chain (tentacle): curve controls, an FK layer, `stretch` and `volume` |
| eyes | eye_l, eye_r, target | `eye_l`, `eye_r` aimed by a look-at control that follows the head or stays in the world |
| jaw | jaw, chin | `jaw` with a rotate control |
| face | brows (3 a side), upper and lower lids, cheeks, nose, upper and lower lip, lip corners | 17 face joints on the head (plus `--joints` 0 to 3 extra lip joints a side, 1 by default), each with a small control that moves and rotates it; lids pivot on the eye, the lower lip rides on the jaw; a face board beside the head (see below) |
| face_board | none | adds the face board to a face that has none |

The first rig detects the body and places the default biped. From then on you
can change the layout and rebuild:

```bash
hive-rig add out/hero.guides.json fingers           # five fingers on each hand
hive-rig add out/hero.guides.json tail --joints 6
hive-rig add out/hero.guides.json eyes              # look-at eyes on the head
hive-rig add out/hero.guides.json jaw
hive-rig add out/hero.guides.json face              # brow, lid, cheek, nose and lip joints, and the face board
hive-rig add out/hero.guides.json face --joints 2   # two extra lip joints a side
hive-rig add out/hero.guides.json chain --parent head --joints 2 --id ear_l
hive-rig add out/hero.guides.json tentacle --joints 10 --controls 5   # curve-driven, from the pelvis
hive-rig spine out/hero.guides.json --spline true    # the spine follows a curve (hips, mid, chest)
hive-rig remove out/hero.guides.json tail           # also removes anything attached to it
hive-rig components out/hero.guides.json            # list the layout
hive-rig rebuild out/hero.hive.json                 # rebuild from the edited guides
hive-rig rig other_character.glb --guides out/hero.guides.json   # reuse a layout
```

### Spline chains: tentacles and a spline spine

A spline chain's joints follow a smooth curve that a few curve controls bend.
`hive-rig add <guides> tentacle` adds one from the pelvis (`--parent` for any
other bone) with 8 joints (`--joints`) and 4 curve controls (`--controls`, ends
included). The guides `j1`, `j2`… set the rest shape; the joints are spread
evenly along them, one per guide segment unless the `joints` option says
otherwise.

* **Curve controls** (`tentacle_ik_01_ctrl`…) sit along the chain. Each joint
  follows them by a smooth weighting (the end controls own the ends), so moving
  one bends the chain and the joints stay on the curve. The end control also
  turns the last joint.
* **FK layer** (`tentacle_fk_01_ctrl`…): each FK control carries the next curve
  control and the FK controls after it, so turning one swings the rest of the
  chain, and the curve controls stay free to tweak on top.
* **`stretch`** on the end control: 1 (the default) stretches the joints to the
  curve's length; 0 keeps their length so they slide along it.
* **`volume`** on the end control: 0 (the default) to 1, thins stretched joints
  and thickens squashed ones to keep their volume.

`hive-rig spine <guides> --spline true` (or **Spline (curve)** in the spine's
Properties) gives the spine the same rig: `hips_ctrl` drives the pelvis and
starts the curve, `spine_mid_ctrl` bends it and `chest_ctrl` ends it and turns
the top spine joint, with `spine_fk_01_ctrl`… as the FK layer. `--controls`
sets the number of curve controls (3 by default) and `--joints` the spine
joints (at least 2 for a spline spine). The bones keep their names, so the head
and arms stay attached; they follow the top joint, and the IK hands and the
head can take `chest_ctrl` as their chest space. `--spline false` brings the
FK spine back.

How each program builds it:

* **Blender:** a Bézier curve hooked to a bone per curve point, which follows
  the curve controls through an Armature constraint (their weights); a hidden
  copy of the joints rides the curve with **Spline IK** (stretch switches its Y
  scale mode, volume drives its bulge) and the deform joints copy it.
* **Maya:** a curve skinned to a joint under each curve control with the same
  weights, an **ikSplineHandle** with advanced twist from the start and end
  controls, and stretch and volume from the curve's length (`curveInfo`).
* **Unreal:** Control Rig's **Fit Chain on Curve** on the closest four-point
  Bézier (exact up to four controls), once keeping lengths and again stretched
  by `<name>_stretch`. `<name>_volume` thins each joint by 1 / sqrt(stretch)
  across its length. Fit Chain on Curve takes one cubic Bézier, so with more
  than four controls the curve is an approximation.

### Quadrupeds: the dog layout and hind legs

`hive-rig demo --quadruped` rigs a built-in dog (written as `hive_dog`): spine,
neck and head, a six-joint tail, two front legs and two three-segment hind legs.
Its guides are in proportion to the body height, so they move like any other
layout's.

* **Hind legs** (`quad_leg`) run thigh, calf, hock (the long bone below the
  hock joint) and paw, with the toes on the paw. In IK the thigh and calf solve
  as a two-bone chain aimed at the hock, the pole sits straight ahead of the
  knee, and `hock_l_ik_ctrl` at the ankle swings the hock about the paw to set
  the leg's shape. The paw has the biped foot's `roll` (heel, pads, then toe
  tip), and `stretch`, FK controls (thigh, calf, hock, paw) and `ik_fk` work as
  on a leg. Legs start in IK.
* **Spring hock**: `spring` (0 to 1, on the IK control; `--spring` on `demo`,
  **Hock spring** in the app's leg options) makes the hock swing with the thigh
  as the paw moves, the way a horse's stifle and hock flex together, so lifting
  a hoof folds the whole leg; the hock control turns it further. At 0 the hock
  keeps the hock control's angle. Each builder solves a helper two-bone chain
  from the hip to the ankle (its upper bone the thigh plus the hock, its lower
  one the calf) to the foot goal with the leg's pole and turns the hock
  control's parent with its upper bone, blended by `spring`
  (`controls::spring_hock` is the maths): Blender (checked in real Blender), a
  helper joint chain with an ikRPsolver and an orient constraint blended by the
  attribute in Maya, and helper nulls solved by a Two Bone IK unit with a
  weighted Set Rotation in Unreal (`<leg>_spring` float; both only against
  stubs). FK to IK matches the hock and pole in turn until both settle. The
  helper chain doesn't stretch in Maya and Unreal. The app's posing doesn't
  run the spring yet.
* **Front legs** are arm components laid out as a leg: the clavicle is the
  shoulder blade and the elbow points back. An arm whose elbow and wrist sit
  below the belly line (`crotch_y`) skins as a leg, starts in IK and gets an
  upright IK control.
* **More legs a side**: any number of arms, legs and quad legs a side get IK.
  The first arm and the first leg (or quad leg) on each side take the
  mannequin names (`thigh_l`, IK chain `leg_l`); every later one is named after
  its component, so `mid_leg_l` builds `mid_leg_thigh_l`,
  `mid_leg_foot_l_ik_ctrl`, `mid_leg_calf_l_pole_ctrl` and IK chain
  `mid_leg_l`, and its twin `mid_leg_r` mirrors it. A later limb whose id
  would reuse the first one's IK name (a second `leg_l`) is refused with a
  hint. Add a pair with `hive-rig add <guides> quad_legs --id mid_leg --parent
  spine_02`. IK/FK matching, mirror, flip and the pose library in Blender,
  Maya and Unreal work per IK chain, so they cover every leg.
* **Knees that fold up** (`knee_up` on a leg or quad leg): for a spider's or
  insect's leg splayed out to the side, the pole sits above the knee instead
  of in front of it, and the checks expect the knee and hock to bend up.

A four-legged character is recognised when it is opened or rigged: by four
limb ends on the floor under a skeleton with two pairs of legs, or, with no
skeleton, by mirrored pairs of feet and legs spread out front to back. It then
gets the dog layout, with the guides fitted to the mesh (body, head end, tail
and four legs) and, when it has a skeleton, moved onto its joints by their
shape: hips and shoulders where each pair of legs meets, the spine between
them, the neck and head forward of the shoulders and the tail behind the hips.
`rig` and `check` print `Template: dog`; the Checks card says so too, and
suggests the dog template when a biped layout sits on a four-legged body. For a
body it gets wrong, start from the dog's guides and move them onto it (in the
app, or Edit Guides in Blender or Maya):
`hive-rig rig my_dog.glb --guides out/hive_dog.guides.json`.

### More creatures: bird, spider and snake

`hive-rig demo --creature <dog|bird|spider|snake|horse|dinosaur>` rigs a built-in creature
with its own demo body (written as `hive_<creature>`; `--quadruped` is
`--creature dog`). In the app, `HiveRig --demo bird` opens one. Each layout is
also a built-in template (see [Rig templates](#rig-templates)), so it loads onto
any character, scaled to its height. They use the existing components only:

* **Bird** (`components::bird`, 0.5 m): a tilted spine, a long neck and a head
  with the beak along the head bone, a three-bone tail chain, two **wings** and
  two **legs with hocks**. The wings are arm components spread to the sides
  (clavicle, upper wing, forearm and hand to the wing tip, the elbow folded
  back), so they get IK/FK, a pole, stretch and the arm's animator tools. The
  legs are `quad_leg` components, whose forward knee and backward hock are how
  a bird's legs fold; the thighs sit inside the body, so the leg skin starts
  below the belly.
* **Spider** (`components::spider`, 0.4 m to the top of the knees): a spine
  through the cephalothorax, a head with the fangs along the head bone, an
  abdomen chain and **eight IK legs** (`leg1_l` to `leg4_r`), each a
  `quad_leg` with `knee_up`: the coxa and femur rise to the knee, the tibia
  comes down to the hock, the metatarsus is the hock bone and the tarsus the
  paw on the floor, hung from the spine bone it grows from; front legs reach
  forward, back legs back. Each has IK and FK, a pole above its knee, a hock
  control and a foot roll; the first leg a side takes the mannequin names
  (`thigh_l`, IK chain `leg_l`) and the others their own (`leg2_thigh_l`,
  `leg2_l`). Checked in real Blender: IK to FK and FK to IK on all eight legs,
  mirror and flip (Maya and Unreal only against stubs).
* **Snake** (`components::snake`, a 0.5 m size unit; about 2 m long): a
  **spline spine** of ten joints driven by five curve controls, a raised neck
  and head, and a spline (`tentacle`) tail of eight joints and four curve
  controls, all in the mid-plane.
* **Horse** (`components::horse`): long front legs and `quad_leg` hind legs
  that start with a full spring hock (`spring` 1), a long neck, head and tail.
* **Dinosaur** (`components::dinosaur`): a theropod on two `quad_leg` legs,
  short FK arms, a neck, a big head and a long spline tail.

Every creature is skinned the same way, by distance through its body (see
[Skin](#how-it-works)), so a spider's legs or a bird's wings never share
weights across the air between them. Their pre-build checks are clear, as the
dog's are.

Hind legs can also replace a biped's legs:

```bash
hive-rig remove out/hero.guides.json leg_l
hive-rig remove out/hero.guides.json leg_r
hive-rig add out/hero.guides.json quad_legs          # hind_leg_l and hind_leg_r on the pelvis
```

### Face joints in Blender, Maya and Unreal

The face is joints, not blendshapes, the way a Maya face rig is usually built,
so it travels in the .glb like any other bone and every program reads it. Each
face joint gets a small diamond control that moves and rotates it (the Blender
builder uses Copy Transforms, Maya a parent constraint, Unreal a transform
copy), and its skin weights fade out within a few centimetres so a brow never
pulls the eye. The eyelids pivot on the eye joint and the lower lip follows the
jaw when the rig has them (add the jaw before the face).

### The face board, lip zippers and sticky lips

`add face` also puts a **face board** beside the head: a panel (moving it
moves the whole board, never the face) with sixteen sliders that pose the face
from one place. Each slider's value is how far it has moved along its track,
-1 to 1, held to the track:

| Slider | Moves | What it does |
|--------|-------|--------------|
| brow_l, brow_r | up/down, in/out | raises and lowers the brow (the outer end most), pulls it in toward the nose |
| lid_l, lid_r | up/down | closes the upper lid over the eye (down) or opens it wide, the lower lid a little |
| nose, cheek_l, cheek_r | up/down | sneer, cheek raise |
| lip_up, lip_low | up/down | raises or lowers the upper or lower lip, most in the middle |
| corner_l, corner_r | up/down, in/out | smile or frown, wide or narrow; the lip joints next to the corner follow, less |
| mouth | up/down, left/right | moves the whole mouth |
| jaw | down (open), up, left/right | turns the jaw |
| zip_l, zip_r | one way | lip zipper: from that corner, the upper and lower lip joints close together onto the line between them, pair by pair, until the whole mouth is shut |
| sticky | one way | sticky lips: as the jaw opens the middle of the mouth stays shut and lets go as it opens wider, the sides first |

The board moves each face control through an offset between the control and
its parent, so the face controls still work on top of it, and its sliders
drive shapes too: `hive-rig shape <rig> board <shape> <slider> <x+|x-|y+|y->
[--weight W]` links one, and `shape board --auto` links shapes by their names
(a `blink_l` follows `lid_l` going down, `jawOpen` the jaw, `smile_r`
`corner_r` going up, and so on; `--off` unlinks). In the app the face card has
a Face board switch and the lip joint count, and picking the board or a slider
shows the whole board as pads in Properties to drag (double-click puts one
back).

Every builder makes the same board: Blender (drivers on helper bones between
each face control and its parent, Limit Location tracks, Copy Location toward
the sealed place for the lip seal, all checked in real Blender against
hive-rig's numbers), Maya (one expression driving a group under each face
control, transformLimits on the sliders, a two-target point constraint per
lip joint for the seal) and Unreal (a null under each face control set by the
Control Rig graph, and the seal after the face bones; both only against
stubs).

### Bring your own skeleton (joints to guides)

Open or rig a .glb that already has a skeleton and its joints become the
guides: Unreal mannequin, Mixamo (`mixamorig:LeftArm`), 3ds Max biped
(`Bip01 L UpperArm`), and side-first or side-last Maya names (`L_arm_jnt`,
`arm_L_JNT`, `thigh.L`) are recognised. Eyes, a jaw, face joints and fingers
are added when the skeleton has them; twist and helper joints are listed as not
used. Guides with no matching joint stay where body detection put them. Pass
`--ignore-joints` to place the guides on the mesh instead.

Names are checked against the skeleton's shape, and the shape fills in what the
names don't say, so unnamed or numbered joints (`leg_joint_L_2`, `Bone.012`)
work too. The limbs are the chains ending at the floor on each side, the pelvis
is where the two legs meet, the arms are the chains reaching furthest out on
each side above them (followed down to the hands when the arms hang), the chest
is where the two arms meet and the head chain runs up from it. A joint's side
comes from where it is, not from its name. Along a limb, the wrist is where the
fingers branch, the ankle where the chain turns along the floor, and the elbow
and knee the joints about halfway. A name is used only when its joint lies in
the limb the shape found; `rig` says how many guides came from the shape. Feet
hung from the root rather than the legs (as IK rigs do) are joined to the leg
ending just above them. Joints are read at the file's rest pose, where a glTF
viewer shows the character. A binary `.fbx` works the same way.

Parts the layout has no guide for come back as **chains**: a branch of the
skeleton whose joints no guide took (ears, a tail on a biped, hair, antennae,
a cape) becomes a `chain` component through its joints, hung from the rig
bone at the joint it grows from (`Ear.L` on the head gives `ear_l` on `head`;
`hair_01` to `hair_03` give `hair`). Its side comes from where it is. A part
that branches becomes a chain along its longest run, with chains for its side
branches hung from it. Without an end joint, the last bone is carried on as
long again. IK, twist, roll, pole and socket helpers and lone end joints are
left out. The dog layout does the same for parts besides its tail. `rig`
lists the chains it added with the other added components.

### Parenting, test poses and control looks

In the guides stage each guide is drawn as a yellow joint control, and each
component has red corner brackets around its first guide. Click or drag a
guide's centre to move it; grab the brackets to move the whole component as
one unit, with everything hanging from it (and its mirror twin, with symmetry
on). Green plus markers show each arm and leg's IK pole, straight ahead of the
knee and behind the elbow.

To parent several parts at once, Shift-click them (in the view or the
outliner), picking the parent last, then press **P** or right-click and choose
**Parent**. The children hang from the parent's bone nearest the guide you
picked it by. Right-clicking another part's guide offers **Parent … here**
instead, and **Unparent** (Shift+P) hangs the picks from the root again. With
several parts picked, Properties lists them as child and parent with the same
buttons. Parenting keeps the layout buildable: parts are reordered when needed,
and hanging a part from its own child is refused.

In the build stages, click a joint or a control to pick it. Its panel in
Properties shows the name and lets you:

* **Test this bone** turns it, with everything hanging from it and the skin
  that follows it, so you can check the rig before it reaches a DCC. Nothing is
  saved.
* **Control shape, size and colour** change how one control looks. The look is
  saved in the guides file and survives a rebuild. Shapes are picked from a
  grid of drawn tiles (hover one for its name); the right-click menu on a
  control has the same grid. Next to the colour picker are swatches for the
  centre, left, right and root colours and a few more. With **Symmetry** on, a
  change reaches the control's twin on the other side too: the same shape and
  size, and the other side's colour when you pick a side colour (left blue on
  one hand is right red on the other); any other colour is the same on both.
  **Same shape and colour for its component** gives every control of the
  component (and, with symmetry, its twin component) this control's shape and
  colour.

### Control shapes

| Shape | `guides.json` name | Unreal library shape |
| --- | --- | --- |
| Circle | `circle` | `Circle_Thick` |
| Square | `square` | `Square_Thick` |
| Cube | `cube` | `Box_Thick` |
| Diamond (three loops) | `diamond` | `Diamond_Thick` |
| Cross | `cross` | `Arrow4_Thick` |
| Sphere (three rings) | `sphere` | `Sphere_Thick` |
| Arrow | `arrow` | `Arrow_Thick` |
| Double arrow | `double_arrow` | `Arrow2_Thick` |
| Four arrows (move cross) | `four_arrows` | `Arrow4_Thick` |
| Hexagon | `hexagon` | `Hexagon_Thick` |
| Octagon | `octagon` | `Octagon_Thick` |
| Triangle | `triangle` | `Triangle_Thick` |
| Half circle | `half_circle` | `HalfCircle_Thick` |
| Pyramid | `pyramid` | `Pyramid_Thick` |
| Four-pointed star | `star` | `Star4_Thick` |
| Rounded square | `rounded_square` | `RoundedSquare_Thick` |
| Rounded triangle | `rounded_triangle` | `RoundedTriangle_Thick` |
| Quarter circle | `quarter_circle` | `QuarterCircle_Thick` |
| Gear (eight teeth) | `gear` | `Octagon_Thick` (look-alike) |
| Curved arrow (three-quarter ring) | `curved_arrow` | `HalfCircle_Thick` (look-alike) |
| Lollipop (ring on a stick) | `lollipop` | `Circle_Thick` (look-alike) |
| Cylinder | `cylinder` | `Circle_Thick` (look-alike) |
| Plus (wide arms) | `plus` | `Arrow4_Thick` (look-alike) |

Set one by hand in `guides.json` with
`"control_styles": { "head_ctrl": { "shape": "sphere", "size": 0.12 } }`,
or from the command line with `hive-rig control`:

```bash
hive-rig control out/hero.guides.json                       # every control: shape, size, colour
hive-rig control out/hero.guides.json hand_l_ik_ctrl --shape gear --scale 1.3 --twin
hive-rig control out/hero.guides.json upperarm_l_fk_ctrl --component --twin --color '#ff8c1a'
hive-rig control out/hero.guides.json head_ctrl --shape lollipop --size 0.16 --color cyan
hive-rig control out/hero.guides.json hand_l_ik_ctrl --twin --reset
hive-rig rebuild out/hero.hive.json                         # the exports get the new looks
```

Controls are named by their internal or exported name. `--size` is in metres,
`--scale` multiplies the current size, and `--color` takes `r,g,b` (0 to 1),
`#rrggbb` or a name (left, right, centre, root, green, cyan, orange, white).
`--twin` and `--component` reach the twin and the component's controls the
way symmetry and the component button do in the app.
The shapes are defined once, in `src/controls.rs`. Flat shapes lie in the
plane facing the control's axis, and arrows and triangles point up, or forward
for a control that faces up or down. Each control's lines are written into the
rig as `outline` (model space, at rest), and the app, Blender (as the bone's
custom shape) and Maya (as one curve per line) all draw exactly those lines.
Unreal draws the look-alike from Control Rig's default shape library in the
table above. If the library lacks that name, Unreal uses the circle. Those
names and their orientation haven't been checked in Unreal yet.

From the right-click menu, **Import components…** brings the premade parts of
another guides file in, hung from the bone under the pointer. The CLI has the
same: `hive-rig import <guides> <other.guides.json> --parent ball_l` and
`hive-rig attach <guides> <id> <bone>`. Components keep their ids, so an id the
rig already uses is refused rather than renamed (a parts template renames instead).

Toes (`hive-rig add <guides> toes`) are five chains from each foot's ball.

### Rig templates

A template is a studio's own setup saved to reuse on other characters: a set of
components with their options and control looks, in `<name>.template.json`.
Guide positions are stored in body heights from the root guide, so a template
made on one character fits another of any size.

* A **layout** template holds every component. Loading it replaces the
  character's guides, scaled to its body, along with the control looks (and the
  naming convention, when the template has its own).
* A **parts** template holds premade parts only (fingers, toes, tails, chains,
  eyes, a jaw, the face). Loading it adds them like an import, except that an id
  the rig already uses gets a number (`index_l` becomes `index2_l`, `tail`
  becomes `tail2`) and the bones and control looks follow the new name. Parts
  that hang from a bone outside the template keep that bone, or hang from the one
  you choose; parts hanging from another part of the template stay on it.

In the app, **File > Save as template…** saves the whole layout and **Save
picked as template…** saves the parts picked with Shift. The rigging window's
**Templates** category shows a card for each template, drawn from its guides;
click one to load it (one undo step). **Load template…** loads a file, and the
viewport's right-click **Load template here…** hangs a parts template from the
bone under the pointer.

Templates are listed from two folders, plus six built in: `biped`,
`biped_face_fingers` (with eyes, a jaw, face joints and fingers), and the
creatures `dog`, `bird`, `spider` and `snake`. A card for a layout much longer
than it is wide or tall (the snake) draws it from the side.

* `templates/` next to the program, for a studio to ship its own;
* the user folder, where new templates are saved: `%APPDATA%\HiveRig\templates`
  on Windows, `$XDG_CONFIG_HOME/hive-rig/templates` or `~/.config/hive-rig/templates`
  elsewhere.

```bash
hive-rig template save out/hero.guides.json studio_biped.template.json
hive-rig template save out/hero.guides.json hands.template.json --only thumb_l,index_l
hive-rig template apply other.guides.json biped_face_fingers     # by name or file
hive-rig template apply other.guides.json hands.template.json --parent head
hive-rig template list
```

### Checks before building

Every change to the guides is checked for the mistakes that build a rig that
works, just not the way you meant. They're warnings: Build still runs.

| Check | Warns when | Fix offered |
| --- | --- | --- |
| Wrong side | a left or right guide is more than 0.5% of the body height across the mid-line | mirror the twin's guide, or flip it back across |
| Bends the wrong way | an elbow (or a front leg's elbow, or a hind leg's hock) bends forward, or a knee bends backward, so IK would flip it | mirror the twin's joint, or bend it the right way |
| Straight limb | the elbow, knee or hock is within 0.5% of the limb's length of the straight line, so IK can't tell which way to fold | mirror the twin's joint, or bend it 4% the right way |
| Outside the mesh | a joint that belongs inside the body (pelvis, chest, neck, head, clavicle to wrist, hip to ankle, hock, jaw) is more than 3% of the body height outside the mesh | mirror the twin's guide (otherwise drag it in) |
| Joints too close | two joints of a bone are within 0.2% of the body height (on top of each other is also the build error) | mirror the twin's guide, or push the joint on along the bone |
| Not symmetric | a right component (or the right half of the eyes or face) is more than 2% of the body height from the mirror image of the left | mirror the left side onto the right |
| Off centre | a spine, head, jaw, root or centre face guide is more than 2% of the body height off the mid-line | centre it |
| Odd parent | a component hangs from an unusual bone: a leg under the head, an arm under a leg, eyes on the chest, a left part on a right limb (chains such as tails and props may hang anywhere) | re-attach it to the usual bone (pelvis, top spine joint, head, or the same bone on its own side) |

Each guide gets at most one warning: wrong side, off centre, too close, bend and
outside are checked in that order, and the symmetry check skips guides that
already have one. The one error is a layout that can't build at all.

In the app, the **Checks** card in Properties (with nothing selected) lists the
warnings. Click one to select its guide; **Fix** buttons apply the offered fix
and **Fix all** applies every one, each as one undo step. Guides with a warning
carry an orange triangle in the viewport, and hovering one shows the problem. A
selected component lists its own warnings at the top of its panel. Build says
how many warnings there are.

```bash
hive-rig check out/hero.guides.json --mesh hero.glb   # or: hive-rig check out/hero.hive.json
hive-rig check out/hero.guides.json --fix             # apply the offered fixes and save
```

`check` prints each warning with what to do, and exits non-zero only when the
guides can't build. It also prints the character's orientation (from the rig,
or detected from `--mesh`) and the layout the guides use, with a warning when
the body looks like the other template (a biped layout on a four-legged body).
The Checks card shows the same two lines, with the orientation control (see
[Input requirements](#input-requirements)). Given a `.hive.json` it uses the rig's mesh and the guides
next to it (the ones `rebuild` reads). `rig`, `rebuild` and `demo` print the
same warnings before building.

**The built skin.** After Build (and after painting, copying or pruning
weights) two more checks look at the weights themselves:

| Check | Warns when | Fix offered |
| --- | --- | --- |
| No weight | a vertex's weights add up to less than 0.001, so it stays behind when the rig moves | fill it from the weights around it on the surface |
| Bone without skin | a deforming bone has no weight above 0.001 on any vertex, so turning it moves nothing | re-run that bone's automatic weights, or paint it |

In the app they're listed under **Built skin** in the Checks card, with
**Select** (select the bare vertices) and **Fill** for the first, **Show** (pick
the bone under Weights to paint it) and **Auto** for the second, each one undo
step. On the command line `check` reports them for a `.hive.json`, `rig`,
`rebuild` and the `weights` commands print them after writing, and:

```bash
hive-rig check out/hero.hive.json --fix          # also fills bare vertices and writes the rig again
hive-rig weights fill out/hero.hive.json         # only the fill
hive-rig weights auto out/hero.hive.json --bone hand_l   # skin for a bone that has none
```

Filling works in from the weighted skin one ring of vertices at a time, each
taking the average of its weighted neighbours, so a bare patch takes the
weights round its edge. A loose piece with no weight anywhere stays bare.

### Versions and compatibility

`guides.json` and `.hive.json` record the hive-rig that last wrote them in
`hive_rig` (`"hive_rig": "0.1.0"`); every write stamps the running version. The
guides `version` is what keeps a layout building the same way: it pins the
builder's rules that the file doesn't spell out, and saving a file again keeps
its version. Guides version 1 pins:

* 3 spine joints when the spine doesn't set `joints`;
* a foot and paw roll that moves from the ball to the toe tip at 30°;
* IK poles half the limb's length straight ahead of the knee (behind the
  elbow), at least 10% of the body height away;
* arms whose elbow and wrist are below the belly line (`crotch_y`) built as
  front legs: leg skin, IK by default, an upright IK control.

A release that changes one of these adds a guides version for the new
behaviour, and older files keep theirs. A file from a newer hive-rig still
opens: settings this one doesn't know are ignored, an unknown guides version
builds with the newest rules this one has, and the app, the command line and
`check` warn about it. Files from before the stamp (no `hive_rig`) are version 1.

### Skin weights in the app

After Build, the Properties panel's skin weights section starts with the
**mesh tool**: **View** (drags orbit), **Paint**, **Select** or **Sculpt** (the
picked shape). One is on at a time, and a line over the viewport says which
and what a drag does. Below it, the **bone list** shows every deform bone with
a search box: click a bone to show its weights as a heat map (click again to
hide them), or switch **Bone** to **All** to colour the mesh by every bone at
once, each in the colour of its swatch in the list. Each row has a lock, and a
bar with the weight of the vertex being inspected. **Mirror L to R** and
**Mirror R to L** copy one side's weights onto the other by mirrored vertex
position, **Smooth** blurs the shown bone's weights (or every bone's) with
their neighbours, and **Copy from rig...** takes the weights of another
`.hive.json` (this character built before, or another with a similar skeleton)
by bone name, as `hive-rig weights copy` does.

* **Paint**: pick a bone in the list, choose the **Paint** tool and drag on the
  mesh. Before you press, the brush shows its radius, the vertices it would
  change (dots, brighter where the falloff and strength change them more) and
  its strength as an arc round the ring. **Add** raises the bone's weight (the vertex's other bones give way),
  **Subtract** lowers it (the other bones take it up; a vertex held by that bone
  alone passes to its parent), **Replace** moves it towards the brush's
  **Value**, **Scale** multiplies it by the **Factor** (0.5 halves it at full
  strength), **Smooth** blends every bone towards the neighbours, and **Mend**
  fixes spikes: only a vertex whose weights stand out from its neighbours' (by
  more than 0.5, see [docs/skinning.md](docs/skinning.md#editing-weights-by-hand))
  takes their average, so smooth skin around it is left as it is. The brush fades
  to nothing at its rim; set its radius (or press `[` and `]`) and strength. With
  Symmetry on, the twin bone is painted on the other side. Each stroke, mirror
  and smooth is one Undo step. Dragging off the mesh still orbits.
* **Bend** (under the brush): turns the painted bone at its joint, 0 to 90
  degrees about X, Y or Z (the axis across the bone by default), through the
  test pose, so you watch the skin deform while you paint. Painting lands on the
  bent skin. Nothing is saved; 0 straightens it.
* **Lock** a bone (its padlock in the list, or **Lock** for the shown bone): no brush mode,
  smooth, flood, paste, fix or automatic-weights rerun changes its weights on
  any vertex; the vertex's unlocked bones share whatever the locked ones leave.
  A locked bone can't be painted itself. Mirror and weight copy still copy every
  bone.
* **Normalize** makes every vertex add up to 1 within the influence limits
  (locked weights stay); **Fix spikes** gives every vertex standing out from
  its neighbours by more than **Spike above** their average weights;
  **Auto weights** re-runs the automatic weights (the voxel method above) on the
  shown bone's skin, wherever it has weight now or would get it, and keeps your
  edits everywhere else.
* **Select** vertices: with **Select** on, click a vertex on the mesh or drag a
  box (Shift adds, Ctrl removes, Alt-drag orbits). A box takes only the vertices
  in front, or everything under it with X-ray on. Copies of a vertex split at a
  UV seam are selected together, so edits never tear a seam. **Grow** and
  **Shrink** add or take off a ring of neighbours, **Its skin** selects
  everything the shown bone moves. For the selection: **Copy** (one vertex's
  weights, or the average of several) and **Paste** (Ctrl+C, Ctrl+V), **Flood**
  (set the shown bone to a value on all of them) and **Auto** (the automatic
  weights again on just those vertices).
* **Weight table**: with vertices selected, a panel at the bottom left lists
  each one's influences (the first 12 vertices). Drag or type a weight; the
  vertex's other unlocked bones share the rest. A whole drag or a typed value is
  one Undo step.
* **Hover** over the mesh with a bone shown (or Select on) to see the vertex
  under the pointer and each influence on it. The **Vertex** inspector in
  Properties keeps the last vertex hovered (or the first selected): its
  influences, how many of the limit it uses and their sum, each weight
  editable as in the weight table.
* **Max influences and prune**: the most bones per vertex, 1 to 12, with the
  engines named: Unreal 8 (12 with its extra-influences setting), Unity and
  glTF 4. Past four, the `.glb` writes a second `JOINTS_1`/`WEIGHTS_1` set (a
  third for more than 8). **Prune below** drops influences smaller than the
  threshold and shares their weight among the rest. Both are stored in the
  guides (`"skin": {"max_influences": 8, "prune": 0.01}`), so rebuilds and
  layout templates keep them. Changing them needs a Build.
* **Keep edited weights on rebuild** (on by default): when the rig is built again
  after a guide edit, every vertex whose weights you changed (painted, mirrored,
  smoothed or copied) keeps them, as Blender's and Maya's Rebuild do, unless the
  new rig gives the vertex to a bone the old one didn't have. Vertices you
  didn't touch take the new guides' automatic weights.

On the command line:

```bash
hive-rig demo -o out --engine unreal               # 8 influences (unreal12, unity, gltf)
hive-rig rig hero.glb -o out --max-influences 6 --prune 0.01
hive-rig weights limit out/hero.hive.json --engine unity --prune 0.02
hive-rig weights copy base/base.hive.json hero/hero.hive.json   # copy weights onto another rig
hive-rig weights normalize out/hero.hive.json      # every vertex adds up to 1
hive-rig weights fix-spikes out/hero.hive.json --threshold 0.6 --lock head
hive-rig weights auto out/hero.hive.json --bone upperarm_l --bone lowerarm_l   # automatic weights again there
hive-rig rebuild out/hero.hive.json --keep-weights  # keep hand-edited weights
```

`normalize`, `fix-spikes` and `auto` take `--lock BONE` (repeatable) for bones
whose weights stay, and `-o` and `--targets` like the other commands.

`weights copy` moves one character's weights onto another with the same or a
similar skeleton, in any pose or size (an A-pose character from a T-pose one,
say). Bones are matched by their hive-rig name whatever the naming convention,
then by name after side tokens, prefixes and suffixes are read (`L_arm_jnt`,
`upperarm_l`). Each target vertex is carried into the source's pose by its own
bones (each matched bone turned onto its source bone), then takes the weights
at the closest point of the source surface, blended across that triangle.
Extra chains left over on both sides (a tail called `brush` on one and `tail` on
the other, ears with two joints on one and three on the other) are then
matched **by position**: each leftover target chain, carried into the source's
pose with the bones it hangs from, pairs with the leftover source chain lying
closest along it (within 5% of the height on average), and along the pair
every bone takes the weight of the bones nearest it, so different joint counts
share the weight out by place. `weights copy` lists the bones it matched by
position. Source bones with no match hand their weight to the nearest matched
parent. The target is written again in its own folder (or `-o`).

`guides.json` is plain JSON, so you can also edit guide positions, the spine's
`joints` option, or `twist: false`, `bendy: true` or `soft` on a limb by hand.
In the app, a selected arm or leg has **Bendy** and **Soft IK** in its options.

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

### Animator tools in Blender, Maya and Unreal

Below the guide tools, the Hive tab (Blender) and the Hive Rig window (Maya)
have an **Animation** section and a **Pose Library**:

* **IK to FK / FK to IK** for each arm and leg. IK to FK turns the FK controls
  to the pose the IK limb has now; FK to IK moves the IK control onto the FK
  hand or foot and puts the pole in front of the elbow or knee, in the plane of
  the limb. Either way the `ik_fk` blend flips afterwards and the limb doesn't
  move. A rolled foot keeps its roll. On a three-segment hind leg the hock
  is matched too: IK to FK sets the hock's FK control, and FK to IK turns the
  IK hock control to the FK hock. FK has no stretch, so a stretched IK limb
  can't be matched exactly in FK. With `soft` on, FK to IK sets the IK control
  out past the FK hand or foot by as much as the ease pulls it back.
* **Mirror Pose** copies the selected controls' pose onto their twins on the
  other side, or every left (right) control's pose when nothing is selected.
  Poses are mirrored relative to each control's parent, along with `ik_fk`,
  `stretch`, `soft`, `roll` and `space`. Centre controls are left alone.
* **Flip Pose** turns the pose into its mirror image: left and right twins swap
  poses (each mirrored) and every centre control is mirrored onto itself, so a
  character leaning and looking left leans and looks right. With controls
  selected, only those and their twins flip. Flipping twice gives the pose back.
* **Studio pose**: a pose posed in the app's Rig stage is saved with the rig
  (`pose` in the `.hive.json`, written on Export and opened again with the rig;
  `rebuild` keeps it) and comes into the Blender and Maya pose libraries as
  **Studio pose**, the rig itself built at rest. The builders get it as where
  the pose moves each control in model space (hive-rig's evaluator,
  `pose::evaluate`) and its settings, since each program's controls have their
  own axes. `hive-rig pose <rig> --set CONTROL=tx,ty,tz,rx,ry,rz --attr
  CONTROL.ik_fk=0 [--reset] [--bones posed.json]` poses a rig from the command
  line the same way. Checked in real Blender (applied, every deforming bone
  lands within 1 mm of hive-rig's pose); Maya only against stubs; not in Unreal
  yet.
* **Pose Library**: **Save Pose** keeps the current pose of the selected
  controls (or every control) under a name, in the scene file (on the armature
  in Blender, on the rig's top node in Maya). Each saved pose has **Apply**
  (only the selected controls, if any are selected) and **Apply Mirrored**,
  which puts each control's saved pose mirrored onto its twin and centre
  controls mirrored onto themselves. **Delete** takes a pose out. Poses hold
  the controls' local transforms and their settings (`ik_fk`, `roll` and so on),
  so they work at any place in the scene.
* **Reset** puts every control, or only the selected ones, back to the bind
  pose and default settings. **Select All Controls** selects the controls only.
* **Switch space without a pop**: Blender shows a button per space for the
  active control; Maya asks which space to follow for the selected control. The
  control keeps its place in the world.
* **Match the whole playback range** (a switch above the IK/FK buttons): the
  match runs on every frame of the scene's playback range in order, keying each
  frame whatever auto key says, so a stretch of animation changes between IK
  and FK without a pop. Each frame reads the keys the frames before it were
  given.

With auto key on, the tools key what they changed. Which controls make up each
limb and which controls are left/right twins come from hive-rig (`ik_chains` and
`mirror_pairs` in the rig data), so the tools work with any naming convention.

In Unreal, IK to FK and FK to IK work on the Level Sequence that animates the
Control Rig: open it in Sequencer and use the **Hive Rig** menu that the script
adds to the editor's menu bar, with an entry per limb for the current frame and
for the playback range. Each frame is loaded from the Sequencer's keys into the
rig, matched and keyed back. From the Output Log the same is
`py "<name>_unreal.py" ik_to_fk arm_l` (add `--range` for the playback range),
and `py "<name>_unreal.py" menu` brings the menu back after an editor restart.
A second menu, **Hive Poses**, has **Flip pose** (this frame, or the playback
range) and the pose library: **Save pose** keeps the current frame's pose of
the selected controls (or every control and setting) as "Pose 1", "Pose 2"...
in `<name>.poses.json` next to the script (rename them there), and each saved
pose has **Apply** and **Apply mirrored**, keyed on the current frame. From the
Output Log: `py "<name>_unreal.py" flip_pose [--range]`,
`py "<name>_unreal.py" save_pose Arms up`,
`py "<name>_unreal.py" apply_pose Arms up [--mirrored] [--range]` and
`delete_pose`. Unreal flips in world space, across the plane halfway between
the left and right controls at rest (so it needs a rig with twins; a snake
can't be flipped there), which for a whole pose is the same mirror image as
Blender and Maya give. The same menu has **Mirror pose** (left to right or
right to left, this frame or the playback range): the selected controls' poses
(or every left, or right, control's) go mirrored onto their twins with their
limb's settings, centre controls staying put; and **Reset pose** (this frame or
the range): the selected controls (with a selected IK control's limb settings),
or every control and setting, back to their initial values, keyed. From the
Output Log: `py "<name>_unreal.py" mirror_pose [--right] [--range]` and
`reset_pose [--range]`. The space switch is Blender and Maya only; Control
Rig's own space switching covers it.

## Input requirements

* `.obj`, `.gltf`, `.glb` or binary `.fbx`
* One humanoid standing on the ground in T-pose or A-pose, or a four-legged
  animal standing on all fours
* Any up axis and facing (see below)
* Arms clear of the body and legs apart

If the legs, arms or neck can't be found from the geometry, the rigger falls
back to average human proportions and prints a warning.

**Orientation.** hive-rig works in one frame: Y up, facing +Z, with the
character's left on +X (the glTF convention). A character that stands
otherwise (Z-up, facing -Z or +X, lying on its back) is turned into that frame
when it is read. The turn is one of the 24 that swap and flip axes, chosen from
the mesh and, when there is one, the skeleton: the axis the body is mirrored
across is left and right; up is the axis with feet (or four feet) on the floor
at one end, legs above them and a head at the top, and the skeleton's legs
hanging below their hips; the facing is the side the toes point to (soles
against the shins, toe joints ahead of their ankles), or a four-legged body's
head end. When nothing stands out (a sphere, say) the file's own axes are kept
and the result says it is unsure. A skinned glTF is read where its skeleton
puts it at rest, as a glTF viewer shows it: the mesh node's own transform,
which glTF ignores for skinned meshes, isn't applied.

```bash
hive-rig rig zup_hero.glb -o out                      # prints: Orientation: up +Z, facing -Y (detected from ...)
hive-rig rig hero.glb -o out --up +z --forward -y     # set either or both; the rest is detected
hive-rig check hero.guides.json --mesh hero.glb --up +y
```

`--up` and `--forward` take `+x`, `-x`, `+y`, `-y`, `+z` or `-z` in the
source file's axes. In the app, the **Checks** card shows the orientation with
**Up** and **Facing** menus that turn the character at once (the guides are
placed again), and **Detect** to go back to the detected one.

The rig records the orientation (`orientation` in `.hive.json`; `rebuild` keeps
it). The `.glb` and `.fbx` turn the character back, so it stands where it stood
in the source and drops into the same scene. The Blender and Maya scripts build
it upright, Y-up facing +Z in glTF terms (Blender's Z up, facing -Y; Maya's Y
up, facing +Z), so their controls face the usual way. The Unreal script follows
the `.glb` it imports. Guides files are always in hive-rig's frame.

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
3. **Skin** (`src/skin.rs`, `src/voxel.rs`): the mesh is voxelized into a
   solid (holes capped, the inside flood-filled), and each bone's distance to
   each vertex is measured through that solid (geodesic voxel binding), so
   weights can't jump across the air between legs, an arm and the ribs, or a
   spider's legs, and loose pieces (eyes, teeth, clothes) follow the bones
   inside or nearest them. Weights fall off with that distance, are limited to
   the rig's max influences (4 unless set), are smoothed over the surface
   (across UV seams), and influences below the prune threshold (if any) are
   dropped. [docs/skinning.md](docs/skinning.md) explains automatic weights,
   smoothing, influence limits, twist and corrective shapes, and measurements
   on real characters. About a second for a 30,000-vertex character.
4. **Controls** (`src/controls.rs`): the DCC-neutral description of every control
   (its parent, shape, colour and driven bone) and the IK chains, as built by the components.
5. **Export** (`src/export/`): the glTF and FBX writers and the three builder templates
   in `templates/`, with the rig data embedded so each script is self-contained.

## Testing

```bash
cargo test                       # landmarks, skeleton, skin, controls, glTF and FBX read back
python3 tests/dcc_stub_run.py maya out/hive_demo_maya.py      # Python-level check
python3 tests/dcc_stub_run.py unreal out/hive_demo_unreal.py  # Python-level check
pip install bpy && python tests/blender_check.py out/hive_demo_blender.py out/hive_demo.glb
python -I tests/blender_skin_eval.py out/hive_demo.hive.json            # skin weight quality
sh tests/fetch_skin_models.sh /tmp/models CesiumMan    # real models (pinned), then:
hive-rig rig /tmp/models/CesiumMan/CesiumMan.glb -o /tmp/cm
python -I tests/blender_skin_eval.py /tmp/cm/CesiumMan.hive.json --source /tmp/models/CesiumMan/CesiumMan.glb
python -I tests/blender_mesh_check.py out/hero_blender.py --source hero.glb --fbx out/hero.fbx   # after rig hero.glb -o out --targets blender,fbx
python -I tests/blender_mesh_check.py out/hive_demo_blender.py --corrective elbow_fix_l lowerarm_l_fk_ctrl
python -I tests/blender_roundtrip_check.py out/hive_demo_blender.py --corrective elbow_fix_r lowerarm_r_fk_ctrl --slider brow_up
python3 tests/dcc_stub_run.py maya out/hive_demo_maya.py --send         # Maya Send to Hive
python3 tests/dcc_stub_run.py unreal out/hive_demo_unreal.py --send     # Unreal Send to Hive (after --targets unreal,fbx)
python -I tests/blender_fbx_edits_check.py out/hive_demo.hive.json out/hive_demo.glb target/debug/hive-rig /tmp/work
hive-rig pose out/hive_demo.hive.json --set head_ctrl=,,,0,-30,5 --bones out/posed.json
python -I tests/blender_studio_pose_check.py out/hive_demo_blender.py out/posed.json   # the studio pose in Blender
# A face board: add eyes, jaw and face to the demo's guides, rebuild, then the
# Blender check above checks the sliders, zippers and sticky lips against hive-rig.
python tests/blender_guides_check.py out/hive_demo_blender.py           # guides and rebuild
python tests/blender_quad_check.py dog/hive_dog_blender.py dog/hive_dog.glb   # after demo --quadruped -o dog
python tests/blender_spline_check.py out/hive_demo_blender.py out/hive_demo.glb   # after add tentacle, spine --spline true, rebuild
python tests/blender_bendy_check.py bendy/hive_demo_blender.py   # after demo --bendy --soft 0.5 -o bendy
python3 tests/dcc_stub_run.py maya out/hive_demo_maya.py --guides       # Maya guide tools
python tests/blender_anim_check.py out/hive_demo_blender.py             # animator tools
python tests/blender_fbx_check.py out/hive_demo.fbx out/hive_demo.hive.json   # after demo --targets fbx -o out
python tests/blender_anim_check.py bird/hive_bird_blender.py   # after demo --creature bird -o bird (or spider, snake)
python -I tests/blender_glb_to_fbx.py /tmp/models/Fox/Fox.glb /tmp/models/Fox/Fox.fbx   # an FBX made by Blender
hive-rig rig /tmp/models/Fox/Fox.fbx -o /tmp/fox_fbx   # and the same from Fox.glb into /tmp/fox_glb, then:
python3 -I tests/compare_guides.py /tmp/fox_glb/Fox.guides.json /tmp/fox_fbx/Fox.guides.json
python -I tests/blender_skin_eval.py /tmp/fox_fbx/Fox.hive.json --source /tmp/models/Fox/Fox.fbx --require-artist
```

FBX input is tested in `cargo test` against hive-rig's own writer (every
vertex, normal, UV, colour, piece, material, texture, blend shape, bone and
skin influence reads back), on the same file rewritten with compressed arrays,
64-bit records, a Z-up axis system in inches and a model turned by rotation,
pre-rotation and pivot, with clusters storing their mesh transform either way
the format is written in the wild, on damaged and ASCII files, and by reading a
turned rig's `.fbx` back into the same guides. In CI, Blender's own importer
and exporter turn public models (Fox, Soldier, Michelle, CesiumMan) into FBX
files; hive-rig rigs each from its `.glb` and its `.fbx`, and the two guides
files must agree (`compare_guides.py`), and `blender_skin_eval.py` reads the
`.fbx` with Blender's FBX importer and requires every rig vertex to sit on one
of Blender's, then compares the weights with the artist's. Unknown chains
(ears, a branching tail, hair, with IK helpers left out, on a biped and on the
dog) and copying weights onto extra chains named differently (by position)
have unit tests in `src/joints.rs` and `src/rig.rs`.

Orientation and joint reading are tested in `cargo test` on built-in bodies in
every one of the 24 orientations (a T-pose, an A-pose and the dog: up, facing
and template), a round trip of a turned rig through the `.glb`, `.fbx` and
`.hive.json`, nameless biped and dog skeletons read by their shape, and the app
opening a turned character and turning it by hand. CI rigs three public
characters fetched at pinned commits (`tests/fetch_skin_models.sh`): Soldier,
which faces -Z, Michelle, whose armature stands her up, and Fox, a quadruped.
It checks the printed orientation and template, that the guides stand upright
with the left side on +X, and the skin weights against the artist's with
`blender_skin_eval.py --require-artist`, which fails when the source's vertices
don't line up with the rig's. The fetch script also gets Xbot, RobotExpressive,
CesiumMan, RiggedFigure and BrainStem;
[docs/skinning.md](docs/skinning.md#orientation-and-joint-reading) has the
before and after measurements on all eight.

`blender_fbx_check.py` imports the `.fbx` with Blender's own FBX importer and
compares it with the `.hive.json`: the bone count, names, parents and rest
positions, the mesh's vertices, every vertex's weights (all influences), that
the skin doesn't move at rest, and that posing a bone moves only the skin it
(or a bone below it) weighs on. CI runs it on the demo, the dog, bendy limbs,
an 8-influence rig and side-first names. `src/export/fbx.rs` reads the file
back with a strict parser in its unit tests.

`blender_check.py` builds the rig in real Blender and checks that it works as
a rig: the rest pose is unchanged, feet and hands reach their IK goals, knees
bend forward and elbows bend back, and FK rotates the limbs. It checks that
every control's custom shape sits on the outline hive-rig wrote for it. It checks the heel,
ball and toe roll stages, stretchy IK, the forearm twist share, and space
switching (no pop at rest, and the IK hand follows the chest only in chest
space). It also checks that every driver uses a simple expression, so the rig
works without enabling Python auto-run. Finally, the COG moves the body, the
mesh deforms, and the `.glb` re-imports with weights on every bone. It also
compares every vertex's weights, as the builder script sets them and as the
`.glb` importer reads them (all `JOINTS_n`/`WEIGHTS_n` sets), with the rig's,
and checks no vertex has more influences than the rig's limit. CI runs it on
rigs with 8 influences, 4 with pruning, and copied weights.
`blender_guides_check.py` edits the guides in real Blender. It moves the elbow,
mirrors it, paints a weight, adds fingers and a tail, and rebuilds. Then it
checks that the bones moved, the painted weight survived, the new fingers got
weights and controls, and a finger control bends the finger.
`blender_anim_check.py` poses each IK arm and leg (moved, turned, pole pushed
aside, foot rolled), runs IK to FK and checks the FK controls and bones land on
the IK pose; then poses each FK limb and checks FK to IK leaves every bone in
place, including a straight limb. It mirrors a posed left side and checks the
right side is its mirror image, mirrors one selected control, switches the IK
hand's space with the chest turned, and resets the rig back to its rest mesh.
Every IK limb gets those checks, however many legs a side (the spider's eight).
A rig without both arms and legs (the spider, the snake) mirrors its posed left
controls onto the right and checks each limb's chain mirrors too, switches the
space of the first control that has spaces, and resets.

`blender_quad_check.py` builds the dog in real Blender. It checks that the paws
reach their IK goals with the hock joined to the paw, the knee folds forward
and the front elbows fold back, the poles stand straight ahead of the knees,
the hock control swings the hock while the paw stays planted, FK moves the
legs, the paw rolls on its heel, pads and toe tip, stretch reaches a far goal,
lifting a hind paw leaves the front paw's skin alone, and the `.glb` re-imports.

`blender_spline_check.py` builds a rig with a tentacle and a spline spine in
real Blender. For both chains it checks that the joints sit on the curve at
rest and stay on it when the middle curve controls bend it, that the chain
bends smoothly with its base in place, that with `stretch` on the chain reaches
a far end control and with it off the joints keep their length, that `volume`
thins stretched joints, that the first FK control swings the chain, and that
the end control turns the last joint. On the spine it checks that the chest
control carries the head and arms with the feet planted, that the hips turn the
pelvis, and that the IK hand can follow the chest. A bent tentacle deforms the
mesh, reset brings it back to rest, and the `.glb` re-imports with every joint.

`blender_bendy_check.py` builds a biped with bendy limbs and soft IK in real
Blender. On every arm and leg it moves the upper bend control and checks that
the bend joints bow out along the curve (the middle point by the full offset,
the others by 4t(1-t) of it) while the shoulder or hip, elbow or knee, wrist or
ankle and the lower segment stay put, and that the skin follows; the pin moves
where the two curves meet. For soft IK it sweeps the IK foot out through full
reach and checks that the knee opens monotonically and smoothly and never snaps
straight (its fastest opening rate drops several-fold against `soft` 0), that
the foot follows hive-rig's `soft_reach` curve, that with stretch the foot
reaches the goal with the knee still bent, and that FK to IK on a nearly
straight leg doesn't pop. Drivers stay simple expressions.
The app's editing operations (add, remove, mirror, symmetric moves, undo, build,
export, reopening a rig and loading templates) have unit tests in `src/studio/app.rs`;
templates have their own (`src/template.rs`: JSON round trip, scaling to another
body height, id clashes, inner parent links and the folders). The pre-build
checks are tested in `src/checks.rs`: the demo biped (T- and A-pose, with every
premade part) and the dog have no warnings (`src/demo.rs` checks the bird, spider
and snake the same way on their demo bodies, and that every deforming bone of
each gets skin), each broken case gives exactly its
own warning, and its fix clears it; the app's tests select and fix them and
build with them. Versioning has tests for the stamp, files from before it
building exactly as before, and files from a newer hive-rig warning. CI runs
`hive-rig check` on the demo and every creature, and opens
the app on a virtual screen and screenshots it after building the demo rig, and
again with the weight brush after one dab (`--dab subtract`). The skin tools
(prune, influence caps, the brush's add, subtract, smooth and its undo, keeping
edits across a rebuild, and copying weights between the T-pose and A-pose demo)
have unit tests in `src/skin.rs`, `src/rig.rs` and `src/studio/app.rs`. The hand
edits have their own in `src/skin/edit.rs` (locks under every brush mode and the
influence cap, Replace, Scale, Mend on a planted spike, fixing spikes, flood,
paste, re-running the automatic weights on a bone's skin, normalize, grow and
shrink) and the app's in `src/studio/app/skin_edit.rs` (a locked bone through a
whole stroke, copy, paste, flood, weight table drags and the automatic-weights
rerun, each one Undo step; click and box selection seeing only the front; the
paint bend). The app screenshots them with `--weights BONE --lock BONE --bend 70
--bend-axis z --dab replace --vertices --hover --tools`; the
`.glb`'s second joints/weights set is read back in `src/export/glb.rs`.
The built-skin checks have tests in `src/checks.rs` (the demo is clean; bare
vertices and a bone stripped of its skin are each reported), `src/skin/edit.rs`
(filling a bare patch from round its edge, a loose piece left bare) and
`src/studio/app.rs` (Select, Fill and Auto from the Checks card, each one undo
step). Control looks have tests in `src/controls.rs` (every shape fits its
size and reads as itself), `src/components.rs` (one control, its twin with the
side colour swapped, a whole component scaled, reset) and the app's (symmetry
reaching the twin in one undo step). `blender_check.py` compares every
control's drawn lines and colour with the rig; CI styles controls with
`hive-rig control`, breaks and repairs a rig's skin with `check --fix` and
`weights auto`, and screenshots the app with a restyled control picked.
`blender_anim_check.py` also flips the pose of the demo, the dog and every
creature (every deforming bone lands on its twin's mirror image, the centre
moves, flipping twice gives the pose back) and runs the pose library (save,
apply, apply mirrored equals the flip, selected only, delete, and the Hive
tab's buttons); the Maya stubs run the same calls.
CI runs all of this (`.github/workflows/hive-rig.yml`), and builds both Windows
executables as a downloadable artifact.

`blender_mesh_check.py` checks that a rigged character still looks like its
source, in real Blender. It reads the source `.glb` with a small reader of its
own (not hive-rig's importer or Blender's) and, after building the rig with the
script, compares: the object count and each object's material pieces, every UV
set (matched by vertex position, so vertex order doesn't matter), the
materials and that every texture is loaded, and every morph target as a shape
key with the same offsets. With `--fbx` it repeats the UV, material, texture and
shape checks on the `.fbx` through Blender's FBX importer. With `--corrective`
it checks the shape's driver: off at rest, on when the control bends the joint
90 degrees, half on at half the bend, the mesh moves, and bending the other way
leaves it off (for a shape with in-betweens, the half-bend check gives way to
the round-trip check below). `blender_roundtrip_check.py` takes a rig with a
driven corrective and an undriven slider shape, both with in-betweens, and in
real Blender checks that every target is a shape key with hive-rig's offsets,
that the drivers are simple expressions, that the corrective peaks at its
in-between's angle and the slider shows each target at its weight; then it
repaints one vertex, resculpts a key, adds a shape key, moves the slider, runs
**Send to Hive** and checks the `.hive.json`: exactly that vertex changed, the
sculpt and the new shape came in, drivers and in-betweens were kept, and
**Rebuild** keeps all of it. CI downloads two public models from the Khronos glTF sample
assets at a pinned commit (CesiumMan, textured and skinned; MorphPrimitivesTest,
two materials and a morph target), rigs them and runs it; the downloads are
never committed. Locally it also passed on Fox, AnimatedMorphCube, MultiUVTest
and VertexColorTest. Unit tests read the `.glb` back through the `gltf` crate
and hive-rig's loader (objects, UV sets, colours, materials, the embedded image,
morph targets), the FBX with the strict reader (objects, UV layers by polygon
vertex, materials, textures, blend shapes, skins and the bind pose), OBJ and MTL
import, the textures folder, and the corrective sculpt (in its pose the
vertices land where they were put; at rest it's off).

`blender_fbx_edits_check.py` stands real Blender in for Unreal: it imports the
rig's `.glb`, repaints one vertex onto the head and pushes one shape key point,
exports an FBX with Blender's exporter, runs `hive-rig edits`, `weights import`
and `shape import`, and checks that exactly that vertex (and its seam copies)
changed weights, the point moved by the push and every other offset is exactly
as it was; CI runs it on the demo (with an in-between) and the horse.
`blender_studio_pose_check.py` checks that the studio pose is in Blender's pose
library and, applied, puts every deforming bone where `hive-rig pose --bones`
says (FK, IK, the body, a pole, a foot roll and an IK/FK switch in CI).

The Maya and Unreal builders, and the Maya and Unreal animator tools and Maya guide tools, are checked against stub
APIs. The stubs also check the spring hock (Maya's helper chain, pole and
blended orient constraint; Unreal's helper solve before the leg's own, the
hock control hung from the turning null), Unreal's mirror pose (only the twins
move, with their limb settings, on a frame or the range) and reset pose, and
Unreal's Send to Hive (an FBX export with morph targets, then `hive-rig edits`
and the imports leave an unedited rig unchanged). That catches script errors but doesn't prove the application calls behave
as intended (the spline chains' ikSplineHandle and Fit Chain on Curve setups
included). The Maya stub also checks that every control curve point is placed
on the control's outline, that every shape reaches a blendShape target and every
corrective an expression, and that every object gets a skinCluster; the Unreal
stub checks a curve and a reader per corrective. The builders haven't been run inside Maya or Unreal yet,
and the `.fbx` has been imported only by Blender, not yet by Unreal, Unity or
Autodesk's FBX SDK.
The Unreal builder recovers the import's axes from four bones (the pelvis,
left hand, head and left ball where the rig has them, otherwise other bones off
their plane); for a flat rig like the snake it takes three and assumes the
glTF-to-Unreal import mirrors, which is untested in Unreal.

## Not yet included

See [the gap list](docs/hive-gaps.md)
for what's planned. In short: cone and RBF pose readers, guide editing in Unreal (rebuild with the CLI and rerun the script),
the Unity avatar template and Mixamo names tried in Unity and Unreal,
the spring hock in the app's posing, the studio pose in Unreal, ASCII FBX input,
FBX animation, and the mesh details listed under [Mesh fidelity](#mesh-fidelity).
