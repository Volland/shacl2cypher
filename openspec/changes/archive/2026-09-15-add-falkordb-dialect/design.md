## Context

See proposal.md for motivation. The compiler lowers shapes to a dialect-neutral IR (`crates/s2c-core/src/ir.rs`) and renders it with one module per dialect (`render/neo4j.rs`, `render/ladybug.rs`), sharing route planning, literals and identifiers in `render/mod.rs`. The runner has one `Executor` per database behind a cargo feature. `backend::open`, `session::dialect_named` and `session::compile_for` are shared by the CLI and the bindings.

FalkorDB is closest to Neo4j: a schemaless property graph with multi-label nodes and untyped properties. Its Cypher differs in ways that affect almost every rendered query. The spike (`spikes/falkordb`, probe scripts and their outputs in `probes/`) confirmed these behaviors on FalkorDB v4.20.4 (graph module 42004), the C engine shipped as `falkordb/falkordb:v4.20.4`:

| Construct | FalkorDB 4.20.4 (C engine) |
|---|---|
| `EXISTS { MATCH … }`, `COUNT { … }` | Parse errors. |
| Pattern predicates `(n)-[:T]->()` | Correct only directly in `WHERE` (with `NOT`/`AND`/`OR`). In `CASE` they are always true; in `RETURN` they yield a list of paths. |
| Pattern comprehensions | Correct when anchored at a variable bound by `MATCH`, `OPTIONAL MATCH` or `WITH`. Silently wrong when anchored at a variable bound by a list quantifier, list comprehension or `reduce`, or returned by a `UNION` in the same `CALL`. `UNWIND` over one fails with an internal error. Their `WHERE` cannot contain another pattern. Two comprehensions reusing variable names in one clause interfere. |
| Correlated `CALL { WITH x … }` | Correct, including `OPTIONAL MATCH` with aggregation (one row per outer row, even with no matches), `UNION`/`UNION ALL` branches, and nesting at least three levels deep. |
| Parallel relationships in `MATCH` | Collapsed to one row per node pair unless the relationship variable is referenced in `WHERE` (`id(r) >= 0`) or an aggregate. Variable-length hops keep them when bound to a path variable. |
| `v IS :: STRING NOT NULL` | Not supported. `typeOf(v)` returns `String`, `Integer`, `Float`, `Boolean`, `List`, `Map`, `Null`, `Date`, `Datetime` (local date-time), `Time` (local time), `Duration`, `Point`, `Vectorf32`, `Node`, `Edge`, `Path`. |
| Type errors | `size`, `string.matchRegEx`, `toLower`, `toString` (on lists), `keys`, `labels`, `id`, `:Label`, `STARTS WITH`, subscripts and property access raise on the wrong type. `CASE` evaluates every branch; only `AND`/`OR` directly in `WHERE` short-circuit. `toStringOrNull` never raises (lists, maps, nodes and edges give null). Comparisons and `IN` across types give null or false. |
| `v =~ pattern` | Not supported. `string.matchRegEx(s, p)` returns all matches (substring search). The engine is Oniguruma with Java-like syntax: `(?i)(?s)(?m)(?x)`, scoped flags, `\p{L}`, Unicode `\d`/`\w`, back-references, lookahead, `&&` class intersection, `\Q…\E`. `\x{…}` silently never matches; `\uHHHH` works for the BMP and astral characters work as literals, including in ranges up to U+10FFFF. An invalid pattern is a runtime error. |
| Label expressions `n:A|B` | Parse error. `WHERE n:A OR n:B` and `n:A:B` work; `[:T1|T2]` works. |
| `elementId(n)` | Not supported; `toString(id(n))` works. |
| `LIMIT coalesce($limit, …)` | Error. `LIMIT $limit` works up to `i64::MAX`; `LIMIT 0` returns no rows. `collect(…)[0..$sampleSize]` works; zero-row `collect` gives `[]`. |
| Temporals | No `datetime()` or `time()`. `localdatetime` silently drops offsets and fractional seconds, turns hour 25 into midnight and drops a time after a space separator. `date('2020-02-30')` rolls over to March 1. Durations compare as total seconds (`P1Y = P365D`); negative and fractional durations are null. Ordering comparisons (`<`, `>`) are wrong when two temporal values are more than 2^31 seconds apart, while equality is correct. `toString` does not zero-pad years below 1000. |
| Literals and identifiers | `\'`, `\\`, `\"`, `\t`, `\n` are decoded; `\uXXXX` is not decoded in strings or identifiers; raw newlines work. A backtick inside a backticked identifier cannot be written (doubling and `` \` `` are parse errors). `toString(1.5)` is `1.500000`. |
| Execution | `GRAPH.RO_QUERY` refuses writes (`graph.RO_QUERY is to be executed only on read-only queries`). On a missing graph it fails (`ERR Invalid graph operation on empty key`), whereas `GRAPH.QUERY` creates the graph. Timeouts report `Query timed out`; without a `TIMEOUT` argument the server's default of 1000 ms applies, and `TIMEOUT 0` disables it. |
| Introspection | `CALL db.labels() YIELD label`, `CALL db.relationshipTypes() YIELD relationshipType`, dynamic access `n[k]` and `r[k]`. No `db.schema.*`. Variable-length upper bounds of 1000 are accepted. |

Stored values cannot be maps or null, and arrays cannot contain nulls.

## Goals / Non-Goals

**Goals:**
- A FalkorDB renderer that gives the same `(ruleId, focus)` results as Neo4j on every conformance fixture the engine can store.
- The FalkorDB C engine 4.20.x is the supported server version and is pinned in CI.
- The runner backend reuses the validate flow, reports and exit codes unchanged.

**Non-Goals:**
- The Rust-rewrite engine (`falkordb/falkordb:edge`). Its regex and aggregation behavior differs, so it gets its own follow-up once it ships as `latest`.
- TLS (`rediss://`), Redis Cluster and Sentinel connections. A `rediss://` URL is a clear "not supported yet" setup error.
- Python and Node.js `FalkorDB` database handles and their wheel and addon packaging.
- Running the W3C suite on FalkorDB. It stays Neo4j-only for now.
- Schema-guaranteed statuses on FalkorDB. Like Neo4j, a snapshot is evidence for resolution and missing-element diagnostics, not a type guarantee, because properties are untyped.
- Query performance parity with Neo4j. Correctness comes first; see Risks.

## Decisions

### 1. The spike gates the renderer

`spikes/falkordb` holds the probe scripts (run with `redis-cli` inside the container) and a Rust client probe, with their outputs committed next to them. The findings are summarized in the Context table and recorded in `lat.md/dialects.md` as "Spike findings", which renderer and runner code cite.

**Alternative considered:** writing the renderer straight from the docs. The spike justified itself: pattern comprehensions anchored at quantifier variables, `CASE` evaluating every branch and parallel-relationship collapsing all return wrong answers without any error.

### 2. A CALL-decorrelated renderer

`render/falkordb.rs` reuses the Neo4j renderer's overall structure (focus, value binding, rows, messages, route planning from `render/mod.rs`), but it renders every relationship traversal as a correlated subquery instead of an expression, because patterns are only reliable when anchored at row variables:

- **Focus:** a single label is `MATCH (v0:A)`; label unions are `MATCH (v0) WHERE v0:A OR v0:B`. Subjects-of and objects-of targets use an existence subquery column. A relationship focus is `MATCH (s0)-[v0:T]->(e0) WHERE id(v0) >= 0`, carried as `WITH s0, v0, e0`.
- **Traversals:** each relationship route becomes `CALL { WITH x OPTIONAL MATCH (x)-[r1:T]->()-[r2:U]->(y) WHERE id(r1) >= 0 AND id(r2) >= 0 RETURN <aggregate> AS c1 }`. Every relationship variable is referenced; variable-length hops bind a path variable (`p1 = (x)-[:T*1..5]->(y)`). Alternative routes are `UNION` branches of an inner `CALL` whose rows feed the aggregate. The renderer never emits pattern predicates or pattern comprehensions.
- **Composition:** IR quantifiers, counts, existence checks and pair sides over relationships become aggregate columns:
  - `All`/`Any`: `all(x IN collect(CASE WHEN y IS NULL THEN null ELSE <test> END) WHERE x)`.
  - `Count`: `count(DISTINCT CASE WHEN <filter> THEN y END)`.
  - Pair sides: `collect(DISTINCT id(y))` or distinct value lists.

  A test on the neighbour `y` that traverses further is itself a nested `CALL` anchored at `y` inside the enclosing subquery, so nesting follows shape depth. The rule condition is `WITH v0, c1, c2 WHERE NOT (<expression over columns>)`.
- **Property values:** scalar properties are normalized with `CASE typeOf(p) WHEN 'Null' THEN [] WHEN 'List' THEN p ELSE [p] END`. Quantifiers over property lists stay list expressions, which the spike showed are correct because no pattern is involved. Property values reached through relationships are unwound inside the traversal subquery.
- **Total value tests:** `CASE` evaluates every branch, so no emitted expression may raise for any value type:
  - Datatype tests use `typeOf`, and integer ranges are `coalesce`-wrapped comparisons.
  - Length and pattern tests operate on `toStringOrNull(v)`, for example `CASE typeOf(v) WHEN 'String' THEN size(toStringOrNull(v)) >= 3 … ELSE false END`.
  - Node-only functions (`labels`, `id`, `:Label`, property access) are applied only to values the IR proves are nodes.
  - Row values and keys use `toStringOrNull`.
- **Regex:** `sh:pattern` renders as `size(string.matchRegEx(toStringOrNull(v), '<flags><pattern>')) > 0`, with no substring wrapper. The renderer rewrites the core's `\x{H}` escapes to `\uHHHH` (BMP) or the literal character (astral), and the empty class to `[^ -￿𐀀-􏿿]`. Back-references are allowed, as on Neo4j.
- **Rows:** `elementId` is `toString(id(n))`. Detail queries end with `LIMIT $limit`, and the runner always sends a number, as on LadybugDB. Summaries slice `collect(…)[0..$sampleSize]`.
- **Names:** every variable the renderer binds is fresh within a query, because comprehension and subquery scopes interfere when names repeat.
- **Literals:** `render::constant` gains a `Dialect::FalkorDb` arm:
  - `xsd:date` constants are validated as real calendar dates before rendering `date('…')`.
  - `xsd:dateTime` constants without a timezone and without fractional seconds, with valid field ranges, render as `localdatetime('…')`.
  - `xsd:time` constants under the same rules render as `localtime('…')`.
  - Zoned date-times and times, fractional seconds and every `xsd:duration` constant are compile errors naming FalkorDB, because the engine drops offsets and fractions and compares durations as seconds. This follows the "inexpressible construct" requirement rather than approximating.
- **Identifiers:** a label, relationship type or property key containing a backtick is a compile error on FalkorDB.

**Alternatives considered:**
- Expression rendering with pattern predicates and comprehensions, as the Neo4j renderer does with `EXISTS {}`/`COUNT {}`. Rejected by probes 04 and 06: nested traversals from quantifier variables return wrong results silently.
- Rejecting nested traversals on FalkorDB. Rejected because it drops Tier 2 coverage that the `dialect-rendering` spec requires.
- Flat `OPTIONAL MATCH` chains with `WITH` grouping instead of `CALL`. They work for one level (probe 08), but sibling conditions multiply rows and grouping keys; `CALL` keeps each condition independent.
- Making the Neo4j renderer configurable. Nearly every emitted construct changes, so two parallel modules are easier to snapshot-test.

### 3. Dialect wiring

- `Dialect::FalkorDb` has the name `"falkordb"`. `max_path_depth` is `None`: an upper bound of 1000 was accepted.
- `hop_pattern` uses `|` between relationship types, as for Neo4j.
- The schema is optional. `SchemaChecker` is not enforcing, as for Neo4j.
- The `--neo4j-labels explicit|inherited` policy also applies to FalkorDB, since both have multi-label nodes. The flag is not renamed, to avoid a breaking CLI change; its help text and the docs say it covers FalkorDB.
- `manifest.dialect` is `"falkordb"`, and `check_dialect` rejects manifests compiled for other dialects.
- The bindings get `dialect="falkordb"` through `session::dialect_named`. Only their type declarations (Python `Literal`, `index.d.ts` union) change.

### 4. Runner backend on the `falkordb` crate

- Feature `falkordb = ["dep:falkordb", "dep:redis"]` on `shacl2cypher-runner`, forwarded by the `shacl2cypher` CLI. The client is the sync `falkordb` crate `=0.10.3` with `redis` pinned to `=1.2.2`, which builds on Rust 1.87.
- `BackendConfig::FalkorDb { url, graph }`. Credentials come from the `redis://user:password@host:port` URL, or from the `FALKORDB_USERNAME` and `FALKORDB_PASSWORD` environment variables when the URL has none. `rediss://` is rejected before connecting.
- **Opening:** build the client, then `list_graphs`. A graph missing from the list is a `BackendError::Connection` ("graph `g` does not exist"). The executor only ever sends `GRAPH.RO_QUERY`, because `GRAPH.QUERY` would create a missing graph.
- **Running a query:** `ro_query` with the `limit` and `sampleSize` parameters.
  - `TIMEOUT` is the `--timeout` in milliseconds, or `0` when none is given, because the server otherwise applies its 1000 ms default. The server-side timeout is the only enforcement: the client probe showed that the crate's response timeout does not interrupt a running query. A server `TIMEOUT_MAX` below `--timeout` is documented as capping it.
  - The crate drops the first word of server errors, so the executor matches message suffixes: `timed out` becomes `ExecError::Timeout`, and every other server error becomes `ExecError::Query` with the message kept.
  - Rows convert `FalkorValue` to JSON explicitly, as the Neo4j executor does. Nodes and edges become property objects with `_labels` or `_type`. Temporal scalars are formatted as ISO strings: dates and local date-times as seconds since the Unix epoch, local times as seconds since 1900-01-01 (taken modulo one day), and durations as total seconds (`PT93600S`). Paths are never returned by generated queries; the crate cannot decode them.
- **Schema dump:** `CALL db.labels() YIELD label` and `CALL db.relationshipTypes() YIELD relationshipType`, then `MATCH (n) UNWIND keys(n) AS k RETURN labels(n), k, collect(DISTINCT typeOf(n[k]))` over nodes and the same over relationships.
  - `typeOf` strings map to neutral types: `String`→`STRING`, `Integer`→`INT64`, `Float`→`DOUBLE`, `Boolean`→`BOOLEAN`, `Date`→`DATE`, `Datetime`→`LOCAL_DATETIME`, `Time`→`LOCAL_TIME`, `Duration`→`DURATION`, `Point`→`POINT`.
  - Lists map to `LIST<T>` from their element types. Several types and `Vectorf32` map to `ANY`.
  - Endpoints come from `MATCH (a)-[r]->(b) RETURN DISTINCT labels(a), type(r), labels(b)`, split into one pair per label, following the Neo4j dump rules. The dump scans all data, as the Neo4j endpoint scan does.

**Alternative considered:** `redis` with raw commands. Rejected because the crate already parses FalkorDB's compact result format, including temporals, points and paths, and builds on the pinned toolchain.

### 5. CLI

`validate` and `schema dump` gain `--falkordb <URL>` and `--graph <NAME>`. `--graph` is required with `--falkordb`, and `--falkordb` conflicts with `--connect` and `--ladybug`. `compile --dialect falkordb` needs no connection. The validate flow's `--limit 0` sends `i64::MAX`, as on LadybugDB.

### 6. Tests

- **Testkit:** `ENGINES` gains `falkordb`. `lpg::falkordb_script` reuses the Neo4j `CREATE` statements, with a check that rejects nulls inside lists and map values. Fixtures FalkorDB cannot store are restricted with `dialects`.
- **Integration:** `crates/s2c-runner/tests/falkordb.rs` runs when `S2C_FALKORDB_URL` is set. Each fixture loads (with `GRAPH.QUERY`) into a fresh uniquely named graph that is deleted afterwards. The tests also check that writes are refused, timeouts are reported, a missing graph is an error, and the schema round-trips.
- **Literal fuzzing:** the existing literal fuzz test gains FalkorDB, excluding identifiers with backticks.
- **Core:** the renderer unit tests, insta snapshots of `tests/data/snapshot.ttl` for FalkorDB, the read-only clause check and the determinism tests all include the new dialect. `S2C_RENDER_DUMP_FALKORDB` dumps queries for smoke runs.
- **Lint:** the null-safety lint gains FalkorDB rules. It flags pattern predicates or comprehensions anywhere, and `size`, `string.matchRegEx` or `toString` applied to anything other than `toStringOrNull(…)` or a known string.
- **CI:** a `falkordb` job with a `falkordb/falkordb:v4.20.4` service on port 6379. Release binaries build with `--features neo4j,ladybug,falkordb`.

## Risks / Trade-offs

- [One correlated `CALL` per traversal runs per focus row, which costs more than Neo4j's `COUNT {}`] → Cost classes are unchanged (`scan+expand`, `quadratic`). The docs record the trade-off, and the smoke dump times queries on sample data.
- [FalkorDB `CALL`/`UNION` correctness beyond the probed shapes] → Every conformance and regex fixture runs on FalkorDB, and the smoke dump compares violation counts with Neo4j. The lint keeps pattern predicates and comprehensions out of queries.
- [Future engine releases change the unreliable behaviors] → CI pins `v4.20.4`. The probe scripts are kept so a version bump reruns them.
- [No `=~` means regex semantics follow Oniguruma rather than Java] → Patterns stay AST-rendered and compile-time validated, and the regex fixtures run on FalkorDB.
- [Temporal and duration constants are rejected rather than approximated] → Users see a compile error naming FalkorDB; `sh:datatype` checks on temporal values still work.
- [Known FalkorDB variable-length path bugs (#2682, #2679, #2293)] → Complex-path fixtures run on FalkorDB. Engine bugs become `known_differences` entries with the issue link, never silent passes.
- [Future `redis` or `falkordb` releases raise the MSRV] → Exact pins (`=`), as for `neo4rs`, and CI on Rust 1.87.
- [Schema dump scans the whole graph] → Same trade-off as the Neo4j endpoint scan, documented. The snapshot is optional for FalkorDB.
- [Rust-engine (`edge`) behavior diverges] → Explicitly not supported. CI pins the C engine tag.

## Migration Plan

This is purely additive: no existing dialect, flag, manifest field or report format changes. Manifests from earlier versions keep working. Rollback means dropping the `falkordb` feature from release builds.
