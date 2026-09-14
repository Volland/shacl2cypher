# Architecture

Rust cargo workspace with a pure compiler core, a CLI, and an optional database runner behind feature flags.

## Crates

The workspace separates deterministic compilation from database I/O so the core stays testable and embeddable.

- `shacl2cypher-core`: input assembly, shapes AST, resolution, IR, dialect renderers, manifest. No database I/O.
- `shacl2cypher`: the `shacl2cypher` binary with `compile`, plus `validate` and `schema dump` when runner features are enabled.
- `shacl2cypher-runner`: executes manifests and introspects schemas; cargo features `neo4j` (Bolt via `neo4rs`) and `ladybug` (embedded `lbug` bindings).
- `s2c-testkit`: test-only, unpublished; loads conformance fixtures and projects them to RDF and LPG load scripts — see [[testing#Conformance Fixtures]].

The crates are published to crates.io as `shacl2cypher-core`, `shacl2cypher-runner` and `shacl2cypher` (the CLI, installable with `cargo install --locked shacl2cypher --features neo4j,ladybug`; `--locked` is needed on Rust 1.87), under the MIT license; `s2c-testkit` is never published. Directories keep their `crates/s2c-*` names. The runner's integration tests are excluded from its package because they need repository fixtures.

Pushing a `v*` tag runs `.github/workflows/release.yml`. It creates the GitHub release and attaches `shacl2cypher` binaries with both backends for Linux x86_64, Linux arm64 and macOS arm64, each with a SHA-256 checksum.

The workspace declares `rust-version = "1.87"` and uses Cargo's `incompatible-rust-versions = "fallback"` resolver so dependency versions stay compatible with that toolchain. Throwaway spikes live in `spikes/`, excluded from the workspace.

## Input Assembly

All input shape files are parsed into one union graph before building the AST, following SHACL's shapes-graph semantics.

- Formats by extension: `.ttl` Turtle, `.nt` N-Triples, `.trig` TriG (named graphs unioned) via `oxttl`/`oxrdf`; JSON-LD later. Other extensions are errors.
- Input paths are canonicalized, de-duplicated and sorted before loading, so argument order never changes the result.
- Blank nodes are renamed per file to deterministic ids (`f{file}b{n}`), so labels never clash across files.
- Each file is parsed with its `file://` URL as base IRI; prefix declarations are collected for rule-name compaction, first file in order winning on conflict.
- Every triple records each file and line it was read from. `oxttl` exposes no positions for parsed triples, so input is fed line by line and a triple is attributed to the line where the parser completed it; syntax errors carry the parser's exact line and column.
- `owl:imports` are followed until no new input appears, in IRI order for determinism. Relative imports resolve against the importing file's `file://` base, and cycles terminate because each canonical path loads once.
- `http(s)` imports require `--allow-remote-imports`; bytes come from a caller-supplied `RemoteFetcher` so the core crate performs no network I/O. Remote documents are parsed by extension, defaulting to Turtle.
- Every input (file or remote IRI) records the SHA-256 of the bytes parsed, for manifest provenance. Import failures point to the `file:line` of the `owl:imports` triple.
- Conflicting single-valued settings across files (e.g. two `sh:severity` values or two `s2c:key`s on one shape) are compile errors listing both source spans.

## Shapes AST

The project owns its SHACL AST rather than reusing an external one, because it must carry source spans, resolved defaults and `s2c:` annotations.

Shape recognition follows SHACL. A node is a shape if it is typed `sh:NodeShape` or `sh:PropertyShape`, declares a target or constraint parameter, is the object of `sh:property`, `sh:node`, `sh:not` or `sh:qualifiedValueShape`, or is a member of an `sh:and`/`sh:or`/`sh:xone` list. A shape with `sh:path` is a property shape.

- Implicit class targets: a shape IRI that is also typed `rdfs:Class` targets its own instances.
- Paths: predicate IRI, `sh:inversePath`, RDF-list sequence, `sh:alternativePath`, `sh:zeroOrMorePath`, `sh:oneOrMorePath`, `sh:zeroOrOnePath`.
- Defaults: severity `sh:Violation`, not deactivated.
- Every constraint keeps the source location of its parameter triple.
- Parsing collects all problems instead of stopping at the first: malformed values (e.g. `sh:minCount "two"`), ill-formed or cyclic RDF lists, and multiple values for single-valued parameters. Each problem carries `file:line`, e.g. `shapes/person.ttl:42`.
- Constructs the compiler later rejects (`sh:sparql`, `sh:targetNode`, `sh:languageIn`, `sh:uniqueLang`) are still parsed into the AST, so rejection or `--lenient` reporting can point at their location.
- Conflicting values for single-valued settings are caught by the same single-value check over the union graph, so settings split across files report every location.
- Recursive shape references are detected over `sh:property`, `sh:node`, `sh:not`, `sh:and`/`sh:or`/`sh:xone` and `sh:qualifiedValueShape` edges. Each group of mutually recursive shapes is reported once, as its shortest cycle starting from the group's smallest shape id, with the location of every step, because inlining `conforms` cannot terminate on a cycle ([[semantics#Violations and Conforms]]).

Multi-valued settings (`sh:property`, repeated constraints, targets, messages) are ordered by content after parsing: named shapes by IRI, blank shapes by their expanded content. Source positions and blank-node labels never influence compiled output. RDF lists keep their order.

## CLI

`shacl2cypher compile` turns shapes files into `manifest.json` and `queries.cypher` in the `-o` directory (default: the working directory).

Options: `--dialect neo4j|ladybug` (required), `--schema`, repeatable `--ontology`, `--node-key`, `--neo4j-labels explicit|inherited`, `--strict`, `--lenient`, `--verbose`, `--max-path-depth` (default 10), `--allow-remote-imports`, `--fail-on-schema-mismatch`.

- Manifest input paths are relative to the working directory.
- Compile errors print one `error:` line per problem and exit 1, writing no files. Usage errors exit 2.
- Static diagnostics print as `warning: file:line: code: message` and do not fail the build without `--fail-on-schema-mismatch`.
- Remote imports are fetched over HTTP(S) by the CLI (30 s timeout, 16 MB cap), so the core stays free of network I/O.

## Runner

The runner executes a manifest's queries against Neo4j or LadybugDB, drills into failing rules, and reports results for humans and CI.

- `shacl2cypher validate` compiles shapes in memory for the connected dialect, or loads `--manifest`. A manifest compiled for another dialect is rejected.
- On LadybugDB without `--schema`, the schema is dumped from the database before compiling.
- LadybugDB is embedded: the runner opens database files directly and may conflict with an application holding the lock.

### Validate Flow

Every rule with queries runs its summary query; rules with violations then run their detail query. Rules without queries are reported from their manifest status.

- Parameters: `$limit` from `--limit` (default 100, `0` lists every violation, sent as the largest 64-bit integer) and `$sampleSize` from `--sample-size` (default 5).
- `--timeout <seconds>` bounds each query. A timed-out summary gives `status: timeout`; a failed query gives `status: error`. Remaining rules still run.
- A detail query that times out or fails after a positive count keeps `status: failed`, with a `statusReason` saying the violations are not listed.
- Statuses: `passed`, `failed`, `timeout`, `error`, `skipped` (deactivated, unsupported, schema-mismatch), `guaranteed-by-schema`. Each rule records `durationMs`.
- The report has `conforms` (no rule found violations) and `complete` (every query finished).

### Exit Codes

The exit code tells CI whether rules at or above `--fail-on` failed, and whether the result can be trusted.

- `0`: no rule at or above `--fail-on` (`violation` default, `warning`, `info`) has violations, and every query completed.
- `1`: a rule at or above the threshold has violations. Custom severities count as violations.
- `2`: usage or setup errors: bad arguments, compile errors, unreachable databases, or no backend compiled in.
- `3`: no failing rule, but a query timed out or failed, so conformance is unknown.

### Reports

`--format` selects a terminal table, JSON, JUnit XML or SARIF 2.1.0; `-o` writes the report to a file instead of stdout.

- Table: one line per rule with status, severity, count and time, up to 10 violations each, then totals.
- JSON: the full report, including `durationMs` per rule and every detail row.
- JUnit: one test case per rule. Violations are failures, timeouts and query errors are errors, and skipped rules are skipped. Cases carry the shapes file and line.
- SARIF: every rule is a tool rule descriptor; each detail row is a result at the constraint's shapes file and line, with focus, value and details as properties. Timeouts and errors become tool execution notifications.

### Backends

Backends are cargo features of `shacl2cypher-runner` and `shacl2cypher`; without them `validate` and `schema dump` report that no database backend is available.

- `neo4j` uses `neo4rs` over Bolt (`--connect`, `--user`/`NEO4J_USER`, `--password`/`NEO4J_PASSWORD`, `--database`). The driver is pinned to `0.9.0-rc.10`: 0.8 decodes the integers −16…−1 as 240…255, which the literal fuzz test caught. Read-mode transactions are unstable in that driver, so every query runs in an explicit transaction that is always rolled back, and nothing a query does persists. A timed-out query abandons its connection pool. Rows are read as Bolt values and converted to JSON explicitly, so report values never depend on the driver's serde mapping.
- `ladybug` uses the embedded `lbug` crate (`--ladybug <file>`). Databases open with `read_only(true)`, and a missing file is an error rather than a new database. Timeouts use the connection's native query timeout.

### Schema Dump

`shacl2cypher schema dump` writes a snapshot that `compile --schema` accepts unchanged; both backends emit neutral type names.

- LadybugDB: `show_tables`, `table_info` and `show_connection` give node and rel tables, declared column types and FROM/TO pairs. `TIMESTAMP` becomes `LOCAL_DATETIME`, `BOOL` becomes `BOOLEAN`, `T[]` becomes `LIST<T>`, and unknown types become `ANY`.
- Neo4j: `db.labels`, `db.relationshipTypes` and the `db.schema.nodeTypeProperties`/`relTypeProperties` procedures give observed property types. Several types, or differing types across label combinations, become `ANY`.
- Neo4j endpoints come from one distinct scan per relationship type (`MATCH (a)-[:T]->(b)` over label pairs). `db.schema.visualization` returns virtual endpoints without labels. The scan reads every relationship, and types without relationships are omitted.
