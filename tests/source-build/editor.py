#!/usr/bin/env python3
"""Check C++ examples with clangd in the checkout and a relocated source-only copy."""
import argparse
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--clangd", default=os.environ.get("WISP_CLANGD") or shutil.which("clangd"))
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    if not args.clangd:
        parser.error("set WISP_CLANGD or pass --clangd with the editor's clangd executable")
    output = args.out.resolve()
    output.mkdir(parents=True, exist_ok=False)
    results = []
    examples = ROOT / "examples"
    # Generated Go module headers belong to the native runtime, not the C++
    # application API checked with the Wasm compiler flags below.
    sources = sorted(path.relative_to(examples) for path in examples.rglob("*")
                     if "vendor" not in path.relative_to(examples).parts
                     and path.suffix in {".cpp", ".hpp", ".h"})

    def check(base, source, name, cwd, valid=True):
        # Check diagnostics and navigation; skip applying optional refactorings to generated headers.
        argv = [args.clangd, "--enable-config", "--tweaks=none", "--check=" + str(base / source)]
        result = subprocess.run(argv, cwd=cwd, text=True, capture_output=True)
        log = result.stdout + result.stderr
        (output / f"{name}.log").write_text(log)
        assert "--target=wasm32-unknown-unknown" in log, log
        if valid:
            assert result.returncode == 0 and "All checks completed, 0 errors" in log, log
        else:
            assert result.returncode != 0, "clangd accepted a string passed to a numeric API"
            assert "no matching function" in log.lower() or "cannot initialize" in log.lower(), log
        results.append({"source": str(source), "copy": name.split("-", 1)[0],
                        "expectedValid": valid, "status": result.returncode})

    with tempfile.TemporaryDirectory(prefix="wisp-editor-relocated-") as temporary:
        relocated = Path(temporary) / "checkout/examples"
        unrelated = Path(temporary) / "unrelated-working-directory"
        unrelated.mkdir()
        # Include no build output, machine-specific configuration, toolchains or caches.
        for path in examples.rglob("*"):
            if "vendor" in path.relative_to(examples).parts:
                continue
            if path.is_file() and (path.suffix in {".cpp", ".hpp", ".h"} or path.name == "compile_flags.txt"):
                destination = relocated / path.relative_to(examples)
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(path, destination)
        for index, source in enumerate(sources):
            check(examples, source, f"checkout-{index}", unrelated)
            check(relocated, source, f"relocated-{index}", unrelated)
        source = Path("cpp-import-go/cpp/main.cpp")
        with (relocated / source).open("a") as file:
            file.write('\nunsigned invalid_call() { return arithmetic::add("wrong", 1); }\n')
        check(relocated, source, "negative-types", unrelated, valid=False)
    report = {"passed": True, "files": len(sources), "relocatedCopyVerified": True,
              "invalidArgumentRejected": True, "checks": results}
    (output / "result.json").write_text(json.dumps(report, indent=2) + "\n")
    print(f"clangd: {len(sources)} C++ sources/headers pass in both locations; invalid API call rejected")


if __name__ == "__main__":
    main()
