"""Run one startup turn; an absent operator does not take a lease."""

import argparse
import json
from pathlib import Path

from prepare_pose_flip import schedule, verify_plan, verify_recovery
from run import run, transport


def execute(args):
    work = args.work.resolve(strict=True)
    plan = args.plan.resolve(strict=True)
    if plan == work or not plan.is_relative_to(work):
        raise ValueError('Use a fresh private startup-turn plan in the designated worker')
    config = (verify_recovery if args.recover else verify_plan)(plan)
    schedule(config)
    transport((plan / 'device.sh').read_text(encoding='ascii'), 'admit', config['label'], 'none')
    if args.preflight:
        print(json.dumps(dict(label=config['label'], validated=True, device_accessed=False, lease_requested=False)))
        return 0
    if not args.owner or not args.lease_tool or (not args.recover and not args.operator_ready):
        raise ValueError('A cooperating operator must be ready for the single turn before acquiring the device')
    return run(args, startup_flip_schedule=schedule)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('work', 'plan'):
        parser.add_argument('--' + name, required=True, type=Path)
    parser.add_argument('--owner')
    parser.add_argument('--lease-tool', type=Path)
    parser.add_argument('--operator-ready', action='store_true', help='The operator is present and ready for one physical turn after the splash appears.')
    modes = parser.add_mutually_exclusive_group()
    modes.add_argument('--preflight', action='store_true')
    modes.add_argument('--recover', type=Path)
    parser.set_defaults(wait=0, admit_only=False)
    raise SystemExit(execute(parser.parse_args()))
