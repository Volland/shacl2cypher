# Semantics

Validation semantics of the compiler: supported SHACL features, how nested shapes compose, null safety, and regex translation.

## Supported Features

Version 1 supports SHACL Core Tier 1 and Tier 2. Unsupported features are compile errors, or `status: unsupported` manifest entries with `--lenient`.

### Tier 1

Direct constraints that compile to one violation pattern each.

- Targets: `sh:targetClass`, implicit class targets, `sh:targetSubjectsOf`, `sh:targetObjectsOf`, plus [[mapping#Relationship Targets]].
- Cardinality: `sh:minCount`, `sh:maxCount`.
- Value type: `sh:datatype`, `sh:nodeKind`, `sh:class`.
- Value range: `sh:minInclusive`, `sh:maxInclusive`, `sh:minExclusive`, `sh:maxExclusive`.
- String: `sh:minLength`, `sh:maxLength`, `sh:pattern` with `sh:flags` — [[semantics#Regex Translation]].
- Other: `sh:in`, `sh:hasValue`, `sh:equals`, `sh:disjoint`, `sh:lessThan`, `sh:lessThanOrEquals`, `sh:closed` with `sh:ignoredProperties`.
- Metadata: `sh:severity`, `sh:message`, `sh:deactivated`.

### Tier 2

Shape-composing constraints and complex paths that require embedding one shape's conformance inside another query.

- Logical: `sh:node`, `sh:not`, `sh:and`, `sh:or`, `sh:xone`.
- Qualified: `sh:qualifiedValueShape` with `sh:qualifiedMinCount`/`sh:qualifiedMaxCount`/`sh:qualifiedValueShapesDisjoint`.
- Paths: sequence paths, `sh:inversePath`, `sh:alternativePath`, `sh:zeroOrMorePath`, `sh:oneOrMorePath`, `sh:zeroOrOnePath`. Unbounded paths are capped by `--max-path-depth` — [[output#Cost Classes]].

### Rejected

Features that have no meaning on an LPG, or that the design explicitly excludes, are rejected at compile time.

`sh:languageIn` and `sh:uniqueLang` (no language tags), `rdf:langString`, `sh:targetNode`, `sh:sparql` and SPARQL-based components, and recursive shape references (cycles through `sh:node`/logical constraints).

## Violations and Conforms

Every shape compiles two ways in the IR: `violations(shape)` produces diagnostic rows, and `conforms(shape, var)` produces a boolean expression for embedding.

`conforms` is the null-safe conjunction of the shape's constraint predicates. `sh:and`→`AND`, `sh:or`→`OR`, `sh:not`→`NOT`, `sh:xone`→exactly-one count over branch booleans, `sh:node`→inlined `conforms` of the referenced shape, `sh:qualifiedValueShape`→`COUNT {}` of neighbours satisfying `conforms`. Inlining is why recursive shapes are rejected.

### Explained Nested Failures

Rows for nested-shape constraints always include `details`: the rule ids of the inner constraints that failed for that value.

This goes beyond SHACL's standard report, which only names the outer failure. Details are computed from the same per-constraint predicates used by `conforms`.

## Null Safety

Constraints quantify over value sets ([[mapping#Value Sets]]). Every generated predicate is null-safe so Cypher's three-valued logic never hides a violation.

1. Value constraints (`sh:datatype`, ranges, lengths, `sh:pattern`, `sh:in`, `sh:nodeKind`, `sh:class`) are vacuously satisfied by an empty value set. Only counting constraints and `sh:hasValue` detect absence.
2. Each value predicate is wrapped as `coalesce(pred(v), false)` before negation; a type-incompatible comparison is a violation.
3. Scalars are normalized to lists at runtime unless the schema snapshot proves the column scalar, in which case renderers specialize.
4. On LadybugDB a NULL in a declared column is the same as absent.
5. No renderer may emit a bare comparison under `NOT`; a lint test in the core crate enforces this.

## Regex Translation

`sh:pattern` uses XSD regex with substring-match semantics. Patterns are parsed into a regex AST and rendered per dialect; they are never passed through verbatim.

- Neo4j `=~` is Java regex and fully anchored, so patterns are wrapped as `(?s).*(?:PATTERN).*` with `sh:flags` mapped to inline flags.
- LadybugDB uses RE2 substring matching (`regexp_matches`) with RE2 flag syntax.
- XSD-specific syntax (`\i`, `\c`, class subtraction) is expanded when possible.
- Constructs a dialect cannot express (e.g. backreferences on RE2) and invalid patterns are compile errors pointing to the source span.
