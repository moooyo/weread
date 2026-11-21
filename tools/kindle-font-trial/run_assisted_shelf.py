"""Run one prepared operator-assisted public shelf visit, without reserving the device."""

import argparse
import json
from pathlib import Path
import re

from assisted_shelf import CONTRACT, verify_plan, verify_recovery
from run import run, transport


def schedule_for_shelf(plan):
    if (not isinstance(plan, dict) or any(plan.get(key) != value for key, value in CONTRACT.items())
            or not isinstance(plan.get('label'), str) or not re.fullmatch('[a-f0-9]{16}', plan['label'])):
        raise ValueError('Not a prepared operator-assisted public shelf visit')
    return ['B-library']


def execute(args):
    work = args.work.resolve(strict=True)
    plan = args.plan.resolve(strict=True)
    if plan == work or not plan.is_relative_to(work):
        raise ValueError('Use a prepared plan beneath the private worker directory')
    config = (verify_recovery if args.recover else verify_plan)(plan)
    schedule_for_shelf(config)
    transport((plan / 'device.sh').read_text(encoding='ascii'), 'admit', config['label'], 'none')
    if args.preflight:
        print(json.dumps({'label': config['label'], 'validated': True, 'device_accessed': False, 'lease_requested': False}))
        return 0
    if not args.owner or not args.lease_tool or (not args.recover and not args.operator_ready):
        raise ValueError('An owner, lease utility and present operator are required for an assisted visit')
    return run(args, assisted_shelf_schedule=schedule_for_shelf)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for option in ('work', 'plan'):
        parser.add_argument('--' + option, type=Path, required=True)
    parser.add_argument('--lease-tool', type=Path)
    parser.add_argument('--owner')
    parser.add_argument('--operator-ready', action='store_true', help='The named operator is present and understands the single tap.')
    group = parser.add_mutually_exclusive_group()
    group.add_argument('--preflight', action='store_true')
    group.add_argument('--recover', type=Path)
    parser.set_defaults(wait=0, admit_only=False)
    raise SystemExit(execute(parser.parse_args()))
