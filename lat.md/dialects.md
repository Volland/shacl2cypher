# Dialects

Cypher generation is split into a dialect-neutral IR and per-dialect renderers. Version 1 ships Neo4j 5 and LadybugDB backends.

## IR

The intermediate representation describes targets, value sets and null-safe predicates without committing to Cypher syntax.

Core node kinds: `Target` (label set, subjects-of, objects-of, relationship type), `ValueSet(path)`, value predicates (`Datatype`, `Range`, `Length`, `Pattern`, `In`, `NodeKind`, `Class`), set predicates (`CountRange`, `HasValue`, `PairCompare`, `Closed`), and logical `And`/`Or`/`Not`/`Xone`/`Conforms`. Lowering is described in [[semantics#Violations and Conforms]].

## Dialect Backends

Each backend renders IR into Cypher text and declares which IR constructs it can express.

### Neo4j

Targets Neo4j 5 Cypher without APOC, using `IS ::` type predicates, `COUNT {}` and `EXISTS {}` subqueries, and label expressions.

Schema snapshot is optional; without it, resolution falls back to convention. Snapshot can be dumped from `db.schema.nodeTypeProperties()` and `db.schema.relTypeProperties()`.

Spike findings (Neo4j 5.26), which renderers must respect:

- `null IS :: T` is true; type checks use `IS :: T NOT NULL`. Union types (`IS :: ZONED DATETIME | LOCAL DATETIME`) work; `LOCAL DATETIME` is not `ZONED DATETIME`.
- `size()` on a non-string, non-list value raises a runtime error, whereas `=~` and comparisons on mismatched types return null. Length and pattern predicates are therefore type-guarded, not just coalesced — see [[semantics#Null Safety]].
- `=~` is fully anchored Java regex, supports backreferences, and raises a runtime error on an invalid pattern.
- The static type checker rejects some literal-only expressions (e.g. `size(5)`), so renderer tests use property values, not literals.
- Confirmed: `COUNT {}` and `EXISTS {}` in `WHERE` and in boolean expressions, nested `EXISTS`, label expressions `n:A|B`, `keys(n)`, map rows, `collect(…)[0..k]`, `LIMIT coalesce(…)`, `replace()` for messages.

### LadybugDB

Targets LadybugDB's strictly-typed Cypher (Kùzu lineage): one table per node, rel tables with declared FROM/TO pairs, and typed columns.

The schema snapshot is required because the binder rejects references to undeclared tables or columns (`Cannot find property`, `Table X does not exist`).

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
