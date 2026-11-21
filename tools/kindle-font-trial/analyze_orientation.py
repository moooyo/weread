"""Analyze a completed indexed orientation batch offline, without opening a device."""

import argparse
from collections import Counter
import json
from pathlib import Path
import statistics

from analyze import QUERY_OUTCOMES, analyze, metrics, request_ns
from freeze import SCHEDULE
from orientation_contract import CANDIDATE, CONTROL, observed_portrait, schedule_for


SITES = ("splash", "event_loop")


def require_closed_trial(plan, operations):
    if schedule_for(plan) != SCHEDULE:
        raise ValueError("Expected the full orientation schedule")
    if operations.get("label") != plan["label"]:
        raise ValueError("Plan and operations do not identify the same batch")
    if operations.get("passed") is not True or not operations.get("released") or operations.get("recovery_required"):
        raise ValueError("Incomplete batch or unconfirmed recovery")
    sequence = [("admit", "none"), ("stage", "none")]
    for index, case in enumerate(SCHEDULE):
        sequence.append(("case", case))
        if index == 3:
            sequence.append(("collect", "none"))
    sequence.extend([("restore", "none"), ("collect", "none"), ("close", "none")])
    observed = operations.get("operations", [])
    if ([(item.get("mode"), item.get("case")) for item in observed] != sequence
            or any(item.get("status") != 0 for item in observed)):
        raise ValueError("Missing, repeated, failed or out-of-order operation; retain the failed attempt")


def orientation_sample(request, trace, *, lane=None, allow_reconciliation=False):
    if lane not in (None, 'A', 'B'):
        raise ValueError('Unknown orientation experiment arm')
    outcomes = QUERY_OUTCOMES if lane == 'A' else (*QUERY_OUTCOMES, 'reused')
    if allow_reconciliation and lane == 'B':
        outcomes += ('reconciled',)
    result = metrics(request, trace, outcomes=outcomes)
    result["request_ns"] = request_ns(request)
    stages = list(result["t_from_request_ns"])
    for site, start, end in [("splash", "main_enter", "startup_feedback_ready"),
                             ("event_loop", "touch_opened", "portrait_opened")]:
        prefix = site + "_orientation_"
        begin, finish = prefix + "begin", prefix + result[site]["outcome"]
        if [stage for stage in stages if stage.startswith(prefix)] != [begin, finish]:
            raise ValueError(f"Ambiguous {site} orientation markers")
        if (start not in stages or end not in stages
                or not stages.index(start) < stages.index(begin) < stages.index(finish) < stages.index(end)):
            raise ValueError(f"{site} orientation markers outside their startup stage")
    order = ["startup_feedback_ready", "window_created", "fonts_registered", "software_ready",
             "touch_opened", "portrait_opened", "first_application_frame_send"]
    if [stage for stage in stages if stage in order] != order:
        raise ValueError("Orientation stages outside the primary endpoint")
    return result


def report(plan, operations, archive):
    require_closed_trial(plan, operations)
    qualifications = {}

    def require_matching_batch(raw):
        names = raw.getnames()
        label_path = "out/orientation-batch"
        if label_path not in names or len(set(names)) != len(names):
            raise ValueError("Unbound or ambiguous orientation archive")
        member = raw.getmember(label_path)
        expected = (plan["label"] + "\n").encode("ascii")
        if not member.isfile() or member.size != len(expected) or raw.extractfile(member).read() != expected:
            raise ValueError("Raw samples do not belong to this orientation batch")

        def read(case, leaf):
            member = raw.getmember(f"out/{case}/{leaf}")
            if not member.isfile() or not 0 < member.size <= 64 * 1024:
                raise ValueError(f"{case}: invalid qualification trace")
            return raw.extractfile(member).read()

        for case in SCHEDULE:
            if observed_portrait(read, case) != (plan["holding"], plan["raw_rotation"]):
                raise ValueError(f"{case}: observed portrait differs from the independent control reference")

        for case in SCHEDULE[:4]:
            row = orientation_sample(read(case, "request").decode("ascii"), read(case, "markers").decode("ascii"), lane=case[0])
            if any((row[site]["outcome"] == "reused") != (case[0] == "B") for site in SITES):
                raise ValueError(f"{case}: qualification pictures do not exercise the declared orientation path")
            qualifications[case] = row

    result = analyze(archive, parse_sample=orientation_sample, verify_archive=require_matching_batch)
    for case, capture in result["qualification"].items():
        if capture["aligned_degrees"] != plan["expected_panel_alignment_degrees"]:
            raise ValueError(f"{case}: observed panel rotation differs from the predeclared orientation")
    counts = {lane: {site: Counter() for site in SITES} for lane in "AB"}
    previous_send = None
    for case, row in {**qualifications, **result["samples"]}.items():
        if previous_send is not None and row["request_ns"] <= previous_send:
            raise ValueError(f"{case}: device times overlap or contradict the counterbalanced order")
        previous_send = row["request_ns"] + row["t_from_request_ns"]["first_application_frame_send"]
        for site in SITES:
            outcome = row[site]["outcome"]
            if case in SCHEDULE[6:]:
                counts[case[0]][site][outcome] += 1
    result["orientation_outcomes"] = counts
    result["site_median_ms"] = {lane: {site: statistics.median(
        result["samples"][f"{lane}{index}"][site]["elapsed_ms"] for index in range(1, 7)
    ) for site in SITES} for lane in "AB"}
    primary = result["request_to_send_ms"]
    result["timing_gate_passed"] = (primary["improved_pairs"] >= 5 and primary["median_paired_saving"] > 0
                                    and all(counts["B"][site]["reused"] == 6 for site in SITES))
    result["holding"] = plan["holding"]
    result["raw_rotation"] = plan["raw_rotation"]
    result["qualification_traces"] = qualifications
    result["font_stage_diagnostic_ms"] = result.pop("font_ms")
    result["scope"] = "Programmatic native request to first Library SEND; not hardware acceptance or deployment."
    result["not_measured"] = ["physical pose correctness", "tap to stable ink", "touch response",
                              "wake recovery with changed pose", "all covers visible"]
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for field in ("plan", "operations", "archive", "output"):
        parser.add_argument(field, type=Path)
    args = parser.parse_args()
    result = report(json.loads(args.plan.read_text()), json.loads(args.operations.read_text()), args.archive)
    with args.output.open("x") as output:
        json.dump(result, output, indent=2)
        output.write("\n")
    print(result["scope"], "Timing gate:", result["timing_gate_passed"])
    raise SystemExit(0 if result["timing_gate_passed"] else 1)
