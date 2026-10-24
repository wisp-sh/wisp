#!/usr/bin/env python3
"""Compile Grain in Wasm and pass its Component through Wisp's normal build flow."""
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

HERE = Path(__file__).resolve().parent
REPO = HERE.parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--cli", type=Path, default=REPO / "target/debug/wisp")
    parser.add_argument("--toolchain", type=Path,
                        default=Path(os.environ.get("WISP_GRAIN_TOOLCHAIN", REPO.parent / "wisp-buildtools/grain")))
    parser.add_argument("--package", action="store_true",
                        help="also produce Wisp language packages; requires the normal packaging tools")
    args = parser.parse_args()
    output = args.out.absolute()
    if output.exists() or output.is_symlink():
        raise ValueError("output already exists; choose a new output directory")
    compiler = (args.toolchain / "compile.py").resolve(strict=True)
    cli = args.cli.resolve(strict=True)
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix=".grain-project-", dir=output.parent) as directory:
        project = Path(directory)
        shutil.copytree(HERE / "wit", project / "wit")
        shutil.copyfile(HERE / "math.gr", project / "math.gr")
        shutil.copyfile(compiler.parent / "dist/licenses/Grain-stdlib.txt",
                        project / "Grain-stdlib-LICENSE.txt")
        command = [sys.executable, str(compiler), str(project / "math.gr"),
                   "--component", "--wit", str(project / "wit"), "--world", "math",
                   "-o", str(project / "math.wasm")]
        adapter = os.environ.get("WISP_PREVIEW1_ADAPTER")
        if adapter:
            command += ["--preview1-adapter", adapter]
        subprocess.run(command, check=True)
        component = project / "math.wasm"
        sha = hashlib.sha256(component.read_bytes()).hexdigest()
        sys.path.insert(0, str(compiler.parent.parent))
        import wasm_tools
        runtime_wit = project / "runtime-wit"
        subprocess.run(wasm_tools.command("component", "wit", component,
                                         "--out-dir", runtime_wit), check=True)
        contract_path = runtime_wit / "component.wit"
        contract = contract_path.read_text()
        if not contract.startswith("package root:component;\n") or "\nworld root {\n" not in contract:
            raise ValueError("unexpected Component contract identity from the pinned Wasm tools")
        contract = contract.replace("package root:component;", "package demo:grain-math@0.1.0;", 1)
        contract = contract.replace("\nworld root {\n", "\nworld math {\n", 1)
        contract_path.write_text(contract)
        (project / "wisp.toml").write_text(
            '[package]\nname = "demo:grain-math"\nversion = "0.1.0"\n'
            '[metadata]\ndescription = "Grain scalar functions compiled in Wasm"\nlicense = "MIT"\n'
            'notice-files = ["Grain-stdlib-LICENSE.txt"]\n'
            '[build]\nroot = "application"\nwit = "runtime-wit"\n'
            '[build.units.application]\nworld = "math"\n'
            f'component = {{ path = "math.wasm", sha256 = "{sha}" }}\n'
        )
        command = [str(cli), "build", str(project), "--out", str(output)]
        if not args.package:
            command.append("--component-only")
        subprocess.run(command, check=True)
        inspection = output / "build/grain"
        inspection.mkdir(parents=True)
        for name in ["math.gr", "math.wasm.wit", "math.wasm.build.json"]:
            shutil.copyfile(project / name, inspection / name)
        report = json.loads((output / "build/build.json").read_text())
        if report["units"]["application"]["compiler"] != "component":
            raise ValueError("Wisp did not preserve the Grain Component unit")
    print(output / "component.wasm")
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except (OSError, ValueError, subprocess.SubprocessError) as error:
        print(f"build-grain: {error}", file=sys.stderr)
        sys.exit(1)
