#!/usr/bin/env python3
"""Accept C# through fresh compiler installs and generated archive consumers."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tomllib

ROOT = Path(__file__).resolve().parents[2]


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def logged_command(argv, *, cwd, environment, stdout, stderr, timeout=1800):
    # Compiler diagnostics belong in evidence, not in the consumer's result.
    with stdout.open('w') as out_stream, stderr.open('w') as err_stream:
        result = subprocess.run(argv, cwd=cwd, env=environment, stdout=out_stream,
                                stderr=err_stream, timeout=timeout)
    return result.returncode, stdout.read_text().strip()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cli', type=Path, required=True)
    parser.add_argument('--bundles', type=Path, required=True)
    parser.add_argument('--out', type=Path, required=True)
    parser.add_argument('--verify-component', type=Path, required=True)
    parser.add_argument('--bindings', type=Path, required=True)
    args = parser.parse_args()
    output = args.out.resolve()
    output.mkdir(parents=True, exist_ok=False)
    logs = output / 'logs'; logs.mkdir()
    cli = args.cli.resolve(strict=True)
    environment = dict(os.environ, PYTHONDONTWRITEBYTECODE='1', CARGO_NET_OFFLINE='true',
                       CARGO_BUILD_JOBS='1', npm_config_offline='true')
    for name in ('WISP_CSHARP_TOOLCHAIN', 'WISP_WASM_TOOLS', 'WISP_WASM_TOOLS_TOOLCHAIN', 'WISP_PREVIEW1_ADAPTER'):
        environment.pop(name, None)
    environment['WISP_TOOLCHAIN_HOME'] = str(output / 'toolchains')
    host_cache = output / 'host-cache'; host_cache.mkdir()
    environment['XDG_CACHE_HOME'] = str(host_cache)
    report = {'complete': False, 'passed': False, 'commands': [], 'checks': [], 'cliSha256': digest(cli)}

    def save():
        (output / 'result.json').write_text(json.dumps(report, indent=2) + '\n')

    def run(argv, *, cwd=ROOT, success=True, env=None):
        argv = list(map(str, argv)); index = len(report['commands'])
        stdout, stderr = logs / f'{index:03}.log', logs / f'{index:03}.stderr.log'
        record = {'argv': argv, 'cwd': str(cwd), 'status': 'running',
                  'stdout': str(stdout), 'stderr': str(stderr)}
        report['commands'].append(record); save()
        status, result = logged_command(argv, cwd=cwd, environment=env or environment,
                                        stdout=stdout, stderr=stderr)
        record['status'] = status; save()
        if (status == 0) != success:
            raise RuntimeError(f'command returned {status}: {argv}; see {stdout} and {stderr}')
        return result

    save()
    try:
        for tool in ('wasm-tools', 'csharp'):
            bundle = args.bundles.resolve() / (tool + '.wisp.zip')
            run([cli, 'toolchain', 'install', bundle, '--prefix', output / 'toolchains', '--sha256', digest(bundle)])
        selection = output / 'toolchains/selection.json'
        selected_hash = digest(selection)
        report['compilerSelection'] = json.loads(selection.read_text())
        installed_files = {str(p.relative_to(output / 'toolchains')): digest(p)
                           for p in (output / 'toolchains/packages').rglob('*') if p.is_file()}
        # C# application stages cannot reach native .NET compilers. Package
        # generators and the Rust consumer use their normal host tools later.
        blocked = output / 'blocked'; blocked.mkdir()
        attempts = output / 'native-compiler-attempts.log'; attempts.write_text('')
        deny = blocked / 'deny'
        deny.write_text('#!' + sys.executable + '\nimport os,sys\n'
                        'with open(os.environ["WISP_NATIVE_TOOL_ATTEMPTS"], "a") as log: log.write(sys.argv[0]+"\\n")\n'
                        'raise SystemExit("native C# application compiler blocked")\n')
        deny.chmod(0o755)
        for name in ('dotnet', 'csc', 'mcs', 'mono', 'ilc', 'msbuild'):
            (blocked / name).symlink_to(deny.name)
        environment['PATH'] = str(blocked) + os.pathsep + environment['PATH']
        environment['WISP_NATIVE_TOOL_ATTEMPTS'] = str(attempts)
        source = output / 'source'
        shutil.copytree(ROOT / 'examples/build-csharp', source)
        cache = output / 'build-cache'

        def build(name, *, component_only=False, success=True):
            destination = output / name
            argv = [cli, 'build', source, '--out', destination, '--cache', cache]
            if component_only: argv.append('--component-only')
            run(argv, success=success)
            if not success:
                if destination.exists(): raise RuntimeError('failed source build published output')
                return None
            receipt = json.loads((destination / 'build/build.json').read_text())
            if receipt['componentSha256'] != digest(destination / 'component.wasm'):
                raise RuntimeError('Component differs from source build receipt')
            if receipt['contractMode'] != ('wit' if name == 'authored' else 'inferred'):
                raise RuntimeError('unexpected contract mode')
            commands = receipt['units']['application']['commands']
            adapters = [c['adapterReceipt'] for c in commands if 'adapterReceipt' in c]
            if len(adapters) != 1 or adapters[0]['sourceCompilationWasmHosted'] is not True:
                raise RuntimeError('missing executed Wasm compiler receipt')
            report['checks'].append({'name': name, 'contractMode': receipt['contractMode'],
                                     'componentSha256': receipt['componentSha256']})
            save()
            return destination

        # Default example is inferred; retain an authored contract regression.
        config = (source / 'wisp.toml').read_text()
        (source / 'wisp.toml').write_text(config + '\nwit="library/library.wit"\nworld="library"\n')
        authored = build('authored', component_only=True)
        run([sys.executable, args.verify_component, authored / 'component.wasm', '--bindings', args.bindings])
        (source / 'wisp.toml').write_text(config)
        release = build('inferred-release')
        run([sys.executable, args.verify_component, release / 'component.wasm', '--bindings', args.bindings,
             '--interface', 'example:csharp-library/api@1.0.0', '--inferred'])
        run([cli, 'library', 'validate', release])

        # Install the emitted npm tarball, then invoke its actual generated API.
        node = output / 'node-consumer'; node.mkdir()
        node_name = json.loads((release / 'node/package.json').read_text())['name']
        (node / 'package.json').write_text('{"name":"csharp-consumer","private":true,"type":"module"}')
        run(['npm', 'install', '--offline', '--ignore-scripts', '--no-audit', '--no-fund', release / 'node.tgz'], cwd=node)
        (node / 'consumer.mjs').write_text(
            "import assert from 'node:assert/strict';\nimport { api } from " + json.dumps(node_name) + ";\n"
            "assert.equal(api.add(9007199254740993n, 1n), 9007199254740994n);\n"
            "assert.equal(api.greet('世界'), 'Hello, 世界!');\n"
            "assert.deepEqual(api.advance({name:'Zoë', age:41}), {name:'Zoë', age:42});\n"
            "assert.throws(() => api.advance({name:'Zoë',age:0}), e => e.payload === 'age must be positive');\n"
            "for (let i=0;i<100;i++) { assert.equal(api.greet('雪'.repeat(1024)), 'Hello, '+ '雪'.repeat(1024)+'!'); assert.equal(api.add(BigInt(i),1n),BigInt(i+1)); }\n"
            "console.log('installed-node-csharp-ok');\n")
        if run([environment['WISP_NODE'], 'consumer.mjs'], cwd=node) != 'installed-node-csharp-ok':
            raise RuntimeError('installed npm consumer failed')
        report['checks'].append({'name': 'installed npm archive consumer', 'calls': 204})

        # Use only the shipped .crate bytes, never the generated source tree.
        installed = output / 'rust-installed'; installed.mkdir()
        with tarfile.open(release / 'rust.crate') as archive:
            archive.extractall(installed, filter='data')
        package = next(installed.iterdir())
        package_name = tomllib.loads((package / 'Cargo.toml').read_text())['package']['name']
        rust = output / 'rust-consumer'; (rust / 'src').mkdir(parents=True)
        (rust / 'Cargo.toml').write_text('[package]\nname="csharp-consumer"\nversion="0.0.0"\nedition="2024"\n[workspace]\n'
            '[dependencies]\nsubject={package=' + json.dumps(package_name) + ',path=' + json.dumps(str(package)) + '}\n')
        (rust / 'src/main.rs').write_text('''use subject::{api::{Client, Person}, CallError};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut api = Client::new()?;
    assert_eq!(api.add(9007199254740993, 1)?, 9007199254740994);
    assert_eq!(api.greet("世界".into())?, "Hello, 世界!");
    assert_eq!(api.advance(Person { name: "Zoë".into(), age: 41 })?.age, 42);
    match api.advance(Person { name: "Zoë".into(), age: 0 }) {
        Err(CallError::Guest(message)) => assert_eq!(message, "age must be positive"),
        other => panic!("unexpected result: {other:?}"),
    }
    for i in 0..100 { assert_eq!(api.add(i, 1)?, i + 1); }
    println!("installed-rust-csharp-ok");
    Ok(())
}
''')
        if run(['cargo', 'run', '--offline', '--quiet'], cwd=rust) != 'installed-rust-csharp-ok':
            raise RuntimeError('installed Cargo archive consumer failed')
        report['checks'].append({'name': 'installed Cargo archive consumer', 'calls': 104})

        helper = source / 'library/Shared/Models.cs'
        helper.write_text(helper.read_text().replace('=> left + right;', '=> left + right + 1;'))
        changed = build('changed-project', component_only=True)
        if run([environment['WISP_WASMTIME'], 'run', '--invoke', 'add(20,22)', changed / 'component.wasm']) != '43':
            raise RuntimeError('referenced source mutation did not invalidate the build')
        if digest(changed / 'component.wasm') == digest(release / 'component.wasm'):
            raise RuntimeError('changed source reused stale output')
        helper.write_text('deliberate invalid C#')
        build('invalid-source', component_only=True, success=False)
        if attempts.read_text(): raise RuntimeError('native C# compiler was invoked')
        if digest(selection) != selected_hash or installed_files != {
            str(p.relative_to(output / 'toolchains')): digest(p)
            for p in (output / 'toolchains/packages').rglob('*') if p.is_file()
        }: raise RuntimeError('source compilation mutated installed compiler packages')
        report.update(complete=True, passed=True, cleanInstallVerified=True, nativeCompilerAttempts=[])
    finally:
        save()


if __name__ == '__main__':
    main()
