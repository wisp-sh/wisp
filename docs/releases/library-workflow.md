# Public library build workflow

This workflow and its composite actions are the public client for the private Wisp server. They compile the caller repository, produce registry archives, attest package hashes, and record builds with GitHub OIDC. Server routes, Convex functions, storage credentials and deployment configuration remain in the server repository.

The workflow accepts `wisp_url` and `go_host` so a caller can use a test deployment. Set `publish: false` for a server and Go-proxy test without publishing to npm, crates.io or PyPI. Pin the reusable workflow by its full commit and approve that same workflow reference and SHA on the server.

The runtime manifest URL and digest are checked into `scripts/ci/runtime.lock.json`. Callers cannot select executable runtime inputs. The first successful combined runtime release must be pinned before this workflow can build a library.
