#!/usr/bin/env python3
"""Require complete installed acceptance for one Wisp commit and compiler pin."""
import argparse
import json
from pathlib import Path

HERE = Path(__file__).resolve().parent
REQUIRED_CHECKS = {
    'fresh checksum-pinned compiler installation',
    'installed Node package: records, enums, bytes, options, results, repeated state and errors',
    'deterministic rebuild; body/resource changes invalidate bytes and preserve template',
    'API edit provisions a new contract',
    'initialization failure leaves previous output intact',
    'src layout with locked wheel/vendor metadata and target markers/extras',
    'authored contract and local package imports',
    'installed compiler inventory unchanged after all builds',
}


def validate(result, acquisition, pin, commit, before, after):
    if (result.get('wispCommit') != commit or result.get('toolchainPin') != pin
            or acquisition.get('pin') != pin):
        raise ValueError('acceptance evidence differs from the tested Wisp commit or compiler pin')
    if any(result.get(key) is not True for key in ('passed', 'complete', 'cleanInstallVerified')):
        raise ValueError('installed acceptance did not complete successfully')
    if not REQUIRED_CHECKS <= set(result.get('checks', [])):
        raise ValueError('installed acceptance omitted required consumer or invalidation checks')
    if (before.get('valid') is not True or after.get('valid') is not True
            or not before.get('packages') or before['packages'] != after.get('packages')):
        raise ValueError('installed compiler inventory changed or was not verified')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--wisp-commit', required=True)
    parser.add_argument('--out', required=True, type=Path)
    parser.add_argument('--acquisition', required=True, type=Path)
    args = parser.parse_args()
    def read(path):
        return json.loads(path.read_text())
    validate(read(args.out / 'result.json'), read(args.acquisition),
             read(HERE / 'toolchain.lock.json'), args.wisp_commit,
             read(args.out / 'doctor-before.json'), read(args.out / 'doctor-after.json'))
    print('Complete Python installed acceptance verified for', args.wisp_commit)


if __name__ == '__main__':
    main()
