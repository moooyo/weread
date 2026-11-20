"""Rehearse file placement and cleanup in a disposable Linux directory, without launching either arm."""

import argparse
import filecmp
import json
from pathlib import Path, PurePosixPath
import platform
import shutil
import tarfile
import tempfile


class InterruptedCopy(Exception):
    pass


def copy_arm(source, stage, interrupt=False):
    app = stage / "app"
    app.mkdir()
    partial = app / "weread.partial"
    with source.open("rb") as incoming, partial.open("xb") as outgoing:
        while chunk := incoming.read(1024 * 1024):
            outgoing.write(chunk)
            if interrupt:
                raise InterruptedCopy
    if not filecmp.cmp(source, partial, shallow=False):
        raise ValueError("Staged copy differs")
    partial.chmod(0o700)
    partial.rename(app / "weread")


def public_members(archive):
    seen = set()
    members = []
    size = 0
    for member in archive:
        path = PurePosixPath(member.name)
        if (not path.parts or path.is_absolute() or ".." in path.parts or path in seen
                or not (member.isdir() or member.isfile())
                or not (path == PurePosixPath("library-covers.json") or path.parts[0] == "demo")):
            raise ValueError(f"Unexpected public fixture member: {member.name}")
        size += member.size
        if len(members) >= 256 or size > 16 * 1024 * 1024:
            raise ValueError("Unexpected public fixture size")
        seen.add(path)
        members.append(member)
    if not {PurePosixPath("library-covers.json"), PurePosixPath("demo/library.sqlite")} <= seen:
        raise ValueError("Missing shelf database or cover bindings")
    return members, size


def rehearse(parent, pair, seed, bindings):
    sizes = {}
    with tempfile.TemporaryDirectory(prefix="weread-volume-", dir=parent) as temporary:
        volume = Path(temporary)
        protected = [volume / name for name in ("documents/weread.sh", "weread/data/untouched", "peer/untouched")]
        for path in protected:
            path.parent.mkdir(parents=True)
            path.write_bytes(b"fixture-owned sentinel\n")
        neighbors = set(volume.iterdir())
        with tarfile.open(seed, "r:gz") as archive:
            members, expanded = public_members(archive)
            for name in ("baseline", "candidate"):
                source = pair / name / "weread"
                if source.is_symlink() or not source.is_file():
                    raise ValueError(f"Expected a retained regular executable for {name}")
                sizes[name] = source.stat().st_size
                if shutil.disk_usage(volume).free < sizes[name] + expanded + 64 * 1024 * 1024:
                    raise ValueError("Insufficient space for one staged arm, restored data and headroom")
                with tempfile.TemporaryDirectory(prefix=".weread-trial-", dir=volume) as directory:
                    stage = Path(directory)
                    copy_arm(source, stage)
                    data = stage / "data"
                    data.mkdir()
                    archive.extractall(data, members=members, filter="data")
                    if json.loads((data / "library-covers.json").read_bytes()) != bindings:
                        raise ValueError("Restored public cover bindings differ")
                    if not all((data / target["thumbnail"]).is_file() for target in bindings["targets"]):
                        raise ValueError("Missing restored thumbnail")
                    with (data / "demo/library.sqlite").open("rb") as database:
                        if database.read(16) != b"SQLite format 3\0":
                            raise ValueError("Missing restored SQLite database")
                    (data / "demo/library.sqlite").unlink()
            with tempfile.TemporaryDirectory(prefix=".weread-trial-", dir=volume) as directory:
                stage = Path(directory)
                try:
                    copy_arm(pair / "candidate/weread", stage, interrupt=True)
                except InterruptedCopy:
                    if (stage / "app/weread").exists() or not (stage / "app/weread.partial").is_file():
                        raise ValueError("Unexpected partial transfer state")
                else:
                    raise ValueError("Interrupted transfer unexpectedly completed")
        if set(volume.iterdir()) != neighbors or any(path.read_bytes() != b"fixture-owned sentinel\n" for path in protected):
            raise ValueError("Staging residue or changed protected neighbors")
    if volume.exists():
        raise ValueError("Disposable model was not removed")
    return {"arm_bytes": sizes, "seed_expanded_bytes": expanded, "covers_per_restore": len(bindings["targets"]),
            "interrupted_copy_cleanup": True, "protected_sentinels_unchanged": len(protected)}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("parent", type=Path, help="existing private worker directory on disk")
    parser.add_argument("pair", type=Path, help="output directory of rebuild-arm.py")
    parser.add_argument("seed", type=Path, help="frozen public seed archive")
    args = parser.parse_args()
    if (platform.system(), platform.machine()) != ("Linux", "x86_64"):
        parser.error("Use the designated x86_64 Linux worker")
    reference = Path(__file__).resolve().parents[1] / "catalog-entry-adapter-20261118/library-cover-bindings.json"
    print(json.dumps(rehearse(args.parent, args.pair, args.seed, json.loads(reference.read_bytes())), indent=2))


if __name__ == "__main__":
    main()
