#!/usr/bin/env python3
"""Publish local fixtures, install their SDKs, and execute all Rust import flows."""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
registry_spec = importlib.util.spec_from_file_location(
    "wisp_example_registry", ROOT / "tests/library-publication/registry.py"
)
registry_module = importlib.util.module_from_spec(registry_spec)
registry_spec.loader.exec_module(registry_module)
snapshot_spec = importlib.util.spec_from_file_location("source_build_snapshot", ROOT / "tests/source-build/snapshot.py")
snapshot_module = importlib.util.module_from_spec(snapshot_spec)
snapshot_spec.loader.exec_module(snapshot_module)


def refresh_local_go_fixture_sums(project, workspace, module, version):
    """Replace this disposable registry's fixture while retaining other pins."""
    project, workspace = Path(project).resolve(), Path(workspace).resolve()
    assert project.is_relative_to(workspace), "Only a fresh verification copy may refresh fixture sums"
    sums = project / "go.sum"
    if not sums.exists():
        return {"module": module, "version": version, "removed": [], "beforeSha256": None}
    before = sums.read_bytes()
    kept, removed = [], []
    for line in before.splitlines(keepends=True):
        fields = line.split()
        if len(fields) == 3 and fields[0] == module.encode() and fields[1] in (
                version.encode(), (version + "/go.mod").encode()):
            removed.append(line.decode().rstrip("\r\n"))
        else:
            kept.append(line)
    retained = b"".join(kept)
    sums.write_bytes(retained)
    return {"module": module, "version": version, "removed": removed,
            "beforeSha256": hashlib.sha256(before).hexdigest(),
            "retainedDependencySumsSha256": hashlib.sha256(retained).hexdigest()}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rust-release", type=Path, required=True,
                        help="demo:rust-math release with --go-module example.org/rust-arithmetic")
    parser.add_argument("--go-release", type=Path, required=True,
                        help="admitted demo:go-arithmetic@0.1.0 release")
    parser.add_argument("--cpp-release", type=Path, required=True,
                        help="admitted demo:cpp-math@0.1.0 release")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--cli", type=Path, default=ROOT / "target/debug/wisp")
    parser.add_argument("--clangd", type=Path,
                        default=os.environ.get("WISP_CLANGD") or shutil.which("clangd"),
                        help="check the generated C++ API in place and after relocation")
    args = parser.parse_args()
    output = args.out.resolve()
    output.mkdir(parents=True, exist_ok=False)
    cli = output / "wisp"
    snapshot_module.freeze_cli(args.cli.resolve(), cli)
    env = dict(os.environ, WISP_EXAMPLE_REGISTRY_TOKEN="test-token", GOWORK="off")
    for name in ("WISP_WASMTIME", "WISP_RUST", "WISP_RUST_WASMTIME"):
        assert env.get(name), f"Configure {name} before running this verification"
    go = env.get("WISP_GO") or shutil.which("go")
    cargo = env.get("WISP_CARGO") or shutil.which("cargo")
    rustc = env.get("WISP_NATIVE_RUSTC") or shutil.which("rustc")
    assert go and cargo and rustc, "Go installs modules; Cargo/rustc also check ordinary editor imports"
    assert args.clangd, "Set WISP_CLANGD or pass --clangd to check the ordinary C++ include"
    clangd = shutil.which(str(args.clangd)) or str(args.clangd.resolve())
    cache = Path(env.get("WISP_BUILD_CACHE", ROOT / ".tools/build/runtime-cache"))
    commands = []

    def run(argv, *, cwd=ROOT, environment=None, success=True):
        argv = list(map(str, argv))
        print("+ " + " ".join(argv), flush=True)
        process = subprocess.run(argv, cwd=cwd, env=environment or env,
                                 text=True, capture_output=True)
        commands.append({"argv": argv, "status": process.returncode,
                         "stdout": process.stdout, "stderr": process.stderr})
        (output / "commands.json").write_text(json.dumps(commands, indent=2) + "\n")
        assert (process.returncode == 0) == success, process.stdout + process.stderr
        return process

    releases = {
        "rust": args.rust_release.resolve(),
        "go": args.go_release.resolve(),
        "cpp": args.cpp_release.resolve(),
    }
    records = {name: json.loads((path / "library-release.json").read_text())
               for name, path in releases.items()}
    for name, identity in [("rust", "demo:rust-math"), ("go", "demo:go-arithmetic"),
                           ("cpp", "demo:cpp-math")]:
        assert records[name]["identity"] == identity and records[name]["version"] == "0.1.0"
        exports = records[name]["contract"]["exports"]
        assert len(exports) == 1 and exports[0]["name"] == "api", (
            f"{name} producer must use the inferred api interface; rebuild its documented example")
        assert {function["name"] for function in exports[0]["functions"]} == {"add", "negate", "widen"}
    assert records["rust"]["packages"]["go"]["name"] == "example.org/rust-arithmetic"
    assert records["rust"]["packages"]["go"]["platform"] == "native_wasm"
    proxy = output / "go-proxy"
    fetched = {}
    with registry_module.Registry() as registry:
        for name, release in releases.items():
            record = records[name]
            config = {
                "cargo": {"index": registry.url + "/cargo/", "api": registry.url,
                          "package": record["packages"]["rust"]["name"],
                          "tokenEnv": "WISP_EXAMPLE_REGISTRY_TOKEN"},
                "npm": {"registry": registry.url + "/npm/",
                        "package": record["packages"]["node"]["name"], "access": "public",
                        "tokenEnv": "WISP_EXAMPLE_REGISTRY_TOKEN"},
                "component": {"registry": registry.url, "repository": "demo/" + name + "-interop",
                              "tokenEnv": "WISP_EXAMPLE_REGISTRY_TOKEN"},
            }
            if name == "rust":
                config["go"] = {"directory": str(proxy), "module": "example.org/rust-arithmetic"}
            config_path = output / f"{name}-publisher.json"
            config_path.write_text(json.dumps(config, indent=2) + "\n")
            plan, state = output / f"{name}-publication.json", output / f"{name}-state.json"
            run([cli, "library", "prepare", release, "--config", config_path, "--out", plan])
            run([cli, "library", "publish", plan, "--state", state])
            assert json.loads(state.read_text())["complete"]
            manifest_digest = json.loads(plan.read_text())["component"]["digest"]
            fetched[name] = output / f"installed-{name}"
            run([cli, "library", "fetch", config_path, "--digest", manifest_digest,
                 "--out", fetched[name]])
            assert (fetched[name] / "component.wasm").read_bytes() == (release / "component.wasm").read_bytes()
    # The registry servers are stopped. Install the remaining language packages
    # locally; subsequent compiler builds use only these installed artifacts.
    consumers = {}
    for name, example in [("go", "go-import-rust"), ("cpp", "cpp-import-rust"),
                          ("rust", "rust-import-wasm")]:
        consumer = output / f"{name}-source"
        shutil.copytree(ROOT / "examples" / example, consumer,
                        ignore=shutil.ignore_patterns("vendor", "deps", "bindings", "proxy",
                                                     "target", ".wisp-cache", "__pycache__"))
        consumers[name] = consumer
    assert not list(consumers["go"].rglob("*.rs"))
    assert not list(consumers["cpp"].rglob("*.rs"))
    shutil.copytree(fetched["rust"], consumers["cpp"] / "deps/arithmetic")
    for language in ("go", "cpp"):
        run([cli, "library", "rust-guest", fetched[language], "--out",
             consumers["rust"] / f"rust/deps/{language}_arithmetic"])
    rust_project = consumers["rust"] / "rust"
    run([cargo, "generate-lockfile", "--offline", "--manifest-path", rust_project / "Cargo.toml"])
    run([cargo, "check", "--offline", "--locked", "--manifest-path", rust_project / "Cargo.toml"])

    go_environment = dict(env, GOPROXY=proxy.as_uri() + ",https://proxy.golang.org",
                          GONOSUMDB=",".join(filter(None, (env.get("GONOSUMDB"), "example.org/rust-arithmetic"))),
                          GOTOOLCHAIN="local", GOOS="wasip1", GOARCH="wasm",
                          CGO_ENABLED="0", GOPATH=str(output / "gopath"),
                          GOMODCACHE=str(output / "module-cache"))
    go_project = consumers["go"] / "go"
    # Each run creates a new local registry. Rebuilding its fixed fixture version
    # changes the Component archive, so refresh only that fixture's copied pins.
    # Public dependency checksums and the checked-in example remain unchanged.
    fixture_sums = refresh_local_go_fixture_sums(
        go_project, output, "example.org/rust-arithmetic", "v0.1.0")
    fixture_sums_path = output / "go-fixture-sums.json"
    fixture_sums_path.write_text(json.dumps(fixture_sums, indent=2) + "\n")
    run([go, "mod", "download", "example.org/rust-arithmetic@v0.1.0"],
        cwd=go_project, environment=go_environment)
    run([go, "mod", "tidy"], cwd=go_project, environment=go_environment)
    run([go, "mod", "vendor"], cwd=go_project, environment=go_environment)
    fixture_sums["installedGoSumSha256"] = hashlib.sha256((go_project / "go.sum").read_bytes()).hexdigest()
    fixture_sums["installedFixtureSums"] = [
        line for line in (go_project / "go.sum").read_text().splitlines()
        if line.split()[:1] == ["example.org/rust-arithmetic"]]
    fixture_sums_path.write_text(json.dumps(fixture_sums, indent=2) + "\n")
    assert "replace " not in (go_project / "go.mod").read_text()
    shutil.copytree(proxy, output / "published-go-proxy")
    shutil.rmtree(proxy)
    run([go, "clean", "-modcache"], cwd=go_project, environment=go_environment)
    go_environment["GOPROXY"] = "off"
    native_environment = dict(go_environment, CGO_ENABLED="1")
    native_environment["GOOS"] = run([go, "env", "GOHOSTOS"]).stdout.strip()
    native_environment["GOARCH"] = run([go, "env", "GOHOSTARCH"]).stdout.strip()
    assert run([go, "run", "-mod=vendor", "."], cwd=go_project,
               environment=native_environment).stdout.strip() == "42"

    # Cargo metadata can query the host compiler. Reject all host source
    # compilation while allowing its version/target discovery queries.
    native_queries = output / "native-rust-queries.jsonl"
    query_shim = output / "rustc-query-only"
    query_shim.write_text("#!" + sys.executable + "\n" +
        "import json,os,sys\n" +
        "args=sys.argv[1:]\n" +
        "with open(os.environ['WISP_NATIVE_QUERY_LOG'],'a') as f:f.write(json.dumps(args)+'\\n')\n" +
        "query=any(x in ('-V','-vV','--version') or x.startswith('--print') for x in args)\n" +
        "if not query or any(x.endswith('.rs') or x in ('--emit','-o') or x.startswith('--emit=') for x in args):\n" +
        " sys.exit('Native Rust source compilation is forbidden in this test')\n" +
        "os.execv(os.environ['WISP_REAL_NATIVE_RUSTC'],[os.environ['WISP_REAL_NATIVE_RUSTC']]+args)\n")
    query_shim.chmod(0o755)
    results = {}
    receipts = {}
    expressions = [("doubled-sum(20,1)", "42"), ("negate(true)", "false"),
                   ("widen(9007199254740993)", "9007199254740994")]
    for language in ("go", "cpp", "rust"):
        environment = dict(go_environment if language == "go" else env)
        if language != "rust":
            environment.update(WISP_RUST="/rust-compiler-must-not-be-used",
                               WISP_RUST_WASMTIME="/rust-runtime-must-not-be-used")
        else:
            environment.update(WISP_GO_TOOLCHAIN="/go-must-not-be-used", WISP_CLANG="/clang-must-not-be-used",
                               RUSTC=str(query_shim), WISP_NATIVE_QUERY_LOG=str(native_queries),
                               WISP_REAL_NATIVE_RUSTC=rustc)
        built = output / f"{language}-consumer"
        run([cli, "build", consumers[language], "--component-only", "--out", built,
             "--cache", cache], environment=environment)
        receipt = json.loads((built / "build/build.json").read_text())
        assert receipt["contractMode"] == "inferred"
        providers = [unit for unit in receipt["units"].values() if unit["compiler"] == "component"]
        assert len(providers) == (2 if language == "rust" else 1)
        assert all(unit["commands"] == [] for unit in providers)
        assert not (consumers[language] / "wit").exists()
        receipts[language] = receipt
        if language == "rust":
            compiler_commands = [command for unit in receipt["units"].values()
                                 for command in unit["commands"]
                                 if command.get("kind") == "compiler"]
            assert compiler_commands, "The Rust build recorded no compiler invocations"
            checked_compilers = {}
            for command in compiler_commands:
                compiler = Path(command["program"])
                if compiler not in checked_compilers:
                    compiler_bytes = compiler.read_bytes()
                    assert compiler_bytes.startswith(b"\0asm\x01\0\0\0"), compiler
                    checked_compilers[compiler] = hashlib.sha256(compiler_bytes).hexdigest()
                assert checked_compilers[compiler] == receipt["tools"]["rust"]["compilerSha256"]
                assert command["sha256"] == checked_compilers[compiler]
                assert Path(command["argv"][0]).resolve() == Path(env["WISP_RUST_WASMTIME"]).resolve()
                assert str(compiler) in command["argv"] and command["exitCode"] == 0
        if language == "cpp":
            header = built / "build/units/application/bindings/arithmetic.hpp"
            inspection = consumers["cpp"] / "bindings/cpp/arithmetic.hpp"
            inspection.parent.mkdir(parents=True)
            shutil.copy2(header, inspection)
            relocated = output / "relocated-cpp-editor"
            for relative in ("cpp/main.cpp", "compile_flags.txt", "bindings/cpp/arithmetic.hpp"):
                destination = relocated / relative
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(consumers["cpp"] / relative, destination)
            for source in (consumers["cpp"] / "cpp/main.cpp", relocated / "cpp/main.cpp"):
                checked = run([clangd, "--enable-config", "--tweaks=none", "--check=" + str(source)])
                assert "All checks completed, 0 errors" in checked.stderr, checked.stderr
            with (relocated / "cpp/main.cpp").open("a") as source:
                source.write('\nunsigned invalid_call() { return arithmetic::add("wrong", 1); }\n')
            rejected = run([clangd, "--enable-config", "--tweaks=none",
                            "--check=" + str(relocated / "cpp/main.cpp")], success=False)
            assert "cannot initialize" in rejected.stderr.lower() or "no matching function" in rejected.stderr.lower(), rejected.stderr
        results[language] = {}
        for expression, expected in expressions:
            actual = run([env["WISP_WASMTIME"], "run", "--invoke", expression,
                          built / "component.wasm"], environment=environment).stdout.strip()
            assert actual == expected, (language, expression, actual)
            results[language][expression] = actual
    assert "rust" not in receipts["go"]["tools"] and "rust" not in receipts["cpp"]["tools"]
    assert "go" not in receipts["rust"]["tools"] and "clang" not in receipts["rust"]["tools"]
    changed = rust_project / "deps/go_arithmetic/src/lib.rs"
    original = changed.read_bytes()
    changed.write_bytes(original + b"\npub fn forged() {}\n")
    rejected = run([cli, "build", consumers["rust"], "--component-only", "--out", output / "tampered"],
                   success=False)
    assert "guest" in rejected.stderr.lower(), rejected.stderr
    assert not (output / "tampered").exists()
    changed.write_bytes(original)
    (output / "result.json").write_text(json.dumps({
        "passed": True, "localRegistryPublicationAndFetch": True,
        "registryStoppedBeforeConsumerBuilds": True, "goProxyAndModuleCacheRemoved": True,
        "localGoFixtureChecksumsRefreshedInFreshCopy": True,
        "ordinaryGoImport": True, "ordinaryCppInclude": True, "ordinaryCargoDependencyAliases": True,
        "rustCompilerWasmMagicAndSha256Verified": True,
        "nativeGoResult": 42, "editorCargoCheck": True, "editorCppCheck": True,
        "relocatedCppEditorCheck": True, "invalidCppArgumentRejected": True,
        "rustGuestTamperRejected": True,
        "flows": ["Go imports Rust", "C++ imports Rust", "Rust imports Go", "Rust imports C++"],
        "results": results, "cliSha256": hashlib.sha256(cli.read_bytes()).hexdigest(),
    }, indent=2) + "\n")
    print("Rust <-> Go/C++: four import flows, scalar calls and offline composition passed")


if __name__ == "__main__":
    main()
