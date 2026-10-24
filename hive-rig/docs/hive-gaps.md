# Hive Rig: what Hive and Zoo Tools do that we don't yet

Sources are public only: Create 3D Characters' Hive product and docs pages, the
digitalproduction.com and lesterbanks.com write-ups, and general rigging practice.
No Hive or Zoo Tools code or assets were opened. "Hive has it" means a public page
describes the feature; the details of how it behaves are our reading, not tested.

## Already covered

Biped from guides, IK/FK with blend, stretch, reverse foot, space switching, twist
bones, build and rebuild from editable guides, keep weights on rebuild, premade
parts (eyes with look-at, jaw, fingers, tail, ears, ponytail, prop, chain),
mirroring, symmetry, naming conventions (Game, Side first, Side last, custom
patterns) applied to every export, and now:

- **Face joints**: brows, eyelids, cheeks, nose and lips as joints with
  move-and-rotate controls. Joints rather than blendshapes, as Maya face rigs
  usually are, so the face reaches Blender and Unreal in the same .glb.
- **Joints to guides**: a .glb or binary .fbx that already has a skeleton
  (Mixamo, Unreal, 3ds Max biped, Maya or Blender names) becomes guides, with
  its eyes, jaw, face and fingers, and its unknown parts (ears, tails, hair)
  as chain components. Checked by renaming and editing a rig in real Blender.
- **FBX input**: a hand-written binary FBX 7.x reader (no SDK): meshes,
  normals, UV sets, colours, materials and textures, blend shapes, bones and
  skin, placed with the full FBX transform and axis/unit settings; every
  command that takes a .glb takes an .fbx. Checked against Blender-made FBX
  files of public models (gap 8).
- **Weight tools**: mirror weights left to right (or back), smooth one bone's
  weights or all of them, a paint brush in the app, prune, a max-influences
  limit per engine, and copying weights from another character (gap 11).
- **Animator tools** (Blender tab and Maya window): IK to FK and FK to IK
  matching per limb, mirror pose, reset pose, select all controls and space
  switching without a pop. Checked in real Blender; Maya only against stubs.
- **Quadruped hind legs and a dog**: the `quad_leg` component and a built-in
  dog layout and test mesh (`hive-rig demo --quadruped`); see gap 1.
- **Bird, spider and snake**: built-in layouts, demo bodies and templates
  (`hive-rig demo --creature bird|spider|snake`) made from the existing
  components: arm wings and `quad_leg` legs with hocks for the bird, eight FK
  leg chains for the spider, a spline spine and spline tail for the snake.
  Checked in real Blender (animator tools); Maya and Unreal only against stubs.
- **Checks before building**: warnings for a guide across the mid-line, an
  elbow, knee or hock bending the wrong way, a straight limb, a joint outside
  the mesh, joints on top of each other, lopsided twins, off-centre spine and
  head guides, and odd parents (a leg under the head), each with a suggestion
  and, where it's mechanical, a fix. In the app (a Checks card, viewport
  markers, a warning count on Build) and the CLI (`hive-rig check`, and
  warnings from `rig` and `rebuild`).
- **Versioning**: guides and rig files carry the hive-rig version that wrote
  them; the guides version pins the builder rules the file doesn't spell out,
  and files from a newer hive-rig open with a warning.
- **Spline chains**: a `tentacle` (spline) component and a spline spine option;
  see gap 4.
- **Bendy limbs and soft IK**: bend and pin controls on arms and legs, and a
  `soft` attribute on their IK; see gap 3.
- **FBX output**: `<name>.fbx` (binary 7.4) for game engines, with the mesh,
  the skeleton in the rig's naming convention, all skin influences and a bind
  pose, written from the public binary layout without an SDK; see gap 8.
