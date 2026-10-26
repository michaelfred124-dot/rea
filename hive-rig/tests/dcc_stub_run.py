"""Run a generated Maya or Unreal builder against stub modules.

Catches Python-level mistakes (typos, wrong names, bad data access) in the
generated scripts without needing the DCC installed. It does not prove the DCC
API calls behave as intended; that needs the real application.

Usage: python3 dcc_stub_run.py maya|unreal <script.py> [--guides] [--send]

With --guides (Maya), also runs the guide tools, which call the hive-rig build
that generated the script and rewrite files next to it. With --send, also runs
Send to Hive, which calls that hive-rig (Unreal: needs the rig's .fbx, from
`--targets fbx`, to stand in for the mesh Unreal exports).
"""

import runpy
import sys
import types
from unittest import mock


class Vec:
    def __init__(self, x=0.0, y=0.0, z=0.0):
        self.x, self.y, self.z = x, y, z

    def __add__(self, o):
        return Vec(self.x + o.x, self.y + o.y, self.z + o.z)

    def __sub__(self, o):
        return Vec(self.x - o.x, self.y - o.y, self.z - o.z)

    def __mul__(self, k):
        return Vec(self.x * k, self.y * k, self.z * k)

    def __xor__(self, o):
        return Vec(self.y * o.z - self.z * o.y, self.z * o.x - self.x * o.z, self.x * o.y - self.y * o.x)

    def __truediv__(self, k):
        return Vec(self.x / k, self.y / k, self.z / k)

    def length(self):
        return (self.x ** 2 + self.y ** 2 + self.z ** 2) ** 0.5

    def normal(self):
        n = self.length() or 1.0
        return Vec(self.x / n, self.y / n, self.z / n)


N_VERTS = [0]
SCRIPT = [""]
CV_POSITIONS = {}  # "curveShape.cv[i]" -> world position set by the Maya builder
TIME = [7.0]  # Maya's current frame
BLEND_SHAPES = []  # blendShape nodes the Maya builder made
EXPRESSIONS = {}  # Maya expression name -> its text


def stub_maya():
    calls = []
    counter = [0]
    attrs = {}  # "node.attr" -> value, for the string attributes the guide tools use
    positions = {}
    nodes = set()
    aliases = {}  # node -> [(attr, alias)], from aliasAttr
    blend_shapes = BLEND_SHAPES

    def node(*args, **kwargs):
        counter[0] += 1
        name = kwargs.get("name", "node%d" % counter[0])
        nodes.add(name)
        return name

    cmds = types.ModuleType("maya.cmds")

    def generic(name):
        def fn(*args, **kwargs):
            calls.append(name)
            if name in ("listRelatives",):
                if kwargs.get("allDescendents"):
                    return sorted(nodes)
                return ["parent_or_shape"]
            if name in ("skinCluster", "ikHandle", "orientConstraint", "aimConstraint", "parentConstraint",
                        "pointConstraint", "poleVectorConstraint"):
                if kwargs.get("query") and kwargs.get("weightAliasList"):
                    targets = [a for a in args if isinstance(a, str)]
                    return ["t%d_W%d" % (i, i) for i in range(max(2, len(targets)))]
                if kwargs.get("query") and kwargs.get("name"):
                    return name + "_constraint"
                calls.append((name, args))
                return [kwargs.get("name", "%s%d" % (name, len(calls)))]
            if name == "xform":
                if kwargs.get("q") and (kwargs.get("matrix") or kwargs.get("m")):
                    x, y, z = positions.get(args[0], [0.0, 0.0, 0.0])
                    return [1.0, 0, 0, 0, 0, 1.0, 0, 0, 0, 0, 1.0, 0, x, y, z, 1.0]
                if kwargs.get("q"):
                    return list(positions.get(args[0], [0.0, 0.0, 0.0]))
                if "t" in kwargs:
                    positions[args[0]] = list(kwargs["t"])
                if "translation" in kwargs and kwargs.get("worldSpace"):
                    CV_POSITIONS[args[0]] = list(kwargs["translation"])
                return None
            if name == "getAttr":
                if kwargs.get("multiIndices"):
                    # The indices set under this multi attribute.
                    prefix = args[0] + "["
                    found = {int(k[len(prefix):].split("]", 1)[0]) for k in attrs if k.startswith(prefix)}
                    return sorted(found)
                return attrs.get(args[0], 10.0)
            if name == "aliasAttr":
                if kwargs.get("query"):
                    out = []
                    for plug, alias in aliases.get(args[0], []):
                        out += [alias, plug]
                    return out
                alias, plug = args[0], args[1]
                node_name, attr = plug.split(".", 1)
                aliases.setdefault(node_name, []).append((attr, alias))
                return None
            if name == "listConnections":
                return []
            if name == "connectAttr":
                calls.append(("connectAttr", args))
                return None
            if name == "setAttr":
                if kwargs.get("type") in ("pointArray", "componentList"):
                    calls.append(("setAttr_" + kwargs["type"], args[0]))
                    attrs[args[0]] = list(args[2:])
                elif len(args) == 2 and isinstance(args[1], (int, float)) and ".weight[" in args[0]:
                    attrs[args[0]] = args[1]
                elif len(args) == 2 and args[0].endswith(".lineWidth"):
                    attrs[args[0]] = args[1]
                if kwargs.get("type") == "string":
                    attrs[args[0]] = args[1]
                return None
            if name == "addAttr":
                attrs.setdefault("%s.%s" % (args[0], kwargs["longName"]), kwargs.get("defaultValue"))
                return None
            if name == "attributeQuery":
                return "%s.%s" % (kwargs["node"], args[0]) in attrs
            if name == "objExists":
                return args[0] in nodes
            if name == "namespace":
                calls.append(("namespace", tuple(sorted(kwargs.items()))))
                if kwargs.get("exists"):
                    return False
                return None
            if name == "delete":
                for n in args:
                    nodes.discard(n)
                    for k in [k for k in attrs if k.startswith(n + ".")]:
                        del attrs[k]
                if args and args[0].endswith("_guides_grp"):
                    for n in [n for n in nodes if n.startswith("GDE_")]:
                        nodes.discard(n)
                return None
            if name == "ls":
                if args and isinstance(args[0], str) and args[0].endswith("*"):
                    return sorted(n for n in nodes if n.startswith(args[0][:-1]))
                if kwargs.get("type") == "skinCluster":
                    return ["skinCluster1"]
                if kwargs.get("type") == "blendShape":
                    return list(blend_shapes)
                return list(args[0]) if args and isinstance(args[0], list) else []
            if name == "spaceLocator":
                return [node(**kwargs)]
            if name == "parent":
                return [args[0]]
            if name == "polyEvaluate":
                return N_VERTS[0]
            if name == "about":
                return True
            if name == "rename":
                nodes.add(args[1])
                return args[1]
            if name in ("group", "curve", "createNode", "shadingNode"):
                return node(*args, **kwargs)
            if name == "sets" and kwargs.get("empty"):
                return node(*args, **kwargs)
            if name == "blendShape":
                calls.append((name, args))
                made = node(*args, **kwargs)
                blend_shapes.append(made)
                return [made]
            if name == "expression":
                calls.append(("expression", kwargs.get("name", "")))
                EXPRESSIONS[kwargs.get("name", "")] = kwargs.get("string", "")
                return None
            if name == "playbackOptions":
                return 1.0 if kwargs.get("minTime") else 3.0
            if name == "currentTime":
                if kwargs.get("q") or kwargs.get("query"):
                    return TIME[0]
                TIME[0] = args[0]
                calls.append(("currentTime", args[0]))
                return None
            if name == "setKeyframe":
                calls.append(("setKeyframe", args[0], TIME[0]))
                return None
            return None

        return fn

    for n in ["about", "group", "rename", "parent", "listRelatives", "sets", "polySoftEdge", "createNode", "xform",
              "joint", "setAttr", "skinCluster", "curve", "matchTransform", "move", "parentConstraint",
              "orientConstraint", "aimConstraint", "pointConstraint", "addAttr", "makeIdentity", "ikHandle", "poleVectorConstraint", "connectAttr",
              "select", "file", "getAttr", "expression", "attributeQuery", "objExists", "delete", "ls",
              "spaceLocator", "listHistory", "polyEvaluate", "warning", "autoKeyframe", "setKeyframe",
              "window", "deleteUI", "columnLayout", "rowLayout", "setParent", "button", "text", "separator",
              "checkBox", "showWindow", "confirmDialog", "skinPercent", "pointConstraint",
              "shadingNode", "uvLink", "blendShape", "aliasAttr", "loadPlugin", "currentTime",
              "playbackOptions", "textScrollList", "promptDialog", "listConnections", "transformLimits", "namespace"]:
        setattr(cmds, n, generic(n))

    om = types.ModuleType("maya.api.OpenMaya")
    om.MPoint = lambda *a: a
    om.MPointArray = list
    om.MVector = Vec
    om.MColor = lambda c: tuple(c)
    om.MIntArray = list
    om.MDoubleArray = lambda n, v: [v] * n
    om.MFn = mock.MagicMock()
    om.MFnDagNode = mock.MagicMock()
    om.MFnMesh = mock.MagicMock()
    om.MFnSingleIndexedComponent = mock.MagicMock()

    class Sel:
        def __init__(self):
            self.items = []

        def add(self, x):
            self.items.append(x)

        def getDependNode(self, i):
            return self.items[i]

        def getDagPath(self, i):
            return self.items[i]

    om.MSelectionList = Sel

    oma = types.ModuleType("maya.api.OpenMayaAnim")

    class Skin:
        def __init__(self, _):
            pass

        def influenceObjects(self):
            import json

            rig = json.loads(open(SCRIPT[0]).read().split('r"""', 1)[1].split('"""', 1)[0])
            names = [b["name"] for b in rig["skeleton"]["bones"] if b["deform"]]
            return [mock.Mock(partialPathName=mock.Mock(return_value=n)) for n in names]

        def setWeights(self, *a):
            calls.append("setWeights")

        def getWeights(self, path, comp):
            n = len(self.influenceObjects())
            weights = [0.0] * (N_VERTS[0] * n)
            for v in range(N_VERTS[0]):
                weights[v * n + v % n] = 1.0
            return weights, n

    oma.MFnSkinCluster = Skin

    maya = types.ModuleType("maya")
    api = types.ModuleType("maya.api")
    maya.cmds, maya.api, api.OpenMaya, api.OpenMayaAnim = cmds, api, om, oma
    sys.modules.update({"maya": maya, "maya.cmds": cmds, "maya.api": api,
                        "maya.api.OpenMaya": om, "maya.api.OpenMayaAnim": oma})
    return calls


