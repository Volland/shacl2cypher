## Context

Greenfield Rust project; see proposal.md for motivation. The detailed design reference lives in `lat.md/` (overview, mapping, semantics, output, dialects, architecture, testing) and specs under `specs/` define required behavior. The two targets differ sharply: Neo4j is schema-optional with multi-label nodes; LadybugDB (Kùzu lineage) is strictly typed with one table per node and a binder that rejects undeclared tables/columns.

## Goals / Non-Goals

**Goals:**
- Pure, deterministic compiler core usable as a library and snapshot-testable.
- Semantic equivalence with SHACL reference engines on the supported feature set, verified differentially.
- One IR, two renderers, with dialect differences isolated in backends.

**Non-Goals:**
- Query optimization beyond cost classification and index recommendations.
- Batched/partitioned execution, incremental validation, or writing validation reports into the graph.
- A general SHACL engine for RDF data.

## Decisions

### D1: Native LPG with convention + `s2c:` annotations
Map IRIs by local-name convention, overridden by in-SHACL `s2c:` annotations, disambiguated by schema snapshot. Alternatives: separate mapping file (drifts from shapes), convention only (property-vs-relationship ambiguity), n10s RDF model (users have app-built LPGs). `--strict` gives explicit-only mode. The vocabulary namespace is `https://w3id.org/shacl2cypher#` (a permanent identifier that survives repository moves; a w3id.org redirect is registered separately).

### D2: Rust workspace `s2c-core` / `s2c-cli` / `s2c-runner`
Core has no DB I/O; runner drivers behind `neo4j`/`ladybug` features. Alternatives: Python (pySHACL oracle in-process, but weaker distribution) and TypeScript. Rust chosen for single-binary distribution and rudof's native oracle.

### D3: Own shapes AST over `oxttl`/`oxrdf`
Needed for source spans, `s2c:` annotations, per-file blank-node scoping and conflict detection. Alternative: rudof `shacl_ast` (API churn, loses spans and unknown triples). rudof `shacl_validation` is still used as a dev-dependency oracle.

### D4: Dialect-neutral IR with `violations` and `conforms`
Every shape lowers to a row-producing `violations` form and a boolean `conforms` form for inlining into `sh:node`/logical/qualified constraints. Inlining forces rejection of recursive shapes. Alternative: materializing conformance per shape in temp results — not possible read-only and not portable.

Null safety is structural: value predicates are built only through a constructor that wraps `coalesce(p, false)`; a lint test scans rendered Cypher for bare comparisons under `NOT`.

### D5: One query per constraint, detail + summary variants
Maximizes independent naming, timing and baselining. Summary always returns a row. Alternative: per-shape `UNION` queries (coarser diagnostics, one slow constraint blocks a shape).

### D6: Structural rule ids with collision hashing, `s2c:name` override, fingerprints
Blank-node property shapes have no stable identity; structural ids survive reordering. Canonical form for hashing: sorted N-Triples-like serialization of the constraint subtree with blank nodes replaced by structural paths.

### D7: Schema snapshot as a first-class input
Dialect-neutral JSON produced by `schema dump`. Required on LadybugDB (binder strictness), optional on Neo4j. Enables static `SchemaMismatch` diagnostics and `guaranteed-by-schema` skipping.

### D8: Inline literals via one typed renderer; backtick all identifiers
Cypher cannot parameterize identifiers anyway, and inlined queries are copy-pasteable. Parameters reserved for `$limit`/`$sampleSize`. Round-trip fuzzing guards escaping.

### D9: XSD regex parsed and re-rendered
Neo4j `=~` is anchored Java regex; LadybugDB uses RE2. Parse via a pre-pass for XSD-only syntax then `regex-syntax`, render per dialect with substring wrapping and flag mapping, reject inexpressible constructs.

### D10: Differential conformance testing
Neutral YAML fixtures project to RDF (rudof, pySHACL) and LPG (Neo4j via testcontainers, LadybugDB in-process), all compared to a hand-written `expect` block. Plus filtered W3C suite, `insta` snapshots, determinism tests.

## Risks / Trade-offs

- [LadybugDB may not support correlated `COUNT {}`/`EXISTS {}` subqueries inside expressions] → **Resolved by spike:** supported up to at least three nesting levels, no hoisting fallback needed. LadybugDB lacks list/pattern comprehensions and `CALL {}`, so its renderer uses `list_filter`/`all()` and typed empty lists (details in `lat.md/dialects.md`).
- [LadybugDB Rust bindings availability/maturity] → **Resolved by spike:** crate `lbug` 0.20.4 builds and runs in-process, including read-only open.
- [Neo4j `size()` raises runtime errors on non-string values] → Length/pattern predicates are type-guarded with `IS :: STRING NOT NULL`, not only coalesced.
- [RE2 silently returns false for backreferences on LadybugDB] → Compile-time rejection is mandatory, covered by regex fixtures.
- [LadybugDB caps variable-length paths at depth 30] → Compile error when `--max-path-depth` exceeds 30 for that dialect.
- [Local toolchain is rustc 1.87, while latest `cxx`/`time` require 1.88] → Workspace declares `rust-version = "1.87"` with Cargo's `incompatible-rust-versions = "fallback"` resolver; revisit when the toolchain is upgraded.
- [Neo4j `IS ::` semantics for list/temporal types differ from XSD] → Datatype fixtures per XSD type; document lossy `xsd:decimal`.
- [Inlined `conforms` expressions grow exponentially with nesting depth] → Cost class plus a compile-time nesting depth warning; memoization via `WITH` stages if needed later.
- [Oracles disagree on edge cases] → Hand-written `expect` is authoritative; disagreements annotated in fixtures.
- [Convention mapping guesses wrong silently] → Schema snapshot evidence, `--strict`, and static diagnostics.
- [Full scans on large graphs are slow] → Cost classes, index recommendations, timeouts; batching deferred.

## Migration Plan

Not applicable (greenfield). Manifest `schemaVersion` starts at 1; breaking manifest changes bump it.

## Open Questions

- Default `--max-path-depth` value (proposed 10).
- Whether JSON-LD input is added after v1.
