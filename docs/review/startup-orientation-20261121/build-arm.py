"""Build the orientation arms or run their focused backend tests in the Ubuntu wrapper."""

import argparse
import json
import os
from pathlib import Path
import re
import runpy
import shutil
import subprocess
import tomllib


SOURCE = Path("/work/src")
OUTPUT = Path("/work/arm-pair")
TARGET = OUTPUT / "target"
ORIENTATION = "startup-orientation-reuse"
PATCHES = ("i-slint-core", "swash", "zeno", "zip")
TEST_MODULE = "platform::startup_orientation_reuse_tests::"
EXPECTED_TESTS = {
    TEST_MODULE + "both_sites_reuse_confirmed_portraits_and_preserve_independent_touch_mapping",
    TEST_MODULE + "missing_changed_or_unreadable_evidence_runs_the_complete_fallback_once",
    TEST_MODULE + "only_the_unchanged_initial_request_can_seed_the_event_loop",
}


def build(lane):
    helper = runpy.run_path(str(SOURCE / "docs/review/library-startup-latency-20261119/rebuild-arm.py"))
    abi = helper["ABI"]
    with (SOURCE / "Cargo.toml").open("rb") as manifest:
        project = tomllib.load(manifest)
    if not (SOURCE / "Cargo.lock").is_file() or not (SOURCE / ".cargo/config.toml").is_file():
        raise ValueError("The source must include the project lock and Cargo configuration")
    if lane == "baseline" and TARGET.exists():
        raise ValueError("The baseline requires a fresh project-private ARM target")
    if lane == "candidate" and not (OUTPUT / "baseline/result.json").is_file():
        raise ValueError("Build the baseline first")
    lane_dir = OUTPUT / lane
    lane_dir.mkdir(parents=True, exist_ok=False)
    features = ["kindle", "bundled-font-preflight"] + ([ORIENTATION] if lane == "candidate" else [])
    common = ["--locked", "--offline", "--release", "--target", abi]
    command = ["cargo", "zigbuild", *common, "--no-default-features", "--features", ",".join(features),
               "--bin", "weread", "--message-format=json-render-diagnostics"]
    environment = dict(os.environ, CARGO_TARGET_DIR=str(TARGET), CARGO_NET_OFFLINE="true", CARGO_INCREMENTAL="0")
    with (lane_dir / "build.log").open("w") as log:
        if lane == "candidate":
            subprocess.run(["cargo", "clean", *common, "-p", "weread", "-p", "slint-backend-kindle"],
                           cwd=SOURCE, env=environment, stdout=log, stderr=subprocess.STDOUT, check=True)
        with subprocess.Popen(command, cwd=SOURCE, env=environment, stdout=subprocess.PIPE,
                              stderr=log, text=True) as cargo:
            messages = [json.loads(line) for line in cargo.stdout]
            if cargo.wait():
                raise RuntimeError(f"Cargo failed; see {lane_dir / 'build.log'}")

    app, declarations = helper["product_outputs"](messages)
    artifacts = [entry for entry in messages if entry["reason"] == "compiler-artifact"]
    backend, = [entry for entry in artifacts if entry["target"]["name"] == "slint_backend_kindle"
                and "lib" in entry["target"]["kind"]]
    if Path(app["manifest_path"]) != SOURCE / "Cargo.toml" or sorted(app["features"]) != sorted(features):
        raise ValueError("Unexpected application manifest or features")
    if (Path(backend["manifest_path"]) != SOURCE / "vendor/slint-backend-kindle/Cargo.toml"
            or backend["features"] != ([ORIENTATION] if lane == "candidate" else [])):
        raise ValueError("Unexpected Kindle backend manifest or features")
    manifests = {entry["manifest_path"] for entry in artifacts}
    if any(str(SOURCE / "vendor" / name / "Cargo.toml") not in manifests for name in PATCHES):
        raise ValueError("Expected the local product dependencies in Cargo's artifact stream")
    generated = declarations.read_text()
    faces = re.findall(r"^static ([A-Z_]+)_BYTES:", generated, re.MULTILINE)
    receipts = re.findall(r"^    \(([A-Z_]+), font_budget::CffLoopCharge", generated, re.MULTILINE)
    if len(faces) != 7 or len(set(faces)) != 7 or receipts != faces:
        raise ValueError("Expected seven distinct font faces and their matching receipts")
    executable = Path(app["executable"])
    if executable != TARGET / abi / "release/weread":
        raise ValueError("Unexpected cross-compiled executable path")
    elf = helper["inspect_elf"](executable)
    qemu = subprocess.run(["qemu-arm", "-cpu", "cortex-a8", executable, "--version"],
                          check=True, text=True, capture_output=True, timeout=30)
    version = qemu.stdout.strip()
    if version != f"weread {project['package']['version']}":
        raise ValueError(f"Unexpected program version: {version}")
    shutil.copy2(executable, lane_dir / "weread")
    result = {
        "lane": lane, "source": str(SOURCE), "command": command, "abi": abi,
        "release_profile": project["profile"]["release"], "application_features": app["features"],
        "backend_features": backend["features"], "both_recompiled": True, "local_dependencies": PATCHES,
        "compiler_flags": {key: value for key, value in environment.items() if key.endswith("RUSTFLAGS")},
        "generated_receipts": receipts, "elf": elf, "qemu_cpu": "cortex-a8", "version_output": version,
    }
    (lane_dir / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result), flush=True)


