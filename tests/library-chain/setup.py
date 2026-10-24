#!/usr/bin/env python3
"""Provision the installed-library acceptance tools and locked dependency caches."""

import hashlib
import io
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import tarfile
import tempfile
import urllib.request

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]


def run(argv, env):
    print("+ " + " ".join(map(str, argv)), flush=True)
    subprocess.run(list(map(str, argv)), cwd=ROOT, env=env, check=True)


def main():
    lock = json.loads((ROOT / "crates/wisp/assets/library-tools.lock.json").read_text())
    host = f"{platform.system()}-{platform.machine()}"
    if host not in lock["platforms"]:
        raise SystemExit(f"no pinned installed-library toolchain for {host}")
    spec = lock["platforms"][host]
    with urllib.request.urlopen(spec["url"], timeout=120) as response:
        payload = response.read()
    if hashlib.sha256(payload).hexdigest() != spec["sha256"]:
        raise SystemExit("Node archive checksum mismatch")
    tools = ROOT / ".tools/library-chain-node"
    tools.mkdir(parents=True, exist_ok=True)
    destination = tools / "node"
    with tempfile.TemporaryDirectory(dir=tools, prefix="extract-") as temporary:
        with tarfile.open(fileobj=io.BytesIO(payload)) as archive:
            archive.extractall(temporary, filter="data")
        roots = list(Path(temporary).iterdir())
        if len(roots) != 1 or not (roots[0] / "bin/node").is_file():
            raise SystemExit("unexpected Node archive layout")
        if destination.exists():
            shutil.rmtree(destination)
        roots[0].rename(destination)
    binary_sha256 = hashlib.sha256((destination / "bin/node").read_bytes()).hexdigest()
    if binary_sha256 != spec["node_sha256"]:
        raise SystemExit("Node executable checksum mismatch")
    (tools / "installed.json").write_text(json.dumps({
        **spec,
    }, indent=2) + "\n")
    env = os.environ.copy()
    env["PATH"] = str(destination / "bin") + os.pathsep + env["PATH"]
    env["RUSTUP_TOOLCHAIN"] = "1.98.1"
    run(["node", "--version"], env)
    run(["npm", "--version"], env)
    run(["rustup", "toolchain", "install", "1.98.1", "--profile", "minimal", "--target", "wasm32-unknown-unknown"], env)
    run(["python3", ROOT / "tests/component-chain/run.py", "--setup"], env)
    js = ROOT / ".tools/library-js"
    js.mkdir(parents=True, exist_ok=True)
    for name in ("package.json", "package-lock.json"):
        shutil.copyfile(ROOT / "crates/wisp/assets/setup-tooling" / name, js / name)
    run(["npm", "ci", "--prefix", js, "--ignore-scripts", "--no-audit", "--no-fund"], env)
    run(["cargo", "fetch", "--locked", "--manifest-path", HERE / "native-seed/Cargo.toml"], env)
    run(["cargo", "fetch", "--locked", "--manifest-path", ROOT / "Cargo.toml"], env)
    for manifest in [
        ROOT / "tests/library-chain/general/library/Cargo.toml",
        ROOT / "tests/library-async/guest/Cargo.toml",
        ROOT / "tests/library-async/native/Cargo.toml",
        ROOT / "tests/library-resources/library/Cargo.toml",
        ROOT / "tests/library-network/library/Cargo.toml",
    ]:
        run(["cargo", "fetch", "--locked", "--manifest-path", manifest], env)
    # The Go package compiles an embedded Rust static library. Fetch its exact
    # pinned graph before acceptance runs turn registry access off.
    with tempfile.TemporaryDirectory(prefix="wisp-go-shim-seed-") as temporary:
        seed = Path(temporary)
        (seed / "src").mkdir()
        (seed / "src/lib.rs").write_text("")
        (seed / "Cargo.lock").write_bytes((ROOT / "crates/wisp/assets/library-go-Cargo.lock").read_bytes())
        (seed / "Cargo.toml").write_text('''[package]
name = "wisp-go-shim-seed"
version = "0.1.0"
edition = "2024"
[workspace]
[lib]
crate-type = ["staticlib"]
[dependencies]
wasmtime = {version="=50.0.0",default-features=false,features=["component-model","component-model-async","cranelift","runtime","std"]}
serde = "=1.0.229"
serde_json = "=1.0.151"
wasmtime-wasi = {version="=50.0.0",default-features=false,features=["p2"]}
wasmtime-wasi-http = {version="=50.0.0",default-features=false,features=["p2","default-send-request"]}
[profile.dev]
debug = 0
''')
        run(["cargo", "fetch", "--locked", "--manifest-path", seed / "Cargo.toml"], env)
    print("Installed-library setup complete.")


if __name__ == "__main__":
    main()
