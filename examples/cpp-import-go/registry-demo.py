#!/usr/bin/env python3
"""Publish Go locally, fetch its package, then build ordinary C++ offline."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("registry", ROOT / "tests/library-publication/registry.py")
registry_module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(registry_module)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--release", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--cli", type=Path, default=ROOT / "target/debug/wisp")
    args = parser.parse_args()
    cli, release, output = args.cli.resolve(), args.release.resolve(), args.out.resolve()
    output.mkdir(parents=True, exist_ok=False)
    frozen_cli = output / "wisp"
    shutil.copy2(cli, frozen_cli)
    cli = frozen_cli
    record = json.loads((release / "library-release.json").read_text())
    assert record["identity"] == "demo:go-arithmetic" and record["version"] == "0.1.0"
    consumer = output / "consumer-source"
    shutil.copytree(ROOT / "examples/cpp-import-go", consumer,
                    ignore=shutil.ignore_patterns("go", "deps", "bindings", ".wisp-cache", "__pycache__"))
    assert not list(consumer.rglob("*.go")) and not list(consumer.rglob("*.wasm"))
    assert not list(consumer.rglob("*.wit"))
    commands = []

    def run(argv, *, success=True):
        argv = list(map(str, argv))
        print("+ " + " ".join(argv), flush=True)
        result = subprocess.run(argv, cwd=ROOT, env=env, text=True, capture_output=True)
        commands.append({"argv": argv, "status": result.returncode, "stdout": result.stdout, "stderr": result.stderr})
        (output / "commands.json").write_text(json.dumps(commands, indent=2) + "\n")
        assert (result.returncode == 0) == success, result.stdout + result.stderr
        return result

    env = dict(os.environ, WISP_BUILD_TEST_TOKEN="test-token", WISP_GO_TOOLCHAIN="/go-must-not-be-used",
               WISP_PREVIEW1_ADAPTER="/adapter-must-not-be-used")
    with registry_module.Registry() as registry:
        config = {
            "cargo": {"index": registry.url + "/cargo/", "api": registry.url, "package": record["packages"]["rust"]["name"], "tokenEnv": "WISP_BUILD_TEST_TOKEN"},
            "npm": {"registry": registry.url + "/npm/", "package": record["packages"]["node"]["name"], "access": "public", "tokenEnv": "WISP_BUILD_TEST_TOKEN"},
            "component": {"registry": registry.url, "repository": "demo/go-arithmetic", "tokenEnv": "WISP_BUILD_TEST_TOKEN"},
        }
        config_path = output / "publisher.json"
        config_path.write_text(json.dumps(config, indent=2) + "\n")
        plan, state = output / "publication.json", output / "publication-state.json"
        run([cli, "library", "prepare", release, "--config", config_path, "--out", plan])
        assert not registry.requests, "prepare contacted a registry"
        run([cli, "library", "publish", plan, "--state", state])
        assert json.loads(state.read_text())["complete"]
        digest = json.loads(plan.read_text())["component"]["digest"]
        dependency = consumer / "deps/arithmetic"
        run([cli, "library", "fetch", config_path, "--digest", digest, "--out", dependency])
        for name in ["component.wasm", "library-release.json", "rust.crate", "node.tgz"]:
            assert (dependency / name).read_bytes() == (release / name).read_bytes(), name
    # All registries have shut down. Only the installed package and C++ source remain.
    result = output / "consumer"
    run([cli, "build", consumer, "--component-only", "--out", result,
         "--cache", ROOT / ".tools/build/runtime-cache"])
    receipt = json.loads((result / "build/build.json").read_text())
    assert receipt["contractMode"] == "inferred"
    assert "go" not in receipt["tools"] and "preview1Adapter" not in receipt["tools"]
    provider = [unit for unit in receipt["units"].values() if unit["compiler"] == "component"]
    assert len(provider) == 1 and provider[0]["commands"] == []
    assert provider[0]["componentSha256"] == record["componentSha256"]
    for expression, expected in [("doubled-sum(20,1)", "42"), ("negate(true)", "false"),
                                 ("widen(9007199254740993)", "9007199254740994")]:
        value = run([env["WISP_WASMTIME"], "run", "--invoke", expression, result / "component.wasm"]).stdout.strip()
        assert value == expected, (expression, value)
    assert (result / "notices/Go-LICENSE.txt").read_text() == record["metadata"]["notices"]["Go-LICENSE.txt"]
    component = dependency / "component.wasm"
    original = component.read_bytes()
    component.write_bytes(original + b"changed")
    rejected = run([cli, "build", consumer, "--component-only", "--out", output / "tampered"], success=False)
    assert "library Component digest mismatch" in rejected.stderr
    assert not (output / "tampered").exists()
    component.write_bytes(original)
    config_path = consumer / "wisp.toml"
    config_path.write_text(config_path.read_text().replace('package = "demo:go-arithmetic"\nversion = "0.1.0"',
                                                         'package = "demo:go-arithmetic"\nversion = "0.1.1"'))
    rejected = run([cli, "build", consumer, "--component-only", "--out", output / "wrong-version"], success=False)
    assert "identity/version differs" in rejected.stderr
    assert not (output / "wrong-version").exists()
    (output / "result.json").write_text(json.dumps({"passed": True, "registryPublished": True,
        "installedByWispFetch": True, "registriesStoppedBeforeBuild": True, "goSourceAbsent": True,
        "goNotUsed": True, "ordinaryCppInclude": True, "result": 42, "boolAndU64Verified": True,
        "dependencyNoticesPreserved": True, "changedComponentRejected": True, "wrongVersionRejected": True,
        "cliSha256": hashlib.sha256(cli.read_bytes()).hexdigest()}, indent=2) + "\n")
    print("published Go package -> ordinary C++ -> composed Wasm: 42")


if __name__ == "__main__":
    main()
