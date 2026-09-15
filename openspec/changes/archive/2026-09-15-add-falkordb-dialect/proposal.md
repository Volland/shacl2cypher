## Why

shacl2cypher validates graphs only on Neo4j and LadybugDB. FalkorDB is a widely used Redis-based property graph database with its own Cypher dialect, and teams running it cannot validate their data against SHACL shapes today. Because its Cypher differs from Neo4j's (different subquery, type-predicate and schema support), it needs its own renderer rather than reusing Neo4j queries.

## What Changes

- New compile target `--dialect falkordb` that renders every rule kind the other dialects support: node-focused shapes (class, subjects-of and objects-of targets), relationship-focused shapes (`s2c:targetRelationship`), all Tier 1 constraints, and Tier 2 shape composition and complex paths. Constructs FalkorDB cannot express are compile errors (or `unsupported` rules with `--lenient`), never silently wrong queries.
- FalkorDB-specific literal, identifier, datatype and regex rendering, with the same null-safety and read-only guarantees as the existing dialects.
- New runner backend behind a `falkordb` cargo feature on `shacl2cypher-runner` and `shacl2cypher`: `validate` and `schema dump` against a FalkorDB server, with read-only execution, per-query timeouts and the existing report formats and exit codes.
- CLI options to connect to FalkorDB (server URL, graph name, credentials).
- The conformance fixtures, literal round-trip and renderer tests run against FalkorDB, with a FalkorDB service in CI.
- Not included: Python and Node.js binding handles for FalkorDB and their packaging. `compile(dialect="falkordb")` works in the bindings through the shared core, but opening a FalkorDB database from them comes in a later change.

## Capabilities

### New Capabilities
<!-- None: FalkorDB support extends the existing rendering, snapshot and runner capabilities. -->

### Modified Capabilities
- `dialect-rendering`: supported dialects gain `falkordb`, with equivalent diagnostic results to Neo4j and LadybugDB on the same fixtures.
- `schema-snapshot`: the snapshot is optional for FalkorDB, and `schema dump` can read a snapshot from a FalkorDB graph.
- `validation-runner`: `validate` runs against a FalkorDB graph, and `falkordb` is an optional driver feature.

## Impact

- `crates/s2c-core`: new `Dialect::FalkorDb` and a FalkorDB renderer module. Dialect-dependent code (compile options, literal rendering, depth limits, determinism and snapshot tests) is extended.
- `crates/s2c-runner`: new `falkordb` feature, executor, `BackendConfig` variant, dialect name parsing and schema introspection.
- `crates/s2c-cli`: `--dialect falkordb` and FalkorDB connection flags for `validate` and `schema dump`.
- `crates/s2c-testkit`: FalkorDB load script and `falkordb` engine in fixtures (`dialects`, `known_differences`).
- New dependency: a FalkorDB (Redis protocol) Rust client, pinned to support Rust 1.87.
- CI: a FalkorDB service container job. Release binaries gain the `falkordb` feature.
- Docs: `lat.md/dialects.md`, `architecture.md`, `mapping.md` and `testing.md`; README.
