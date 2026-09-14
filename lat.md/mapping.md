# Mapping

How SHACL's RDF terms (classes, predicates, literals, IRIs) are mapped onto LPG labels, tables, properties, relationship types and values.

## Resolution

Each class and path IRI resolves to an LPG element by naming convention, overridden by `s2c:` annotations, and cross-checked against the schema snapshot when present.

Resolution order for a path: explicit `s2c:` annotation, then schema snapshot evidence (existing column vs rel table), then convention. See [[dialects#Schema Awareness]].

### Convention

Without annotations, local names are used verbatim for labels and property keys; relationship types use UPPER_SNAKE_CASE of the local name.

A path is a **relationship** only if its property shape has `sh:class`, `sh:node`, or `sh:nodeKind` of `sh:IRI`/`sh:BlankNode`/`sh:BlankNodeOrIRI`; otherwise it is a **property**. Relationship direction defaults to outgoing; `sh:inversePath` flips it.

### Strict Mode

With `--strict`, any class or path resolved by convention alone (no annotation, no schema evidence) is a compile error.

Intended for teams that want every mapping explicit and reviewable.

## Annotation Vocabulary

The `s2c:` namespace adds LPG-specific hints inside SHACL files. Standard SHACL engines ignore these triples, so shapes stay portable.

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

| XSD | Neo4j 5 | LadybugDB |
|---|---|---|
| `xsd:string` | `IS :: STRING` | `STRING` |
| `xsd:integer`, `long`, `int`, `short`, `byte` | `IS :: INTEGER` plus range check for narrow types | `INT64`/`INT32`/`INT16`/`INT8` |
| `xsd:decimal`, `double`, `float` | `IS :: FLOAT` (decimal is lossy) | `DECIMAL`/`DOUBLE`/`FLOAT` |
| `xsd:boolean` | `IS :: BOOLEAN` | `BOOL` |
| `xsd:date` | `IS :: DATE` | `DATE` |
| `xsd:dateTime` | `IS :: ZONED DATETIME` or `LOCAL DATETIME` | `TIMESTAMP` |
| `xsd:duration` | `IS :: DURATION` | `INTERVAL` |
| `xsd:anyURI` | `IS :: STRING` | `STRING` |
| `rdf:langString` | compile error | compile error |

On LadybugDB, a datatype constraint on a declared column is resolved statically: guaranteed-by-schema or a schema mismatch. No query is generated.

## Value Sets

Every path yields a value set. A scalar property is one value, a list property is one value per element, and null, absent or empty list is the empty set.

Nulls inside lists are dropped. `s2c:collection "scalar"` forces a list to be treated as one LIST-typed value. Semantics over value sets are defined in [[semantics#Null Safety]].

## IRI Constants

IRI constants in `sh:in` and `sh:hasValue` become strings on property paths and key matches on relationship paths.

On a property path the IRI renders as its local name, or the full IRI with `s2c:iriAsString "full"`. On a relationship path it matches the end node's identifying key (`m.<key> = 'Active'`), see [[output#Focus Identity]].

## Class Hierarchy

`rdfs:subClassOf` triples from the shape files and optional `--ontology` files are expanded statically into targets and `sh:class` checks.

- Neo4j: `--neo4j-labels explicit` (default) expands to `(n:Person|Employee)`; `inherited` assumes nodes already carry all superclass labels and skips expansion.
- LadybugDB: always expands, since a node belongs to exactly one table; multi-table patterns like `(n:Person:Employee)` match any listed table.
- Subclass cycles are compile errors.
- A target class with no table (and no subclass tables) is a static schema mismatch on LadybugDB.

## Relationship Targets

`s2c:targetRelationship "TYPE"` makes each relationship of that type a focus, so relationship properties can be validated with ordinary property shapes.

Only property-kind constraints apply (no traversal from an edge). Rows carry a relationship focus instead of a node focus — [[output#Focus Identity]]. Endpoint label rules remain on node shapes via `sh:class`.
