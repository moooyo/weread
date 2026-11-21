"""Inspect two disposable userstore files; no production entry or private database is opened."""

import argparse
from pathlib import Path

from run_pose_probe import main


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--runs', type=Path, required=True)
    parser.add_argument('--helper', type=Path)
    parser.add_argument('--lease-tool', type=Path, required=True)
    parser.add_argument('--owner', required=True)
    group = parser.add_mutually_exclusive_group()
    group.add_argument('--preflight', action='store_true')
    group.add_argument('--recover', type=Path)
    parser.set_defaults(display_trace=False, turn_cue=False, operator_ready=False)
    raise SystemExit(main(parser.parse_args(), volume=True))
