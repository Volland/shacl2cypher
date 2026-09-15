## 1. Spike: confirm FalkorDB behavior

- [x] 1.1 Create `spikes/falkordb` (outside the workspace) with a runner that executes probe queries against `falkordb/falkordb:v4.20.4` in Docker
- [x] 1.2 Probe regex via `string.matchRegEx`: anchoring, `(?i)(?s)(?m)` flags, `\x{…}`, `\p{L}`, backreferences, invalid-pattern errors
- [x] 1.3 Probe `typeOf` for every storable type (including `localdatetime`, zoned date-times, `localtime`, lists, points) and whether zoned date-times can be stored
- [x] 1.4 Probe `AND` short-circuiting around `size()` type errors, and `CASE` guards
- [x] 1.5 Probe pattern comprehensions with `WHERE`, nesting in `WHERE`/`CASE`/`reduce`, variable-length hops, and parallel-relationship counting with and without a referenced relationship variable
- [x] 1.6 Probe `n[k]` on nodes and relationships, string escapes (`\'`, `\\`, `\uXXXX`, newlines), doubled-backtick identifiers, zero-row aggregates, `collect(…)[0..$sampleSize]`, `LIMIT $limit` with `i64::MAX`, variable-length depth limits
- [x] 1.7 Record timeout, write-refusal and missing-graph error texts from `GRAPH.RO_QUERY`
- [x] 1.8 Build the `falkordb` crate 0.10.3 with `redis = "=1.2.2"` on Rust 1.87; choose it or the raw-`redis` fallback (design decision 4)
- [x] 1.9 Write the results as "Spike findings" under a new FalkorDB section in `lat.md/dialects.md` and adjust design decisions 2 and 4 wherever a finding contradicts them

## 2. Core dialect wiring

- [x] 2.1 Add `Dialect::FalkorDb` (`"falkordb"`) with no path-depth limit; extend `hop_pattern` and every `Dialect` match in `render/mod.rs` and `compile.rs`
- [x] 2.2 Add the FalkorDB arm to `render::constant`: calendar-validated `date`, whole-second local `localdatetime`/`localtime`, and compile errors for zoned or fractional temporals and all durations; unit-test each case
- [x] 2.3 Make the schema optional and non-enforcing for FalkorDB, apply `--neo4j-labels` policy, and dispatch to the new renderer in `compile.rs`; add compile tests for manifest dialect and missing-schema acceptance
- [x] 2.4 Reject labels, relationship types and property keys containing a backtick on FalkorDB, with a unit test

## 3. FalkorDB renderer

- [x] 3.1 Create `render/falkordb.rs` with focus matching: single label, label unions via `WHERE v0:A OR v0:B`, subjects-of/objects-of targets via existence subquery columns, relationship focus `MATCH (s0)-[v0:T]->(e0) WHERE id(v0) >= 0`
- [x] 3.2 Render relationship traversals as correlated `CALL { WITH x OPTIONAL MATCH … RETURN <aggregate> }` subqueries: referenced relationship variables, path variables for variable-length hops, `UNION` branches for alternative routes, fresh variable names, and no pattern predicates or comprehensions
- [x] 3.3 Render value binding and quantifiers: scalar-to-list normalization with `typeOf`, per-value rows over property and relationship routes, and `All`/`Count`/existence as aggregate columns nested per shape depth and combined in `WITH … WHERE NOT (…)`
- [x] 3.4 Render total value tests: `typeOf` datatype checks with `coalesce`-wrapped integer ranges and comparisons, `sh:in`, node-key membership, length and pattern tests over `toStringOrNull`, and `string.matchRegEx` with `\x{…}` rewritten to `\uHHHH` or literal code points
- [x] 3.5 Render pair constraints (`sh:equals`, `sh:disjoint`, `sh:lessThan`, `sh:lessThanOrEquals`), `sh:closed`, `sh:xone`, qualified counts and nested `details`
- [x] 3.6 Render rows and summaries: focus maps with `toString(id(…))` and `toStringOrNull` keys, relationship focus `{type, startKey, endKey, elementId}`, `--verbose` properties, message placeholders, `LIMIT $limit`, `collect(…)[0..$sampleSize]`
- [x] 3.7 Map unsupported constructs to compile errors naming FalkorDB and the source location, and to `status: unsupported` under `--lenient`
- [x] 3.8 Port the Neo4j renderer unit tests to FalkorDB (every compiled rule renders without write clauses; nested details, pairs, closed and relationship focus)
- [x] 3.9 Extend the null-safety lint for FalkorDB: flag pattern predicates and comprehensions, and `size`/`string.matchRegEx`/`toString` over anything but `toStringOrNull(…)` or a known string
- [x] 3.10 Add FalkorDB insta snapshots of `tests/data/snapshot.ttl`, FalkorDB cases in `tests/determinism.rs`, and the `S2C_RENDER_DUMP_FALKORDB` dump

