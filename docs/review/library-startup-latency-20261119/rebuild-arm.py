"""Rebuild one font experiment arm on the Linux worker, never on a Kindle."""

import argparse
import json
import os
from pathlib import Path
import platform
import re
import shutil
import struct
import subprocess


ABI = "armv7-unknown-linux-musleabihf"
FEATURE = "bundled-font-preflight"


def inspect_elf(path):
    with path.open("rb") as binary:
        header = binary.read(52)
        if header[:7] != b"\x7fELF\x01\x01\x01":
            raise ValueError("Expected a little-endian ELF32 executable")
        kind, machine = struct.unpack_from("<HH", header, 16)
        table, = struct.unpack_from("<I", header, 28)
        flags, = struct.unpack_from("<I", header, 36)
        entry_size, count = struct.unpack_from("<HH", header, 42)
        if (kind, machine, flags >> 24, flags & 0x600) != (2, 40, 5, 0x400):
            raise ValueError("Expected an ARM EABI5 hard-float executable")
        if entry_size != 32 or not 0 < count <= 128:
            raise ValueError("Invalid program headers")
        binary.seek(table)
        types = [struct.unpack("<I28x", binary.read(32))[0] for _ in range(count)]
        if 1 not in types or 2 in types or 3 in types:
            raise ValueError("Expected static load segments, no interpreter or dynamic segment")
    return {"bytes": path.stat().st_size, "elf_flags": hex(flags), "program_types": types}


def product_outputs(messages):
    main = [
        item for item in messages
        if item["reason"] == "compiler-artifact" and item["target"]["name"] == "weread"
        and item["target"]["kind"] == ["bin"] and item["executable"]
    ]
    app, = main
    backend, = [
        item for item in messages
        if item["reason"] == "compiler-artifact" and item["target"]["name"] == "slint_backend_kindle"
        and "lib" in item["target"]["kind"]
    ]
    if app["fresh"] or backend["fresh"]:
        raise ValueError("Both the application and Kindle backend must be recompiled")
    generated, = [
        Path(item["out_dir"]) / "bundled_fonts.rs" for item in messages
        if item["reason"] == "build-script-executed" and item["package_id"] == app["package_id"]
    ]
    return app, generated


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("lane", choices=["baseline", "candidate"])
    parser.add_argument("source", type=Path, help="frozen checkout or git archive of the product")
    parser.add_argument("output", type=Path, help="private build root, shared only between these two arms")
    args = parser.parse_args()
    if (platform.system(), platform.machine()) != ("Linux", "x86_64"):
        parser.error("Use the designated x86_64 Linux worker")
    source = args.source.resolve(strict=True)
    if not (source / "Cargo.lock").is_file():
        parser.error("Expected the locked WeRead source")
    output = args.output.resolve()
    if output == source or output.is_relative_to(source):
        parser.error("Keep the build outside the frozen source")
    output.mkdir(exist_ok=True)
    lane = output / args.lane
    lane.mkdir()
    target = output / "target"
    env = dict(os.environ, CARGO_TARGET_DIR=str(target), CARGO_NET_OFFLINE="true", LC_ALL="C")
    features = ["kindle"] + ([FEATURE] if args.lane == "candidate" else [])
    common = ["--locked", "--offline", "--release", "--target", ABI]
    with (lane / "build.log").open("w") as log:
        if target.exists():
            subprocess.run(
                ["cargo", "clean", *common, "-p", "weread", "-p", "slint-backend-kindle"],
                cwd=source, env=env, stdout=log, stderr=subprocess.STDOUT, check=True,
            )
        command = [
            "cargo", "zigbuild", *common, "--no-default-features", "--features", ",".join(features),
            "--bin", "weread", "--message-format=json-render-diagnostics",
        ]
        with (lane / "cargo.jsonl").open("w") as stream:
            subprocess.run(command, cwd=source, env=env, stdout=stream, stderr=log, check=True)
    with (lane / "cargo.jsonl").open() as stream:
        app, generated = product_outputs([json.loads(line) for line in stream])
    if set(app["features"]) != set(features):
        raise ValueError(f"Unexpected application features: {app['features']}")
    declarations = generated.read_text()
    faces = re.findall(r"^static ([A-Z_]+)_BYTES:", declarations, re.MULTILINE)
    receipts = re.findall(r"^    \(([A-Z_]+), font_budget::CffLoopCharge", declarations, re.MULTILINE)
    if len(set(faces)) != 7 or len(faces) != 7 or receipts != (faces if FEATURE in features else []):
        raise ValueError("Generated font declarations do not match this lane")
    executable = Path(app["executable"])
    if executable != target / ABI / "release" / "weread":
        raise ValueError("Unexpected Cargo output path")
    elf = inspect_elf(executable)
    shutil.copy2(executable, lane / "weread")
    result = {"lane": args.lane, "features": features, "generated_receipts": receipts, "elf": elf}
    (lane / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result), flush=True)


if __name__ == "__main__":
    main()