- **Mesh fidelity**: a real character keeps its objects and material pieces,
  normals, every UV set, tangents, vertex colours, glTF and MTL materials with
  their textures (written once into `<name>_textures/`, named from the rig
  file) and its morph targets, through import, `.hive.json`, `.glb`, `.fbx` and
  the Blender, Maya and Unreal builders. Checked in real Blender (and through
  Blender's FBX importer) on public Khronos sample models.
- **Blendshapes and correctives**: shapes with weights, sculpted at rest or in a
  test pose (worked back through the skin), mirrored with their drivers,
  split, combined, renamed, with in-between targets; in the app (a searchable
  shape list with sliders, grab, smooth, relax, inflate and erase brushes on any
  target, Corrective from pose) and the CLI (`hive-rig shape`). Correctives are
  driven by angle pose readers: Blender shape-key drivers (checked in real
  Blender), a Maya blendShape with native in-betweens and a node network,
  Unreal Control Rig curves blended through the in-betweens, with float
  controls for undriven shapes (both only against stubs); see gap 9. Weights
  and shapes edited in Blender or Maya come back with **Send to Hive**
  (`weights import`, `shape import`).

## Gaps, ranked by how much they'd help a rigger

| # | Area | What Hive/Zoo offer publicly | What we'd build | Size |
|---|------|------------------------------|-----------------|------|
| 1 | Quadrupeds and creatures | Dog, horse, spider, dinosaur, werewolf templates; quad legs | Done: a `quad_leg` component (thigh, calf, hock and paw; two-bone IK aimed at a hock control, reverse paw roll, FK, stretch) and a dog layout (`demo --quadruped`), checked in real Blender. Also done: bird (arm wings, `quad_leg` legs with hocks, tail), spider (eight four-bone FK leg chains) and snake (spline spine and tail) layouts, demo bodies and built-in templates (`demo --creature`). Next: horse and dinosaur templates, IK legs beyond one arm and one leg a side (the spider's legs are FK only), wing feather chains, an automatic (spring) hock | Medium |
| 2 | Face, next steps | Joint-based face plus face controls on a panel | Done: face joints. Next: a 2D face control board, lip zippers, sticky lips | Medium |
| 3 | Bendy limbs | Bendy/ribbon arms and legs, soft IK | Done: `bendy: true` on an arm or leg puts each segment's skin on four bend joints that follow a curve, arced by a bend control per segment, with a pin control at the elbow or knee; `soft` (0 to 1) on every arm and leg IK eases the reach near full extension and works with stretch. Checked in real Blender; Maya only against stubs; Unreal moves the bend joints onto the curve without turning them; soft IK in Unreal only against stubs. Next: bendy hind legs (`quad_leg`), a bend control twist, per-segment bend joint counts | Medium |
| 4 | Spine and tentacles | Spline (IK) spine, tentacles, multi-branch FK | Done: a `spline` component (`hive-rig add <guides> tentacle`; joint and curve-control counts) whose curve controls bend a smooth curve the joints follow, with an FK layer, `stretch` on/off and `volume`, and `spine --spline true` for a spline spine (hips and chest controls end the curve). Spline IK on a hooked curve in Blender (checked in real Blender), ikSplineHandle with advanced twist in Maya, Fit Chain on Curve in Unreal (both only against stubs). Volume in Unreal (only against stubs). Next: twist along the chain in Blender, an exact curve in Unreal past four controls, a sliding (offset) attribute, multi-branch FK | Medium |
| 5 | IK/FK and space matching | Snap IK to FK and back without a pop | Done in the Blender tab and Maya window, with matching across the playback range (keyed per frame). Unreal: IK/FK matching on the Sequencer's Control Rig track, per frame or over the playback range, from a Hive Rig editor menu or the Output Log (only against stubs). Next: try it in Unreal, mirror and reset pose in Unreal | Small |
| 6 | Animator tools | Mirror poses, select-all-controls, reset pose | Done in Blender and Maya, and now flip a whole pose (twins swap, centre controls mirror onto themselves) and a pose library (save, apply, apply mirrored, delete; kept in the scene) in Blender and Maya, and both in Unreal on the Sequencer's Control Rig track (a Hive Poses menu; poses in `<name>.poses.json`). Checked in real Blender on the demo, face, dog and every creature; Maya and Unreal only against stubs (Unreal's flip maths checked with real turns in the stub run). Next: try the Unreal tools in Unreal, mirror and reset pose in Unreal, partial-strength (blended) pose apply, sharing poses between programs | Small |
| 7 | Joints to guides, next steps | Turn an existing skeleton into guides | Done for .glb and binary .fbx, with unknown chains (ears, tails, hair, side branches) read back as chain components hung from the bone they grow from. Next: ASCII FBX, reading a chain's spline (tentacle) or bendy setup back, and recognising helper joints by what they drive rather than by name | Small |
| 8 | FBX and more targets | FBX export; Unity and Houdini use | Done: a hand-written binary FBX 7.4 writer (`--targets fbx`, Export FBX in the app) with mesh, skeleton, every skin influence and a bind pose, checked through Blender's FBX importer. Also done: objects, UV sets, vertex colours, materials with texture files and blend shapes, checked through Blender's FBX importer. Also done: FBX input (binary 7.x, 32- and 64-bit records, compressed arrays; meshes, materials, textures, shapes, skeleton and skin), checked on public models turned into FBX by Blender against their .glb and Blender's importer. Next: try it in Unreal, Unity and Houdini (not yet done), ASCII FBX input, Unity Humanoid bone names, animation takes, embedded textures on export | Small |
| 9 | Pose-driven correctives | Corrective shapes and helper joints driven by joint angles (Brave Rabbit's weightDriver is the well-known Maya RBF/pose reader) | Done: angle pose readers (one bone's turn about an axis, start to end degrees) driving corrective shapes sculpted in the pose, in Blender, Maya and Unreal; in-between targets in every target (native in Maya, a slider with hat-function keys in Blender, blended curves in Unreal, `<shape>_ib50` morph targets in glTF and FBX); undriven shapes as keyable float controls in Unreal; shapes sculpted in Blender or Maya sent back into the rig. Next: cone readers (a swing of any direction), RBF readers over several poses, readers driving helper joints, trying the Maya and Unreal setups in the real applications, a Pose Asset alternative in Unreal | Medium |
| 10 | Control shapes | A library of control curve shapes, recolouring | Done: 23 shapes defined once (`src/controls.rs`) and drawn as the same lines in the app, Blender and Maya, with Unreal library look-alikes; per-control shape, size and colour in the app (shape grid, size, colour picker and swatches; with symmetry the twin follows, a side colour swapping sides; one button for the whole component) and the CLI (`hive-rig control`, with `--twin`, `--component`, `--scale`, `--reset`), kept in the guides so every rebuild and exporter gets them. Checked in real Blender; Maya and Unreal only against stubs. Next: check the Unreal look-alikes in Unreal, custom shapes drawn or imported by the user, line thickness | Small |
| 11 | Skin tools, next steps | Auto skin plus weight tools | Done: mirror, smooth, paint brush (add, subtract, smooth, symmetry, undo; edits kept on rebuild), prune, max influences per engine (glTF writes a second joints/weights set past four), and copy weights between characters by closest surface point with naming-aware bone matching, and extra chains left unmatched by name (tails, ears with other names or joint counts) matched by position. Also done: locks, flood, a weight table, fix spikes, a searchable bone list with an all-bones colour view, a vertex inspector, explicit mirror either way, copy weights from a saved rig in the app, a brush preview of its falloff and strength, one mesh tool at a time, and weights painted in Blender or Maya sent back with **Send to Hive** (`hive-rig weights import`; Blender checked in real Blender, Maya only against stubs). Next: a send-back tool in Unreal | Small |
| 12 | Proximity controls | Controls appear as the mouse comes near (lesterbanks) | Blender-only overlay; Maya has no clean equivalent | Small |
| 13 | Developer API | Scriptable rig building | Already partly there: the CLI and the guides JSON; document it | Small |

## Things we haven't been thinking about

- Validation before build: done (see Already covered), and checks on the
  built skin: vertices with no weight (filled from the weights around them)
  and deforming bones with no skin (automatic weights again), in the app's
  Checks card and the CLI (`check`, `weights fill`). Next: per-studio
  tolerances, and skin checks for weights that pull a vertex the wrong way
  (a bone far from the vertex it moves).
- Rig presets per engine: Unreal mannequin is done; Unity Humanoid and Mixamo
  bone names would let one rig retarget everywhere.
- Saving the rigging window's setups as user templates: done (rig templates).
- Versioning: done for the guides and rig files (see Already covered). Next:
  a way to move an old layout onto newer rules on purpose, once a release
  changes one; templates still refuse a newer template version.
- Multiple characters in one file, and props parented across characters.

## Suggested order

1 (quad leg) gives the biggest jump for game work now that matching is in; 2
(face) for film. 10 is a quick win we could fit in any time.
