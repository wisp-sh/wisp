#!/usr/bin/env python3
"""Build packaged language examples directly through Wisp and execute their exports."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import time


LANGUAGES = {
    "zig": ("zig/examples/add.zig", None),
    "assemblyscript": ("assemblyscript/examples/add.ts", None),
    "nim": ("nim/examples/add.nim", None),
    "javascript": ("javascript/examples/add.js", None),
    "typescript": ("javascript/examples/add.ts", None),
    "python": ("python/examples/add/main.py", None),
    "grain": ("grain/examples/math.gr", None),
    "bend": (str(Path(__file__).resolve().parents[2] / "examples/build-bend/math.bend"), None),
    "goose": ("goose/examples/add.goose", None),
    "swift": ("swift/examples/add.swift", None),
    "csharp": ("csharp/examples/library/Api/Api.csproj", "Example.Api"),
    "java": ("java/examples/Math.java", "Math"),
    "kotlin": ("kotlin/examples/Math.kt", "MathKt"),
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path, default=Path("target/debug/wisp"))
    parser.add_argument("--buildtools", type=Path, required=True)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--cache", type=Path)
    parser.add_argument("--languages", nargs="+", choices=LANGUAGES, default=list(LANGUAGES))
    parser.add_argument("--wasmtime", default=os.environ.get("WISP_WASMTIME", "wasmtime"))
    args = parser.parse_args()
    cli = args.cli.resolve(strict=True)
    tools = args.buildtools.resolve(strict=True)
    output = args.out.absolute()
    output.mkdir(parents=True, exist_ok=False)
    runtime = shutil.which(args.wasmtime) or str(Path(args.wasmtime).resolve(strict=True))
    environment = os.environ.copy()
    environment["WISP_WASMTIME"] = runtime
    for language in args.languages:
        package = "javascript" if language == "typescript" else language
        root = Path(__file__).resolve().parents[2] / "tools/bend" if language == "bend" else tools / package
        environment.setdefault("WISP_" + language.upper() + "_TOOLCHAIN", str(root))
    if "bend" in args.languages:
        environment.setdefault("WISP_JAVASCRIPT_TOOLCHAIN", str(tools / "javascript"))
    guard_dir = output / "blocked-native-tools"
    guard_dir.mkdir()
    attempts = output / "native-tool-attempts.log"
    attempts.write_text("")
    guard = guard_dir / "deny.py"
    guard.write_text(
        "#!" + sys.executable + "\n"
        "import os, sys\n"
        "from pathlib import Path\n"
        "with open(os.environ['WISP_NATIVE_TOOL_ATTEMPTS'], 'a') as log:\n"
        "    log.write(Path(sys.argv[0]).name + '\\n')\n"
        "raise SystemExit('native compiler blocked during source build')\n"
    )
    guard.chmod(0o755)
    for name in (
        "node", "nodejs", "npm", "npx", "rustc", "cargo", "go", "cc", "c++",
        "gcc", "g++", "clang", "clang++", "ld", "wasm-ld", "wasm-tools", "wac",
        "wasm-opt", "esbuild", "componentize-py", "componentize-qjs", "zig", "nim",
        "dotnet", "csc", "mcs", "mono", "ilc", "grain", "grainc", "bend", "bun", "goose", "swift", "swiftc", "java", "javac", "kotlin", "kotlinc",
    ):
        (guard_dir / name).symlink_to(guard.name)
    environment["PATH"] = str(guard_dir) + os.pathsep + environment.get("PATH", "")
    environment["WISP_NATIVE_TOOL_ATTEMPTS"] = str(attempts)
    report = {"schema": 1, "complete": False, "passed": False, "checks": []}

    def save():
        pending = output / ".verification.json.tmp"
        pending.write_text(json.dumps(report, indent=2) + "\n")
        pending.replace(output / "verification.json")

    save()
    for language in dict.fromkeys(args.languages):
        started = time.monotonic()
        work = output / language
        work.mkdir()
        source_name, entry = LANGUAGES[language]
        source = tools / source_name
        if language == "csharp":
            shutil.copytree(source.parent.parent, work / "src")
            source_relative = "Api/Api.csproj"
        else:
            shutil.copytree(source.parent, work / "src")
            source_relative = source.name
        config = (
            f'[package]\nname="test:{language}"\nversion="0.1.0"\n'
            f'[build]\ncompiler="{language}"\nsources=["src/{source_relative}"]\n'
        )
        if entry:
            config += f"entry={json.dumps(entry)}\n"
        if language == "csharp":
            config = config.replace('name="test:csharp"', 'name="example:csharp-library"').replace('version="0.1.0"', 'version="1.0.0"')
            config += 'project="src"\nwit="src/library.wit"\nworld="library"\n'
        (work / "wisp.toml").write_text(config)
        row = {"language": language, "passed": False}
        try:
            command = [cli, "build", work, "--component-only", "--out", work / "release"]
            if args.cache:
                command += ["--cache", args.cache.resolve()]
            result = subprocess.run([str(v) for v in command], env=environment, capture_output=True, text=True)
            (work / "build.log").write_text(result.stdout + result.stderr)
            if result.returncode:
                raise RuntimeError(f"build exited {result.returncode}; see {work / 'build.log'}")
            component = work / "release/component.wasm"
            receipt = json.loads((work / "release/build/build.json").read_text())
            unit = receipt["units"]["application"]
            expected = hashlib.sha256(component.read_bytes()).hexdigest()
            if unit["compiler"] != language or receipt["componentSha256"] != expected:
                raise RuntimeError("compiler identity or Component hash differs from build receipt")
            call = subprocess.run([runtime, "run", "--invoke", "add(20,22)", str(component)],
                                  env=environment, capture_output=True, text=True)
            (work / "invoke.log").write_text(call.stdout + call.stderr)
            if call.returncode or call.stdout.strip().splitlines()[-1:] != ["42"]:
                raise RuntimeError(f"unexpected invocation result; see {work / 'invoke.log'}")
            if language == "csharp":
                for expression, expected_value in [
                    ('greet("世界")', '"Hello, 世界!"'),
                    ('advance({name: "Ana", age: 41})', 'ok({name: "Ana", age: 42})'),
                    ('choose(some(high))', '[42]'),
                ]:
                    rich = subprocess.run([runtime, "run", "--invoke", expression, str(component)],
                                          env=environment, capture_output=True, text=True)
                    if rich.returncode or rich.stdout.strip() != expected_value:
                        raise RuntimeError(f"{expression}: {rich.stdout}{rich.stderr}")
                row["richInterfaceCalls"] = 3
            if attempts.read_text():
                raise RuntimeError("source build attempted a native compiler")
            row.update(passed=True, result=42, componentSha256=expected,
                       bytes=component.stat().st_size, contractMode=receipt["contractMode"])
        except Exception as error:
            row["error"] = str(error)
        row["seconds"] = round(time.monotonic() - started, 3)
        report["checks"].append(row)
        save()
        print(json.dumps(row), flush=True)
    report["complete"] = True
    report["attemptedNativeTools"] = attempts.read_text().splitlines()
    report["passed"] = all(row["passed"] for row in report["checks"]) and not report["attemptedNativeTools"]
    save()
    return 0 if report["passed"] else 1


if __name__ == "__main__":
    sys.exit(main())
