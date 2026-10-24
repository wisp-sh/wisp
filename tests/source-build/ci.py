#!/usr/bin/env python3
"""Execute source compilers, published packages, Serde derives and four Rust import flows."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import platform
import shlex
import shutil
import signal
import subprocess
import sys

from acquire import digest, require
import native
from snapshot import freeze_cli

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
SCALARS = '''let mut api = library.api();
assert_eq!(api.add(20,22).unwrap(),42);
assert!(!api.negate(true).unwrap());
assert_eq!(api.widen(9007199254740993).unwrap(),9007199254740994);'''
IMPORTED = SCALARS.replace("api.add(20,22)", "api.doubled_sum(20,1)")
VALUES = r'''let mut api = library.api();
for label in ["", "hello 🦀 世界", "\0start\0end"] {
    assert_eq!(api.echo(label.into()).unwrap(), label);
}
assert_eq!(api.echo("x".repeat(100000)).unwrap().len(),100000);
let packet = api.checked(true).unwrap();
assert_eq!(packet.label,"hello 🦀"); assert_eq!(packet.data,vec![0,128,255]);
assert_eq!(packet.sequence,9007199254740994); assert!(packet.enabled);
let packet = api.PACKET_FN(packet).unwrap();
assert_eq!(packet.sequence,9007199254740994);
assert!(matches!(api.checked(false),Err(sdk::CallError::Guest(error)) if error == "denied"));
for i in 0..1000 {
    let bytes = (0..i%193).map(|j| ((i+j)&255) as u8).collect::<Vec<_>>();
    assert_eq!(api.buffer(bytes.clone()).unwrap(),bytes);
}'''


def values_assertion(case):
    assertion = VALUES.replace("PACKET_FN", "packet_value" if case == "go" else "packet")
    wide_type = "Wide" if case == "go" else "WideRecord"
    wide_fn = "wide_value" if case == "go" else "wide"
    fields = ", ".join(f'{field}: "{field}".repeat(101) + "🦀"' for field in "abcdefghi")
    assertion += f'\nlet wide = sdk::api::{wide_type} {{ {fields} }};\nlet wide = api.{wide_fn}(wide).unwrap();\n'
    assertion += "\n".join(f'assert_eq!(wide.{field}, "{field}".repeat(101) + "🦀");' for field in "abcdefghi")
    if case != "go":
        assertion += r'''
let packet = api.checked(true).unwrap();
assert_eq!(api.relay(Ok(packet)).unwrap().sequence,9007199254740994);
assert!(matches!(api.relay(Err("denied".into())),Err(sdk::CallError::Guest(error)) if error == "denied"));
assert_eq!(api.numeric32(Ok(1.25)).unwrap(),1.25);
assert_eq!(api.numeric64(Ok(std::f64::consts::PI)).unwrap(),std::f64::consts::PI);
assert!(matches!(api.numeric32(Err(42)),Err(sdk::CallError::Guest(42))));
assert!(matches!(api.numeric64(Err(9007199254740994)),Err(sdk::CallError::Guest(9007199254740994))));'''
    return assertion


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--prefix", type=Path, help="verify the installed compiler prefix before and after acceptance")
    parser.add_argument("--setup-evidence", type=Path, help="clean installation evidence produced by setup.py")
    parser.add_argument("--runtime-cache", type=Path, help="reuse a content-addressed runtime cache for local diagnosis")
    parser.add_argument("--serde-project", type=Path, default=os.environ.get("WISP_SOURCE_SERDE"), help="Serde fixture with authenticated cargo-vendor directory")
    parser.add_argument("--case", action="append", choices=["legacy", "rust", "serde", "interop", "values"], help="diagnose selected cases; a partial run never reports complete acceptance")
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
    selected = set(args.case or ["legacy", "rust", "serde", "interop", "values"])
    environment = dict(os.environ, CARGO_INCREMENTAL="0", WISP_BUILD_TEST_TOKEN="test-token")
    environment.update(WISP_FROZEN_CLI=str(cli), WISP_FROZEN_CLI_SHA256=cli_sha)
    environment.setdefault("CARGO_TARGET_DIR", str(output / "native-target"))
    runtime_cache = args.runtime_cache.absolute() if args.runtime_cache else output / "runtime-cache"
    environment["WISP_BUILD_CACHE"] = str(runtime_cache)
    commands, results, native_results = [], {}, {}
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

    def native_consumer(name, release, assertion):
        native_results[name] = native.execute(cli, release, output / "native" / name, assertion, run)

    def copied_example(relative, name):
        project = output / "sources" / name
        shutil.copytree(ROOT / relative, project, ignore=shutil.ignore_patterns(".wisp-cache", "target", "__pycache__"))
        return project

    def build(name, project, *extra):
        release = output / name
        run([cli, "build", project, "--out", release, "--cache", runtime_cache, *extra])
        run([cli, "library", "validate", release])
        return release

    try:
        for name in ("WISP_CLANG", "WISP_GO_TOOLCHAIN", "WISP_LLVM", "WISP_RUST",
                     "WISP_WASMTIME", "WISP_RUST_WASMTIME", "WISP_PREVIEW1_ADAPTER", "WISP_NODE", "WISP_JCO"):
            require(environment.get(name) and Path(environment[name]).exists(), f"configure installed tool: {name}")
        composition_tools = {}
        if selected & {"legacy", "interop", "values"}:
            require(environment.get("WISP_WAC") and Path(environment["WISP_WAC"]).is_file(), "set WISP_WAC to pinned wac-cli 0.10.1; run setup.py")
            host = f"{platform.system()}-{platform.machine()}"
            lock = json.loads((ROOT / "tests/component-chain/tools.lock.json").read_text())
            require(host in lock["platforms"], f"no pinned WAC for {host}")
            require(digest(environment["WISP_WAC"]) == lock["platforms"][host]["wac"]["sha256"], "WAC checksum mismatch")
            require(run([environment["WISP_WAC"], "--version"]).stdout.strip() == "wac-cli 0.10.1", "unexpected WAC version")
            composition_tools["wac"] = {"path": environment["WISP_WAC"], "sha256": digest(environment["WISP_WAC"])}
        if "interop" in selected:
            clangd = environment.get("WISP_CLANGD") or shutil.which("clangd", path=environment["PATH"])
            require(clangd and Path(clangd).is_file(), "set WISP_CLANGD to the native clangd used for editor acceptance")
            environment["WISP_CLANGD"] = str(Path(clangd).absolute())
            version = run([environment["WISP_CLANGD"], "--version"]).stdout.strip()
            composition_tools["clangd"] = {"path": environment["WISP_CLANGD"], "sha256": digest(environment["WISP_CLANGD"]), "version": version}
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

        if "legacy" in selected:
            run([sys.executable, HERE / "run.py", "--cli", cli, "--out", output / "legacy", "--runtime-cache", runtime_cache])
            results["legacy"] = json.loads((output / "legacy/result.json").read_text())
            for case in ("cpp", "go", "mixed", "cpp-import-go", "go-import-wasm"):
                assertion = SCALARS
                if case == "go":
                    assertion += '\nassert_eq!(api.environment_count().unwrap(),0); assert!(api.clock_ready().unwrap()); assert!(api.random_ready().unwrap()); assert!(api.filesystem_denied().unwrap()); assert_eq!(api.stress().unwrap(),42);'
                elif case == "mixed":
                    assertion = 'assert_eq!(library.calculator().doubled_sum(20,1).unwrap(),42);'
                elif case in {"cpp-import-go", "go-import-wasm"}:
                    assertion = IMPORTED if case == "cpp-import-go" else 'assert_eq!(library.api().doubled_sum(20,1).unwrap(),42);'
                native_consumer(case, output / "legacy" / case, assertion)
        if selected & {"rust", "interop"}:
            rust_project = copied_example("examples/build-rust", "rust")
            rust_release = build("rust", rust_project, "--go-module", "example.org/rust-arithmetic")
            native_consumer("rust", rust_release, SCALARS)
            results["rust"] = {"passed": True, "componentSha256": digest(rust_release / "component.wasm")}
        if "serde" in selected:
            require(args.serde_project and (args.serde_project / "vendor").is_dir(), "--serde-project must contain fixture and vendor; run setup.py first")
            run([sys.executable, "-I", HERE / "rust-serde.py", "--candidate", rust_root,
                 "--compiler-sha256", expected["rust"], "--compiler-wasmtime", environment["WISP_RUST_WASMTIME"],
                 "--fixture", args.serde_project, "--vendor-dir", args.serde_project / "vendor",
                 "--cli", cli, "--wasmtime", environment["WISP_WASMTIME"], "--preview1-adapter", environment["WISP_PREVIEW1_ADAPTER"],
                 "--node", environment["WISP_NODE"], "--jco", environment["WISP_JCO"], "--out", output / "serde"])
            results["serde"] = json.loads((output / "serde/result.json").read_text())
            native_consumer("serde", output / "serde/release", 'assert_eq!(library.api().roundtrip_score(20,22).unwrap(),42);')
        if "interop" in selected:
            cpp_release = (output / "legacy/cpp") if "legacy" in selected else build("cpp", copied_example("examples/build-cpp", "cpp"))
            go_release = build("go-arithmetic", copied_example("examples/cpp-import-go/go", "go-arithmetic"))
            run([sys.executable, "-I", ROOT / "examples/rust-import-wasm/verify.py", "--cli", cli,
                 "--rust-release", rust_release, "--go-release", go_release, "--cpp-release", cpp_release,
                 "--out", output / "interop"])
            results["interop"] = json.loads((output / "interop/result.json").read_text())
            for language in ("go", "cpp", "rust"):
                release = output / f"interop-{language}-package"
                run([cli, "library", "pack", output / f"interop/{language}-consumer", "--out", release])
                native_consumer("interop-" + language, release, IMPORTED)
        if "values" in selected:
            run([sys.executable, HERE / "values.py", "--cli", cli, "--out", output / "values", "--cache", runtime_cache])
            results["values"] = json.loads((output / "values/result.json").read_text())
            run([sys.executable, HERE / "values-interop.py", "--release", output / "values/go-release",
                 "--cli", cli, "--out", output / "values-interop", "--cache", runtime_cache])
            results["valuesInterop"] = json.loads((output / "values-interop/result.json").read_text())
            run([sys.executable, HERE / "values-edge.py", "--cli", cli, "--out", output / "values-edge", "--cache", runtime_cache])
            results["valuesEdge"] = json.loads((output / "values-edge/result.json").read_text())
            for case in ("cpp", "go", "rust"):
                native_consumer("values-" + case, output / f"values/{case}-release", values_assertion(case))
            for case in ("cpp", "rust"):
                native_consumer("values-interop-" + case, output / f"values-interop/{case}-release",
                                'for _ in 0..3 { assert_eq!(library.api().score().unwrap(),42); }')
        # Each receipt must name the exact compiler bytes selected before work
        # started. This catches accidental use of a stale or native compiler.
        receipts = {}
        for path in sorted(output.rglob("build/build.json")):
            receipt = json.loads(path.read_text())
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
        complete = selected == {"legacy", "rust", "serde", "interop", "values"}
        summary.update(status="passed", complete=complete, cleanInstallVerified=bool(args.setup_evidence),
                       cliSha256=inputs["cliSha256"], compilerInputs=expected, results=results, nativeConsumers=native_results,
                       compilerReceiptCount=len(receipts))
        save()
        print("Source build acceptance passed" + ("" if complete else " for selected cases; full acceptance was not run"))
    except Exception as error:
        summary.update(status="failed", error=str(error))
        save()
        raise


if __name__ == "__main__":
    main()
