#!/usr/bin/env python3
"""Acquire pinned compiler bytes, package the candidate and install it fresh."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'scripts/ci'))
from runtime import download


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--buildtools', type=Path, required=True)
    parser.add_argument('--runtime', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--flat-install', action='store_true',
                        help='use the buildtools installer and pinned execution host for recipe regression tests')
    args = parser.parse_args()
    tools, runtime, out = args.buildtools.resolve(), args.runtime.resolve(), args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    catalog = json.loads((runtime/'wisp-runtime.json').read_text())
    env = dict(os.environ)
    env.update({k:str(runtime/v) for k,v in catalog['env'].items() if k != 'CARGO_TARGET_DIR'})
    env['PATH'] = os.pathsep.join(str(runtime/p) for p in ('rust/bin','node/bin','bin','js/node_modules/.bin')) + os.pathsep + env['PATH']
    env.update(CARGO_BUILD_JOBS='1', CARGO_INCREMENTAL='0', PYTHONDONTWRITEBYTECODE='1')
    # The supplied environment is only for the verified execution hosts and
    # native consumer tools. Compiler paths below come from fresh installations.
    runtime_host = env['WISP_WASMTIME']
    bootstrap = out/'bootstrap'
    sys.path.insert(0, str(tools))
    import toolchain_packages
    manifest = json.loads((ROOT/'tests/ocaml/compiler-inputs.json').read_text())
    archives = {}
    for spec in manifest['bundles']:
        path = download(spec['url'], spec['sha256'], out/(spec['tool']+'.wisp.zip'), size=spec['size'])
        archives[spec['tool']] = path
    toolchain_packages.install(list(archives.values()), bootstrap)
    # The bytecode compiler/runtime have not changed. Keep their authenticated
    # bootstrap manifest and original hashes when packaging the new adapters.
    shutil.copytree(bootstrap/'ocaml/dist', tools/'ocaml/dist')
    original = json.loads((tools/'ocaml/dist/manifest.json').read_text())
    for relative, expected in original['bootstrapInputs'].items():
        assert digest(tools/'ocaml'/relative) == expected, 'compiler bootstrap inputs changed; rebuild required'
    assert original['bootstrap'] == json.loads((tools/'ocaml/toolchain.lock.json').read_text())
    subprocess.run([sys.executable, tools/'build.py', 'package', 'ocaml', '--source-only', '--output-dir', out/'candidate'], env=env, check=True)
    candidate = out/'candidate/ocaml.wisp.zip'
    toolchain_packages.validate(candidate)
    cli = ROOT/'target/debug/wisp'
    prefix = out/'installed'
    if args.flat_install:
        toolchain_packages.install([archives[name] for name in ('llvm','clang','wasm-tools')] + [candidate], prefix)
    else:
        for name in ('llvm','clang','wasm-tools','ocaml'):
            archive = candidate if name == 'ocaml' else archives[name]
            subprocess.run([cli,'toolchain','install',archive,'--prefix',prefix,'--sha256',digest(archive)],env=env,check=True)
    # Do not retain bootstrap compiler overrides for the installed source test.
    for key in list(env):
        if key.startswith('WISP_') and key not in ('WISP_NODE','WISP_JCO','WISP_WIT_BINDGEN','WISP_NPM_CACHE'):
            del env[key]
    env['WISP_WASMTIME'] = runtime_host
    env['WISP_TOOLCHAIN_HOME'] = str(prefix)
    if args.flat_install:
        env.update({key: str(prefix/value) for key,value in {
            'WISP_OCAML_TOOLCHAIN':'ocaml', 'WISP_LLVM_TOOLCHAIN':'llvm',
            'WISP_LLVM':'llvm/dist/shared', 'WISP_CLANG_RESOURCE_DIR':'clang/include',
            'WISP_WASM_TOOLS_TOOLCHAIN':'wasm-tools', 'WISP_WASM_TOOLS':'wasm-tools/dist/wasm-tools.wasm',
            'WISP_PREVIEW1_ADAPTER':'wasm-tools/dist/wasi_snapshot_preview1.reactor.wasm',
        }.items()})
        # The pinned host executes the compiler; it is not an application compiler.
        env['WISP_WORKER'] = runtime_host
    else:
        text = subprocess.check_output([cli,'toolchain','env','--prefix',prefix],env=env,text=True)
        for line in text.splitlines():
            _, value = shlex.split(line); key, path = value.split('=',1); env[key] = path
        env['WISP_WORKER'] = str(ROOT/'target/debug/wisp-worker')
    env['WISP_OCAML_WASMTIME'] = runtime_host
    # Record exact installed adapter and compiler bytes used by acceptance.
    selected = Path(env['WISP_OCAML_TOOLCHAIN'])
    assert selected.is_relative_to(prefix)
    for name in ('compile.py','contracts.py','bindings.py','backend.py','runner.py'):
        assert digest(selected/name) == digest(tools/'ocaml'/name), name
    report = {'cleanInstall': True, 'buildtoolsCommit': subprocess.check_output(['git','-C',tools,'rev-parse','HEAD'],text=True).strip(),
              'candidateSha256': digest(candidate), 'compilerManifestSha256': digest(selected/'dist/manifest.json'),
              'installedAdapter': str(selected), 'dependencies': manifest['bundles'],
              'installer': 'buildtools' if args.flat_install else 'wisp',
              'executionHostSha256': digest(env['WISP_WORKER'])}
    if not args.flat_install:
        report['wispSha256'] = digest(cli)
    (out/'setup.json').write_text(json.dumps(report,indent=2)+'\n')
    (out/'environment.json').write_text(json.dumps({k:v for k,v in env.items() if k.startswith(('WISP_','CARGO_')) or k=='PATH'},indent=2)+'\n')
    (out/'environment.sh').write_text('\n'.join('export '+k+'='+shlex.quote(v) for k,v in env.items()
                                        if k.startswith(('WISP_','CARGO_')) or k=='PATH')+'\n')


if __name__ == '__main__': main()
