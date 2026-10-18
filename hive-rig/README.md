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
cargo run --release -- demo -o out/ --creature bird # also dog, spider, snake
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
  shows the rig summary, the pre-build [checks](#checks-before-building), skin weights (influence limits, the paint brush, Mirror and Smooth) and naming, or, with a component
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
| `<name>.glb` | Blender, Unreal, any glTF tool | Mesh, skeleton and skin weights |
| `<name>.fbx` (with `--targets fbx`) | Unreal, Unity, any FBX tool | Mesh, skeleton, skin weights and bind pose |
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
directly as a skeletal mesh, with no script. It holds the mesh (triangles,
smooth normals, one material), one joint per bone named with the rig's
[naming convention](#naming-conventions) and parented as in the skeleton, the
skin weights with every influence the skin keeps (8 for `--engine unreal`, up to
12; FBX has no limit of four), and a bind pose for the mesh and every bone. It
is Y-up, facing +Z, in centimetres; Unity and Blender scale it to metres. Joints
carry translation only, like the `.glb`. FBX has no controls: use the Unreal
script for a Control Rig.

```bash
hive-rig demo -o out --targets fbx --engine unreal   # out/hive_demo.fbx with 8 influences
hive-rig rebuild out/hive_demo.hive.json --targets unreal,fbx
```

The writer is a small hand-written one in `src/export/fbx.rs` (no FBX SDK, no new
crates), from the publicly described binary layout. Arrays are written
uncompressed, so files are larger than ones written with compression.

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
  Blender (drivers and constraints) and Maya (a node network) build it; Unreal
  doesn't yet and solves plain stretchy IK.
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

```bash
hive-rig naming out/hero.guides.json prefix
hive-rig naming out/hero.guides.json --joint "{side}_{name}_{desc}_bnd" --left Lf --right Rt --center Ct
hive-rig rebuild out/hero.hive.json
```

Patterns place `{name}` (the part, such as `upperarm` or `index`), `{side}` and
`{desc}` (`fk`, `ik`, `pole`, `aim`, chain numbers); empty parts and their
underscores drop out. The convention is stored in the guides file, so rebuilds
from Blender, Maya or the command line keep it, and the guides themselves keep
the internal names. Names that would collide are refused.

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
| quad_leg | hip, knee, hock, ankle, ball, toe, heel | three-segment hind leg (`thigh`, `calf`, `hock`, `foot`, `ball`): IK with a hock control, reverse paw roll, FK, stretch, spaces, twist bones |
| chain | two or more | an FK chain: fingers, tails, ears, hair, props |
| spline | two or more (`j1`…) | a curve-driven chain (tentacle): curve controls, an FK layer, `stretch` and `volume` |
| eyes | eye_l, eye_r, target | `eye_l`, `eye_r` aimed by a look-at control that follows the head or stays in the world |
| jaw | jaw, chin | `jaw` with a rotate control |
| face | brows (3 a side), upper and lower lids, cheeks, nose, upper and lower lip, lip corners | 17 face joints on the head, each with a small control that moves and rotates it; lids pivot on the eye, the lower lip rides on the jaw |

The first rig detects the body and places the default biped. From then on you
can change the layout and rebuild:

```bash
hive-rig add out/hero.guides.json fingers           # five fingers on each hand
hive-rig add out/hero.guides.json tail --joints 6
hive-rig add out/hero.guides.json eyes              # look-at eyes on the head
hive-rig add out/hero.guides.json jaw
hive-rig add out/hero.guides.json face              # brow, lid, cheek, nose and lip joints
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
  by `<name>_stretch`. Volume isn't built in Unreal yet, and Fit Chain on Curve
  takes one cubic Bézier, so with more than four controls the curve is an
  approximation.

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
* **Front legs** are arm components laid out as a leg: the clavicle is the
  shoulder blade and the elbow points back. An arm whose elbow and wrist sit
  below the belly line (`crotch_y`) skins as a leg, starts in IK and gets an
  upright IK control.

Body detection only knows bipeds. To rig your own four-legged mesh, start from
the dog's guides and move them onto it (in the app, or Edit Guides in Blender
or Maya): `hive-rig rig my_dog.glb --guides out/hive_dog.guides.json`.

### More creatures: bird, spider and snake

`hive-rig demo --creature <dog|bird|spider|snake>` rigs a built-in creature
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
  abdomen chain and **eight legs**, each a four-bone FK chain (`leg1_l` to
  `leg4_r`) hung from the spine bone it grows from; front legs reach forward,
  back legs back. FK chains have no IK, so a spider is posed leg by leg.
* **Snake** (`components::snake`, a 0.5 m size unit; about 2 m long): a
  **spline spine** of ten joints driven by five curve controls, a raised neck
  and head, and a spline (`tentacle`) tail of eight joints and four curve
  controls, all in the mid-plane.

The spider and snake have no leg or arm skin regions: every vertex takes the
bones nearest it. Their pre-build checks are clear, as the dog's are.

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
jaw when the rig has them.

### Bring your own skeleton (joints to guides)

Open or rig a .glb that already has a skeleton and its joints become the
guides: Unreal mannequin, Mixamo (`mixamorig:LeftArm`), 3ds Max biped
(`Bip01 L UpperArm`), and side-first or side-last Maya names (`L_arm_jnt`,
`arm_L_JNT`, `thigh.L`) are recognised. Eyes, a jaw, face joints and fingers
are added when the skeleton has them; twist and helper joints are listed as not
used. Guides with no matching joint stay where body detection put them. Pass
`--ignore-joints` to place the guides on the mesh instead.

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
  control has the same grid.

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

Set one by hand in `guides.json` with
`"control_styles": { "head_ctrl": { "shape": "sphere", "size": 0.12 } }`.
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
guides can't build. Given a `.hive.json` it uses the rig's mesh and the guides
next to it (the ones `rebuild` reads). `rig`, `rebuild` and `demo` print the
same warnings before building.

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

### Weight tools

After Build, the Properties panel's skin weights section has **Mirror L to R**
(right-click for R to L), which copies one side's weights onto the other by
mirrored vertex position, and **Smooth**, which blurs the shown bone's weights
(or every bone's) with their neighbours.

* **Paint**: pick a bone in the weights list, turn on **Paint** and drag on the
  mesh. **Add** raises the bone's weight (the vertex's other bones give way),
  **Subtract** lowers it (the other bones take it up; a vertex held by that bone
  alone passes to its parent), **Smooth** blends towards the neighbours. The
  brush fades to nothing at its rim; set its radius (or press `[` and `]`) and
  strength. With Symmetry on, the twin bone is painted on the other side. Each
  stroke, mirror and smooth is one Undo step. Dragging off the mesh still orbits.
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
hive-rig rebuild out/hero.hive.json --keep-weights  # keep hand-edited weights
```

`weights copy` moves one character's weights onto another with the same or a
similar skeleton, in any pose or size (an A-pose character from a T-pose one,
say). Bones are matched by their hive-rig name whatever the naming convention,
then by name after side tokens, prefixes and suffixes are read (`L_arm_jnt`,
`upperarm_l`). Each target vertex is carried into the source's pose by its own
bones (each matched bone turned onto its source bone), then takes the weights
at the closest point of the source surface, blended across that triangle.
Source bones with no match hand their weight to the nearest matched parent. The
target is written again in its own folder (or `-o`).

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

### Animator tools in Blender and Maya

Below the guide tools, the Hive tab (Blender) and the Hive Rig window (Maya)
have an **Animation** section:

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
* **Reset** puts every control, or only the selected ones, back to the bind
  pose and default settings. **Select All Controls** selects the controls only.
* **Switch space without a pop**: Blender shows a button per space for the
  active control; Maya asks which space to follow for the selected control. The
  control keeps its place in the world.

With auto key on, the tools key what they changed. Which controls make up each
limb and which controls are left/right twins come from hive-rig (`ik_chains` and
`mirror_pairs` in the rig data), so the tools work with any naming convention.
Unreal doesn't have these tools yet.

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
   to the rig's max influences (4 unless set), are smoothed over the surface,
   and influences below the prune threshold (if any) are dropped.
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
python tests/blender_guides_check.py out/hive_demo_blender.py           # guides and rebuild
python tests/blender_quad_check.py dog/hive_dog_blender.py dog/hive_dog.glb   # after demo --quadruped -o dog
python tests/blender_spline_check.py out/hive_demo_blender.py out/hive_demo.glb   # after add tentacle, spine --spline true, rebuild
python tests/blender_bendy_check.py bendy/hive_demo_blender.py   # after demo --bendy --soft 0.5 -o bendy
python3 tests/dcc_stub_run.py maya out/hive_demo_maya.py --guides       # Maya guide tools
python tests/blender_anim_check.py out/hive_demo_blender.py             # animator tools
python tests/blender_fbx_check.py out/hive_demo.fbx out/hive_demo.hive.json   # after demo --targets fbx -o out
python tests/blender_anim_check.py bird/hive_bird_blender.py   # after demo --creature bird -o bird (or spider, snake)
```

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
A rig without arms and legs (the spider, the snake) skips the limb checks: it
mirrors its posed left controls (the spider's FK legs) onto the right, switches
the space of the first control that has spaces (the head), and resets.

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
have unit tests in `src/skin.rs`, `src/rig.rs` and `src/studio/app.rs`; the
`.glb`'s second joints/weights set is read back in `src/export/glb.rs`.
CI runs all of this (`.github/workflows/hive-rig.yml`), and builds both Windows
executables as a downloadable artifact.

The Maya and Unreal builders, and the Maya guide and animator tools, are checked against stub
APIs. That catches script errors but doesn't prove the application calls behave
as intended (the spline chains' ikSplineHandle and Fit Chain on Curve setups
included). The Maya stub also checks that every control curve point is placed
on the control's outline. The builders haven't been run inside Maya or Unreal yet,
and the `.fbx` has been imported only by Blender, not yet by Unreal, Unity or
Autodesk's FBX SDK.
The Unreal builder recovers the import's axes from four bones (the pelvis,
left hand, head and left ball where the rig has them, otherwise other bones off
their plane); for a flat rig like the snake it takes three and assumes the
glTF-to-Unreal import mirrors, which is untested in Unreal.

## Not yet included

See [the gap list](docs/hive-gaps.md)
for what's planned. In short: corrective shapes, more creature templates (horse, dinosaur), IK legs beyond one
arm and one leg a side (the spider's legs are FK chains), guide editing in Unreal (rebuild with the CLI and rerun the script), the
animator tools (IK/FK matching, mirror and reset pose) in Unreal, FBX input, and
FBX animation, blend shapes or UVs (the mesh has none yet).
