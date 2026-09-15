# Bindings

Python and Node.js packages that compile shapes, validate databases and dump schemas in-process, with results identical to the CLI.

Both are thin layers over `shacl2cypher-core` and `shacl2cypher-runner`: they convert values, errors and threads, and keep no compile or validate logic of their own.

## Crates and Packages

Two workspace crates, never published to crates.io, build the packages; their backend features are off by default so workspace builds stay free of C++.

- `crates/s2c-python` (PyO3 0.29, abi3 for CPython 3.9+) builds the `shacl2cypher` PyPI package with the `shacl2cypher._native` extension module under a typed Python layer in `python/shacl2cypher/`.
- `crates/s2c-node` (napi-rs 3.4) builds the `shacl2cypher` npm package: the native addon plus `index.js`, `index.mjs` and `index.d.ts`.
- Both declare features `neo4j`, `ladybug` and `remote-imports`, forwarded to the runner. `pyproject.toml` and the npm `build` script turn all three on.
- Feature flags given to `maturin` replace the `pyproject.toml` list, so `--no-default-features -F remote-imports` builds a compile-only package.

Dependency versions are pinned for Rust 1.87 — see [[architecture#Crates]].

## Shared Runner Glue

The CLI and both bindings share one implementation of remote fetching, backend opening and the validate-flow rules, all in the runner crate.

- `remote::HttpFetcher` (feature `remote-imports`) fetches `owl:imports` with a 30 s timeout and a 16 MB cap.
- `backend::open` turns a [[crates/s2c-runner/src/backend.rs#BackendConfig]] into an executor; `BackendError` separates backends missing from the build from connection failures.
- `session::compile_for` compiles shapes for a connected database, dumping the LadybugDB schema when none is given; `check_dialect` rejects manifests of another dialect.
- `session::Sources` owns shapes, documents and schema text so compilation can run on another thread; name parsers accept the CLI's option values.

In-memory shape documents are a core feature — see [[architecture#Input Assembly]].

## Database Worker

A [[crates/s2c-runner/src/worker.rs#DatabaseWorker]] owns one executor on its own thread, because executors are not `Send` and bindings must not block their host runtime.

- Opening runs on the worker thread; the caller waits for success or the `BackendError`.
- Jobs (validate, schema dump) queue on a channel and run one at a time; each job hands its result to a reply callback on the worker thread.
- Python waits on a blocking reply with the GIL released. Node awaits a tokio oneshot, so no libuv thread is held while queries run.
- `close` stops accepting jobs, lets queued jobs finish and joins the thread; later jobs fail with `WorkerError::Closed`.

## Data Model

Manifests, reports and schema snapshots keep the camelCase keys of their JSON files in both languages, so one documented data model serves the CLI and bindings.

Python returns dicts described by `TypedDict`s in `_types.py`; Node returns plain objects described by `index.d.ts`. Tests check both declarations against serialized data.

Byte parity with the CLI holds by construction: manifest JSON comes from `Compilation::manifest_json`, and a schema passed as an object is re-serialized with `session::snapshot_text`, the `schema dump` text, so it hashes like the dumped file. Reports deserialize again, so `render` and exit codes work on reports loaded from JSON.

## Errors

Both bindings expose `Shacl2CypherError` with `CompileError` (carrying `errors`), `ManifestError`, `DatabaseConnectionError`, `BackendUnavailableError` and `DatabaseClosedError`.

- Python creates the classes with `pyo3::create_exception!`; invalid option values raise `ValueError`.
- Node's addon throws errors whose message is JSON `{"s2c": kind, "message", "errors"}`; `index.js` rethrows them as the exported classes, invalid option values as `TypeError` and a non-positive `timeoutMs` as `RangeError`.
- Per-rule timeouts and query errors are report statuses, as in the CLI, never exceptions.

## Python API

`compile`, `load_manifest`, `Neo4j`/`Ladybug` database handles with `validate`, `schema` and `schema_json`, and `Report` with `render` and `exit_code`.

Functions and keyword arguments are snake_case. `timeout` is in seconds; `limit` of `0` or `None` lists every violation. Handles are context managers, and every native call releases the GIL.

## Node API

`compile` (Promise) and `compileSync`, `loadManifest`, `Neo4j.connect`/`Ladybug.open` returning a `Database`, and `renderReport`/`exitCode` over plain report objects.

Options are camelCase objects and unknown keys are rejected. Values cross the addon boundary as JSON text, which avoids napi's float conversion of integers. `timeoutMs` is in milliseconds; `limit` of `0` or `null` lists every violation.

The loader picks the `shacl2cypher-<platform>` package (or a local `shacl2cypher.<platform>.node`) and names unsupported platforms, including musl Linux.

## Packaging

Wheels and npm addons cover Linux x86_64 and arm64 on glibc 2.28+ and macOS 13.3+ arm64, each with both database backends; the packages share the workspace version.

- PyPI: one abi3 wheel per platform plus an sdist whose default build includes both backends (cmake and a C++ compiler needed).
- npm: `shacl2cypher` lists `shacl2cypher-linux-x64-gnu`, `shacl2cypher-linux-arm64-gnu` and `shacl2cypher-darwin-arm64` as optional dependencies; npm installs only the matching one.

## Release

Pushing a `v*` tag builds every wheel, the sdist and every addon, smoke-tests them, and publishes to PyPI and npm only when all of them succeeded.

- `check-version` fails the release when the tag differs from the workspace version, or when `crates/s2c-node/scripts/version.js check` finds an npm `package.json` on another version.
- One `bindings` job per platform builds the wheel and the addon from one `target/`, so LadybugDB's C++ compiles once. Linux jobs run in `manylinux_2_28` images that install Rust 1.87, cmake and Node 22, giving both packages the glibc 2.28 baseline.
- `lbug` always links `ssl` and `crypto`. Release jobs, including the CLI binaries, build a static-only OpenSSL with `scripts/ci/static-openssl.sh` and point `OPENSSL_DIR` at it, so no artifact needs OpenSSL at runtime. `scripts/ci/check-no-dynamic-openssl.sh` fails the job otherwise.
- `MACOSX_DEPLOYMENT_TARGET` is 13.3 because `lbug`'s C++ uses `std::format`; maturin's default of 11.0 fails to compile it.
- Each job smoke-tests its artifacts with `tests/bindings/smoke.py` and `smoke.js`: backends, a compile, and validation of an `s2c-fixture ladybug-db` database. The `sdist` job installs its archive as a compile-only build (`MATURIN_PEP517_ARGS`) and smoke-tests that.
- `publish` needs every build and uploads wheels and sdist with PyPI trusted publishing, then the npm platform packages and the main package with `NPM_TOKEN` and provenance.

`scripts/release.sh <version>` drives a release end to end:
- It checks that the tree is clean on a synced `main`, the tag is new, and the `release` environment and its `NPM_TOKEN` exist.
- It bumps the workspace version, crate requirements, npm packages, Cypher snapshot headers and `Cargo.lock`, then runs fmt, clippy and tests.
- It commits, pushes, waits for CI, and asks before pushing the tag. It then waits for the Release workflow.
- Options: `--publish-crates` also runs `cargo publish` for the three crates; `--dry-run` only checks and prints the plan.

## Testing

Each binding has its own suite that runs against the CLI and `s2c-fixture ladybug-db` databases built from the conformance fixtures.

- Python: pytest in `crates/s2c-python/tests`, plus `mypy --strict` over `examples/typed_usage.py`. Specs: [[tests#Python Binding]].
- Node: `node:test` in `crates/s2c-node/test`, plus `tsc --strict` over `examples/typed-usage.ts`. Specs: [[tests#Node Binding]].
- Binaries come from `target/debug` or `S2C_CLI` and `S2C_FIXTURE`; Neo4j tests run when `S2C_NEO4J_URI` is set.
