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

temp_dir=$(mktemp -d)
cleanup() {
  python3 - "$temp_dir" <<'PY'
import pathlib
import shutil
import sys
shutil.rmtree(pathlib.Path(sys.argv[1]), ignore_errors=True)
PY
}
trap cleanup EXIT
runtime_entry=$(unzip -Z1 "$server_jar" | awk '/^META-INF\/versions\/[^/]+\/server-[^/]+\.jar$/ { print; exit }')
if [[ -n "$runtime_entry" ]]; then
  unzip -p "$server_jar" "$runtime_entry" > "$temp_dir/server-runtime.jar"
  classpath="$temp_dir/server-runtime.jar:$library_classpath"
else
  classpath="$server_jar:$library_classpath"
fi
java -cp "$classpath" net.minecraft.data.Main --server --reports --output "$temp_dir/generated"

report_dir="$temp_dir/generated/reports/biome_parameters/minecraft"
[[ -f "$report_dir/overworld.json" && -f "$report_dir/nether.json" ]] || {
  echo "vanilla data generator did not produce Overworld and Nether biome parameter reports" >&2
  exit 1
}
install -D -m 0644 "$report_dir/overworld.json" "$resources_dir/worldgen/biome_parameters/overworld.json"
install -D -m 0644 "$report_dir/nether.json" "$resources_dir/worldgen/biome_parameters/nether.json"
echo "Exported vanilla Overworld and Nether biome parameter reports"
