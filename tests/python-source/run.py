#!/usr/bin/env python3
"""Accept an installed Python compiler with real guest builds and package consumers."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import zipfile

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
MODELS = '''from dataclasses import dataclass
from enum import Enum
@dataclass
class Person:
    name: str
    age: int
class Color(Enum):
    DARK_BLUE = 'blue'
    RED = 'red'
Values = list[int]
'''
PROGRAM = '''from .models import Person, Color, Values
from .exports import hello as greet
from typing import Optional
from componentize_py_types import Result, Err
import helper
_calls = 0
def total(values: Values) -> int:
    return sum(values)
def birthday(value: Person) -> Person:
    return Person(value.name, value.age + 1)
def echo_color(value: Color) -> Color:
    return value
def echo(value: bytes) -> bytes:
    return value
def maybe(value: Optional[str]) -> Optional[str]:
    return value
def checked(value: int) -> Result[int, str]:
    if value < 0:
        raise Err('negative')
    return value * 2
def bump() -> int:
    global _calls
    _calls += 1
    return _calls
def package_info() -> str:
    return helper.info()
def resource() -> str:
    from importlib.resources import files
    return files(__package__).joinpath('data.txt').read_text()
def explode() -> int:
    raise RuntimeError('ordinary exception')
'''
HELPER = '''from importlib.metadata import version
from datetime import date
from vendor_prefix import prefix
def info():
    assert prefix == 'locked vendor'
    return version('helper') + ':' + str(date(2026, 1, 2))
'''


def digest(path):
    with Path(path).open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def write(path, text):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)
    return path


def fixture(project, src_layout=False):
    source_root = project / 'src' if src_layout else project
    app = source_root / 'app'
    write(app / '__init__.py', '')
    write(app / 'models.py', MODELS)
    write(app / 'exports.py', 'def hello(name: str) -> str: return "Hello, " + name + "!"\n')
    source = write(app / 'api.py', PROGRAM)
    write(app / 'data.txt', 'original resource')
    wheel = project / 'wheels/helper-1.0-py3-none-any.whl'
    wheel.parent.mkdir(parents=True)
    with zipfile.ZipFile(wheel, 'w') as archive:
        archive.writestr('helper/__init__.py', HELPER)
        archive.writestr('helper-1.0.dist-info/WHEEL', 'Wheel-Version: 1.0\nRoot-Is-Purelib: true\nTag: py3-none-any\n')
        archive.writestr('helper-1.0.dist-info/METADATA', 'Metadata-Version: 2.3\nName: helper\nVersion: 1.0\nRequires-Python: >=3.14\nProvides-Extra: data\nRequires-Dist: vendor-prefix>=1; extra == "data"\nRequires-Dist: absent; sys_platform == "win32"\n')
    vendor = write(project / 'vendor/vendor_prefix.py', "prefix = 'locked vendor'\n")
    files = {'vendor_prefix.py': digest(vendor)}
    vendor_sha = hashlib.sha256(json.dumps(files, sort_keys=True, separators=(',', ':')).encode()).hexdigest()
    lock = {'schema': 1, 'packages': [
        {'name': 'helper', 'version': '1.0', 'path': 'wheels/' + wheel.name, 'sha256': digest(wheel)},
        {'name': 'vendor-prefix', 'version': '1.0', 'path': 'vendor', 'sha256': vendor_sha, 'files': files}]}
    write(project / 'python-deps.lock.json', json.dumps(lock))
    write(project / 'pyproject.toml', '[project]\nrequires-python=">=3.14,<3.15"\ndependencies=["helper[data]>=1,<2"]\n')
    write(project / 'wisp.toml', '[package]\nname="parity:python"\nversion="0.1.0"\n[build]\ncompiler="python"\n'
          + 'project=' + json.dumps('src' if src_layout else '.') + '\nsources=[' + json.dumps(str(source.relative_to(project))) + ']\n')
    return source


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cli', required=True, type=Path)
    parser.add_argument('--wisp-commit', required=True)
    parser.add_argument('--prefix', required=True, type=Path)
    parser.add_argument('--bundles', required=True, type=Path)
    parser.add_argument('--out', required=True, type=Path)
    args = parser.parse_args()
    tested_commit = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=ROOT, text=True, timeout=30).strip()
    require(tested_commit == args.wisp_commit, 'checkout differs from requested Wisp commit')
    cli, prefix, out = args.cli.resolve(), args.prefix.resolve(), args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    environment = dict(os.environ, WISP_TOOLCHAIN_HOME=str(prefix), PYTHONDONTWRITEBYTECODE='1', CARGO_NET_OFFLINE='true', npm_config_offline='true')
    # Resolve compiler packages through the fresh installation only.
    for name in ('WISP_PYTHON_TOOLCHAIN', 'WISP_WASM_TOOLS_TOOLCHAIN', 'WISP_WASM_TOOLS'):
        environment.pop(name, None)
    records, checks = [], []
    report = {'schema': 1, 'passed': False, 'complete': False, 'wispCommit': tested_commit,
              'cliSha256': digest(cli), 'checks': checks, 'commands': records}
    def save():
        write(out / 'result.json', json.dumps(report, indent=2) + '\n')
    def run(argv, *, cwd=ROOT, success=True):
        argv = list(map(str, argv))
        result = subprocess.run(argv, cwd=cwd, env=environment, text=True, capture_output=True, timeout=1800)
        name = f'{len(records):03d}'
        write(out / 'logs' / (name + '.log'), (result.stdout + result.stderr)[-1024 * 1024:])
        records.append({'argv': argv, 'status': result.returncode, 'log': name + '.log'})
        save()
        require((result.returncode == 0) == success, f'command status {result.returncode}: {argv}\n{result.stdout[-2000:]}\n{result.stderr[-4000:]}')
        return result.stdout
    def check(name):
        checks.append(name)
        save()
        print('PASS', name, flush=True)
    save()
    try:
        for name in ('wasm-tools', 'python'):
            archive = args.bundles / (name + '.wisp.zip')
            expected = (args.bundles / (name + '.wisp.zip.sha256')).read_text().split()[0]
            require(digest(archive) == expected, 'bundle checksum differs: ' + name)
            run([cli, 'toolchain', 'install', archive, '--prefix', prefix, '--sha256', expected])
        before = json.loads(run([cli, 'toolchain', 'doctor', '--prefix', prefix]))
        require(before['valid'], 'installed package inventory failed')
        write(out / 'doctor-before.json', json.dumps(before, indent=2))
        report['toolchainPin'] = json.loads((HERE / 'toolchain.lock.json').read_text())
        check('fresh checksum-pinned compiler installation')
        project = out / 'flat'
        source = fixture(project)
        cache = project / 'compiler-cache'
        def build(label, *, full=False, config=project):
            release = project / 'out' / label
            command = [cli, 'build', config, '--out', release, '--cache', cache]
            if not full:
                command.append('--component-only')
            run(command)
            receipt = json.loads((release / 'build/build.json').read_text())
            require(receipt['units']['application']['compiler'] == 'python', 'wrong compiler')
            require(receipt['componentSha256'] == digest(release / 'component.wasm'), 'component hash differs')
            compiler = receipt['tools']['python']['adapter']['path']
            require(Path(compiler).is_relative_to(prefix), 'compiler did not come from the installed bundle')
            application = json.loads(receipt['units']['application']['commands'][0]['stdout'])
            require(application['fullyWasmHostedBuild'] is True, 'application stages were not Wasm hosted')
            require(application['templateBootstrap']['applicationSourceRead'] is False, 'bootstrap consumed application source')
            write(out / 'receipts' / (label + '.json'), json.dumps(receipt, indent=2))
            return release, receipt
        def call(release, expression, expected):
            result = run([os.environ['WISP_WASMTIME'], 'run', '-O', 'opt-level=0', '-C', 'parallel-compilation=n', '--invoke', expression, release / 'component.wasm'])
            require(result.strip() == expected, f'{expression}: {result!r}')
        first, receipt = build('first', full=True)
        require(receipt['contractMode'] == 'inferred', 'API was not inferred')
        required = ['app/api.py', 'app/models.py', 'app/exports.py', 'app/data.txt', 'pyproject.toml', 'python-deps.lock.json', 'wheels/helper-1.0-py3-none-any.whl', 'vendor/vendor_prefix.py']
        require(all(name in receipt['units']['application']['inputs'] for name in required), 'incomplete input inventory')
        consumer = out / 'consumer'
        consumer.mkdir()
        shutil.copyfile(HERE / 'consumer.mjs', consumer / 'consumer.mjs')
        write(consumer / 'package.json', '{"private":true,"type":"module"}\n')
        run(['npm', 'install', '--offline', '--ignore-scripts', '--no-audit', '--no-fund', first / 'node.tgz'], cwd=consumer)
        require(run(['node', 'consumer.mjs'], cwd=consumer).strip() == 'python-installed-consumer-ok', 'installed consumer failed')
        check('installed Node package: records, enums, bytes, options, results, repeated state and errors')
        second, _ = build('second')
        require(digest(first / 'component.wasm') == digest(second / 'component.wasm'), 'identical build changed bytes')
        manifests = {str(p): digest(p) for p in cache.rglob('manifest.json')}
        require(manifests, 'missing cached contract template')
        source.write_text(source.read_text().replace('return sum(values)', 'return sum(values) + 1'))
        changed, changed_receipt = build('body-edit')
        call(changed, 'total([20,22])', '43')
        require(changed_receipt['units']['application']['inputs'] != receipt['units']['application']['inputs'], 'source edit was not hashed')
        require(manifests == {str(p): digest(p) for p in cache.rglob('manifest.json')}, 'body edit changed template')
        write(source.parent / 'data.txt', 'changed resource')
        changed, changed_receipt = build('data-edit')
        call(changed, '"resource"()', '"changed resource"')
        require(changed_receipt['units']['application']['inputs']['app/data.txt'] != receipt['units']['application']['inputs']['app/data.txt'], 'data edit was not hashed')
        check('deterministic rebuild; body/resource changes invalidate bytes and preserve template')
        source.write_text(source.read_text() + '\ndef ready() -> bool: return True\n')
        changed, _ = build('api-edit')
        call(changed, 'ready()', 'true')
        require(len(list(cache.rglob('manifest.json'))) > len(manifests), 'API edit did not select a new template')
        check('API edit provisions a new contract')
        source.write_text(source.read_text() + '\nraise RuntimeError("bad initializer")\n')
        failed = project / 'out/failed'
        run([cli, 'build', project, '--out', failed, '--cache', cache, '--component-only'], success=False)
        require(not failed.exists() and digest(first / 'component.wasm') == receipt['componentSha256'], 'failure damaged output')
        check('initialization failure leaves previous output intact')
        src_project = out / 'src-layout'
        fixture(src_project, src_layout=True)
        src_release, _ = build('src-layout', config=src_project)
        call(src_release, 'package-info()', '"1.0:2026-01-02"')
        check('src layout with locked wheel/vendor metadata and target markers/extras')
        authored = out / 'authored'
        shutil.copytree(ROOT / 'examples/build-python', authored)
        authored_release, authored_receipt = build('authored', config=authored / 'wisp-authored.toml')
        require(authored_receipt['contractMode'] == 'wit', 'authored WIT changed mode')
        call(authored_release, 'greet("世界")', '"Hello, 世界!"')
        call(authored_release, 'checked(-1)', 'err("value must be nonnegative")')
        check('authored contract and local package imports')
        after = json.loads(run([cli, 'toolchain', 'doctor', '--prefix', prefix]))
        require(after['valid'] and before['packages'] == after['packages'], 'installed compiler files changed during use')
        write(out / 'doctor-after.json', json.dumps(after, indent=2))
        check('installed compiler inventory unchanged after all builds')
        report.update(passed=True, complete=True, cleanInstallVerified=True)
        save()
    except Exception as error:
        report['error'] = str(error)
        save()
        raise


if __name__ == '__main__':
    main()
