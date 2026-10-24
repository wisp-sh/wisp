"""Freeze CLI and worker bytes; nested verifiers may share a frozen CLI."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess


WORKER_NAME = "wisp-worker.exe" if os.name == "nt" else "wisp-worker"
WORKER_VERSION = "wisp-worker 0.2.0 wasmtime 50.0.0 protocol 1"


def resolve_worker(cli, environment):
    if "WISP_WORKER" in environment:
        worker = Path(environment["WISP_WORKER"]).absolute()
        if not worker.is_file():
            raise ValueError(f"WISP_WORKER is not a worker file: {worker}")
        return worker.resolve()
    sibling = Path(cli).resolve().with_name(WORKER_NAME)
    if sibling.is_file():
        return sibling
    for directory in environment.get("PATH", "").split(os.pathsep):
        candidate = Path(directory) / WORKER_NAME
        if candidate.is_file():
            return candidate.resolve()
    raise ValueError("missing wisp-worker: install it beside wisp or set WISP_WORKER")


def check_worker(worker):
    try:
        result = subprocess.run([str(worker), "--version"], text=True, capture_output=True,
                                timeout=30, check=False)
    except (OSError, subprocess.TimeoutExpired) as error:
        raise ValueError(f"cannot execute wisp-worker {worker}: {error}") from error
    if result.returncode or result.stdout.strip() != WORKER_VERSION:
        raise ValueError(f"incompatible wisp-worker {worker}: expected {WORKER_VERSION!r}; "
                         f"got {result.stdout.strip()!r} {result.stderr.strip()!r}")


def verify_cli(cli, cli_sha256, worker_sha256, *, environment=None):
    """Check frozen bytes and any explicit worker selection before replay."""
    environment = os.environ if environment is None else environment
    cli = Path(cli).absolute()
    if not cli.is_file() or digest(cli) != cli_sha256:
        raise ValueError("packaging executable missing or changed since installation")
    worker = cli.with_name(WORKER_NAME)
    if not worker_sha256:
        raise ValueError("missing worker hash; prepare a fresh snapshot before replay")
    if not worker.is_file() or digest(worker) != worker_sha256:
        raise ValueError("frozen wisp-worker missing or changed since installation")
    selected = resolve_worker(cli, environment)
    if digest(selected) != worker_sha256:
        raise ValueError("WISP_WORKER differs from the frozen worker; prepare a fresh snapshot")
    check_worker(selected)
    environment["WISP_WORKER"] = str(selected)


def worker_digest(cli):
    return digest(Path(cli).with_name(WORKER_NAME))


def digest(path):
    with Path(path).open("rb") as file:
        return hashlib.file_digest(file, "sha256").hexdigest()


def freeze_cli(source, destination, *, allow_link=True, environment=None):
    """Freeze the selected pair and pin environment to the copied worker.

    Keep returning the CLI hash for existing source-build callers. The separate
    worker receipt survives callers writing their existing frozen-cli.json.
    """
    environment = os.environ if environment is None else environment
    source, destination = Path(source).resolve(), Path(destination).absolute()
    if destination.exists() or destination.is_symlink():
        raise ValueError(f"CLI snapshot already exists: {destination}")
    expected = digest(source)
    marker = environment.get("WISP_FROZEN_CLI")
    frozen_parent = marker and Path(marker).resolve() == source
    linked = False
    if frozen_parent:
        receipt = json.loads((source.parent / "frozen-cli.json").read_text())
        if (receipt != {"schema": 1, "kind": "source-build-cli-snapshot", "sha256": expected}
                or environment.get("WISP_FROZEN_CLI_SHA256") != expected):
            raise ValueError("parent CLI snapshot changed")
    worker = resolve_worker(source, environment)
    worker_expected = digest(worker)
    worker_receipt = source.parent / "frozen-worker.json"
    if frozen_parent:
        if (not worker_receipt.is_file()
                or json.loads(worker_receipt.read_text()).get("sha256") != worker_expected):
            raise ValueError("parent worker snapshot changed")
    check_worker(worker)
    frozen_worker = destination.with_name(WORKER_NAME)
    destination_receipt = destination.parent / "frozen-worker.json"
    if frozen_worker.exists() or frozen_worker.is_symlink():
        if (frozen_worker.resolve() != worker or not destination_receipt.is_file()
                or json.loads(destination_receipt.read_text()).get("sha256") != worker_expected):
            raise ValueError(f"worker snapshot already exists: {frozen_worker}")
    else:
        shutil.copy2(worker, frozen_worker)
    if digest(frozen_worker) != worker_expected:
        raise ValueError("worker changed while freezing its snapshot")
    destination_receipt.write_text(json.dumps({
        "schema": 1, "kind": "source-build-worker-snapshot", "sha256": worker_expected,
    }) + "\n")
    if allow_link and frozen_parent:
        try:
            os.link(source, destination)
            linked = True
        except OSError:
            pass
    if not linked:
        shutil.copy2(source, destination)
    if digest(destination) != expected:
        raise ValueError("CLI changed while freezing its snapshot")
    environment["WISP_WORKER"] = str(frozen_worker)
    return expected
