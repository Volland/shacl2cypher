# Architecture

Rust cargo workspace with a pure compiler core, a CLI, and an optional database runner behind feature flags.

## Crates

The workspace separates deterministic compilation from database I/O so the core stays testable and embeddable.

- `s2c-core`: input assembly, shapes AST, resolution, IR, dialect renderers, manifest. No database I/O.
- `s2c-cli`: the `shacl2cypher` binary with `compile`, plus `validate` and `schema dump` when runner features are enabled.
- `s2c-runner`: executes manifests and introspects schemas; cargo features `neo4j` (Bolt via `neo4rs`) and `ladybug` (embedded `lbug` bindings).
- `s2c-testkit`: test-only, unpublished; loads conformance fixtures and projects them to RDF and LPG load scripts — see [[testing#Conformance Fixtures]].

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

## Runner

The runner compiles in memory, executes summary queries, drills into failing rules with detail queries, and produces reports.

- Reports: terminal table, JSON, JUnit (one test case per rule) and SARIF (results linked to `.ttl` spans).
- Exit code by severity threshold: `--fail-on violation|warning|info`.
- Per-query `--timeout`; timeouts produce `status: timeout` results, not crashes. Per-rule timings appear in reports.
- LadybugDB is embedded: the runner opens database files directly and may conflict with an application holding the lock.
