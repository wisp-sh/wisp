#!/usr/bin/env python3
"""Compile ordinary rich APIs, admit packages, and execute installed Node consumers."""
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
    result = subprocess.run([str(v) for v in command], cwd=cwd, text=True, capture_output=True)
    if result.returncode:
        raise RuntimeError(f"{command}\n{result.stdout}\n{result.stderr}")
    return result.stdout


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path, default=ROOT / "target/debug/wisp")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--cache", type=Path, default=ROOT / ".tools/build/runtime-cache")
    parser.add_argument("--case", action="append", choices=["cpp", "go", "rust"])
    args = parser.parse_args()
    output = args.out.resolve()
    output.mkdir(parents=True, exist_ok=False)
    cli = output / "wisp"
    snapshot_module.freeze_cli(args.cli.resolve(), cli)
    records = []
    for case in args.case or ["cpp", "go", "rust"]:
        project = output / f"{case}-source"
        shutil.copytree(ROOT / "examples/build-values" / case, project)
        release = output / f"{case}-release"
        run([cli, "build", project, "--out", release, "--cache", args.cache.resolve()])
        run([cli, "library", "validate", release])
        consumer = output / f"{case}-consumer"
        consumer.mkdir()
        (consumer / "package.json").write_text('{"private":true,"type":"module"}\n')
        run(["npm", "install", "--offline", "--ignore-scripts", "--no-audit", "--no-fund", release / "node.tgz"], cwd=consumer)
        package = json.loads((release / "library-release.json").read_text())["packages"]["node"]["name"]
        source = "import * as lib from " + json.dumps(package) + ";\n" + JAVASCRIPT.replace("PACKET_FN", "packetValue" if case == "go" else "packet").replace("WIDE_FN", "wideValue" if case == "go" else "wide").replace("HAS_RELAY", "false" if case == "go" else "true")
        (consumer / "check.mjs").write_text(source)
        result = run([os.environ.get("WISP_NODE", "node"), "check.mjs"], cwd=consumer).strip()
        if result != "source-values-ok":
            raise RuntimeError(result)
        receipt = json.loads((release / "build/build.json").read_text())
        records.append({"language": case, "result": result, "componentSha256": receipt["componentSha256"], "tools": receipt["tools"], "receipt": str(release / "build/build.json")})
    result = {"passed": True, "cliSha256": hashlib.sha256(cli.read_bytes()).hexdigest(), "cases": records}
    (output / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))


JAVASCRIPT = r"""
import assert from 'node:assert/strict';
const api = lib.api;
if (!HAS_RELAY) assert.deepEqual(api.note({value:'source type named WispNote'}), {value:'source type named WispNote'});
for (const label of ['', 'hello', 'hello 🦀 世界', '\u0000start\u0000end', 'x'.repeat(100000)]) {
  assert.equal(api.echo(label), label);
}
const packet = {label: 'hello 🦀', data: new Uint8Array([0,128,255]), sequence: 9007199254740994n, enabled: true};
assert.deepEqual(api.PACKET_FN(packet), packet);
assert.deepEqual(api.checked(true), packet);
assert.throws(() => api.checked(false), error => String(error).includes('denied') || error.payload === 'denied');
if (HAS_RELAY) {
  assert.deepEqual(api.relay({tag:'ok',val:packet}), packet);
  assert.throws(() => api.relay({tag:'err',val:'denied'}), error => String(error).includes('denied') || error.payload === 'denied');
  assert.equal(api.numeric32({tag:'ok',val:1.25}), 1.25);
  assert.equal(api.numeric64({tag:'ok',val:Math.PI}), Math.PI);
  assert.throws(() => api.numeric32({tag:'err',val:42}), error => error.payload === 42);
  assert.throws(() => api.numeric64({tag:'err',val:9007199254740994n}), error => error.payload === 9007199254740994n);
}
const wide = Object.fromEntries('abcdefghi'.split('').map(c => [c, c.repeat(101)+'🦀']));
assert.deepEqual(api.WIDE_FN(wide), wide);
for (let i=0; i<1000; ++i) {
  const bytes = Uint8Array.from({length: i % 193}, (_, j) => (i + j) & 255);
  assert.deepEqual(api.buffer(bytes), bytes);
  const value = {...packet,label:`hello ${i} 🦀`, data: bytes,enabled:!!(i%2)};
  assert.deepEqual(api.PACKET_FN(value), value);
  assert.throws(() => api.checked(false));
}
console.log('source-values-ok');
"""


if __name__ == "__main__":
    main()
