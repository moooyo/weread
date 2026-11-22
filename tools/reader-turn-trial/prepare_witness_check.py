"""Prepare a tiny Linux / ARM-user-mode test with the exact observer and WAIT adapter."""

import argparse
from pathlib import Path
import re

from apply_backend import apply


def prepare(source, output):
    crate = output / "src"
    crate.mkdir(parents=True, exist_ok=False)
    base = source / "vendor/slint-backend-kindle/src"
    if not (base / "reader_trial.rs").is_file():
        apply(source)
    observer = (base / "reader_trial.rs").read_text()
    if "mod tests {" not in observer:
        observer += "\n" + Path(__file__).with_name("backend_tests.rs").read_text()
    (crate / "reader_trial.rs").write_text(observer)
    actual = (base / "framebuffer/mod.rs").read_text()
    method = re.search(r"    fn wait_for_reader_trial_marker\(.*?\n    \}", actual, re.S)
    if not method:
        raise ValueError("Missing trial marker ioctl")
    ffi = (base / "framebuffer/ffi.rs").read_text()
    start = ffi.index("pub(super) const MXCFB_WAIT_FOR_UPDATE_COMPLETE:")
    stop = ffi.index("\n}", start) + 2
    adapter = "use std::os::fd::AsRawFd;\n" + ffi[start:stop] + "\n" + method.group() + '''

#[test]
fn a_normal_file_descriptor_cannot_acknowledge_an_epdc_marker() {
    let file = std::fs::File::open("/dev/null").unwrap();
    for marker in [1, 11, 43] {
        assert!(!wait_for_reader_trial_marker(&file, marker));
    }
}
'''
    (crate / "ioctl_adapter.rs").write_text(adapter)
    (crate / "lib.rs").write_text('''
#[derive(Clone, Copy)]
pub struct PhysicalRect { pub x: i32, pub y: i32, pub width: u32, pub height: u32 }
pub mod reader_trial;
#[cfg(test)] mod ioctl_adapter;
''')
    dependencies = (source / "Cargo.lock").read_text()
    version = re.search(r'\[\[package\]\]\nname = "libc"\nversion = "([0-9.]+)"', dependencies)
    if not version:
        raise ValueError("Missing locked libc version")
    (output / "Cargo.toml").write_text(f'''[package]
name = "reader-witness-check"
version = "0.0.0"
edition = "2024"
publish = false
[dependencies]
libc = "={version.group(1)}"
''')


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    prepare(args.source, args.output)