## 4. Runner backend

- [x] 4.1 Add the `falkordb` feature and pinned dependencies to `shacl2cypher-runner` and forward it from `shacl2cypher`; extend `available_backends`
- [x] 4.2 Implement `FalkorDbExecutor::open`: URL parsing (reject `rediss://`), env credentials, `list_graphs` existence check, and `GRAPH.RO_QUERY`-only execution
- [x] 4.3 Implement `run` with `ro_query`, `limit`/`sampleSize` params, `TIMEOUT` (`0` without `--timeout`), timeout error mapping, and explicit `FalkorValue` to JSON conversion including temporal formatting
- [x] 4.4 Implement `schema()`: `CALL db.labels() YIELD label`, relationship types, per-label and per-type `typeOf` sampling mapped to neutral types, endpoint scan split per label pair
- [x] 4.5 Add `BackendConfig::FalkorDb`, `backend::open` arm, `session::dialect_named("falkordb")`, and ensure `compile_for` does not dump a schema for FalkorDB; add unit tests for unavailable-backend messages and dialect parsing

## 5. CLI and bindings types

- [x] 5.1 Add `falkordb` to `--dialect` and `--falkordb <URL>`/`--graph <NAME>` to `validate` and `schema dump` with conflicts and requirements; update `--neo4j-labels` help text
- [x] 5.2 Add CLI tests: compile with `--dialect falkordb`, backend-missing exit code 2, `--graph` required, manifest dialect mismatch
- [x] 5.3 Allow `"falkordb"` in the Python `Dialect` literal and the Node `index.d.ts` dialect type, with a compile test in each binding

## 6. Testkit and integration tests

- [x] 6.1 Add `falkordb` to `ENGINES` and implement `lpg::falkordb_script` with a check rejecting nulls in lists and map values; unit-test it
- [x] 6.2 Add `dialects`/`known_differences` entries for fixtures FalkorDB cannot store or where confirmed engine bugs diverge (with issue links)
- [x] 6.3 Create `crates/s2c-runner/tests/falkordb.rs` (gated on `S2C_FALKORDB_URL`): conformance fixtures on fresh per-fixture graphs, schema round-trip, write refusal, timeout reporting, missing graph error
- [x] 6.4 Extend `crates/s2c-runner/tests/literals.rs` round-trip fuzzing to FalkorDB
- [x] 6.5 Run the renderer smoke dump against FalkorDB sample data and fix any query the engine rejects or whose counts disagree with Neo4j
- [x] 6.6 Confirm all conformance and regex fixtures agree with `expect` on FalkorDB, Neo4j and LadybugDB

## 7. CI, release and documentation

- [x] 7.1 Add a `falkordb` CI job with a `falkordb/falkordb:v4.20.4` service and `cargo test --workspace --features shacl2cypher/falkordb`
- [x] 7.2 Add the `falkordb` feature to release binary builds in `.github/workflows/release.yml` and verify the Linux and macOS builds
- [x] 7.3 Update `lat.md/dialects.md` (FalkorDB backend section), `architecture.md` (crates, CLI, backends, schema dump), `mapping.md` (datatypes table, class hierarchy), `semantics.md` (regex translation), `output.md` (cost notes) and `testing.md`
- [x] 7.4 Add `Fixtures on FalkorDB` and new runner test specs to `lat.md/tests.md` with `// @lat:` references from the tests
- [x] 7.5 Update README with FalkorDB usage, then run `lat check`, `cargo fmt --check`, `cargo clippy --workspace --all-targets --features shacl2cypher/falkordb -- -D warnings`
