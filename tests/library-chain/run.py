#!/usr/bin/env python3
"""Install Wisp Rust/npm libraries from loopback registries and execute all consumers."""

import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

from registry import Registries

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(ROOT / "tests/source-build"))
from snapshot import freeze_cli, verify_cli, worker_digest
PROOF = ROOT / "tests/component-chain"
EXPECTED = "ok(21)\nerr(overflow)\nok(0)"
TARGET = "wasm32-unknown-unknown"
PACKAGE = "wisp-wisp--library-chain"


def sha256(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def require(condition, message):
    if not condition:
        raise RuntimeError(message)


def write(path, contents):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(contents)


class Runner:
    def __init__(self, args):
        self.general = args.general
        self.fixture = HERE / "general" if self.general else HERE
        self.identity = "wisp:catalog" if self.general else "wisp:library-chain"
        self.package = "wisp-wisp--catalog" if self.general else PACKAGE
        self.npm_package = "@wisp/catalog" if self.general else "@wisp/library-chain"
        self.expected = "nested-success\ntyped-empty\ntyped-invalid(bad)\nhello world\ncount(2)\nshared-state(2)" if self.general else EXPECTED
        self.library_wit = self.fixture / "library/wit/world.wit" if self.general else HERE / "library.wit"
        self.source_inputs = {str(path.relative_to(HERE)): sha256(path) for path in
                              [HERE / "run.py", HERE / "registry.py", self.library_wit,
                               *[self.fixture / name for name in ["native.rs", "component.rs", "component.wit", "consumer.mjs"]]]}
        self.output = args.output_dir.resolve()
        self.build = (args.build_dir or (self.output / "cargo-target")).resolve()
        self.tools = args.tools_dir.resolve()
        self.wisp = args.wisp.resolve()
        if args.replay:
            self.wisp = self.output / "packaging-wisp"
        self.jco = args.jco.resolve()
        self.commands = []
        self.env = os.environ.copy()
        self.env["RUSTUP_TOOLCHAIN"] = "1.98.1"
        self.env["RUSTUP_AUTO_INSTALL"] = "0"
        self.env["WISP_JCO"] = str(self.jco)
        self.env["WISP_NODE"] = str(args.node_dir.resolve() / "bin/node")
        self.env["PATH"] = str(args.node_dir.resolve() / "bin") + os.pathsep + self.env["PATH"]
        # Keep compiled host dependencies between native consumers without changing Cargo sources.
        self.env["CARGO_TARGET_DIR"] = str(self.build)
        self.env["CARGO_PROFILE_DEV_DEBUG"] = "0"

    def run(self, argv, cwd=None, offline=True):
        argv = list(map(str, argv))
        print("+ " + " ".join(argv), flush=True)
        env = self.env.copy()
        if offline:
            env["CARGO_NET_OFFLINE"] = "true"
            env["npm_config_offline"] = "true"
        else:
            env.pop("CARGO_NET_OFFLINE", None)
            env.pop("npm_config_offline", None)
        result = subprocess.run(argv, cwd=cwd or ROOT, env=env, text=True, capture_output=True)
        self.commands.append({"argv": argv, "cwd": str(cwd or ROOT), "offline": offline,
                              "status": result.returncode, "stdout": result.stdout, "stderr": result.stderr})
        require(result.returncode == 0, f"command failed: {' '.join(argv)}\n{result.stdout}{result.stderr}")
        return result.stdout.strip()

    def prepare(self):
        require(not (self.output / "release").exists(), "output already prepared; use --replay or a fresh --output-dir")
        self.output.mkdir(parents=True, exist_ok=True)
        snapshot = self.output / "packaging-wisp"
        freeze_cli(self.wisp, snapshot, environment=self.env)
        self.wisp = snapshot
        source_inputs = {}
        for base in [ROOT / "crates/wisp/src", ROOT / "crates/wisp/assets"]:
            for source in sorted(base.rglob("*")):
                if not source.is_file():
                    continue
                relative = source.relative_to(ROOT / "crates/wisp")
                destination = self.output / "packaging-sources" / relative
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(source, destination)
                source_inputs[str(relative)] = sha256(source)
        write(self.output / "packaging-tool.json", json.dumps({
            "binary_sha256": sha256(snapshot), "worker_sha256": worker_digest(snapshot),
            "sources": source_inputs,
            "source_commit": self.run(["git", "rev-parse", "HEAD"]),
            "source_status": self.run(["git", "status", "--porcelain"]),
        }, indent=2) + "\n")
        require(self.run(["rustc", "--version"]).split()[1] == "1.98.1", "requires Rust 1.98.1")
        require(self.run(["node", "--version"]) == "v24.20.0", "run setup.py for pinned Node 24.20.0")
        require(self.run(["npm", "--version"]) == "11.19.0", "run setup.py for pinned npm 11.19.0")
        self.run(["rustup", "target", "add", TARGET], offline=False)
        # Build A from the same locked guest source used by the architecture proof.
        library_manifest = self.fixture / "library/Cargo.toml" if self.general else PROOF / "Cargo.toml"
        self.run(["cargo", "build", "--manifest-path", library_manifest, "--locked",
                  "--offline", "--release", "--target", TARGET])
        author = self.output / "author"
        write(author / "wit/library.wit", self.library_wit.read_text())
        component = author / "component.wasm"
        library_core = "catalog_library.wasm" if self.general else "chain_library.wasm"
        self.run([self.tools / "wasm-tools", "component", "new",
                  self.build / TARGET / "release" / library_core, "-o", component])
        write(author / "wisp.toml", f'''[package]
name = "{self.identity}"
version = "0.1.0"
[component]
path = "component.wasm"
sha256 = "{sha256(component)}"
''')
        release = self.output / "release"
        self.run([self.wisp, "library", "pack", author, "--out", release])
        metadata = json.loads(self.run(["cargo", "metadata", "--no-deps", "--format-version", "1",
                                        "--manifest-path", release / "rust/Cargo.toml"]))

        native = self.output / "native"
        consumer = self.output / "component-consumer"
        node = self.output / "node-consumer"
        write(native / "src/main.rs", (self.fixture / "native.rs").read_text())
        write(consumer / "src/lib.rs", (self.fixture / "component.rs").read_text())
        write(consumer / "wit/world.wit", (self.fixture / "component.wit").read_text())
        write(consumer / "wit/deps/library/library.wit", self.library_wit.read_text())
        write(node / "consumer.mjs", (self.fixture / "consumer.mjs").read_text())
        write(node / "package.json", json.dumps({"name": "wisp-library-acceptance", "version": "0.0.0",
                                                 "private": True, "type": "module"}, indent=2))
        with Registries(release, metadata) as registries:
            config = f'[registries.wisp-test]\nindex = "sparse+{registries.url}/cargo/"\n'
            for directory, name, library in [(native, "native-consumer", False), (consumer, "component-consumer", True)]:
                write(directory / ".cargo/config.toml", config)
                manifest = f'''[package]
name = "{name}"
version = "0.0.0"
edition = "2024"
publish = false
[workspace]
[dependencies]
{self.package} = {{ version = "=0.1.0", registry = "wisp-test" }}
'''
                if library:
                    manifest += 'wit-bindgen = "=0.61.1"\n[lib]\ncrate-type = ["cdylib"]\n'
                write(directory / "Cargo.toml", manifest)
                self.run(["cargo", "fetch", "--manifest-path", directory / "Cargo.toml"], directory, offline=False)
            self.run(["npm", "install", "--save-exact", "--ignore-scripts", "--no-audit", "--no-fund",
                      "--registry", registries.url + "/npm/", "--cache", self.output / "npm-cache",
                      f"{self.npm_package}@0.1.0"], node, offline=False)
            require(f"/crates/{self.package}/0.1.0/download" in registries.requests,
                    "Cargo did not download the generated package from the local registry")
            require("/npm/package.tgz" in registries.requests,
                    "npm did not download the generated package from the local registry")
            write(self.output / "registry-requests.json", json.dumps(registries.requests, indent=2) + "\n")
            write(self.output / "registry-url.txt", registries.url + "\n")
        # Servers are closed before the first consumer build or invocation.
        self.replay()

    def replay(self):
        release = self.output / "release"
        require((release / "library-release.json").is_file(), "prepare the local registry installations first")
        report_path = self.output / "report.json"
        report_path.unlink(missing_ok=True)
        packaging_tool = json.loads((self.output / "packaging-tool.json").read_text())
        verify_cli(self.wisp, packaging_tool["binary_sha256"], packaging_tool.get("worker_sha256"),
                   environment=self.env)
        require(self.run(["node", "--version"]) == "v24.20.0", "requires pinned Node 24.20.0")
        require(self.run(["npm", "--version"]) == "11.19.0", "requires pinned npm 11.19.0")
        self.run([self.wisp, "library", "validate", release])
        if not self.general:
            self.run([sys.executable, HERE / "admission.py", release, "--wisp", self.wisp,
                      "--report", self.output / "admission-report.json"])
        source_component_hash = sha256(self.output / "author/component.wasm")
        require(sha256(release / "component.wasm") == source_component_hash, "packaging changed original Component")
        native = self.output / "native"
        consumer = self.output / "component-consumer"
        node = self.output / "node-consumer"
        native_output = self.run(["cargo", "run", "--locked", "--offline", "--manifest-path", native / "Cargo.toml"], native)
        require(native_output == self.expected, f"native Rust returned {native_output!r}")
        self.run(["npm", "ci", "--offline", "--ignore-scripts", "--no-audit", "--no-fund",
                  "--cache", self.output / "npm-cache"], node)
        node_output = self.run(["node", "consumer.mjs"], node)
        require(node_output == self.expected, f"Node returned {node_output!r}")
        self.run(["cargo", "build", "--locked", "--offline", "--release", "--target", TARGET,
                  "--manifest-path", consumer / "Cargo.toml"], consumer)
        graph = self.run(["cargo", "tree", "--locked", "--offline", "--target", TARGET,
                          "--edges", "normal,build", "--manifest-path", consumer / "Cargo.toml"], consumer)
        require("wasmtime" not in graph and "wasmer" not in graph and "wasmi " not in graph,
                "Component guest contains a host runtime dependency")
        write(self.output / "guest-cargo-tree.txt", graph + "\n")
        b = self.output / "consumer.wasm"
        self.run([self.tools / "wasm-tools", "component", "new",
                  self.build / TARGET / "release/component_consumer.wasm", "-o", b])
        composition = self.output / "chain.wac"
        imports = ["types", "catalog", "formatting"] if self.general else ["arithmetic"]
        mappings = "\n".join(f'    "{self.identity}/{name}@0.1.0": library["{self.identity}/{name}@0.1.0"],'
                             for name in imports)
        checkout = "wisp:catalog-consumer/checkout@0.1.0" if self.general else "wisp:library-consumer/checkout@0.1.0"
        shared_types = 'export library["wisp:catalog/types@0.1.0"];\n' if self.general else ""
        write(composition, f'''package wisp:installed-library-chain;
let library = new wisp:library {{}};
let consumer = new wisp:consumer {{
{mappings}
}};
{shared_types}export consumer["{checkout}"];
''')
        composed = self.output / "composed.wasm"
        compose_args = [self.tools / "wac", "compose", composition, "--dep", f"wisp:library={release}/component.wasm",
                        "--dep", f"wisp:consumer={b}"]
        self.run([*compose_args, "-o", composed])
        self.run([*compose_args, "-o", self.output / "composed-repeat.wasm"])
        require(sha256(composed) == sha256(self.output / "composed-repeat.wasm"), "composition was not reproducible")
        self.run([self.tools / "wasm-tools", "validate", composed])
        wit = self.run([self.tools / "wasm-tools", "component", "wit", composed])
        require("\n  import " not in wit, "composed consumer has unresolved imports")
        write(self.output / "composed.wit", wit + "\n")
        results = []
        cases = json.loads((HERE / "general/cases.json").read_text()) if self.general else [
            {"call": f"total({{quantity: {quantity}, unit-price: {price}}})", "expected": expected}
            for quantity, price, expected in [(3, 7, "ok(21)"), (2, 18446744073709551615, "err(overflow)"),
                                             (0, 18446744073709551615, "ok(0)")]]
        for case in cases:
            actual = self.run([self.tools / "wasmtime", "run", "--invoke", case["call"], composed])
            direct = actual if case.get("composed_only") else self.run([
                self.tools / "wasmtime", "run", "--invoke", case["call"], release / "component.wasm"])
            require(actual == case["expected"] and direct == actual,
                    f"Component case {case['call']}: expected {case['expected']!r}, direct={direct!r}, composed={actual!r}")
            results.append(actual)
        component_output = "\n".join(results)
        require(sha256(release / "component.wasm") == source_component_hash, "composition modified library bytes")
        # Cargo.lock must prove registry installation, with the published archive checksum.
        cargo_lock = (native / "Cargo.lock").read_text()
        package_lock = next((block for block in cargo_lock.split("[[package]]")
                             if f'\nname = "{self.package}"\n' in block), "")
        registry_url = (self.output / "registry-url.txt").read_text().strip()
        require(f'source = "sparse+{registry_url}/cargo/"' in package_lock,
                "native package was not installed through sparse Cargo")
        require(sha256(release / "rust.crate") in package_lock, "Cargo lock does not bind the generated archive")
        node_lock = (node / "package-lock.json").read_text()
        require("http://127.0.0.1:" in node_lock, "npm package was not installed through its registry")
        artifacts = ["release/component.wasm", "release/rust.crate", "release/node.tgz", "consumer.wasm",
                     "composed.wasm", "native/Cargo.lock", "component-consumer/Cargo.lock", "node-consumer/package-lock.json"]
        report = {
            "schema": 1, "verified_at": datetime.now(timezone.utc).isoformat(),
            "rustc": self.run(["rustc", "--version"]), "node": self.run(["node", "--version"]),
            "build_directory": str(self.build),
            "packaging_tool": packaging_tool,
            "tools": {name: {"sha256": sha256(self.tools / name),
                              "version": self.run([self.tools / name, "--version"])}
                      for name in ["wasm-tools", "wac", "wasmtime"]},
            "original_component_sha256": source_component_hash,
            "results": {"native": native_output, "node": node_output, "component": component_output},
            "registry_requests": json.loads((self.output / "registry-requests.json").read_text()),
            "artifacts": {name: sha256(self.output / name) for name in artifacts},
            "source_inputs": self.source_inputs,
            "component_cases": cases,
            "commands": self.commands,
        }
        write(report_path, json.dumps(report, indent=2) + "\n")
        print(f"PASS: Cargo, npm and composed Component consumers agree.\n{report_path}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--replay", action="store_true", help="reuse prepared registries' caches and locks entirely offline")
    parser.add_argument("--general", action="store_true", help="exercise the nested multi-interface catalog contract")
    parser.add_argument("--output-dir", type=Path, default=ROOT / "out/library-chain")
    parser.add_argument("--build-dir", type=Path, help="reuse a Cargo target directory across independent registry installations")
    parser.add_argument("--tools-dir", type=Path, default=ROOT / ".tools/component-library-chain")
    parser.add_argument("--wisp", type=Path, default=ROOT / "target/debug/wisp")
    parser.add_argument("--jco", type=Path, default=ROOT / ".tools/library-js/node_modules/.bin/jco")
    parser.add_argument("--node-dir", type=Path, default=ROOT / ".tools/library-chain-node/node")
    args = parser.parse_args()
    runner = Runner(args)
    if args.replay:
        runner.replay()
    else:
        runner.prepare()


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, OSError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        sys.exit(1)
