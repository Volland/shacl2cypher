# Mapping

How SHACL's RDF terms (classes, predicates, literals, IRIs) are mapped onto LPG labels, tables, properties, relationship types and values.

## Resolution

Each class and path IRI resolves to an LPG element by naming convention, overridden by `s2c:` annotations, and cross-checked against the schema snapshot when present.

Resolution order for a path step: a shape-level `s2c:` annotation, then an annotation on the predicate IRI, then structural hints and schema snapshot evidence, then convention. Shape-level annotations only apply to simple paths (a predicate or its inverse). See [[dialects#Schema Awareness]].

### Convention

Without annotations, local names (after the last `#`, `/` or `:`) are used verbatim for labels and property keys; relationship types use UPPER_SNAKE_CASE of the local name.

Class labels come from the node shape's `s2c:label` when that shape has a single class target (with several it is an ambiguity error), then the class IRI's `s2c:label`, then the local name. A local name found in the schema snapshot counts as schema evidence.

### Relationship Detection

A path step is a relationship when annotated with `s2c:relationship` or `s2c:direction`, or when its position must yield nodes; otherwise hints and the schema decide, defaulting to a property.

- Positions that must yield nodes: inverse steps, repeated steps (`*`, `+`, `?`) and every step before the last in a sequence. An explicit `s2c:property` there is a compile error.
- Final-step hints: `sh:class`, `sh:node`, or `sh:nodeKind` of `sh:IRI`/`sh:BlankNode`/`sh:BlankNodeOrIRI` on the property shape. A hint is ignored when the snapshot declares the property on the focus labels and has no relationship type of that name. Schema facts outweigh hints, so a hint never turns a declared property into an always-empty relationship.
- Schema evidence: the snapshot has the conventional relationship type and no focus label declares a property with the local name (all node types are consulted when focus labels are unknown).
- Direction defaults to outgoing, `s2c:direction` overrides it, and `sh:inversePath` flips it. Alternatives must be all properties or all relationships.
- Combining `s2c:property` with `s2c:relationship` or `s2c:direction` on the same subject is a compile error.

### Strict Mode

With `--strict`, any class label, property key or relationship type chosen by convention alone (neither annotated nor found in the schema snapshot) is a compile error.

Intended for teams that want every mapping explicit and reviewable.

## Annotation Vocabulary

The `s2c:` namespace adds LPG-specific hints inside SHACL files. Standard SHACL engines ignore these triples, so shapes stay portable.

The namespace IRI is `https://w3id.org/shacl2cypher#` (declare `@prefix s2c: <https://w3id.org/shacl2cypher#> .`). String-valued annotations take plain string literals; `s2c:direction`, `s2c:collection` and `s2c:iriAsString` only accept the values listed below.

Parsing is strict so typos cannot silently fall back to convention. An unknown `s2c:` predicate, an invalid enumerated value, or an annotation on the wrong kind of subject is a compile error with its location. Wrong-subject examples are `s2c:key` on a property shape and `s2c:direction` on a node shape.

| Annotation | Subject | Meaning |
|---|---|---|
| `s2c:label` | class IRI or node shape | label / node table name |
| `s2c:property` | predicate or property shape | property key |
| `s2c:relationship` | predicate or property shape | relationship type / rel table |
| `s2c:direction` | predicate or property shape | `out` or `in` |
| `s2c:key` | node shape | per-label identifying property, see [[output#Focus Identity]] |
| `s2c:datatype` | predicate or property shape | native type override, see [[mapping#Datatypes]] |
| `s2c:collection` | predicate or property shape | `list` or `scalar`, see [[mapping#Value Sets]] |
| `s2c:iriAsString` | property shape | `local` (default) or `full` |
| `s2c:targetRelationship` | node shape | relationship focus target, see [[mapping#Relationship Targets]] |
| `s2c:name` | any shape | explicit rule name, see [[output#Rule Naming]] |

Conflicting single-valued annotations across files are compile errors — [[architecture#Input Assembly]].

## Datatypes

XSD datatypes map to native value types per dialect; overridable per predicate with `s2c:datatype`.

| XSD | Neo4j 5 | LadybugDB | FalkorDB |
|---|---|---|---|
| `xsd:string` | `IS :: STRING` | `STRING` | `typeOf(v) = 'String'` |
| `xsd:integer`, `long`, `int`, `short`, `byte` | `IS :: INTEGER` plus range check for narrow types | `INT64`/`INT32`/`INT16`/`INT8` | `'Integer'` plus range check for narrow types |
| `xsd:decimal`, `double`, `float` | `IS :: FLOAT` (decimal is lossy) | `DECIMAL`/`DOUBLE`/`FLOAT` | `'Float'` (decimal is lossy) |
| `xsd:boolean` | `IS :: BOOLEAN` | `BOOL` | `'Boolean'` |
| `xsd:date` | `IS :: DATE` | `DATE` | `'Date'` |
| `xsd:dateTime` | `IS :: ZONED DATETIME` or `LOCAL DATETIME` | `TIMESTAMP` | `'Datetime'` (local only) |
| `xsd:duration` | `IS :: DURATION` | `INTERVAL` | `'Duration'` |
| `xsd:anyURI` | `IS :: STRING` | `STRING` | `'String'` |
| `rdf:langString` | compile error | compile error | compile error |

Before dialect rendering, each XSD datatype maps to the neutral schema value types that can hold it:

- `xsd:string`, `normalizedString`, `token`, `anyURI` → `STRING`; `xsd:boolean` → `BOOLEAN`; `xsd:date` → `DATE`; `xsd:dateTime` → `ZONED_DATETIME` or `LOCAL_DATETIME`; `xsd:dateTimeStamp` → `ZONED_DATETIME`; `xsd:time` → `LOCAL_TIME` or `ZONED_TIME`; `xsd:duration` and its subtypes → `DURATION`.
- `xsd:integer` → any integer type. Derived types (`long`, `int`, `short`, `byte`, unsigned and sign-restricted types) also carry an inclusive value range.
- `xsd:decimal` → `DECIMAL`, `DOUBLE` or `FLOAT` (the last two lossy); `xsd:double` and `xsd:float` → `DOUBLE` or `FLOAT`.
- Any other datatype is a compile error.

`s2c:datatype` replaces the mapping with exactly one schema type name such as `LOCAL_DATETIME` or `LIST<STRING>`. It is case-insensitive and accepts spaces for underscores.

Against a declared column, a datatype constraint is either guaranteed (allowed type within range), needs a range check (e.g. `xsd:short` on `INT64`), needs a value check (`ANY`), or is contradicted. List columns are judged by their element type. On LadybugDB, guaranteed and contradicted constraints generate no query; see [[output#Static Diagnostics]].

## Value Sets

Every path yields a value set. A scalar property is one value, a list property is one value per element, and null, absent or empty list is the empty set.

Nulls inside lists are dropped. `s2c:collection "scalar"` forces a list to be treated as one LIST-typed value. Semantics over value sets are defined in [[semantics#Null Safety]].

## IRI Constants

IRI constants in `sh:in` and `sh:hasValue` become strings on property paths and key matches on relationship paths.

On a property path the IRI renders as its local name, or the full IRI with `s2c:iriAsString "full"`. On a relationship path it matches the end node's identifying key (`m.<key> = 'Active'`), see [[output#Focus Identity]].

## Class Hierarchy

`rdfs:subClassOf` triples from the shape files and optional `--ontology` files are expanded statically into targets and `sh:class` checks.

- Neo4j: `--neo4j-labels explicit` (default) expands to `(n:Person|Employee)`; `inherited` assumes nodes already carry all superclass labels and skips expansion.
- FalkorDB: `--neo4j-labels` works as on Neo4j. FalkorDB has no label expressions, so expansion renders `WHERE n:Person OR n:Employee`.
- LadybugDB: always expands, since a node belongs to exactly one table; multi-table patterns like `(n:Person:Employee)` match any listed table.
- Expansion is transitive, ignores reflexive `rdfs:subClassOf` statements and lists classes sorted by IRI, so generated patterns are deterministic.
- Subclass cycles are compile errors listing every `rdfs:subClassOf` statement of the cycle with its location.
- A target class with no table (and no subclass tables) is a static schema mismatch on LadybugDB.

## Relationship Targets

`s2c:targetRelationship "TYPE"` makes each relationship of that type a focus, so relationship properties can be validated with ordinary property shapes.

Only property-kind constraints apply (no traversal from an edge). Rows carry a relationship focus instead of a node focus — [[output#Focus Identity]]. Endpoint label rules remain on node shapes via `sh:class`.
