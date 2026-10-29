#!/usr/bin/env python3
"""Exercise installed JS compilers, Wisp source builds and generated consumers."""
from __future__ import annotations

import argparse
import base64
import hashlib
import io
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys
import tarfile
import urllib.request

HERE = Path(__file__).resolve().parent
ROOT = HERE.parents[1]
sys.path.insert(0, str(ROOT / "tests/library-chain"))
from registry import Registries
sys.path.insert(0, str(ROOT / "tests/source-build"))
from snapshot import freeze_cli, verify_cli, worker_digest


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def write(path, text):
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(text)


def unpack(payload, target):
    with tarfile.open(fileobj=io.BytesIO(payload)) as archive:
        for member in archive:
            parts = Path(member.name).parts
            if member.name.startswith("/") or ".." in parts or not parts:
                raise ValueError("unsafe upstream archive path")
            if member.isdir():
                continue
            if not member.isfile() or member.size > 32 * 1024**2:
                raise ValueError("unsupported upstream archive entry")
            path = target.joinpath(*parts[1:])
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(archive.extractfile(member).read())


def fetch(url, expected, algorithm="sha256"):
    if not url.startswith("https://"):
        raise ValueError("recipe archives require HTTPS")
    with urllib.request.urlopen(url, timeout=120) as response:
        payload = response.read(32 * 1024**2 + 1)
    if len(payload) > 32 * 1024**2 or hashlib.new(algorithm, payload).hexdigest() != expected:
        raise ValueError("recipe archive checksum mismatch: " + url)
    return payload


def prepare_recipes(output):
    for name in ("yaml", "markdown-it"):
        fixture = HERE / "recipes" / name
        spec = json.loads((fixture / "source.json").read_text())
        project = output / name
        project.mkdir(parents=True)
        shutil.copyfile(fixture / "entry.ts", project / "entry.ts")
        shutil.copyfile(fixture / "world.wit", project / "world.wit")
        if name == "yaml":
            unpack(fetch(spec["url"], spec["sha256"]), project / "upstream")
            for shim in ("browser-buffer.ts", "browser-process.ts"):
                shutil.copyfile(fixture / shim, project / "upstream/src" / shim)
            subprocess.run(["patch", "--batch", "--fuzz=0", "-p1", "-i",
                            str(fixture / "browser-target.patch")], cwd=project / "upstream", check=True)
        else:
            lock_path = fixture / "package-lock.json"
            if digest(lock_path) != spec["lockSha256"]:
                raise ValueError("markdown-it lock changed")
            lock = json.loads(lock_path.read_text())
            for path, package in lock["packages"].items():
                if not path:
                    continue
                if not path.startswith("node_modules/") or ".." in Path(path).parts:
                    raise ValueError("invalid offline npm dependency path")
                algorithm, expected = package["integrity"].split("-", 1)
                if algorithm != "sha512":
                    raise ValueError("npm recipe requires pinned sha512 archives")
                unpack(fetch(package["resolved"], base64.b64decode(expected).hex(), algorithm), project / path)


