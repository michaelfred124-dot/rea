"""Run a generated Maya or Unreal builder against stub modules.

Catches Python-level mistakes (typos, wrong names, bad data access) in the
generated scripts without needing the DCC installed. It does not prove the DCC
API calls behave as intended; that needs the real application.

Usage: python3 dcc_stub_run.py maya|unreal <script.py>
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


def stub_maya():
    calls = []
    counter = [0]

    def node(*args, **kwargs):
        counter[0] += 1
        return kwargs.get("name", "node%d" % counter[0])

    cmds = types.ModuleType("maya.cmds")

    def generic(name):
        def fn(*args, **kwargs):
            calls.append(name)
            if name in ("listRelatives",):
                return ["parent_or_shape"]
            if name in ("skinCluster", "ikHandle", "orientConstraint", "parentConstraint", "poleVectorConstraint"):
                if kwargs.get("query") and kwargs.get("weightAliasList"):
                    targets = [a for a in args if isinstance(a, str)]
                    return ["t%d_W%d" % (i, i) for i in range(max(2, len(targets)))]
                if kwargs.get("query") and kwargs.get("name"):
                    return name + "_constraint"
                calls.append((name, args))
                return [kwargs.get("name", name + "1")]
            if name == "xform" and kwargs.get("q"):
                return [0.0, 0.0, 0.0]
            if name == "getAttr":
                return 10.0
            if name in ("group", "curve", "createNode", "rename"):
                return node(*args, **kwargs) if name != "rename" else args[1]
            return None

        return fn

    for n in ["about", "group", "rename", "parent", "listRelatives", "sets", "polySoftEdge", "createNode", "xform",
              "joint", "setAttr", "skinCluster", "curve", "matchTransform", "move", "parentConstraint",
              "orientConstraint", "addAttr", "makeIdentity", "ikHandle", "poleVectorConstraint", "connectAttr",
              "select", "file", "getAttr", "expression"]:
        setattr(cmds, n, generic(n))

    om = types.ModuleType("maya.api.OpenMaya")
    om.MPoint = lambda *a: a
    om.MPointArray = list
    om.MVector = Vec
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

            rig = json.loads(open(sys.argv[2]).read().split('r"""', 1)[1].split('"""', 1)[0])
            names = [b["name"] for b in rig["skeleton"]["bones"] if b["deform"]]
            return [mock.Mock(partialPathName=mock.Mock(return_value=n)) for n in names]

        def setWeights(self, *a):
            calls.append("setWeights")

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
        def rotator(self):
            return self

        def unrotate_vector(self, v):
            return v

    class T:
        def __init__(self, location=None, scale=None, rotation=None):
            self.translation = location or Vec()
            self.rotation = Q()

        def make_relative(self, other):
            return T(self.translation - other.translation)

        def inverse_transform_direction(self, v):
            return v

    unreal.Transform = T
    blueprint = unreal.ControlRigBlueprintFactory.create_control_rig_from_skeletal_mesh_or_skeleton.return_value
    positions = {}

    def global_transform(key, initial=True):
        name = getattr(key, "name", "")
        return T(positions.setdefault(str(name), Vec(len(positions), 1.0, 0.5 * len(positions))))

    blueprint.hierarchy.get_global_transform.side_effect = global_transform
    vm = blueprint.get_controller_by_name.return_value
    vm.get_graph.return_value.get_nodes.return_value = []
    sys.modules["unreal"] = unreal
    return vm


if __name__ == "__main__":
    kind, script = sys.argv[1], sys.argv[2]
    if kind == "maya":
        calls = stub_maya()
        runpy.run_path(script, run_name="__main__")
        assert "setWeights" in calls and "ikHandle" in calls, calls
    elif kind == "unreal":
        vm = stub_unreal()
        runpy.run_path(script, run_name="__main__")
        assert vm.add_link.call_count > 20
    else:
        raise SystemExit("unknown kind " + kind)
    print("%s builder ran against stubs: ok" % kind)
