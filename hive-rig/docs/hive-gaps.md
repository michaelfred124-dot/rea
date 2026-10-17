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
- **Weight tools**: mirror weights left to right (or back) and smooth one
  bone's weights or all of them.
- **Quadruped hind legs and a dog**: the `quad_leg` component and a built-in
  dog layout and test mesh (`hive-rig demo --quadruped`); see gap 1.

## Gaps, ranked by how much they'd help a rigger

| # | Area | What Hive/Zoo offer publicly | What we'd build | Size |
|---|------|------------------------------|-----------------|------|
| 1 | Quadrupeds and creatures | Dog, horse, spider, dinosaur, werewolf templates; quad legs | Done: a `quad_leg` component (thigh, calf, hock and paw; two-bone IK aimed at a hock control, reverse paw roll, FK, stretch) and a dog layout (`demo --quadruped`), checked in real Blender. Next: horse, spider and dinosaur templates, multi-leg sides, an automatic (spring) hock | Medium |
| 2 | Face, next steps | Joint-based face plus face controls on a panel | Done: face joints. Next: a 2D face control board, lip zippers, sticky lips | Medium |
| 3 | Bendy limbs | Bendy/ribbon arms and legs, soft IK | Extra mid-limb controls driving the twist bones; soft IK to stop knee pop | Medium |
| 4 | Spine and tentacles | Spline (IK) spine, tentacles, multi-branch FK | A spline-chain component with stretch and volume | Medium |
| 5 | IK/FK and space matching | Snap IK to FK and back without a pop | A match button in the Blender tab and Maya window | Medium |
| 6 | Animator tools | Mirror poses, select-all-controls, reset pose | Pose mirror using our L/R naming, reset to bind | Small |
| 7 | Joints to guides, next steps | Turn an existing skeleton into guides | Done for .glb. Next: unknown chains (ears, tails, hair) read back as chain components, and FBX input | Small |
| 8 | FBX and more targets | FBX export; Unity and Houdini use | FBX writer, or document the glTF route for Unity/Houdini | Medium |
| 9 | Pose-driven correctives | Corrective shapes and helper joints driven by joint angles (Brave Rabbit's weightDriver is the well-known Maya RBF/pose reader) | Our own pose reader: a cone angle between a joint and its rest pose drives a helper joint or shape, written as Blender drivers and Maya nodes | Large |
| 10 | Control shapes | A library of control curve shapes, recolouring | More shapes and per-control colour/size in the app | Small |
| 11 | Skin tools, next steps | Auto skin plus weight tools | Done: mirror and smooth. Next: copy weights between characters, prune tiny weights, a max-influences limit per engine, paint brush | Medium |
| 12 | Proximity controls | Controls appear as the mouse comes near (lesterbanks) | Blender-only overlay; Maya has no clean equivalent | Small |
| 13 | Developer API | Scriptable rig building | Already partly there: the CLI and the guides JSON; document it | Small |

## Things we haven't been thinking about

- Validation before build: warn when a guide crosses the mid-line, an elbow
  bends the wrong way, or a joint sits outside the mesh.
- Rig presets per engine: Unreal mannequin is done; Unity Humanoid and Mixamo
  bone names would let one rig retarget everywhere.
- Saving the rigging window's setups as user templates (Hive calls these
  templates), so a studio can share its own biped.
- Versioning: a rig built today should rebuild the same way next year; store the
  hive-rig version in the guides file and keep old behaviour behind it.
- Multiple characters in one file, and props parented across characters.

## Suggested order

1 (quad leg) and 5 (matching) give the biggest jump for game work; 2 (face) for
film. 6 and 10 are quick wins we could fit in any time.
