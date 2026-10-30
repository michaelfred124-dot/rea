"""Check two guides files read from the same character agree: the same
components, each guide within a tolerance (a share of the body height).

    python3 -I compare_guides.py a.guides.json b.guides.json [--tolerance 0.002]

Used to check that a character rigged from its .fbx (made from its .glb by
Blender) gets the guides its .glb gets.
"""

import argparse
import json
import math
import sys

ap = argparse.ArgumentParser()
ap.add_argument("a")
ap.add_argument("b")
ap.add_argument("--tolerance", type=float, default=0.002)
args = ap.parse_args()

a, b = json.load(open(args.a)), json.load(open(args.b))
height = a["body"]["height"]
ga = {c["id"]: c for c in a["components"]}
gb = {c["id"]: c for c in b["components"]}
fail = []
if set(ga) != set(gb):
    fail.append(f"components differ: only in a {sorted(set(ga) - set(gb))}, only in b {sorted(set(gb) - set(ga))}")
worst = (0.0, "")
for cid in sorted(set(ga) & set(gb)):
    pa = {g["name"]: g["position"] for g in ga[cid]["guides"]}
    pb = {g["name"]: g["position"] for g in gb[cid]["guides"]}
    if set(pa) != set(pb):
        fail.append(f"{cid}: guides differ")
        continue
    if ga[cid].get("parent") != gb[cid].get("parent"):
        fail.append(f"{cid}: parent {ga[cid].get('parent')} vs {gb[cid].get('parent')}")
    for name in pa:
        d = math.dist(pa[name], pb[name]) / height
        worst = max(worst, (d, f"{cid}.{name}"))
        if d > args.tolerance:
            fail.append(f"{cid}.{name}: {d:.4f} of the height apart")
print(f"{len(ga)} components; the furthest guide apart is {worst[1]} at {worst[0]:.2e} of the height")
if fail:
    sys.exit("guides differ:\n  " + "\n  ".join(fail[:20]))
