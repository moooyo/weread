"""Check the disposable application overlay without opening a display or a device."""

import tempfile
import unittest
from pathlib import Path

from apply_app import HERE, add_arm_probe, apply


class ApplicationOverlayTests(unittest.TestCase):
    def export(self, parent):
        source = Path(parent) / "export"
        (source / "src/app").mkdir(parents=True)
        repository = HERE.parents[1]
        for name in ("src/main.rs", "src/app/mod.rs"):
            (source / name).write_bytes((repository / name).read_bytes())
        return source

    def test_uses_normal_tick_and_waits_for_successful_shutdown(self):
        with tempfile.TemporaryDirectory() as parent:
            source = self.export(parent)
            apply(source)
            main = (source / "src/main.rs").read_text()
            self.assertLess(main.index("Driver::configure(&arguments)"), main.index("App::attach("))
            self.assertLess(main.index("timer_application.borrow_mut().tick()"), main.index("driver.tick(&ui"))
            self.assertLess(main.index("terminal_shutdown()"), main.index("driver.finish()?"))
            self.assertLess(main.index("if !messages.is_empty()"), main.index("driver.finish()?"))
            for location, original in [("src/trial_reader_turn.rs", "app_driver.rs"),
                                       ("src/app/trial_reader_probe.rs", "app_probe.rs")]:
                self.assertEqual((source / location).read_bytes(), (HERE / original).read_bytes())
            with self.assertRaises(FileExistsError):
                apply(source)

    def test_rejects_drift_before_writing(self):
        with tempfile.TemporaryDirectory() as parent:
            source = self.export(parent)
            main = source / "src/main.rs"
            drift = main.read_text().replace("    let timer_application = application.clone();", "")
            main.write_text(drift)
            with self.assertRaises(ValueError):
                apply(source)
            self.assertEqual(main.read_text(), drift)
            self.assertFalse((source / "src/trial_reader_turn.rs").exists())
            self.assertFalse((source / "src/app/trial_reader_probe.rs").exists())

    def test_arm_probe_is_first_and_idempotent_on_a_staged_tree(self):
        with tempfile.TemporaryDirectory() as parent:
            source = self.export(parent)
            apply(source)
            main = (source / "src/main.rs").read_text()
            call = main.index("trial_reader_turn::probe_build_arm()")
            self.assertLess(main.index("fn main() -> std::process::ExitCode"), call)
            self.assertLess(call, main.index('startup_mark("main_enter")'))
            self.assertLess(call, main.index("Arguments::parse()"))
            self.assertEqual(add_arm_probe(main), main)
            with self.assertRaises(ValueError):
                add_arm_probe(main + "trial_reader_turn::probe_build_arm()")

    def test_refuses_repository_and_linked_destinations(self):
        with tempfile.TemporaryDirectory() as parent:
            source = self.export(parent)
            marker = source / ".git"
            marker.touch()
            with self.assertRaises(ValueError):
                apply(source)
            marker.unlink()
            target = Path(parent) / "missing"
            (source / "src/trial_reader_turn.rs").symlink_to(target)
            with self.assertRaises(FileExistsError):
                apply(source)
            self.assertFalse(target.exists())


if __name__ == "__main__":
    unittest.main()
