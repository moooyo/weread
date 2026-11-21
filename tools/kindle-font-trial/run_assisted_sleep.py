"""Run or close exactly one operator-assisted Reader cycle on the designated worker."""

import argparse
import json
from pathlib import Path
import re

from assisted_sleep_visit import CONTRACT, verify_plan, verify_recovery
from run import run, transport


def schedule_for_sleep(plan):
    if (not isinstance(plan, dict) or any(plan.get(key) != value for key, value in CONTRACT.items())
            or not isinstance(plan.get('label'), str) or not re.fullmatch('[0-9a-f]{16}', plan['label'])):
        raise ValueError('Not a frozen single Scribe assisted Reader cycle')
    return ['B-reader']


def execute(args):
    work = args.work.resolve(strict=True)
    plan = args.plan.resolve(strict=True)
    if work == plan or not plan.is_relative_to(work):
        raise ValueError('Use a private plan beneath the designated worker')
    config = (verify_recovery if args.recover else verify_plan)(plan)
    schedule_for_sleep(config)
    script = (plan / 'device.sh').read_text(encoding='ascii')
    for operation, name in [('case', 'B-reader'), ('permit', 'recover-wake'), ('close', 'none')]:
        transport(script, operation, config['label'], name)
    if args.preflight:
        print(json.dumps({'label': config['label'], 'purpose': config['purpose'], 'validated': True,
                          'device_accessed': False, 'lease_requested': False}))
        return 0
    if not args.owner or not args.lease_tool or (not args.recover and not args.operator_ready):
        raise ValueError('Require this owner, a lease utility, and a present operator with the live notice bridge')
    return run(args, assisted_sleep_schedule=schedule_for_sleep)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    for option in ('work', 'plan'):
        parser.add_argument('--' + option, type=Path, required=True)
    parser.add_argument('--owner')
    parser.add_argument('--lease-tool', type=Path)
    parser.add_argument('--operator-ready', action='store_true')
    group = parser.add_mutually_exclusive_group()
    group.add_argument('--preflight', action='store_true')
    group.add_argument('--recover', type=Path)
    parser.set_defaults(wait=0, admit_only=False)
    raise SystemExit(execute(parser.parse_args()))
