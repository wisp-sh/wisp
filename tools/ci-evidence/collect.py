#!/usr/bin/env python3
"""Stage bounded CI diagnostics from explicit paths, never build trees or caches."""
from __future__ import annotations

import argparse
import glob
import json
import os
from pathlib import Path
import re
import stat

LOG_BYTES = 256 * 1024
JSON_BYTES = 1024 * 1024
TOTAL_BYTES = 8 * 1024 * 1024
MAX_FILES = 128
PATTERNS = {
    "source-build": [
        "evidence/source-build/*.log", "evidence/source-build/*.json",
        "out/source-build-ci/*.json", "out/source-build-ci/logs/*.stdout",
        "out/source-build-ci/logs/*.stderr", "out/source-build-ci/sources/result.json",
        "out/source-build-ci/values/result.json", "out/source-build-ci/serde/result.json",
    ],
    "compose": ["evidence/component-library-chain/*.log", "evidence/component-library-chain/*.json"],
    "installed-packages": [
        "evidence/library-chain/*.log", "evidence/library-chain/publication/*.log",
        "evidence/library-chain/publication/*.json", "evidence/library-chain/go-publication/*.log",
        "evidence/library-chain/go-publication/*.json",
        "out/library-chain/report.json", "out/library-chain/packaging-tool.json",
        "out/catalog-chain/report.json", "out/library-async-chain/report.json",
        "out/library-resources/report.json", "out/library-error-context-chain/report.json",
        "out/library-networks/report.json",
    ],
}


def redactor(root, environment):
    replacements = {str(root): "$WORKSPACE"}
    if environment.get("HOME"):
        replacements[environment["HOME"]] = "$HOME"
    for key, value in environment.items():
        if len(value) >= 8 and re.search(r"TOKEN|PASSWORD|SECRET|CREDENTIAL|(?:^|_)KEY(?:$|_)", key, re.I):
            replacements[value] = "[REDACTED]"

    def redact(text):
        for value in sorted(replacements, key=len, reverse=True):
            text = text.replace(value, replacements[value])
        text = re.sub(r"(https?://)[^/\s@]+@", r"\1[REDACTED]@", text)
        text = re.sub(r"(https?://[^\s?#\"']+)[?#][^\s\"']*", r"\1?[REDACTED]", text)
        text = re.sub(r"(?i)(authorization\s*[:=]\s*)(?:bearer|basic)\s+[^\s\"']+", r"\1[REDACTED]", text)
        text = re.sub(r"\b(?:gh[pousr]_[A-Za-z0-9_]+|github_pat_[A-Za-z0-9_]+)\b", "[REDACTED]", text)
        return text
    return redact


def redact_json(value, redact):
    if isinstance(value, str):
        return redact(value)
    if isinstance(value, list):
        return [redact_json(item, redact) for item in value]
    if isinstance(value, dict):
        return {key: ("[REDACTED]" if not isinstance(item, bool) and re.search(r"token|password|secret|authorization|credential", key, re.I)
                      else redact_json(item, redact)) for key, item in value.items()}
    return value


def collect(root, output, group, environment=None):
    root, output = root.resolve(), output.absolute()
    output.mkdir(parents=True, exist_ok=False)
    redact = redactor(root, os.environ if environment is None else environment)
    records, seen, total = [], set(), 0
    limit_reached = False
    for pattern in PATTERNS[group]:
        for name in sorted(glob.glob(pattern, root_dir=root)):
            if name in seen:
                continue
            seen.add(name)
            # Reserve one file for collection.json.
            if len(records) == MAX_FILES - 1:
                limit_reached = True
                break
            path = root / name
            record = {"path": name}
            records.append(record)
            try:
                # Do not follow symlinks, including parent directories.
                if any(part.is_symlink() for part in [path, *path.parents] if part != root):
                    record["omitted"] = "symlink"
                    continue
                if not stat.S_ISREG(path.stat().st_mode):
                    record["omitted"] = "not a regular file"
                    continue
                is_json = path.suffix == ".json"
                with path.open("rb") as stream:
                    size = os.fstat(stream.fileno()).st_size
                    record["originalBytes"] = size
                    if is_json and size > JSON_BYTES:
                        record["omitted"] = "JSON size limit"
                        continue
                    if not is_json:
                        stream.seek(max(0, size - LOG_BYTES))
                    data = stream.read((JSON_BYTES if is_json else LOG_BYTES) + 1)
                if is_json:
                    data = (json.dumps(redact_json(json.loads(data), redact), indent=2) + "\n").encode()
                    if len(data) > JSON_BYTES:
                        record["omitted"] = "JSON size limit"
                        continue
                else:
                    data = redact(data.decode("utf-8", errors="replace")).encode()[-LOG_BYTES:]
                    record["tailOnly"] = size > LOG_BYTES
                # Leave room for the collection receipt itself.
                if total + len(data) > TOTAL_BYTES - 64 * 1024:
                    record["omitted"] = "total size limit"
                    continue
                destination = output / name
                destination.parent.mkdir(parents=True, exist_ok=True)
                destination.write_bytes(data)
                total += len(data)
                record["savedBytes"] = len(data)
            except (OSError, ValueError, RecursionError):
                record["omitted"] = "unreadable or invalid diagnostic"
    receipt = {"schema": 1, "group": group, "files": records, "fileLimitReached": limit_reached,
               "savedBytes": total, "limits": {"logBytes": LOG_BYTES, "jsonBytes": JSON_BYTES,
                                              "totalBytes": TOTAL_BYTES, "files": MAX_FILES}}
    receipt["recordsOmitted"] = 0
    serialized = json.dumps(receipt, indent=2) + "\n"
    while len(serialized.encode()) > 64 * 1024:
        records.pop()
        receipt["recordsOmitted"] += 1
        serialized = json.dumps(receipt, indent=2) + "\n"
    (output / "collection.json").write_text(serialized)
    return receipt


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--group", choices=PATTERNS, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    collect(Path.cwd(), args.output, args.group)
