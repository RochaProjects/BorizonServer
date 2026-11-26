#!/usr/bin/env python3
"""Flatten vanilla 26.3 server-jar worldgen JSON into horizon-world bundles."""

import io
import json
import pathlib
import re
import sys
import zipfile

JSON_BUNDLES = {
    "noises": "worldgen/noise",
    "densities": "worldgen/density_function",
    "rules": "worldgen/material_rule",
    "conditions": "worldgen/material_condition",
    "settings": "worldgen/noise_settings",
    "carvers": "worldgen/carver",
    "biomes": "worldgen/biome",
    "world_presets": "worldgen/world_preset",
    "features": "worldgen/feature",
    "placed_features": "worldgen/placed_feature",
    "state_providers": "worldgen/block_state_provider",
    "biome_tags": "tags/worldgen/biome",
    "feature_tags": "tags/worldgen/feature",
    "block_tags": "tags/block",
    "fluid_tags": "tags/fluid",
    "processor_lists": "worldgen/processor_list",
    "template_pools": "worldgen/template_pool",
    "structures": "worldgen/structure",
    "structure_sets": "worldgen/structure_set",
}


def open_runtime_jar(path: pathlib.Path) -> zipfile.ZipFile:
    outer = zipfile.ZipFile(path)
    candidates = [
        name
        for name in outer.namelist()
        if re.fullmatch(r"META-INF/versions/[^/]+/server-[^/]+\.jar", name)
    ]
    if not candidates:
        return outer
    if len(candidates) != 1:
        raise ValueError(f"expected one bundled server runtime jar, found {candidates}")
    runtime_bytes = outer.read(candidates[0])
    outer.close()
    return zipfile.ZipFile(io.BytesIO(runtime_bytes))


def entries(jar: zipfile.ZipFile, resource_path: str) -> dict[str, str]:
    resource_prefix = f"{resource_path}/"
    output = {}
    for name in jar.namelist():
        if not name.startswith("data/") or not name.endswith(".json"):
            continue
        parts = name.split("/", 2)
        if len(parts) != 3 or parts[0] != "data":
            continue
        namespace, path = parts[1], parts[2]
        if not path.startswith(resource_prefix):
            continue
        relative = path[len(resource_prefix) :]
        if not relative or relative.startswith("../"):
            continue
        identifier = f"{namespace}:{relative[:-5]}"
        output[identifier] = jar.read(name).decode("utf-8")
    return output


def write_bundle(path: pathlib.Path, content: dict[str, str]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(content, separators=(",", ":")) + "\n")


def main(server_jar: str, resources: str) -> None:
    jar = open_runtime_jar(pathlib.Path(server_jar))
    root = pathlib.Path(resources) / "worldgen"
    try:
        for name, resource_path in JSON_BUNDLES.items():
            bundle = entries(jar, resource_path)
            if not bundle:
                raise ValueError(f"no server data found for {name} ({resource_path})")
            write_bundle(root / f"{name}.json", bundle)

        structure_prefix = "data/minecraft/structure/"
        templates = [
            name
            for name in jar.namelist()
            if name.startswith(structure_prefix) and name.endswith(".nbt")
        ]
        if not templates:
            raise ValueError("server jar has no vanilla structure templates")
        for name in templates:
            relative = pathlib.PurePosixPath(name[len(structure_prefix) :])
            if relative.is_absolute() or ".." in relative.parts:
                raise ValueError(f"invalid structure path {name}")
            output = root / "structure" / "minecraft" / pathlib.Path(*relative.parts)
            output.parent.mkdir(parents=True, exist_ok=True)
            output.write_bytes(jar.read(name))
    finally:
        jar.close()

    print(f"Exported {len(JSON_BUNDLES)} worldgen bundles and {len(templates)} templates to {root}")


if __name__ == "__main__":
    if len(sys.argv) != 3:
        raise SystemExit("usage: export-worldgen-bundles.py <server-26.3.jar> <resources-dir>")
    main(sys.argv[1], sys.argv[2])
