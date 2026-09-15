## Context

The proposal explains the motivation, and `specs/language-bindings` and `specs/binding-packaging` hold the requirements. Four facts about the current code shape the approach:

- **Core compiles from disk only.** `shacl2cypher_core::compile::CompileRequest` takes `&[PathBuf]` for shapes and ontologies. `ShapesGraph::load_with` canonicalizes and reads those files, and manifest input paths are made relative to `CompileOptions::base_dir`.
- **The CLI owns glue the bindings also need.** `crates/s2c-cli/src/main.rs` holds the `ureq` `HttpFetcher` for remote imports, `open_backend` (feature-gated Neo4j/LadybugDB selection), and the validate flow's rules: read the schema from LadybugDB when none is given, and reject a manifest from another dialect.
- **`Executor` is not `Send`.** `shacl2cypher_runner::executor::Executor` is a `&mut self` trait without a `Send` bound. `Neo4jExecutor` owns a current-thread tokio runtime, and `LadybugExecutor` wraps embedded `lbug` handles.
- **Toolchain limits.**
  - The workspace pins Rust 1.87 with the `fallback` MSRV resolver.
  - The LadybugDB backend compiles C++ and needs cmake.
  - CI runs `cargo clippy/test --workspace` without backend features.
  - Release binaries target Linux x86_64 and arm64 plus macOS arm64.

## Goals / Non-Goals

**Goals:**
- Thin bindings. All compile, validate, report and schema logic stays in the Rust crates, and the bindings only convert types, errors and threads.
- Output identical to the CLI for the same inputs, enforced by tests.
- Idiomatic calling conventions in each language, over data shapes identical to the JSON artifacts.
- Adding the bindings must not slow the existing fmt/clippy/test job or require Python or Node in it.

**Non-Goals:**
- A browser/WASM build, Deno or Bun support guarantees, Windows, musl Linux, and Intel macOS.
- A Python `asyncio`-native API. Releasing the GIL makes `asyncio.to_thread` sufficient.
- Streaming violations row by row, or cancelling a validation in progress (the per-query timeout remains the bound).
- Exposing the AST, IR or renderers. Only the CLI-level operations are bound.
- Publishing the binding crates to crates.io.

## Decisions

### 1. Two binding crates on top of core and runner

```
crates/s2c-core ──► crates/s2c-runner ──┬──► crates/s2c-cli     (binary)
                                        ├──► crates/s2c-python  (PyO3 cdylib → wheel)
                                        └──► crates/s2c-node    (napi-rs cdylib → npm)
```

`s2c-python` and `s2c-node` are workspace members with `publish = false`. Each has cargo features `neo4j`, `ladybug` and `remote-imports`, forwarded to the runner and all off by default. `pyproject.toml` (`[tool.maturin] features`) and the napi build script turn them on. This keeps the workspace jobs free of C++ builds, while packaged builds get both backends.

*Alternatives:*
- uniffi, one generator for both languages: rejected. Its TypeScript/Node support is immature, and it would impose uniffi's object model on an API that is mostly JSON-shaped data.
- A wrapper that spawns the CLI: rejected. Every call needs a subprocess and temp files, and typed errors and non-blocking handles are awkward.
- WASM for TypeScript: rejected by the user in favor of napi-rs, because WASM cannot embed LadybugDB or open Bolt sockets.

### 2. In-memory documents in core, not temp files

`CompileRequest` gains `shape_documents` and `ontology_documents: &[Document]`, where `Document { name, text, format: Option<RdfFormat> }`. `ShapesGraph` gains an `InputSource::Document(PathBuf)` whose path is `base_dir.join(name)` (lexically normalized, not canonicalized). Base IRI, relative imports, sorting, blank-node file ids, SHA-256 and manifest paths therefore all follow the file code path. Names colliding with each other, or with a canonicalized file, are rejected (see `specs/shapes-loading`). The existing `shapes: &[PathBuf]` fields stay, so the change is additive for Rust users and the CLI.

*Alternative:* the bindings write documents to a temp directory. Rejected: manifest `inputs` paths would point into the temp directory, relative imports would break, and the files would need cleanup.

### 3. Shared glue moves into the runner

New runner items, which the CLI is refactored onto with no behavior change:

- `runner::remote::HttpFetcher` (feature `remote-imports`, brings `ureq`), keeping the 30-second timeout and 16 MB cap.
- `runner::backend::{BackendConfig, open, available_backends}`, where `BackendConfig` is `Neo4j { uri, user, password, database }` or `Ladybug { path }`. It returns `Box<dyn Executor>` or `BackendError::{Unavailable, Connection}`.
- `runner::session::manifest_for(executor, Source::Shapes{..} | Source::Manifest(..), options)` holds the validate-flow rules: read the schema from LadybugDB when none is given, and reject a manifest from another dialect.

