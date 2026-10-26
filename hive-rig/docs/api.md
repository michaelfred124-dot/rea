# hive-rig developer API

Everything the Hive Rig app does can be scripted three ways:

1. **The `hive-rig` command line**: every command and flag is listed in the
   [CLI reference](#cli-reference) at the end of this page.
2. **The files it reads and writes**: plain JSON (guides, rig, scene, edits,
   pose), so any language can make or change them and hand them back.
3. **The Rust library** (`hive_rig` crate): the same functions the CLI and the
   app call.

All positions in every file are in hive-rig's frame: **Y up, metres, the
character facing +Z** (the glTF convention). A source file in another frame is
turned into it on import (its `orientation` is kept in the rig, and the .glb
and .fbx turn it back). Angles are degrees unless a field says otherwise.

## Scripting with the command line

A typical batch: rig a character, add parts to its guides, check them, rebuild,
pose it and export for engines.

```sh
hive-rig rig hero.fbx -o out --targets blender,maya,unreal,fbx --naming game
hive-rig add out/hero.guides.json fingers
hive-rig add out/hero.guides.json tail --joints 6
hive-rig add out/hero.guides.json face
hive-rig check out/hero.guides.json --mesh hero.fbx --fix
hive-rig rebuild out/hero.hive.json --keep-weights --targets blender,fbx
hive-rig pose out/hero.hive.json --set "hand_l_ik_ctrl=0,0.1,0,0,0,30" --attr hand_l_ik_ctrl.ik_fk=1
hive-rig weights limit out/hero.hive.json --engine unity
```

Commands exit non-zero on an error, with the reason on stderr; warnings (such
as `check` findings, or a file written by a newer hive-rig) go to stderr too,
and what was written goes to stdout. Commands that change a guides file save it
in place; commands that change a rig write its `.glb`, `.hive.json` and the
scripts and `.fbx` already next to it again.

Accepted inputs: `.obj` (with its `.mtl`), `.gltf`, `.glb` and `.fbx` (FBX 7.x,
binary or ASCII). A file with a skeleton has its joints turned into guides
unless `--ignore-joints` is given.

`hive-rig help-all` prints the help of every command; `hive-rig help-all
--markdown` prints the [CLI reference](#cli-reference) below. A test fails when
this page and the program disagree, so the reference can't fall behind.

## File formats

Every hive-rig file starts with `format` (what it is), `version` (its layout
version) and `hive_rig` (the program version that wrote it). A file from a
newer hive-rig opens with a warning; guides keep their `version` when saved
again, because it pins the builder rules the file doesn't spell out.

### Guides: `<name>.guides.json`

The editable layout a rig is built from: components, their guide positions and
their options. This is the file to generate or edit when building rigs from a
script; `hive-rig rebuild`, `hive-rig rig --guides` and the Rust
`build_rig` take it.

```json
{
  "format": "hive-rig-guides",
  "version": 1,
  "hive_rig": "0.1.0",
  "body": { "floor": 0.0, "height": 1.8, "center_x": 0.0, "crotch_y": 0.845, "torso_half_width": 0.1 },
  "components": [
    { "id": "root", "kind": "root", "side": "center",
      "guides": [ { "name": "root", "position": [0.0, 0.0, 0.0] } ],
      "options": { "twist": true } },
    { "id": "spine", "kind": "spine", "side": "center", "parent": "root",
      "guides": [ { "name": "pelvis", "position": [0.0, 0.953, 0.0] },
                  { "name": "chest",  "position": [0.0, 1.570, 0.0] } ],
      "options": { "twist": true, "joints": 3 } },
    { "id": "arm_l", "kind": "arm", "side": "left", "parent": "spine_03",
      "guides": [ { "name": "clavicle", "position": [0.036, 1.375, 0.018] },
                  { "name": "shoulder", "position": [0.085, 1.411, 0.0] },
                  { "name": "elbow",    "position": [0.410, 1.400, -0.011] },
                  { "name": "wrist",    "position": [0.674, 1.400, 0.0] },
                  { "name": "hand_tip", "position": [0.860, 1.400, 0.0] } ],
      "options": { "twist": true, "bendy": true, "soft": 0.3 } }
  ],
  "naming": { "joint": "{name}", "control": "{name}", "left": "l", "right": "r", "center": "" },
  "control_styles": { "hand_l_ik_ctrl": { "shape": "cube", "size": 0.08, "color": [0.1, 0.45, 1.0], "line_width": 2.0 } },
  "custom_shapes": {},
  "skin": { "max_influences": 8, "prune": 0.01 }
}
```

- `body`: whole-body measurements (floor height, height, mid-line x, crotch
  height, half the chest width), used by the checks and to place added parts.
- `components[]`: `id` (unique), `kind`, `side` (`left`, `right`, `center`),
  `parent` (the **bone** it hangs from; only the root has none), `guides`
  (named points) and `options`.
- `kind` and its guide names, in order:

  | kind | guides |
  |------|--------|
  | `root` | `root` |
  | `spine` | `pelvis`, `chest` |
  | `head` | `neck`, `head`, `head_top` |
  | `arm` | `clavicle`, `shoulder`, `elbow`, `wrist`, `hand_tip` |
  | `leg` | `hip`, `knee`, `ankle`, `ball`, `toe`, `heel` |
  | `quad_leg` | `hip`, `knee`, `hock`, `ankle`, `ball`, `toe`, `heel` |
  | `eyes` | `eye_l`, `eye_r`, `target` |
  | `jaw` | `jaw`, `chin` |
  | `face` | `brow_in_l` ... `lip_corner_r` (17, see `FACE_GUIDES` in `src/components.rs`) |
  | `chain`, `spline` | two or more, any names (fingers, tails, ears, tentacles) |

- `options` (all optional): `joints` (spine joints, a spline chain's joints, or
  a face's extra lip joints), `twist` (twist helper bones, default on),
  `spline` (spline spine), `controls` (curve controls on a spline), `bendy`,
  `soft` (0 to 1), `spring` (quad leg hock spring, 0 to 1), `board` (face
  board), `knee_up` (splayed legs whose knee folds up) and the others in
  `Options` in `src/components.rs`, each documented there.
- `naming`: patterns using `{name}`, `{side}` and `{desc}`, or set from a
  preset with `hive-rig naming <guides> game|prefix|suffix|unity|mixamo`.
- `control_styles`: per-control look by internal control name (`shape`,
  `custom`, `size`, `color`, `line_width`); `custom_shapes`: drawn or
  imported shapes by name (lines of 2D points).
- `skin`: `max_influences` (1 to 12) and `prune` (smallest weight kept).

Bone names a `parent` can use are those the layout builds (`root`, `pelvis`,
`spine_01`..., `neck_01`, `head`, `clavicle_l`, `hand_l`, `foot_l`, ...);
`hive-rig components <guides>` lists the components and their bones.

### Rig: `<name>.hive.json`

The whole built rig, for other tools to read and for `rebuild`, `pose`,
`weights`, `shape`, `edits` and `check` to work on. It holds everything the
builder scripts are made from.

```json
{
  "format": "hive-rig",
  "version": 2,
  "hive_rig": "0.1.0",
  "name": "hero",
  "up_axis": "Y",
  "units": "m",
  "orientation": { "up": "+z", "forward": "-y" },
  "guides": { "format": "hive-rig-guides", "...": "the guides it was built from" },
  "mesh": {
    "positions": [[0.1, 0.06, -0.065], "..."],
    "normals":   [[0.037, -0.828, 0.56], "..."],
    "indices":   [0, 16, 17, "..."],
    "uvs": [[[0.5, 0.5], "..."]], "colors": [], "objects": ["body"],
    "parts": [], "materials": [], "images": [], "shapes": []
  },
  "skeleton": { "bones": [
    { "name": "pelvis", "parent": 0, "head": [0.0, 0.953, 0.0], "tail": [0.0, 1.025, 0.0],
      "deform": true, "side": "center", "region": "center", "component": "spine", "translate": true }
  ] },
  "skin": { "joints": [[23, 21, 24], "..."], "weights": [[0.507, 0.493, 0.0002], "..."] },
  "controls": {
    "controls": [
      { "name": "cog_ctrl", "kind": "cog", "parent": { "type": "control", "name": "root_ctrl" },
        "bone": null, "position": [0.0, 0.953, 0.0], "axis": [0.0, 1.0, 0.0],
        "shape": "square", "size": 0.306, "color": [1.0, 0.85, 0.1], "outline": [[[-0.306, 0.953, -0.306], "..."]] }
    ],
    "ik_chains": [
      { "name": "arm_l", "upper": "upperarm_l", "lower": "lowerarm_l", "end": "hand_l",
        "ik_control": "hand_l_ik_ctrl", "pole_control": "lowerarm_l_pole_ctrl",
        "fk_controls": ["upperarm_l_fk_ctrl", "lowerarm_l_fk_ctrl", "hand_l_fk_ctrl"],
        "default_blend": 0.0, "default_stretch": 0.0, "default_soft": 0.0, "soft_zone": 0.1 }
    ],
    "mirror_pairs": [["clavicle_l_ctrl", "clavicle_r_ctrl"]]
  },
  "pose": { "channels": {}, "attributes": {} }
}
```

- `mesh`: one vertex list (split where normals, UVs or colours differ),
  triangles in `indices`, every UV set, vertex colours, objects and their
  material `parts`, materials with texture `images` (files in
  `<name>_textures/`), and blendshapes (`shapes`: name, weight, sparse
  `vertices` and `offsets`, in-betweens, and the pose readers driving them).
- `skeleton.bones[]`: internal names (the guides' `naming` renames them on
  export), `parent` as an index into `bones`, `head` and `tail`, `deform`.
- `skin`: per vertex, bone indices and weights (summing to 1), as wide as
  `guides.skin.max_influences`.
- `controls`: every control with its parent (`none`, a `control` or a
  `bone`), the bone it drives, its look and drawn `outline`; IK chains with
  their pole and FK controls; spline chains; left/right `mirror_pairs`; the
  face board when there is one.
- Optional fields are left out when empty or at their default.

### Pose: `pose` in the `.hive.json`, and `--bones`

The control pose posed in the app or with `hive-rig pose`, kept with the rig
(the Blender and Maya builders add it to their pose library as **Studio
pose**). Channels are offsets from rest in each control's rest frame; scale
defaults to 1. Attributes are the controls' custom settings (`ik_fk`,
`stretch`, `soft`, `spring`, `held`, face board sliders...).

```json
"pose": {
  "channels": {
    "hand_l_ik_ctrl": { "translate": [0.0, 0.1, 0.0], "rotate": [0.0, 0.0, 30.0], "scale": [1.0, 1.0, 1.0] }
  },
  "attributes": { "hand_l_ik_ctrl": { "ik_fk": 1.0 } }
}
```

`hive-rig pose <rig> --bones bones.json` also writes where the pose puts every
bone (metres, hive-rig's frame), keyed by internal bone name:

```json
{ "ball_l": { "head": [0.1, 0.036, 0.1435], "tail": [0.1, 0.018, 0.2] } }
```

The Unreal builder keeps its own pose library in `<name>.poses.json` next to
its script: pose name to control name to `{"location", "rotation" (roll,
pitch, yaw), "scale"}`, `{"position"}` or `{"float"}`, as Unreal's Control
Rig stores them.

### Scene: `<name>.scene.json`

Several characters, each with its own guides and last-built rig, and props held
across characters. Made with `hive-rig scene new`, built with `hive-rig scene
build`.

```json
{
  "format": "hive-rig-scene",
  "version": 1,
  "hive_rig": "0.1.0",
  "name": "duel",
  "characters": [
    { "name": "hero", "source": "hero.glb", "offset": [0.0, 0.0, 0.0],
      "guides": { "format": "hive-rig-guides", "...": "..." }, "rig": "hero.hive.json" },
    { "name": "rival", "source": "demo", "offset": [-1.45, 0.0, 0.15],
      "guides": { "format": "hive-rig-guides", "...": "..." } }
  ],
  "holds": [
    { "prop": { "character": "hero", "control": "prop_r_01_ctrl" },
      "holder": { "character": "rival", "bone": "hand_l" }, "held": 1.0 }
  ]
}
```

`source` is a character file, a saved rig or a built-in demo (`demo`,
`demo:dog`, `demo:bird`...), relative to the scene file; `orientation` is kept
when the source wasn't in hive-rig's frame.

### Edits: `<name>.edits.json`

Weights painted and shapes sculpted in another program, brought back with
`hive-rig weights import` and `hive-rig shape <rig> import`. The Blender and
Maya **Send to Hive** buttons write it; `hive-rig edits <rig> <mesh.fbx>`
makes it from any FBX exported from the rig's mesh (Unreal's tool does this).

```json
{
  "hive_rig_edits": 1,
  "rig": "hero",
  "source": "blender",
  "vertex_count": 2676,
  "weights": [ { "upperarm_l": 0.75, "clavicle_l": 0.25 }, "... one per vertex" ],
  "shapes": [ {
    "name": "elbow_fix_l", "weight": 0.0,
    "vertices": [12, 13], "offsets": [[0, 0.01, 0], [0, 0.02, 0]],
    "inbetweens": [ { "weight": 0.5, "vertices": [12], "offsets": [[0, 0.004, 0]] } ]
  } ],
  "weight_tolerance": 0.005
}
```

`weights` and `shapes` are both optional. Vertices are the rig's, in its order
(the builders keep it); bones by the rig's internal names; offsets in metres in
hive-rig's frame at rest.

### Templates and shape libraries

`hive-rig template save` writes a `hive-rig-template` file (a layout's
components, options, naming and looks, without the character's positions) and
`template apply` fits one onto a guides file. The user's custom control shapes
live in a `hive-rig-shapes` library, `~/.config/hive-rig/shapes.json` by default (the `HIVE_RIG_SHAPES` environment variable names another file).

## The Rust library

Add the crate (`hive-rig = { path = "hive-rig", default-features = false }`
leaves out the desktop app) and call the same functions the CLI does:

| Entry point | What it does |
|---|---|
| `import::load_character(path, choice)` | Read .obj/.gltf/.glb/.fbx, detect up and facing (or take `orient::Choice`), turn it into hive-rig's frame; gives the mesh, its joints and body kind |
| `import::load_mesh(path)`, `import::fbx::load(path)` | Just the mesh; an FBX's mesh, bones and skin |
| `rig::detect_guides(&mesh)` | Find the body landmarks and place the biped guides |
| `rig::character_guides(&mesh, &joints, body)` | Guides for a character, moved onto its own skeleton when it has one |
| `components::biped`, `quadruped`, `bird`, `spider`, `snake`, `horse`, `dinosaur` | Built-in layouts |
| `components::add_preset(&mut guides, Preset::Fingers, &AddOptions)` | Add a premade part (fingers, tail, chain, eyes, jaw, face, face board, toes, quad legs, tentacle) |
| `components::remove`, `attach`, `import`, `mirror_side`, `set_spine_joints`, `set_spine_spline`, `set_limb_options`, `restyle` | Edit a layout |
| `Guides::load(path)`, `guides.save(path)` | Read and write a guides file |
| `checks::check(&guides, Some(&mut checks::Surface::new(&mesh)))` | The pre-build checks; `checks::apply_fix` applies a mechanical fix |
| `components::build(&guides)` | Build the skeleton and control rig alone (no mesh, no skin): `Built { skeleton, controls }` |
| `build_rig(mesh, name, guides, RigOptions)` | The whole rig: skeleton, controls and skin weights |
| `auto_rig(mesh, name, RigOptions)` | `detect_guides` then `build_rig` |
| `Rig::load(path)` | Read a `.hive.json` |
| `rig::keep_weights(&old, None, &mut new)`, `rig::copy_weights(&from, &to)` | Keep hand-edited weights on rebuild; copy weights between characters |
| `pose::evaluate(&skeleton, &controls, &pose)` | Where a control pose puts every bone |
| `export::write_all(&rig, dir, &Target::ALL)` | Write the .glb, .hive.json, .guides.json, builder scripts and .fbx |
| `export::glb::write(&rig)`, `export::fbx::write(&rig)` | One file's bytes |
| `export::script(&rig, Target::Blender, glb_path)` | One builder script as text |
| `scene::Scene::load`, `scene::build_and_write` | Scenes of several characters |
| `edits::Edits::read`, `edits::from_scene` | Edits files, and edits from an FBX of the rig's mesh |

A short program, in `examples/script_rig.rs` (`cargo run --example script_rig
-- [MESH or -] [OUT_DIR]`; `-` uses the built-in test character):

```rust
use hive_rig::components::{self, AddOptions, Preset};
use hive_rig::{RigOptions, Target, build_rig, checks, demo, export, rig};

fn main() -> anyhow::Result<()> {
    let mesh = demo::demo_humanoid(); // or import::load_character(path, Default::default())?.mesh
    let (mut guides, _) = rig::detect_guides(&mesh);
    components::add_preset(&mut guides, Preset::Fingers, &AddOptions::default())?;
    components::add_preset(&mut guides, Preset::Tail,
        &AddOptions { joints: Some(6), ..Default::default() })?;

    let issues = checks::check(&guides, Some(&mut checks::Surface::new(&mesh)));
    anyhow::ensure!(checks::errors(&issues) == 0, "the guides don't build");

    let built = components::build(&guides)?; // skeleton and controls only
    println!("{} bones", built.skeleton.bones.len());

    let r = build_rig(mesh, "scripted", guides, RigOptions::default())?;
    export::write_all(&r, std::path::Path::new("out"), &Target::ALL)?;
    Ok(())
}
```

The example is built by `cargo test` (as every example is) and run in CI.

## How the builder scripts call back into hive-rig

The Blender (`<name>_blender.py`), Maya (`<name>_maya.py`) and Unreal
(`<name>_unreal.py`) scripts carry the rig's data inside them, and near the top
a line

```python
HIVE_EXE = r"""/path/to/hive-rig"""
```

filled in with the `hive-rig` program that wrote the script (the app writes the
path of the `hive-rig` beside it). Their tools run it as a subprocess,
`[HIVE_EXE, *args]`, raise an error with its stderr when it exits non-zero, and
read back the files it wrote:

| Tool in Blender / Maya / Unreal | Runs |
|---|---|
| Add Fingers, Add Tail, Remove Component | `hive-rig add\|remove <name>.guides.json ...`, then reloads the guides |
| Rebuild Rig (Keep Weights on / off) | `hive-rig rebuild <name>.hive.json --guides <name>.guides.json [--keep-weights]` |
| Send to Hive (Blender, Maya) | writes `<name>.edits.json`, then `hive-rig weights import <rig> <edits>` and `hive-rig shape <rig> import <edits>` |
| Send to Hive (Unreal) | exports `<name>_unreal_edits.fbx`, then `hive-rig edits <rig> <fbx> -o <edits> --source unreal`, then the two imports above |

If the program has moved since the script was written, set `HIVE_EXE` at the
top of the script. Your own DCC tools can do the same: write the guides or an
edits file, run `hive-rig`, and re-run the regenerated builder script.

## CLI reference

Generated by `hive-rig help-all --markdown`; `cargo test` fails when it differs
from the program. After changing a command, run
`HIVE_UPDATE_API_DOCS=1 cargo test --test api_docs` to rewrite this section.

<!-- cli-reference:start -->
### `hive-rig rig`

Rig a character (.obj, .gltf, .glb or .fbx; a biped in T- or A-pose, or an animal on four legs). Its up axis and facing are detected (or given with --up and --forward)

```
hive-rig rig [OPTIONS] <INPUT>
```

- `<INPUT>`
- `-o, --out <OUT>`: Output directory (default `hive_out`)
- `-n, --name <NAME>`: Rig name (defaults to the input file name)
- `-g, --guides <GUIDES>`: Build from this guides file instead of detecting the body
- `--ignore-joints`: Detect the body even when the file has a skeleton (by default its joints become the guides)
- `--up <UP>`: The file's axis that points up through the character: +x, -x, +y, -y, +z or -z (default: detected from the feet, legs, head and skeleton)
- `--forward <FORWARD>`: The file's axis the character faces along (default: detected from the toes, or for an animal on four legs its head end)
- `-t, --targets <TARGETS>` (repeatable): What to write besides the .glb: builder scripts for blender, maya and unreal, and fbx for a binary .fbx (mesh, skeleton, skin, bind pose) for game engines such as Unreal and Unity (default `blender,maya,unreal`)
- `--max-influences <MAX_INFLUENCES>`: Most bones that may influence one vertex (1 to 12). glTF writes them in sets of four
- `--engine <ENGINE>`: Set max influences for an engine: unreal (8), unreal12 (12), unity (4) or gltf (4)
- `--prune <PRUNE>`: Drop influences below this weight (such as 0.01) and renormalise
- `--smooth <SMOOTH>`: Skin weight smoothing passes (default `3`)
- `--naming <NAMING>`: Name the exported joints and controls with this convention and keep it in the guides: game (Unreal mannequin), prefix, suffix, unity (Unity Humanoid bone names) or mixamo (mixamorig:Hips and so on). Unset, the guides' own naming is used

### `hive-rig demo`

Rig the built-in test character to try the pipeline without an asset

```
hive-rig demo [OPTIONS]
```

- `-o, --out <OUT>` (default `hive_demo`)
- `--arm-drop <ARM_DROP>`: Lower the demo character's arms by this many degrees (45 = A-pose) (default `0`)
- `--quadruped`: Rig the built-in dog instead (the same as `--creature dog`)
- `--creature <CREATURE>`: Rig a built-in creature instead, written as `hive_<creature>`: a dog (four legs with three-segment hind legs), a bird (wings, two legs with hocks, tail), a spider (eight IK/FK legs), a snake (spline spine and tail with a head), a horse (long legs, spring hocks) or a dinosaur (a theropod: three-segment legs, short arms, spline tail) (one of `dog`, `bird`, `spider`, `snake`, `horse`, `dinosaur`)
- `--bendy`: Make the arms and legs (and quad legs) bendy: bend controls and an elbow or knee pin
- `--soft <SOFT>`: Default soft IK (0 to 1) on the arms and legs (and quad legs) (default `0`)
- `--spring <SPRING>`: Default hock spring (0 to 1) on the quad legs: at 1 the hock swings with the thigh as the paw moves. The horse starts at 1, the others at 0
- `-t, --targets <TARGETS>` (repeatable): What to write besides the .glb: builder scripts for blender, maya and unreal, and fbx for a binary .fbx (mesh, skeleton, skin, bind pose) for game engines such as Unreal and Unity (default `blender,maya,unreal`)
- `--max-influences <MAX_INFLUENCES>`: Most bones that may influence one vertex (1 to 12). glTF writes them in sets of four
- `--engine <ENGINE>`: Set max influences for an engine: unreal (8), unreal12 (12), unity (4) or gltf (4)
- `--prune <PRUNE>`: Drop influences below this weight (such as 0.01) and renormalise
- `--smooth <SMOOTH>`: Skin weight smoothing passes (default `3`)
- `--naming <NAMING>`: Name the exported joints and controls with this convention and keep it in the guides: game (Unreal mannequin), prefix, suffix, unity (Unity Humanoid bone names) or mixamo (mixamorig:Hips and so on). Unset, the guides' own naming is used

### `hive-rig rebuild`

Rebuild a rig from its (edited) guides, reusing the mesh in its .hive.json

```
hive-rig rebuild [OPTIONS] <RIG>
```

- `<RIG>`: The `<name>.hive.json` of the rig to rebuild
- `-g, --guides <GUIDES>`: Guides to build from (default: `<name>.guides.json` next to the rig)
- `-o, --out <OUT>`: Output directory (default: the rig's directory)
- `--keep-weights`: Keep the weights edited by hand (painted, mirrored, smoothed or copied) in the old rig, except where the new guides add a bone that claims the vertex
- `-t, --targets <TARGETS>` (repeatable): What to write besides the .glb: builder scripts for blender, maya and unreal, and fbx for a binary .fbx (mesh, skeleton, skin, bind pose) for game engines such as Unreal and Unity (default `blender,maya,unreal`)
- `--max-influences <MAX_INFLUENCES>`: Most bones that may influence one vertex (1 to 12). glTF writes them in sets of four
- `--engine <ENGINE>`: Set max influences for an engine: unreal (8), unreal12 (12), unity (4) or gltf (4)
- `--prune <PRUNE>`: Drop influences below this weight (such as 0.01) and renormalise
- `--smooth <SMOOTH>`: Skin weight smoothing passes (default `3`)
- `--naming <NAMING>`: Name the exported joints and controls with this convention and keep it in the guides: game (Unreal mannequin), prefix, suffix, unity (Unity Humanoid bone names) or mixamo (mixamorig:Hips and so on). Unset, the guides' own naming is used

### `hive-rig weights`

Skin weight tools for a rig that has been built

```
hive-rig weights <COMMAND>
```

### `hive-rig weights copy`

Copy one rig's weights onto another character's rig with the same or a similar skeleton, in any pose or size, and write the target rig again

```
hive-rig weights copy [OPTIONS] <FROM> <TO>
```

- `<FROM>`: The `<name>.hive.json` to copy from
- `<TO>`: The `<name>.hive.json` to copy onto
- `-o, --out <OUT>`: Output directory (default: the target rig's directory)
- `--max-influences <MAX_INFLUENCES>`: Most bones that may influence one vertex (1 to 12). glTF writes them in sets of four
- `--engine <ENGINE>`: Set max influences for an engine: unreal (8), unreal12 (12), unity (4) or gltf (4)
- `--prune <PRUNE>`: Drop influences below this weight (such as 0.01) and renormalise
- `-t, --targets <TARGETS>` (repeatable): What to write besides the .glb: builder scripts for blender, maya and unreal, and fbx for a binary .fbx (mesh, skeleton, skin, bind pose) for game engines such as Unreal and Unity (default `blender,maya,unreal`)

### `hive-rig weights limit`

Prune small weights and cap the influences per vertex of a built rig, keeping its painted weights. The limits are stored with the guides, so rebuilds keep them

```
hive-rig weights limit [OPTIONS] <RIG>
```

- `<RIG>`: The `<name>.hive.json` of the rig
- `-o, --out <OUT>`: Output directory (default: the rig's directory)
- `--max-influences <MAX_INFLUENCES>`: Most bones that may influence one vertex (1 to 12). glTF writes them in sets of four
- `--engine <ENGINE>`: Set max influences for an engine: unreal (8), unreal12 (12), unity (4) or gltf (4)
- `--prune <PRUNE>`: Drop influences below this weight (such as 0.01) and renormalise
- `-t, --targets <TARGETS>` (repeatable): What to write besides the .glb: builder scripts for blender, maya and unreal, and fbx for a binary .fbx (mesh, skeleton, skin, bind pose) for game engines such as Unreal and Unity (default `blender,maya,unreal`)

### `hive-rig weights normalize`

Make every vertex's weights add up to 1 within the rig's influence limits

```
hive-rig weights normalize [OPTIONS] <RIG>
```

- `<RIG>`: The `<name>.hive.json` of the rig
- `--lock <LOCK>` (repeatable): A bone whose weights stay as they are (repeatable)
- `-o, --out <OUT>`: Output directory (default: the rig's directory)
- `-t, --targets <TARGETS>` (repeatable): What to write besides the .glb: builder scripts for blender, maya and unreal, and fbx for a binary .fbx (mesh, skeleton, skin, bind pose) for game engines such as Unreal and Unity (default `blender,maya,unreal`)

### `hive-rig weights fix-spikes`

Replace every vertex whose weights stand out from its neighbours' (a spike) with their average

```
hive-rig weights fix-spikes [OPTIONS] <RIG>
```

- `<RIG>`: The `<name>.hive.json` of the rig
- `--threshold <THRESHOLD>`: How far a vertex must differ from its neighbours' average, from 0 to 1: 0.5 is a hard edge between two bones, 1 a vertex sharing no bone with them (default `0.5`)
- `--lock <LOCK>` (repeatable): A bone whose weights stay as they are (repeatable)
- `-o, --out <OUT>`: Output directory (default: the rig's directory)
- `-t, --targets <TARGETS>` (repeatable): What to write besides the .glb: builder scripts for blender, maya and unreal, and fbx for a binary .fbx (mesh, skeleton, skin, bind pose) for game engines such as Unreal and Unity (default `blender,maya,unreal`)

### `hive-rig weights fill`

Give every vertex with no weight the weights around it on the surface (what `hive-rig check` flags as "no weight")

```
hive-rig weights fill [OPTIONS] <RIG>
```

- `<RIG>`: The `<name>.hive.json` of the rig
- `--lock <LOCK>` (repeatable): A bone whose weights stay as they are (repeatable)
- `-o, --out <OUT>`: Output directory (default: the rig's directory)
- `-t, --targets <TARGETS>` (repeatable): What to write besides the .glb: builder scripts for blender, maya and unreal, and fbx for a binary .fbx (mesh, skeleton, skin, bind pose) for game engines such as Unreal and Unity (default `blender,maya,unreal`)

### `hive-rig weights auto`

Re-run the automatic weights on the skin of some bones (wherever they have weight now or would get it), keeping the hand edits elsewhere

```
hive-rig weights auto [OPTIONS] <RIG>
```

- `<RIG>`: The `<name>.hive.json` of the rig
- `--bone <BONE>` (repeatable): A bone whose skin gets the automatic weights again (repeatable)
- `--lock <LOCK>` (repeatable): A bone whose weights stay as they are (repeatable)
- `-o, --out <OUT>`: Output directory (default: the rig's directory)
- `-t, --targets <TARGETS>` (repeatable): What to write besides the .glb: builder scripts for blender, maya and unreal, and fbx for a binary .fbx (mesh, skeleton, skin, bind pose) for game engines such as Unreal and Unity (default `blender,maya,unreal`)

### `hive-rig weights import`

Bring weights painted in Blender or Maya back into the rig: the edits file the builder scripts' Send to Hive writes. A rebuild with --keep-weights keeps them. Writes the rig again (the outputs already next to it)

```
hive-rig weights import [OPTIONS] <RIG> <EDITS>
```

- `<RIG>`: The `<name>.hive.json` of the rig
- `<EDITS>`: The edits file (`<name>.edits.json`)
- `-o, --out <OUT>`: Output directory (default: the rig's directory)

### `hive-rig pose`

Pose a built rig's controls, the way the app's Rig stage does, and keep the pose with the rig (the Blender and Maya builders put it in their pose library as "Studio pose"). Writes the rig again

```
hive-rig pose [OPTIONS] <RIG>
```

- `<RIG>`: The `<name>.hive.json` of the rig
- `--set <SET>` (repeatable): A control's channels: CONTROL=tx,ty,tz,rx,ry,rz,sx,sy,sz (offsets from rest in its rest frame, degrees; empty or missing values keep what it has). Repeatable
- `--attr <ATTR>` (repeatable): A control's attribute: CONTROL.NAME=VALUE, such as `arm_l_ik_ctrl.ik_fk=0`. Repeatable
- `--reset`: Start from rest instead of the rig's pose
- `--bones <BONES>`: Also write where the pose puts every bone's head and tail (metres, hive-rig's frame) to this JSON file
- `--readers <READERS>`: Also write what the cone and RBF readers read in the pose to this JSON file: each reader's outputs, every shape's weight, each helper joint's skinning matrix, and the pose as the builders' pose libraries take it (`studio_pose`), for checking a DCC against

### `hive-rig edits`

Turn a skinned mesh another program exported from the rig (an FBX written by Unreal's exporter, say) into the edits file that `weights import` and `shape import` read. Its frame, units and vertex order may differ: the frame is found from the bones it shares with the rig, and each rig vertex takes the weights and shapes of the file's vertex at its place

```
hive-rig edits [OPTIONS] <RIG> <MESH>
```

- `<RIG>`: The `<name>.hive.json` of the rig
- `<MESH>`: The exported mesh (.fbx, binary or ASCII), in the rig's rest pose
- `-o, --out <OUT>`: Where to write the edits (default: `<name>.edits.json` next to the rig)
- `--source <SOURCE>`: The program that wrote the mesh, kept in the file (default `unreal`)
- `--weight-tolerance <WEIGHT_TOLERANCE>`: Weights this close to the rig's count as unchanged (Unreal keeps 8-bit weights unless High Precision Skin Weights is on) (default `0.005`)

### `hive-rig shape`

Blendshapes on a built rig: sculpt corrective shapes in a pose, drive them from a joint's rotation, mirror, split, combine and set weights. Writes the rig again (its .glb, .hive.json and the scripts and .fbx already next to it)

```
hive-rig shape <RIG> <COMMAND>
```

- `<RIG>`: The `<name>.hive.json` of the rig

### `hive-rig shape list`

List the shapes, their weights and what drives them

```
hive-rig shape list
```

### `hive-rig shape sculpt`

Move vertices in a pose and keep the change as (part of) a shape: the offsets are worked back through the skin, so the shape puts the vertices there in that pose

```
hive-rig shape sculpt [OPTIONS] <NAME>
```

- `<NAME>`
- `--pose <POSE>`: The pose to sculpt in: BONE=DEGREES, or BONE=DEGREES@DIRECTION for the way the bone's tip swings (up, down, forward, back, left, right or x,y,z). A limb's middle bone (lowerarm_l, calf_r) bends the way its IK bends without a direction. Leave out to sculpt at rest
- `--at <AT>`: Brush centre (x,y,z in metres, in the posed mesh)
- `--radius <RADIUS>`: Brush radius in metres; vertices fade out toward the edge
- `--offset <OFFSET>`: How far the brush moves the vertices at its centre (x,y,z metres)
- `--from <FROM>`: A sculpted copy of the mesh (same vertices, .obj/.gltf/.glb), posed as `--pose` says (or at rest)
- `--drive`: Also drive the shape from the pose: off at rest, fully on at the pose's angle
- `--inbetween <INBETWEEN>`: Sculpt the shape's in-between at this weight instead of the full shape (add it first with `inbetween`)

### `hive-rig shape drive`

Drive a shape from a bone's rotation (a corrective shape): its weight goes from 0 at `--start` degrees to 1 at `--end` degrees of turn

```
hive-rig shape drive [OPTIONS] <NAME>
```

- `<NAME>`
- `--bone <BONE>`
- `--toward <TOWARD>`: The way the bone's tip swings to turn the shape on (up, down, forward, back, left, right or x,y,z). Default for a limb's middle bone: the way it bends
- `--axis <AXIS>`: The axis the turn is read about (x,y,z, model space, right-handed)
- `--start <START>` (default `0`)
- `--end <END>` (default `90`)

### `hive-rig shape board`

Drive a shape from a face board slider: its weight is how far the slider has gone one way (times --weight). PART is a slider (brow_l, lid_r, corner_l, jaw, ...) and DIR x+, x-, y+ or y- (up, down). With --auto, every undriven shape named like a board move (blink_l, smile_r, jawOpen) is linked instead

```
hive-rig shape board [OPTIONS] [<NAME>] [<PART>] [<DIR>]
```

- `<NAME>`
- `<PART>`
- `<DIR>`
- `--weight <WEIGHT>` (default `1`)
- `--off`: Stop following the board
- `--auto`

### `hive-rig shape undrive`

Stop driving a shape; it keeps its own weight

```
hive-rig shape undrive <NAME>
```

- `<NAME>`

### `hive-rig shape mirror`

Mirror a shape onto the other side (smile_l to smile_r), with its driver

```
hive-rig shape mirror [OPTIONS] <NAME>
```

- `<NAME>`
- `--as <NEW>`: Name of the mirrored shape (default: the side token swapped)

### `hive-rig shape split`

Split a shape into left and right halves (`<name>_l`, `<name>_r`)

```
hive-rig shape split [OPTIONS] <NAME>
```

- `<NAME>`
- `--width <WIDTH>`: Width of the blend across the centre line, in metres (default 2% of the body height)
- `--names <NAMES>` (repeatable): Names of the halves: LEFT,RIGHT

### `hive-rig shape combine`

Add shapes together as a new shape

```
hive-rig shape combine [OPTIONS] [<NAMES>...]
```

- `<NAMES>`: The shapes to add, comma separated
- `--as <NEW>`

### `hive-rig shape weight`

Set the weight a shape sits at (0 off, 1 fully on)

```
hive-rig shape weight <NAME> <VALUE>
```

- `<NAME>`
- `<VALUE>`

### `hive-rig shape remove`

```
hive-rig shape remove <NAME>
```

- `<NAME>`

### `hive-rig shape rename`

```
hive-rig shape rename <NAME> <NEW>
```

- `<NAME>`
- `<NEW>`

### `hive-rig shape inbetween`

Give a shape an in-between target at a weight between 0 and 1 (it starts where the shape puts the vertices at that weight; sculpt it with `sculpt --inbetween`), or take one off with --remove

```
hive-rig shape inbetween [OPTIONS] <NAME> <WEIGHT>
```

- `<NAME>`
- `<WEIGHT>`
- `--remove`

### `hive-rig shape import`

Bring shapes sculpted in Blender or Maya back into the rig, with their in-betweens: the edits file the builder scripts' Send to Hive writes. A shape the rig has keeps its pose reader

```
hive-rig shape import [OPTIONS] <EDITS>
```

- `<EDITS>`: The edits file (`<name>.edits.json`)
- `--only <ONLY>` (repeatable): Only this shape (repeatable)

### `hive-rig shape reader`

Cone and RBF pose readers: weights from where a bone points or from sample poses, driving shapes and helper joints

```
hive-rig shape reader <COMMAND>
```

### `hive-rig shape reader list`

List the readers; with --pose or --studio, also their weights there

```
hive-rig shape reader list [OPTIONS]
```

- `--pose <POSE>`: BONE=DEGREES or BONE=DEGREES@DIRECTION (as `sculpt --pose`)
- `--studio`: The pose saved with the rig (`hive-rig pose`, or posed in the app)

### `hive-rig shape reader cone`

Make or change a cone reader: fully on while BONE points within --angle degrees of its direction, easing off over --falloff more (with --away: off inside, on once the bone swings away in any direction). The direction is --direction, where the bone points in --pose or --studio, or else its rest direction

```
hive-rig shape reader cone [OPTIONS] <NAME>
```

- `<NAME>`
- `--bone <BONE>`
- `--direction <DIRECTION>`: The cone's centre, x,y,z or a word (up, forward, ...), in the parent bone's rest frame
- `--pose <POSE>`: BONE=DEGREES or BONE=DEGREES@DIRECTION (as `sculpt --pose`)
- `--studio`: The pose saved with the rig (`hive-rig pose`, or posed in the app)
- `--angle <ANGLE>` (default `15`)
- `--falloff <FALLOFF>` (default `30`)
- `--away`
- `--shape <SHAPES>` (repeatable): A shape it drives (repeatable; made empty when the rig has none)

### `hive-rig shape reader rbf`

Make an RBF reader on BONE; give it sample poses with `reader pose`

```
hive-rig shape reader rbf [OPTIONS] <NAME>
```

- `<NAME>`
- `--bone <BONE>`
- `--radius <RADIUS>`: Kernel width in degrees (default: from the samples' spacing) (default `0`)
- `--swing`: Tell poses apart by where the bone points, ignoring its twist

### `hive-rig shape reader pose`

Add the reader's bone as it is in a pose as a sample of RBF reader NAME, with --set OUTPUT=VALUE for the shapes and helpers on there (others are off; a shape it doesn't drive yet is added). --remove K takes sample K off instead

```
hive-rig shape reader pose [OPTIONS] <NAME>
```

- `<NAME>`
- `--pose <POSE>`: BONE=DEGREES or BONE=DEGREES@DIRECTION (as `sculpt --pose`)
- `--studio`: The pose saved with the rig (`hive-rig pose`, or posed in the app)
- `--rest`: Take the sample from the rest pose
- `--set <SET>` (repeatable)
- `--remove <REMOVE>`

### `hive-rig shape reader shape`

Drive another shape too (made empty when new), or stop with --remove

```
hive-rig shape reader shape [OPTIONS] <NAME> <SHAPE>
```

- `<NAME>`
- `<SHAPE>`
- `--remove`

### `hive-rig shape reader helper`

Add or change a helper joint the reader moves and scales: skinned like any other joint within --radius, and at full weight moved by --offset (metres, in its parent's frame) and scaled --along and --across its parent bone. New helpers sit at the read bone's head under its parent

```
hive-rig shape reader helper [OPTIONS] <NAME> <HELPER>
```

- `<NAME>`
- `<HELPER>`
- `--parent <PARENT>`
- `--at <AT>`: Where it sits at rest (x,y,z metres)
- `--offset <OFFSET>`
- `--along <ALONG>`
- `--across <ACROSS>`
- `--radius <RADIUS>`
- `--on <ON>` (repeatable): RBF samples (numbers from 1) where a new helper is fully on
- `--remove`

### `hive-rig shape reader mirror`

Mirror a reader onto the other side (mirror its shapes first)

```
hive-rig shape reader mirror [OPTIONS] <NAME>
```

- `<NAME>`
- `--as <NEW>`

### `hive-rig shape reader remove`

```
hive-rig shape reader remove <NAME>
```

- `<NAME>`

### `hive-rig add`

Add a premade component to a guides file

```
hive-rig add [OPTIONS] <GUIDES> <PRESET>
```

- `<GUIDES>`
- `<PRESET>` (one of `fingers`, `tail`, `chain`, `eyes`, `jaw`, `face`, `face_board`, `toes`, `quad_legs`, `tentacle`)
- `--side <SIDE>`: Which hands get fingers (default: both) (one of `left`, `right`)
- `--parent <PARENT>`: Bone to attach a tail or chain to
- `--joints <JOINTS>`: Bones in a tail or chain; for a face or face board, the extra lip joints on each lip between its middle and each corner (0 to 3)
- `--id <ID>`: Component id for a tail or chain, or for quad legs added to a side that has a leg already (`leg2` adds `leg2_l` and `leg2_r` with IK of their own; hang them with --parent)
- `--controls <CONTROLS>`: Curve controls on a tentacle (ends included, default 4)

### `hive-rig spine`

Change the spine: its joint count, and whether a curve between the hips and chest controls drives it (a spline spine) instead of FK

```
hive-rig spine [OPTIONS] <GUIDES>
```

- `<GUIDES>`
- `--joints <JOINTS>`: Spine joints (1 to 16)
- `--spline <SPLINE>`: Drive the spine with a curve (one of `true`, `false`)
- `--controls <CONTROLS>`: Curve controls of a spline spine (ends included, default 3)

### `hive-rig attach`

Hang a component from another bone (its guides stay where they are)

```
hive-rig attach <GUIDES> <ID> <BONE>
```

- `<GUIDES>`
- `<ID>`: Component id
- `<BONE>`: Bone to hang it from

### `hive-rig import`

Import the premade components of another guides file, hung from a bone

```
hive-rig import [OPTIONS] <GUIDES> <FROM>
```

- `<GUIDES>`
- `<FROM>`: The guides file to take components from
- `--parent <PARENT>`: Bone to hang the imported components from

### `hive-rig remove`

Remove a component (and everything attached to it) from a guides file

```
hive-rig remove <GUIDES> <ID>
```

- `<GUIDES>`
- `<ID>`

### `hive-rig components`

List the components in a guides file

```
hive-rig components <GUIDES>
```

- `<GUIDES>`

### `hive-rig check`

Look for likely mistakes before building: guides on the wrong side, limbs bending the wrong way or not at all, joints outside the body or on top of each other, lopsided twins and odd parents. On a built rig, also vertices with no weight and deforming bones with no skin. Exits non-zero only when the guides can't build

```
hive-rig check [OPTIONS] <FILE>
```

- `<FILE>`: A `.guides.json`, or a `.hive.json` (its mesh is checked against, and the guides next to it, which a rebuild would use; and its skin)
- `--mesh <MESH>`: A character mesh (.obj, .gltf, .glb, .fbx) to check the guides against
- `--fix`: Apply the fixes that are mechanical and save the guides file (on a rig, also fill vertices with no weight and write the rig again)
- `--up <UP>`: The file's axis that points up through the character: +x, -x, +y, -y, +z or -z (default: detected from the feet, legs, head and skeleton)
- `--forward <FORWARD>`: The file's axis the character faces along (default: detected from the toes, or for an animal on four legs its head end)

### `hive-rig template`

Save a layout as a template, load one onto a layout, or list them

```
hive-rig template <COMMAND>
```

### `hive-rig template save`

Save the whole layout, or only some premade parts, as `<name>.template.json`

```
hive-rig template save [OPTIONS] <GUIDES> <OUT>
```

- `<GUIDES>`
- `<OUT>`: The template file (`<name>.template.json`), or a folder to save it in
- `--only <ONLY>` (repeatable): Only these components (premade parts such as fingers, a tail or eyes)
- `--name <NAME>`: Template name (default: the file name)
- `--description <DESCRIPTION>`: A line describing the template

### `hive-rig template apply`

Load a template onto a guides file: a whole layout replaces the guides (scaled to this body), parts are added (ids in use get a number)

```
hive-rig template apply [OPTIONS] <GUIDES> <TEMPLATE>
```

- `<GUIDES>`
- `<TEMPLATE>`: A template file, or the name of a built-in or saved template
- `--parent <PARENT>`: Bone to hang parts from whose parent isn't in the template

### `hive-rig template list`

List the built-in templates and the ones in the template folders

```
hive-rig template list
```

### `hive-rig control`

List the controls with their look, or change the look (shape, size, colour) of some of them. The look is kept in the guides file, so every rebuild and every exporter (Blender, Maya, Unreal) gets it

```
hive-rig control [OPTIONS] <GUIDES> [<CONTROLS>...]
```

- `<GUIDES>`
- `<CONTROLS>`: Controls to change, by internal (`hand_l_ik_ctrl`) or exported name. None lists every control
- `--shape <SHAPE>`: The shape, by its guides.json name (circle, gear, curved_arrow...)
- `--custom <CUSTOM>`: A custom shape by name, from the guides or the user's shape library (see `hive-rig shapes`)
- `--import-shape <IMPORT_SHAPE>`: Import a shape (.svg polylines or .json points) into the guides and the shape library, and use it on these controls
- `--line-width <LINE_WIDTH>`: Line thickness, 1 being normal (Blender wire width, Maya line width, Unreal's thin or thick shape)
- `--size <SIZE>`: Radius or half-extent in metres
- `--scale <SCALE>`: Multiply the current size by this
- `--color <COLOR>`: `r,g,b` from 0 to 1, `#rrggbb`, or left, right, centre, root, green, cyan, orange or white
- `--twin`: Also change each control's left/right twin (a side colour swaps sides)
- `--component`: Change every control of each named control's component
- `--reset`: Put the default look back instead

### `hive-rig shapes`

Custom control shapes: the user's shape library and the ones in a guides file. Draw one from points, import one (.svg or .json), export one, or remove one

```
hive-rig shapes <COMMAND>
```

### `hive-rig shapes list`

List the shapes in the user's library and, given a guides file, in it

```
hive-rig shapes list [OPTIONS]
```

- `--guides <GUIDES>`

### `hive-rig shapes draw`

Make a shape from points `x,y x,y ...` on a grid centred on the control

```
hive-rig shapes draw [OPTIONS] <NAME> <POINTS>
```

- `<NAME>`
- `<POINTS>`: Points as `x,y` pairs separated by spaces; `|` starts another line
- `--closed`: Join the last point back to the first
- `--mirror`: Mirror the lines across the up axis (draw one half)
- `--guides <GUIDES>`: Also keep it in this guides file

### `hive-rig shapes import`

Import a shape from an .svg (polylines, polygons, lines, straight paths) or a .json file of points, into the library (and a guides file)

```
hive-rig shapes import [OPTIONS] <FILE>
```

- `<FILE>`
- `--name <NAME>`: The shape's name; the file's name if not given
- `--guides <GUIDES>`

### `hive-rig shapes export`

Write a shape (from the guides if given, else the library) as .svg or .json

```
hive-rig shapes export [OPTIONS] <NAME> <OUT>
```

- `<NAME>`
- `<OUT>`
- `--guides <GUIDES>`

### `hive-rig shapes remove`

Remove a shape from the library, or from a guides file (its controls get their built-in shape back)

```
hive-rig shapes remove [OPTIONS] <NAME>
```

- `<NAME>`
- `--guides <GUIDES>`

### `hive-rig scene`

Several characters in one scene file: make one, add characters and props held across characters, and build and export them together or one at a time

```
hive-rig scene <COMMAND>
```

### `hive-rig scene new`

Make a scene file from characters: `NAME=FILE[@x,y,z]`, where FILE is a character (.glb, .fbx, .obj), a saved rig (.hive.json) or `demo`, `demo:dog`, `demo:bird`... and `@x,y,z` places it in the scene (metres)

```
hive-rig scene new <OUT> <CHARACTERS>...
```

- `<OUT>`
- `<CHARACTERS>`

### `hive-rig scene add`

Add a character to a scene

```
hive-rig scene add <SCENE> <CHARACTER>
```

- `<SCENE>`
- `<CHARACTER>`: `NAME=FILE[@x,y,z]`, as for `scene new`

### `hive-rig scene hold`

Hold a prop of one character in another's hand: `--prop A:CONTROL` (a control of character A, such as its prop socket's) follows `--holder B:BONE`, switchable with a `held` setting on the control

```
hive-rig scene hold [OPTIONS] <SCENE>
```

- `<SCENE>`
- `--prop <PROP>`
- `--holder <HOLDER>`
- `--held <HELD>`: Start held (1) or on its own socket (0) (default `1`)

### `hive-rig scene release`

Stop holding a prop

```
hive-rig scene release [OPTIONS] <SCENE>
```

- `<SCENE>`
- `--prop <PROP>`

### `hive-rig scene list`

List the characters and the held props

```
hive-rig scene list <SCENE>
```

- `<SCENE>`

### `hive-rig scene prop`

Give a character a prop socket: a one-bone chain in a hand whose control carries the prop (and another character can hold)

```
hive-rig scene prop [OPTIONS] <SCENE> <CHARACTER>
```

- `<SCENE>`
- `<CHARACTER>`
- `--hand <HAND>`: The bone it hangs from (default `hand_r`)
- `--id <ID>`: The socket component's id (default `prop_r`)

### `hive-rig scene remove`

Take a character out of the scene (and the holds that name it)

```
hive-rig scene remove <SCENE> <CHARACTER>
```

- `<SCENE>`
- `<CHARACTER>`

### `hive-rig scene build`

Build every character (or `--only` some) and export them with the scene builders `<scene>_blender.py`, `_maya.py` and `_unreal.py`

```
hive-rig scene build [OPTIONS] <SCENE>
```

- `<SCENE>`
- `-o, --out <OUT>`
- `--only <ONLY>` (repeatable): Only these characters (the scene builders then hold the others' last export)
- `-t, --targets <TARGETS>` (repeatable): What to write besides the .glb: builder scripts for blender, maya and unreal, and fbx for a binary .fbx (mesh, skeleton, skin, bind pose) for game engines such as Unreal and Unity (default `blender,maya,unreal`)
- `--max-influences <MAX_INFLUENCES>`: Most bones that may influence one vertex (1 to 12). glTF writes them in sets of four
- `--engine <ENGINE>`: Set max influences for an engine: unreal (8), unreal12 (12), unity (4) or gltf (4)
- `--prune <PRUNE>`: Drop influences below this weight (such as 0.01) and renormalise
- `--smooth <SMOOTH>`: Skin weight smoothing passes (default `3`)
- `--naming <NAMING>`: Name the exported joints and controls with this convention and keep it in the guides: game (Unreal mannequin), prefix, suffix, unity (Unity Humanoid bone names) or mixamo (mixamorig:Hips and so on). Unset, the guides' own naming is used

### `hive-rig naming`

Show or set how a guides file names joints and controls on export

```
hive-rig naming [OPTIONS] <GUIDES> [<PRESET>]
```

- `<GUIDES>`
- `<PRESET>`: game (Unreal mannequin), prefix (L_arm_jnt), suffix (arm_L_JNT), unity (Unity Humanoid: Hips, LeftUpperArm) or mixamo (mixamorig:Hips)
- `--joint <JOINT>`: Joint pattern using {name}, {side} and {desc}
- `--control <CONTROL>`: Control pattern using {name}, {side} and {desc}
- `--left <LEFT>`
- `--right <RIGHT>`
- `--center <CENTER>`: Centre side token (empty to leave it out)

### `hive-rig help-all`

Print the help of every command and subcommand, with every flag

```
hive-rig help-all [OPTIONS]
```

- `--markdown`: As Markdown (the CLI reference in docs/api.md is this output)

<!-- cli-reference:end -->
