#!/usr/bin/env python3
"""Execute the candidate installed OCaml adapter and generated native consumers."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import time
import tomllib

ROOT = Path(__file__).resolve().parents[2]


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--out', required=True, type=Path)
    parser.add_argument('--buildtools', required=True, type=Path)
    args = parser.parse_args()
    out = args.out.resolve(); commands=[]
    env = dict(os.environ)
    report = {'passed':False, 'complete':False}
    (out/'result.json').write_text(json.dumps(report)+'\n')
    logs = out/'logs'; logs.mkdir()
    def run(argv, *, cwd=ROOT, environment=env, success=True, input_data=None):
        argv=list(map(str,argv)); index=len(commands)
        path=logs/f'{index:03}.stdout.log'
        errors=logs/f'{index:03}.stderr.log'
        print('+ '+' '.join(argv),flush=True)
        started=time.monotonic()
        with path.open('w') as stream, errors.open('w') as stderr:
            result=subprocess.run(argv,cwd=cwd,env=environment,
                                  stdin=subprocess.DEVNULL if input_data is None else None,
                                  input=input_data,stdout=stream,stderr=stderr,timeout=3600)
        commands.append({'argv':argv,'cwd':str(cwd),'exitCode':result.returncode,
                         'elapsedSeconds':round(time.monotonic()-started,3),'log':str(path),'stderrLog':str(errors),
                         'stdinSha256':hashlib.sha256(input_data).hexdigest() if input_data is not None else None})
        (out/'commands.json').write_text(json.dumps(commands,indent=2)+'\n')
        if (result.returncode==0)!=success:
            print(path.read_text()[-24000:],file=sys.stderr)
            print(errors.read_text()[-24000:],file=sys.stderr)
            raise RuntimeError(f'command failed; see {path}')
        return path.read_text().strip()
    cli = ROOT/'target/debug/wisp'
    installed = Path(env['WISP_OCAML_TOOLCHAIN'])
    for tool in ('cargo','rustc','node','npm'):
        assert shutil.which(tool, path=env['PATH']), f'installed acceptance PATH is missing {tool}'
    report['consumerTools']={tool:shutil.which(tool,path=env['PATH']) for tool in ('cargo','rustc','node','npm')}
    (out/'result.json').write_text(json.dumps(report,indent=2)+'\n')
    guard = out/'guard'; guard.mkdir()
    attempts = out/'native-attempts.txt'; attempts.touch()
    for name in ('ocaml','ocamlc','ocamlrun','ocamlopt','opam','dune','gcc','cc','clang','wasm-ld'):
        path=guard/name
        path.write_text('#!/bin/sh\nprintf "%s\\n" "$0" >> "$WISP_NATIVE_TOOL_ATTEMPTS"\nexit 97\n')
        path.chmod(0o755)
    guarded = dict(env,PATH=str(guard)+os.pathsep+env['PATH'],WISP_NATIVE_TOOL_ATTEMPTS=str(attempts))
    command = out/'command'; command.mkdir()
    shutil.copyfile(args.buildtools/'ocaml/examples/check.ml',command/'check.ml')
    run([sys.executable,installed/'compile.py',command/'check.ml','-o',command/'check.wasm'],environment=guarded)
    (command/'input.bin').write_bytes(bytes([0,255,128,1]))
    result=run([env['WISP_WASMTIME'],'run','-W','exceptions=y','--dir',f'{command}::/',command/'check.wasm'],environment=guarded)
    assert result=='42' and (command/'output.bin').read_bytes()==bytes([0,255,128,1])
    project=out/'project'; shutil.copytree(ROOT/'examples/build-ocaml',project)
    (project/'unused.bin').write_bytes(b'not an OCaml source input')
    release=out/'release'
    run([cli,'build',project,'--out',release,'--cache',out/'cache'],environment=guarded)
    run([cli,'library','validate',release])
    receipt=json.loads((release/'build/build.json').read_text())
    assert receipt['contractMode']=='inferred' and receipt['componentSha256']==digest(release/'component.wasm')
    assert set(receipt['units']['application']['inputs']) == {'library.ml','library.mli','support.ml'}
    assert not attempts.read_text(), 'native application compiler was invoked'
    candidate_receipts=[x['adapterReceipt'] for x in receipt['units']['application']['commands'] if 'adapterReceipt' in x]
    assert len(candidate_receipts)==1 and candidate_receipts[0]['fullyWasmHostedBuild']
    assert candidate_receipts[0]['compilerManifestSha256']==digest(installed/'dist/manifest.json')
    for name in ('library.ml','library.mli','support.ml'):
        assert candidate_receipts[0]['sources'][name]==digest(project/name)
    # Install the actual archives into consumer directories outside the source
    # tree. Both consumers call repeatedly in one Component instance, including GC.
    npm=out/'npm'; npm.mkdir()
    (npm/'package.json').write_text('{"private":true,"type":"module"}\n')
    shutil.copyfile(ROOT/'tests/ocaml/consumer.mjs',npm/'consumer.mjs')
    run(['npm','install','--offline','--ignore-scripts','--no-audit','--no-fund',release/'node.tgz'],cwd=npm)
    package_report=json.loads((release/'library-release.json').read_text())
    node_flags=package_report['targetRequirements']['node']['flags']
    assert '--experimental-wasm-exnref' in node_flags
    run(['node',*node_flags,'consumer.mjs'],cwd=npm)
    rust=out/'rust'; rust.mkdir(); (rust/'src').mkdir()
    crate=out/'installed-rust'; crate.mkdir()
    with tarfile.open(release/'rust.crate') as archive: archive.extractall(crate,filter='data')
    package=next(crate.iterdir())
    name=tomllib.loads((package/'Cargo.toml').read_text())['package']['name']
    assert name=='wisp-example--ocaml-values'
    (rust/'Cargo.toml').write_text('[package]\nname="ocaml-consumer"\nversion="0.1.0"\nedition="2024"\n[workspace]\n[dependencies]\n'+name+'={path='+json.dumps(str(package))+'}\n[profile.dev]\ndebug=0\n')
    shutil.copyfile(ROOT/'tests/ocaml/consumer.rs',rust/'src/main.rs')
    run(['cargo','run','--offline','-j1','--manifest-path',rust/'Cargo.toml'])
    # Mutating a dependency must change both the receipt and executable output.
    (project/'support.ml').write_text('let offset = 3l\n')
    changed=out/'changed'
    run([cli,'build',project,'--component-only','--out',changed,'--cache',out/'cache'],environment=guarded)
    assert digest(changed/'component.wasm')!=digest(release/'component.wasm')
    request={'version':1,'id':1,'module':str(changed/'component.wasm'),
             'sha256':digest(changed/'component.wasm'),'interface':'example:ocaml-values/api@0.1.0',
             'function':'add','params':[20,20]}
    response=json.loads(run([env['WISP_WORKER'],'serve'],input_data=(json.dumps(request)+'\n').encode()))
    assert response['version']==1 and response['id']==1 and response['ok'] and response['exit_code']==0
    assert response['results']==[43], response
    (project/'library.ml').write_text('let malformed = (\n')
    run([cli,'build',project,'--component-only','--out',out/'failed','--cache',out/'cache'],environment=guarded,success=False)
    assert not (out/'failed').exists()
    (project/'library.mli').write_text("val unsupported : 'a -> 'a\n")
    run([cli,'build',project,'--component-only','--out',out/'bad-api'],environment=guarded,success=False)
    assert not (out/'bad-api').exists()
    report.update(passed=True,complete=True,cleanInstall=True,componentSha256=digest(release/'component.wasm'),
                  compilerManifestSha256=candidate_receipts[0]['compilerManifestSha256'],nativeToolAttempts=[],
                  checks=['installed command compiler','binary file I/O','inferred .mli API','multiple modules',
                          'installed npm consumer','installed Rust consumer','Unicode and binary bytes','float records',
                          'list and option','typed results','64-bit integers','200 repeated calls with GC/compaction',
                          'unexpected exceptions and poisoned instance','dependency invalidation','atomic compile failure','unsupported API rejection'])
    (out/'result.json').write_text(json.dumps(report,indent=2)+'\n')
    print(json.dumps(report,indent=2))


if __name__=='__main__': main()
