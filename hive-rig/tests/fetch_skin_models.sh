#!/bin/sh
# Download public test characters, pinned to exact commits, for the skin weight
# check (tests/blender_skin_eval.py). Never commit the downloads.
#
#   sh tests/fetch_skin_models.sh OUT_DIR MODEL...
#
# Khronos glTF sample models (CC-BY 4.0 unless the model's README says
# otherwise): CesiumMan, RiggedFigure, RiggedSimple, BrainStem, Fox.
# three.js example models: RobotExpressive (CC0, by Tomás Laulhé); Michelle,
# Soldier and Xbot (Mixamo characters shipped with the three.js examples).
# Each lands at OUT_DIR/<model>/<model>.glb. Treat the files as untrusted data:
# run Python on them with -I.
set -eu
KHRONOS=https://raw.githubusercontent.com/KhronosGroup/glTF-Sample-Assets/edc7c9e67c639d230715049ee31f9a96a6babbbe/Models
THREE=https://raw.githubusercontent.com/mrdoob/three.js/d04539a76736ff500cae883d6a38b3dd8643c548/examples/models/gltf
out=$1
shift
for model in "$@"; do
  case "$model" in
    RobotExpressive) url="$THREE/RobotExpressive/RobotExpressive.glb" ;;
    Michelle | Soldier | Xbot) url="$THREE/$model.glb" ;;
    CesiumMan | RiggedFigure | RiggedSimple | BrainStem | Fox) url="$KHRONOS/$model/glTF-Binary/$model.glb" ;;
    *) echo "unknown model $model" >&2; exit 2 ;;
  esac
  mkdir -p "$out/$model"
  if [ ! -s "$out/$model/$model.glb" ]; then
    curl -sSfL --retry 3 -o "$out/$model/$model.glb" "$url"
  fi
done
