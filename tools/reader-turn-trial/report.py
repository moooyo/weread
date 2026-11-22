"""Compare one or two completed reader-turn pairs from a leased Scribe pilot."""

import argparse
import json
import re
import statistics
from collections import Counter
from pathlib import Path

import seed


TURNS = 24
WARMUPS = 3
WIDTH, HEIGHT = 1860, 2480
U32 = 2**32 - 1
U64 = 2**64 - 1
TIMES = ("press_ns", "release_ns", "model_ns", "draw_begin_ns", "draw_end_ns", "written_ns", "sent_ns")
WAITS = ("wait_begin_ns", "wait_end_ns", "wait_marker")
WAIT_STATUSES = {"acknowledged", "unavailable", "not-witnessed", "timed-out", "worker-unavailable"}
TARGET = ("chapter", "source_page", "target_page", "source_anchor", "target_anchor", "source_line_start", "target_line_start")
FIELDS = set(TIMES + WAITS + TARGET + ("sequence", "warmup", "marker", "full", "rect", "waveform", "wait_status"))
ENDPOINTS = {"release_to_model": "model_ns", "release_to_draw_end": "draw_end_ns",
             "release_to_send_return": "sent_ns", "release_to_matching_wait_return": "wait_end_ns"}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def integer(value, low=0, high=U32):
    return type(value) is int and low <= value <= high


def unique_fields(items):
    result = dict(items)
    require(len(result) == len(items), "duplicate JSON field")
    return result


def read_file(directory, name, limit):
    path = directory / name
    require(path.is_file() and not path.is_symlink(), f"missing regular {name}")
    with path.open("rb") as stream:
        data = stream.read(limit + 1)
    require(len(data) <= limit, f"oversize {name}")
    return data


def check_rows(rows):
    require(len(rows) == TURNS, "expected exactly 24 numbered turns")
    body_scalars = len(seed.public_body())
    previous = None
    for sequence, row in enumerate(rows, 1):
        def check(condition, message):
            require(condition, f"turn {sequence}: {message}")

        check(type(row) is dict and set(row) == FIELDS, "unexpected record fields")
        check(integer(row["sequence"]) and row["sequence"] == sequence, "missing or unordered sequence")
        check(row["warmup"] is (sequence <= WARMUPS), "wrong warmup assignment")
        check(all(integer(row[key]) for key in TARGET), "invalid page or scalar identity")
        check((row["chapter"], row["source_page"], row["target_page"]) == (0, sequence, sequence + 1),
              "not the planned same-chapter forward turn")
        check(row["source_anchor"] <= row["source_line_start"] < row["target_anchor"]
              <= row["target_line_start"] < body_scalars, "inconsistent text anchors")
        check(all(integer(row[key], 1, U64) for key in TIMES), "invalid monotonic timestamp")
        stamps = [row[key] for key in TIMES]
        check(stamps == sorted(stamps) and 45_000_000 <= row["release_ns"] - row["press_ns"] < 480_000_000
              and row["release_ns"] < row["sent_ns"],
              "event timestamps outside this turn")
        check(integer(row["marker"], 1), "invalid SEND marker")
        check(row["full"] is (sequence % 6 == 0), "actual mode differs from the fresh six-turn refresh cycle")
        check(integer(row["waveform"]) and row["waveform"] == (2 if row["full"] else 257),
              "unexpected submitted waveform for the light text fixture")
        rect = row["rect"]
        check(isinstance(rect, list) and len(rect) == 4 and all(integer(value) for value in rect),
              "invalid physical rectangle")
        left, top, width, height = rect
        check(width > 0 and height > 0 and left + width <= WIDTH and top + height <= HEIGHT,
              "rectangle outside the Scribe framebuffer")
        check(isinstance(row["wait_status"], str) and row["wait_status"] in WAIT_STATUSES,
              "invalid wait status")
        if row["wait_status"] == "acknowledged":
            check(integer(row["wait_marker"], 1) and row["wait_marker"] == row["marker"]
                  and integer(row["wait_begin_ns"], 1, U64) and integer(row["wait_end_ns"], 1, U64)
                  and row["sent_ns"] <= row["wait_begin_ns"] <= row["wait_end_ns"],
                  "incomplete or unrelated marker wait")
        else:
            check(all(row[key] is None for key in WAITS), "unacknowledged sample has a completion timestamp or marker")
        if previous is None:
            check(row["source_anchor"] == 0 and row["source_line_start"] == 0, "not the initial public page")
        else:
            check((row["source_anchor"], row["source_line_start"]) ==
                  (previous["target_anchor"], previous["target_line_start"]), "page discontinuity")
            check(row["marker"] > previous["marker"], "reused or out-of-order SEND marker")
            check(row["press_ns"] > (previous["wait_end_ns"] or previous["sent_ns"]), "overlapping turns")
        previous = row


