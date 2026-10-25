#!/usr/bin/env python3
"""Build inferred and authored APIs with Wasm compilers and execute them through JCO."""
from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("source_build_snapshot", ROOT / "tests/source-build/snapshot.py")
snapshot = importlib.util.module_from_spec(spec)
spec.loader.exec_module(snapshot)

CASES = {
    "c": "assert.equal(lib.add(20, 22), 42);",
    "cpp": "assert.equal(lib.add(20, 22), 42); assert.equal(lib.negate(true), false); assert.equal(lib.widen(9007199254740993n), 9007199254740994n);",
    "rust": "assert.equal(lib.add(20, 22), 42); assert.equal(lib.negate(false), true); assert.equal(lib.widen(9007199254740993n), 9007199254740994n);",
    "go": "assert.equal(lib.add(20, 22), 42); assert.equal(lib.negate(false), true); assert.equal(lib.widen(9007199254740993n), 9007199254740994n); assert.equal(lib.environmentCount(), 0); assert.ok(lib.clockReady()); assert.equal(lib.stress(), 42); assert.ok(lib.randomReady()); assert.ok(lib.filesystemDenied()); assert.equal(lib.add(20, 22), 42);",
    "mixed": "assert.equal(lib.calculator.doubledSum(20, 1), 42); assert.equal(lib.calculator.richCheck(), 42);",
}


class Acceptance:
    """Keep generated JavaScript and command evidence under the output directory."""
    def __init__(self, cli, output):
        self.output = output
        output.mkdir(parents=True, exist_ok=False)
        self.cli = output / "wisp"
        snapshot.freeze_cli(cli.resolve(strict=True), self.cli)
        self.node = Path(shutil.which(os.environ.get("WISP_NODE", "node")) or os.environ.get("WISP_NODE", "node")).resolve(strict=True)
        self.jco = Path(shutil.which(os.environ.get("WISP_JCO", "jco")) or os.environ.get("WISP_JCO", "jco")).resolve(strict=True)
        self.modules = next((p for p in self.jco.parents if p.name == "node_modules"), None)
        if self.modules is None:
            raise RuntimeError("WISP_JCO must select the installed JCO JavaScript entrypoint")
        (output / "node_modules").symlink_to(self.modules, target_is_directory=True)
        (output / "package.json").write_text('{"private":true,"type":"module"}\n')
        (output / "logs").mkdir()
        self.commands = []

    def run(self, argv, *, success=True):
        argv = list(map(str, argv))
        print("+ " + shlex.join(argv), flush=True)
        index = len(self.commands)
        stdout = self.output / f"logs/{index:03}.stdout"
        stderr = self.output / f"logs/{index:03}.stderr"
        with stdout.open("w") as out_log, stderr.open("w") as err_log:
            process = subprocess.run(argv, cwd=ROOT, stdin=subprocess.DEVNULL, stdout=out_log, stderr=err_log)
        self.commands.append({"argv": argv, "status": process.returncode, "stdout": str(stdout), "stderr": str(stderr)})
        (self.output / "commands.json").write_text(json.dumps(self.commands, indent=2) + "\n")
        if (process.returncode == 0) != success:
            raise RuntimeError(f"command returned {process.returncode}; see {stdout} and {stderr}")
        return stdout.read_text().strip()

    def build(self, example, case, cache):
        project = self.output / f"{case}-source"
        shutil.copytree(example, project, ignore=shutil.ignore_patterns(".wisp-cache", "target", "dist", "__pycache__"))
        release = self.output / f"{case}-release"
        self.run([self.cli, "build", project, "--component-only", "--out", release, "--cache", cache])
        receipt = json.loads((release / "build/build.json").read_text())
        actual = hashlib.sha256((release / "component.wasm").read_bytes()).hexdigest()
        if receipt["contractMode"] not in {"wit", "inferred"} or receipt["packageGenerated"] or receipt["componentSha256"] != actual:
            raise RuntimeError("build receipt differs from the Component")
        return project, release, receipt

    def transpile(self, release, case):
        destination = self.output / f"{case}-js"
        self.run([self.node, self.jco, "transpile", release / "component.wasm", "--out-dir", destination,
                  "--name", "library", "--no-typescript", "--quiet"])
        return destination / "library.js"

    def finish(self, records, **extra):
        report = {"passed": True, "cliSha256": hashlib.sha256(self.cli.read_bytes()).hexdigest(), "cases": records, **extra}
        (self.output / "result.json").write_text(json.dumps(report, indent=2) + "\n")
        print(json.dumps(report, indent=2))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--runtime-cache", type=Path, required=True)
    parser.add_argument("--case", action="append", choices=CASES)
    args = parser.parse_args()
    acceptance = Acceptance(args.cli, args.out.resolve())
    records = {}
    for case in dict.fromkeys(args.case or CASES):
        _, release, receipt = acceptance.build(ROOT / f"examples/build-{case}", case, args.runtime_cache.resolve())
        module = acceptance.transpile(release, case)
        verifier = acceptance.output / f"{case}-check.mjs"
        verifier.write_text(
            "import assert from 'node:assert/strict';\n"
            "import { _setEnv, _setArgs } from '@bytecodealliance/preview2-shim/cli';\n"
            "import { _setPreopens } from '@bytecodealliance/preview2-shim/filesystem';\n"
            "_setEnv({}); _setArgs([]); _setPreopens({});\n"
            f"const module = await import({json.dumps(module.as_uri())}); const lib = module.api ?? module;\n"
            + CASES[case] + f"\nconsole.log('source-{case}-ok');\n"
        )
        result = acceptance.run([acceptance.node, verifier])
        if result != f"source-{case}-ok":
            raise RuntimeError(result)
        records[case] = {"componentSha256": receipt["componentSha256"], "units": list(receipt["units"]), "runtimeResult": result}
    broken = acceptance.output / "broken-source"
    shutil.copytree(ROOT / "examples/build-cpp", broken)
    (broken / "math.cpp").write_text("#error deliberate source acceptance failure\n")
    failed_output = acceptance.output / "failed-build"
    acceptance.run([acceptance.cli, "build", broken, "--component-only", "--out", failed_output,
                    "--cache", args.runtime_cache.resolve()], success=False)
    if failed_output.exists():
        raise RuntimeError("failed compiler left a published output")
    acceptance.finish(records, compilerFailureAtomic=True)


if __name__ == "__main__":
    main()
