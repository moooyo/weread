"""Assemble the qualified stock-tile kit and render observer without extracting either."""

import argparse
import gzip
import io
import json
import os
from pathlib import Path
import re
import struct
import tarfile
import tempfile


MIB = 1024 * 1024
MAX_ARCHIVE = 64 * MIB
MAX_PAYLOAD = 128 * MIB
MAX_FILE = 48 * MIB
MAX_MEMBERS = 96
SHELL = "tile/native_tile.sh"
OBSERVER = "render-observer"
LOADER = "tile/lib/ld-linux-armhf.so.3"
EXECUTABLES = {SHELL, OBSERVER, LOADER, "tile/native-target", "tile/card-input"}
MODELS = {"tile/models/eng.traineddata", "tile/models/chi_sim.traineddata"}
REQUIRED = EXECUTABLES | MODELS | {"tile/lib/" + name for name in (
    "libX11.so.6", "libXtst.so.6", "libc.so.6", "libtesseract.so.5")}
LIBRARY = re.compile(r"tile/lib/[A-Za-z0-9][A-Za-z0-9._+\-]{0,80}")
BLOCK = tarfile.BLOCKSIZE
ZERO = bytes(BLOCK)


def regular_size(path, maximum):
    if path.is_symlink() or not path.is_file() or not 0 < (size := path.stat().st_size) <= maximum:
        raise ValueError("Missing or unbounded regular input")
    return size


def check_content(name, data):
    if name == SHELL:
        if not data.startswith(b"#!/bin/sh\n") or b"\x00" in data or b"\r" in data:
            raise ValueError("Expected the POSIX tile helper with LF endings")
    elif name not in MODELS:
        if (len(data) < 52 or data[:7] != b"\x7fELF\x01\x01\x01"
                or struct.unpack_from("<HHI", data, 16) not in ((2, 40, 1), (3, 40, 1))
                or struct.unpack_from("<I", data, 36)[0] & 0xFF000600 != 0x05000400):
            raise ValueError("Expected a little-endian ARM32 hard-float ELF")
        if name == OBSERVER:
            table = struct.unpack_from("<I", data, 28)[0]
            width, count = struct.unpack_from("<HH", data, 42)
            if width != 32 or not 1 <= count <= 128 or table < 52 or table + width * count > len(data):
                raise ValueError("Invalid render observer program headers")
            segments = {struct.unpack_from("<I", data, table + width * index)[0] for index in range(count)}
            if 1 not in segments or 3 in segments:
                raise ValueError("The directly invoked render observer must have no ELF interpreter")


def write_member(archive, name, data):
    info = tarfile.TarInfo(name)
    info.size = len(data)
    info.mode = 0o755 if name in EXECUTABLES else 0o644
    archive.addfile(info, io.BytesIO(data))


def scan(path, *, kit=False, accept=None):
    archive_bytes = regular_size(path, MAX_ARCHIVE)
    required = REQUIRED - {OBSERVER} if kit else REQUIRED
    seen = set()
    payload_bytes = 0
    with gzip.open(path, "rb") as source:
        while True:
            header = source.read(BLOCK)
            if header == ZERO:
                if source.read(BLOCK) != ZERO:
                    raise ValueError("Missing tar end marker")
                padding = source.read(tarfile.RECORDSIZE + 1)
                if len(padding) > tarfile.RECORDSIZE or len(padding) % BLOCK or any(padding):
                    raise ValueError("Unexpected trailing archive content")
                break
            if len(header) != BLOCK or header[257:265] != b"ustar\x0000":
                raise ValueError("Expected a POSIX USTAR file header")
            member = tarfile.TarInfo.frombuf(header, "ascii", "strict")
            name = member.name
            if (member.type != tarfile.REGTYPE or member.uid or member.gid or member.linkname
                    or not (name in required or (LIBRARY.fullmatch(name) and ".." not in name))
                    or name in seen or len(seen) >= MAX_MEMBERS
                    or not 0 < member.size <= MAX_FILE or payload_bytes + member.size > MAX_PAYLOAD):
                raise ValueError("Unexpected, duplicate or unbounded bundle member")
            if kit:
                if member.mode not in (0o600, 0o644, 0o700, 0o755):
                    raise ValueError("Unexpected qualified kit member mode")
            elif member.mode != (0o755 if name in EXECUTABLES else 0o644):
                raise ValueError("Unexpected installation member mode")
            data = source.read(member.size)
            if len(data) != member.size or source.read(-member.size % BLOCK) != bytes(-member.size % BLOCK):
                raise ValueError("Incomplete or misaligned bundle member")
            check_content(name, data)
            if accept is not None:
                accept(name, data)
            seen.add(name)
            payload_bytes += member.size
    if not required <= seen:
        raise ValueError("Missing a required tool, model or runtime library")
    return {"members": len(seen), "payload_bytes": payload_bytes, "archive_bytes": archive_bytes}


def validate(path):
    try:
        return scan(Path(path))
    except (EOFError, tarfile.TarError) as error:
        raise ValueError("Invalid installation archive") from error


def prepare(kit, shell, observer, output):
    regular_size(shell, MIB)
    regular_size(observer, MAX_FILE)
    current_shell = shell.read_bytes()
    observer_bytes = observer.read_bytes()
    check_content(OBSERVER, observer_bytes)
    if output.exists() or output.is_symlink():
        raise FileExistsError(output)
    with tempfile.TemporaryDirectory(dir=output.parent) as temporary:
        pending = Path(temporary) / "install.tgz"
        with tarfile.open(pending, "x:gz", format=tarfile.USTAR_FORMAT, compresslevel=1) as archive:
            def include(name, data):
                if name == SHELL and data != current_shell:
                    raise ValueError("Qualified tile helper differs from the current source")
                write_member(archive, name, data)

            scan(kit, kit=True, accept=include)
            write_member(archive, OBSERVER, observer_bytes)
        report = validate(pending)
        pending.chmod(0o600)
        os.link(pending, output)
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    operations = parser.add_subparsers(dest="operation", required=True)
    check = operations.add_parser("validate")
    check.add_argument("archive", type=Path)
    build = operations.add_parser("prepare")
    for argument in ("kit", "shell", "observer", "output"):
        build.add_argument(argument, type=Path)
    args = parser.parse_args()
    try:
        result = validate(args.archive) if args.operation == "validate" else prepare(
            args.kit, args.shell, args.observer, args.output)
    except (OSError, EOFError, ValueError, tarfile.TarError) as error:
        parser.exit(1, f"Bundle refused: {error}\n")
    print(json.dumps(result, sort_keys=True))
