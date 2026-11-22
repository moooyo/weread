"""Build an isolated native Reader study archive from a public test database."""

import argparse
import io
from pathlib import Path
import tarfile


ROOT = Path(__file__).resolve().parents[2]
COMMAND = b'"$APP_DIR/weread" --kindle --device "$PROFILE" --data-dir "$DATA_DIR" >"$DATA_DIR/logs/application-bootstrap.log"'
PROFILE = {"data/device-profile": b"scribe\n", "data/device-allowlist": b"scribe 5.19.5\n"}
EXECUTABLES = {"launch.sh", "app/weread", "app/weread-frame-guard", "app/weread-native-query"}
MEMBERS = EXECUTABLES | PROFILE.keys() | {"data/library.sqlite", "launch-gate.sh"}


def launcher():
    content = (ROOT / "packaging/kindle/launch.sh").read_bytes()
    if content.count(COMMAND) != 1:
        raise ValueError("The monitored launcher command must be unique")
    return content.replace(COMMAND, COMMAND.replace(b' >', b' --offline --screen reader >'))


def write_member(archive, name, reader, size):
    if not 0 < size <= 160 * 1024 ** 2:
        raise ValueError("Unexpected public input size")
    info = tarfile.TarInfo(name)
    info.size = size
    info.mode = 0o755 if name in EXECUTABLES else 0o600
    archive.addfile(info, reader)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("app", "helpers", "seed", "output"):
        parser.add_argument("--" + name, type=Path, required=True)
    args = parser.parse_args()
    if args.seed.name != "library.sqlite" or any(Path(str(args.seed) + suffix).exists() for suffix in ("-wal", "-shm", "-journal")):
        parser.error("Supply the closed public fixture produced by seed.py")
    with args.output.open("xb") as output, tarfile.open(fileobj=output, mode="w|gz", format=tarfile.USTAR_FORMAT, compresslevel=1) as archive:
        for name, body in {**PROFILE, "launch.sh": launcher(), "launch-gate.sh": Path(__file__).with_name("device.sh").read_bytes()}.items():
            write_member(archive, name, io.BytesIO(body), len(body))
        inputs = {"app/weread": args.app, "data/library.sqlite": args.seed}
        for name in ("weread-frame-guard", "weread-native-query"):
            inputs["app/" + name] = args.helpers / name
        for name, path in inputs.items():
            if path.is_symlink() or not path.is_file():
                raise ValueError("An input is not a regular file")
            with path.open("rb") as reader:
                write_member(archive, name, reader, path.stat().st_size)
    print("Prepared public Reader trial archive; no device contacted.")


if __name__ == "__main__":
    main()
