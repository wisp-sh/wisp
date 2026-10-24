"""Install a generated Rust crate through Cargo, close the registry, and execute it."""
from __future__ import annotations

import importlib.util
import json
from pathlib import Path
import tarfile

from acquire import digest, require

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("source_build_registry", ROOT / "tests/library-publication/registry.py")
registry_module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(registry_module)


def execute(cli, release, output, assertion, run):
    release, output = Path(release), Path(output)
    output.mkdir(parents=True)
    (output / "src").mkdir()
    (output / ".cargo").mkdir()
    record = json.loads((release / "library-release.json").read_text())
    package = record["packages"]["rust"]["name"]
    version = record["version"]
    (output / "src/main.rs").write_text(
        'fn main() { eprintln!("native: instantiate source-built library");\n'
        + 'let library = sdk::Library::new().expect("instantiate source-built library");\n'
        + 'eprintln!("native: exercise source-built library");\n'
        + assertion + '\nprintln!("native-source-build-ok"); }\n')
    (output / "Cargo.toml").write_text(
        '[package]\nname="source-build-native-consumer"\nversion="0.0.0"\nedition="2024"\n'
        '[workspace]\n[dependencies]\n'
        f'sdk={{package={json.dumps(package)},version={json.dumps("=" + version)},registry="source-build"}}\n'
        '[profile.dev]\ndebug=0\n')
    # Preserve the graph shipped in the generated crate. Add only the consumer
    # and the local registry origin for the installed package.
    with tarfile.open(release / "rust.crate") as archive:
        lock_name = f"{package}-{version}/Cargo.lock"
        member = archive.getmember(lock_name)
        require(member.isfile() and member.size <= 4 * 1024**2, "generated crate has no bounded Cargo.lock")
        lock = archive.extractfile(member).read().decode()
    with registry_module.Registry() as registry:
        registry_index = "sparse+" + registry.url + "/cargo/"
        (output / ".cargo/config.toml").write_text(
            f'[registries.source-build]\nindex={json.dumps(registry_index)}\n')
        package_header = f'[[package]]\nname = "{package}"\nversion = "{version}"\n'
        require(lock.count(package_header) == 1, "generated Cargo.lock has an ambiguous library entry")
        crate_sha = digest(release / "rust.crate")
        lock = lock.replace(package_header, package_header
            + f'source = {json.dumps(registry_index)}\nchecksum = "{crate_sha}"\n')
        root_entry = '\n[[package]]\nname = "source-build-native-consumer"\nversion = "0.0.0"\ndependencies = [\n ' + json.dumps(package) + ',\n]\n'
        blocks = lock.split('\n[[package]]\n')
        insertion = next((index for index, block in enumerate(blocks[1:], 1)
                          if block.split('"', 2)[1] > 'source-build-native-consumer'), len(blocks))
        blocks.insert(insertion, root_entry.removeprefix('\n[[package]]\n'))
        lock = '\n[[package]]\n'.join(blocks)
        (output / "Cargo.lock").write_text(lock)
        config = {
            "cargo": {"index": registry.url + "/cargo/", "api": registry.url,
                      "package": package, "tokenEnv": "WISP_BUILD_TEST_TOKEN"},
            "npm": {"registry": registry.url + "/npm/", "package": record["packages"]["node"]["name"],
                    "access": "public", "tokenEnv": "WISP_BUILD_TEST_TOKEN"},
            "component": {"registry": registry.url, "repository": "demo/native-source-build", "tokenEnv": "WISP_BUILD_TEST_TOKEN"},
        }
        if "go" in record["packages"]:
            config["go"] = {"directory": str(output / "go-proxy"), "module": record["packages"]["go"]["name"]}
        config_path, plan, state = output / "publisher.json", output / "publication.json", output / "publication-state.json"
        config_path.write_text(json.dumps(config, indent=2) + "\n")
        run([cli, "library", "prepare", release, "--config", config_path, "--out", plan])
        require(not registry.requests, "preparing native publication contacted a registry")
        run([cli, "library", "publish", plan, "--state", state])
        require(json.loads(state.read_text())["complete"], "native crate publication did not complete")
        run(["cargo", "fetch", "--locked", "--manifest-path", output / "Cargo.toml"], cwd=output)
        require(("GET", f"/crates/{package}/{version}/download") in registry.requests, "Cargo did not install the source-built package from the registry")
        (output / "registry-requests.json").write_text(json.dumps(registry.requests, indent=2) + "\n")
    actual = run(["cargo", "run", "--locked", "--offline", "--manifest-path", output / "Cargo.toml"], cwd=output).stdout.strip()
    require(actual == "native-source-build-ok", f"native Rust consumer returned {actual!r}")
    result = {
        "passed": True, "package": package, "componentSha256": record["componentSha256"],
        "crateSha256": crate_sha, "cargoLockSha256": digest(output / "Cargo.lock"),
        "installedThroughCargoRegistry": True, "registryStoppedBeforeBuildAndExecution": True,
        "nativeRustExecuted": True, "result": actual,
    }
    (output / "result.json").write_text(json.dumps(result, indent=2) + "\n")
    return result
