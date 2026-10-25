#!/usr/bin/env python3
"""Execute inferred and authored source builds, JCO value checks and Wasm-hosted Serde derives."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import platform
import shlex
import signal
import subprocess
import sys

from acquire import digest, require
from snapshot import freeze_cli

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
CASES = {"sources", "serde", "values"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--prefix", type=Path, help="verify the installed compiler prefix before and after acceptance")
    parser.add_argument("--setup-evidence", type=Path, help="clean installation evidence produced by setup.py")
    parser.add_argument("--runtime-cache", type=Path, help="reuse a content-addressed runtime cache for local diagnosis")
    parser.add_argument("--serde-project", type=Path, default=os.environ.get("WISP_SOURCE_SERDE"), help="Serde fixture with authenticated cargo-vendor directory")
    parser.add_argument("--case", action="append", choices=sorted(CASES), help="diagnose selected cases; a partial run never reports complete acceptance")
    parser.add_argument("--timeout", type=int, default=7200, help="seconds per command")
    args = parser.parse_args()
    require(args.timeout > 0, "command timeout must be positive")
    output = args.out.absolute()
    output.mkdir(parents=True, exist_ok=False)
    logs = output / "logs"
    logs.mkdir()
    cli = output / "wisp"
    cli_sha = freeze_cli(args.cli.absolute(), cli, allow_link=False)
    (output / "frozen-cli.json").write_text(json.dumps({"schema": 1, "kind": "source-build-cli-snapshot", "sha256": cli_sha}) + "\n")
    selected = set(args.case or CASES)
    environment = dict(os.environ, CARGO_INCREMENTAL="0")
    environment.update(WISP_FROZEN_CLI=str(cli), WISP_FROZEN_CLI_SHA256=cli_sha)
    runtime_cache = args.runtime_cache.absolute() if args.runtime_cache else output / "runtime-cache"
    environment["WISP_BUILD_CACHE"] = str(runtime_cache)
    commands, results = [], {}
    summary = {"schema": 1, "status": "running", "complete": False, "selectedCases": sorted(selected)}

    def save():
        (output / "commands.json").write_text(json.dumps(commands, indent=2) + "\n")
        (output / "result.json").write_text(json.dumps(summary, indent=2) + "\n")

    def run(argv, *, cwd=ROOT, success=True, env=None):
        argv = list(map(str, argv))
        print("+ " + shlex.join(argv), flush=True)
        index = len(commands)
        stdout, stderr = logs / f"{index:03}.stdout", logs / f"{index:03}.stderr"
        record = {"argv": argv, "cwd": str(cwd), "stdout": str(stdout), "stderr": str(stderr), "status": "running"}
        commands.append(record)
        save()
        with stdout.open("w") as out_log, stderr.open("w") as err_log:
            process = subprocess.Popen(argv, cwd=cwd, env=env or environment, stdin=subprocess.DEVNULL,
                                       stdout=out_log, stderr=err_log, start_new_session=True)
            try:
                status = process.wait(timeout=args.timeout)
            except subprocess.TimeoutExpired:
                os.killpg(process.pid, signal.SIGTERM)
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(process.pid, signal.SIGKILL)
                    process.wait()
                record["status"] = "timeout"
                save()
                raise
        record.update(status="completed", exitCode=status)
        save()
        require((status == 0) == success, f"command returned {status}; see {stdout} and {stderr}")
        return subprocess.CompletedProcess(argv, status, stdout.read_text(), stderr.read_text())

    try:
        for name in ("WISP_CLANG", "WISP_CLANG_WASI_SYSROOT", "WISP_WIT_BINDGEN", "WISP_GO_TOOLCHAIN", "WISP_LLVM", "WISP_RUST",
                     "WISP_WASMTIME", "WISP_RUST_WASMTIME", "WISP_PREVIEW1_ADAPTER", "WISP_NODE", "WISP_JCO"):
            require(environment.get(name) and Path(environment[name]).exists(), f"configure installed tool: {name}")
        composition_tools = {}
        if "sources" in selected:
            require(environment.get("WISP_WAC") and Path(environment["WISP_WAC"]).is_file(), "set WISP_WAC to pinned wac-cli 0.10.1; run setup.py")
            host = f"{platform.system()}-{platform.machine()}"
            lock = json.loads((ROOT / "tests/component-chain/tools.lock.json").read_text())
            require(host in lock["platforms"], f"no pinned WAC for {host}")
            require(digest(environment["WISP_WAC"]) == lock["platforms"][host]["wac"]["sha256"], "WAC checksum mismatch")
            require(run([environment["WISP_WAC"], "--version"]).stdout.strip() == "wac-cli 0.10.1", "unexpected WAC version")
            composition_tools["wac"] = {"path": environment["WISP_WAC"], "sha256": digest(environment["WISP_WAC"])}
        rust_root = Path(environment["WISP_RUST"]).resolve()
        rust_manifest = json.loads((rust_root / "toolchain.json").read_text())
        expected = {
            "clang": digest(environment["WISP_CLANG"]),
            "goInventory": digest(Path(environment["WISP_GO_TOOLCHAIN"]) / "SHA256SUMS"),
            "rust": rust_manifest["compiler"]["moduleSha256"],
        }
        inputs = {
            "cliSha256": digest(cli), "expectedCompilerInputs": expected,
            "sourceCommit": run(["git", "rev-parse", "HEAD"]).stdout.strip(),
            "sourceStatus": run(["git", "status", "--porcelain"]).stdout,
            "sources": {str(p.relative_to(ROOT)): digest(p) for base in (ROOT / "crates/wisp/src", ROOT / "tests/source-build")
                        for p in sorted(base.rglob("*")) if p.is_file() and "__pycache__" not in p.parts},
            "executionHosts": {name: {"path": environment[name], "sha256": digest(environment[name])}
                               for name in ("WISP_WASMTIME", "WISP_RUST_WASMTIME", "WISP_NODE", "WISP_JCO")},
            "bindingGenerator": {"path": environment["WISP_WIT_BINDGEN"], "sha256": digest(environment["WISP_WIT_BINDGEN"])},
            "compositionTools": composition_tools,
        }
        if args.setup_evidence:
            setup = json.loads((args.setup_evidence / "setup-inputs.json").read_text())
            require(setup["cliSha256"] == inputs["cliSha256"] and setup["cleanInstall"] is True, "CLI differs from clean installation setup")
            require(args.prefix and Path(setup["prefix"]).resolve() == args.prefix.resolve(), "acceptance prefix differs from clean installation")
            inputs["cleanInstallation"] = setup
        (output / "inputs.json").write_text(json.dumps(inputs, indent=2) + "\n")
        if args.prefix:
            before = run([cli, "toolchain", "doctor", "--prefix", args.prefix]).stdout
            (output / "doctor-before.json").write_text(before)
            report = json.loads(before)
            require(report.get("valid") is True and report.get("hosts", {}).get("available") is True
                    and report.get("adapter", {}).get("available") is True,
                    "installed compiler inventory, reactor adapter or execution hosts failed preflight")

        if "sources" in selected:
            run([sys.executable, HERE / "run.py", "--cli", cli, "--out", output / "sources", "--runtime-cache", runtime_cache])
            results["sources"] = json.loads((output / "sources/result.json").read_text())
        if "serde" in selected:
            require(args.serde_project and (args.serde_project / "vendor").is_dir(), "--serde-project must contain fixture and vendor; run setup.py first")
            run([sys.executable, "-I", HERE / "rust-serde.py", "--candidate", rust_root,
                 "--compiler-sha256", expected["rust"], "--compiler-wasmtime", environment["WISP_RUST_WASMTIME"],
                 "--fixture", args.serde_project, "--vendor-dir", args.serde_project / "vendor",
                 "--cli", cli, "--wasmtime", environment["WISP_WASMTIME"], "--preview1-adapter", environment["WISP_PREVIEW1_ADAPTER"],
                 "--node", environment["WISP_NODE"], "--jco", environment["WISP_JCO"], "--out", output / "serde"])
            results["serde"] = json.loads((output / "serde/result.json").read_text())
        if "values" in selected:
            run([sys.executable, HERE / "values.py", "--cli", cli, "--out", output / "values", "--cache", runtime_cache])
            results["values"] = json.loads((output / "values/result.json").read_text())
        # Each receipt must name the exact compiler bytes selected before work
        # started. This catches accidental use of a stale or native compiler.
        receipts = {}
        for path in sorted(output.rglob("build/build.json")):
            receipt = json.loads(path.read_text())
            require(receipt["contractMode"] in {"wit", "inferred"}, "invalid source contract mode")
            tools = receipt["tools"]
            if "clang" in tools:
                require(tools["clang"]["sha256"] == expected["clang"], "receipt selected a different Clang compiler")
            if "go" in tools:
                require(tools["go"]["sha256SumsSha256"] == expected["goInventory"], "receipt selected a different Go compiler inventory")
            if "rust" in tools:
                require(tools["rust"]["compilerSha256"] == expected["rust"], "receipt selected a different Rust compiler")
            receipts[str(path.relative_to(output))] = {"sha256": digest(path), "componentSha256": receipt["componentSha256"], "tools": tools}
        require(receipts, "no source build receipts were produced")
        (output / "compiler-receipts.json").write_text(json.dumps(receipts, indent=2) + "\n")
        if args.prefix:
            after = run([cli, "toolchain", "doctor", "--prefix", args.prefix]).stdout
            (output / "doctor-after.json").write_text(after)
            report = json.loads(after)
            require(report.get("valid") is True and report.get("hosts", {}).get("available") is True
                    and report.get("adapter", {}).get("available") is True,
                    "installed compiler inventory, reactor adapter or execution hosts changed during acceptance")
        complete = selected == CASES
        summary.update(status="passed", complete=complete, cleanInstallVerified=bool(args.setup_evidence),
                       cliSha256=inputs["cliSha256"], compilerInputs=expected, results=results,
                       compilerReceiptCount=len(receipts))
        save()
        print("Source build acceptance passed" + ("" if complete else " for selected cases; full acceptance was not run"))
    except Exception as error:
        summary.update(status="failed", error=str(error))
        save()
        raise


if __name__ == "__main__":
    main()
