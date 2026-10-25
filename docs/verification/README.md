# Verification and internals

These pages record how Wisp is checked and why it is built the way it is. You
don't need them to build or consume a library. The user guides are listed in
the [README](../../README.md#documentation).

- [Architecture](architecture.md): selected tools and the Component composition design
- [Admission and generator authentication](admission.md): what `wisp library validate` checks and the worker trust model
- [Test suites](test-suites.md): executable acceptance suites for builds, Go modules, publication and dependencies
- [Source compiler CI](source-build-ci.md): provisioning compiler bundles and build receipts in CI
- [Target support evidence](target-evidence.md): the detailed per-target matrix behind [target support](../target-support.md)
