#!/usr/bin/env python3
"""Build and execute the library composition proof. Only --setup uses the network."""

import argparse
from datetime import datetime, timezone
import hashlib
import io
import json
import os
from pathlib import Path
import platform
import re
import subprocess
import sys
import tarfile
import urllib.request

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
TARGET = "wasm32-unknown-unknown"
RUST_VERSION = "1.93.0"
ARITHMETIC = "wisp:library-chain/arithmetic@0.1.0"


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def guest_packages(metadata):
    packages = sorted(f"{p['name']}@{p['version']}" for p in metadata["packages"])
    prohibited = [p for p in packages if re.match(r"(?:wasmtime|wasmer|wasmi)(?:-|@)", p)]
    require(not prohibited, f"guest build includes a host runtime: {prohibited}")
    return packages


def world_imports(wit):
    # wasm-tools emits the decoded component world first, before package interfaces.
    world = re.search(r"\bworld\s+\S+\s*\{(.*?)\n\}", wit, re.S)
    require(world is not None, "wasm-tools did not emit a component world")
    return re.findall(r"^\s*import\s+([^;]+);", world.group(1), re.M)


def run(args, commands=None, offline=True):
    args = [str(arg) for arg in args]
    print("+ " + " ".join(args), flush=True)
    env = os.environ.copy()
    if offline:
        env["CARGO_NET_OFFLINE"] = "true"
        env["RUSTUP_AUTO_INSTALL"] = "0"
    result = subprocess.run(args, cwd=HERE, env=env, text=True, capture_output=True)
    if commands is not None:
        commands.append({"argv": args, "status": result.returncode,
                         "stdout": result.stdout, "stderr": result.stderr})
    require(result.returncode == 0,
            f"command failed with status {result.returncode}: {' '.join(args)}\n"
            f"{result.stdout}{result.stderr}")
    return result.stdout.strip()


def setup(tools_dir, lock):
    host = f"{platform.system()}-{platform.machine()}"
    require(host in lock["platforms"], f"no pinned proof tools for {host}")
    tools_dir.mkdir(parents=True, exist_ok=True)
    installed = {}
    for name, spec in lock["platforms"][host].items():
        print(f"Fetching {name} {lock['versions'][name]}", flush=True)
        with urllib.request.urlopen(spec["url"], timeout=120) as response:
            payload = response.read()
        require(hashlib.sha256(payload).hexdigest() == spec["sha256"],
                f"download hash mismatch for {name}")
        if spec["url"].endswith((".tar.gz", ".tar.xz")):
            with tarfile.open(fileobj=io.BytesIO(payload)) as archive:
                candidates = [m for m in archive.getmembers()
                              if m.isfile() and m.name.endswith("/" + name)]
                require(len(candidates) == 1, f"ambiguous {name} archive")
                payload = archive.extractfile(candidates[0]).read()
        binary = tools_dir / name
        temporary = tools_dir / (name + ".part")
        temporary.write_bytes(payload)
        temporary.chmod(0o755)
        temporary.replace(binary)
        installed[name] = {**spec, "binary_sha256": sha256(binary)}
    (tools_dir / "installed.json").write_text(json.dumps(installed, indent=2) + "\n")
    run(["rustup", "target", "add", TARGET], offline=False)
    run(["cargo", "fetch", "--locked", "--manifest-path", HERE / "Cargo.toml"], offline=False)
    print("Setup complete. Run without --setup for the offline proof.")


