#!/usr/bin/env python3
"""Compile inferred rich APIs and run their shared JCO value verifier."""
from __future__ import annotations

import argparse
import importlib.util
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("source_acceptance", Path(__file__).with_name("run.py"))
source_acceptance = importlib.util.module_from_spec(spec)
spec.loader.exec_module(source_acceptance)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cli", type=Path, default=ROOT / "target/debug/wisp")
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--cache", type=Path, default=ROOT / ".tools/build/runtime-cache")
    parser.add_argument("--case", action="append", choices=["cpp", "go", "rust"])
    args = parser.parse_args()
    acceptance = source_acceptance.Acceptance(args.cli, args.out.resolve())
    verifier = ROOT / "examples/build-values/go/verify.mjs"
    records = []
    for case in dict.fromkeys(args.case or ["cpp", "go", "rust"]):
        _, release, receipt = acceptance.build(ROOT / "examples/build-values" / case, case, args.cache.resolve())
        module = acceptance.transpile(release, case)
        result = acceptance.run([acceptance.node, verifier, module])
        if result != "go-upstream-values-ok":
            raise RuntimeError(result)
        records.append({"language": case, "result": result, "componentSha256": receipt["componentSha256"],
                        "tools": receipt["tools"], "receipt": str(release / "build/build.json")})
    acceptance.finish(records)


if __name__ == "__main__":
    main()
