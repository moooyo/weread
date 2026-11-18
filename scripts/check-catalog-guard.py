#!/usr/bin/env python3
"""Compare two previously collected catalog protection snapshots offline.

Use --aidoku-handoff for the owner-reported deployment baseline shipped with
this checkout. It is an expectation for preflight and closeout, never a device
observation or an instruction to restore a file. To compare a different
explicitly frozen pair, supply --inventory, --inventory-sha256 and --expected.

Freeze an inventory from both owners before the observation. It contains the
SHA-256 of the exact expected snapshot file and each owner's approved paths:
  {"expected_sha256": "...", "aidoku": {"program": [...], "entry": [...]},
   "bilicomics": {"program": [...], "entry": [...]}}
Keep the SHA-256 of the inventory file in an independent, trusted record.
Each list must name canonical, absolute public install paths. A string requires
a file hash. A separate {"path": "/public/path", "state": "absent"} entry
protects the absence of that exact path (file or directory). Either owner may
include a nonempty "config" role for individually frozen public paths. Omitting
that role makes no claim about configuration. Aidoku may also include "asset".
Its installed program and entry need positive file hashes. Only a pair of
BiliComics spellings in the same role may differ
only by case, and both must be explicitly absent. No installed path is opened.

Both snapshots use this shape:
  {"registration": {"runtime_version": "v1-<64 lowercase hex digits>",
   "projection_sha256": "..."}, "files": {"/absolute/public/path": "...", ...}}
Derive the version from the registered target. Use the owners' agreed complete
registration projection and exact serialization for its SHA-256, including the
handler and association. Collectors must establish that all required rows are
present. A digest of only the command string is not equivalent.
File values are lowercase SHA-256 hashes of public file contents or exactly
{"state": "absent"} at the named key. Both snapshots must cover every path,
even when absent. An unreadable or omitted path does not establish absence;
a dangling symlink is present. An observed {"state": "present"} can report
that an absence-only path now exists, including a directory; it always fails.
Never include credentials, device IDs or private data.

Exit 0 means the supplied evidence matches, not that a device was inspected.
Exit 1 reports drift; exit 2 rejects invalid inputs or unqualified evidence.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import re
import sys
from typing import NamedTuple


OWNERS = ("aidoku", "bilicomics")
REQUIRED_ROLES = ("program", "entry")
ABSENT = {"state": "absent"}
PRESENT = {"state": "present"}
SHA256 = re.compile(r"[0-9a-f]{64}")
EMPTY_SHA256 = hashlib.sha256(b"").hexdigest()
PUBLIC_PATH = re.compile(r"(?:/[A-Za-z0-9_.+ -]+)+")
MAX_BYTES = 1024 * 1024
HANDOFF_DIR = Path(__file__).resolve().parents[1] / "docs/review/catalog-entry-adapter-20261118"
# Owner's promotion receipt promote-f2889bdb575f. The inventory pins the expected snapshot.
HANDOFF_INVENTORY_SHA256 = "d9e493d5f9f81d22552fca875817e7ba1841703a50cc047113dabaaf9bfea4c7"


class Target(NamedTuple):
    owner: str
    role: str
    absent: bool


def digest(value):
    return isinstance(value, str) and SHA256.fullmatch(value) is not None


def unique_members(pairs):
    result = dict(pairs)
    if len(result) != len(pairs):
        raise ValueError("duplicate JSON member")
    return result


def read_input(path, label, pin=None):
    if pin is not None and not digest(pin):
        raise ValueError(f"{label}: invalid frozen SHA-256")
    try:
        if str(path).startswith(("//", "\\\\")) or not path.is_file():
            raise ValueError(f"{label}: expected a regular local file")
        with path.open("rb") as stream:
            identity = os.fstat(stream.fileno())
            raw = stream.read(MAX_BYTES + 1)
    except OSError:
        raise ValueError(f"{label}: could not read the input file") from None
    if len(raw) > MAX_BYTES:
        raise ValueError(f"{label}: input exceeds the size limit")
    if pin is not None and hashlib.sha256(raw).hexdigest() != pin:
        raise ValueError(f"{label}: frozen SHA-256 does not match")
    try:
        data = json.loads(raw.decode("utf-8"), object_pairs_hook=unique_members)
    except (ValueError, RecursionError):
        raise ValueError(f"{label}: invalid UTF-8 JSON or duplicate member") from None
    return data, (identity.st_dev, identity.st_ino)


def fields(value, keys, label):
    if not isinstance(value, dict) or set(value) != set(keys):
        raise ValueError(f"{label}: missing or unexpected fields")


def check_path(path, label):
    if not isinstance(path, str) or len(path) > 4096 or not PUBLIC_PATH.fullmatch(path) or any(
        part in (".", "..") or part != part.strip() or part.endswith(".")
        for part in path.split("/")[1:]
    ):
        raise ValueError(f"{label}: invalid public installation path")


def inventory_paths(inventory):
    fields(inventory, ("expected_sha256", *OWNERS), "inventory")
    if not digest(inventory["expected_sha256"]):
        raise ValueError("inventory: invalid expected snapshot SHA-256")
    result, spellings = {}, {}
    for owner in OWNERS:
        lists = inventory[owner]
        optional = ("config", "asset") if owner == "aidoku" else ("config",)
        roles = REQUIRED_ROLES + tuple(role for role in optional if isinstance(lists, dict) and role in lists)
        fields(lists, roles, f"inventory {owner}")
        for role in roles:
            label = f"{owner} {role}"
            paths = lists[role]
            if not isinstance(paths, list) or not paths:
                raise ValueError(f"inventory: {label} needs at least one public file")
            for item in paths:
                absent = isinstance(item, dict)
                if absent:
                    fields(item, ("path", "state"), f"inventory {label}")
                    if item["state"] != "absent":
                        raise ValueError(f"inventory {label}: only an explicit absent declaration is valid")
                path = item["path"] if absent else item
                check_path(path, f"inventory {label}")
                target = Target(owner, role, absent)
                group = spellings.setdefault(path.lower(), [])
                if path in result or (group and not (len(group) == 1 and owner == "bilicomics"
                                                     and absent and result[group[0]] == target)):
                    raise ValueError(f"inventory {label}: duplicate installation path or case alias")
                result[path] = target
                group.append(path)
            if owner == "aidoku" and role in ("program", "entry") and not any(isinstance(p, str) for p in paths):
                raise ValueError(f"inventory: {label} needs a positive file hash")
    for path, target in result.items():
        if not target.absent and any(str(p).lower() in spellings for p in PurePosixPath(path).parents):
            raise ValueError("inventory: a protected file or absent path cannot contain a present file")
    return result


def describe(inventory, path):
    target = inventory[path]
    return f"{target.owner} {target.role}: {path}"


def snapshot(data, inventory, label):
    fields(data, ("registration", "files"), label)
    registration, files = data["registration"], data["files"]
    fields(registration, ("runtime_version", "projection_sha256"), f"{label} registration")
    version = registration["runtime_version"]
    if not isinstance(version, str) or not re.fullmatch(r"v1-[0-9a-f]{64}", version):
        raise ValueError(f"{label}: unversioned or malformed registration target")
    if version == "v1-" + EMPTY_SHA256:
        raise ValueError(f"{label}: empty registered runtime identity")
    if not digest(registration["projection_sha256"]) or registration["projection_sha256"] == EMPTY_SHA256:
        raise ValueError(f"{label}: invalid or empty registration projection SHA-256")
    if not isinstance(files, dict):
        raise ValueError(f"{label}: expected a map of public file hashes")
    seen = {}
    for path, witness in files.items():
        check_path(path, label)
        target = inventory.get(path)
        if witness == ABSENT:
            if target is None:
                raise ValueError(f"{label}: absent witness outside the frozen inventory")
        elif witness == PRESENT and label == "observed" and target and target.absent:
            pass  # This witness can only report the appearance of a frozen absent path.
        elif not digest(witness):
            raise ValueError(f"{label}: invalid protected file SHA-256 or state")
        elif witness == EMPTY_SHA256 and target and target.role in ("program", "entry"):
            raise ValueError(f"{label}: empty {describe(inventory, path)}")
        if label == "expected" and target and (witness == ABSENT) != target.absent:
            raise ValueError(f"expected: presence contradicts the frozen inventory for {describe(inventory, path)}")
        other = seen.setdefault(path.lower(), path)
        if other != path:
            if not target or other not in inventory:
                raise ValueError(f"{label}: duplicate installation path or unapproved case alias")
            appeared = [p for p in (other, path) if files[p] != ABSENT]
            if appeared:
                details = "; ".join(f"appeared {describe(inventory, p)}" for p in appeared)
                raise ValueError(f"{label}: {details}; case aliases must both remain explicitly absent")
    return registration, files


def coverage(inventory, files):
    problems = [f"missing {describe(inventory, path)} (no witness)" for path in sorted(inventory.keys() - files)]
    extra = files.keys() - inventory
    if extra:
        problems.append(f"{len(extra)} file(s) outside the frozen inventory")
    return problems


def compare(args):
    approved, inventory_id = read_input(args.inventory, "inventory", args.inventory_sha256)
    paths = inventory_paths(approved)
    expected, expected_id = read_input(args.expected, "expected", approved["expected_sha256"])
    before, old_files = snapshot(expected, paths, "expected")
    incomplete = coverage(paths, old_files)
    if incomplete:
        raise ValueError("invalid frozen snapshot: " + "; ".join(incomplete))
    observed, observed_id = read_input(args.observed, "observed")
    if len({inventory_id, expected_id, observed_id}) != 3:
        raise ValueError("supply three distinct files, including a separate observation")
    after, new_files = snapshot(observed, paths, "observed")
    problems = coverage(paths, new_files)
    if before["runtime_version"] != after["runtime_version"]:
        problems.append("registered runtime differs from the frozen version")
    if before["projection_sha256"] != after["projection_sha256"]:
        problems.append("shared registration projection changed")
    for path in sorted(paths.keys() & new_files):
        if old_files[path] != new_files[path]:
            change = "appeared" if old_files[path] == ABSENT else "disappeared" if new_files[path] == ABSENT else "changed"
            problems.append(f"{change} {describe(paths, path)}")
    return problems, len(paths), sum(target.absent for target in paths.values())


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    scope = parser.add_mutually_exclusive_group(required=True)
    scope.add_argument("--aidoku-handoff", action="store_true", help="use the pinned owner deployment reference")
    scope.add_argument("--inventory", type=Path, help="previously approved joint inventory")
    parser.add_argument("--inventory-sha256", help="independently recorded SHA-256 of an explicit inventory")
    parser.add_argument("--expected", type=Path, help="separately frozen snapshot for an explicit inventory")
    parser.add_argument("--observed", required=True, type=Path, help="independently collected observation")
    args = parser.parse_args()
    if args.aidoku_handoff:
        if args.expected is not None or args.inventory_sha256 is not None:
            parser.error("--aidoku-handoff cannot override the pinned reference")
        args.inventory = HANDOFF_DIR / "post-deployment-inventory.json"
        args.inventory_sha256 = HANDOFF_INVENTORY_SHA256
        args.expected = HANDOFF_DIR / "post-deployment-protection.json"
    elif args.expected is None or args.inventory_sha256 is None:
        parser.error("--inventory requires --inventory-sha256 and --expected")
    try:
        problems, count, absent = compare(args)
    except ValueError as error:
        print(error, file=sys.stderr)
        return 2
    if problems:
        print("\n".join(problems), file=sys.stderr)
        return 1
    print(f"Supplied registration and {count} protected paths match the frozen evidence ({absent} explicitly absent).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
