#!/usr/bin/env python3
"""Publish admitted archives, exercise retries, and install outside the checkout."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile

from registry import Registry


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--cli", required=True, type=Path)
    parser.add_argument("--release", required=True, type=Path)
    parser.add_argument("--evidence", required=True, type=Path)
    args = parser.parse_args()
    cli = str(args.cli.resolve())
    source = args.release.resolve()
    args.evidence.mkdir(parents=True, exist_ok=False)
    record = json.loads((source / "library-release.json").read_text())
    has_go = "go" in record["packages"]
    filenames = ["library-release.json", "component.wasm", "rust.crate", "node.tgz"]
    if has_go: filenames += ["go.zip", "go.mod", "go.info"]
    provenance = {"cliSha256": hashlib.sha256(Path(cli).read_bytes()).hexdigest(),
                  "inputs": {name: hashlib.sha256((source / name).read_bytes()).hexdigest()
                             for name in filenames}}
    (args.evidence / "inputs.json").write_text(json.dumps(provenance, indent=2))
    env = dict(os.environ, WISP_PUBLISH_TEST_TOKEN="test-token")
    with tempfile.TemporaryDirectory(prefix="wisp-publication-") as temporary:
        root = Path(temporary)
        release = root / "release"
        release.mkdir()
        for name in filenames:
            shutil.copy2(source / name, release / name)
        record = json.loads((release / "library-release.json").read_text())
        cargo_name = record["packages"]["rust"]["name"]
        npm_name = record["packages"]["node"]["name"]
        plan = root / "publication.json"
        state = root / "state.json"
        log = (args.evidence / "commands.log").open("w")

        def run(command, *, cwd=root, succeeds=True, extra_env=None):
            log.write("$ " + repr(command) + "\n")
            log.flush()
            result = subprocess.run(command, cwd=cwd, env=dict(env, **(extra_env or {})), text=True, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
            log.write(result.stdout)
            log.flush()
            assert (result.returncode == 0) == succeeds, result.stdout
            return result

        with Registry() as registry:
            configuration = {
                "cargo": {"index": registry.url + "/cargo/", "api": registry.url, "package": cargo_name, "tokenEnv": "WISP_PUBLISH_TEST_TOKEN"},
                "npm": {"registry": registry.url + "/npm/", "package": npm_name, "access": "public", "tokenEnv": "WISP_PUBLISH_TEST_TOKEN"},
                "component": {"registry": registry.url, "repository": "wisp/library", "tokenEnv": "WISP_PUBLISH_TEST_TOKEN"},
            }
            if has_go:
                configuration["go"] = {"directory": str(root / "go-proxy"), "module": record["packages"]["go"]["name"]}
            configuration_path = root / "publisher.json"
            configuration_path.write_text(json.dumps(configuration))
            run([cli, "library", "prepare", str(release), "--config", str(configuration_path), "--out", str(plan)])
            assert not registry.requests, "prepare made registry requests"
            registry.fail_npm_once = True
            run([cli, "library", "publish", str(plan), "--state", str(state)], succeeds=False)
            partial = json.loads(state.read_text())
            assert set(partial["registries"]) == {"component", "cargo"}
            assert not partial["complete"] and partial["lastError"]["registry"] == "npm"
            writes = len(registry.writes())
            run([cli, "library", "publish", str(plan), "--state", str(state)])
            assert len(registry.writes()) == writes + 1, "retry repeated completed registry writes"
            assert json.loads(state.read_text())["complete"]
            writes = len(registry.writes())
            run([cli, "library", "publish", str(plan), "--state", str(state)])
            assert len(registry.writes()) == writes, "completed retry wrote registry content"
            fetched = root / "fetched"
            component_digest = json.loads(plan.read_text())["component"]["digest"]
            run([cli, "library", "fetch", str(configuration_path), "--digest", component_digest, "--out", str(fetched)])
            for filename in filenames:
                assert (fetched / filename).read_bytes() == (release / filename).read_bytes()
            assert json.loads((fetched / "component.json").read_text())["component"]["sha256"] == record["componentSha256"]
            blob = "sha256:" + record["componentSha256"]
            saved_blob = registry.blobs[blob]
            registry.blobs[blob] = b"forged"
            failure = run([cli, "library", "fetch", str(configuration_path), "--digest", component_digest, "--out", str(root / "forged-fetch")], succeeds=False)
            assert "digest mismatch" in failure.stdout
            assert not (root / "forged-fetch").exists()
            registry.blobs[blob] = saved_blob
            if has_go:
                go_consumer = root / "go-consumer"
                go_consumer.mkdir()
                module = record["packages"]["go"]["name"]
                go_environment = {"GOPROXY": (root / "go-proxy").as_uri(), "GOSUMDB": "off", "GOMODCACHE": str(root / "go-cache"), "GOTOOLCHAIN": "local"}
                (go_consumer / "go.mod").write_text(f'module consumer.invalid/test\n\ngo 1.27.0\n\nrequire {module} v{record["version"]}\n')
                (go_consumer / "main.go").write_text(f'package main\nimport lib "{module}"\nfunc main() {{ client,err:=lib.NewClient();if err!=nil{{panic(err)}};defer client.Close() }}\n')
                run(["go", "mod", "tidy"], cwd=go_consumer, extra_env=go_environment)
                run(["go", "run", "."], cwd=go_consumer, extra_env=dict(go_environment, GOPROXY="off"))
                version_file = root / "go-proxy" / module / "@v" / f'v{record["version"]}.mod'
                saved = version_file.read_bytes()
                version_file.write_bytes(b"module forged.invalid/code\n")
                conflict = run([cli, "library", "publish", str(plan), "--state", str(state)], succeeds=False)
                assert "Go proxy version content conflict" in conflict.stdout
                version_file.write_bytes(saved)
                run([cli, "library", "publish", str(plan), "--state", str(state)])
            tarball = next(iter(registry.npm_tarballs))
            original = registry.npm_tarballs[tarball]
            registry.npm_tarballs[tarball] = b"substituted archive"
            conflict = run([cli, "library", "publish", str(plan), "--state", str(state)], succeeds=False)
            assert "conflict" in conflict.stdout
            assert len(registry.writes()) == writes
            registry.npm_tarballs[tarball] = original
            run([cli, "library", "publish", str(plan), "--state", str(state)])

            published = registry.npm[npm_name]["versions"][record["version"]]
            published["dependencies"] = {"unreviewed-code": "*"}
            conflict = run([cli, "library", "publish", str(plan), "--state", str(state)], succeeds=False)
            assert "metadata conflict" in conflict.stdout
            assert len(registry.writes()) == writes
            del published["dependencies"]
            cargo_metadata = registry.crates[(cargo_name, record["version"])][0]
            saved_registry = cargo_metadata["deps"][0]["registry"]
            cargo_metadata["deps"][0]["registry"] = "https://unreviewed.example/index"
            conflict = run([cli, "library", "publish", str(plan), "--state", str(state)], succeeds=False)
            assert "dependency metadata conflicts" in conflict.stdout
            assert len(registry.writes()) == writes
            cargo_metadata["deps"][0]["registry"] = saved_registry
            run([cli, "library", "publish", str(plan), "--state", str(state)])

            local_archive = release / "node.tgz"
            saved_archive = local_archive.read_bytes()
            local_archive.write_bytes(saved_archive + b"changed")
            request_count = len(registry.requests)
            run([cli, "library", "publish", str(plan), "--state", str(state)], succeeds=False)
            assert len(registry.requests) == request_count, "changed local archives reached a registry"
            local_archive.write_bytes(saved_archive)

            node = root / "node-consumer"
            node.mkdir()
            (node / "package.json").write_text(json.dumps({"name": "wisp-publication-consumer", "version": "1.0.0", "type": "module", "private": True}))
            run(["npm", "install", "--ignore-scripts", "--no-audit", "--no-fund", "--registry", registry.url + "/npm/", "--cache", str(root / "npm-cache"), f"{npm_name}@{record['version']}"], cwd=node)
            run(["node", "--input-type=module", "-e", f"await import({json.dumps(npm_name)}); console.log('published-node-import-ok')"], cwd=node)

            rust = root / "rust-consumer"
            (rust / "src").mkdir(parents=True)
            (rust / ".cargo").mkdir()
            (rust / "src/main.rs").write_text("fn main() {}\n")
            (rust / "Cargo.toml").write_text(f'[package]\nname="wisp-publication-consumer"\nversion="1.0.0"\nedition="2024"\n[dependencies]\n{cargo_name}={{version="={record["version"]}",registry="wisp-publication"}}\n[workspace]\n')
            (rust / ".cargo/config.toml").write_text(f'[registries.wisp-publication]\nindex="sparse+{registry.url}/cargo/"\n')
            run(["cargo", "fetch"], cwd=rust, extra_env={"CARGO_NET_OFFLINE": "false"})
            assert any(path.startswith("/crates/") for method, path in registry.requests)
            assert "test-token" not in plan.read_text() + state.read_text()
            shutil.copy2(plan, args.evidence / "publication.json")
            shutil.copy2(state, args.evidence / "state.json")
            (args.evidence / "requests.json").write_text(json.dumps(registry.requests, indent=2))
        with Registry() as malicious, Registry() as destination:
            malicious.upload_location_override = destination.url + "/collect-credentials"
            configuration["cargo"].update(index=malicious.url + "/cargo/", api=malicious.url)
            configuration["npm"]["registry"] = malicious.url + "/npm/"
            configuration["component"]["registry"] = malicious.url
            configuration_path.write_text(json.dumps(configuration))
            bad_plan = root / "redirect-plan.json"
            run([cli, "library", "prepare", str(release), "--config", str(configuration_path), "--out", str(bad_plan)])
            failure = run([cli, "library", "publish", str(bad_plan), "--state", str(root / "redirect-state.json")], succeeds=False)
            assert "changed origin" in failure.stdout
            assert not destination.requests, "OCI upload forwarded credentials to another origin"
        run(["cargo", "fetch", "--offline", "--locked"], cwd=rust)
        run(["node", "--input-type=module", "-e", f"await import({json.dumps(npm_name)}); console.log('offline-published-node-import-ok')"], cwd=node)
        log.close()
        (args.evidence / "result.json").write_text(json.dumps({"passed": True, "partialSuccess": True, "retryVerified": True, "conflictRejected": True, "registryMetadataVerified": True, "changedLocalBytesRejected": True, "credentialForwardingRejected": True, "cargoInstalledOutsideCheckout": True, "npmInstalledOutsideCheckout": True, "offlineCacheVerified": True, "ociFetchVerified": True, "forgedOciBlobRejected": True, "goInstalledOutsideCheckout": has_go}, indent=2))


if __name__ == "__main__":
    main()
