# Publishing Component libraries

`wisp library prepare` validates a schema-3 library release and writes a
reviewable publication plan. `wisp library publish` sends the exact admitted
archives to configured Cargo, npm, and OCI registries, and optionally writes a
Go module proxy directory. It records each completed
registry write separately and verifies downloaded bytes before marking it done.

## Configure and prepare

Package names come from the admitted release. Publisher configuration must
repeat those exact names and name the destination registries. The following
`wisp` names are examples; this project has not established ownership of them
on any public service.

```json
{
  "cargo": {
    "index": "https://cargo.wisp.example/index/",
    "api": "https://cargo.wisp.example",
    "package": "wisp-wisp--library-chain",
    "tokenEnv": "WISP_CARGO_TOKEN"
  },
  "npm": {
    "registry": "https://npm.wisp.example/",
    "package": "@wisp/library-chain",
    "access": "public",
    "tokenEnv": "WISP_NPM_TOKEN"
  },
  "component": {
    "registry": "https://oci.wisp.example",
    "repository": "wisp/library-chain",
    "tokenEnv": "WISP_OCI_TOKEN"
  },
  "go": {
    "directory": "/absolute/path/to/wisp-go-proxy",
    "module": "wisp.example/wisp/library-chain"
  }
}
```

Use the HTTP URL of the Cargo sparse index without the `sparse+` prefix.
`tokenEnv` names an environment variable, or is `null` for an unauthenticated
registry. Credentials are read only when publishing and are absent from the
configuration, plan, and state files. Cargo receives its registry API token;
npm and OCI receive a bearer token. HTTPS is required except for literal
`http://127.0.0.1` test registries. HTTP redirects and cross-origin OCI upload
locations are rejected, so credentials do not follow an upload to another host.

```bash
wisp library prepare out/library-release --config publisher.json \
  --out publication.json
cat publication.json

# After reviewing the destinations, package names, versions and hashes:
wisp library publish publication.json --state publication-state.json
```

Omit `go` when the release has no Go module. Its configured module path must
match the release generated with `wisp library pack --go-module`.

Preparation makes no registry requests. It snapshots and validates the Component,
release metadata, and language archives. The plan records their hashes and sizes,
the absolute release directory, registry configuration, and the exact OCI
manifest and digest. Keep that directory and the pinned validation tools available
through publication. Publishing revalidates a private copy of the files and
compares the regenerated plan before contacting registries. Changed local bytes
require a new plan.

The plan file must be absent before preparation. State files belong to the hash
of one exact plan. A state file from another plan fails before any registry write.
No command claims or transfers package ownership. Registry access, existing name
ownership, and acceptable package metadata remain publisher responsibilities.
For example, generated crates without a license declaration do not meet
crates.io's publication requirements. The publisher does not invent one.

## Registry transports

