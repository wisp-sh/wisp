#!/usr/bin/env python3
"""Check Bend 2 in Wasm, emit JavaScript, then compile a WIT Component in Wasm."""
from __future__ import annotations

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

from bootstrap import ROOT, digest, javascript_package


def integrity():
    manifest = json.loads((ROOT / "dist/manifest.json").read_text())
    if manifest["upstream"] != json.loads((ROOT / "upstream.lock.json").read_text()):
        raise ValueError("Bend distribution differs from upstream.lock.json")
    for directory, values in [(ROOT, manifest["recipe"]), (ROOT / "dist", manifest["files"])]:
        for name, expected in values.items():
            path = directory / name
            if path.is_symlink() or not path.resolve().is_relative_to(directory) or digest(path) != expected:
                raise ValueError(f"Bend compiler input changed: {name}; rebuild with bootstrap.py")
    if "compiler.wasm" not in manifest["files"]:
        raise ValueError("Bend distribution is missing compiler.wasm")
    return manifest


def compile_request(request):
    bindings = os.environ.get("WISP_WASMTIME_PYTHON")
    if bindings:
        sys.path.insert(0, bindings)
    try:
        import wasmtime
        from wasmtime.component import Component, Linker
    except ImportError as error:
        raise ValueError("Bend requires Wasmtime Python Component bindings; set WISP_WASMTIME_PYTHON") from error
    config = wasmtime.Config()
    config.cranelift_opt_level = "none"
    config.parallel_compilation = False
    config.max_wasm_stack = 1024 * 1024
    engine = wasmtime.Engine(config)
    component = Component.from_file(engine, str(ROOT / "dist/compiler.wasm"))
    store = wasmtime.Store(engine)
    linker = Linker(engine)
    instance = linker.instantiate(store, component)
    function = instance.get_func(store, "compile")
    response = function(store, json.dumps(request))
    function.post_return(store)
    result = json.loads(response)
    if "error" in result:
        raise ValueError(result["error"])
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("source", type=Path)
    parser.add_argument("--project-root", type=Path)
    parser.add_argument("--wit", type=Path, required=True)
    parser.add_argument("--world", required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    manifest = integrity()
    package = javascript_package()
    source = args.source.resolve(strict=True)
    project = (args.project_root or source.parent).resolve(strict=True)
    if source.suffix != ".bend" or not source.is_file() or not source.is_relative_to(project):
        raise ValueError("Bend source must be a .bend file inside --project-root")
    output = args.output.absolute()
    if any(path.exists() or path.is_symlink() for path in [output, Path(str(output) + ".build.json"), Path(str(output) + ".wit")]):
        raise ValueError("output already exists; choose a new output path")
    inputs, hashes = {}, {}
    size = 0
    for path in sorted(project.rglob("*.bend")):
        if path.is_symlink() or not path.resolve().is_relative_to(project):
            raise ValueError(f"Bend source symlinks are unsupported: {path}")
        payload = path.read_bytes()
        size += len(payload)
        if size > 32 * 1024 * 1024:
            raise ValueError("Bend source tree exceeds 32 MiB")
        name = path.relative_to(project).as_posix()
        inputs["/project/" + name] = payload.decode("utf-8")
        hashes[name] = digest(path)
    checked = subprocess.run([sys.executable, str(ROOT / "compile.py"), "--compiler-request"],
                             input=json.dumps({"entry": source.relative_to(project).as_posix(), "files": inputs}),
                             text=True, capture_output=True, timeout=600)
    if checked.returncode:
        raise ValueError("Bend checker/compiler failed: " + checked.stderr.strip())
    result = json.loads(checked.stdout)
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".bend-", dir=output.parent) as directory:
        work = Path(directory)
        js = work / "source.js"
        js.write_text(result["javascript"])
        candidate = work / "output.wasm"
        subprocess.run([sys.executable, str(package / "compile.py"), str(js), "--module-root", str(work),
                        "--wit", str(args.wit.resolve(strict=True)), "--world", args.world,
                        "--output", str(candidate)], check=True)
        receipt_path = Path(str(candidate) + ".build.json")
        receipt = json.loads(receipt_path.read_text())
        receipt.update(toolchain="bend", bend={"commit": manifest["upstream"]["commit"],
                       "compilerSha256": manifest["files"]["compiler.wasm"], "inputs": hashes,
                       "exports": result["exports"], "backend": "javascript", "parallelExecution": False})
        receipt["output"]["path"] = str(output)
        receipt_path.write_text(json.dumps(receipt, indent=2) + "\n")
        published = []
        try:
            # Publish complete sidecars first, then the Component as commit marker.
            for suffix in [".wit", ".build.json", ""]:
                target = Path(str(output) + suffix)
                os.link(Path(str(candidate) + suffix), target)
                published.append(target)
        except OSError:
            for path in published:
                path.unlink()
            raise
    return 0


if __name__ == "__main__":
    try:
        if sys.argv[1:] == ["--compiler-request"]:
            print(json.dumps(compile_request(json.load(sys.stdin))))
        else:
            sys.exit(main())
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"bend compile: {error}", file=sys.stderr)
        sys.exit(1)
