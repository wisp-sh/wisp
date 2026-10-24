#!/usr/bin/env python3
"""Verify a source-built Rust macro candidate through Wisp admission and execution.

This is separate from the Rust toolchain's six standalone compiler cases. No candidate is
selected implicitly. --preflight reads inputs only and never claims compilation.
"""
from __future__ import annotations
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import signal
import shutil
import subprocess
import sys
import tomllib

ROOT = Path(__file__).resolve().parents[2]
snapshot_spec = importlib.util.spec_from_file_location('source_build_snapshot', ROOT / 'tests/source-build/snapshot.py')
snapshot_module = importlib.util.module_from_spec(snapshot_spec)
snapshot_spec.loader.exec_module(snapshot_module)
MACRO_TARGET = 'wasm32-unknown-unknown'
DIAGNOSTIC = 'Serde does not support derive for unions'


class VerificationError(RuntimeError):
    pass


def require(condition, message):
    if not condition:
        raise VerificationError(message)


def digest(path):
    with Path(path).open('rb') as file:
        return hashlib.file_digest(file, 'sha256').hexdigest()


def json_read(path):
    value = json.loads(Path(path).read_text())
    require(isinstance(value, dict), f'expected JSON object: {path}')
    return value


def json_write(path, value):
    Path(path).write_text(json.dumps(value, indent=2) + '\n')


def executable(path):
    # Preserve multicall invocation names such as the cargo/rustc rustup links.
    path = Path(path).absolute()
    require(path.is_file() and os.access(path, os.X_OK), f'executable missing: {path}')
    return path


def wasm(path, expected, *, component=False):
    with Path(path).open('rb') as file:
        require(file.read(8) == (b'\0asm\x0d\0\x01\0' if component else b'\0asm\x01\0\0\0'), f'unexpected Wasm artifact format: {path}')
    require(digest(path) == expected, f'Wasm checksum mismatch: {path}')


def inventory(directory, checksums, *, excluded=()):
    actual = set()
    for path in directory.rglob('*'):
        require(not path.is_symlink(), f'symlink in verified inputs: {path}')
        if path.is_file() and str(path.relative_to(directory)) not in excluded:
            actual.add(str(path.relative_to(directory)))
    require(actual == set(checksums), f'file inventory mismatch: {directory}')
    for relative, expected in checksums.items():
        path = (directory / relative).resolve()
        require(path.is_relative_to(directory.resolve()) and path.is_file(), f'unsafe input: {relative}')
        require(digest(path) == expected, f'input checksum mismatch: {path}')


