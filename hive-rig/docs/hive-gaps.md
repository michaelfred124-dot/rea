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
- **Joints to guides**: a .glb that already has a skeleton (Mixamo, Unreal,
  3ds Max biped, Maya or Blender names) becomes guides, with its eyes, jaw,
  face and fingers. Checked by renaming and editing a rig in real Blender.
- **Weight tools**: mirror weights left to right (or back), smooth one bone's
  weights or all of them, a paint brush in the app, prune, a max-influences
  limit per engine, and copying weights from another character (gap 11).
- **Animator tools** (Blender tab and Maya window): IK to FK and FK to IK
  matching per limb, mirror pose, reset pose, select all controls and space
  switching without a pop. Checked in real Blender; Maya only against stubs.
- **Quadruped hind legs and a dog**: the `quad_leg` component and a built-in
  dog layout and test mesh (`hive-rig demo --quadruped`); see gap 1.
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

## Gaps, ranked by how much they'd help a rigger

| # | Area | What Hive/Zoo offer publicly | What we'd build | Size |
|---|------|------------------------------|-----------------|------|
| 1 | Quadrupeds and creatures | Dog, horse, spider, dinosaur, werewolf templates; quad legs | Done: a `quad_leg` component (thigh, calf, hock and paw; two-bone IK aimed at a hock control, reverse paw roll, FK, stretch) and a dog layout (`demo --quadruped`), checked in real Blender. Next: horse, spider and dinosaur templates, multi-leg sides, an automatic (spring) hock | Medium |
| 2 | Face, next steps | Joint-based face plus face controls on a panel | Done: face joints. Next: a 2D face control board, lip zippers, sticky lips | Medium |
| 3 | Bendy limbs | Bendy/ribbon arms and legs, soft IK | Done: `bendy: true` on an arm or leg puts each segment's skin on four bend joints that follow a curve, arced by a bend control per segment, with a pin control at the elbow or knee; `soft` (0 to 1) on every arm and leg IK eases the reach near full extension and works with stretch. Checked in real Blender; Maya only against stubs; Unreal moves the bend joints onto the curve without turning them and has no soft IK. Next: soft IK in Unreal, bendy hind legs (`quad_leg`), a bend control twist, per-segment bend joint counts | Medium |
| 4 | Spine and tentacles | Spline (IK) spine, tentacles, multi-branch FK | Done: a `spline` component (`hive-rig add <guides> tentacle`; joint and curve-control counts) whose curve controls bend a smooth curve the joints follow, with an FK layer, `stretch` on/off and `volume`, and `spine --spline true` for a spline spine (hips and chest controls end the curve). Spline IK on a hooked curve in Blender (checked in real Blender), ikSplineHandle with advanced twist in Maya, Fit Chain on Curve in Unreal (both only against stubs). Next: twist along the chain in Blender, volume in Unreal, an exact curve in Unreal past four controls, a sliding (offset) attribute, multi-branch FK | Medium |
| 5 | IK/FK and space matching | Snap IK to FK and back without a pop | Done in the Blender tab and Maya window. Next: the same in Unreal (a Control Rig function or editor utility), and matching across a frame range | Small |
| 6 | Animator tools | Mirror poses, select-all-controls, reset pose | Done in Blender and Maya. Next: flip a whole pose (centre controls too), a pose library | Small |
| 7 | Joints to guides, next steps | Turn an existing skeleton into guides | Done for .glb. Next: unknown chains (ears, tails, hair) read back as chain components, and FBX input | Small |
| 8 | FBX and more targets | FBX export; Unity and Houdini use | Done: a hand-written binary FBX 7.4 writer (`--targets fbx`, Export FBX in the app) with mesh, skeleton, every skin influence and a bind pose, checked through Blender's FBX importer. Next: try it in Unreal, Unity and Houdini (not yet done), FBX input, Unity Humanoid bone names, animation takes | Small |
| 9 | Pose-driven correctives | Corrective shapes and helper joints driven by joint angles (Brave Rabbit's weightDriver is the well-known Maya RBF/pose reader) | Our own pose reader: a cone angle between a joint and its rest pose drives a helper joint or shape, written as Blender drivers and Maya nodes | Large |
| 10 | Control shapes | A library of control curve shapes, recolouring | More shapes and per-control colour/size in the app | Small |
| 11 | Skin tools, next steps | Auto skin plus weight tools | Done: mirror, smooth, paint brush (add, subtract, smooth, symmetry, undo; edits kept on rebuild), prune, max influences per engine (glTF writes a second joints/weights set past four), and copy weights between characters by closest surface point with naming-aware bone matching. Next: weight transfer for unmatched extra chains (tails, ears) by position, a flood/set-value tool, locking bones while painting, painting in Blender/Maya round-tripping into the guides | Small |
| 12 | Proximity controls | Controls appear as the mouse comes near (lesterbanks) | Blender-only overlay; Maya has no clean equivalent | Small |
| 13 | Developer API | Scriptable rig building | Already partly there: the CLI and the guides JSON; document it | Small |

## Things we haven't been thinking about

- Validation before build: done (see Already covered). Next: checks on the
  built skin (vertices with no weight, a bone with no skin), and per-studio
  tolerances.
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
