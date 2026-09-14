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

### Lowering Rules

Each targeted shape yields one rule per constraint of its own and of its property shapes; shapes without targets only contribute through `conforms`.

- Per-focus rows: `sh:minCount`, `sh:maxCount`, `sh:hasValue`, qualified counts, `sh:equals`/`sh:disjoint`/`sh:lessThan`/`sh:lessThanOrEquals` and `sh:closed`. Per-value rows: the other value tests and `sh:node`, `sh:not`, `sh:and`, `sh:or`, `sh:xone`.
- Value kinds are decided statically:
  - Literal tests (datatype, ranges, lengths, pattern) fail on node values, and `sh:class` fails on property values.
  - `sh:nodeKind` follows the path kind: nodes are IRIs, property values are literals, and blank nodes never occur.
  - Checks that always hold get `status: guaranteed-by-schema`.
- IRI constants in `sh:in` and `sh:hasValue` compare as local names (full IRIs with `s2c:iriAsString "full"`) with property values. They compare with node values through `--node-key`, which is required in that case.
- `sh:closed` works on node shapes; allowed keys are its property shapes' property keys plus `sh:ignoredProperties`.
- `sh:qualifiedValueShapesDisjoint` excludes values conforming to the qualified value shapes of sibling property shapes of the same parent.
- Unbounded repeated paths are capped at `--max-path-depth` (default 10), recorded on the rule. A depth above the dialect limit is an error.
- On a relationship focus, only value constraints on single-property paths are allowed.
- Rejected features (`sh:sparql`, `sh:targetNode`, `sh:languageIn`, `sh:uniqueLang`, `rdf:langString`, recursive references) are errors, or rules with `status: unsupported` under `--lenient`. Deactivated shapes yield `status: deactivated` rules.
- Constraints contradicted by an enforced schema yield `status: schema-mismatch` plus a static diagnostic ([[output#Static Diagnostics]]).

### Explained Nested Failures

Rows for nested-shape constraints always include `details`: the rule ids of the inner constraints that failed for that value.

This goes beyond SHACL's standard report, which only names the outer failure. Details are computed from the same per-constraint predicates used by `conforms`.

A named inner shape reports its own rule ids (`ex:AddressShape/ex:zip/sh:minCount`). A blank inner shape extends the outer rule id with its branch (`ex:PersonShape/sh:or[0]/ex:email/sh:minCount`).

## Null Safety

Constraints quantify over value sets ([[mapping#Value Sets]]). Every generated predicate is null-safe so Cypher's three-valued logic never hides a violation.

1. Value constraints (`sh:datatype`, ranges, lengths, `sh:pattern`, `sh:in`, `sh:nodeKind`, `sh:class`) are vacuously satisfied by an empty value set. Only counting constraints and `sh:hasValue` detect absence.
2. Each value predicate is wrapped as `coalesce(pred(v), false)` before negation; a type-incompatible comparison is a violation.
3. Scalars are normalized to lists at runtime unless the schema snapshot proves the column scalar, in which case renderers specialize.
4. On LadybugDB a NULL in a declared column is the same as absent.
5. No renderer may emit a bare comparison under `NOT`; a lint test in the core crate enforces this.
6. Predicates that raise runtime errors on mismatched types (Neo4j `size()`) are guarded by a type test, e.g. `CASE WHEN v IS :: STRING NOT NULL THEN size(v) >= 3 ELSE false END`; `coalesce` alone cannot catch an error.

## Regex Translation

`sh:pattern` uses XSD regex with substring-match semantics. Patterns are parsed into a regex AST and rendered per dialect; they are never passed through verbatim.

- Neo4j `=~` is Java regex and fully anchored, so patterns are wrapped as `(?s).*(?:PATTERN).*` with `sh:flags` mapped to inline flags. Java supports backreferences, so they are allowed on Neo4j.
- LadybugDB `=~` is also fully anchored; the renderer uses RE2 substring matching via `regexp_matches(v, 'PATTERN')` with inline RE2 flags (`(?i)`, `(?s)`, `(?m)` confirmed). Backslashes are doubled inside the string literal.
- RE2 silently returns false for backreferences instead of failing, so they must be rejected at compile time for LadybugDB.
- Patterns are first normalized to the syntax Java and RE2 share, then validated with a full regex parser.
- `\i`, `\c`, `\I` and `\C` expand to XML name-character classes. Character classes using subtraction (`[a-z-[aeiou]]`) or negated name escapes are expanded to explicit `\x{…}` ranges, since neither engine supports XSD subtraction. `&` and `~` inside classes are escaped.
- Flags: `s`, `m` and `i` are kept for rendering; `x` removes whitespace outside character classes and `q` escapes the whole pattern. Any other flag is a compile error.
- Unicode block escapes (`\p{IsBasicLatin}`) are compile errors. Back-references are kept and flagged so each dialect can accept or reject them.
- Both engines fail at runtime on an invalid pattern, so every pattern is validated at compile time; errors point to the source span.
