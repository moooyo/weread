#!/usr/bin/env python3
"""Archive only explicit build outputs and notices; never runtime user data."""

from __future__ import annotations

import argparse
import hashlib
from pathlib import Path
import stat
import zipfile


REQUIRED = (
    "documents/weread.sh",
    "weread/app/weread",
    "weread/app/README.md",
    "weread/app/THIRD_PARTY_NOTICES.md",
    "weread/app/config/device-profile.example",
    "weread/app/config/device-allowlist.example",
    "weread/app/manifest.draft.json",
)

EXECUTABLE_INPUTS = frozenset((
    "documents/weread.sh",
    "weread/app/weread",
))


def archive(staged: Path, output: Path) -> dict[str, str]:
    staged = staged.resolve(strict=True)
    paths = [staged / name for name in REQUIRED]
    required_paths = set(paths)
    for path in paths:
        if not path.is_file():
            name = path.relative_to(staged).as_posix()
            raise ValueError(f"Missing required staged build input: {name}")
    notices = staged / "weread/app/licenses"
    if not notices.is_dir():
        raise ValueError("The staged dependency notices are missing")
    paths.extend(path for path in notices.rglob("*") if path.is_file())
    for path in paths:
        if not path.is_file() or path.is_symlink() or not path.resolve().is_relative_to(staged):
            raise ValueError(f"Invalid staged build input: {path.name}")
        if path not in required_paths and path.suffix not in (".txt", ".md", ".json") and path.name not in ("LICENSE", "LICENSE-MIT", "LICENSE-APACHE", "NOTICE", "UPSTREAM_REVISION"):
            raise ValueError(f"Unexpected dependency notice file: {path.name}")
    output.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=6) as result:
        for path in sorted(set(paths)):
            name = path.relative_to(staged).as_posix()
            info = zipfile.ZipInfo.from_file(path, arcname=name)
            info.create_system = 3
            mode = 0o755 if name in EXECUTABLE_INPUTS else 0o644
            info.external_attr = (stat.S_IFREG | mode) << 16
            info.compress_type = zipfile.ZIP_DEFLATED
            result.writestr(info, path.read_bytes())
    checksums = {
        output.name: hashlib.sha256(output.read_bytes()).hexdigest(),
        "weread/app/weread": hashlib.sha256((staged / "weread/app/weread").read_bytes()).hexdigest(),
    }
    output.with_suffix(".sha256").write_text(
        "".join(f"{checksum}  {name}\n" for name, checksum in checksums.items()),
        encoding="utf-8",
    )
    return checksums


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--staged", type=Path, default=Path("dist/kindle"))
    parser.add_argument("--output", type=Path, default=Path("dist/weread-kindle-0.2.0-dev.zip"))
    arguments = parser.parse_args()
    checksums = archive(arguments.staged, arguments.output)
    print(f"Development archive: {arguments.output}")
    for name, checksum in checksums.items():
        print(f"{checksum}  {name}")


if __name__ == "__main__":
    main()
