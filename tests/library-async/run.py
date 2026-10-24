#!/usr/bin/env python3
"""Install an async Component package, then replay consumers without registries."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(ROOT / "tests/source-build"))
from snapshot import WORKER_NAME, freeze_cli, verify_cli, worker_digest
sys.path.insert(0, str(ROOT / "tests/library-chain"))
from registry import Registries


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def write(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(value)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--output-dir', type=Path, default=ROOT / 'out/library-async-chain')
    parser.add_argument('--replay', action='store_true')
    args = parser.parse_args()
    output = args.output_dir.resolve()
    output.mkdir(parents=True, exist_ok=True)
    node_bin = ROOT / '.tools/library-chain-node/node/bin'
    tools = ROOT / '.tools/component-library-chain'
    target = ROOT / 'out/library-chain/cargo-target'
    env = {**os.environ, 'RUSTUP_TOOLCHAIN': '1.98.1', 'RUSTUP_AUTO_INSTALL': '0',
           'WISP_JCO': str(ROOT / '.tools/library-js/node_modules/.bin/jco'),
           'WISP_NODE': str(node_bin / 'node'), 'CARGO_TARGET_DIR': str(target),
           'CARGO_PROFILE_DEV_DEBUG': '0', 'PATH': str(node_bin) + os.pathsep + os.environ['PATH']}
    commands = []
    def run(argv, cwd=ROOT):
        argv = list(map(str, argv))
        process = subprocess.run(argv, cwd=cwd, env=env, text=True, capture_output=True, timeout=600)
        commands.append({'argv': argv, 'cwd': str(cwd), 'status': process.returncode,
                         'stdout': process.stdout, 'stderr': process.stderr})
        if process.returncode:
            raise RuntimeError(f'{argv!r}\n{process.stdout}{process.stderr}')
        return process.stdout.strip()
    release = output / 'release'
    native, guest, node = [output / name for name in ['native', 'guest', 'node']]
    snapshot = output / 'wisp'
    if not args.replay:
        if release.exists():
            raise RuntimeError('use a fresh output directory or --replay')
        freeze_cli(ROOT / 'target/debug/wisp', snapshot, environment=env)
        source_hashes = {}
        for base in [ROOT / 'crates/wisp/src', ROOT / 'crates/wisp/assets']:
            for source in sorted(base.rglob('*')):
                if not source.is_file():
                    continue
                relative = source.relative_to(ROOT)
                destination = output / 'packaging-sources' / relative
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(source, destination)
                source_hashes[str(relative)] = digest(source)
        write(output / 'packaging-tool.json', json.dumps({'sha256': digest(snapshot), 'worker_sha256': worker_digest(snapshot),
              'sources': source_hashes,
              'commit': run(['git', 'rev-parse', 'HEAD']), 'status': run(['git', 'status', '--porcelain'])}, indent=2)+'\n')
        run(['cargo', 'build', '--locked', '--offline', '--manifest-path', HERE/'guest/Cargo.toml', '--target', 'wasm32-unknown-unknown'])
        author = output / 'author'
        shutil.copytree(HERE / 'guest/wit', author / 'wit')
        run([tools/'wasm-tools', 'component', 'new', target/'wasm32-unknown-unknown/debug/wisp_library_async_fixture.wasm', '-o', author/'component.wasm'])
        write(author/'wisp.toml', f'[package]\nname = "wisp:async-values"\nversion = "0.1.0"\n[component]\npath = "component.wasm"\nsha256 = "{digest(author / "component.wasm")}"\n')
        run([snapshot, 'library', 'pack', author, '--out', release])
        metadata = json.loads(run(['cargo', 'metadata', '--format-version', '1', '--no-deps', '--manifest-path', release/'rust/Cargo.toml']))
        write(native/'src/main.rs', (HERE/'native-consumer.rs').read_text())
        write(guest/'src/lib.rs', (HERE/'component.rs').read_text())
        write(guest/'wit/world.wit', (HERE/'component.wit').read_text())
        write(guest/'wit/deps/library/world.wit', (HERE/'guest/wit/world.wit').read_text())
        write(node/'consumer.mjs', (HERE/'consumer.mjs').read_text())
        write(node/'package.json', '{"name":"async-installed-consumer","version":"0.0.0","private":true,"type":"module"}')
        with Registries(release, metadata) as registry:
            for directory, name in [(native,'async-native-consumer'),(guest,'async-guest-consumer')]:
                write(directory/'.cargo/config.toml', f'[registries.wisp-test]\nindex = "sparse+{registry.url}/cargo/"\n')
                manifest = f'[package]\nname = "{name}"\nversion = "0.0.0"\nedition = "2024"\npublish = false\n[workspace]\n[dependencies]\nwisp-wisp--async-values = {{ version = "=0.1.0", registry = "wisp-test" }}\n'
                if directory == guest:
                    manifest += 'wit-bindgen = { version = "=0.61.1", features = ["async", "async-spawn"] }\n[lib]\ncrate-type = ["cdylib"]\n'
                write(directory/'Cargo.toml', manifest)
                run(['cargo','fetch','--manifest-path',directory/'Cargo.toml'], directory)
            run(['npm','install','--save-exact','--ignore-scripts','--no-audit','--no-fund','--registry',registry.url+'/npm/','--cache',output/'npm-cache','@wisp/async-values@0.1.0'],node)
            assert '/npm/package.tgz' in registry.requests
            assert '/crates/wisp-wisp--async-values/0.1.0/download' in registry.requests
            write(output/'registry-requests.json',json.dumps(registry.requests,indent=2)+'\n')
    report = output / 'report.json'
    report.unlink(missing_ok=True)
    packaging_tool = json.loads((output/'packaging-tool.json').read_text())
    verify_cli(snapshot, packaging_tool['sha256'], packaging_tool.get('worker_sha256'), environment=env)
    assert run(['node','--version']) == 'v24.20.0'
    run([snapshot,'library','validate',release])
    native_result = run(['cargo','run','--locked','--offline','--manifest-path',native/'Cargo.toml'],native)
    assert native_result == 'installed Rust async record + strings + cancellation passed'
    run(['npm','ci','--offline','--ignore-scripts','--no-audit','--no-fund','--cache',output/'npm-cache'],node)
    node_result = run(['node','--experimental-wasm-jspi','consumer.mjs'],node)
    future_limit = json.loads(run(['node','--experimental-wasm-jspi',HERE/'future-cancellation.mjs',release/'node/index.js']))
    assert future_limit['status'] == 'unsupported'
    write(output/'jco-future-cancellation.json',json.dumps(future_limit,indent=2)+'\n')
    assert node_result == 'future record + stream strings + stream cancellation passed'
    run(['cargo','build','--locked','--offline','--target','wasm32-unknown-unknown','--manifest-path',guest/'Cargo.toml'],guest)
    graph = run(['cargo','tree','--locked','--offline','--target','wasm32-unknown-unknown','--edges','normal,build'],guest)
    assert 'wasmtime' not in graph and 'wasmer' not in graph
    write(output/'guest-cargo-tree.txt',graph+'\n')
    run([tools/'wasm-tools','component','new',target/'wasm32-unknown-unknown/debug/async_guest_consumer.wasm','-o',output/'consumer.wasm'])
    write(output/'chain.wac','''package wisp:async-chain;
let library = new wisp:library {};
let consumer = new wisp:consumer { "wisp:async-values/operations@0.1.0": library["wisp:async-values/operations@0.1.0"] };
export consumer["wisp:async-consumer/check@0.1.0"];
''')
    for filename in ['composed.wasm','composed-again.wasm']:
        run([tools/'wac','compose',output/'chain.wac','--dep',f'wisp:library={release / "component.wasm"}','--dep',f'wisp:consumer={output / "consumer.wasm"}','-o',output/filename])
        run([tools/'wasm-tools','validate','--features','all',output/filename])
    assert digest(output/'composed.wasm') == digest(output/'composed-again.wasm')
    run(['node',env['WISP_JCO'],'transpile',output/'composed.wasm','--async-mode','jspi','--name','index','--out-dir',output/'composed-node'])
    write(output/'composed-node/package.json','{"type":"module"}')
    write(output/'composed-node/run.mjs',"import {check} from './index.js'; const result=await check.run(); if(result!==42)throw Error(String(result)); console.log('composed async guest 42');\n")
    node_composed = run(['node','--experimental-wasm-jspi','run.mjs'],output/'composed-node')
    assert node_composed == 'composed async guest 42'
    composed_result = run(['cargo','run','--locked','--offline','--manifest-path',HERE/'native/Cargo.toml','--',output/'composed.wasm','--composed'])
    assert composed_result == 'composed async guest 42'
    artifacts = {str(p.relative_to(output)): digest(p) for p in [snapshot, output/WORKER_NAME, release/'component.wasm',release/'rust.crate',release/'node.tgz',native/'Cargo.lock',guest/'Cargo.lock',node/'package-lock.json',output/'composed.wasm']}
    write(report,json.dumps({'schema':1,'status':'passed','native':native_result,'node':node_result,'composed':composed_result,
         'registryStoppedBeforeConsumers':True,'guestHasHostRuntime':False,'reproducibleComposition':True,
         'limitations':['JCO 1.33.0 future values expose no cancellation or dispose operation','nested async handles and async resource methods are rejected','async browser execution has not been verified'],
         'artifacts':artifacts,'commands':commands},indent=2)+'\n')
    print(report)


if __name__ == '__main__':
    main()
