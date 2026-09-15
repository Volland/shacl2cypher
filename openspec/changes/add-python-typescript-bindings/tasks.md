## 1. Preflight

- [x] 1.1 Pin PyO3, pythonize, napi, napi-derive and napi-build versions that build on Rust 1.87 with the `fallback` resolver; verify with a throwaway `cargo build` of each binding skeleton in `spikes/` and record the chosen versions in `lat.md`
- [x] 1.2 Check availability of `shacl2cypher` on PyPI and of `shacl2cypher`, `shacl2cypher-linux-x64-gnu`, `shacl2cypher-linux-arm64-gnu`, `shacl2cypher-darwin-arm64` on npm; report any taken name to the user before continuing to group 8

## 2. Core: in-memory documents

- [x] 2.1 Add `Document { name, text, format }` and `RdfFormat` to `shacl2cypher-core`, plus `shape_documents`/`ontology_documents` on `CompileRequest` with a `CompileRequest::new` constructor
- [x] 2.2 Add `InputSource::Document` to `ShapesGraph` loading: path `base_dir.join(name)`, `file://` base IRI, format from explicit value or extension, per-document blank-node ids, SHA-256 of UTF-8 bytes, merged sorting with files
- [x] 2.3 Reject duplicate document names and documents colliding with a given file; reject unknown formats naming the supported ones
- [x] 2.4 Resolve relative `owl:imports` from documents against `base_dir`, reusing the existing import loop
- [x] 2.5 Unit tests for the `shapes-loading` "In-memory shape documents" scenarios, including byte-identical manifests versus files and `name:line` error locations

## 3. Runner: shared glue and CLI refactor

- [x] 3.1 Move `HttpFetcher` into `runner::remote` behind feature `remote-imports` (ureq, 30 s timeout, 16 MB cap), with a unit test for the size cap using a local fetch stub
- [x] 3.2 Add `runner::backend::{BackendConfig, BackendError, open, available_backends}` replacing the CLI's `open_backend`
- [x] 3.3 Add `runner::session::manifest_for` (shapes vs manifest, LadybugDB schema auto-dump, dialect mismatch rejection) and a timeout validation helper
- [x] 3.4 Refactor `crates/s2c-cli/src/main.rs` onto 3.1–3.3; forward `remote-imports` from the CLI crate by default; confirm `cargo test --workspace` and backend test jobs pass unchanged
- [x] 3.5 Add `s2c-fixture ladybug-db <fixture.yaml> <out.lbug>` (testkit feature `ladybug`) that materializes a fixture's LPG projection into a LadybugDB file, so binding tests can create databases without write APIs

## 4. Python binding

- [x] 4.1 Create `crates/s2c-python` (workspace member, `publish = false`, features `neo4j`/`ladybug`/`remote-imports` off by default) with `pyproject.toml` (maturin, abi3-py39, dynamic version, `[tool.maturin] features` enabling all three)
- [x] 4.2 Implement `_native.compile` with option parsing (`ValueError` for invalid enum values), `Source` documents, schema as str or dict, `allow_threads` around compilation, and `Compilation` (`manifest`, `manifest_json`, `cypher`, `static_diagnostics`, `write`)
- [x] 4.3 Define the exception hierarchy (`Shacl2CypherError`, `CompileError.errors`, `ManifestError`, `DatabaseConnectionError`, `BackendUnavailableError`, `DatabaseClosedError`) and `load_manifest`
- [x] 4.4 Implement the worker-thread `Database` core (request channel, one-shot replies, serialized calls, close/drop joins thread) shared by `Neo4j` and `Ladybug` classes, with context-manager support
- [x] 4.5 Implement `validate` (shapes or manifest, `limit`/`sample_size`/`timeout` checks), `schema()`/`schema_json()`, and `Report` (`conforms`, `complete`, `summary`, `rules`, `to_dict`, `to_json`, `render`, `exit_code`)
- [x] 4.6 Write `python/shacl2cypher/__init__.py` re-exports, `__version__`, `available_backends`, `_types.py` TypedDicts and `py.typed`
- [x] 4.7 pytest suite covering each `language-bindings` scenario applicable to Python: compile parity vs CLI output, errors, diagnostics, manifest loading, LadybugDB validate via `s2c-fixture ladybug-db`, GIL release, concurrent calls, closed handle, backend-unavailable (compile-only build), Neo4j tests gated on `S2C_NEO4J_URI`
- [x] 4.8 `mypy --strict` check of an example program in `crates/s2c-python/examples/`, and a test that `_types.py` TypedDict keys match serialized manifest/report fixtures

