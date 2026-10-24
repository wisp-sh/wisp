#!/usr/bin/env python3
"""Execute nested and unit-payload result APIs compiled by the source toolchains."""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[2]
snapshot_spec = importlib.util.spec_from_file_location("source_build_snapshot", ROOT / "tests/source-build/snapshot.py")
snapshot_module = importlib.util.module_from_spec(snapshot_spec)
snapshot_spec.loader.exec_module(snapshot_module)


def run(command, *, cwd=None):
    result = subprocess.run([str(value) for value in command], cwd=cwd, text=True, capture_output=True)
    if result.returncode:
        raise RuntimeError(f"{command}\n{result.stdout}\n{result.stderr}")
    return result.stdout


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path, default=ROOT / "target/debug/wisp")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--cache", type=Path, default=ROOT / ".tools/build/runtime-cache")
    parser.add_argument("--case", action="append", choices=["cpp", "rust"])
    args = parser.parse_args()
    output = args.out.resolve()
    output.mkdir(parents=True, exist_ok=False)
    cli = output / "wisp"
    snapshot_module.freeze_cli(args.cli.resolve(), cli)
    records = []
    for case in args.case or ["cpp", "rust"]:
        print(f"Building {case} nested and unit-payload results", flush=True)
        project = output / f"{case}-source"
        project.mkdir()
        build = '[build]\ncompiler = "clang"\nsources = ["main.cpp"]\n'
        if case == "cpp":
            (project / "main.cpp").write_text(CPP)
        else:
            crate = project / "rust"
            (crate / "src").mkdir(parents=True)
            (crate / "Cargo.toml").write_text('[package]\nname="result-edges"\nversion="0.1.0"\nedition="2021"\n[workspace]\n')
            (crate / "Cargo.lock").write_text('version=4\n[[package]]\nname="result-edges"\nversion="0.1.0"\n')
            (crate / "src/lib.rs").write_text(RUST)
            build = '[build]\ncompiler = "rust"\nproject = "rust"\n'
        (project / "wisp.toml").write_text(
            f'[package]\nname="test:{case}-result-edges"\nversion="0.1.0"\n'
            '[metadata]\nlicense="MIT"\ndescription="Nested and unit result regression"\n' + build
        )
        release = output / f"{case}-release"
        run([cli, "build", project, "--out", release, "--cache", args.cache.resolve()])
        run([cli, "library", "validate", release])
        consumer = output / f"{case}-node"
        consumer.mkdir()
        (consumer / "package.json").write_text('{"private":true,"type":"module"}\n')
        run(["npm", "install", "--offline", "--ignore-scripts", "--no-audit", "--no-fund", release / "node.tgz"], cwd=consumer)
        package = json.loads((release / "library-release.json").read_text())["packages"]["node"]["name"]
        (consumer / "check.mjs").write_text("import {api} from " + json.dumps(package) + ";\n" + JAVASCRIPT)
        result = run([os.environ.get("WISP_NODE", "node"), "check.mjs"], cwd=consumer).strip()
        if result != "result-edges-ok":
            raise RuntimeError(result)
        receipt = json.loads((release / "build/build.json").read_text())
        records.append({"language": case, "result": result, "componentSha256": receipt["componentSha256"], "tools": receipt["tools"], "receipt": str(release / "build/build.json")})
    result = {"passed": True, "cliSha256": hashlib.sha256(cli.read_bytes()).hexdigest(), "cases": records}
    (output / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))


CPP = r'''
#include "wisp_types.hpp"
wisp::result<void,void> empty(wisp::result<void,void> value) { return value; }
wisp::result<wisp::unit,wisp::string> error_only(wisp::result<wisp::unit,wisp::string> value) { return value; }
wisp::result<wisp::string,void> ok_only(wisp::result<wisp::string,void> value) { return value; }
wisp::result<wisp::result<wisp::string,unsigned>,wisp::result<wisp::bytes,unsigned long long>> nested(wisp::result<wisp::result<wisp::string,unsigned>,wisp::result<wisp::bytes,unsigned long long>> value) { return value; }
struct Boxed { wisp::result<wisp::string,unsigned> result; wisp::bytes bytes; };
Boxed record_result(Boxed value) { return value; }
'''

RUST = r'''
pub fn empty(value:Result<(),()>)->Result<(),()>{value}
pub fn error_only(value:Result<(),String>)->Result<(),String>{value}
pub fn ok_only(value:Result<String,()>)->Result<String,()>{value}
pub fn nested(value:Result<Result<String,u32>,Result<Vec<u8>,u64>>)->Result<Result<String,u32>,Result<Vec<u8>,u64>>{value}
pub struct Boxed { pub result:Result<String,u32>, pub bytes:Vec<u8> }
pub fn record_result(value:Boxed)->Boxed{value}
'''

JAVASCRIPT = r'''
import assert from 'node:assert/strict';
for(let i=0;i<100;i++) {
  assert.equal(api.empty({tag:'ok'}),undefined);
  assert.throws(()=>api.empty({tag:'err'}));
  assert.equal(api.errorOnly({tag:'ok'}),undefined);
  assert.throws(()=>api.errorOnly({tag:'err',val:'denied'}),error=>error.payload==='denied');
  assert.equal(api.okOnly({tag:'ok',val:'hello 🦀'}),'hello 🦀');
  assert.throws(()=>api.okOnly({tag:'err'}));
  assert.deepEqual(api.nested({tag:'ok',val:{tag:'ok',val:'hello 🦀'}}),{tag:'ok',val:'hello 🦀'});
  assert.deepEqual(api.nested({tag:'ok',val:{tag:'err',val:42}}),{tag:'err',val:42});
  assert.throws(()=>api.nested({tag:'err',val:{tag:'ok',val:new Uint8Array([0,128,255])}}),error=>{assert.deepEqual(error.payload,{tag:'ok',val:new Uint8Array([0,128,255])});return true;});
  assert.throws(()=>api.nested({tag:'err',val:{tag:'err',val:9007199254740994n}}),error=>{assert.deepEqual(error.payload,{tag:'err',val:9007199254740994n});return true;});
  for(const result of [{tag:'ok',val:'record 🦀'},{tag:'err',val:42}]) {
    const record={result,bytes:new Uint8Array([0,128,255])};
    assert.deepEqual(api.recordResult(record),record);
  }
}
console.log('result-edges-ok');
'''

if __name__ == "__main__":
    main()