def preflight(args):
    candidate = args.candidate.resolve()
    require(candidate.is_dir(), f'explicit macro candidate is missing: {candidate}; finish its source build first')
    manifest = json_read(candidate / 'toolchain.json')
    require(manifest.get('capabilities', {}).get('proceduralMacros') is True,
            'candidate does not enable procedural macros')
    require(manifest.get('schemaVersion') == 1 and manifest.get('compilerHost') == 'wasm32-wasip3'
            and manifest.get('applicationTarget') == 'wasm32-wasip1'
            and manifest.get('proceduralMacroTarget') == MACRO_TARGET, 'unexpected candidate target profile')
    require(re.fullmatch('[0-9a-f]{64}', args.compiler_sha256) is not None, 'expected compiler SHA-256 is invalid')
    prefix = candidate / 'build/bootstrap'
    compiler = (prefix / manifest['compiler']['module']).resolve()
    require(compiler.is_relative_to(prefix), 'compiler escapes candidate prefix')
    require(manifest['compiler']['moduleSha256'] == args.compiler_sha256, 'candidate selects a different compiler SHA-256')
    wasm(compiler, args.compiler_sha256, component=True)
    inventory(prefix, json_read(prefix / 'SHA256SUMS.json'), excluded=('SHA256SUMS.json',))
    runner = candidate / 'scripts/runner.py'
    require('llvmProvider' not in manifest, 'candidate must contain its LLVM backend')
    source_build = json_read(candidate / 'source-build.json')
    require(source_build['buildReport'].get('compilerBuildCompleted') is True
            and source_build['buildReport'].get('status') == 'build-passed'
            and source_build['buildReport']['compiler']['sha256'] == args.compiler_sha256,
            'candidate has no matching completed source-build receipt')
    require(digest(runner) == source_build['runnerSha256'], 'candidate runner changed after packaging')
    for target in ('wasm32-wasip3', 'wasm32-wasip1', MACRO_TARGET):
        libraries = prefix / 'sysroot/lib/rustlib' / target / 'lib'
        require(any(libraries.glob('libstd-*.rlib')) and any(libraries.glob('libproc_macro-*.rlib')),
                f'candidate sysroot is incomplete for {target}')
    for name, expected in manifest['licenses'].items():
        path = (candidate / 'licenses' / name).resolve()
        require(path.is_relative_to(candidate / 'licenses') and digest(path) == expected,
                f'candidate license mismatch: {name}')
    require(args.compiler_wasmtime is not None, '--compiler-wasmtime must select the shared compiler execution host')
    compiler_runtime = executable(args.compiler_wasmtime)
    tools = {key: executable(getattr(args, key)) for key in ('cli', 'wasmtime', 'node', 'jco', 'cargo', 'native_rustc')}
    adapter = args.preview1_adapter.resolve(strict=True)
    fixture = args.fixture.resolve() if args.fixture else args.rust_toolchain.resolve() / 'examples/serde'
    require((fixture / 'src/lib.rs').is_file() and (fixture / 'Cargo.lock').is_file(), 'Serde fixture is missing')
    lock = tomllib.loads((fixture / 'Cargo.lock').read_text())
    vendor = args.vendor_dir.resolve(strict=True)
    packages, notices = {}, []
    for package in lock['package']:
        if not package.get('source'):
            continue
        directory = vendor / package['name']
        checksum = json_read(directory / '.cargo-checksum.json')
        require(checksum['package'] == package['checksum'], f'crate archive differs from lock: {package["name"]}')
        inventory(directory, checksum['files'], excluded=('.cargo-checksum.json',))
        cargo = tomllib.loads((directory / 'Cargo.toml').read_text())['package']
        require(cargo['name'] == package['name'] and cargo['version'] == package['version'], 'vendored crate identity mismatch')
        packages[package['name']] = {'version': package['version'], 'crateSha256': package['checksum']}
        licenses = [path for path in sorted(directory.iterdir()) if path.is_file()
                    and path.name.lower().startswith(('license', 'copying', 'notice', 'unlicense'))]
        require(licenses, f'vendored crate has no packaged notices: {package["name"]}')
        for path in licenses:
            notices.append(f'===== {package["name"]} {package["version"]}: {path.name} =====\n\n' + path.read_text())
    require(packages.get('serde', {}).get('version') == '1.0.228'
            and packages.get('serde_derive', {}).get('version') == '1.0.228'
            and packages.get('serde_json', {}).get('version') == '1.0.150', 'fixture Serde pins changed')
    notice = '\n\n'.join(notices) + '\n'
    require(len(notice.encode()) <= 256 * 1024, 'dependency notices exceed one Wisp notice file')
    return candidate, manifest, compiler, compiler_runtime, runner, tools, adapter, fixture, vendor, packages, notice


class RelocatedBuild:
    """Resolve original staging paths after Wisp's atomic release-directory rename."""
    def __init__(self, release, report):
        self.release = release.resolve()
        manifest = Path(report['manifestPath'])
        suffix = Path('build/units/application/source/Cargo.toml')
        require(manifest.is_absolute() and tuple(manifest.parts[-len(suffix.parts):]) == suffix.parts,
                'unexpected staged Rust manifest path')
        self.original = manifest.parents[4]

    def path(self, value):
        require(isinstance(value, str) and Path(value).is_absolute(), 'execution paths must be absolute')
        path = Path(value)
        if path.is_relative_to(self.original):
            path = self.release / path.relative_to(self.original)
        return path.resolve()


def argument_pair(argv, name, value):
    return any(argv[index:index + 2] == [name, value] for index in range(len(argv)))


