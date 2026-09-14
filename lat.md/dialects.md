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

### LadybugDB

Targets LadybugDB's strictly-typed Cypher (Kùzu lineage): one table per node, rel tables with declared FROM/TO pairs, and typed columns.

The schema snapshot is required because the binder rejects references to undeclared tables or columns. Nested subqueries inside expressions are an open risk: if unsupported, the backend hoists them into `OPTIONAL MATCH … WITH` chains. The first implementation spike verifies this.

## Schema Awareness

The compiler consumes a dialect-neutral schema snapshot describing node types, relationship types with endpoints, and typed properties.

Format: `nodeTypes[{name, properties[{name, type}]}]`, `relTypes[{name, from[], to[], properties[]}]`. Obtained from a JSON file or live introspection via the runner. The snapshot drives three things: static diagnostics for missing elements ([[output#Static Diagnostics]]), skipping constraints guaranteed by the schema (`status: guaranteed-by-schema`), and disambiguating property vs relationship paths ([[mapping#Resolution]]).

## Literals and Identifiers

All shape constants are inlined through a single typed literal renderer per dialect; identifiers are always backtick-escaped.

- String escaping, number formatting (NaN/Infinity rejected) and temporal constructors live in one renderer; building literals ad hoc with string formatting is forbidden.
- Identifiers are backticked with inner backticks doubled; names a target cannot represent are compile errors.
- Runtime parameters are reserved for execution controls: `$limit`, `$sampleSize`.
- `sh:message` placeholders (`{$this}`, `{?value}`) are substituted in Cypher at runtime so messages carry actual values.
