#!/usr/bin/env python3
"""Publish a C++ package, then run the same Go imports natively and in Wasm."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile


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
    cli = args.cli.resolve()
    release = args.release.resolve()
    output = args.out.resolve()
    output.mkdir(parents=True, exist_ok=False)
    go = os.environ.get("WISP_GO") or shutil.which("go")
    assert go, "Set WISP_GO to Go 1.27.1 or put it on PATH. It installs modules; the Wasm Go toolchain compiles."
    record = json.loads((release / "library-release.json").read_text())
    assert record["packages"]["go"]["platform"] == "native_wasm"
    module = record["packages"]["go"]["name"]
    proxy = output / "go-proxy"
    commands = []

    def run(argv, *, cwd=ROOT, env=None, success=True):
        argv = list(map(str, argv))
        result = subprocess.run(argv, cwd=cwd, env=env, text=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)
        commands.append({"argv": argv, "status": result.returncode, "stdout": result.stdout, "stderr": result.stderr})
        (output / "commands.json").write_text(json.dumps(commands, indent=2) + "\n")
        assert (result.returncode == 0) == success, result.stdout + result.stderr
        return result

    assert "go1.27." in run([go, "version"]).stdout
    publisher_env = dict(os.environ, WISP_BUILD_TEST_TOKEN="test-token")
    with registry_module.Registry() as registry:
        config = {
            "cargo": {"index": registry.url + "/cargo/", "api": registry.url, "package": record["packages"]["rust"]["name"], "tokenEnv": "WISP_BUILD_TEST_TOKEN"},
            "npm": {"registry": registry.url + "/npm/", "package": record["packages"]["node"]["name"], "access": "public", "tokenEnv": "WISP_BUILD_TEST_TOKEN"},
            "component": {"registry": registry.url, "repository": "demo/cpp-arithmetic", "tokenEnv": "WISP_BUILD_TEST_TOKEN"},
            "go": {"directory": str(proxy), "module": module},
        }
        config_path = output / "publisher.json"
        config_path.write_text(json.dumps(config, indent=2) + "\n")
        plan = output / "publication.json"
        state = output / "publication-state.json"
        run([cli, "library", "prepare", release, "--config", config_path, "--out", plan], env=publisher_env)
        run([cli, "library", "publish", plan, "--state", state], env=publisher_env)
        assert json.loads(state.read_text())["complete"]
    with tempfile.TemporaryDirectory(prefix="wisp-go-module-consumer-") as temporary:
        consumer = Path(temporary)
        shutil.copytree(ROOT / "examples/go-import-wasm", consumer, dirs_exist_ok=True,
                        ignore=shutil.ignore_patterns("cpp", "vendor", "bindings", "proxy", "go.sum", ".wisp-cache", "__pycache__"))
        assert not list(consumer.rglob("*.wasm"))
        assert not list(consumer.rglob("*.cpp"))
        env = dict(os.environ, GOPROXY=proxy.as_uri() + ",https://proxy.golang.org", GOSUMDB="off", GOTOOLCHAIN="local", GOWORK="off",
                   GOOS="wasip1", GOARCH="wasm", CGO_ENABLED="0", GOPATH=str(output / "gopath"),
                   GOMODCACHE=str(output / "module-cache"))
        run([go, "mod", "download", f"{module}@v{record['version']}"], cwd=consumer / "go", env=env)
        # Resolve both target implementations with ordinary Go module tooling.
        run([go, "mod", "tidy"], cwd=consumer / "go", env=env)
        run([go, "mod", "vendor"], cwd=consumer / "go", env=env)
        assert "replace " not in (consumer / "go/go.mod").read_text()
        vendored = consumer / "go/vendor" / module
        assert (vendored / "bindings_wasip1.go").is_file()
        assert (vendored / "component.wasm").read_bytes() == (release / "component.wasm").read_bytes()
        # Installation is complete. Remove the published proxy and module cache.
        # The build now has only the Go project's ordinary vendor directory.
        shutil.rmtree(proxy)
        run([go, "clean", "-modcache"], cwd=consumer / "go", env=env)
        env.update(GOPROXY="off", WISP_CLANG="/clang-must-not-be-used")
        native_env = dict(env, CGO_ENABLED="0")
        native_env["GOOS"] = run([go, "env", "GOHOSTOS"]).stdout.strip()
        native_env["GOARCH"] = run([go, "env", "GOHOSTARCH"]).stdout.strip()
        native = run([go, "run", "-mod=vendor", "."], cwd=consumer / "go", env=native_env)
        assert native.stdout.strip() == "42", native.stdout
        result = output / "consumer"
        run([cli, "build", consumer, "--component-only", "--out", result,
             "--cache", ROOT / ".tools/build/runtime-cache"], env=env)
        receipt = json.loads((result / "build/build.json").read_text())
        assert "clang" not in receipt["tools"]
        providers = [unit for unit in receipt["units"].values() if unit["compiler"] == "component"]
        assert len(providers) == 1 and providers[0]["commands"] == []
        assert receipt["contractMode"] == "inferred"
        assert not (consumer / "wit").exists()
        assert not (consumer / "go/wisp_bindings.go").exists()
        assert any("vendor/" in p for p in receipt["units"]["application"]["inputs"])
        value = run([os.environ["WISP_WASMTIME"], "run", "--invoke", "doubled-sum(20, 1)", result / "component.wasm"], env=env).stdout.strip()
        assert value == "42", value
        # Retain inspectable installed package and Go's checksum lock.
        shutil.copytree(consumer / "go/vendor", output / "installed-vendor")
        shutil.copyfile(consumer / "go/go.sum", output / "go.sum")
        source = vendored / "bindings_wasip1.go"
        source.write_text(source.read_text() + "\nfunc Forged() {}\n")
        rejected = run([cli, "build", consumer, "--component-only", "--out", output / "tampered",
                        "--cache", ROOT / ".tools/build/runtime-cache"], env=env, success=False)
        assert "vendored Go guest package differs" in rejected.stderr
        assert not (output / "tampered").exists()
    (output / "result.json").write_text(json.dumps({"passed": True, "module": module,
        "ordinaryGoImport": True, "registryPublished": True, "installedByGoModules": True,
        "proxyAndModuleCacheRemovedBeforeBuild": True, "nativeGoCompilation": True,
        "nativeResult": 42, "wasmResult": 42, "sameApplicationSource": True,
        "result": 42, "tamperedBindingsRejected": True}, indent=2) + "\n")
    print("published Go dependency -> native Go and composed Wasm: 42")


if __name__ == "__main__":
    main()
