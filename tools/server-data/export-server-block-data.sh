#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo "usage: $0 <server-26.3.jar> <libraries-dir> <resources-dir>" >&2
  exit 2
fi

server_jar=$1
libraries_dir=$2
resources_dir=$3
[[ -f "$server_jar" ]] || { echo "server jar not found: $server_jar" >&2; exit 2; }
[[ -d "$libraries_dir" ]] || { echo "libraries directory not found: $libraries_dir" >&2; exit 2; }

library_classpath=""
while IFS= read -r -d '' library; do
  library_classpath+="$library:"
done < <(find "$libraries_dir" -type f -name '*.jar' -print0)
tool_dir=$(cd "$(dirname "$0")" && pwd)
source_dir="$tool_dir/java/dev/horizonlab/parity"
build_dir=$(mktemp -d)
work_dir=$(mktemp -d)
trap 'rm -rf "$build_dir" "$work_dir"' EXIT
mkdir -p "$build_dir/classes" "$resources_dir"

runtime_entry=$(unzip -Z1 "$server_jar" | awk '/^META-INF\/versions\/[^/]+\/server-[^/]+\.jar$/ { print; exit }')
if [[ -n "$runtime_entry" ]]; then
  unzip -p "$server_jar" "$runtime_entry" > "$build_dir/server-runtime.jar"
  classpath="$build_dir/server-runtime.jar:$library_classpath"
else
  classpath="$server_jar:$library_classpath"
fi

javac -cp "$classpath" -d "$build_dir/classes" "$source_dir"/*.java
java -cp "$build_dir/classes:$classpath" \
  dev.horizonlab.parity.VanillaHeightmapOracle export "$work_dir/facts.json"
java -cp "$build_dir/classes:$classpath" \
  dev.horizonlab.parity.VanillaBlockBehaviourExporter "$work_dir/block-behaviours.json"
java -cp "$build_dir/classes:$classpath" \
  dev.horizonlab.parity.VanillaGameplayFactsExporter "$work_dir/gameplay-facts.json"
java -cp "$build_dir/classes:$classpath" \
  dev.horizonlab.parity.VanillaLightOcclusionExporter "$work_dir/light-occlusion.json"

python3 - "$work_dir/facts.json" "$work_dir/block_flags.json" <<'PY'
import json
import pathlib
import sys

facts = json.loads(pathlib.Path(sys.argv[1]).read_text())
states = facts.get("states")
if not isinstance(states, list) or not states or any(not isinstance(state, list) or len(state) < 3 for state in states):
    raise SystemExit("vanilla block-state exporter returned malformed facts")
pathlib.Path(sys.argv[2]).write_text(
    json.dumps([[state[1], state[2]] for state in states], separators=(",", ":")) + "\n"
)
PY

for file in facts.json block-behaviours.json gameplay-facts.json light-occlusion.json; do
  [[ -s "$work_dir/$file" ]] || { echo "server block exporter did not produce $file" >&2; exit 1; }
  install -m 0644 "$work_dir/$file" "$resources_dir/$file"
done
mkdir -p "$resources_dir/worldgen"
install -m 0644 "$work_dir/block_flags.json" "$resources_dir/worldgen/block_flags.json"
echo "Server block registry and gameplay facts exported to $resources_dir"