def verify_macros(report, relocated, compiler, compiler_runtime, expected_sha):
    require(report.get('schemaVersion') == 1 and report.get('target') == 'wasm32-wasip1', 'unexpected Rust execution report')
    executions = report['executions']
    require(executions, 'Rust receipt records no Wasm compiler execution')
    require(report['compiler']['path'] == str(compiler) and report['compiler']['sha256'] == expected_sha
            and report['compiler']['host'] == 'wasm', 'Rust receipt names a different compiler')
    for execution in executions:
        require(type(execution['exitCode']) is int and execution['exitCode'] == 0, 'Rust receipt contains a failed execution')
        argv = execution['argv']
        require(Path(argv[0]).resolve() == compiler_runtime.resolve() and argv[1] == 'run'
                and execution['program'] in argv, 'execution does not use the candidate Wasmtime')
        program = relocated.path(execution['program'])
        wasm(program, execution['sha256'], component=execution['kind'] == 'compiler')
        require(relocated.path(execution['cwd']).is_dir(), 'execution working directory is missing after relocation')
        if execution['kind'] == 'compiler':
            require(program == compiler and execution['sha256'] == expected_sha, 'native or foreign source compiler recorded')
        else:
            require(execution['kind'] == 'build-script' and program.is_relative_to(relocated.release), 'unexpected source execution kind')
    completed, traces = set(), {}
    records = report['proceduralMacros']
    for record in records:
        require(record.get('loader') == 'compiler-wasm-runtime' and record.get('runtime') == 'wasmi', 'macro did not use the compiler interpreter')
        module = relocated.path(record['program'])
        require(module.is_relative_to(relocated.release), 'macro artifact escapes release build evidence')
        wasm(module, record['sha256'])
        index, compilation = record['execution'], record['compilation']
        require(type(index) is int and type(compilation) is int and 0 <= compilation < index < len(executions), 'macro has no earlier compilation')
        require(executions[index]['kind'] == executions[compilation]['kind'] == 'compiler', 'macro is not linked to compiler executions')
        argv = executions[compilation]['argv']
        require(argument_pair(argv, '--target', MACRO_TARGET) and argument_pair(argv, '--crate-type', 'proc-macro')
                and argument_pair(argv, '-o', record['program']) and '-Zwasm-proc-macros' in argv,
                'macro compilation target/output differs from its artifact')
        require('WISP_WASM_PROC_MACRO_LOG=' + record['trace'] in executions[index]['argv'], 'trace is not attached to consuming compiler')
        trace = relocated.path(record['trace'])
        require(trace.is_relative_to(relocated.release) and digest(trace) == record['traceSha256'], 'macro trace checksum mismatch')
        if trace not in traces:
            traces[trace] = [json.loads(line) for line in trace.read_text().splitlines()]
        identity = record['macroIndex'], record['macroName'], record['macroKind']
        require(type(identity[0]) is int and 0 <= identity[0] < 2**32
                and isinstance(identity[1], str) and identity[2] in ('function', 'attribute', 'derive'), 'invalid macro identity')
        loaded, started, success = False, False, False
        for event in traces[trace]:
            if event['module'] != record['program'] or (event['index'], event['name'], event['kind']) != identity:
                continue
            require(event['schemaVersion'] == 1 and event['runtime'] == 'wasmi' and event['macroTarget'] == MACRO_TARGET, 'invalid macro trace runtime')
            if event['event'] == 'load' and event['status'] == 'succeeded':
                loaded = True
            elif event['event'] == 'invoke' and event['status'] == 'started' and loaded and not started:
                started = True
            elif event['event'] == 'invoke' and event['status'] == 'succeeded' and started:
                started, success = False, True
            else:
                raise VerificationError('macro trace records an unsuccessful invocation')
        require(success and not started, 'macro artifact exists without a completed interpreter invocation')
        completed.add((record['package'], record['macroName'], record['macroKind']))
    for name in ('Serialize', 'Deserialize'):
        require(('serde_derive', name, 'derive') in completed, f'no actual Serde {name} invocation')
    scripts = {item['package'] for item in report['buildScripts']}
    require({'serde', 'serde_core', 'serde_json'} <= scripts, 'Serde build scripts did not run')
    for record in report['buildScripts']:
        index = record['execution']
        require(type(index) is int and 0 <= index < len(executions)
                and executions[index]['kind'] == 'build-script' and executions[index]['program'] == record['program'],
                'build script has no Wasm execution')
        require(any(execution['kind'] == 'compiler'
                    and argument_pair(execution['argv'], '-o', record['program'])
                    and argument_pair(execution['argv'], '--crate-type', 'bin')
                    and argument_pair(execution['argv'], '--target', 'wasm32-wasip1')
                    for execution in executions[:index]), 'build script was not compiled by the Wasm compiler')
        wasm(relocated.path(record['program']), record['sha256'])
    artifact = relocated.path(report['artifact']['path'])
    require(artifact.is_relative_to(relocated.release), 'application core artifact escapes release')
    wasm(artifact, report['artifact']['sha256'])
    require(any(execution['kind'] == 'compiler'
                and argument_pair(execution['argv'], '-o', report['artifact']['path'])
                and argument_pair(execution['argv'], '--crate-type', 'cdylib')
                and argument_pair(execution['argv'], '--target', 'wasm32-wasip1')
                for execution in executions), 'application artifact has no Wasm source compilation')
    return {'completedMacros': len(records), 'macroTraceFiles': len(traces),
            'buildScripts': sorted(scripts), 'originalStagingRoot': str(relocated.original),
            'publishedReleaseRoot': str(relocated.release), 'rawCoreSha256': digest(artifact)}


