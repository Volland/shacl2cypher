---
lat:
  require-code-mention: true
---
# Tests

Specifications of the behaviours that define shacl2cypher; `lat check` requires each leaf section to be referenced by the test that covers it.

## Loading

Assembling many shapes files into one graph with locations, imports and conflict checks.

### Shapes Split Across Files

A shape declared in one file and extended in another compiles as one shape; input order does not change the graph.

### Import Cycles

Files importing each other through `owl:imports` are each loaded once and compilation proceeds.

### Remote Imports Need Opt-In

An `https://` import fails compilation, naming the import, unless `--allow-remote-imports` is set.

### Conflicting Settings List Every Location

Conflicting values for a single-valued setting across files are one error that lists every source location.

### Recursion Is Rejected

Shapes that reach themselves through `sh:node`, logical or qualified constraints are reported as recursion.

## Mapping

Resolving SHACL classes and paths to labels, properties and relationships.

### Convention Names

Without annotations, local names become labels and property keys and UPPER_SNAKE_CASE relationship types.

### Schema Disambiguates Paths

A path without hints is a relationship when the snapshot has the relationship type and no such property.

### Schema Outweighs Hints

A declared property with no same-named relationship type stays a property even under `sh:nodeKind sh:IRI` or `sh:class`.

### Strict Mode

With `--strict`, a name chosen by convention alone is a compile error.

### Subclass Expansion

Class targets and `sh:class` include transitive `rdfs:subClassOf` subclasses in a stable order.

## Compilation

Lowering constraints to null-safe, dialect-correct queries.

### Null-Safe Queries

No rendered query compares a value directly under `NOT`, so three-valued logic cannot hide a violation.

### Explained Nested Failures

Logical branches report failed inner constraints as detail ids extended with the branch index.

### Regex Normalization

XSD character class subtraction is expanded to explicit ranges that both engines accept.

### Schema-Decided Constraints

On an enforced schema, guaranteed constraints need no query and contradicted ones become static diagnostics.

### Unsupported Features

Rejected features are compile errors, or `unsupported` rules with `--lenient`.

### Unrepresentable Neo4j Identifiers

Identifiers containing `\uXXXX`, which Neo4j decodes inside backticks, fail rendering instead of matching the wrong name.

### LadybugDB Limits

Constructs LadybugDB cannot evaluate, such as regex back-references, are rejected at render time.

## Manifest

Stable names, fingerprints and deterministic output files.

### Compile Outputs

`compile -o out/` writes `manifest.json` and `queries.cypher` with a name header per query, identically on reruns.

### Collision Hashes

Rules sharing a structural id get content hashes that survive moving shapes within a file.

### Explicit Names

`s2c:name` replaces generated name parts; names still duplicated are compile errors listing their locations.

### Fingerprints

Fingerprints change with a rule's meaning or dialect and are stable across runs.

### Order-Independent Output

Reordered `sh:property` values and shuffled triples leave rule names, ids, fingerprints and queries unchanged.

### Cypher Snapshots

The generated `.cypher` file per dialect is snapshotted so query-shape changes show in review.

## Runner

Executing queries safely and reporting results for humans and CI.

### Summaries Drive Details

Every rule runs its summary; only rules with violations run their detail query.

### Severity Exit Codes

Warning-only violations exit 0 at `--fail-on violation` and 1 at `--fail-on warning`.

### Query Timeouts

A timed-out rule reports `status: timeout`, other rules still run, and the exit code is 3.

### JUnit Report

JUnit output has one test case per rule, with failures, errors and skips counted.

### SARIF Locations

Each SARIF result points at the shapes file and line of the violated constraint.

### Read-Only LadybugDB

LadybugDB files open read-only, writes fail, a missing file is not created, and slow queries time out.

### Rolled-Back Neo4j Queries

Writes inside Neo4j queries never persist, and a timed-out query leaves the connection usable.

### Compile-Only Builds

Without backend features, `validate` and `schema dump` report that no database backend is available.

### Setup Error Exit Codes

Compile errors exit 1 without writing files; usage errors exit 2.

## Conformance

Agreement with reference engines, the W3C suite and both databases.

### Fixtures on LadybugDB

Every fixture loads into a fresh LadybugDB file, validates end to end and matches `expect`, including `details`.

### Fixtures on Neo4j

Every fixture loads into Neo4j, validates end to end and matches `expect`, including `details`.

### W3C Core Suite

Every W3C SHACL Core test outcome on Neo4j matches `tests/w3c/status.yaml`.

### Literal Round-Trips

Random strings, identifiers and typed constants come back unchanged from Neo4j.

### Fixture Projection

Every fixture loads, validates its ids and projects to RDF and to each LPG dialect it declares.

### Known Differences

Documented engine differences are ignored only for their engine, and expected `details` must match exactly.