def recipe_rust(name):
    if name == "yaml":
        checks = '''
    for i in 0..100 {
        let text = format!("i: {i}\\ntext: 世界");
        let value: serde_json::Value = serde_json::from_str(&api.parse_yaml(text)?).unwrap();
        assert_eq!(value["value"]["i"], i);
        assert_eq!(value["value"]["text"], "世界");
    }
    let bad: serde_json::Value = serde_json::from_str(&api.parse_yaml("a: [".into())?).unwrap();
    assert_eq!(bad["ok"], false);
    let good: serde_json::Value = serde_json::from_str(&api.parse_yaml("answer: 42".into())?).unwrap();
    assert_eq!(good["value"]["answer"], 42);
'''
    else:
        checks = '''
    assert_eq!(api.render("# Hello\\n".into())?, "<h1>Hello</h1>\\n");
    assert_eq!(api.render_inline("<script>".into())?, "&lt;script&gt;");
    for i in 0..100 {
        assert_eq!(api.render_inline(format!("**世界-{i}**"))?, format!("<strong>世界-{i}</strong>"));
    }
'''
    return ('fn main() -> Result<(), Box<dyn std::error::Error>> {\n'
            f'    let library = wisp_wisp__{name.replace("-", "_")}::Library::new()?;\n'
            '    let mut api = library.world();\n' + checks + '    Ok(())\n}\n')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--bundles", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--deny-network", action="store_true", help="run source builds under sudo unshare -n on hosted Linux")
    args = parser.parse_args()
    cli, bundles, out = args.cli.resolve(), args.bundles.resolve(), args.out.resolve()
    out.mkdir(parents=True, exist_ok=False)
    env = dict(os.environ, PYTHONDONTWRITEBYTECODE="1", CARGO_BUILD_JOBS="1", RAYON_NUM_THREADS="1")
    frozen = out / "wisp"
    freeze_cli(cli, frozen, allow_link=False, environment=env)
    cli = frozen
    commands = []
    report = {"passed": False, "complete": False, "sourceCommit": subprocess.check_output(
        ["git", "rev-parse", "HEAD"], text=True).strip(), "cliSha256": digest(cli),
        "workerSha256": worker_digest(cli), "cases": []}

    def save():
        write(out / "result.json", json.dumps(report, indent=2) + "\n")
        write(out / "commands.json", json.dumps(commands, indent=2) + "\n")

    def run(argv, cwd=ROOT, environment=None, success=True):
        argv = list(map(str, argv))
        print("+ " + shlex.join(argv), flush=True)
        result = subprocess.run(argv, cwd=cwd, env=environment or env, capture_output=True, text=True, timeout=1800)
        commands.append({"argv": argv, "cwd": str(cwd), "exitCode": result.returncode,
                         "stdout": result.stdout[-4000:], "stderr": result.stderr[-12000:]})
        save()
        if (result.returncode == 0) != success:
            raise RuntimeError(result.stdout[-4000:] + result.stderr[-12000:])
        return result.stdout.strip()

    try:
        prefix = out / "compiler-install"
        for name in ("wasm-tools", "tsrs", "wizer", "javascript"):
            bundle = bundles / (name + ".wisp.zip")
            run([cli, "toolchain", "install", bundle, "--prefix", prefix, "--sha256", digest(bundle)])
        for line in run([cli, "toolchain", "env", "--prefix", prefix]).splitlines():
            assignment = shlex.split(line.removeprefix("export "))
            if len(assignment) != 1:
                raise ValueError("invalid installed compiler environment")
            key, value = assignment[0].split("=", 1)
            env[key] = value
        for key in ("WISP_JAVASCRIPT_TOOLCHAIN", "WISP_TSRS_TOOLCHAIN", "WISP_WIZER_TOOLCHAIN", "WISP_WASM_TOOLS_TOOLCHAIN"):
            if not Path(env[key]).is_relative_to(prefix):
                raise ValueError("compiler escaped the clean installation: " + key)
        report["installedPackages"] = {name: digest(bundles / (name + ".wisp.zip"))
                                        for name in ("javascript", "tsrs", "wizer", "wasm-tools")}
        prepare_recipes(out / "recipe-inputs")
        guards = out / "guards"
        guards.mkdir()
        attempts = out / "native-attempts.log"
        for name in ("node", "npm", "npx", "cargo", "rustc", "go", "esbuild", "jco", "componentize-qjs", "wasm-tools"):
            path = guards / name
            path.write_text("#!/bin/sh\nprintf '%s\\n' " + shlex.quote(name) + " >> " + shlex.quote(str(attempts)) + "\nexit 97\n")
            path.chmod(0o755)
        source_env = {**env, "PATH": str(guards) + os.pathsep + env["PATH"]}

        def build(project, destination, success=True):
            argv = [cli, "build", project, "--component-only", "--out", destination]
            if args.deny_network:
                argv = ["sudo", "-E", "unshare", "-n", "--", "env", "PATH=" + source_env["PATH"], *argv]
            result = run(argv, environment=source_env, success=success)
            if not success:
                assert not destination.exists(), "failed source build published output"
            return result

        for engine in ("quickjs", "starlingmonkey"):
            inferred = out / engine / "inferred"
            write(inferred / "project/main.ts", "export function echo(values: number[]): Array<number> { return values; }\n")
            write(inferred / "project/wisp.toml", '[package]\nname="wisp:typescript-inferred"\nversion="0.1.0"\n'
                  f'[build]\ncompiler="typescript"\nsources=["main.ts"]\ntypecheck=true\njavascript-engine="{engine}"\n')
            run([cli, "build", inferred / "project", "--out", inferred / "release"])
            run([cli, "library", "validate", inferred / "release"])
            result = run([env["WISP_WASMTIME"], "run", "--invoke", "echo([1,2])", inferred / "release/component.wasm"])
            assert json.loads(result) == [1, 2], result
            report.setdefault("inferredApis", {})[engine] = {"passed": True, "result": result}
            for name in ("typescript-catalog", "yaml", "markdown-it"):
                work = out / engine / name
                work.mkdir(parents=True)
                project = work / "project"
                if name == "typescript-catalog":
                    shutil.copytree(ROOT / "examples/build-typescript-catalog", project)
                    config = (project / "wisp.toml").read_text().replace('"starlingmonkey"', json.dumps(engine))
                    (project / "wisp.toml").write_text(config)
                    version = "0.1.0"
                    node_source = (HERE / "consumer.mjs").read_text()
                    native_source = (HERE / "native.rs").read_text()
                else:
                    shutil.copytree(out / "recipe-inputs" / name, project)
                    spec = json.loads((HERE / "recipes" / name / "source.json").read_text())
                    version = spec["version"]
                    world = "yaml" if name == "yaml" else "markdown"
                    write(project / "wisp.toml", f'[package]\nname="wisp:{name}"\nversion="{version}"\n'
                        f'[build]\ncompiler="typescript"\nproject="."\nsources=["entry.ts"]\nwit="world.wit"\nworld="{world}"\njavascript-engine="{engine}"\n')
                    node_source = (HERE / "recipes" / name / "check-library.mjs").read_text().replace(
                        "'./library/node/index.js'", repr("@wisp/" + name))
                    native_source = recipe_rust(name)
                author, release = work / "author", work / "release"
                build(project, author)
                receipt = json.loads((author / "build/build.json").read_text())
                adapter = next(command["adapterReceipt"] for command in receipt["units"]["application"]["commands"] if "adapterReceipt" in command)
                assert adapter["engine"]["name"] == engine
                assert adapter["sourceCompilerWasmHosted"] is True
                assert adapter["fullyWasmHostedBuild"] is (engine == "quickjs")
                if name == "typescript-catalog":
                    assert adapter["typecheck"]["passed"] is True
                run([cli, "library", "pack", author, "--out", release])
                run([cli, "library", "validate", release])
                metadata = json.loads(run(["cargo", "metadata", "--no-deps", "--format-version", "1", "--manifest-path", release / "rust/Cargo.toml"]))
                cargo_name = "wisp-wisp--" + name
                native, node = work / "native", work / "node"
                write(native / "src/main.rs", native_source)
                write(native / "Cargo.toml", f'[package]\nname="js-acceptance-{name}"\nversion="0.0.0"\nedition="2024"\n[workspace]\n[dependencies]\n{cargo_name}={{version="={version}",registry="wisp-test"}}\nserde_json="=1.0.151"\n')
                write(node / "package.json", '{"name":"javascript-acceptance","private":true,"type":"module"}')
                write(node / "consumer.mjs", node_source)
                with Registries(release, metadata) as registry:
                    write(native / ".cargo/config.toml", f'[registries.wisp-test]\nindex="sparse+{registry.url}/cargo/"\n')
                    run(["cargo", "fetch"], cwd=native)
                    run(["npm", "install", "--ignore-scripts", "--no-audit", "--no-fund", "--registry", registry.url + "/npm/",
                         "@wisp/" + name + "@" + version], cwd=node)
                    requests = list(registry.requests)
                native_result = run(["cargo", "run", "--locked", "--offline", "-j1"], cwd=native)
                node_result = run(["node", "consumer.mjs"], cwd=node)
                if name == "typescript-catalog":
                    changed = project / "src/state.ts"
                    changed.write_text(changed.read_text().replace("hello ", "changed "))
                    build(project, work / "changed")
                    changed_component = work / "changed/component.wasm"
                    assert digest(changed_component) != digest(author / "component.wasm")
                    assert 'changed world' in run([env["WISP_WASMTIME"], "run", "--invoke", 'greet("world")', changed_component])
                    write(project / "src/broken.ts", 'const invalid: number = "wrong";\n')
                    build(project, work / "invalid", success=False)
                report["cases"].append({"engine": engine, "name": name, "passed": True,
                    "componentSha256": digest(author / "component.wasm"), "native": native_result,
                    "node": node_result, "registryRequests": requests, "adapter": adapter})
                save()
        assert not attempts.exists(), attempts.read_text()
        verify_cli(cli, report["cliSha256"], report["workerSha256"], environment=env)
        report.update(passed=True, complete=True, networkDeniedDuringSourceBuild=args.deny_network)
    finally:
        save()


if __name__ == "__main__":
    main()
