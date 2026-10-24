#!/usr/bin/env python3
"""Install a pinned compiler manifest into a fresh Linux amd64 prefix."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import platform
import shlex
import shutil
import subprocess
import tarfile
import tempfile

from acquire import MAX_MANIFEST, digest, download, manifest, require, verify_file

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent


def provision_wac(prefix, cache):
    lock = json.loads((ROOT / "tests/component-chain/tools.lock.json").read_text())
    spec = lock["platforms"]["Linux-x86_64"]["wac"]
    saved = download(spec["url"], spec["sha256"], Path(cache) / (spec["sha256"] + ".wac"), limit=64 * 1024**2)
    native = Path(prefix) / "native"
    native.mkdir(parents=True, exist_ok=True)
    binary = native / "wac"
    shutil.copyfile(saved, binary)
    binary.chmod(0o755)
    require(digest(binary) == spec["sha256"], "installed WAC checksum mismatch")
    return binary, {**spec, "version": lock["versions"]["wac"], "path": str(binary)}


def write_environment(prefix, compiler_environment, exports, node_bin, go_bin):
    env_file = Path(prefix) / "acceptance-env.sh"
    env_file.write_text(compiler_environment + "\n" + "".join(
        f"export {key}={shlex.quote(str(value))}\n" for key, value in exports.items())
        + f"export PATH={shlex.quote(str(node_bin) + os.pathsep + str(go_bin))}:\"$PATH\"\n"
        + "export CARGO_INCREMENTAL=0\n")
    return env_file


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    source = parser.add_mutually_exclusive_group(required=True)
    source.add_argument("--manifest", type=Path)
    source.add_argument("--manifest-url")
    parser.add_argument("--manifest-sha256", required=True)
    parser.add_argument("--bundle-dir", type=Path, help="use locally saved bundles; URLs remain required manifest provenance")
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--prefix", type=Path, required=True)
    parser.add_argument("--cache", type=Path, required=True)
    parser.add_argument("--evidence", type=Path, required=True)
    args = parser.parse_args()
    require(platform.system() == "Linux" and platform.machine() == "x86_64", "source compiler CI currently requires Linux x86_64")
    prefix, cache, evidence, cli = args.prefix.absolute(), args.cache.absolute(), args.evidence.absolute(), args.cli.absolute()
    require(not prefix.exists(), "clean installation requires an absent prefix")
    evidence.mkdir(parents=True, exist_ok=True)
    cache.mkdir(parents=True, exist_ok=True)
    saved_manifest = args.manifest
    if args.manifest_url:
        saved_manifest = download(args.manifest_url, args.manifest_sha256,
                                  cache / (args.manifest_sha256 + ".json"), limit=MAX_MANIFEST)
    locked = manifest(saved_manifest, args.manifest_sha256)
    shutil.copyfile(saved_manifest, evidence / "acquisition-manifest.json")
    commands = []
    environment = dict(os.environ, CARGO_INCREMENTAL="0")

    def run(argv, *, cwd=ROOT, env=environment):
        argv = list(map(str, argv))
        print("+ " + shlex.join(argv), flush=True)
        index = len(commands)
        log = evidence / f"setup-{index:02}.log"
        record = {"argv": argv, "cwd": str(cwd), "log": str(log), "status": "running"}
        commands.append(record)
        (evidence / "setup-commands.json").write_text(json.dumps(commands, indent=2) + "\n")
        with log.open("w") as output:
            result = subprocess.run(argv, cwd=cwd, env=env, stdin=subprocess.DEVNULL,
                                    stdout=output, stderr=subprocess.STDOUT, timeout=10800)
        record.update(status="completed", exitCode=result.returncode)
        (evidence / "setup-commands.json").write_text(json.dumps(commands, indent=2) + "\n")
        require(result.returncode == 0, f"setup failed with exit {result.returncode}; see {log}")
        return log.read_text()

    bundles = []
    for spec in locked["bundles"]:
        archive = (args.bundle_dir / (spec["tool"] + ".wisp.zip")) if args.bundle_dir else download(
            spec["url"], spec["sha256"], cache / (spec["sha256"] + ".wisp.zip"), size=spec["size"])
        verify_file(archive, spec["sha256"], spec["size"])
        record = json.loads(run([cli, "toolchain", "validate", archive]))
        require(record.get("tool") == spec["tool"] and record.get("version") == spec["version"], "bundle identity or version differs from acquisition manifest")
        require(record.get("platform") == "wasm32-wasi" and record.get("sizes", {}).get("nativeBytes") == 0,
                "source compiler CI requires Wasm toolchain bundles without native payloads")
        run([cli, "toolchain", "install", archive, "--prefix", prefix, "--sha256", spec["sha256"]])
        bundles.append({**spec, "localArchive": str(archive.absolute())})
    run([cli, "toolchain", "setup-hosts", "--prefix", prefix, "--cache", cache / "hosts", "--host", "all"])
    doctor = run([cli, "toolchain", "doctor", "--prefix", prefix])
    (evidence / "toolchain-doctor.json").write_text(doctor)
    report = json.loads(doctor)
    require(report.get("valid") is True and report.get("hosts", {}).get("available") is True
            and report.get("adapter", {}).get("available") is True,
            "installed compiler inventory, reactor adapter or execution host capability check failed")
    compiler_environment = run([cli, "toolchain", "env", "--prefix", prefix])

    # Application compilers are Wasm. These pinned native tools install ordinary
    # package dependencies and run the generated host packages.
    support = prefix / "acceptance-tools"
    support.mkdir()
    wac, wac_receipt = provision_wac(prefix, cache)
    require(run([wac, "--version"]).strip() == "wac-cli 0.10.1", "unexpected WAC version")
    tool_lock = json.loads((ROOT / "crates/wisp/assets/tools.lock.json").read_text())
    for name in ("node", "go"):
        spec = tool_lock[name]
        archive = download(spec["url"], spec["sha256"], cache / spec["url"].rsplit("/", 1)[1], limit=512 * 1024**2)
        with tempfile.TemporaryDirectory(dir=support) as temporary:
            with tarfile.open(archive) as source_tar:
                source_tar.extractall(temporary, filter="data")
            roots = list(Path(temporary).iterdir())
            require(len(roots) == 1 and (roots[0] / "bin" / name).is_file(), f"unexpected {name} archive layout")
            roots[0].rename(support / name)
    node_bin, go_bin = support / "node/bin", support / "go/bin"
    environment["PATH"] = os.pathsep.join((str(node_bin), str(go_bin), environment["PATH"]))
    run([node_bin / "node", "--version"])
    run([go_bin / "go", "version"])
    js = support / "js"
    js.mkdir()
    for name in ("package.json", "package-lock.json"):
        shutil.copyfile(ROOT / "crates/wisp/assets/setup-tooling" / name, js / name)
    run([node_bin / "npm", "ci", "--prefix", js, "--ignore-scripts", "--no-audit", "--no-fund"])
    run(["cargo", "fetch", "--locked", "--manifest-path", ROOT / "Cargo.toml"])
    run(["cargo", "fetch", "--locked", "--manifest-path", ROOT / "tests/library-chain/native-seed/Cargo.toml"])
    # Rust producers also publish the native Go SDK used by the import verifier.
    # Its shim builds offline with this distinct, shipped lock graph.
    seed = support / "go-shim-seed"
    (seed / "src").mkdir(parents=True)
    (seed / "src/lib.rs").write_text("")
    shutil.copyfile(ROOT / "crates/wisp/assets/library-go-Cargo.lock", seed / "Cargo.lock")
    (seed / "Cargo.toml").write_text('''[package]
name="wisp-go-shim-seed"
version="0.1.0"
edition="2024"
[workspace]
[lib]
crate-type=["staticlib"]
[dependencies]
wasmtime={version="=50.0.0",default-features=false,features=["component-model","component-model-async","cranelift","runtime","std"]}
serde="=1.0.229"
serde_json="=1.0.151"
wasmtime-wasi={version="=50.0.0",default-features=false,features=["p2"]}
wasmtime-wasi-http={version="=50.0.0",default-features=false,features=["p2","default-send-request"]}
[profile.dev]
debug=0
''')
    run(["cargo", "fetch", "--locked", "--manifest-path", seed / "Cargo.toml"])
    serde = support / "serde"
    shutil.copytree(HERE / "fixtures/serde", serde)
    # cargo vendor authenticates each crate using the committed Cargo.lock.
    run(["cargo", "vendor", "--locked", "--manifest-path", serde / "Cargo.toml", serde / "vendor"])
    exports = {
        "WISP_NODE": node_bin / "node", "WISP_JCO": js / "node_modules/.bin/jco",
        "WISP_GO": go_bin / "go", "WISP_SOURCE_SERDE": serde, "WISP_WAC": wac,
    }
    env_file = write_environment(prefix, compiler_environment, exports, node_bin, go_bin)
    (evidence / "setup-inputs.json").write_text(json.dumps({
        "schema": 1, "manifestSha256": args.manifest_sha256, "cliSha256": digest(cli),
        "prefix": str(prefix), "bundles": bundles, "hostPackagingArchives": {name: tool_lock[name] for name in ("node", "go")},
        "compositionTool": wac_receipt,
        "npmLockSha256": digest(js / "package-lock.json"), "serdeLockSha256": digest(serde / "Cargo.lock"),
        "environment": str(env_file), "cleanInstall": True,
    }, indent=2) + "\n")
    print(f"Clean compiler installation ready. Source {env_file}")


if __name__ == "__main__":
    main()
