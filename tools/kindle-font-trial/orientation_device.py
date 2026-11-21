"""Render the explicit orientation transaction with reviewed, unchanged Kindle readers."""

from pathlib import Path
import re


HERE = Path(__file__).resolve().parent


def section(source, begin, end):
    if source.count(begin) != 1 or source.count(end) != 1 or source.index(begin) >= source.index(end):
        raise ValueError("Review the shared Kindle reader boundaries")
    return source[source.index(begin):source.index(end)].rstrip()


def render_device(arm_a_bytes: int, arm_b_bytes: int, *, reconciled=False) -> str:
    if any(type(size) is not int or not 0 < size <= 110 * 1024**2 for size in (arm_a_bytes, arm_b_bytes)):
        raise ValueError("Invalid retained ARM size")
    common = (HERE / "device.sh").read_text(encoding="ascii")
    source = (HERE / "orientation-device.sh").read_text(encoding="ascii")
    parameters = {
        "@FONT_COMMON@": section(common, "fail() {", "peer_proof() {"),
        "@FONT_GUI@": section(common, "gui_owned() {", "archive_public() {"),
        "@ARM_A_BYTES@": str(arm_a_bytes), "@ARM_B_BYTES@": str(arm_b_bytes),
        "@EXPERIMENT_ARM@": '-v arm="$arm" ' if reconciled else '',
        "@EXPERIMENT_OUTCOME@": '!(arm=="B" && $3=="event_loop_orientation_reconciled") && ' if reconciled else '',
    }
    for key, value in parameters.items():
        if source.count(key) != 1:
            raise ValueError(f"Invalid orientation parameter: {key}")
        source = source.replace(key, value)
    if re.search(r"@[A-Z][A-Z_]*@", source) or len(source.encode("ascii")) > 32768:
        raise ValueError("Invalid rendered orientation collector")
    return source
