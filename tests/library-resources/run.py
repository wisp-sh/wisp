#!/usr/bin/env python3
"""Install a resource library from Cargo/npm, then execute native and guest consumers."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(ROOT / "tests/source-build"))
from snapshot import WORKER_NAME, freeze_cli, verify_cli, worker_digest
sys.path.insert(0, str(HERE.parent / "library-chain"))
from registry import Registries


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output-dir", type=Path, default=ROOT / "out/library-resources")
    parser.add_argument("--build-dir", type=Path, required=True)
    parser.add_argument("--wisp", type=Path, default=ROOT / "target/debug/wisp")
    parser.add_argument("--tools-dir", type=Path, default=ROOT / ".tools/component-library-chain")
    args = parser.parse_args()
    out, build, tools = args.output_dir.resolve(), args.build_dir.resolve(), args.tools_dir.resolve()
    if out.exists():
        raise RuntimeError("use a fresh output directory")
    out.mkdir(parents=True)
    env = os.environ.copy()
    env.update(RUSTUP_TOOLCHAIN="1.98.1", CARGO_TARGET_DIR=str(build), CARGO_PROFILE_DEV_DEBUG="0")
    commands = []
    def run(argv, cwd=ROOT, offline=True):
        command = list(map(str, argv))
        print("+ " + " ".join(command), flush=True)
        invocation = env.copy()
        if offline: invocation.update(CARGO_NET_OFFLINE="true", npm_config_offline="true")
        else:
            invocation.pop("CARGO_NET_OFFLINE", None)
            invocation.pop("npm_config_offline", None)
        result = subprocess.run(command, cwd=cwd, env=invocation, text=True, capture_output=True)
        commands.append({"argv":command, "cwd":str(cwd), "offline":offline, "status":result.returncode,
                         "stdout":result.stdout, "stderr":result.stderr})
        (out / "commands.json").write_text(json.dumps(commands, indent=2) + "\n")
        if result.returncode: raise RuntimeError(result.stdout + result.stderr)
        return result.stdout.strip()
    def write(path, text):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)

    wisp = out / "packaging-wisp"
    freeze_cli(args.wisp, wisp, environment=env)
    (out / "inputs.json").write_text(json.dumps({"wisp":digest(wisp), "worker_sha256":worker_digest(wisp), "sources":{
        str(path.relative_to(ROOT)):digest(path) for path in HERE.rglob("*") if path.is_file()
    }}, indent=2) + "\n")
    inputs = json.loads((out / "inputs.json").read_text())
    verify_cli(wisp, inputs["wisp"], inputs.get("worker_sha256"), environment=env)
    run(["cargo", "build", "--locked", "--target", "wasm32-unknown-unknown", "--manifest-path", HERE / "library/Cargo.toml"])
    author, release = out / "author", out / "release"
    shutil.copytree(HERE / "library/wit", author / "wit")
    run([tools / "wasm-tools", "component", "new", build / "wasm32-unknown-unknown/debug/wisp_resource_library.wasm", "-o", author / "component.wasm"])
    write(author / "wisp.toml", f'[package]\nname = "wisp:resource-library"\nversion = "0.1.0"\n[component]\npath = "component.wasm"\nsha256 = "{digest(author / "component.wasm")}"\n')
    run([wisp, "library", "pack", author, "--out", release])
    run([wisp, "library", "validate", release])
    metadata = json.loads(run(["cargo", "metadata", "--no-deps", "--format-version", "1", "--manifest-path", release / "rust/Cargo.toml"]))
    native, guest, node = [out / name for name in ["native", "guest", "node"]]
    write(native / "src/main.rs", (HERE / "native.rs").read_text())
    write(guest / "src/lib.rs", (HERE / "component.rs").read_text())
    write(guest / "wit/world.wit", (HERE / "component.wit").read_text())
    write(node / "consumer.mjs", (HERE / "consumer.mjs").read_text())
    write(node / "package.json", '{"name":"resource-consumer","private":true,"type":"module"}')
    with Registries(release, metadata) as registry:
        for directory, name in [(native,"native-resource-consumer"), (guest,"guest-resource-consumer")]:
            write(directory / ".cargo/config.toml", f'[registries.wisp-test]\nindex = "sparse+{registry.url}/cargo/"\n')
            manifest = f'[package]\nname = "{name}"\nversion = "0.0.0"\nedition = "2024"\n[workspace]\n[dependencies]\nwisp-wisp--resource-library = {{version="=0.1.0",registry="wisp-test"}}\n'
            if directory == guest: manifest += 'wit-bindgen = "=0.61.1"\n[lib]\ncrate-type = ["cdylib"]\n'
            write(directory / "Cargo.toml", manifest)
            run(["cargo", "fetch", "--manifest-path", directory / "Cargo.toml"], directory, offline=False)
        run(["npm", "install", "--save-exact", "--ignore-scripts", "--no-audit", "--no-fund", "--registry", registry.url + "/npm/", "@wisp/resource-library@0.1.0"], node, offline=False)
        requests = list(registry.requests)
    native_output = run(["cargo", "run", "--locked", "--offline"], native)
    assert native_output == "resources-native-passed", native_output
    node_output = run(["node", "consumer.mjs"], node)
    assert node_output == "resources-node-passed", node_output
    run(["cargo", "build", "--locked", "--offline", "--target", "wasm32-unknown-unknown"], guest)
    graph = run(["cargo", "tree", "--locked", "--offline", "--target", "wasm32-unknown-unknown", "--edges", "normal,build"], guest)
    assert "wasmtime" not in graph and "wasmer" not in graph and "wasmi " not in graph, graph
    write(out / "guest-cargo-tree.txt", graph + "\n")
    run([tools / "wasm-tools", "component", "new", build / "wasm32-unknown-unknown/debug/guest_resource_consumer.wasm", "-o", out / "consumer.wasm"])
    write(out / "compose.wac", 'package wisp:resource-proof;\nlet library = new wisp:library {};\nlet consumer = new wisp:consumer { "wisp:resource-library/counters@0.1.0": library["wisp:resource-library/counters@0.1.0"], "wisp:resource-library/warehouse@0.1.0": library["wisp:resource-library/warehouse@0.1.0"] };\nexport consumer.verify;\n')
    run([tools / "wac", "compose", out / "compose.wac", "--dep", f'wisp:library={release / "component.wasm"}', "--dep", f'wisp:consumer={out / "consumer.wasm"}', "-o", out / "composed.wasm"])
    guest_output = run([tools / "wasmtime", "run", "--invoke", "verify()", out / "composed.wasm"])
    assert guest_output == "42", guest_output
    report = {"passed":True, "native":native_output, "node":node_output, "guest":guest_output,
              "registry_requests":requests, "rustc":run(["rustc", "--version"]), "node_version":run(["node", "--version"]),
              "artifacts":{name:digest(out / name) for name in ["packaging-wisp", WORKER_NAME, "release/component.wasm", "release/rust.crate", "release/node.tgz", "consumer.wasm", "composed.wasm"]}}
    write(out / "report.json", json.dumps(report, indent=2) + "\n")
    print("PASS: installed resource library native, Node and Component consumers")


if __name__ == "__main__":
    main()
