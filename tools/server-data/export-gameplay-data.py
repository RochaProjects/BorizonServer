#!/usr/bin/env python3
"""Copy vanilla 26.3 loot and related server data from its server runtime jar."""

from __future__ import annotations

import io
import pathlib
import re
import sys
import zipfile


DATA_KINDS = (
    "loot_table",
    "item_modifier",
    "predicate",
    "enchantment",
    "recipe",
    "tags/item",
    "tags/block",
    "tags/enchantment",
    "tags/instrument",
    "tags/entity_type",
    "tags/damage_type",
    "tags/worldgen/biome",
)


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
    runtime = zipfile.ZipFile(io.BytesIO(outer.read(candidates[0])))
    outer.close()
    return runtime


def export(server_jar: pathlib.Path, output: pathlib.Path) -> int:
    output.mkdir(parents=True, exist_ok=True)
    jar = open_runtime_jar(server_jar)
    count = 0
    try:
        names = sorted(jar.namelist())
        for name in names:
            if not name.startswith("data/") or not name.endswith(".json"):
                continue
            parts = name.split("/", 2)
            if len(parts) != 3:
                continue
            namespace, resource = parts[1], parts[2]
            # Built-in data packs are nested under data/<namespace>/datapacks.
            # They have their own enablement/priority rules and must not be
            # flattened into the base vanilla resource pack.
            if resource.startswith("datapacks/"):
                continue
            if not any(resource.startswith(kind + "/") for kind in DATA_KINDS):
                continue
            relative = pathlib.PurePosixPath(namespace) / pathlib.PurePosixPath(resource)
            if relative.is_absolute() or ".." in relative.parts:
                raise ValueError(f"invalid server resource path in jar: {name}")
            destination = output / "data" / pathlib.Path(*relative.parts)
            destination.parent.mkdir(parents=True, exist_ok=True)
            destination.write_bytes(jar.read(name))
            count += 1
    finally:
        jar.close()
    if count == 0:
        raise ValueError("server runtime jar contains no loot/gameplay JSON resources")
    print(f"Exported {count} vanilla loot and gameplay JSON files to {output / 'data'}")
    return count


if __name__ == "__main__":
    if len(sys.argv) != 3:
        raise SystemExit("usage: export-gameplay-data.py <server-26.3.jar> <resources-dir>")
    export(pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2]))
