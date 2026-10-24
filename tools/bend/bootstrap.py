#!/usr/bin/env python3
"""Build a pinned Bend 2 checker/compiler Component with Wisp's JavaScript tools."""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import urllib.request

ROOT = Path(__file__).resolve().parent


def digest(path):
    return hashlib.sha256(Path(path).read_bytes()).hexdigest()


def javascript_package(configured=None):
    value = configured or os.environ.get("WISP_JAVASCRIPT_TOOLCHAIN")
    if not value:
        raise ValueError("set WISP_JAVASCRIPT_TOOLCHAIN to the JavaScript adapter directory")
    path = Path(value).resolve(strict=True)
    return path.parent if path.is_file() else path


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--javascript-toolchain", type=Path)
    parser.add_argument("--offline", action="store_true")
    args = parser.parse_args()
    package = javascript_package(args.javascript_toolchain)
    lock = json.loads((ROOT / "upstream.lock.json").read_text())
    cache = ROOT / "build/downloads"
    cache.mkdir(parents=True, exist_ok=True)
    for name, spec in lock["files"].items():
        target = cache / Path(name).name
        if not target.exists():
            if args.offline:
                raise ValueError(f"offline bootstrap needs {target}")
            url = f"https://raw.githubusercontent.com/bendlang/bend/{lock['commit']}/{name}"
            with urllib.request.urlopen(url, timeout=60) as response:
                data = response.read(spec["bytes"] + 1)
            if len(data) != spec["bytes"] or hashlib.sha256(data).hexdigest() != spec["sha256"]:
                raise ValueError(f"Bend download checksum mismatch: {name}")
            target.write_bytes(data)
        if target.stat().st_size != spec["bytes"] or digest(target) != spec["sha256"]:
            raise ValueError(f"Bend cached source checksum mismatch: {name}")
    destination = ROOT / "dist"
    if destination.exists():
        raise ValueError("Bend dist already exists; remove it explicitly to rebuild")
    with tempfile.TemporaryDirectory(prefix=".bend-bootstrap-", dir=ROOT / "build") as directory:
        work = Path(directory)
        source = work / "src"
        shutil.copytree(ROOT / "src", source)
        bend = (cache / "bend.ts").read_text()
        # The upstream network loader is unused. Remove its module-level Node
        # setup; parsing, checking, proof validation and JS codegen stay upstream.
        imports = '\n'.join(f'import * as {name} from "node:{name}";' for name in ["fs", "os", "path", "url"])
        start = bend.index("export const BEND_DIR =")
        end = bend.index("export const NAMED =", start)
        bend = bend[:start] + 'export const BASE_BEND = "/bend2/base.bend";\nconst BEND_LIB = "/unsupported-hub";\nexport const BEND_HUB = "";\n\n' + bend[end:]
        if bend.count(imports) != 1:
            raise ValueError("pinned Bend imports changed")
        bend = bend.replace(imports, 'import { fs } from "./host";')
        (source / "bend.ts").write_text(bend)
        comp = (cache / "comp.ts").read_text()
        if comp.count('import * as fs from "node:fs";') != 1:
            raise ValueError("pinned Bend compiler imports changed")
        comp = comp.replace('import * as fs from "node:fs";', 'import { fs } from "./host";')
        signature = "export function js_lib(book: Bend.Book, mod = false): string {"
        selection = "&& io_base(FL.book, t.T) === null;"
        if comp.count(signature) != 1 or comp.count(selection) != 1:
            raise ValueError("pinned Bend module export selection changed")
        # Select the numeric public API without changing the semantics of helpers
        # or marking user code as trusted Base definitions.
        comp = comp.replace(signature, "export function js_lib(book: Bend.Book, mod = false, roots?: string[]): string {")
        comp = comp.replace(selection, "&& io_base(FL.book, t.T) === null && (roots === undefined || roots.includes(k));")
        (source / "comp.ts").write_text(comp)
        (source / "base.ts").write_text("export default " + json.dumps((cache / "base.bend").read_text()) + ";\n")
        result = work / "dist"
        result.mkdir()
        subprocess.run([sys.executable, str(package / "compile.py"), str(source / "compiler.ts"),
                        "--module-root", str(source), "--wit", str(ROOT / "compiler.wit"),
                        "--world", "compiler", "--stub-wasi", "--output", str(result / "compiler.wasm")], check=True)
        shutil.copyfile(cache / "LICENSE", result / "Bend-LICENSE.txt")
        recipes = [ROOT / "bootstrap.py", ROOT / "compiler.wit", *sorted((ROOT / "src").glob("*.ts"))]
        manifest = {"schema": 1, "upstream": lock,
                    "recipe": {str(path.relative_to(ROOT)): digest(path) for path in recipes},
                    "files": {path.name: digest(path) for path in result.iterdir()},
                    "compilerExecution": "Bend 2 checker and JavaScript compiler inside QuickJS Wasm",
                    "fullyWasmHostedBuild": True}
        (result / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
        result.rename(destination)
    print(destination / "compiler.wasm")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"bend bootstrap: {error}", file=sys.stderr)
        sys.exit(1)
