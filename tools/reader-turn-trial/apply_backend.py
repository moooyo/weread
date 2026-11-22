"""Install the Reader measurement witness into a disposable source export."""

import argparse
from pathlib import Path


def once(source, before, after):
    if source.count(before) != 1:
        raise ValueError(f"Expected one original backend anchor: {before[:100]!r}")
    return source.replace(before, after, 1)


def apply(root):
    if (root / ".git").exists():
        raise ValueError("Use a disposable source export, not a working checkout")
    base = root / "vendor/slint-backend-kindle/src"
    library = base / "lib.rs"
    platform = base / "platform.rs"
    framebuffer = base / "framebuffer/mod.rs"
    probe = base / "reader_trial.rs"
    if probe.exists():
        raise ValueError("The backend witness already exists")
    module = Path(__file__).with_name("backend.rs").read_text()
    tests = Path(__file__).with_name("backend_tests.rs").read_text()
    lib = once(library.read_text(), "mod refresh;", "pub mod reader_trial;\nmod refresh;")
    loop = platform.read_text()
    loop = once(loop, "        loop {\n            if self.quit_flag.load", "        loop {\n            crate::reader_trial::new_iteration();\n            if self.quit_flag.load")
    render = "                let dirty = renderer.render(&mut rgb_buffer, width);"
    loop = once(loop, render, "                crate::reader_trial::draw_begin(!force_full && !mode_changed && observed_frame.is_none() && image_snapshot.regions.is_empty());\n" + render + "\n                crate::reader_trial::draw_end();")
    after_write = "            if force_full || observed_frame.is_some() {\n                if has_raw_images {"
    loop = once(loop, after_write, "            crate::reader_trial::written(written.region, !force_full && !mode_changed && !written.mode_changed && observed_frame.is_none() && !previous_raw_images && !has_raw_images);\n" + after_write)
    fb = framebuffer.read_text()
    start = "        match self.pack_update_request(variant, region, waveform, mode, marker) {"
    end = "            None => false,\n        }\n    }\n\n    fn pack_update_request("
    if fb.count(start) != 1 or fb.count(end) != 1:
        raise ValueError("Unexpected SEND dispatch")
    prefix, rest = fb.split(start); original, suffix = rest.split(end)
    for request in ("MXCFB_SEND_UPDATE", "MXCFB_SEND_UPDATE_REX", "MXCFB_SEND_UPDATE_ZELDA", "MXCFB_SEND_UPDATE_MTK"):
        call = f"self.send_update_ioctl({request}, &update)"
        original = once(original, call, f"({call}, update.waveform_mode)")
    fb = prefix + start.replace("match self", "let (accepted, sent_waveform) = match self") + original + '''            None => return false,
        };
        if accepted {
            crate::reader_trial::accepted(&self.file, marker, mode == UPDATE_MODE_FULL, sent_waveform,
                [region.left, region.top, region.width, region.height], Self::wait_for_reader_trial_marker);
        }
        accepted
    }

    fn wait_for_reader_trial_marker(file: &std::fs::File, marker: u32) -> bool {
        let mut record = UpdateMarkerData { update_marker: marker, collision_test: 0 };
        let result = unsafe { libc::ioctl(file.as_raw_fd(), MXCFB_WAIT_FOR_UPDATE_COMPLETE as _, &mut record) };
        result >= 0 && record.update_marker == marker && record.collision_test == 0
    }

    fn pack_update_request(''' + suffix
    for path, contents in [(probe, module + "\n" + tests), (library, lib), (platform, loop), (framebuffer, fb)]:
        path.write_text(contents)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    apply(parser.parse_args().source)
