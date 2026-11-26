#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo "usage: $0 <server-26.3.jar> <libraries-dir> <resource-output-dir>" >&2
  exit 2
fi

server_jar=$1
libraries_dir=$2
resource_output=$3

[[ -f "$server_jar" ]] || { echo "server jar not found: $server_jar" >&2; exit 2; }
[[ -d "$libraries_dir" ]] || { echo "libraries directory not found: $libraries_dir" >&2; exit 2; }

library_classpath=""
while IFS= read -r -d '' library; do
  library_classpath+="$library:"
done < <(find "$libraries_dir" -type f -name '*.jar' -print0)

build_dir=$(mktemp -d)
trap 'rm -rf "$build_dir"' EXIT
runtime_entry=$(unzip -Z1 "$server_jar" | awk '/^META-INF\/versions\/[^/]+\/server-[^/]+\.jar$/ { print; exit }')
if [[ -n "$runtime_entry" ]]; then
  unzip -p "$server_jar" "$runtime_entry" > "$build_dir/server-runtime.jar"
  classpath="$build_dir/server-runtime.jar:$library_classpath"
else
  classpath="$server_jar:$library_classpath"
fi

javac -cp "$classpath" -d "$build_dir" "$(dirname "$0")/RegistryBundleExporter.java"
java -cp "$build_dir:$classpath" RegistryBundleExporter "$resource_output"
