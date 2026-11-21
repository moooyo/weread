"""Package the tested ARM programs, their declared loader closure and public OCR models."""

import io
import json
from pathlib import Path
import tarfile


def package(work, destination):
    receipt = json.loads((work / "paired-kit.json").read_text())
    allowed = receipt["members"]
    with tarfile.open(work / "paired-kit.tar.gz", "r:gz") as source, tarfile.open(destination, "x:gz", format=tarfile.USTAR_FORMAT) as output:
        if source.getnames() != allowed or len(allowed) != len(set(allowed)):
            raise ValueError("Unexpected or ambiguous ARM loader bundle")
        required = {"native-target", "card-input", "models/eng.traineddata", "models/chi_sim.traineddata",
                    "lib/ld-linux-armhf.so.3", "lib/libX11.so.6", "lib/libXtst.so.6", "lib/libtesseract.so.5"}
        if not required <= set(allowed):
            raise ValueError("Incomplete native tile runtime")
        for name in [*allowed, "native_tile.sh"]:
            info = tarfile.TarInfo("tile/" + name)
            info.mode = 0o644 if name.startswith("models/") else 0o755
            if name == "native_tile.sh":
                payload = (work / name).read_bytes()
            else:
                member = source.getmember(name)
                if not member.isfile() or not (name in required or (name.startswith("lib/") and len(Path(name).parts) == 2)):
                    raise ValueError("Unexpected ARM package member")
                payload = source.extractfile(member).read()
                if name in ("native-target", "card-input") and payload != (work / name).read_bytes():
                    raise ValueError("Packed executable differs from the tested ARM build")
            info.size = len(payload)
            output.addfile(info, io.BytesIO(payload))


if __name__ == "__main__":
    package(Path("/work"), Path("/work/native-tile-qualified.tar.gz"))
