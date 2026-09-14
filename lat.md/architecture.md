# Architecture

Rust cargo workspace with a pure compiler core, a CLI, and an optional database runner behind feature flags.

## Crates

The workspace separates deterministic compilation from database I/O so the core stays testable and embeddable.

- `s2c-core`: input assembly, shapes AST, resolution, IR, dialect renderers, manifest. No database I/O.
- `s2c-cli`: the `shacl2cypher` binary with `compile`, plus `validate` and `schema dump` when runner features are enabled.
- `s2c-runner`: executes manifests and introspects schemas; cargo features `neo4j` (Bolt via `neo4rs`) and `ladybug` (embedded bindings).

## Input Assembly

All input shape files are parsed into one union graph before building the AST, following SHACL's shapes-graph semantics.

- Formats: Turtle, N-Triples, TriG (graphs unioned) via `oxttl`/`oxrdf`; JSON-LD later.
- Blank nodes are scoped per file so labels never clash across files.
- `owl:imports` follows local paths relative to the importing file with cycle detection; HTTP imports require `--allow-remote-imports` and are recorded with digests in the manifest.
- Conflicting single-valued settings across files (e.g. two `sh:severity` values or two `s2c:key`s on one shape) are compile errors listing both source spans.

## Shapes AST

The project owns its SHACL AST rather than reusing an external one, because it must carry source spans, resolved defaults and `s2c:` annotations.

Parsing handles RDF lists (`sh:in`, `sh:or`, …), blank-node path structures, implicit class targets and default severity. Errors reference file and span, e.g. `shapes/person.ttl:42`.

## Runner

The runner compiles in memory, executes summary queries, drills into failing rules with detail queries, and produces reports.

- Reports: terminal table, JSON, JUnit (one test case per rule) and SARIF (results linked to `.ttl` spans).
- Exit code by severity threshold: `--fail-on violation|warning|info`.
- Per-query `--timeout`; timeouts produce `status: timeout` results, not crashes. Per-rule timings appear in reports.
- LadybugDB is embedded: the runner opens database files directly and may conflict with an application holding the lock.
