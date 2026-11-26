#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo "usage: $0 <server-26.3.jar> <libraries-dir> <resources-dir>" >&2
  exit 2
fi

server_jar=$1
libraries_dir=$2
resources_dir=$3
worldgen_dir="$resources_dir/worldgen"
output_dir="$worldgen_dir/blocks"

[[ -f "$server_jar" ]] || { echo "server jar not found: $server_jar" >&2; exit 2; }
[[ -d "$libraries_dir" ]] || { echo "libraries directory not found: $libraries_dir" >&2; exit 2; }
[[ -f "$resources_dir/facts.json" ]] || { echo "resources/facts.json is required" >&2; exit 2; }
[[ -f "$worldgen_dir/block_tags.json" ]] || { echo "worldgen/block_tags.json is required" >&2; exit 2; }
[[ -f "$worldgen_dir/fluid_tags.json" ]] || { echo "worldgen/fluid_tags.json is required" >&2; exit 2; }

library_classpath=""
while IFS= read -r -d '' library; do
  library_classpath+="$library:"
done < <(find "$libraries_dir" -type f -name '*.jar' -print0)
classpath="$server_jar:$library_classpath"
tool_dir=$(cd "$(dirname "$0")" && pwd)
source_dir="$tool_dir/java/dev/horizonlab/parity"
build_dir=$(mktemp -d)
work_dir=$(mktemp -d)
trap 'rm -rf "$build_dir" "$work_dir"' EXIT
mkdir -p "$output_dir" "$build_dir/classes"

runtime_entry=$(unzip -Z1 "$server_jar" | awk '/^META-INF\/versions\/[^/]+\/server-[^/]+\.jar$/ { print; exit }')
if [[ -n "$runtime_entry" ]]; then
  unzip -p "$server_jar" "$runtime_entry" > "$build_dir/server-runtime.jar"
  classpath="$build_dir/server-runtime.jar:$library_classpath"
else
  classpath="$server_jar:$library_classpath"
fi

javac -cp "$classpath" -d "$build_dir/classes" "$source_dir"/*.java
cp "$resources_dir/facts.json" "$work_dir/facts.json"
cp "$worldgen_dir/block_tags.json" "$work_dir/tags.json"
cp "$worldgen_dir/fluid_tags.json" "$work_dir/fluid-tags.json"
java -cp "$build_dir/classes:$classpath" dev.horizonlab.parity.VanillaSimpleBlockOracle export "$work_dir"
python3 "$tool_dir/bind-survival-rules.py" "$work_dir"

for file in \
  block-properties.nbt \
  cached-collision.nbt \
  cached-support.nbt \
  survival-rules.json \
  survival-extra-definitions.json \
  placement-definitions.json; do
  [[ -s "$work_dir/$file" ]] || { echo "block behavior exporter did not produce $file" >&2; exit 1; }
  install -m 0644 "$work_dir/$file" "$output_dir/$file"
done

echo "Server worldgen block data exported to $output_dir"