Cargo publication uses the standard framed metadata/archive upload. The client
checks that the sparse index's configured API matches the reviewed destination,
then verifies the version checksum, dependency metadata, and downloaded archive.
An index that has not exposed a newly uploaded version produces a retryable
failure with the completed registry state retained. The original `.crate` bytes
are never repacked. See the [Cargo publish API](https://doc.rust-lang.org/cargo/reference/registry-web-api.html)
and [sparse index contract](https://doc.rust-lang.org/cargo/reference/registry-index.html).

npm publication uses its package metadata PUT and base64 tarball attachment,
with SHA-1 and SHA-512 integrity fields. Wisp verifies the published version's
executable and dependency metadata and downloads the archive to compare its
exact bytes. The request follows [npm's publishing implementation](https://github.com/npm/cli/blob/latest/workspaces/libnpmpublish/lib/publish.js).
An npm publication uses the configured access level and updates the `latest`
dist-tag to that version. Existing versions with different content are refused.

The Component transport uses the [OCI Distribution API](https://github.com/opencontainers/distribution-spec/blob/v1.1.1/spec.md)
and an [OCI 1.1 artifact manifest](https://github.com/opencontainers/image-spec/blob/v1.1.0/manifest.md).
It uploads each blob through a POST/PUT upload session, then publishes a manifest
by digest. The manifest contains the original Component, schema-3 release JSON,
Cargo archive, and npm archive. Version and Component identity are annotations.
The returned `registry/repository@sha256:...` reference identifies the exact
manifest. Optional Go `.zip`, `.mod`, and `.info` files are additional layers.
Wisp does not move a mutable OCI version tag.

Go publication writes the standard module proxy layout under the configured
absolute directory. Serve that directory as a static HTTPS site, or use a
`file://` GOPROXY for local installation. Wisp writes immutable version files
and updates the version list under a directory lock. Retries verify existing
bytes and complete missing files from an interrupted write. A changed published
version fails. Wisp does not invent a Go registry upload API or create source
repository tags.

This is a Wisp release artifact with its own media types. It does not implement
the single-Wasm-layer [CNCF Wasm OCI artifact layout](https://tag-runtime.cncf.io/wgs/wasm/deliverables/wasm-oci-artifact/),
and it does not claim direct `wkg` compatibility. That layout is suitable for
transporting a Wasm package; the release artifact here also binds both language
archives. [Warg](https://github.com/bytecodealliance/registry) provides a separate
Component registry protocol with signed package logs. This implementation chooses
OCI's content-addressed transport and does not provide Warg publication,
federated naming, or package-log verification.

OCI registries requiring an interactive bearer challenge, token exchange, or
chunked-only uploads are outside this first client. Supply a bearer token already
authorized for the configured repository. Cargo and npm registries must provide
the standard readback endpoints; cross-origin archive downloads receive no
publisher credentials. Public-registry interoperability is not established by
the loopback fixture.

## Partial success and retries

Publication runs OCI, Cargo, npm, then the optional Go proxy. After each
destination's bytes are verified,
Wisp writes and syncs a temporary state file, renames it over the state path,
and syncs the containing directory. An OS file lock prevents two local processes
from updating the same state concurrently. A process crash releases the lock.

The registries do not share a transaction. A later failure preserves earlier
publications and records the failing registry. Retry the same command and state
file. The client rereads completed registry content without uploading it again,
checks incomplete destinations for an already committed matching version, and
continues. A previously completed object that disappeared or changed fails.
There is no rollback, unpublish, or claim of atomic publication across registries.

The readback limits and request timeout bound transport work. Hash verification
does not establish registry ownership, source provenance, or safe execution of
arbitrary Component code.

## Fetch an immutable Component release

```bash
wisp library fetch publisher.json --digest sha256:MANIFEST_DIGEST \
  --out registry/wisp/library-chain/0.1.0
```

Fetch verifies the OCI manifest digest and every layer, then runs library
admission before exposing the output directory. It also checks the configured
Cargo and npm names. The directory contains `component.json`, which lets
`wisp deps lock --registry registry` resolve the downloaded Component through
the local registry. Fetch requires the pinned admission tools and, for a Go
release, its supported native builder. The output path must not already exist.

## Executable acceptance

After creating a schema-3 release with the pinned generator, run:

```bash
python3 tests/library-publication/run.py --cli /absolute/path/to/wisp \
  --release out/library-release --evidence evidence/publication-new-run
```

The fixture starts empty loopback registries and publishes through their standard
Cargo/npm APIs and OCI upload endpoints. It injects an npm failure after OCI and
Cargo succeed, verifies that retry preserves those successes, and checks a
completed retry makes no writes. Negative controls alter remote tarball bytes,
registry dependency metadata, and local release bytes, and send an OCI upload
location to another origin.

Fresh Cargo, npm, and optional Go consumers live in a temporary directory outside the checkout.
They install from the published registries, then reuse their caches after those
registries shut down. Node imports the installed public package. Cargo verifies
package installation and offline fetching here; native execution and the typed
success/error comparison belong to the separate library-chain acceptance.
No public registry is contacted for publication. Initial Cargo dependency
resolution may fetch the generated package's pinned runtime dependencies from
crates.io when they are absent from the local cache.

The all-channel run at `evidence/publication-all-v2/result.json` passed package
installation, offline cache reuse, partial-success retry, immutable Go version
checks, OCI fetch, and rejection of a forged OCI blob. These are local registry
results, not evidence of publication to public accounts.
