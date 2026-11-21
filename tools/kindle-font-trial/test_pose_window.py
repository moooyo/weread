import unittest

from analyze_orientation import orientation_sample
from pose_window import classify
from test_orientation import records


class PoseWindowTests(unittest.TestCase):
    def setUp(self):
        self.identity = dict(label="0123456789abcdef", visit="B-flip", boot_id="11111111-1111-1111-1111-111111111111",
                             gui_pid=137, gui_birth=85243, clock="CLOCK_MONOTONIC")
        request, markers = records("B-library", 100)
        self.app = dict(launch=self.identity, request=request, markers=markers)
        trace = orientation_sample(request, markers)
        self.start, self.before_draw, self.sent = [trace["request_ns"] + trace["t_from_request_ns"][key] for key in
                                                  ("startup_feedback_ready", "portrait_opened", "first_application_frame_send")]

    def sensor(self, samples):
        return dict(launch=self.identity.copy(), source="evdev_abs_pressure_snapshot", samples=samples)

    def two(self, earliest, latest, origin=15, destination=16):
        return self.sensor([[1, earliest, earliest + 1, origin], [2, latest - 1, latest, destination]])

    def test_both_directions_use_the_outer_bounds_of_the_actual_reads(self):
        early, late = self.start + 1, self.before_draw - 1
        for origin, destination, names in [(15, 18, ("up", "down")), (16, 17, ("down", "up"))]:
            self.assertEqual(classify(self.app, self.two(early, late, origin, destination)),
                             {"coverage": "hit", "from": names[0], "to": names[1], "observed_change_ns": [early, late]})

    def test_observed_change_definitely_before_or_after_window(self):
        for bounds in [(self.start - 20, self.start - 1), (self.sent + 1, self.sent + 20)]:
            self.assertEqual(classify(self.app, self.two(*bounds))["coverage"], "missed")

    def test_ambiguous_edges_or_between_pre_draw_and_post_send_are_not_hits(self):
        for bounds in [(self.start - 1, self.start + 12), (self.start, self.start + 12),
                       (self.start + 1, self.before_draw), (self.before_draw + 1, self.sent - 1),
                       (self.sent - 4, self.sent + 4), (self.sent, self.sent + 5)]:
            self.assertEqual(classify(self.app, self.two(*bounds))["coverage"], "insufficient")

    def test_binding_requires_same_session_and_device_clock(self):
        for field, value in [("label", "f" * 16), ("visit", "A-library"), ("gui_pid", 138), ("gui_birth", 85244),
                             ("clock", "CLOCK_BOOTTIME"), ("boot_id", "22222222-2222-2222-2222-222222222222")]:
            sensor = self.two(self.start + 1, self.before_draw - 1)
            sensor["launch"][field] = value
            self.assertEqual(classify(self.app, sensor)["coverage"], "insufficient")

    def test_lost_partial_repeated_or_mixed_samples_do_not_qualify(self):
        early, late = self.start + 100, self.before_draw - 100
        for samples in [[], [[1, early, early, 15]], [[1, early, early, 15], [2, late, late, 17]],
                        [[1, early, early, 15], [3, late, late, 16]], [[1, early, early, 15], [2, late, late, None]],
                        [[1, early, early, 15], [2, late, late, 16], [3, late + 1, late + 2, 15]],
                        [[1, early, late, 15], [2, late, late + 1, 16]], [[True, early, early, 15], [2, late, early, 16]]]:
            self.assertEqual(classify(self.app, self.sensor(samples))["coverage"], "insufficient")
        sensor = self.two(early, late)
        for source in ("application_log", "xtest", "uinput_replay"):
            self.assertEqual(classify(self.app, sensor | {"source": source})["coverage"], "insufficient")

    def test_no_replacement_for_missing_or_duplicate_app_markers(self):
        for markers in [self.app["markers"].replace("startup_feedback_ready", "splash_begin"),
                        self.app["markers"].replace("first_application_frame_send", "draw_started"),
                        self.app["markers"] + self.app["markers"]]:
            self.assertEqual(classify(self.app | {"markers": markers}, self.two(self.start + 1, self.before_draw - 1))["coverage"], "insufficient")


if __name__ == "__main__":
    unittest.main()
