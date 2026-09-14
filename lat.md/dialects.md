# Dialects

Cypher generation is split into a dialect-neutral IR and per-dialect renderers. Version 1 ships Neo4j 5 and LadybugDB backends.

## IR

The intermediate representation describes targets, value sets and null-safe predicates without committing to Cypher syntax.

A compiled rule has a structural id, source location, severity, messages, focus sets, a violation form, optional details, a status and an optional path-depth cap. Focus sets are labels, subjects of a path, objects of a path, or relationships of a type. The status is `compiled`, `guaranteed-by-schema`, `deactivated` or `unsupported`. Lowering is described in [[semantics#Violations and Conforms]].

- Violation forms: `PerValue` reports each value of a source (the focus itself or a resolved path) that fails a test; `PerFocus` reports each focus that fails a condition.
- Boolean expressions: constants, `Not`/`And`/`Or`/`Xone`, `All` (every value of a source passes), `Count` (distinct values, optionally filtered, compared to a bound), `Test` on one bound value, `Pair` (`sh:equals`, `sh:disjoint`, `sh:lessThan`, `sh:lessThanOrEquals`) and `Closed`.
- Value tests: datatype ([[mapping#Datatypes]]), label membership, comparison with a typed constant, string length, regex ([[semantics#Regex Translation]]), membership in constants, and node-key membership for IRI constants compared with relationship values.
- Constants are typed: string, integer, decimal, double, boolean, date, date-time, time, duration. Language-tagged, non-finite and unsupported-datatype literals are rejected.
- Invariant: tests only reference variables bound by `All`, `Count` or the rule's own value variable, and value sets never contain nulls. Every test is therefore total and renderers never need three-valued logic. `Rule::validate` enforces the binding.
- Constructors fold constants (`Not(true)`, `And` containing `false`, `Count >= 0`, always-false filters), so trivially decided checks never reach a renderer.

## Dialect Backends

Each backend renders IR into Cypher text and declares which IR constructs it can express.

### Neo4j

Targets Neo4j 5 Cypher without APOC, using `IS ::` type predicates, `COUNT {}` and `EXISTS {}` subqueries, and label expressions.

Schema snapshot is optional; without it, resolution falls back to convention. Snapshot can be dumped from `db.schema.nodeTypeProperties()` and `db.schema.relTypeProperties()`.

Rendering matches the focus with `MATCH (v0:A|B)`, `MATCH (s0)-[v0:T]->(e0)`, or `MATCH (v0) WHERE …` for subject and object targets. Values are then bound and the rule filters with `WHERE NOT (…)`:

- Value binding: `UNWIND` over normalized property lists, a relationship `MATCH`, or `CALL { … UNION … }` for paths with several routes.
- Quantifiers become list predicates or `NOT EXISTS { MATCH … }`. Distinct counts use `reduce` over property values or `COUNT { … RETURN DISTINCT … }` over relationships.
- Value tests are guarded with `IS :: STRING NOT NULL` or `coalesce(…, false)`. Regexes use the `(?flags)(?s:.*)(?:…)(?s:.*)` wrapper.
- Detail queries end with `LIMIT coalesce($limit, 9223372036854775807)`; summaries return `collect(focus)[0..$sampleSize]`.

Spike findings (Neo4j 5.26), which renderers must respect:

- `null IS :: T` is true; type checks use `IS :: T NOT NULL`. Union types (`IS :: ZONED DATETIME | LOCAL DATETIME`) work; `LOCAL DATETIME` is not `ZONED DATETIME`.
- `size()` on a non-string, non-list value raises a runtime error, whereas `=~` and comparisons on mismatched types return null. Length and pattern predicates are therefore type-guarded, not just coalesced — see [[semantics#Null Safety]].
- `=~` is fully anchored Java regex, supports backreferences, and raises a runtime error on an invalid pattern.
- The static type checker rejects some literal-only expressions (e.g. `size(5)`), so renderer tests use property values, not literals.
- Confirmed: `COUNT {}` and `EXISTS {}` in `WHERE` and in boolean expressions, nested `EXISTS`, label expressions `n:A|B`, `keys(n)`, map rows, `collect(…)[0..k]`, `LIMIT coalesce(…)`, `replace()` for messages.

### LadybugDB

Targets LadybugDB's strictly-typed Cypher (Kùzu lineage): one table per node, rel tables with declared FROM/TO pairs, and typed columns.

The schema snapshot is required because the binder rejects references to undeclared tables or columns (`Cannot find property`, `Table X does not exist`).

Rendering relies on the snapshot's column types. Each value binding knows its column type, so datatype, comparison, `sh:in`, length and regex tests resolve to `true`, `false`, a range check or a typed expression. Mismatched types therefore never reach the database.

- Focus: `MATCH (v0:A:B)` over node tables, `MATCH (s0)-[v0:T]->(e0)`, or `MATCH (v0) WHERE …`.
- Values: scalar columns bind with `WITH …, v0.k AS v1`, list columns with `UNWIND v0.k AS v1`, and every value row is guarded by `v1 IS NOT NULL`.
  - Unwinding `list_filter(coalesce(…))` directly leaks list elements from one row into rows whose list is NULL, an engine bug found by the renderer smoke run. List functions therefore appear only inside expressions, and alternatives unwind `coalesce`-wrapped lists.
  - Alternatives across different relationship routes are rejected.
- Counts:
  - A rule-level count over one relationship route is hoisted into `OPTIONAL MATCH … count(DISTINCT …)`.
  - Property counts use `size(list_distinct(…))`.
  - Nested counts support existence checks, or `COUNT { MATCH }` over single hops.
- `sh:xone` renders as a sum of `CASE` terms to avoid nested lambdas. `sh:closed` requires every declared column outside the allowed keys to be NULL. `sh:lessThan` over list properties and regex back-references are rejected.
- Rows use `label()` and `CAST(id(…) AS STRING)`. Detail queries end with `LIMIT $limit`; summaries slice `collect(…)` with `list_slice(…, 1, $sampleSize)`.

Spike findings (Rust crate `lbug` 0.20.4), which renderers must respect:

- Correlated `EXISTS { MATCH … }` and `COUNT { MATCH … }` work in `WHERE`, `RETURN`, `CASE`, list literals and nested three levels deep, so no `OPTIONAL MATCH` hoisting is needed. The `MATCH` keyword inside the braces is mandatory.
- Unsupported: list comprehensions `[x IN l WHERE …]`, pattern comprehensions and `CALL {}`. Use `list_filter(l, x -> p)`, `all(x IN l WHERE p)` and subqueries instead.
- `collect()` over zero rows yields NULL, and empty list literals need a type: wrap as `coalesce(collect(…), CAST([] AS T[]))`.
- Mixed-type comparisons raise conversion errors; since columns are typed, type mismatches are decided at compile time from the schema snapshot.
- Variable-length relationship upper bounds are capped at 30 — see [[output#Cost Classes]].
- Focus identity uses `label(n)` and `id(n)`; `typeof()` exists but there is no `IS ::` syntax.
- Introspection: `CALL show_tables()`, `CALL table_info('T')`, `CALL show_connection('R')`.
- `SystemConfig::read_only(true)` opens a database that rejects writes, which the runner uses.
- Regex behavior is described in [[semantics#Regex Translation]].

### Renderer Probes

A second spike (Neo4j 5.26, `lbug` 0.20.4) checked the constructs renderers emit. These results shape rendering.

- Both: raw newlines and `\uXXXX` escapes work in string literals, and so do `\x{…}` regex escapes, maps/structs containing nulls, and zero-row aggregates that still return one row. Element ids come from `elementId()` or `CAST(id(n) AS STRING)`.
- Neo4j: `COUNT { MATCH … RETURN DISTINCT m }` counts distinct nodes. `COUNT { UNWIND … }`, list comprehensions and `reduce` work. `LIMIT coalesce($limit, …)` accepts a null parameter. Scoped `(?s:.*)` wrappers leave pattern flags untouched.
- LadybugDB: `COUNT { MATCH … }` counts matched paths and has no `RETURN DISTINCT`.
  - Exact distinct counts in rule-level conditions are hoisted into `OPTIONAL MATCH … count(DISTINCT …)`.
  - Nested counts other than existence checks count paths, so parallel duplicate relationships count twice.
- LadybugDB: `list_distinct`, `list_filter`, `list_contains`, `list_slice` and `s[1:$n]` work. A lambda cannot reference an enclosing lambda's variable, so nested list quantifiers (such as `sh:lessThan` over two list properties) are unsupported.
- LadybugDB: `LIMIT` accepts only a literal or a parameter, so `$limit` must be a number; runners pass the maximum for "unlimited".
- LadybugDB: table names are case-insensitive, so a node table and a rel table whose names differ only in case collide (`Address` and `ADDRESS`, the conventional names for `ex:Address` and `ex:address`). Such mappings need an `s2c:label` or `s2c:relationship` annotation on LadybugDB.
- LadybugDB: a property missing from one of several matched tables reads as NULL. Comparing mismatched types raises an error, so type compatibility is decided from the schema before comparing.
- LadybugDB: `interval()` rejects ISO 8601 durations (`P1DT2H`), so they are rendered in words (`1 day 2 hours`). `timestamp()` accepts ISO date-times and `TIMESTAMP_TZ` casts keep offsets. `CAST(… AS STRING)` formats doubles and booleans unlike Neo4j (`1.500000`, `True`).

## Schema Awareness

The compiler consumes a dialect-neutral schema snapshot describing node types, relationship types with endpoints, and typed properties.

Format: `nodeTypes[{name, properties[{name, type}]}]` and `relTypes[{name, endpoints[{from, to}], properties[]}]`. Endpoints are explicit FROM/TO pairs, as LadybugDB declares them, so endpoint reasoning never mixes pairs.

Property types use neutral names: `STRING`, `INT64`/`INT32`/`INT16`/`INT8`, `UINT64`/`UINT32`/`UINT16`/`UINT8`, `DOUBLE`, `FLOAT`, `DECIMAL`, `BOOLEAN`, `DATE`, `LOCAL_DATETIME`, `ZONED_DATETIME`, `LOCAL_TIME`, `ZONED_TIME`, `DURATION`, `POINT`, `BLOB`, `ANY` (mixed or unknown), and `LIST<T>`. A snapshot is rejected on load for unknown fields or types, duplicate type or property names, missing endpoints, or endpoints naming undeclared node types.

Snapshots come from a JSON file or from live introspection via the runner. The snapshot drives three things: static diagnostics for missing elements ([[output#Static Diagnostics]]), skipping constraints guaranteed by the schema (`status: guaranteed-by-schema`), and disambiguating property vs relationship paths ([[mapping#Resolution]]).

## Literals and Identifiers

All shape constants are inlined through a single typed literal renderer per dialect; identifiers are always backtick-escaped.

- String escaping, number formatting (NaN/Infinity rejected) and temporal constructors live in one renderer; building literals ad hoc with string formatting is forbidden.
- Identifiers are backticked with inner backticks doubled; names a target cannot represent are compile errors.
- Runtime parameters are reserved for execution controls: `$limit`, `$sampleSize`.
- `sh:message` placeholders (`{$this}`, `{?value}`) are substituted in Cypher at runtime so messages carry actual values.
