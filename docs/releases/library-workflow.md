# Public library build workflow

This workflow and its composite actions are the public client for the private Wisp server. They compile the caller repository, produce registry archives, attest package hashes, and record builds with GitHub OIDC. Server routes, Convex functions, storage credentials and deployment configuration remain in the server repository.

The workflow accepts `wisp_url` and `go_host` so a caller can use a test deployment. Set `publish: false` for a server and Go-proxy test without publishing to npm, crates.io or PyPI. Pin the reusable workflow by its full commit and approve that same workflow reference and SHA on the server.

The runtime manifest URL and digest are checked into `scripts/ci/runtime.lock.json`. Callers cannot select executable runtime inputs. The initial pinned release is `runtime-2026.10.26`. Runtime selection is part of the immutable action commit approved by the server.

## Declare packages

Put `.wisp.json` at the repository root. This schema-2 example builds the Wisp
package in `packages/math` and publishes under names owned by your organization.
Replace the registry names with yours and omit targets you do not use.

```json
{
  "schema": 2,
  "packages": [
    {
      "name": "math",
      "path": "packages/math",
      "summary": "Arithmetic Component bindings",
      "targets": {
        "npm": "@your-org/math",
        "cargo": "your-org-math",
        "pypi": "your-org-math",
        "go": true
      }
    }
  ]
}
```

Each package directory needs its normal Wisp sources and `wisp.toml`. The version
comes from `wisp.toml`, not the GitHub release tag. Python currently requires a
stable `MAJOR.MINOR.PATCH` version and synchronous exports without host imports.
Its registry name must normalize to a valid Python import name.

Each logical package must have a distinct identity within each registry, even
when versions differ. PyPI lowercases names and collapses runs of `.`, `_`, and
`-`, so `Foo.Bar` and `foo-bar` cannot name two packages in one manifest. Cargo
also treats case and `_` versus `-` as equivalent. The build rejects collisions
before invoking the compiler, and artifact admission checks them again. See
[Python name normalization](https://packaging.python.org/en/latest/specifications/name-normalization/)
and [crates.io name restrictions](https://doc.rust-lang.org/cargo/reference/registry-index.html#name-restrictions).

## Caller workflow

Save this complete workflow as `.github/workflows/library.yml` in the caller
repository. Branch pushes build and record only. A published GitHub release also
publishes the declared npm and Cargo archives, then runs the PyPI job below.
Changing `publish` to `false` disables all three registry uploads, including the
caller job, while preserving builds, attestations and recording.

```yaml
name: Library releases
on:
  push:
    branches: ['**']
  release:
    types: [published]
permissions: {}
concurrency:
  group: library-${{ github.ref }}
  cancel-in-progress: false
jobs:
  library:
    uses: wisp-sh/wisp/.github/workflows/wisp-build.yml@WISP_WORKFLOW_COMMIT
    with:
      publish: true
    permissions:
      contents: read
      actions: read
      id-token: write
      attestations: write
  pypi:
    needs: library
    if: github.event_name == 'release' && needs.library.outputs.pypi == 'true'
    runs-on: ubuntu-24.04
    timeout-minutes: 20
    environment: pypi
    permissions:
      actions: read
      id-token: write
    steps:
      - name: Download this run's exact recorded artifact
        uses: actions/download-artifact@d3f86a106a0bac45b974a628896c90dbdf5c8093
        with:
          artifact-ids: ${{ needs.library.outputs.artifact_id }}
          path: ${{ runner.temp }}/wisp-artifacts
          merge-multiple: true
      - name: Verify artifacts and stage only missing Python wheels
        id: python
        uses: wisp-sh/wisp/.github/actions/library-publish@WISP_ACTION_COMMIT
        with:
          command: pypi
      - name: Publish Python wheels with trusted publishing
        if: steps.python.outputs.pypi == 'true'
        uses: pypa/gh-action-pypi-publish@7245906fb61fa33a13554f9d5fb856fe61758086
        with:
          packages-dir: wisp-wheels
          skip-existing: false
```

Register each Python project's trusted publisher on PyPI with your GitHub owner,
repository, the caller filename `library.yml`, and environment `pypi`. Create the
matching GitHub environment and set its release protection rules. Approve the
pinned reusable workflow reference and SHA with your Wisp server. Configure npm
and crates.io trusted publishers separately if those targets are present.

The PyPI upload job must stay in this top-level workflow. PyPI does not support
trusted publishing from a reusable workflow, as documented in
[PyPI troubleshooting](https://docs.pypi.org/trusted-publishers/troubleshooting/#reusable-workflows-on-github)
and the
[pinned PyPA action README](https://github.com/pypa/gh-action-pypi-publish/blob/7245906fb61fa33a13554f9d5fb856fe61758086/README.md#trusted-publishing).

The reusable call needs permission to record and attest, but its build job
reduces permissions to `contents: read`. Source compilation has no OIDC or
registry credentials. Recording and publication run on fresh runners, with
pinned actions reading inert artifacts. The caller's PyPI job does not check out
source, build packages, install dependencies, or execute code from the artifact.
Keep it that way when adapting the example. The workspace-relative `wisp-wheels`
directory lets the PyPA Docker action read the staged wheels.

The reusable workflow exposes these outputs:

| Output | Meaning |
| --- | --- |
| `artifact_id` | Exact GitHub artifact ID produced by this run and attempt. Use this rather than an artifact name or a previous run. |
| `artifact_digest` | GitHub artifact SHA-256 passed to Wisp when recording. Per-file SHA-256 and size checks run again before publication. |
| `pypi` | String `true` only when recording succeeded for a release, `publish` is enabled, and a package declares PyPI. Callers must also depend on successful completion of the reusable workflow. |

## Retrying a Python release

The `pypi` command revalidates the artifact tree, then queries PyPI's
[release JSON API](https://docs.pypi.org/api/json/#get-a-release) for each declared
project and version. It compares the exact wheel filename, SHA-256 and size.
Identical files stay out of the upload directory. A missing release or missing
filename stages the verified local wheel. A release with only an sdist, another
platform wheel, or another version spelling does not prove that this wheel was
published. PyPI's metadata version spelling does not replace filename matching.

A checksum mismatch, missing integrity data, yanked matching file, malformed
response, or registry error fails the preparation step. It checks all projects
before staging files or writing a success output. Neither preparation nor wheel
creation replaces existing files. Run `plan` and `pypi` in separate jobs or give
them distinct fresh `wheels-directory` inputs.

Keep `skip-existing: false` on the PyPA action. If another upload wins the race
after preparation, the upload fails visibly. Rerun the failed caller job to
check the registry again; it will skip that file only if the bytes match. An
all-identical retry skips the upload action entirely. A partially uploaded
release stages only its missing wheels. Resolve a mismatch by publishing a new
package version, and review a yanked release before changing its yank status.
