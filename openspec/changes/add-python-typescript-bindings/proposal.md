## Why

Today shacl2cypher is usable as a CLI or from Rust only. Most teams that validate graph data work in Python (data pipelines, notebooks, pySHACL users) or TypeScript (Node services, Neo4j apps). They currently shell out to the binary and parse files. Native bindings let them compile shapes and validate a database in-process, with typed results and no subprocess or temp-file handling.

## What Changes

- New Python package `shacl2cypher` (PyPI), built from a new `crates/s2c-python` crate with PyO3 and maturin. It exposes `compile`, `Database` handles for Neo4j and LadybugDB, `validate`, `schema` dump, report rendering and exit-code evaluation.
- New Node.js package `shacl2cypher` (npm), built from a new `crates/s2c-node` crate with napi-rs. It exposes the same API in idiomatic TypeScript: Promise-based functions, bundled `.d.ts` types, and prebuilt per-platform binaries.
- Both bindings keep the manifest and report JSON field names (camelCase). Documentation and tooling therefore work the same in Rust, the CLI, Python and TypeScript.
- Shapes can be given as file paths or as in-memory documents (name + text). Core compilation gains in-memory sources so bindings never write temp files.
- Remote-import fetching and backend opening move out of the CLI into `shacl2cypher-runner`, so the CLI and both bindings share one implementation.
- Release workflow: pushing a `v*` tag also builds and publishes abi3 wheels and an sdist to PyPI, and napi prebuilds to npm. Targets are Linux x86_64 and arm64 plus macOS arm64, with both database backends.
- CI gains pytest and Node test jobs, including LadybugDB validation through each binding.

## Capabilities

### New Capabilities
- `language-bindings`: The Python and TypeScript APIs. Covers compile from files or in-memory sources, compile options, typed errors, database handles, validate, schema dump, report rendering, exit codes, JSON shape parity with the CLI, and non-blocking execution.
- `binding-packaging`: How the bindings are built and distributed. Covers PyPI wheels and sdist, npm platform packages, supported platforms, bundled backends, version lockstep with the crates, and release on `v*` tags.

### Modified Capabilities
- `shapes-loading`: Shapes and ontologies can come from named in-memory documents as well as files, with the same determinism, provenance hashing and `owl:imports` rules.

## Impact

- **Code**:
  - New crates `crates/s2c-python` and `crates/s2c-node`, not published to crates.io.
  - New Python sources under `crates/s2c-python/python/shacl2cypher/` and npm package files under `crates/s2c-node/`.
  - `shacl2cypher-core`: `CompileRequest` accepts in-memory sources, an additive API change.
  - `shacl2cypher-runner`: gains the HTTP fetcher (feature `remote-imports`) and a backend-opening helper.
  - `shacl2cypher` CLI is refactored onto these helpers; its behavior is unchanged.
- **Dependencies**: `pyo3`, `pythonize`, `napi`/`napi-derive`, `maturin` and `@napi-rs/cli` as build tooling. `ureq` moves from the CLI to the runner. All must build on Rust 1.87.
- **CI/Release**:
  - `ci.yml` adds Python and Node jobs.
  - `release.yml` adds wheel, sdist and npm build matrices, publishing with PyPI trusted publishing and an npm token.
  - LadybugDB builds need cmake inside the manylinux containers.
- **Docs**: README install and usage sections for Python and TypeScript; `lat.md/` gains a bindings section and test specs.