def native_guard(path):
    path.write_text('#!' + str(Path(sys.executable).resolve()) + ' -I\n' +
        'import json,os,sys\nargs=sys.argv[1:]\n' +
        'with open(os.environ["WISP_SERDE_NATIVE_QUERY_LOG"],"a") as f:f.write(json.dumps({"phase":os.environ["WISP_SERDE_PHASE"],"argv":args})+"\\n")\n' +
        'version=args in (["-V"],["-vV"],["--version"],["--version","--verbose"],["--verbose","--version"])\n' +
        'requested=[]\n' +
        'for i,x in enumerate(args):\n' +
        ' if x.startswith("--print="):requested.extend(x.partition("=")[2].split(","))\n' +
        ' elif x=="--print" and i+1<len(args):requested.extend(args[i+1].split(","))\n' +
        'query=version or (bool(requested) and set(requested)<=set(["cfg","sysroot","file-names","split-debuginfo","crate-name","target-libdir","target-list"]))\n' +
        'if not query or any(x.endswith(".rs") or x in ("--emit","-o","--out-dir","--test") or x.startswith(("--emit=","--out-dir=","-o")) for x in args):\n' +
        ' sys.exit("Native Rust source compilation forbidden in Wisp Serde acceptance")\n' +
        'os.execv(os.environ["WISP_SERDE_NATIVE_RUSTC"],[os.environ["WISP_SERDE_NATIVE_RUSTC"]]+args)\n')
    path.chmod(0o755)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('candidate', 'cli', 'wasmtime', 'vendor-dir', 'preview1-adapter', 'node', 'jco', 'out'):
        parser.add_argument('--' + name, type=Path, required=True)
    fixture_source = parser.add_mutually_exclusive_group(required=True)
    fixture_source.add_argument('--rust-toolchain', dest='rust_toolchain', type=Path)
    fixture_source.add_argument('--fixture', type=Path, help='ordinary Serde Cargo project, independent of compiler source checkout')
    parser.add_argument('--compiler-wasmtime', type=Path, help='Wisp shared Rust compiler execution host')
    parser.add_argument('--compiler-sha256', required=True, help='SHA from the successful source-build receipt')
    parser.add_argument('--cargo', type=Path, default=os.environ.get('WISP_CARGO') or shutil.which('cargo'))
    parser.add_argument('--native-rustc', type=Path, default=shutil.which('rustc'), help='host packaging queries only; source compilation is forbidden')
    parser.add_argument('--timeout', type=int, default=3600)
    parser.add_argument('--preflight', action='store_true', help='verify inputs only; do not compile or write result.json')
    args = parser.parse_args()
    require(args.timeout > 0 and args.cargo and args.native_rustc, 'positive timeout and host Cargo/rustc query tools required')
    candidate, manifest, compiler, compiler_runtime, runner, tools, adapter, fixture, vendor, packages, notice = preflight(args)
    if args.preflight:
        print('Inputs verified; no application compilation or macro execution was attempted')
        return 0
    output = args.out.resolve()
    output.mkdir(parents=True, exist_ok=False)
    logs = output / 'logs'
    logs.mkdir()
    commands = []
    result = {'status': 'running', 'compilerSha256': args.compiler_sha256, 'standaloneSixCases': 'separate acceptance required'}
    cli = output / 'wisp'
    snapshot_module.freeze_cli(tools['cli'], cli)
    query_log = output / 'native-query-log.jsonl'
    query_log.write_text('')
    guard = output / 'native-query-only'
    native_guard(guard)
    environment = dict(os.environ, WISP_RUST=str(candidate), WISP_RUST_WASMTIME=str(compiler_runtime),
                       WISP_WASMTIME=str(tools['wasmtime']), WISP_PREVIEW1_ADAPTER=str(adapter),
                       WISP_NODE=str(tools['node']), WISP_JCO=str(tools['jco']),
                       WISP_GO_TOOLCHAIN='/go-must-not-be-used', WISP_CLANG='/clang-must-not-be-used',
                       RUSTC=str(guard), RUSTC_WRAPPER='', RUSTC_WORKSPACE_WRAPPER='',
                       CARGO_BUILD_RUSTC=str(guard), CARGO_BUILD_RUSTC_WRAPPER='', CARGO_BUILD_RUSTC_WORKSPACE_WRAPPER='',
                       WISP_SERDE_NATIVE_RUSTC=str(tools['native_rustc']), WISP_SERDE_NATIVE_QUERY_LOG=str(query_log))
    environment['PATH'] = os.pathsep.join([str(tools['cargo'].parent), str(tools['node'].parent), environment.get('PATH', '')])

    def run(argv, phase, success=True):
        argv = list(map(str, argv))
        print('+ ' + ' '.join(argv), flush=True)
        index = len(commands)
        stdout, stderr = logs / f'{index:02}-{phase}.stdout', logs / f'{index:02}-{phase}.stderr'
        record = {'phase': phase, 'argv': argv, 'cwd': str(ROOT), 'status': 'running',
                  'stdout': str(stdout), 'stderr': str(stderr)}
        commands.append(record)
        json_write(output / 'commands.json', commands)
        with stdout.open('w') as out_log, stderr.open('w') as error_log:
            process = subprocess.Popen(argv, cwd=ROOT, env=dict(environment, WISP_SERDE_PHASE=phase),
                                       stdin=subprocess.DEVNULL, stdout=out_log, stderr=error_log, start_new_session=True)
            try:
                status = process.wait(timeout=args.timeout)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait()
                record.update(status='timeout')
                json_write(output / 'commands.json', commands)
                raise
        record.update(status='completed', exitCode=status)
        json_write(output / 'commands.json', commands)
        completed = subprocess.CompletedProcess(argv, status, stdout=stdout.read_text(), stderr=stderr.read_text())
        require((status == 0) == success, f'{phase} returned {status}:\n{completed.stdout}{completed.stderr}')
        return completed

    try:
        version = run([tools['wasmtime'], '--version'], 'application-runtime-preflight').stdout.split()
        require(len(version) > 1 and version[1].startswith('50.'), 'application runtime must be Wasmtime 50')
        compiler_version = run([compiler_runtime, '--version'], 'compiler-runtime-preflight').stdout.split()
        require(len(compiler_version) > 1 and compiler_version[1] == manifest['runtime']['version'], 'candidate compiler runtime version differs from its manifest')
        inputs = {'candidate': str(candidate), 'compilerSha256': args.compiler_sha256,
                  'runnerSha256': digest(runner), 'manifestSha256': digest(candidate / 'toolchain.json'),
                  'inventorySha256': digest(candidate / 'build/bootstrap/SHA256SUMS.json'),
                  'compilerRuntimeSha256': digest(compiler_runtime), 'applicationRuntimeSha256': digest(tools['wasmtime']),
                  'cliSha256': digest(cli), 'adapterSha256': digest(adapter), 'hostPackagingTools': {
                      key: {'path': str(tools[key]), 'sha256': digest(tools[key])} for key in ('cargo', 'native_rustc', 'node', 'jco')},
                  'registryCrates': packages, 'fixture': str(fixture)}
        project = output / 'project'
        project.mkdir()
        shutil.copytree(fixture, project / 'rust', ignore=shutil.ignore_patterns('target', 'vendor', '.wisp-cache', '__pycache__'))
        shutil.copytree(vendor, project / 'rust/vendor')
        (project / 'DEPENDENCY-NOTICES.txt').write_text(notice)
        (project / 'wisp.toml').write_text('[package]\nname="demo:rust-serde-candidate"\nversion="0.1.0"\n\n'
            '[metadata]\ndescription="Unmodified Serde compiled inside the source-built Rust Wasm compiler"\n'
            'license="MIT"\nnotice-files=["DEPENDENCY-NOTICES.txt"]\n\n[build]\ncompiler="rust"\nproject="rust"\n')
        inputs['projectFiles'] = {str(path.relative_to(project)): digest(path) for path in sorted(project.rglob('*')) if path.is_file()}
        json_write(output / 'inputs.json', inputs)
        release = output / 'release'
        run([cli, 'build', project, '--out', release, '--cache', output / 'runtime-cache'], 'wasm-source-build-and-host-packaging')
        run([cli, 'library', 'validate', release], 'host-package-admission')
        receipt = json_read(release / 'build/build.json')
        tool = receipt['tools']['rust']
        require(receipt['contractMode'] == 'inferred' and receipt['packageGenerated'] is True
                and tool['compilerSha256'] == args.compiler_sha256 and Path(tool['path']).resolve() == candidate
                and tool['runner']['sha256'] == inputs['runnerSha256']
                and tool['manifestSha256'] == inputs['manifestSha256']
                and tool['inventorySha256'] == inputs['inventorySha256'], 'Wisp admitted different source compiler inputs')
        require('go' not in receipt['tools'] and 'clang' not in receipt['tools'], 'foreign application compiler was used')
        raw_report_path = release / 'build/units/application/work/rust/build.json'
        raw_report = json_read(raw_report_path)
        mapping = RelocatedBuild(release, raw_report)
        macro_evidence = verify_macros(raw_report, mapping, compiler, compiler_runtime, args.compiler_sha256)
        for package, expected in packages.items():
            require(any(item.get('name') == package and item.get('version') == expected['version'] and item.get('source') == 'vendored'
                        for item in raw_report['dependencies']), f'expected vendored crate missing: {package}')
        staged = release / 'build/units/application/source'
        for relative in ('Cargo.toml', 'Cargo.lock', 'src/lib.rs', 'src/main.rs'):
            require(digest(staged / relative) == digest(fixture / relative), f'Serde fixture was modified before compilation: {relative}')
        json_write(output / 'macro-evidence.json', dict(macro_evidence, rawReportPath=str(raw_report_path), rawReportSha256=digest(raw_report_path),
            pathMapping='Original execution/trace strings remain unchanged. Files moved from originalStagingRoot to publishedReleaseRoot during atomic admission.'))
        actual = run([tools['wasmtime'], 'run', '--invoke', 'roundtrip-score(20,22)', release / 'component.wasm'], 'wasm-component-execution').stdout.strip()
        require(actual == '42', f'expected 42, got {actual!r}')
        negative = output / 'invalid-project'
        shutil.copytree(project, negative)
        with (negative / 'rust/src/lib.rs').open('a') as file:
            file.write('\n#[derive(Serialize)]\nunion InvalidSerdeUnion { value: u32, other: u64 }\n')
        inputs['invalidUnionSourceSha256'] = digest(negative / 'rust/src/lib.rs')
        json_write(output / 'inputs.json', inputs)
        invalid_release = output / 'invalid-release'
        rejected = run([cli, 'build', negative, '--out', invalid_release, '--cache', output / 'runtime-cache'], 'invalid-union-wasm-source-build', success=False)
        diagnostic = rejected.stdout + rejected.stderr
        require(DIAGNOSTIC in diagnostic and 'proc macro panicked' not in diagnostic.lower(), 'invalid union did not produce the normal Serde diagnostic')
        require(not invalid_release.exists() and not list(negative.rglob('app.wasm'))
                and not any(path.name in ('app.wasm', 'component.wasm') for stage in output.glob('.tmp*') for path in stage.rglob('*')),
                'failed union build left an application artifact')
        release_record = json_read(release / 'library-release.json')
        require(release_record['componentSha256'] == digest(release / 'component.wasm') == receipt['componentSha256'], 'admitted Component changed')
        require(release_record['metadata']['notices'].get('DEPENDENCY-NOTICES.txt') == notice, 'admitted release lost dependency notices')
        result.update(status='passed', ordinaryRustProject=True, admittedRelease=True, componentResult=42,
                      macroEvidence=macro_evidence, normalSerdeUnionDiagnostic=True, failedBuildHasNoApplicationArtifact=True,
                      nativeSourceCompilation='forbidden by query-only RUSTC guard',
                      hostPackageTooling='recorded separately; not application compiler evidence',
                      componentSha256=release_record['componentSha256'], cliSha256=inputs['cliSha256'])
        json_write(output / 'result.json', result)
        print('Wisp Serde candidate: admitted Component returned 42; invalid union rejected without an artifact')
        return 0
    except Exception as error:
        result.update(status='failed', error=str(error))
        json_write(output / 'result.json', result)
        raise


if __name__ == '__main__':
    try:
        raise SystemExit(main())
    except (VerificationError, OSError, ValueError, KeyError, subprocess.TimeoutExpired) as error:
        print('Wisp Serde acceptance: ' + str(error), file=sys.stderr)
        raise SystemExit(2)
