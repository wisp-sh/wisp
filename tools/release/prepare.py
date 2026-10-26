#!/usr/bin/env python3
"""Prepare the Linux CLI release kit, including hosts and native package tools.

Run in a disposable Ubuntu 24.04 builder after building wisp and wisp-worker.
Application compiler bundles are added by wisp-buildtools, not by this script.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tarfile
import tempfile
import urllib.request

ROOT = Path(__file__).resolve().parents[2]
RUST = "1.98.1"
MAX_ASSET = 2 * 1024**3


def digest(path):
    with Path(path).open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def run(argv, env=None):
    print('+ ' + ' '.join(map(str, argv)), flush=True)
    return subprocess.run(list(map(str, argv)), check=True, env=env, text=True, stdout=None)


def download(spec, path):
    with urllib.request.urlopen(spec['url'], timeout=120) as response, path.open('xb') as output:
        if not response.url.startswith('https://'):
            raise ValueError('release download redirected away from HTTPS')
        total = 0
        while chunk := response.read(1024 * 1024):
            total += len(chunk)
            if total > MAX_ASSET:
                raise ValueError('native tool download exceeds limit')
            output.write(chunk)
    if digest(path) != spec['sha256']:
        raise ValueError('native tool checksum mismatch')


def prepare(out, scratch, binary_dir):
    if (platform.system(), platform.machine()) != ('Linux', 'x86_64'):
        raise ValueError('release kits require Linux x86_64')
    out.mkdir(parents=True, exist_ok=False)
    scratch.mkdir(parents=True, exist_ok=False)
    (out / 'bin').mkdir()
    for name in ('wisp', 'wisp-worker'):
        shutil.copy2(binary_dir / name, out / 'bin' / name)
    shutil.copytree(ROOT / 'examples/build-c', out / 'share/wisp/smoke-c')
    shutil.copyfile(ROOT / 'LICENSE', out / 'share/wisp/LICENSE')
    cli = out / 'bin/wisp'
    run([cli, 'toolchain', 'setup-hosts', '--prefix', out / 'toolchains',
         '--cache', scratch / 'hosts', '--host', 'all', '--jobs', '1'])
    lock = json.loads((ROOT / 'crates/wisp/assets/tools.lock.json').read_text())
    for name in ('node', 'go'):
        archive = scratch / f'{name}.tar'
        download(lock[name], archive)
        unpack = scratch / f'{name}-unpack'
        unpack.mkdir()
        with tarfile.open(archive) as source:
            source.extractall(unpack, filter='data')
        roots = list(unpack.iterdir())
        if len(roots) != 1 or not (roots[0] / 'bin' / name).is_file():
            raise ValueError(f'unexpected {name} archive layout')
        shutil.move(roots[0], out / name)
        archive.unlink()
    # Copy the real toolchain, rather than rustup shims with machine-specific paths.
    rustc = subprocess.check_output(['rustup', 'which', '--toolchain', RUST, 'rustc'], text=True).strip()
    shutil.copytree(Path(rustc).parent.parent, out / 'rust', symlinks=False)
    env = dict(os.environ, PATH=os.pathsep.join([str(out / 'rust/bin'), str(out / 'node/bin'), str(out / 'go/bin'), os.environ['PATH']]),
               CARGO_HOME=str(out / 'cargo-home'), CARGO_BUILD_JOBS='1', CARGO_INCREMENTAL='0')
    run([out / 'rust/bin/rustc', '--version'], env)
    run([out / 'node/bin/node', '--version'], env)
    run([out / 'go/bin/go', 'version'], env)
    js = out / 'js'
    js.mkdir()
    for name in ('package.json', 'package-lock.json'):
        shutil.copyfile(ROOT / 'crates/wisp/assets/setup-tooling' / name, js / name)
    run([out / 'node/bin/npm', 'ci', '--prefix', js, '--ignore-scripts', '--no-audit', '--no-fund'], env)
    run(['python3', ROOT / 'tools/upstream-bindings/setup.py', '--out', out / 'generators'], env)
    # The seed covers dependencies cargo package resolves for generated libraries.
    seed = scratch / 'seed'
    (seed / 'src').mkdir(parents=True)
    for source, target in [('Cargo.toml.txt', 'Cargo.toml'), ('Cargo.lock.txt', 'Cargo.lock'), ('lib.rs.txt', 'src/lib.rs')]:
        shutil.copyfile(ROOT / 'crates/wisp/assets/setup-cargo-seed' / source, seed / target)
    run(['cargo', 'fetch', '--locked', '--manifest-path', seed / 'Cargo.toml'], env)
    # Go SDK packaging builds its own exact Wasmtime dependency graph offline.
    go_seed = scratch / 'go-seed'
    (go_seed / 'src').mkdir(parents=True)
    (go_seed / 'src/lib.rs').write_text('')
    shutil.copyfile(ROOT / 'crates/wisp/assets/library-go-Cargo.lock', go_seed / 'Cargo.lock')
    (go_seed / 'Cargo.toml').write_text('''[package]
name="wisp-go-shim-seed"
version="0.1.0"
edition="2024"
[workspace]
[lib]
crate-type=["staticlib"]
[dependencies]
wasmtime={version="=50.0.0",default-features=false,features=["component-model","component-model-async","cranelift","runtime","std"]}
serde="=1.0.229"
serde_json="=1.0.151"
wasmtime-wasi={version="=50.0.0",default-features=false,features=["p2"]}
wasmtime-wasi-http={version="=50.0.0",default-features=false,features=["p2","default-send-request"]}
''')
    run(['cargo', 'fetch', '--locked', '--manifest-path', go_seed / 'Cargo.toml'], env)
    exports = {'WISP_TOOLCHAIN_HOME': 'toolchains', 'WISP_WORKER': 'bin/wisp-worker',
               'WISP_NODE': 'node/bin/node', 'WISP_GO': 'go/bin/go',
               'WISP_JCO': 'js/node_modules/.bin/jco', 'WISP_WIT_BINDGEN': 'generators/wit-bindgen',
               'WISP_WIT_BINDGEN_WORKDIR': 'binding-work', 'CARGO_HOME': 'cargo-home',
               'CARGO_TARGET_DIR': 'cargo-target', 'GOCACHE': 'go-cache', 'GOMODCACHE': 'go-mod-cache',
               'WISP_GO_CARGO_TARGET_DIR': 'go-cargo-target'}
    for path in ('binding-work', 'cargo-target', 'go-cache', 'go-mod-cache', 'go-cargo-target'):
        (out / path).mkdir(parents=True)
    for path in exports.values():
        if not (out / path).exists():
            raise ValueError(f'missing release tool: {path}')
    (out / 'native-env.json').write_text(json.dumps(exports, indent=2) + '\n')
    return exports


def package(prepared, archive, url):
    # npm launchers must retain their canonical package paths. Flattening .bin
    # links changes both relative imports and the generator's entry-point proof.
    for path in prepared.rglob('*'):
        if path.is_symlink():
            if not path.resolve().is_relative_to(prepared.resolve()):
                raise ValueError('release link escapes prepared tree')
            if not path.is_file():
                raise ValueError('release links must resolve to regular files')
        elif not (path.is_dir() or path.is_file()):
            raise ValueError('unsupported release file type')
    with tarfile.open(archive, 'w:gz', dereference=False) as output:
        for path in sorted(prepared.rglob('*')):
            entry = output.gettarinfo(path, arcname=path.relative_to(prepared).as_posix())
            if path.is_symlink():
                entry.linkname = os.path.relpath(path.resolve(), path.parent.resolve())
                output.addfile(entry)
            elif path.is_dir():
                output.addfile(entry)
            else:
                # Write every regular file as data, including hard-linked cache
                # inputs. The runtime extractor deliberately rejects hard links.
                entry.type, entry.linkname, entry.size = tarfile.REGTYPE, '', path.stat().st_size
                with path.open('rb') as source:
                    output.addfile(entry, source)
    if archive.stat().st_size > MAX_ASSET:
        raise ValueError('native kit exceeds the 2 GiB release asset limit')
    return {'schema': 1, 'platform': 'linux-x86_64', 'cli': {'path': 'bin/wisp'},
            'env': json.loads((prepared / 'native-env.json').read_text()),
            'runtime': {'url': url, 'sha256': digest(archive), 'size': archive.stat().st_size}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--scratch', type=Path, required=True)
    parser.add_argument('--binary-dir', type=Path, required=True)
    parser.add_argument('--archive', type=Path, required=True)
    parser.add_argument('--manifest', type=Path, required=True)
    parser.add_argument('--url', required=True)
    args = parser.parse_args()
    prepare(args.out.resolve(), args.scratch.resolve(), args.binary_dir.resolve())
    args.manifest.write_text(json.dumps(package(args.out, args.archive, args.url), indent=2) + '\n')


if __name__ == '__main__':
    main()