The CLI keeps argument parsing, printing and exit codes. Existing CLI integration tests guard the refactor.

### 4. Data crosses the boundary as serde values with the JSON field names

`Manifest`, `Report` and `SchemaSnapshot` already derive `Serialize` with camelCase names. Python converts with `pythonize` into dicts and lists, and Node converts with napi's `serde-json` feature into plain objects. Report rendering and exit codes take the value back through `serde` into `Report`, so `render`/`exit_code` accept a report a user loaded from JSON. Manifest JSON text comes from `Compilation::manifest_json`, so byte parity with the CLI holds by construction.

The Python API is snake_case for functions and keyword arguments but keeps camelCase inside data dicts (`rule["ruleId"]`). One documented data model across the CLI, JSON files, Python and TypeScript was preferred over Pythonic keys that would diverge from `manifest.json`. `TypedDict`s in `shacl2cypher/_types.py` and `index.d.ts` describe these shapes. They are hand-written and checked against serialized fixtures by tests (task 6.5).

*Alternative:* generated Python classes per struct. Rejected: large surface area, drift from the JSON, and no benefit for users who already work with JSON reports.

### 5. Each database handle runs on a dedicated worker thread

A `Database` handle spawns one OS thread that owns the `Box<dyn Executor>` and serves requests (`Validate`, `Schema`, `Close`) from an `mpsc` channel. Each request carries a one-shot reply channel. This removes any need for `Executor: Send`, serializes concurrent calls on a handle as the spec requires, and gives a clear place to implement close: drain, drop the executor, join the thread. Calls after close return `DatabaseClosedError`.

- **Python:** methods send the request, then wait on the reply inside `Python::allow_threads`, releasing the GIL. `compile` runs inline inside `allow_threads`.
- **Node:** napi-rs `async fn` exports (feature `tokio_rt`) await a `tokio::sync::oneshot` reply, so no libuv pool thread blocks while a query runs. `compile` uses `tokio::task::spawn_blocking`, and `compileSync` runs on the calling thread.

*Alternatives:*
- Wrapping executors in a `Mutex` and requiring `Send`: rejected, because it forces changes to the backend types and holds libuv or GIL threads for the duration of queries.
- napi `AsyncTask` blocking on the channel: rejected, because it occupies one of 4 default libuv threads per running validation.

### 6. Error mapping

| Rust | Python | TypeScript |
|---|---|---|
| `CompileError(Vec<String>)` | `CompileError.errors: list[str]` | `CompileError.errors: string[]` |
| manifest parse/version/dialect | `ManifestError` | `ManifestError` |
| `BackendError::Connection`, `ExecError::Connection` | `DatabaseConnectionError` | `DatabaseConnectionError` |
| `BackendError::Unavailable` | `BackendUnavailableError` | `BackendUnavailableError` |
| closed handle | `DatabaseClosedError` | `DatabaseClosedError` |
| invalid option value | `ValueError` | `TypeError` |

In Python every error except `ValueError` subclasses `Shacl2CypherError`, created with `pyo3::create_exception!`. In Node, the native layer throws an `Error` whose `code` is `S2C_COMPILE`, `S2C_MANIFEST` and so on, with the message list JSON-encoded in `message`. A small handwritten `index.js`/`index.mjs` wrapper rethrows these as exported error classes that extend `Shacl2CypherError`. Per-rule query errors and timeouts are not raised: they are statuses in the report, as in the CLI.

### 7. API surface

```python
# Python
compile(shapes, *, dialect, schema=None, ontologies=(), node_key=None, neo4j_labels="explicit",
        strict=False, lenient=False, verbose=False, max_path_depth=10,
        allow_remote_imports=False, fail_on_schema_mismatch=False, base_dir=None) -> Compilation
Source(name: str, text: str, format: Literal["turtle","ntriples","trig"] | None = None)
Compilation.manifest / .manifest_json / .cypher / .static_diagnostics / .write(dir)
load_manifest(text_or_dict) -> Manifest dict
Neo4j(uri, user="neo4j", password="", database=None); Ladybug(path)  # context managers
db.dialect / db.validate(shapes=None, *, manifest=None, limit=100, sample_size=5, timeout=None, **compile_options) -> Report
db.schema() -> dict; db.schema_json() -> str; db.close()
Report.conforms / .complete / .summary / .rules / .to_dict() / .to_json() / .render(format) / .exit_code(fail_on="violation")
available_backends() -> list[str]; __version__
```

```ts
// TypeScript
compile(opts: CompileOptions): Promise<Compilation>; compileSync(opts): Compilation
// CompileOptions.shapes: Array<string | { name: string; text: string; format?: RdfFormat }>
loadManifest(json: string | Manifest): Manifest
Neo4j.connect({ uri, user?, password?, database? }): Promise<Database>
Ladybug.open(path: string): Promise<Database>
db.dialect; db.validate({ shapes? | manifest?, limit?, sampleSize?, timeoutMs?, ...compile options }): Promise<Report>
db.schema(): Promise<SchemaSnapshot>; db.close(): Promise<void>
renderReport(report, format): string; exitCode(report, failOn?): 0 | 1 | 3
availableBackends(): string[]; version: string
```