def stub_unreal():
    unreal = mock.MagicMock()
    unreal.Vector = Vec
    unreal.SkeletalMesh = type("SkeletalMesh", (), {})
    unreal.load_asset = lambda path: unreal.SkeletalMesh()
    unreal.AssetImportTask.return_value.imported_object_paths = ["/Game/HiveRig/mesh"]

    class Q:
        x = y = z = 0.0
        w = 1.0

        def rotator(self):
            return self

        def unrotate_vector(self, v):
            return v

        def rotate_vector(self, v):
            return v

    class T:
        # Translation only: enough for the builder's placements and the
        # animator tools' arithmetic.
        def __init__(self, location=None, scale=None, rotation=None):
            self.translation = location or Vec()
            self.rotation = Q()

        def make_relative(self, other):
            return T(self.translation - other.translation)

        def inverse(self):
            return T(self.translation * -1.0)

        def __mul__(self, other):
            return T(self.translation + other.translation)

        def inverse_transform_direction(self, v):
            return v

    unreal.Transform = T
    blueprint = unreal.ControlRigBlueprintFactory.create_control_rig_from_skeletal_mesh_or_skeleton.return_value
    positions = {}

    def global_transform(key, initial=True):
        name = getattr(key, "name", "")
        return T(positions.setdefault(str(name), Vec(len(positions), 1.0, 0.5 * len(positions))))

    def set_global_transform(key, transform, initial=False, affect_children=True):
        positions[str(getattr(key, "name", ""))] = transform.translation

    blueprint.hierarchy.get_global_transform.side_effect = global_transform
    blueprint.hierarchy.set_global_transform.side_effect = set_global_transform
    unreal.RigElementKey.side_effect = lambda type=None, name="": types.SimpleNamespace(type=type, name=name)
    vm = blueprint.get_controller_by_name.return_value
    vm.get_graph.return_value.get_nodes.return_value = []
    # New nulls and controls come back as their keys.
    hc = blueprint.get_hierarchy_controller.return_value
    hc.add_null.side_effect = lambda name, *a, **k: types.SimpleNamespace(type="Null", name=name)
    hc.add_control.side_effect = lambda name, *a, **k: types.SimpleNamespace(type="Control", name=name)
    # The animator tools: an open Level Sequence (frame 7, playback 1 to 3) that
    # animates this Control Rig, whose hierarchy is the blueprint's.
    editor = unreal.LevelSequenceEditorBlueprintLibrary
    editor.get_current_time.return_value = 7
    seq = editor.get_current_level_sequence.return_value
    seq.get_playback_start.return_value = 1
    seq.get_playback_end.return_value = 4
    proxy = mock.MagicMock()
    proxy.control_rig.get_hierarchy.return_value = blueprint.hierarchy
    unreal.ControlRigSequencerLibrary.get_control_rigs.return_value = [proxy]
    unreal.RigHierarchy.get_float_from_control_value.return_value = 0.5
    unreal.RigHierarchy.get_vector_from_control_value.return_value = Vec()
    unreal.RigHierarchy.get_euler_transform_from_control_value.return_value = types.SimpleNamespace(
        location=Vec(0.0, 1.0, 2.0), rotation=types.SimpleNamespace(roll=10.0, pitch=0.0, yaw=-5.0), scale=Vec(1.0, 1.0, 1.0))
    unreal.FrameNumber = int
    sys.modules["unreal"] = unreal
    return vm, blueprint.get_hierarchy_controller.return_value