def lock_identities(lock):
    with lock.open("rb") as stream:
        return {(item["name"], item["version"], item.get("source")) for item in tomllib.load(stream)["package"]}


def test_backend():
    for lane in ("baseline", "candidate"):
        if not (OUTPUT / lane / "result.json").is_file() or not (OUTPUT / lane / "weread").is_file():
            raise ValueError("Build and retain both application arms before running the backend tests")
    if not TARGET.is_dir():
        raise ValueError("Use the same private Cargo target as the application arms")
    helper = runpy.run_path(str(SOURCE / "docs/review/library-startup-latency-20261119/rebuild-arm.py"))
    abi = helper["ABI"]
    output = OUTPUT / "backend-tests"
    output.mkdir()
    scratch = OUTPUT / "backend-test-src"
    shutil.copytree(SOURCE / "vendor/slint-backend-kindle", scratch)
    shutil.copyfile(SOURCE / "Cargo.lock", scratch / "Cargo.lock")
    environment = dict(os.environ, CARGO_TARGET_DIR=str(TARGET), CARGO_NET_OFFLINE="true", CARGO_INCREMENTAL="0")
    patches = {name: SOURCE / "vendor" / name for name in ("i-slint-core", "swash", "zeno")}
    with (SOURCE / "Cargo.toml").open("rb") as stream:
        profile = tomllib.load(stream)["profile"]["release"]
    overrides = {f"patch.crates-io.{name}.path": str(path) for name, path in patches.items()}
    overrides.update({f"profile.release.{key}": value for key, value in profile.items()})
    config = []
    for key, value in overrides.items():
        config.extend(["--config", f"{key}={json.dumps(value)}"])
    command = ["cargo", "zigbuild", "--locked", "--offline", "--release", "--target", abi, "--tests",
               "--no-default-features", "--features", ORIENTATION, "--message-format=json-render-diagnostics", *config]
    with (output / "build.log").open("w") as log:
        metadata = json.loads(subprocess.check_output(
            ["cargo", "metadata", "--offline", "--format-version", "1", "--filter-platform", abi, *config],
            cwd=scratch, env=environment, stderr=log,
        ))
        identities = lock_identities(scratch / "Cargo.lock")
        if not identities <= lock_identities(SOURCE / "Cargo.lock"):
            raise ValueError("The standalone lock must contain only identities from the application lock")
        manifests = {entry["manifest_path"] for entry in metadata["packages"] if entry["source"] is None}
        if {str(path / "Cargo.toml") for path in patches.values()} - manifests:
            raise ValueError("A required local backend patch is missing")
        cargo_output = subprocess.check_output(command, cwd=scratch, env=environment, stderr=log, text=True)
    artifacts = (json.loads(line) for line in cargo_output.splitlines())
    test, = [entry for entry in artifacts if entry["reason"] == "compiler-artifact" and entry["profile"]["test"]
             and entry["manifest_path"] == str(scratch / "Cargo.toml") and entry["executable"]]
    if test["features"] != [ORIENTATION] or test["fresh"]:
        raise ValueError("Expected a freshly compiled, candidate-only backend test executable")
    executable = Path(test["executable"])
    if executable.parent != TARGET / abi / "release/deps":
        raise ValueError("Unexpected cross-compiled backend test path")
    elf = helper["inspect_elf"](executable)
    with (output / "qemu.log").open("w") as log:
        subprocess.run(["qemu-arm", "-cpu", "cortex-a8", executable, TEST_MODULE, "--test-threads=1"],
                       stdout=log, stderr=subprocess.STDOUT, check=True, timeout=60)
    transcript = (output / "qemu.log").read_text()
    passed = re.findall(r"^test (\S+) \.\.\. ok$", transcript, re.MULTILINE)
    if set(passed) != EXPECTED_TESTS or len(passed) != len(EXPECTED_TESTS) or "test result: ok. 3 passed; 0 failed" not in transcript:
        raise ValueError("The three orientation tests must pass without running unrelated backend cases")
    result = {"abi": abi, "command": command, "backend_features": test["features"], "release_profile": profile,
              "lock_identities_from_application": len(identities), "local_patches": list(patches), "elf": elf,
              "qemu_cpu": "cortex-a8", "passed": passed}
    (output / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(transcript, end="", flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("lane", choices=("baseline", "candidate", "backend-tests"))
    lane = parser.parse_args().lane
    if lane == "backend-tests":
        test_backend()
    else:
        build(lane)
