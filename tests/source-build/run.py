#!/usr/bin/env python3
"""Build with the real Wasm compilers, publish locally, and execute installed npm packages."""
import argparse
import hashlib
import io
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import tarfile

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("publication_registry", ROOT / "tests/library-publication/registry.py")
registry_module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(registry_module)
snapshot_spec = importlib.util.spec_from_file_location("source_build_snapshot", ROOT / "tests/source-build/snapshot.py")
snapshot_module = importlib.util.module_from_spec(snapshot_spec)
snapshot_spec.loader.exec_module(snapshot_module)



def verify_reactor_admission(cli, release, work, run):
    tampered = work / "tampered-reactor"
    tampered.mkdir()
    for name in ["component.wasm", "rust.crate", "node.tgz", "library-release.json"]:
        shutil.copyfile(release / name, tampered / name)
    archive = io.BytesIO()
    changed = False
    with tarfile.open(release / "node.tgz", "r:gz") as original, tarfile.open(fileobj=archive, mode="w:gz", format=tarfile.USTAR_FORMAT) as rebuilt:
        for member in original:
            if not member.isfile():
                rebuilt.addfile(member)
                continue
            data = original.extractfile(member).read()
            if member.name == "package/reactor-host.js":
                data += b"\nexport const tampered = true;\n"
                changed = True
            member.size = len(data)
            member.mtime = int(member.mtime)
            rebuilt.addfile(member, io.BytesIO(data))
    assert changed
    data = archive.getvalue()
    (tampered / "node.tgz").write_bytes(data)
    record = json.loads((tampered / "library-release.json").read_text())
    record["packages"]["node"]["sha256"] = hashlib.sha256(data).hexdigest()
    (tampered / "library-release.json").write_text(json.dumps(record))
    failure = run([cli, "library", "validate", tampered], success=False)
    assert "regenerated Node package differs" in failure.stderr, failure.stderr


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--runtime-cache", type=Path, required=True)
    parser.add_argument("--case", action="append", choices=["cpp", "go", "mixed", "go-import-wasm", "cpp-import-go"],
                        help="Run selected cases; defaults to all examples")
    args = parser.parse_args()
    cli = args.cli.resolve()
    output = args.out.resolve()
    output.mkdir(parents=True, exist_ok=False)
    frozen_cli = output / "wisp"
    snapshot_module.freeze_cli(cli, frozen_cli)
    cli = frozen_cli
    runtime_cache = args.runtime_cache.resolve()
    commands = []

    def run(argv, *, cwd=ROOT, success=True, environment=None):
        argv = list(map(str, argv))
        print("+ " + " ".join(argv), flush=True)
        result = subprocess.run(argv, cwd=cwd, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE, env=dict(os.environ, WISP_BUILD_TEST_TOKEN="test-token", **(environment or {})))
        commands.append({"argv": argv, "status": result.returncode, "stdout": result.stdout, "stderr": result.stderr})
        (output / "commands.json").write_text(json.dumps(commands, indent=2) + "\n")
        assert (result.returncode == 0) == success, result.stdout + result.stderr
        return result

    cases = {
        "cpp": "const {api:a}=lib; if(a.add(20,22)!==42||a.negate(true)!==false||a.widen(9007199254740993n)!==9007199254740994n) throw Error('C++ values');",
        "go": "const {api:a}=lib; if(a.add(20,22)!==42||a.negate(false)!==true||a.widen(9007199254740993n)!==9007199254740994n||a.environmentCount()!==0||!a.clockReady()||a.stress()!==42||!a.randomReady()||!a.filesystemDenied()||a.add(20,22)!==42||!a.clockReady()) throw Error('Go values or host isolation');",
        "mixed": "if(lib.calculator.doubledSum(20,1)!==42) throw Error('Go did not call C++ dependency');",
        "cpp-import-go": "if(lib.api.doubledSum(20,1)!==42||lib.api.negate(true)!==false||lib.api.widen(9007199254740993n)!==9007199254740994n) throw Error('C++ did not call Go dependency');",
        "go-import-wasm": "if(lib.api.doubledSum(20,1)!==42) throw Error('Go did not call the prebuilt Wasm dependency');",
    }
    results = {}
    with tempfile.TemporaryDirectory(prefix="wisp-build-installed-") as temporary:
        installed = Path(temporary)
        for case, assertions in cases.items():
            if args.case and case not in args.case:
                continue
            release = output / case
            original = ROOT / f"examples/build-{case}"
            project = output / "sources" / case
            if case not in {"go-import-wasm", "cpp-import-go"}:
                shutil.copytree(original, project, ignore=shutil.ignore_patterns(".wisp-cache"))
            environment = None
            if case == "go-import-wasm":
                project = installed / "go-consumer-source"
                shutil.copytree(ROOT / "examples/go-import-wasm", project,
                                ignore=shutil.ignore_patterns("cpp", "bindings", "proxy", "github.com", ".wisp-cache", "__pycache__"))
                assert not list(project.rglob("*.cpp"))
                environment = {"WISP_CLANG": "/clang-must-not-be-used"}
            if case == "cpp-import-go":
                project = installed / "cpp-consumer-source"
                shutil.copytree(ROOT / "examples/cpp-import-go", project,
                                ignore=shutil.ignore_patterns("go", "bindings", ".wisp-cache", "__pycache__"))
                assert not list(project.rglob("*.go"))
                environment = {"WISP_GO_TOOLCHAIN": "/go-must-not-be-used", "WISP_PREVIEW1_ADAPTER": "/adapter-must-not-be-used"}
            run([cli, "build", project, "--out", release, "--cache", runtime_cache], environment=environment)
            record = json.loads((release / "library-release.json").read_text())
            receipt = json.loads((release / "build/build.json").read_text())
            assert record["componentSha256"] == receipt["componentSha256"]
            assert receipt["packageGenerated"]
            if case == "go-import-wasm":
                assert "clang" not in receipt["tools"]
                dependency = next(unit for unit in receipt["units"].values() if unit["compiler"] == "component")
                assert dependency["compiler"] == "component"
                assert dependency["commands"] == []
                assert dependency["componentSha256"] == hashlib.sha256((project / "go/vendor/example.org/cpp-arithmetic/component.wasm").read_bytes()).hexdigest()
            if case == "cpp-import-go":
                assert "go" not in receipt["tools"] and "preview1Adapter" not in receipt["tools"]
                assert receipt["contractMode"] == "inferred"
                dependency = next(unit for unit in receipt["units"].values() if unit["compiler"] == "component")
                assert dependency["commands"] == []
                assert dependency["componentSha256"] == hashlib.sha256((project / "deps/arithmetic/component.wasm").read_bytes()).hexdigest()
            if case != "cpp":
                assert "Go-LICENSE.txt" in record["metadata"]["notices"]
                assert (release / "node/notices/Go-LICENSE.txt").is_file()
            with registry_module.Registry() as registry:
                config = {
                    "cargo": {"index": registry.url + "/cargo/", "api": registry.url, "package": record["packages"]["rust"]["name"], "tokenEnv": "WISP_BUILD_TEST_TOKEN"},
                    "npm": {"registry": registry.url + "/npm/", "package": record["packages"]["node"]["name"], "access": "public", "tokenEnv": "WISP_BUILD_TEST_TOKEN"},
                    "component": {"registry": registry.url, "repository": f"demo/{case}", "tokenEnv": "WISP_BUILD_TEST_TOKEN"},
                }
                config_path = output / f"{case}-publisher.json"
                config_path.write_text(json.dumps(config))
                plan = output / f"{case}-publication.json"
                state = output / f"{case}-publication-state.json"
                run([cli, "library", "prepare", release, "--config", config_path, "--out", plan])
                assert not registry.requests, "prepare contacted a registry"
                run([cli, "library", "publish", plan, "--state", state])
                assert json.loads(state.read_text())["complete"]
                fetched = output / f"{case}-fetched"
                digest = json.loads(plan.read_text())["component"]["digest"]
                run([cli, "library", "fetch", config_path, "--digest", digest, "--out", fetched])
                for name in ["component.wasm", "rust.crate", "node.tgz", "library-release.json"]:
                    assert (release / name).read_bytes() == (fetched / name).read_bytes(), name
                consumer = installed / case
                consumer.mkdir()
                (consumer / "package.json").write_text('{"private":true,"type":"module"}')
                package = record["packages"]["node"]["name"]
                run(["npm", "install", "--ignore-scripts", "--no-audit", "--no-fund", "--registry", registry.url + "/npm/", "--cache", output / "npm-cache", f"{package}@{record['version']}"], cwd=consumer)
            # The registry is gone. Execute the installed package with no code downloads.
            run([os.environ.get("WISP_NODE", "node"), "--input-type=module", "-e", f"const lib=await import({json.dumps(package)}); {assertions} console.log('installed-{case}-ok');"], cwd=consumer)
            results[case] = {"componentSha256": record["componentSha256"], "units": list(receipt["units"]), "cargoNpmOciPublished": True, "ociFetchIdentical": True, "installedNpmExecutedAfterRegistryShutdown": True}
        if "go" in results:
            verify_reactor_admission(cli, output / "go", installed, run)
        broken = installed / "broken"
        shutil.copytree(ROOT / "examples/build-cpp", broken)
        (broken / "math.cpp").write_text("unsigned add(unsigned a, unsigned b) { return missing_function(a, b); }\n")
        failed_output = output / "failed-build"
        run([cli, "build", broken, "--component-only", "--out", failed_output, "--cache", runtime_cache], success=False)
        assert not failed_output.exists(), "failed build left a published output"
    (output / "result.json").write_text(json.dumps({"passed": True, "cliSha256": hashlib.sha256(cli.read_bytes()).hexdigest(), "cases": results, "compilerFailureAtomic": True, "reactorHostTamperRejected": "go" in results}, indent=2) + "\n")


if __name__ == "__main__":
    main()