## 5. Node binding

- [x] 5.1 Create `crates/s2c-node` (workspace member, `publish = false`, same features) with napi-rs `tokio_rt` and `serde-json`, `package.json` (napi config, Node ≥ 18, `optionalDependencies` on three platform packages), and `npm/<platform>/package.json` files
- [x] 5.2 Implement native `compile` (spawn_blocking) and `compileSync`, option validation, document sources, schema as string or object, and the compilation result with `write`
- [x] 5.3 Implement native `Database` over the worker thread with `Neo4j.connect`, `Ladybug.open`, `validate`, `schema`, `close`, awaiting one-shot replies without blocking libuv threads
- [x] 5.4 Implement native `renderReport`, `exitCode`, `loadManifest`, `availableBackends`, `version`, and error codes (`S2C_COMPILE`, `S2C_MANIFEST`, …)
- [x] 5.5 Write `index.js` (CommonJS) and `index.mjs` (ESM) loaders that pick the platform package, throw a clear unsupported-platform error, and rethrow native errors as exported error classes
- [x] 5.6 Write `index.d.ts` with option, compilation, manifest, report, schema snapshot, database and error types
- [x] 5.7 `node:test` suite covering each `language-bindings` scenario applicable to TypeScript, including event-loop responsiveness during a slow query, ESM and CommonJS imports, and Neo4j tests gated on `S2C_NEO4J_URI`
- [x] 5.8 `tsc --strict --noEmit` check of an example program in `crates/s2c-node/examples/`, and a test that `index.d.ts` shapes match serialized manifest/report fixtures

## 6. Cross-binding parity

- [x] 6.1 Add a shared parity fixture set (a subset of conformance fixtures plus one multi-file and one import case) under `tests/bindings/`
- [x] 6.2 Parity script run by both test suites: build the CLI, compile each fixture via CLI, Python and Node, and compare `manifest.json`/`queries.cypher` byte for byte
- [x] 6.3 Report parity: validate a LadybugDB fixture via CLI (`--format json|table|junit|sarif`) and via both bindings' render, comparing with `durationMs` normalized
- [x] 6.4 Schema round trip: `db.schema()` output from each binding compiles to the same manifest as CLI `schema dump` output
- [x] 6.5 Key-shape check: serialize manifest, report and schema fixtures and assert the TypedDicts (4.8) and `.d.ts` types (5.8) list exactly those keys

## 7. CI

- [x] 7.1 Add `python` job to `ci.yml`: Rust 1.87, Python 3.9 and 3.13, `maturin develop` with all features, pytest, mypy example check; share the LadybugDB rust-cache key
- [x] 7.2 Add a compile-only Python build step asserting `available_backends() == []` and `BackendUnavailableError`
- [x] 7.3 Add `node` job: Node 18 and 22, `napi build` with all features, `node --test`, `tsc` example check
- [x] 7.4 Run the Neo4j-gated binding tests in the existing `neo4j` job (or a sibling job with the same service container)

## 8. Release

- [x] 8.1 Add `check-version` job to `release.yml` comparing the tag with the workspace version
- [x] 8.2 Add wheel matrix via `PyO3/maturin-action`: Linux x86_64 and arm64 in `manylinux_2_28` with cmake, macOS arm64; sdist job; smoke-test each wheel (import, `available_backends`, compile a fixture, validate LadybugDB in the manylinux image)
- [x] 8.3 Add npm build matrix via `@napi-rs/cli` for the three targets in matching environments; set package versions from the workspace version; smoke-test each addon
- [x] 8.4 Add `publish` job needing all build and smoke jobs: PyPI trusted publishing, then npm platform packages and main package with `--provenance` using `NPM_TOKEN`
- [ ] 8.5 Dry-run the release workflow on a pre-release tag in a fork or with publishing disabled, and confirm artifacts install on each target

## 9. Documentation

- [x] 9.1 README: Python and TypeScript install, compile, validate and report examples; note camelCase data keys, platform support, compile-only source builds
- [x] 9.2 Add `lat.md/bindings.md` (architecture of binding crates, worker-thread handles, error mapping, data model, packaging and release) and update `lat.md/architecture.md#Crates`, `#Input Assembly` (in-memory documents), `#CLI` (runner glue)
- [x] 9.3 Add binding test specs to `lat.md/tests.md` with `@lat:` references from pytest and `node:test` tests; run `lat check` until it passes