def run_scene(kind, script):
    """A scene builder: every character's own script run in turn, then the holds."""
    import json
    import os

    data = json.loads(open(script).read().split('r"""', 1)[1].split('"""', 1)[0])
    if kind == "maya":
        calls = stub_maya()
        sys.argv = [script, "--save", "/dev/null"]
    elif kind == "unreal":
        vm, hc = stub_unreal()
        # Each character's import leaves its Skeletal Mesh in its folder.
        sys.modules["unreal"].EditorAssetLibrary.list_assets.return_value = ["/Game/HiveRig/mesh"]
        sys.argv = [script]
    else:
        raise SystemExit("unknown kind " + kind)
    ns = runpy.run_path(script, run_name="hive_scene_stub")
    live = ns["build"].__globals__
    load = live["load_character"]
    loaded = []

    def load_one(c):
        # The stubs read the character's own rig data (weights, vertex count).
        path = os.path.join(live["HERE"], c["script"])
        SCRIPT[0] = path
        if kind == "maya":
            N_VERTS[0] = len(json.loads(open(path).read().split('r"""', 1)[1].split('"""', 1)[0])["mesh"]["positions"])
        loaded.append(c["name"])
        return load(c)

    live["load_character"] = load_one
    live["main"]()
    names = [c["name"] for c in data["characters"]]
    assert loaded == names, loaded
    if kind == "maya":
        added = [dict(c[1]).get("add") for c in calls if isinstance(c, tuple) and c[0] == "namespace"]
        assert [a for a in added if a] == names, added
        assert calls.count("setWeights") >= len(names), calls.count("setWeights")
        cmds = live["cmds"]
        for h in data["holds"]:
            ctrl = "%s:%s" % (h["prop_character"], h["prop_control"])
            joint = "%s:%s" % (h["holder_character"], h["holder_bone"])
            assert ("parentConstraint", (joint, ctrl + "_held")) in calls, (joint, ctrl)
            assert cmds.getAttr(ctrl + ".held") == h["held"], ctrl
            assert any(c[0] == "connectAttr" and c[1][0] == ctrl + ".held" for c in calls if isinstance(c, tuple))
    else:
        unreal = sys.modules["unreal"]
        assert vm.add_link.call_count > 20 * len(names)
        sockets = [c.args[1] for c in unreal.SkeletalMeshSocket.return_value.set_editor_property.call_args_list
                   if c.args[0] == "socket_name"]
        want = ["held_%s_%s" % (h["prop_character"], h["prop_control"]) for h in data["holds"]]
        assert sockets == want, sockets
        spawned = unreal.get_editor_subsystem.return_value.spawn_actor_from_object.call_args_list
        assert [(c.args[1].x, c.args[1].y, c.args[1].z) for c in spawned] == [
            (o[0] * 100.0, o[2] * 100.0, o[1] * 100.0) for o in (c["offset"] for c in data["characters"])], spawned
    print("%s scene builder ran %d characters and %d holds against stubs: ok" % (kind, len(names), len(data["holds"])))


