#!/usr/bin/env python3
"""Authenticate a compiler artifact against an immutable source pin and executed CI steps."""
import argparse
import json
from pathlib import Path
import subprocess

HERE = Path(__file__).resolve().parent
REQUIRED_STEPS = {
    'Build compiler and dependencies from pinned inputs',
    'Test the Wasm compiler',
    'Package verified compiler',
    'Validate bundle and report package sizes',
    'Upload compiler package',
}


def validate(pin, run, jobs, artifacts):
    if (run.get('head_sha') != pin['commit'] or run.get('conclusion') != 'success'
            or run.get('path') != pin['workflow'] or run.get('status') != 'completed'
            or run.get('repository', {}).get('full_name') != pin['repository']):
        raise ValueError('compiler run does not match the successful pinned source/workflow')
    executed = [job for job in jobs['jobs'] if job.get('name') == 'Build python / python'
                and job.get('conclusion') == 'success'
                and REQUIRED_STEPS <= {step['name'] for step in job.get('steps', []) if step.get('conclusion') == 'success'}]
    if len(executed) != 1:
        raise ValueError('real Python compiler, verification and packaging steps did not all execute')
    selected = [artifact for artifact in artifacts['artifacts'] if artifact['name'] == pin['artifact'] and not artifact['expired']]
    if len(selected) != 1 or not selected[0].get('digest', '').startswith('sha256:'):
        raise ValueError('expected one unexpired compiler artifact with a GitHub digest')
    return {'pin': pin, 'run': run['html_url'], 'job': executed[0]['html_url'], 'artifact': selected[0]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--run-id', required=True, type=int)
    parser.add_argument('--out', required=True, type=Path)
    args = parser.parse_args()
    pin = json.loads((HERE / 'toolchain.lock.json').read_text())
    endpoint = f"repos/{pin['repository']}/actions/runs/{args.run_id}"
    def api(path):
        return json.loads(subprocess.check_output(['gh', 'api', path], timeout=60))
    record = validate(pin, api(endpoint), api(endpoint + '/jobs?per_page=100'), api(endpoint + '/artifacts'))
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(record, indent=2) + '\n')
    print(record['run'])


if __name__ == '__main__':
    main()
