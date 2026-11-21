"""Run one prepared orientation experiment on the worker host, without reserving the Kindle."""

import argparse
from pathlib import Path

from orientation_contract import schedule_for
from run import run


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for option in ("work", "plan", "lease-tool"):
        parser.add_argument("--" + option, type=Path, required=True)
    parser.add_argument("--owner", required=True)
    parser.add_argument("--recover", type=Path, help="Close only the original interrupted transaction")
    parser.set_defaults(admit_only=False, wait=0)
    args = parser.parse_args()
    if not args.recover:
        from prepare_orientation import verify_prepared

        verify_prepared(args.plan.resolve(strict=True), args.work.resolve(strict=True))
    raise SystemExit(run(args, orientation_schedule=schedule_for))