if __name__ == "__main__":
    import os

    sys_args = sys.argv[3:]
    kind, script = sys.argv[1], sys.argv[2]
    SCRIPT[0] = script
    if '\nSCENE = json.loads(r"""' in open(script).read():
        run_scene(kind, script)
        sys.exit(0)
    if kind == "maya":
        import json

        N_VERTS[0] = len(json.loads(open(script).read().split('r"""', 1)[1].split('"""', 1)[0])["mesh"]["positions"])
        calls = stub_maya()
        sys.argv = [script, "--save", "/dev/null"]  # batch: no window
        ns = runpy.run_path(script, run_name="__main__")
        # IK handles for the limbs and spline chains; a spider's FK legs have none.
        rig = json.loads(open(script).read().split('r"""', 1)[1].split('"""', 1)[0])
        has_ik = bool(rig["controls"]["ik_chains"] or rig["controls"].get("splines"))
        assert "setWeights" in calls and ("ikHandle" in calls) == has_ik, calls
        # Spline chains: every curve point gets its weights, the last joint follows the end.
        splines = json.loads(open(script).read().split('r"""', 1)[1].split('"""', 1)[0])["controls"].get("splines", [])
        assert calls.count("skinPercent") == sum(len(s["points"]) for s in splines), calls.count("skinPercent")
        ends = [s["bones"][-1] for s in splines]
        assert all(any(c[0] == "pointConstraint" and c[1][-1] == e for c in calls if isinstance(c, tuple)) for e in ends)
        # Every CV of every control curve lands on hive-rig's outline (in cm).
        for c in rig["controls"]["controls"]:
            for i, line in enumerate(c["outline"]):
                shape = c["name"] + "Shape" + (str(i) if i else "")
                for j, p in enumerate(line):
                    got = CV_POSITIONS.get("%s.cv[%d]" % (shape, j))
                    assert got is not None and max(abs(g - 100.0 * w) for g, w in zip(got, p)) < 1e-6, (shape, j)
            # A control's own line width reaches every curve shape of it.
            if c.get("line_width") is not None:
                for i in range(len(c["outline"])):
                    shape = c["name"] + "Shape" + (str(i) if i else "")
                    assert ns["cmds"].getAttr(shape + ".lineWidth") == c["line_width"], shape
        # Animator tools: IK/FK matching, mirror, reset, select, space switch, and the window.
        anim = json.loads(ns["cmds"].getAttr(ns["TOP"] + ".hiveAnim"))
        # Limbs and left/right pairs where the rig has them (a snake has neither).
        assert len(anim["limbs"]) == len(rig["controls"]["ik_chains"]), sorted(anim)
        assert len(anim["mirror"]) == len(rig["controls"].get("mirror_pairs", [])), sorted(anim)
        assert anim["spaces"], sorted(anim)
        for limb in anim["limbs"]:
            # Three-segment legs carry their hock joint and FK control in order.
            segments = 4 if limb["hock"] else 3
            assert len(limb["joints"]) == len(limb["fk"]) == len(limb["fk_joints"]) == segments, limb["name"]
            ns["ik_to_fk"](limb["name"])
            ns["fk_to_ik"](limb["name"])
            # Across the playback range (1 to 3 here): every frame visited and
            # keyed whatever auto key says, then back to the frame we were on.
            for to, keyed in (("fk", limb["fk"][0]), ("ik", limb["ik"])):
                start = len(calls)
                assert ns["match_range"](limb["name"], to) == 3
                made = [c for c in calls[start:] if isinstance(c, tuple)]
                assert [c[1] for c in made if c[0] == "currentTime"] == [1, 2, 3, 7.0], made
                frames = {c[2] for c in made if c[0] == "setKeyframe" and c[1] == keyed}
                assert frames == {1, 2, 3}, (limb["name"], to, frames)
        # Soft IK on every two-bone limb; bendy limbs constrain each bend joint to its curve.
        for chain in rig["controls"]["ik_chains"]:
            ik = chain["ik_control"]
            assert ns["cmds"].attributeQuery("soft", node=ik) != bool(chain.get("hock")), chain["name"]
            if chain.get("hock"):
                # Spring hock: a `spring` attribute at its default, a helper chain
                # solved to the goal, and the hock control's group turned between
                # the goal and the helper's upper joint by it.
                n = chain["name"]
                assert ns["cmds"].getAttr(ik + ".spring") == chain["hock"].get("default_spring", 0.0), n
                assert ("orientConstraint", (n + "_spring_keep", n + "_spring_carry", n + "_spring_grp")) in calls, n
                assert ("poleVectorConstraint", (chain["pole_control"], n + "_spring_ikHandle")) in calls, n
                assert ("connectAttr", (ik + ".spring", n + "_spring_reverse.inputX")) in calls, n
                assert anim["defaults"][ik]["spring"] == chain["hock"].get("default_spring", 0.0), n
            for seg in ("upper", "lower") if chain.get("bendy") else ():
                for joint in chain["bendy"][seg]["joints"]:
                    assert ("parentConstraint", (joint + "_bend_twist", joint)) in calls, joint
        assert ns["mirror_pose"](None, "left") == len(anim["mirror"])
        if anim["mirror"]:
            assert ns["mirror_pose"]([anim["mirror"][0][1]], "right") == 1
        # Flip: every control, twins swapped and centre controls onto themselves,
        # mirrored against bind frames every control has.
        assert sorted(anim["bind"]) == sorted(anim["controls"])
        assert ns["flip_pose"]() == len(anim["controls"])
        if anim["mirror"]:
            assert ns["flip_pose"]([anim["mirror"][0][0]]) == 2
        # A pose posed in hive-rig is in the library from the start.
        if rig.get("studio_pose"):
            assert "Studio pose" in ns["pose_library"](), sorted(ns["pose_library"]())
            stored = ns["pose_library"]()["Studio pose"]
            assert sorted(stored) == sorted(anim["controls"]), len(stored)
            ns["delete_pose"]("Studio pose")
            print("maya studio pose in the pose library against stubs: ok")
        # Pose library: save, apply, apply mirrored, selected only, delete.
        assert ns["save_pose"]("stub pose") == len(anim["controls"])
        assert sorted(ns["pose_library"]()) == ["stub pose"]
        assert ns["apply_pose"]("stub pose") == len(anim["controls"])
        assert ns["apply_pose"]("stub pose", None, True) == len(anim["controls"])
        assert ns["apply_pose"]("stub pose", [anim["controls"][0]]) == 1
        ns["delete_pose"]("stub pose")
        assert ns["pose_library"]() == {}
        assert ns["reset_pose"]() == len(anim["controls"])
        assert ns["select_controls"]() == len(anim["controls"])
        ns["switch_space"](next(iter(anim["spaces"])), 1)
        ns["show_window"]()
        # A row of match buttons per limb.
        assert ("rowLayout" in calls or not anim["limbs"]) and "confirmDialog" not in calls
        assert "textScrollList" in calls, "the window has the pose library"
        print("maya animator tools ran against stubs: ok")
        # Blend shapes: every shape lands on a blendShape target with its
        # offsets and vertices; every corrective gets a reader network.
        shapes = rig["mesh"].get("shapes", [])
        targets = [c for c in calls if isinstance(c, tuple) and c[0] == "setAttr_pointArray"]
        comps = [c for c in calls if isinstance(c, tuple) and c[0] == "setAttr_componentList"]
        assert len(targets) == len(comps) >= len(shapes), (len(targets), len(shapes))
        readers = [s for s in shapes if s.get("driver")]
        exprs = [c for c in calls if isinstance(c, tuple) and c[0] == "expression" and c[1].endswith("_reader_expr")]
        assert len(exprs) >= len(readers), (len(exprs), len(readers))
        objects = len({p["object"] for p in rig["mesh"].get("parts", [])}) or 1
        assert calls.count("setWeights") == objects, (calls.count("setWeights"), objects)
        if shapes:
            print("maya blend shapes ran against stubs: %d shapes, %d readers: ok" % (len(shapes), len(readers)))
        # In-betweens are their own target items (5000 + 1000 * weight).
        for s in shapes:
            for b in s.get("inbetweens", []):
                item = "inputTargetItem[%d]" % (5000 + int(round(b["weight"] * 1000)))
                assert any(item in c[1] for c in targets), (s["name"], item)
        # Send to Hive reads back what the builder put there: every shape with
        # its in-betweens and offsets, and each vertex's weights.
        if objects == 1:
            edits = ns["gather_edits"]()
            assert edits["vertex_count"] == N_VERTS[0] and len(edits["weights"]) == N_VERTS[0]
            assert all(len(w) == 1 for w in edits["weights"]), edits["weights"][:3]
            back = {s["name"]: s for s in edits["shapes"]}
            assert sorted(back) == sorted(s["name"] for s in shapes), (sorted(back), [s["name"] for s in shapes])
            for s in shapes:
                got = back[s["name"]]
                pairs = [(s["vertices"], s["offsets"], got["vertices"], got["offsets"])]
                assert [b["weight"] for b in got["inbetweens"]] == [b["weight"] for b in s.get("inbetweens", [])], s["name"]
                pairs += [(a["vertices"], a["offsets"], b["vertices"], b["offsets"])
                          for a, b in zip(s.get("inbetweens", []), got["inbetweens"])]
                for va, oa, vb, ob in pairs:
                    assert va == vb, s["name"]
                    assert all(abs(x - y) < 1e-6 for p, q in zip(oa, ob) for x, y in zip(p, q)), s["name"]
                if not s.get("driver"):
                    assert abs(got["weight"] - s.get("weight", 0.0)) < 1e-6, (s["name"], got.get("weight"))
            print("maya send to hive read back %d shapes and %d vertices' weights against stubs: ok"
                  % (len(shapes), len(edits["weights"])))
            if os.path.isfile(ns["HIVE_EXE"]) and "--send" in sys_args:
                said = ns["send_edits"](False, True)
                assert "Imported shapes" in said or not shapes, said
                print("maya send to hive ran hive-rig shape import: ok")
        board = rig["controls"].get("face_board")
        if board:
            # The face board's expression, run here: each slider's amounts, the
            # helpers' moves and turns, the lip seal's weights and the shapes
            # that follow a slider, from the sliders' translates.
            import re

            text = EXPRESSIONS[ns["NAME"] + "_face_board_expr"]
            node_attr = re.compile(r"(?<![\w.$])([A-Za-z_]\w*)\.([A-Za-z]\w*(?:\[\d+\])?)")

            class Plugs(dict):
                def __missing__(self, k):
                    return 1.0 if k.endswith(".scaleX") else 0.0

            def run_expression(plugs):
                env = {"min": min, "max": max, "clamp": lambda lo, hi, v: min(max(v, lo), hi)}
                out = {}
                for line in text.split("\n"):
                    line = line.strip().rstrip(";")
                    if line.startswith("float "):
                        line = line[len("float "):]
                    lhs, rhs = line.split(" = ", 1)
                    rhs = node_attr.sub(lambda m: "P[%r]" % (m.group(1) + "." + m.group(2)), rhs)
                    rhs = rhs.replace("$", "v_")
                    env["P"] = Plugs(plugs)
                    value = eval(rhs, env)
                    if lhs.startswith("$"):
                        env["v_" + lhs[1:]] = value
                    else:
                        out[lhs] = value
                return out

            sliders = {s_["part"]: s_ for s_ in board["sliders"]}

            def at(values, gaps=None):
                plugs = dict(gaps or {})
                for part, (x, y) in values.items():
                    s_ = sliders[part]
                    plugs[s_["control"] + ".translateX"] = x * s_["travel"] * 100.0
                    plugs[s_["control"] + ".translateY"] = y * s_["travel"] * 100.0
                return run_expression(plugs)

            def amount(v, d):
                return max(v[0 if d[0] == "x" else 1] * (1.0 if d[1] == "+" else -1.0), 0.0)

            # At rest nothing moves; posed, every helper moves and turns by its
            # drives, sliders held to their tracks.
            assert all(abs(v - (1.0 if ".t0_W0" in k else 0.0)) < 1e-12 for k, v in at({}).items())
            pose = {"jaw": (0.0, -3.0), "brow_l": (-1.0, 1.0), "corner_r": (0.6, 0.8), "lid_l": (0.0, -0.5)}
            got = at(pose)
            held = {"jaw": (0.0, -1.0), "brow_l": (-1.0, 1.0), "corner_r": (0.6, 0.8), "lid_l": (0.0, -0.5)}
            checked = 0
            for o in board["offsets"]:
                for attr, key, k in (("translate", "translate", 100.0), ("rotate", "rotate", 1.0)):
                    for i, axis in enumerate("XYZ"):
                        want = sum(d.get(key, [0, 0, 0])[i] * k * amount(held.get(d["part"], (0.0, 0.0)), d["dir"])
                                   for d in o["drives"])
                        plug = "%s_board.%s%s" % (o["control"], attr, axis)
                        assert abs(got.get(plug, 0.0) - want) < 1e-6, (plug, got.get(plug), want)
                        checked += abs(want) > 1e-9
            assert checked > 10, checked
            made = {c[1][-1] for c in calls if isinstance(c, tuple) and c[0] == "pointConstraint"}
            seal = board.get("seal") or {"pairs": []}
            for p in seal["pairs"]:
                assert p["upper"] in made and p["lower"] in made, p
            if seal["pairs"]:
                weights = [k for k in got if ".t1_W1" in k]
                # A zipper all the way seals every pair; at rest none.
                assert len(weights) == 2 * len(seal["pairs"]), sorted(got)
                assert all(abs(got[k]) < 1e-12 for k in weights)
                gaps = {p["upper"] + "_seal_gap.distance": 100.0 * (p["rest_gap"] + 0.03) for p in seal["pairs"]}
                zipped = at({"jaw": (0.0, -1.0), "zip_r": (1.0, 0.0)}, gaps)
                assert all(abs(zipped[k] - 1.0) < 1e-9 for k in weights), zipped
                # Sticky: the middle stays shut while its gap is small, lets go wide open.
                middle = min(seal["pairs"], key=lambda p: abs(p["along"] - 0.5))
                plug = next(k for k in weights if k.startswith(middle["upper"]))
                small = {middle["upper"] + "_seal_gap.distance": 100.0 * (middle["rest_gap"] + 0.1 * seal["release"])}
                assert abs(at({"sticky": (1.0, 0.0)}, small)[plug] - 1.0) < 1e-9
                wide = {middle["upper"] + "_seal_gap.distance": 100.0 * (middle["rest_gap"] + 2.0 * seal["release"])}
                assert abs(at({"sticky": (1.0, 0.0)}, wide)[plug]) < 1e-9
            # Shapes that follow a slider.
            linked = [s_ for s_ in shapes if s_.get("board")]
            for s_ in linked:
                link = s_["board"]
                v = [0.0, 0.0]
                v[0 if link["dir"][0] == "x" else 1] = 0.5 * (1.0 if link["dir"][1] == "+" else -1.0)
                w = [val for k, val in at({link["part"]: tuple(v)}).items() if ".weight[" in k]
                assert w and all(abs(x - min(0.5 * link.get("weight", 1.0), 1.0)) < 1e-9 for x in w), (s_["name"], w)
            assert calls.count("transformLimits") == len(board["sliders"])
            print("maya face board ran against stubs: %d helpers, %d lip pairs, %d linked shapes: ok"
                  % (len(board["offsets"]), len(seal["pairs"]), len(linked)))
        if os.path.isfile(ns["HIVE_EXE"]) and "--guides" in sys_args:
            # Guide tools: edit, mirror, add a component, rebuild keeping weights.
            ns["show_guides"]()
            guides = sum(len(c["guides"]) for c in rig["guides"]["components"])
            assert len(ns["guide_nodes"]()) == guides, (len(ns["guide_nodes"]()), guides)
            ns["mirror_guides"]()
            out = ns["change_components"]("add", "chain", "--parent", "head", "--joints", "2", "--id", "stub_ear")
            assert "stub_ear" in out, out
            calls.clear()
            ns["rebuild"](True)
            objects = len({p["object"] for p in rig["mesh"].get("parts", [])}) or 1
            assert calls.count("setWeights") == 2 * objects, calls.count("setWeights")
            assert not ns["guide_nodes"]()
            print("maya guide tools ran against stubs: ok")
    elif kind == "unreal":
        import json

        vm, hc = stub_unreal()
        vm_curves = hc.add_curve
        sys.argv = [script]  # Output Log `py "<script>"`: build
        ns = runpy.run_path(script, run_name="__main__")
        assert vm.add_link.call_count > 20
        # Thin lines (below 1) pick the library's thin shapes.
        for c in json.loads(open(script).read().split('r"""', 1)[1].split('"""', 1)[0])["controls"]:
            if c.get("line_width") is not None and c["line_width"] < 1.0:
                assert ns["shape_for"](c["shape"], c["line_width"]).endswith("_Thin"), c["name"]
        # Spline chains: two Fit Chain on Curve units (keep length, then stretch) each.
        splines = json.loads(open(script).read().split('r"""', 1)[1].split('"""', 1)[0]).get("splines", [])
        units = [c.args[0] for c in vm.add_unit_node_from_struct_path.call_args_list]
        assert units.count("/Script/ControlRig.RigUnit_FitChainToCurvePerItem") == 2 * len(splines), units
        # Bendy limbs: every bend joint is placed on its curve after the solve.
        rig = json.loads(open(script).read().split('r"""', 1)[1].split('"""', 1)[0])
        made = {c.args[3] for c in vm.add_unit_node_from_struct_path.call_args_list}
        for chain in rig["ik_chains"]:
            for seg in ("upper", "lower") if chain.get("bendy") else ():
                for joint in chain["bendy"][seg]["joints"]:
                    assert "Bend_" + joint in made, joint
        # Corrective shapes: a curve per driven shape, set from its pose reader.
        all_shapes = rig.get("shapes", [])
        driven = [s["name"] for s in all_shapes if s.get("driver")]
        for name in driven:
            assert "Reader_%s_weight" % name in made, name
        # Every shape's curves: one per morph target (in-betweens first), set
        # from its reader or, undriven, from its `<shape>_shape` float control.
        expected = []
        for s in all_shapes:
            expected += ["%s_ib%d" % (s["name"], round(w * 100)) for w in s.get("inbetweens", [])] + [s["name"]]
            if len(s.get("inbetweens", [])):
                for w in s["inbetweens"]:
                    assert "Blend_%s_ib%d" % (s["name"], round(w * 100)) in made, s["name"]
        for target in expected:
            assert "Curve_" + target in made, target
        curves = [c.args[0] for c in vm_curves.call_args_list]
        assert sorted(curves) == sorted(expected), (curves, expected)
        undriven = [s["name"] + "_shape" for s in all_shapes if not s.get("driver") and not s.get("board")]
        shape_floats = {str(c.args[0]) for c in hc.add_control.call_args_list}
        assert all(u in shape_floats for u in undriven), (undriven, sorted(shape_floats)[:5])
        if all_shapes:
            print("unreal shape curves ran against stubs: %d curves, %d readers, %d sliders: ok"
                  % (len(expected), len(driven), len(undriven)))
        # The face board: every slider's value both ways, a null under every
        # face control it moves, set from its drives, the lip seal on every
        # sealed joint, and the shapes that follow a slider.
        board = rig.get("face_board")
        if board:
            for sl in board["sliders"]:
                assert "BoardValue_%s_x" % sl["part"] in made and "BoardHeld_%s_y" % sl["part"] in made, sl["part"]
            nulls = {str(c.args[0]) for c in hc.add_null.call_args_list}
            for o in board["offsets"]:
                assert o["control"] + "_board" in nulls, o["control"]
                moves = any(any(abs(x) > 1e-9 for x in d.get("translate", [0, 0, 0])) for d in o["drives"])
                turns = any(any(abs(x) > 1e-9 for x in d.get("rotate", [0, 0, 0])) for d in o["drives"])
                assert ("Board_" + o["control"] in made) == moves and ("BoardTurn_" + o["control"] in made) == turns, o
            pairs = (board.get("seal") or {}).get("pairs", [])
            for pair in pairs:
                assert "Seal_" + pair["upper"] in made and "Seal_" + pair["lower"] in made, pair
            linked = [s["name"] for s in all_shapes if s.get("board")]
            for name in linked:
                assert "BoardShape_" + name in made and "Curve_" + name in made, name
            print("unreal face board ran against stubs: %d sliders, %d nulls, %d lip pairs, %d linked shapes: ok"
                  % (len(board["sliders"]), len(board["offsets"]), len(pairs), len(linked)))
        # Soft IK and stretch on every limb (a soft control on two-bone limbs);
        # volume on every spline joint on the curve.
        floats = {c.args[0] for c in hc.add_control.call_args_list}
        for chain in rig["ik_chains"]:
            n = chain["name"]
            assert "SoftAim_" + n in made and "Stretch_" + chain["lower"] in made, n
            assert (n + "_soft" in floats) != bool(chain.get("hock")), n
            if chain.get("hock"):
                # Spring hock: a `<limb>_spring` float at its default, the helper
                # chain solved before the leg's own solve, and the hock control
                # hung from the null that turns with it.
                assert n + "_spring" in floats and n + "_spring" in ns["ANIM"]["floats"], n
                spring_default = [c.args[3] for c in hc.add_control.call_args_list if c.args[0] == n + "_spring"]
                assert len(spring_default) == 1, n
                order = [c.args[3] for c in vm.add_unit_node_from_struct_path.call_args_list]
                assert order.index("SpringIK_" + n) < order.index("Spring_" + n) < order.index("IK_" + n), n
                nulls = {c.args[0]: c.args[1] for c in hc.add_null.call_args_list}
                assert nulls[n + "_spring_carry"].name == n + "_spring_upper", n
                hock_parent = [c.args[1] for c in hc.add_control.call_args_list if c.args[0] == chain["hock"]["control"]]
                assert [p.name for p in hock_parent] == [n + "_spring_follow"], (n, hock_parent)
        for sp in splines:
            assert sp["name"] + "_volume" in floats, sp["name"]
            for b in sp["bones"][:-1]:
                assert "Volume_" + b in made, b
        # Animator tools: IK/FK matching both ways on the current frame and over
        # the playback range, keyed on each frame, and a menu entry for each.
        lib = sys.modules["unreal"].ControlRigSequencerLibrary
        for chain in rig["ik_chains"]:
            n = chain["name"]
            for to, keyed in (("fk", chain["fk_controls"][0]), ("ik", chain["ik_control"])):
                for argv, frames in (([n], {7}), ([n, "--range"], {1, 2, 3})):
                    lib.reset_mock()
                    assert ns["main"](["%s_to_%s" % ("ik" if to == "fk" else "fk", to)] + argv) == len(frames)
                    loaded = {c.args[3] for c in lib.get_local_control_rig_euler_transform.call_args_list}
                    keys = {c.args[3] for c in lib.set_local_control_rig_euler_transform.call_args_list
                            if c.args[2] == keyed}
                    blend = {(c.args[3], c.args[4]) for c in lib.set_local_control_rig_float.call_args_list
                             if c.args[2] == n + "_ik_fk"}
                    assert loaded == keys == frames, (n, to, argv, loaded, keys)
                    assert blend == {(f, 0.5) for f in frames}, blend  # the stub reads every float as 0.5
        entries = ns["register_menu"]()
        assert entries == 4 * len(rig["ik_chains"]) + 3, entries  # and Send to Hive
        if rig["ik_chains"]:
            print("unreal animator tools ran against stubs: %d limbs: ok" % len(rig["ik_chains"]))
        # Pose tools: flip (every control, on this frame and over the range),
        # the pose library (save, apply, apply mirrored, delete) and their menu.
        names = [c["name"] for c in rig["controls"]]
        hierarchy = lib.get_control_rigs.return_value[0].control_rig.get_hierarchy.return_value
        poses = ns["poses_path"]()
        if os.path.exists(poses):
            os.remove(poses)
        if rig.get("mirror_pairs"):
            for argv, frames in ((["flip_pose"], {7}), (["flip_pose", "--range"], {1, 2, 3})):
                lib.reset_mock()
                hierarchy.set_global_transform.reset_mock()
                assert ns["main"](argv) == len(names)
                placed = {c.args[0].name for c in hierarchy.set_global_transform.call_args_list}
                assert placed == set(names), sorted(set(names) - placed)
                keyed = {c.args[3] for c in lib.set_local_control_rig_euler_transform.call_args_list}
                assert keyed == frames, keyed
            # The limb settings swap between twin limbs.
            twins_f = ns["float_twins"]()
            for chain in rig["ik_chains"]:
                if chain["name"].endswith("_l"):
                    assert twins_f[chain["name"] + "_ik_fk"] == chain["name"][:-2] + "_r_ik_fk", twins_f
            # Twins land on each other's mirror image: with every rest frame
            # the same, a centre control stays on the mirror plane.
            point, normal = ns["mirror_plane"]((None, lib.get_control_rigs.return_value[0].control_rig))
            assert abs(sum(n * n for n in normal) - 1.0) < 1e-6
            # Mirror pose: every left control onto its right twin (or back), with
            # the limb settings, on this frame or over the range; only the twins move.
            pairs = rig["mirror_pairs"]
            for argv, onto, frames in ((["mirror_pose"], {r for _, r in pairs}, {7}),
                                       (["mirror_pose", "--range"], {r for _, r in pairs}, {1, 2, 3}),
                                       (["mirror_pose", "--right"], {l for l, _ in pairs}, {7})):
                lib.reset_mock()
                hierarchy.set_global_transform.reset_mock()
                assert ns["main"](argv) == len(pairs), argv
                placed = {c.args[0].name for c in hierarchy.set_global_transform.call_args_list}
                assert placed == onto, (argv, sorted(placed ^ onto))
                keyed = {c.args[3] for c in lib.set_local_control_rig_euler_transform.call_args_list}
                assert keyed == frames, (argv, keyed)
                settings = {c.args[2] for c in lib.set_local_control_rig_float.call_args_list}
                for chain in rig["ik_chains"]:
                    side = chain["ik_control"] in onto
                    has_twin = chain["name"] + "_ik_fk" in twins_f
                    assert (chain["name"] + "_ik_fk" in settings) == (side and has_twin), (argv, chain["name"])
            # One control given: only its twin takes the pose.
            hierarchy.set_global_transform.reset_mock()
            assert ns["mirror_pose"](None, [pairs[0][0]]) == 1
            assert {c.args[0].name for c in hierarchy.set_global_transform.call_args_list} == {pairs[0][1]}
            print("unreal mirror pose ran against stubs: %d twins: ok" % len(pairs))
        else:
            for argv in (["flip_pose"], ["mirror_pose"]):
                try:
                    ns["main"](argv)
                    raise AssertionError("a rig without twins can't be flipped or mirrored")
                except RuntimeError:
                    pass
        # Reset pose: every control and setting back to its initial value, keyed.
        every = len(names) + len(ns["ANIM"]["floats"])
        for argv, frames in ((["reset_pose"], {7}), (["reset_pose", "--range"], {1, 2, 3})):
            lib.reset_mock()
            hierarchy.get_control_value.reset_mock()
            assert ns["main"](argv) == every, argv
            initial = sys.modules["unreal"].RigControlValueType.INITIAL
            rest = {c.args[0].name for c in hierarchy.get_control_value.call_args_list if c.args[1] is initial}
            assert rest == set(names) | set(ns["ANIM"]["floats"]), sorted(set(names) - rest)[:5]
            keyed = {c.args[3] for c in lib.set_local_control_rig_euler_transform.call_args_list}
            assert keyed == frames, keyed
        # A selected IK control takes its limb's settings back with it.
        if rig["ik_chains"]:
            chain = rig["ik_chains"][0]
            got = ns["reset_pose"](None, [chain["ik_control"]])
            assert got == 1 + len([f for f in ns["ANIM"]["floats"] if f.startswith(chain["name"] + "_")]), got
        print("unreal reset pose ran against stubs: ok")
        # The flip's maths with real turns: quaternions survive the round trip,
        # and a posed left control's twin lands on its mirror image.
        import math as m_
        from types import SimpleNamespace as NS_

        def quat(axis, angle):
            n = sum(a * a for a in axis) ** 0.5
            s_ = m_.sin(angle / 2) / n
            return NS_(x=axis[0] * s_, y=axis[1] * s_, z=axis[2] * s_, w=m_.cos(angle / 2))

        for q in (quat((1, 2, 3), 0.7), quat((0, 0, 1), 3.0), quat((1, 0, 0), m_.pi), quat((0.2, -1, 0.1), 2.9)):
            back = ns["matrix_quat"](ns["quat_matrix"](q))
            sign = 1.0 if sum(a * b for a, b in zip(back, (q.x, q.y, q.z, q.w))) > 0 else -1.0
            assert max(abs(a - sign * b) for a, b in zip(back, (q.x, q.y, q.z, q.w))) < 1e-9, (back, q)
        mm, qm = ns["m_mul"], ns["quat_matrix"]
        flip_x = [[-1.0, 0, 0], [0, 1.0, 0], [0, 0, 1.0]]
        r_left = qm(quat((0.3, 1.0, -0.4), 1.1))
        r_right = mm(mm(flip_x, r_left), flip_x)  # a mirror image, right-handed again
        pose = qm(quat((1.0, 0.5, 0.2), 0.6))
        frames = {"L": (r_left, [0.3, 1.0, 0.2]), "R": (r_right, [-0.3, 1.0, 0.2])}
        posed_left = (mm(pose, r_left), [0.35, 1.1, 0.1])

        def fake_global(ctx, k, rest=False):
            rot, loc = frames[k.name] if rest or k.name == "R" else posed_left
            qx, qy, qz, qw = ns["matrix_quat"](rot)
            return NS_(rotation=NS_(x=qx, y=qy, z=qz, w=qw), translation=NS_(x=loc[0], y=loc[1], z=loc[2]))

        live = ns["flip_targets"].__globals__  # run_path returned a copy of these
        saved_global, saved_twins = live["global_of"], live["twins"]
        live["global_of"], live["twins"] = fake_global, lambda: {"L": "R", "R": "L"}
        target = ns["flip_targets"](None, ["R"], ([0.0, 0.0, 0.0], [1.0, 0.0, 0.0]))["R"]
        live["global_of"], live["twins"] = saved_global, saved_twins
        want = mm(mm(mm(flip_x, pose), flip_x), r_right)  # the left turn, mirrored, from the right's rest
        got = qm(NS_(x=target[1][0], y=target[1][1], z=target[1][2], w=target[1][3]))
        assert max(abs(got[i][j] - want[i][j]) for i in range(3) for j in range(3)) < 1e-9, (got, want)
        assert max(abs(a - b) for a, b in zip(target[0], [-0.35, 1.1, 0.1])) < 1e-9, target[0]
        assert ns["main"](["save_pose", "Arms", "up"]) == len(names) + len(ns["ANIM"]["floats"])
        assert ns["main"](["save_pose"]) > 0 and sorted(ns["pose_library"]()) == ["Arms up", "Pose 1"]
        lib.reset_mock()
        assert ns["main"](["apply_pose", "Arms", "up"]) == len(names) + len(ns["ANIM"]["floats"])
        assert {c.args[3] for c in lib.set_local_control_rig_euler_transform.call_args_list} == {7}
        if rig.get("mirror_pairs"):
            assert ns["main"](["apply_pose", "Arms", "up", "--mirrored", "--range"]) > 0
        entries = ns["register_pose_menu"]()
        assert entries == 9 + 2 * 2, entries
        ns["main"](["delete_pose", "Pose 1"])
        assert sorted(ns["pose_library"]()) == ["Arms up"]
        os.remove(poses)
        print("unreal pose tools ran against stubs: ok")
        # Send to Hive: the Skeletal Mesh is exported as a binary FBX with its
        # morph targets (here the rig's own .fbx stands in for Unreal's), which
        # hive-rig turns into the edits file and imports: nothing changed.
        own_fbx = os.path.join(os.path.dirname(os.path.abspath(script)), rig["name"] + ".fbx")
        if os.path.isfile(ns["HIVE_EXE"]) and "--send" in sys_args and os.path.isfile(own_fbx):
            import shutil

            ue = sys.modules["unreal"]
            ue.EditorAssetLibrary.list_assets.return_value = ["/Game/HiveRig/mesh"]
            exported = []

            def export(task):
                exported.append(task)
                shutil.copy(own_fbx, task.filename)
                return True

            ue.Exporter.run_asset_export_task.side_effect = export
            hive_path = ns["rig_files"]()[0]
            before = json.load(open(hive_path))
            said = ns["main"](["send_edits"])
            assert len(exported) == 1 and exported[0].options.export_morph_targets is True, exported
            assert exported[0].options.ascii is False and exported[0].exporter is ue.SkeletalMeshExporterFBX.return_value
            assert "0 of %d vertices changed" % len(before["mesh"]["positions"]) in said, said
            edits = json.load(open(os.path.join(os.path.dirname(hive_path), rig["name"] + ".edits.json")))
            assert edits["source"] == "unreal" and edits["weight_tolerance"] == 0.005, sorted(edits)
            assert sorted(s["name"] for s in edits["shapes"]) == sorted(s["name"] for s in rig.get("shapes", []))
            after = json.load(open(hive_path))
            assert after["skin"] == before["skin"] and after["mesh"].get("shapes") == before["mesh"].get("shapes")
            assert ("Imported shapes" in said) == bool(rig.get("shapes")), said
            print("unreal send to hive ran hive-rig edits, weights import and shape import against stubs: ok")
    else:
        raise SystemExit("unknown kind " + kind)
    print("%s builder ran against stubs: ok" % kind)
