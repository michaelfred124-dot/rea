# Skin weights

How a skinned character deforms, how riggers usually get there, and what
hive-rig does at each step.

## Linear blend skinning in one paragraph

Every vertex stores a short list of (bone, weight) pairs that add up to 1. When
the skeleton moves, each bone carries the vertex to where it would be if the
vertex were glued to that bone alone, and the vertex lands on the weighted
average of those positions. Game engines and DCCs all do this the same way
(Unreal, Unity, glTF, Blender's Armature modifier, Maya's skinCluster in its
classic mode), so the weights are the whole of the deformation. Averaging
positions also explains the classic artefacts: a joint bent far loses volume
(the elbow or knee pinches) and a joint twisted far collapses to a "candy
wrapper", because the average of two rotated points lies inside both.

## Automatic weights

Riggers start from automatic weights and paint from there. The published
approaches all answer one question: how near is this vertex to this bone?

* **Envelopes / straight-line distance.** Weight falls off with the distance
  to the bone segment. Fast, but distance through the air counts: the inner
  thigh is close to the other leg, an arm hanging by the ribs is close to the
  spine, and fingers held together share weights.
* **Bone heat** (Baran and Popović, "Automatic Rigging and Animation of 3D
  Characters", SIGGRAPH 2007). Heat diffuses from each bone over the surface
  (a Laplace equation on the mesh), with a bone only heating the vertices it
  can "see" in a straight line. Smooth and well behaved on a single clean,
  closed surface; it needs a connected manifold mesh, so separate pieces
  (eyes, teeth, buttons, clothes), holes and non-manifold edges either fail or
  must be solved piece by piece with visibility guesses.
* **Geodesic voxel binding** (Dionne and de Lasa, "Geodesic Voxel Binding for
  Production Character Meshes", SCA 2013). Fill the inside of the mesh with
  voxels and measure distance from each bone *through the solid*. Air gaps are
  not solid, so weight can't jump across them; overlapping pieces simply merge
  into one solid; holes and non-manifold geometry don't matter to a voxel
  grid. Designed for production meshes, which are rarely one closed surface.

### What hive-rig does

hive-rig uses geodesic voxel binding (`src/voxel.rs`, `skin::compute`), written
from the paper's ideas:

1. **Voxelize** the mesh at 256 voxels along its longest side (about 7 mm on a
   1.8 m character). Every voxel a triangle touches is marked by an exact
   triangle/box overlap test, so the shell has no gaps. Open edge loops (a
   neck hole, eye sockets, a sleeve end) are first closed with a fan of
   triangles so they don't let the outside in; vertices at the same position
   count as one, so UV seams aren't mistaken for holes.
2. **Fill** by flooding the outside from the edge of the grid. Whatever the
   flood can't reach is solid: shells, their insides, and anything enclosed.
3. **Measure** each deforming bone's distance to each vertex through the solid
   (Dijkstra over the 26 neighbours of each voxel, all bones in parallel),
   starting from the voxels the bone's segment passes through. A vertex's
   distance is never less than its straight-line distance to the bone.
4. **Separate pieces.** A solid piece belongs to the bones that pass through
   it: an eyeball to its eye bone, a robot's forearm plate to the forearm. A
   piece no bone passes through (teeth, a loose button, hair cards) takes the
   straight-line distance to every bone and goes to the nearest ones.
5. **Weight** each bone by `1 / (d² + ε)⁴`, `d` being the distance through the
   body as a fraction of the mesh size (so the result doesn't depend on units).
   Face joints and other bones with a `reach` only weight vertices within it,
   fading to nothing at its edge, so a brow joint never pulls the forehead.
   Bones with `deform: false` (the root, a bendy limb's segment bones, whose
   bend joints take the skin) get no weight.

Nothing in this knows about bodies: the same code skins a biped, a dog, a bird,
a spider and a snake. (Earlier versions split a biped into five body regions by
the crotch height and torso width; that failed on A-poses, arms near the body
and anything that isn't a biped.)

Limits worth knowing: limbs pressed together closer than a voxel or two (fingers
held together, thighs touching) merge in the grid, and weight can cross where
they touch; voxelization doesn't see detail smaller than a voxel.

## Smoothing

Raw weights change abruptly where the nearest bone changes, which shows as a
crease when the joint bends. Smoothing averages each vertex's weights with its
neighbours' a few times (Laplacian smoothing, the "smooth weights" brush in
every DCC). hive-rig runs 3 passes at half strength (`--smooth N` changes the
count) over the surface, treating vertices split at UV or normal seams as one,
so smoothing crosses seams and both copies of a seam vertex keep the same
weights (otherwise the mesh would tear open along the seam when posed). The
Properties panel's **Smooth** and the brush's Smooth mode use the same pass.

## Editing weights by hand

Automatic weights get a character most of the way; riggers then fix what they
see while bending the joints. hive-rig's tools, in `src/skin.rs` (the brush) and
`src/skin/edit.rs` (everything else), all keep every vertex adding up to 1 and
within the influence limits after each edit.

* **Setting one bone's weight.** Add, Subtract, Replace and Scale only decide
  the painted bone's new weight `t` on a vertex. The vertex's other bones are
  then scaled together so they add up to `1 - t` (their proportions stay). When
  no other bone holds the vertex, taking weight off the bone hands it to the
  nearest deforming parent. Flood and the weight table set the weight the same
  way.
* **Locks.** A locked bone keeps its weight `l` on every vertex. The painted
  bone can then reach at most `1 - (sum of locked weights)`, and only the
  unlocked bones are rescaled, pruned or dropped by the influence cap (the cap
  drops the weakest unlocked bone first). Edits that blend whole vertices
  (smooth, paste, the automatic weights again) put the locked weights back
  afterwards and rescale the rest into the room left; if nothing unlocked would
  be left to fill it, the vertex isn't changed.
* **Spikes.** A vertex's *spike* is half the summed difference, bone by bone,
  between its weights and the average of its neighbours' (the same measure as
  the "spikes" column below): 0 where it agrees with them, about 0.5 on a hard
  edge between two bones, near 1 for a vertex given to a bone nothing around it
  has. Mend (a brush mode) and Fix spikes (the whole mesh) give a vertex above
  the threshold (0.5 by default) its neighbours' average, computed from the
  weights before the pass so one fix doesn't spread into the next. Unlike
  Smooth, vertices below the threshold aren't touched, so a deliberate soft
  falloff survives. Neighbours are taken over the surface with seam copies
  welded, as for smoothing.
* **Normalize** scales each vertex that doesn't add up to 1 as a whole, then
  applies the limits. Edits in hive-rig never leave such vertices; they come
  from rigs edited elsewhere or by hand.
* **Re-running the automatic weights** on a selection or on one bone's skin
  (every vertex the bone weighs now or would weigh automatically) copies the
  rig's automatic weights (geodesic voxel binding, above) onto just those
  vertices; the hand edits elsewhere stay.
* **Selections** grow and shrink by one ring of surface neighbours. Selecting a
  vertex selects its seam copies too, so an edit never gives two copies of one
  point different weights (which tears the mesh open when posed).
* **Seeing it bend.** The app's Bend slider turns the painted bone at its joint
  through the test pose (linear blend skinning of the rest mesh with that bone
  and its children turned), so painting happens on the deformed skin.

## Influence limits and pruning

Engines cap how many bones may move one vertex: 4 for Unity's usual setting and
glTF's first joints/weights set, 8 for Unreal's default skin cache, 12 with
Unreal's extra influences. Capping keeps the strongest influences and
renormalises the rest to 1; pruning drops influences too small to matter (say
under 0.01) so exports stay clean. hive-rig keeps both in the guides
(`SkinLimits`, `--max-influences`, `--engine`, `--prune`), applies them after
smoothing, and the paint brush keeps the bone being painted even while it is
among the smallest.

## Twist distribution

A forearm turns about its length by up to 180 degrees between elbow and wrist.
With one bone the whole forearm turns rigidly from the elbow and the wrist
collapses (the candy wrapper). Rigs spread the twist over helper bones along
the limb, each turning a share of the full twist, and give each a slice of the
skin.

In hive-rig each limb segment has a twist bone (`upperarm_twist_01_l`,
`lowerarm_twist_01_l`, `thigh_twist_01_l`, `calf_twist_01_l`) at its middle.
The segment bone's skinned length stops at its twist bone
(`Skeleton::skin_segment`), so the near half of the limb follows the segment
and the far half the twist bone. Upper twists counter half of their own bone's
twist and lower twists carry half of the hand's or foot's. Bendy limbs replace
the segment's skin with evenly spaced bend joints, each carrying a growing share
of the twist.

## Corrective shapes

Weights alone can't fix every pose: an elbow bent past 90 degrees still loses
volume and a shoulder raised overhead still folds. Production rigs add
*corrective shapes* (blendshapes sculpted for a pose, faded in by the joint
angle through a pose reader such as an RBF or cone reader) or *helper joints*
that push skin out as the joint bends. hive-rig builds angle-driven
correctives (see the README's blendshapes section and `docs/hive-gaps.md`,
item 9); the bent-joint thickness measured below is pure linear blend skinning,
without them.

**Sculpting through the skin.** A shape's offsets are stored at rest, but a
corrective is sculpted in its pose. Each sculpt step takes the moves made on
the posed mesh and works them back through linear blend skinning: a vertex's
rest-space offset is its posed move turned by the inverse of its blended bone
rotation, so in that pose the shape puts the vertex where the brush left it.
The brushes all work on the picked target *fully on*, in the current pose:

* **Grab** moves the vertices under the brush by the pointer's drag, faded by
  the falloff `(1 - (d/r)^2)^2` (the weight brush's).
* **Smooth** moves each vertex toward the average of its surface neighbours
  (seam copies welded) by `strength x falloff`.
* **Relax** does the same with the move's component along the vertex normal
  taken off, so vertices even out along the surface without the surface
  shrinking.
* **Inflate** moves each vertex along its normal (averaged over the welded
  seam copies, so a seam doesn't split) by a tenth of the radius at full
  strength; inverted, it deflates.
* **Erase** scales the target's offsets under the brush toward zero (no
  change from rest) by `strength x falloff`.

**In-betweens.** A shape with targets at weights `p_1 < ... < p_k < 1` mixes
them piecewise linearly: at weight `w` between `p_i` and `p_(i+1)` it is
`(1 - t) T_i + t T_(i+1)` with `t = (w - p_i) / (p_(i+1) - p_i)`, and below
`p_1` it fades the first target in from rest. Each target's contribution is a
hat function of `w`, which is how the DCCs get it: Blender keys read the
shape's slider through a hat expression, Unreal's Control Rig builds the hats
from two clamped remaps and a min per target, and Maya's blendShape does the
same blend natively. In flat formats (glTF, FBX) each target becomes its own
morph target named `<shape>_ib<percent>`; hive-rig folds them back on import.

## Measured on real characters

`tests/blender_skin_eval.py` rebuilds a rig's mesh, bones and weights in real
Blender (an Armature modifier), bends each elbow and knee 90 degrees and
measures the result; with the source glTF it also maps the artist's weights onto
the rig's bones and measures them the same way. `tests/fetch_skin_models.sh`
downloads the public models at pinned commits. Each model was re-rigged with
`hive-rig rig model.glb` (joints to guides).

Columns: **leg leak** and **arm leak** are the mean weight a vertex takes from
a limb it isn't on (the other leg; arm bones on the torso and torso bones on the
arm); **pieces** is the share of loose-piece vertices whose strongest bone is
neither the artist's nor one across a joint from it; **thickness** is the
smallest bent elbow or knee's mean skin distance to its bones over the rest
distance (below 1 the joint thins); **spikes** is the 99th percentile of how far
a vertex's weights are from its neighbours' average; **vs artist** is half the
summed weight difference from the artist's weights per vertex (0 is identical).

| Model (vertices) | Weights | Leg leak | Arm leak | Pieces | Thickness | Spikes p99 | vs artist |
|---|---|---|---|---|---|---|---|
| Michelle (16,340) | before | 0.287 | 0.022 | 0.220 | 0.945 | 0.341 | 0.462 |
| | **after** | **0.010** | 0.022 | **0.010** | 0.945 | **0.139** | **0.222** |
| | artist | 0.011 | 0.008 | 0 | 0.964 | 0.406 | 0 |
| Soldier (7,434) | before | 0.910 | 0.257 | 0.491 | 0.912 | 0.378 | 0.795 |
| | **after** | **0.023** | **0.021** | **0.030** | 0.882 | **0.208** | **0.437** |
| | artist | 0.007 | 0.008 | 0 | 0.865 | 0.580 | 0 |
| Xbot (28,374) | before | 0.004 | 0.005 | 0.015 | 0.918 | 0.186 | 0.426 |
| | **after** | 0.001 | 0.004 | 0.012 | **0.940** | 0.164 | **0.354** |
| | artist | 0 | 0 | 0 | 0.981 | 0.071 | 0 |
| RobotExpressive (7,214) | before | – | 0 | 0 | 0.954 | 1.012 | 0.143 |
| | **after** | – | 0 | 0 | 0.956 | **0.396** | **0.116** |
| | artist | – | 0 | 0 | 0.956 | 0.292 | 0 |
| CesiumMan (3,273) | before | 0 | 0.163 | – | 0.937 | 0.343 | 0.676 |
| | after | 0.001 | 0.153 | – | 0.941 | **0.192** | 0.683 |
| | artist | 0.002 | 0.012 | – | 0.983 | 0.492 | 0 |
| RiggedFigure (370) | before | 0 | 0.181 | – | 1.088 | 1.113 | 0.525 |
| | after | 0.005 | 0.142 | – | 1.083 | **0.529** | 0.484 |
| BrainStem (34,159) | before | 0.025 | 0.206 | 0.318 | 0.877 | 0.322 | 0.672 |
| | after | 0 | 0.229 | 0.293 | 0.890 | **0.159** | 0.639 |
| Demo, A-pose (2,676) | before | 0.002 | 0.003 | – | 0.900 | 0.276 | – |
| | after | 0.002 | 0 | – | 0.927 | 0.166 | – |

Michelle, Soldier, Xbot and RobotExpressive are three.js example models (the
first three from Mixamo, RobotExpressive CC0); the rest are Khronos glTF sample
models. When this table was measured, Soldier (which faces -Z) had its left
and right guides on the wrong sides: the old body regions then fought the
bones, the distance through the body doesn't care. CesiumMan, RiggedFigure and
BrainStem have joint names hive-rig doesn't recognise, so their guides came
from body detection and sat off the artist's joints; their arm leak and artist
difference measure those guides more than the weights. Both are fixed since:
see [Orientation and joint reading](#orientation-and-joint-reading).

The automatic weights take about a second for a 30,000-vertex character in a
release build on 4 cores (Xbot 0.9 s, Michelle 0.8 s, BrainStem 0.7 s for the
whole `hive-rig rig` run; about 2.3 s on one core).

## Orientation and joint reading

The same eight characters, rigged with `hive-rig rig model.glb` before and after
characters were turned into hive-rig's frame (Y up, facing +Z, left on +X) on
import and joints were read by the skeleton's shape (see the README's
[Input requirements](../README.md#input-requirements) and
[Bring your own skeleton](../README.md#bring-your-own-skeleton-joints-to-guides)).
Both runs were measured with the current `blender_skin_eval.py`, which places a
skinned source where its skeleton puts it at rest and turns it into the rig's
frame.

Columns: **template** is the layout chosen; **up** whether the character stands
upright in the rig; **left** how many of the shoulder, elbow, wrist, hip, knee
and ankle guides that have an artist's joint are on the character's own side
(the left on +X); **guide to joint** is the mean and largest distance from each
body guide to the artist's joint for it, as a share of the character's height
(the expected joints are listed by hand per model); the skin columns are as
above, and **vs artist** is the mean weight difference and the share of
vertices with the same strongest bone. "–" is not measured: the old rig's
vertices didn't line up with the source as a viewer shows it (so there is no
artist reference), or the rig has no such bone.

| Model | | template | up | left | guide to joint | leg leak | arm leak | thickness | vs artist |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| Michelle | before | biped | **lying** | 12/12 | 0 / 0 | 0.001 | 0 | 0.945 | – |
| | after | biped | yes | 12/12 | 0 / 0 | 0.011 | 0.020 | 0.939 | 0.229 / 0.883 |
| Soldier | before | biped | yes | **0/12** | 0 / 0 | 0.023 | 0.021 | 0.882 | 0.283 / 0.707 |
| | after | biped | yes | 12/12 | 0 / 0 | 0.023 | 0.021 | 0.931 | 0.278 / 0.705 |
| Xbot | before | biped | yes | 12/12 | 0 / 0 | 0.001 | 0 | 0.940 | – |
| | after | biped | yes | 12/12 | 0 / 0 | 0.001 | 0.004 | 0.940 | 0.354 / 0.719 |
| RobotExpressive | before | biped | yes | 10/10 | 0 / 0 | 0 | 0.002 | 0.956 | – |
| | after | biped | yes | 10/10 | 0.003 / 0.033 | – | 0 | 0.875 | 0.103 / 0.988 |
| CesiumMan | before | biped | yes | 12/12 | 0.066 / 0.195 | 0.001 | 0.153 | 0.941 | 0.683 / 0.306 |
| | after | biped | yes | 12/12 | 0 / 0 | 0 | 0.054 | 0.838 | 0.485 / 0.558 |
| RiggedFigure | before | biped | yes | 12/12 | 0.071 / 0.182 | 0.005 | 0.142 | 1.083 | 0.484 / 0.489 |
| | after | biped | yes | 12/12 | 0 / 0 | 0.004 | 0.077 | 0.780 | 0.460 / 0.514 |
| BrainStem | before | biped | yes | 8/8 | 0.048 / 0.132 | 0 | 0.229 | 0.890 | 0.639 / 0.348 |
| | after | biped | yes | 8/8 | 0 / 0 | 0 | 0.053 | 0.812 | 0.401 / 0.675 |
| Fox | before | **biped** | yes | 12/12 | 0.216 / 0.455 | 0 | 0.032 | – | 0.572 / 0.636 |
| | after | dog | yes | 12/12 | 0 / 0 | 0.006 | – | 0.774 | 0.521 / 0.502 |

* **Michelle** and **Xbot**: the old reader applied the skinned mesh node's
  own transform, which glTF ignores for skinned meshes. Michelle came out lying
  down, and neither lined up with the source as a viewer shows it. Vertices are
  now placed by their joints at rest.
* **Soldier** faces -Z. Its guides were read onto the right joints by name, but
  with its left on -X: every left guide was flagged across the mid-line, and
  elbows and knees were judged for a character facing the other way. It is now
  turned to face +Z; the bend test, which folds elbows forward and knees back,
  now folds them the way they bend (thickness 0.88 to 0.93).
* **CesiumMan**, **RiggedFigure** and **BrainStem** have numbered or no joint
  names. Their guides now sit on the artist's joints. Arm leak falls (0.153 to
  0.054, 0.142 to 0.077, 0.229 to 0.053) and the weights get closer to the
  artist's. Bent-joint thickness falls too, to 0.78–0.84: the joints are now
  where the artist put them, and CesiumMan's own weights score 0.816 on the
  same bend.
* **RobotExpressive** is rigid parts hung from joints, posed at rest with the
  arms down. Only the hands are skinned, and they are bound in a T-pose. The
  old reader left the hands at their bind pose, out at the sides and away from
  the arms. Now they hang at the ends of the arms, and the wrist is read where
  the finger joints fan out. Its feet hang from the root, as in an IK rig, and
  are joined to the legs ending above them. The pelvis guide sits on the joint
  where the legs and spine meet (`Body`), 3% of the height above the artist's
  `Hips`.
* **Fox** gets the dog layout, with its guides on its joints (hips, shoulders,
  spine, neck, head, tail and four legs).

With the skeleton left out of detection, the mesh alone gave the same
orientation and template for all eight.