The timeout is `timeout` in seconds (a float) in Python, following `socket` and `requests`, and `timeoutMs` in TypeScript, following Node timer conventions. The base directory defaults to the process working directory, as in the CLI.

### 8. Layout and build tooling

- **Python** (`crates/s2c-python/`):
  - Files: `Cargo.toml`, `pyproject.toml` (maturin backend, `dynamic = ["version"]` from Cargo), `src/lib.rs` (the `shacl2cypher._native` module), `python/shacl2cypher/{__init__.py,_types.py,py.typed}`, and `tests/` (pytest).
  - PyO3 uses the `abi3-py39` feature.
- **Node** (`crates/s2c-node/`):
  - Files: `Cargo.toml`, `package.json` (napi config, `optionalDependencies` on platform packages), `index.js`, `index.mjs`, `index.d.ts`, `npm/<platform>/package.json`, and `test/` (the `node:test` runner, so no test framework dependency).
- **Versions:** dependencies are pinned to releases that build on Rust 1.87, and task 1.1 verifies this before any code is written. If a required release needs a newer compiler, only the binding crates may declare a higher `rust-version`; core, runner and CLI stay on 1.87.

### 9. Release and CI

- **`ci.yml`:**
  - Adds `python` and `node` jobs on Ubuntu. Each builds its binding with `ladybug` (sharing a rust-cache key with the existing LadybugDB job) and runs its tests.
  - The parity tests build the CLI in the same job and compare outputs.
  - Neo4j tests reuse the existing service-container pattern and run when `S2C_NEO4J_URI` is set.
- **`release.yml`:**
  - A `check-version` job compares the tag with the workspace version.
  - Build jobs:
    - Wheels via `PyO3/maturin-action`: Linux in `manylinux_2_28` containers with cmake installed; arm64 natively on `ubuntu-24.04-arm`; macOS arm64 on `macos-latest`.
    - The sdist.
    - Node addons via `@napi-rs/cli`, built in the same `manylinux_2_28` images so the glibc baseline matches.
  - Every artifact is smoke-tested on its runner: import, `available_backends`, and compile a fixture.
  - A final `publish` job needs all builds. It uploads to PyPI with trusted publishing (OIDC) and to npm with `NPM_TOKEN` and `--provenance`, platform packages first and the main package last.
  - The existing binary release jobs are unchanged.

## Risks / Trade-offs

- **[Rust 1.87 compatibility of PyO3/napi-rs releases]** → Pin the newest compatible releases (task 1.1). Allow a higher `rust-version` only on the binding crates if unavoidable, and record it in `lat.md`.
- **[LadybugDB C++ build time across 6 packaged builds]** → Share rust-cache keys per target between the wheel and npm jobs, and build both packages in one job per target, reusing one `target/` directory.
- **[glibc baseline: an addon built on a new Ubuntu fails on older systems]** → Build Linux artifacts in `manylinux_2_28` containers, and smoke-test the wheel in that image.
- **[camelCase keys in Python dicts feel foreign]** → Documented deliberately (decision 4). The `TypedDict`s give editor completion, and keyword arguments and methods stay snake_case.
- **[Worker thread per handle: many open handles mean many threads]** → Handles are expected to be few and long-lived. Document "reuse a handle", and threads exit on close or drop.
- **[Embedded LadybugDB lock conflicts when the host app holds the database]** → Same limitation as the CLI. The error surfaces as `DatabaseConnectionError` with the engine message, and is documented.
- **[Package names `shacl2cypher` taken on PyPI or npm]** → Checked in task 1.2 before publishing is wired. A fallback name changes only the install command, not the import name or API.
- **[Report timings make parity tests flaky]** → Parity tests compare reports with `durationMs` fields normalized, and compile outputs byte for byte.

## Migration Plan

- The change is additive. Rust users see a new optional `CompileRequest` field (constructed with `..` defaults via a new `CompileRequest::new` helper, so existing literal constructions keep compiling). CLI behavior is unchanged and covered by the existing CLI tests.
- Roll out in order: core documents → runner glue and CLI refactor → Python → Node → CI → release. Every step is mergeable on its own.
- Rollback: binding releases can be yanked on PyPI or deprecated on npm independently of the Rust binaries. Reverting the release workflow jobs does not affect the binary release.

## Resolved During Implementation

- `CompileRequest` gained public fields and derives `Default` instead of a `CompileRequest::new` constructor. Code building it with a struct literal must add `..CompileRequest::default()`, which is a breaking change for Rust users, so the first binding release is `0.2.0`.
