#!/usr/bin/env python3
"""Exercise the read-only catalog guard with synthetic owner-approved inputs."""

import copy
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[2] / "scripts/check-catalog-guard.py"
ABSENT = {"state": "absent"}
BILI_ROOT = "/fixture/bili-install"
BILI_ENTRY = "/fixture/documents/Bili.sh"
ENTRY_VARIANT = "/fixture/documents/bili.sh"
ASSET = "/fixture/aidoku/assets/cover.png"


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def write_json(path, data):
    path.write_bytes(json.dumps(data, indent=2).encode("ascii"))


class CatalogGuardTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory(prefix="weread-catalog-guard-")
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.inventory = self.root / "inventory.json"
        self.expected = self.root / "expected.json"
        self.observed = self.root / "observed.json"
        self.scope = {
            owner: {role: [f"/fixture/{owner}/{role}"] for role in ("program", "entry", "config")}
            for owner in ("aidoku", "bilicomics")
        }
        self.scope["aidoku"]["program"].append("/fixture/aidoku/public-helper")
        self.baseline = {
            "registration": {
                "runtime_version": "v1-" + sha(b"approved version"),
                "projection_sha256": sha(b"complete agreed registration projection"),
            },
            "files": {path: sha(path.encode("ascii")) for roles in self.scope.values()
                      for paths in roles.values() for path in paths},
        }
        write_json(self.observed, self.baseline)
        self.freeze()

    def freeze(self):
        write_json(self.expected, self.baseline)
        self.scope["expected_sha256"] = sha(self.expected.read_bytes())
        self.freeze_inventory()

    def freeze_inventory(self):
        write_json(self.inventory, self.scope)
        self.pin = sha(self.inventory.read_bytes())

    def freeze_absent_install(self):
        files = self.baseline["files"]
        for paths in self.scope["bilicomics"].values():
            for path in paths:
                del files[path]
        for role, paths in (("program", [BILI_ROOT]), ("entry", [BILI_ENTRY, ENTRY_VARIANT]),
                            ("config", [BILI_ROOT + "/public-profile"])):
            self.scope["bilicomics"][role] = [{"path": path, **ABSENT} for path in paths]
            files.update({path: ABSENT for path in paths})
        self.scope["aidoku"]["asset"] = [ASSET]
        files[ASSET] = sha(b"public asset")
        self.freeze()
        write_json(self.observed, self.baseline)

    def run_guard(self, code, *messages, observed=None, pin=None):
        result = subprocess.run([
            sys.executable, str(SCRIPT), "--inventory", str(self.inventory),
            "--inventory-sha256", self.pin if pin is None else pin,
            "--expected", str(self.expected), "--observed", str(observed or self.observed),
        ], capture_output=True, text=True, timeout=5)
        output = result.stdout + result.stderr
        self.assertEqual(result.returncode, code, output)
        for message in messages:
            self.assertIn(message, output)
        return output

    def test_complete_separate_evidence_matches_without_editing_inputs(self):
        # Observations need semantic equality; only the approved input bytes are pinned.
        self.observed.write_bytes(json.dumps(self.baseline, sort_keys=True).encode("ascii"))
        before = {path: path.read_bytes() for path in self.root.iterdir()}
        self.run_guard(0, "7 protected paths match the frozen evidence", "0 explicitly absent")
        self.assertEqual({path: path.read_bytes() for path in self.root.iterdir()}, before)

    def test_all_changed_missing_and_unexpected_files_are_reported(self):
        files = self.baseline["files"]
        files["/fixture/aidoku/program"] = sha(b"different program")
        files["/fixture/bilicomics/config"] = sha(b"different public config")
        del files["/fixture/bilicomics/entry"]
        files["/fixture/unreviewed-name"] = sha(b"outside scope")
        write_json(self.observed, self.baseline)
        output = self.run_guard(1, "changed aidoku program", "changed bilicomics config",
                                "missing bilicomics entry", "1 file(s) outside")
        self.assertNotIn("unreviewed-name", output)

    def test_projection_and_version_are_independent_gates(self):
        for field, replacement, diagnostic in [
            ("runtime_version", "v1-" + sha(b"older installed version"), "differs from the frozen version"),
            ("projection_sha256", sha(b"changed association, same command"), "registration projection changed"),
        ]:
            with self.subTest(field=field):
                data = copy.deepcopy(self.baseline)
                data["registration"][field] = replacement
                write_json(self.observed, data)
                self.run_guard(1, diagnostic)

    def test_unversioned_target_cannot_be_an_observation_or_a_baseline(self):
        self.baseline["registration"]["runtime_version"] = "/old/unversioned/dispatcher"
        write_json(self.observed, self.baseline)
        self.run_guard(2, "observed: unversioned or malformed registration")
        self.freeze()
        self.run_guard(2, "expected: unversioned or malformed registration")

    def test_matching_but_incomplete_snapshots_do_not_prove_coverage(self):
        del self.baseline["files"]["/fixture/aidoku/public-helper"]
        self.freeze()
        write_json(self.observed, self.baseline)
        self.run_guard(2, "invalid frozen snapshot: missing aidoku program: /fixture/aidoku/public-helper")
        self.baseline["files"].clear()
        self.freeze()
        write_json(self.observed, self.baseline)
        self.run_guard(2, "invalid frozen snapshot", "missing bilicomics config")

    def test_empty_registered_runtime_projection_program_or_entry_is_not_evidence(self):
        for section, field, value, issue in [
            ("registration", "projection_sha256", sha(b""), "empty registration projection"),
            ("registration", "runtime_version", "v1-" + sha(b""), "empty registered runtime"),
            ("files", "/fixture/aidoku/program", sha(b""), "empty aidoku program"),
            ("files", "/fixture/bilicomics/entry", sha(b""), "empty bilicomics entry"),
        ]:
            with self.subTest(field=field):
                original = self.baseline[section][field]
                self.baseline[section][field] = value
                write_json(self.observed, self.baseline)
                self.run_guard(2, "observed:", issue)
                self.freeze()
                self.run_guard(2, "expected:", issue)
                self.baseline[section][field] = original
                self.freeze()
        self.baseline["files"]["/fixture/bilicomics/config"] = sha(b"")
        self.freeze()
        write_json(self.observed, self.baseline)
        self.run_guard(0)

    def test_each_owner_needs_program_and_entry_and_no_declared_role_may_be_empty(self):
        complete = copy.deepcopy(self.scope)
        for owner in ("aidoku", "bilicomics"):
            for role in ("program", "entry", "config"):
                with self.subTest(owner=owner, role=role):
                    self.scope = copy.deepcopy(complete)
                    self.scope[owner][role] = []
                    self.freeze_inventory()
                    self.run_guard(2, f"{owner} {role} needs at least one public file")
            for role in ("program", "entry"):
                self.scope = copy.deepcopy(complete)
                del self.scope[owner][role]
                self.freeze_inventory()
                self.run_guard(2, f"inventory {owner}: missing or unexpected fields")
            self.scope = copy.deepcopy(complete)
            del self.scope[owner]
            self.freeze_inventory()
            self.run_guard(2, "inventory: missing or unexpected fields")

    def test_inventory_and_expected_bytes_are_bound_before_comparison(self):
        self.run_guard(2, "inventory: invalid frozen SHA-256", pin="not a checksum")
        self.run_guard(2, "inventory: frozen SHA-256 does not match", pin=sha(b"wrong inventory"))
        self.expected.write_bytes(self.expected.read_bytes() + b"\n")
        self.run_guard(2, "expected: frozen SHA-256 does not match")
        self.inventory.write_bytes(self.inventory.read_bytes() + b"\n")
        self.run_guard(2, "inventory: frozen SHA-256 does not match")

    def test_observation_must_not_reuse_the_frozen_file_or_a_hard_link(self):
        self.run_guard(2, "separate observation", observed=self.expected)
        alias = self.root / "alias.json"
        os.link(self.expected, alias)
        self.run_guard(2, "separate observation", observed=alias)

    def test_inventory_rejects_aliases_and_duplicate_paths_across_owners(self):
        for value, issue in [("/fixture/aidoku/../public", "invalid public installation path"),
                             ("/fixture//aidoku/public", "invalid public installation path"),
                             ("/fixture/aidoku/program", "duplicate installation path"),
                             ("/fixture/AIDOKU/PROGRAM", "duplicate installation path")]:
            with self.subTest(value=value):
                self.scope["bilicomics"]["config"] = [value]
                self.freeze_inventory()
                self.run_guard(2, issue)

    def test_duplicate_json_members_and_extra_fields_fail_without_echoing_values(self):
        original = self.observed.read_text("ascii")
        samples = [
            original.replace('"files": {', '"files": {}, "files": {'),
            original.replace('"runtime_version": ', '"runtime_version": "secret-sentinel", "runtime_version": '),
            original.replace('"files": {', '"files": {"/fixture/aidoku/program": "secret-sentinel",'),
        ]
        for sample in samples:
            self.observed.write_text(sample, encoding="ascii")
            self.assertNotIn("secret-sentinel", self.run_guard(2, "duplicate member"))
        self.baseline["secret-sentinel"] = "unexpected"
        write_json(self.observed, self.baseline)
        self.assertNotIn("secret-sentinel", self.run_guard(2, "unexpected fields"))

    def test_malformed_and_incomplete_registration_or_file_evidence_fails(self):
        for field, value, issue in [
            ("registration", {"runtime_version": self.baseline["registration"]["runtime_version"]}, "missing or unexpected"),
            ("registration", dict(self.baseline["registration"], projection_sha256="A" * 64), "projection SHA-256"),
            ("files", [], "map of public file hashes"),
            ("files", {"/fixture/aidoku/program": None}, "protected file SHA-256"),
        ]:
            with self.subTest(field=field, issue=issue):
                write_json(self.observed, dict(self.baseline, **{field: value}))
                self.run_guard(2, issue)

    def test_confirmed_absent_install_and_both_case_variants_are_complete_evidence(self):
        self.freeze_absent_install()
        self.run_guard(0, "9 protected paths match the frozen evidence", "4 explicitly absent")
        del self.baseline["files"][ENTRY_VARIANT]
        write_json(self.observed, self.baseline)
        self.run_guard(1, f"missing bilicomics entry: {ENTRY_VARIANT} (no witness)")
        self.freeze()
        self.run_guard(2, "invalid frozen snapshot", "(no witness)")

    def test_synthetic_six_public_files_and_three_absent_paths_without_config(self):
        # Match the historical scope shape using only invented paths and contents.
        self.scope = {
            "aidoku": {
                "entry": ["/fixture/documents/reader.sh"],
                "program": ["/fixture/reader/launch.sh", "/fixture/reader/app", "/fixture/reader/query",
                            "/fixture/shared/frozen-version/entry-dispatch"],
                "asset": [ASSET],
            },
            "bilicomics": {
                "program": [{"path": BILI_ROOT, **ABSENT}],
                "entry": [{"path": path, **ABSENT} for path in (BILI_ENTRY, ENTRY_VARIANT)],
            },
        }
        files = {path: sha(("synthetic " + path).encode("ascii"))
                 for paths in self.scope["aidoku"].values() for path in paths}
        files.update({path: ABSENT for path in (BILI_ROOT, BILI_ENTRY, ENTRY_VARIANT)})
        self.baseline["files"] = files
        self.freeze()
        write_json(self.observed, self.baseline)
        self.run_guard(0, "9 protected paths", "3 explicitly absent")

    def test_optional_config_is_independent_and_requires_every_witness_when_frozen(self):
        full_scope, full_snapshot = copy.deepcopy(self.scope), copy.deepcopy(self.baseline)
        for owner, other in (("aidoku", "bilicomics"), ("bilicomics", "aidoku")):
            with self.subTest(owner=owner):
                self.scope, self.baseline = copy.deepcopy(full_scope), copy.deepcopy(full_snapshot)
                unscoped = self.scope[other].pop("config")[0]
                del self.baseline["files"][unscoped]
                self.freeze()
                write_json(self.observed, self.baseline)
                self.run_guard(0)
                del self.baseline["files"][self.scope[owner]["config"][0]]
                write_json(self.observed, self.baseline)
                self.run_guard(1, f"missing {owner} config:", "(no witness)")

    def test_appearing_directory_disappearing_program_and_changed_asset_are_distinct(self):
        self.freeze_absent_install()
        files = self.baseline["files"]
        files[BILI_ROOT] = {"state": "present"}
        files["/fixture/aidoku/program"] = ABSENT
        files[ASSET] = sha(b"changed public asset")
        write_json(self.observed, self.baseline)
        self.run_guard(1, f"appeared bilicomics program: {BILI_ROOT}", "disappeared aidoku program",
                       f"changed aidoku asset: {ASSET}")
        files[BILI_ROOT] = sha(b"new regular file at a previously absent path")
        write_json(self.observed, self.baseline)
        self.run_guard(1, "appeared bilicomics program")

    def test_only_the_exact_structured_absence_can_witness_a_frozen_absence(self):
        self.freeze_absent_install()
        path = BILI_ROOT + "/public-profile"
        for value in (None, "", "absent", {}, {"state": "absent", "ignored": True}):
            with self.subTest(value=value):
                self.baseline["files"][path] = value
                write_json(self.observed, self.baseline)
                self.run_guard(2, "invalid protected file SHA-256 or state")
        self.baseline["files"][path] = sha(b"")
        write_json(self.observed, self.baseline)
        self.run_guard(1, f"appeared bilicomics config: {path}")
        self.freeze()
        self.run_guard(2, "expected: presence contradicts the frozen inventory")
        self.baseline["files"][path] = {"state": "present"}
        self.freeze()
        self.run_guard(2, "expected: invalid protected file SHA-256 or state")

    def test_presence_only_and_absence_cannot_expand_or_replace_a_required_digest(self):
        self.freeze_absent_install()
        files = self.baseline["files"]
        files[ASSET] = {"state": "present"}
        write_json(self.observed, self.baseline)
        self.run_guard(2, "observed: invalid protected file SHA-256 or state")
        files[ASSET] = ABSENT
        files["/unknown/secret-sentinel"] = ABSENT
        write_json(self.observed, self.baseline)
        self.assertNotIn("secret-sentinel", self.run_guard(2, "absent witness outside"))
        del files["/unknown/secret-sentinel"]
        self.freeze()
        self.run_guard(2, "expected: presence contradicts the frozen inventory")

    def test_case_variants_must_each_remain_absent_and_keep_the_frozen_spellings(self):
        self.freeze_absent_install()
        for path in (BILI_ENTRY, ENTRY_VARIANT):
            with self.subTest(path=path):
                data = copy.deepcopy(self.baseline)
                data["files"][path] = sha(b"appeared")
                write_json(self.observed, data)
                self.run_guard(2, f"appeared bilicomics entry: {path}", "case aliases must both remain explicitly absent")
        for entries in ([{"path": BILI_ENTRY, **ABSENT}, ENTRY_VARIANT],
                        [{"path": p, **ABSENT} for p in (BILI_ENTRY, ENTRY_VARIANT, BILI_ENTRY.upper())],
                        [{"path": BILI_ENTRY, **ABSENT}, {"path": BILI_ENTRY, **ABSENT}]):
            self.scope["bilicomics"]["entry"] = entries
            self.freeze_inventory()
            self.run_guard(2, "duplicate installation path or case alias")

    def test_absence_cannot_hide_the_installed_owner_or_conflict_with_a_present_child(self):
        self.freeze_absent_install()
        complete = copy.deepcopy(self.scope)
        for role in ("program", "entry"):
            self.scope = copy.deepcopy(complete)
            self.scope["aidoku"][role] = [{"path": p, **ABSENT} for p in self.scope["aidoku"][role]]
            self.freeze_inventory()
            self.run_guard(2, f"aidoku {role} needs a positive file hash")
        self.scope = complete
        self.scope["bilicomics"]["config"] = [BILI_ROOT.upper() + "/public-profile"]
        self.freeze_inventory()
        self.run_guard(2, "absent path cannot contain a present file")


if __name__ == "__main__":
    unittest.main(verbosity=2)