def prove(tools_dir, output_dir, lock):
    output_dir.mkdir(parents=True, exist_ok=True)
    # Remove stale success evidence before any gate can fail.
    report_path = output_dir / "report.json"
    report_path.unlink(missing_ok=True)
    commands = []
    rustc = run(["rustc", "--version"], commands)
    require(rustc.split()[1] == RUST_VERSION,
            f"proof requires Rust {RUST_VERSION}; found {rustc}. "
            f"Install it and use RUSTUP_TOOLCHAIN={RUST_VERSION}.")
    installed_path = tools_dir / "installed.json"
    require(installed_path.is_file(), "proof tools are not provisioned; run with --setup")
    installed = json.loads(installed_path.read_text())
    tool_evidence = {}
    for name, version in lock["versions"].items():
        binary = tools_dir / name
        require(binary.is_file(), f"missing {name}; run with --setup")
        require(sha256(binary) == installed[name]["binary_sha256"], f"modified tool: {name}")
        actual = run([binary, "--version"], commands)
        require(re.search(rf"\b{re.escape(version)}\b", actual) is not None,
                f"expected {name} {version}, found {actual}")
        tool_evidence[name] = {"version": actual, "path": str(binary), "sha256": sha256(binary)}

    manifest = HERE / "Cargo.toml"
    metadata = json.loads(run(["cargo", "metadata", "--format-version", "1", "--locked",
                               "--offline", "--manifest-path", manifest], commands))
    packages = guest_packages(metadata)
    (output_dir / "cargo-metadata.json").write_text(json.dumps(metadata, indent=2) + "\n")
    build_dir = output_dir / "cargo-build"
    run(["cargo", "build", "--release", "--locked", "--offline", "--target", TARGET,
         "--target-dir", build_dir, "--manifest-path", manifest], commands)
    wasm_tools, wac, wasmtime = [tools_dir / x for x in ("wasm-tools", "wac", "wasmtime")]
    imports = {}
    for name in ("library", "consumer"):
        core = build_dir / TARGET / "release" / f"chain_{name}.wasm"
        component = output_dir / f"{name}.wasm"
        run([wasm_tools, "component", "new", core, "-o", component], commands)
        run([wasm_tools, "validate", component], commands)
        wit = run([wasm_tools, "component", "wit", component], commands)
        (output_dir / f"{name}.wit").write_text(wit + "\n")
        imports[name] = world_imports(wit)
    require(imports["library"] == [], "library has unexpected host imports")
    require(imports["consumer"] == [ARITHMETIC], "consumer has unexpected Component imports")
    original = {name: sha256(output_dir / f"{name}.wasm") for name in ("library", "consumer")}
    compose = [wac, "compose", HERE / "chain.wac", "--dep",
               f"wisp:library={output_dir}/library.wasm", "--dep",
               f"wisp:consumer={output_dir}/consumer.wasm"]
    for name in ("composed", "composed-repeat"):
        run([*compose, "-o", output_dir / f"{name}.wasm"], commands)
    composed = output_dir / "composed.wasm"
    require(sha256(composed) == sha256(output_dir / "composed-repeat.wasm"),
            "same composition inputs produced different bytes")
    for name, digest in original.items():
        require(sha256(output_dir / f"{name}.wasm") == digest,
                f"composition modified original {name} bytes")
    run([wasm_tools, "validate", composed], commands)
    wit = run([wasm_tools, "component", "wit", composed], commands)
    (output_dir / "composed.wit").write_text(wit + "\n")
    imports["composed"] = world_imports(wit)
    require(imports["composed"] == [], "composition retains unresolved imports")

    cases = {}
    for name, call, expected in [
        ("success", "total({quantity: 3, unit-price: 7})", "ok(21)"),
        ("overflow", "total({quantity: 2, unit-price: 18446744073709551615})", "err(overflow)"),
        ("zero", "total({quantity: 0, unit-price: 18446744073709551615})", "ok(0)"),
    ]:
        cases[name] = {"call": call, "expected": expected}
        for artifact in ("library", "composed"):
            actual = run([wasmtime, "run", "--invoke", call, output_dir / f"{artifact}.wasm"], commands)
            require(actual == expected, f"{artifact}/{name}: expected {expected}, got {actual}")
            cases[name][artifact] = actual

    inputs = ["Cargo.toml", "Cargo.lock", "tools.lock.json", "chain.wac", "wit/world.wit",
              "library/Cargo.toml", "library/src/lib.rs", "consumer/Cargo.toml",
              "consumer/src/lib.rs", "run.py"]
    report = {
        "schema": 1, "verified_at": datetime.now(timezone.utc).isoformat(),
        "host": platform.platform(), "rustc": rustc, "target": TARGET,
        "source_commit": run(["git", "rev-parse", "HEAD"], commands),
        "source_status": run(["git", "status", "--porcelain", "--", str(HERE)], commands),
        "inputs": {name: sha256(HERE / name) for name in inputs},
        "tools": tool_evidence, "guest_packages": packages, "host_imports": imports,
        "artifacts": {name: {"path": str(output_dir / f"{name}.wasm"),
                             "sha256": sha256(output_dir / f"{name}.wasm")}
                      for name in ("library", "consumer", "composed", "composed-repeat")},
        "cases": cases, "commands": commands,
    }
    report_path.write_text(json.dumps(report, indent=2) + "\n")
    print(f"PASS: library and composed consumer agree on success, overflow and zero.\n{report_path}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--setup", action="store_true", help="download tools and Cargo dependencies, then exit")
    parser.add_argument("--tools-dir", type=Path, default=ROOT / ".tools/component-library-chain")
    parser.add_argument("--output-dir", type=Path, default=ROOT / "out/component-library-chain")
    args = parser.parse_args()
    lock = json.loads((HERE / "tools.lock.json").read_text())
    if args.setup:
        setup(args.tools_dir.resolve(), lock)
    else:
        prove(args.tools_dir.resolve(), args.output_dir.resolve(), lock)


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, OSError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        sys.exit(1)
