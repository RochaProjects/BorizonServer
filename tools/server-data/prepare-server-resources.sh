#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo "usage: $0 <server-26.3.jar> <libraries-dir> <resource-output-dir>" >&2
  exit 2
fi

server_jar=$1
libraries_dir=$2
resources_dir=$3
tool_dir=$(cd "$(dirname "$0")" && pwd)

[[ -f "$server_jar" ]] || { echo "server jar not found: $server_jar" >&2; exit 2; }
[[ -d "$libraries_dir" ]] || { echo "libraries directory not found: $libraries_dir" >&2; exit 2; }
mkdir -p "$resources_dir"

"$tool_dir/export-worldgen-bundles.py" "$server_jar" "$resources_dir"
# Behaviour loading consumes the same vanilla tag bundle at the resource root,
# while the generator consumes it under worldgen/.
cp "$resources_dir/worldgen/block_tags.json" "$resources_dir/block-tags.json"
python3 - "$resources_dir/worldgen/fluid_tags.json" "$resources_dir/data" <<'PY'
import json
import pathlib
import sys

bundles = json.loads(pathlib.Path(sys.argv[1]).read_text())
root = pathlib.Path(sys.argv[2])
for resource_id, content in bundles.items():
    namespace, name = resource_id.split(":", 1)
    path = root / namespace / "tags" / "fluid" / f"{name}.json"
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(content + "\n")
PY
"$tool_dir/export-biome-parameter-reports.sh" "$server_jar" "$libraries_dir" "$resources_dir"
"$tool_dir/export-server-block-data.sh" "$server_jar" "$libraries_dir" "$resources_dir"
"$tool_dir/export-registry-sync.sh" "$server_jar" "$libraries_dir" "$resources_dir"
"$tool_dir/export-worldgen-blocks.sh" "$server_jar" "$libraries_dir" "$resources_dir"
"$tool_dir/export-gameplay-data.py" "$server_jar" "$resources_dir"
cp "$resources_dir/worldgen/blocks/cached-collision.nbt" "$resources_dir/cached-collision.nbt"

echo "Vanilla 26.3 server resource inputs prepared at $resources_dir"
