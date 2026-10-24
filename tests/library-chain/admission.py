#!/usr/bin/env python3
"""Exercise Rust library admission with altered packages and recalculated hashes."""
import argparse
import hashlib
import io
import json
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile


def mutate_archive(path, suffix, transform):
    with tarfile.open(path, "r:gz") as archive:
        files = [(member.name, archive.extractfile(member).read())
                 for member in archive.getmembers() if member.isfile()]
    matched = False
    with tarfile.open(path, "w:gz", format=tarfile.USTAR_FORMAT) as archive:
        for name, contents in files:
            if name.endswith(suffix):
                contents = transform(contents)
                matched = True
            member = tarfile.TarInfo(name)
            member.mode = 0o644
            member.size = len(contents)
            archive.addfile(member, io.BytesIO(contents))
    if not matched:
        raise RuntimeError(f"missing mutation target: {suffix}")


def run(wisp, release):
    results = []

    def check(directory, expected):
        result = subprocess.run([str(wisp), "library", "validate", str(directory)],
                                text=True, capture_output=True)
        if expected is None:
            if result.returncode or not json.loads(result.stdout)["valid"]:
                raise RuntimeError(result.stdout + result.stderr)
        elif not result.returncode or expected not in result.stderr:
            raise RuntimeError(f"expected {expected!r}: {result.stdout}{result.stderr}")
        return result.stderr.strip()

    check(release, None)
    mutations = [
        ("rust-wrapper", "rust", "src/lib.rs", lambda b: b + b"\npub fn injected() {}\n",
         "regenerated Rust file differs"),
        ("guest-contract", "rust", "wit/guest.wit", lambda b: b.replace(b"u64", b"u32"),
         "regenerated Rust file differs"),
        ("cargo-dependency", "rust", "Cargo.toml",
         lambda b: b.replace(b'=50.0.0', b'=49.0.0'), "Rust package descriptor differs"),
        ("node-wrapper", "node", "index.js", lambda b: b + b"\nexport const injected = 1;\n",
         "regenerated Node package differs"),
        ("node-install-script", "node", "package.json",
         lambda b: json.dumps(dict(json.loads(b), scripts={"install": "echo injected"})).encode(),
         "regenerated Node package differs"),
    ]
    with tempfile.TemporaryDirectory(prefix="wisp-library-admission-") as temporary:
        for name, target, suffix, transform, expected in mutations:
            directory = Path(temporary) / name
            directory.mkdir()
            for filename in ["library-release.json", "component.wasm", "rust.crate", "node.tgz"]:
                shutil.copyfile(release / filename, directory / filename)
            metadata = json.loads((directory / "library-release.json").read_text())
            archive = directory / ("rust.crate" if target == "rust" else "node.tgz")
            mutate_archive(archive, suffix, transform)
            metadata["packages"][target]["sha256"] = hashlib.sha256(archive.read_bytes()).hexdigest()
            (directory / "library-release.json").write_text(json.dumps(metadata))
            results.append({"name": name, "diagnostic": check(directory, expected)})

        for name, mutate, expected in [
            ("target-support", lambda m: m["targets"].append("go"), "forged library tool or target support"),
            ("contract", lambda m: m["contract"].update(world="forged"), "library contract mismatch"),
            ("component", lambda m: m.update(componentSha256="0" * 64), "library Component digest mismatch"),
        ]:
            directory = Path(temporary) / name
            directory.mkdir()
            for filename in ["library-release.json", "component.wasm", "rust.crate", "node.tgz"]:
                shutil.copyfile(release / filename, directory / filename)
            metadata = json.loads((directory / "library-release.json").read_text())
            mutate(metadata)
            (directory / "library-release.json").write_text(json.dumps(metadata))
            results.append({"name": name, "diagnostic": check(directory, expected)})
    return {"status": "passed", "controls": results}


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("release", type=Path)
    parser.add_argument("--wisp", type=Path, required=True)
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    args.report.unlink(missing_ok=True)
    report = run(args.wisp.resolve(), args.release.resolve())
    args.report.parent.mkdir(parents=True, exist_ok=True)
    args.report.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))
