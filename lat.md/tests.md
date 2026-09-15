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

### In-Memory Documents

Shapes given as named text compile to the byte-identical manifest of the same files in the base directory, and report `name:line` locations.

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

### FalkorDB Queries

Every compiled rule renders for FalkorDB without write clauses, pattern predicates or comprehensions, `EXISTS`/`COUNT` subqueries, or string functions over raw values.

### FalkorDB Constants

FalkorDB receives only temporal constants it holds exactly; zoned, fractional, invalid-calendar and duration constants are compile errors naming FalkorDB.

### Unrepresentable FalkorDB Identifiers

Labels, relationship types and property keys containing a backtick fail rendering, because FalkorDB cannot escape a backtick inside an identifier.

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

### Database Worker

A worker thread owns one executor, completes concurrent jobs in turn, rejects jobs after close and returns open errors.

### Read-Only FalkorDB

FalkorDB graphs are queried read-only: writes fail, a missing graph is an error and is not created, slow queries time out, the schema dump round-trips, and manifests of other dialects are rejected.

### FalkorDB Setup Errors

TLS URLs, unreachable servers and missing graphs make `validate --falkordb` exit 2 with a message naming the problem.

## Python Binding

The `shacl2cypher` PyPI package, tested with pytest against the CLI and fixture LadybugDB files — see [[bindings#Testing]].

### Compile From Files And Documents

`compile` accepts paths and `Source` documents, returns manifest dict, manifest JSON and Cypher, and rejects an empty shapes list.

### Output Parity With The CLI

Manifest JSON and Cypher from `compile` equal the CLI's `manifest.json` and `queries.cypher` byte for byte.

### Typed Errors

`CompileError.errors` lists every problem with its `name:line`, nothing is written, and every error class derives from `Shacl2CypherError`.

### Remote Imports

Remote imports fail without opt-in, and with it a body over 16 MB fails naming the import and the limit.

### Backend Availability

A compile-only build reports no backends and raises `BackendUnavailableError` when a database is opened.

### Database Handles

A missing LadybugDB file raises `DatabaseConnectionError` without creating it; closed handles raise `DatabaseClosedError`.

### Validate

`validate` runs shapes or manifests with limits, rejects other-dialect manifests with `ManifestError` and non-positive timeouts with `ValueError`.

### Reports And Exit Codes

Reports equal the CLI's JSON report apart from timings, render in every format and score exit codes by `fail_on`.

### Schema Dump

`schema_json` equals `schema dump` output, and the parsed snapshot compiles to the same manifest as that text.

### Non-Blocking Execution

A validation of 300 rules releases the GIL, so another thread keeps counting while it runs.

### Published Types

Manifest, report and schema dicts have exactly the keys their `TypedDict`s declare.

### Shared Parity Cases

Every case in `tests/bindings/cases.json` (split files, a local import, a schema-backed LadybugDB compile) matches the CLI's files byte for byte.

### Report Parity

A LadybugDB fixture report renders as table, JSON, JUnit and SARIF exactly like the CLI once per-run timings are zeroed.

## Node Binding

The `shacl2cypher` npm package, tested with `node:test` against the CLI and fixture LadybugDB files — see [[bindings#Testing]].

### Compile From Files And Documents

`compile` and `compileSync` accept paths and `{ name, text }` documents, agree with each other, and reject an empty shapes list.

### Output Parity With The CLI

`manifestJson` and `cypher` equal the CLI's `manifest.json` and `queries.cypher` byte for byte.

### Typed Errors

`CompileError.errors` lists every problem with its `name:line`, nothing is written, and every error class derives from `Shacl2CypherError`.

### Remote Imports

Remote imports reject without opt-in, and with it a body over 16 MB rejects naming the import and the limit.

### Backend Availability

A compile-only build rejects `Ladybug.open` with `BackendUnavailableError`.

### Database Handles

A missing LadybugDB file rejects with `DatabaseConnectionError` without creating it; closed handles reject with `DatabaseClosedError`.

### Validate

`validate` runs shapes or manifests with limits, rejects other-dialect manifests with `ManifestError` and negative `timeoutMs` with `RangeError`.

### Reports And Exit Codes

Reports equal the CLI's JSON report apart from timings, `renderReport` covers every format and `exitCode` follows `failOn`.

### Schema Dump

The snapshot from `db.schema()` compiles to the same manifest as the CLI's `schema dump` file.

### Non-Blocking Execution

A 10 ms timer fires before a validation of 300 rules settles, so the event loop stays free.

### Published Types

Manifest, report and schema objects have exactly the keys `index.d.ts` declares, read with the TypeScript compiler API.

### Shared Parity Cases

Every case in `tests/bindings/cases.json` (split files, a local import, a schema-backed LadybugDB compile) matches the CLI's files byte for byte.

### Report Parity

A LadybugDB fixture report renders as table, JSON, JUnit and SARIF exactly like the CLI once per-run timings are zeroed.

## Conformance

Agreement with reference engines, the W3C suite and both databases.

### Fixtures on LadybugDB

Every fixture loads into a fresh LadybugDB file, validates end to end and matches `expect`, including `details`.

### Fixtures on Neo4j

Every fixture loads into Neo4j, validates end to end and matches `expect`, including `details`.

### Fixtures on FalkorDB

Every fixture FalkorDB can store loads into its own FalkorDB graph, validates end to end and matches `expect`, including `details`.

### Literal Round-Trips on FalkorDB

Random strings, representable identifiers and typed constants come back unchanged from FalkorDB.

### W3C Core Suite

Every W3C SHACL Core test outcome on Neo4j matches `tests/w3c/status.yaml`.

### Literal Round-Trips

Random strings, identifiers and typed constants come back unchanged from Neo4j.

### Fixture Projection

Every fixture loads, validates its ids and projects to RDF and to each LPG dialect it declares.

### Known Differences

Documented engine differences are ignored only for their engine, and expected `details` must match exactly.
