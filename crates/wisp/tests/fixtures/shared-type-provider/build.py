#!/usr/bin/env python3
"""Rebuild canonical shared type providers with the pinned guest toolchain."""
import os
from pathlib import Path
import shlex
import subprocess
ROOT = Path(__file__).resolve().parents[5]
HERE = Path(__file__).resolve().parent
TARGET = ROOT / "out/shared-type-provider-target"
ENV = {**os.environ, "RUSTUP_TOOLCHAIN": "1.98.1", "RUSTUP_AUTO_INSTALL": "0"}
flags = os.environ.get("CARGO_ENCODED_RUSTFLAGS")
flags = flags.split("\x1f") if flags is not None else shlex.split(os.environ.get("RUSTFLAGS", ""))
flags += [f"--remap-path-prefix={Path.home()}=/wisp/home", f"--remap-path-prefix={ROOT}=/wisp/source"]
ENV.pop("RUSTFLAGS", None)
ENV["CARGO_ENCODED_RUSTFLAGS"] = "\x1f".join(flags)
subprocess.run(["cargo", "build", "--locked", "--offline", "--manifest-path", str(HERE / "Cargo.toml"), "--target", "wasm32-unknown-unknown", "--target-dir", str(TARGET)], check=True, env=ENV)
for name in ("provider", "consumer"):
    output = HERE / (name + ".wasm")
    tool = ROOT / ".tools/component-library-chain/wasm-tools"
    subprocess.run([str(tool), "component", "new", str(TARGET / "wasm32-unknown-unknown/debug" / ("shared_type_" + name + ".wasm")), "-o", str(output)], check=True)
    subprocess.run([str(tool), "strip", str(output), "-o", str(output)], check=True)
    subprocess.run([str(tool), "validate", str(output)], check=True)