def load_visit(lane, label, location):
    directory = Path(location)
    require(lane in ("A", "B") and re.fullmatch(r"[0-9a-f]{16}", label), "invalid lane or visit ID")
    require(directory.name == f"visit-{label}" and directory.is_dir() and not directory.is_symlink(),
            "input directory does not match this visit")
    receipt = json.loads(read_file(directory, "receipt.json", 131072), object_pairs_hook=unique_fields)
    require(isinstance(receipt, dict) and receipt.get("lane") == lane and receipt.get("label") == label
            and receipt.get("mode") == "native" and type(receipt.get("chapters")) is int
            and receipt["chapters"] == seed.CHAPTERS
            and all(receipt.get(key) is True for key in ("passed", "native_restored", "connection_closed", "released")),
            "lane, public dataset or physical closeout receipt mismatch")
    require(read_file(directory, "turn-complete", 32) == b"24\n", "no 24-turn completion witness")
    raw = read_file(directory, "turn-result.jsonl", 131072)
    require(raw.endswith(b"\n"), "incomplete last turn record")
    rows = [json.loads(line, object_pairs_hook=unique_fields) for line in raw.splitlines()]
    check_rows(rows)
    return lane, label, rows


def describe(values):
    if not values:
        return {"n": 0}
    return {"n": len(values), "median_ms": round(statistics.median(values) / 1e6, 6),
            "min_ms": round(min(values) / 1e6, 6), "max_ms": round(max(values) / 1e6, 6)}


def groups(rows, other=None):
    result = {}
    for name, full in [("partial", False), ("full", True)]:
        measured = [index for index, row in enumerate(rows) if not row["warmup"] and row["full"] is full]
        group = {"sequences": [rows[index]["sequence"] for index in measured]}
        if other is None:
            group["wait_statuses"] = dict(Counter(rows[index]["wait_status"] for index in measured))
        for endpoint, key in ENDPOINTS.items():
            values = []
            for index in measured:
                row = rows[index]
                if row[key] is None or (other is not None and other[index][key] is None):
                    continue
                elapsed = row[key] - row["release_ns"]
                if other is not None:
                    elapsed -= other[index][key] - other[index]["release_ns"]
                values.append(elapsed)
            group[endpoint] = describe(values)
        result[name] = group
    return result


def compare(visits):
    require(len(visits) in (2, 4), "supply one or two complete pairs in execution order")
    require(len({label for _, label, _ in visits}) == len(visits), "a visit was used twice")
    order = "".join(lane for lane, _, _ in visits)
    require(order in ("AB", "BA", "ABBA", "BAAB"), "expected AB or BA, reversed for the second pair")
    reference = [tuple(row[key] for key in TARGET) for row in visits[0][2]]
    for _, _, rows in visits[1:]:
        require([tuple(row[key] for key in TARGET) for row in rows] == reference,
                "visits did not display the same public page and line anchors")
    pairs = []
    for start in range(0, len(visits), 2):
        baseline, candidate = sorted(visits[start:start + 2], key=lambda visit: visit[0])
        pairs.append({"A": baseline[1], "B": candidate[1],
                      "same_target_A_minus_B": groups(baseline[2], candidate[2])})
    return {"scope": "Synthetic Slint pointer release on Scribe 2022 / firmware 5.19.5; not physical touch or optical completion.",
            "clock": "per-visit device CLOCK_MONOTONIC nanoseconds", "warmups_per_visit": WARMUPS,
            "unit": "milliseconds", "order": order,
            "interpretation": "One or two paired visits, not independent per-page trials. Positive paired differences favor B. Missing exact-marker WAITs have no completion estimate.",
            "visits": [{"lane": lane, "visit": label, "timings": groups(rows)} for lane, label, rows in visits],
            "pairs": pairs}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--visit", nargs=3, action="append", required=True, metavar=("LANE", "LABEL", "DIRECTORY"))
    parser.add_argument("--output", type=Path, help="new output file; default: stdout")
    args = parser.parse_args()
    try:
        result = compare([load_visit(*visit) for visit in args.visit])
        text = json.dumps(result, indent=2, allow_nan=False)
        if args.output:
            with args.output.open("x", encoding="utf-8") as output:
                output.write(text + "\n")
        else:
            print(text)
    except (ValueError, OSError) as error:
        parser.exit(1, f"Not timed: {error}\n")


if __name__ == "__main__":
    main()
