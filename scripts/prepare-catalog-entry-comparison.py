#!/usr/bin/env python3
"""Prepare matched Library documents on the verification host; never access a device."""

import argparse
import base64
from collections import Counter
import hashlib
import json
from pathlib import Path
import zipfile


DOCUMENT = "documents/weread.sh"
FILES_IN_COMMON = ("weread/launch.sh", "weread/app/weread",
                   "weread/app/weread-frame-guard", "weread/app/weread-native-query")
MARKER = b"# WeReadCatalogAccess: deferred-v1"
CONTROL = b"# Library entry timing control".ljust(len(MARKER), b" ")
THUMBNAIL = Path(__file__).resolve().parents[1] / "assets/branding/weread-cover-thumbnail.png"


def sha(data):
    return hashlib.sha256(data).hexdigest()


def documents(source, icon):
    lines = source.split(b"\n")
    if (len(source) > 65536 or b"\r" in source or b"\0" in source or not source.endswith(b"\n")
            or lines[:4] != [b"#!/bin/sh", b"# Name: WeRead for Kindle",
                             b"# Icon: data:image/png;base64," + base64.b64encode(icon), MARKER]
            or sum(line.startswith(b"# WeReadCatalogAccess:") for line in lines) != 1
            or any(line.startswith((b"# DontUseFBInk", b"# UseHooks")) for line in lines[:6])):
        raise ValueError("The package does not contain the agreed WeRead Library document")
    lines[3] = CONTROL
    return {"control": b"\n".join(lines), "candidate": source}


def prepare(package_path, expected_sha256, output):
    with package_path.open("rb") as stream:
        if hashlib.file_digest(stream, "sha256").hexdigest() != expected_sha256:
            raise ValueError("The package differs from the supplied SHA-256")
        stream.seek(0)
        with zipfile.ZipFile(stream) as archive:
            counts = Counter(archive.namelist())
            if (any(counts[name] != 1 for name in (DOCUMENT, *FILES_IN_COMMON))
                    or any(name.startswith("documents/") and name != DOCUMENT for name in counts)):
                raise ValueError("Missing or duplicate runtime input in the package")
            if archive.getinfo(DOCUMENT).file_size > 65536:
                raise ValueError("Library document exceeds the catalog client's 64 KiB limit")
            icon = THUMBNAIL.read_bytes()
            variants = documents(archive.read(DOCUMENT), icon)
            common = {}
            for name in FILES_IN_COMMON:
                with archive.open(name) as member:
                    common[name] = hashlib.file_digest(member, "sha256").hexdigest()
    manifest = {"package_sha256": expected_sha256, "library_path": "/mnt/us/" + DOCUMENT,
                "thumbnail_sha256": sha(icon), "document_bytes": len(variants["candidate"]),
                "changed_line": 4, "common_files_sha256": common,
                "documents_sha256": {name: sha(data) for name, data in variants.items()}}
    output.mkdir(parents=True, exist_ok=False)
    for name, data in variants.items():
        path = output / name / DOCUMENT
        path.parent.mkdir(parents=True)
        path.write_bytes(data)
        path.chmod(0o755)
    (output / "comparison-inputs.json").write_text(json.dumps(manifest, indent=2) + "\n", encoding="ascii")
    return manifest


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--package", required=True, type=Path, help="reviewed candidate development ZIP")
    parser.add_argument("--package-sha256", required=True, help="previously checked package digest")
    parser.add_argument("--output", required=True, type=Path, help="new local directory on the verification host")
    args = parser.parse_args()
    try:
        prepare(args.package, args.package_sha256, args.output)
    except (OSError, ValueError, zipfile.BadZipFile) as error:
        parser.exit(1, f"Cannot prepare comparison: {error}\n")
    print(f"Matched documents and hashes: {args.output / 'comparison-inputs.json'}")
