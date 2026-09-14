# shacl2cypher

Compile [SHACL](https://www.w3.org/TR/shacl/) shapes into named, read-only Cypher queries that validate a labeled property graph and point at every broken rule.

shacl2cypher reads a set of SHACL files and maps classes and paths to labels, properties and relationships. It writes one detail query and one summary query per constraint. It can also run those queries against **Neo4j 5** or **LadybugDB** and report violations as a table, JSON, JUnit or SARIF.

```text
$ shacl2cypher validate shapes.ttl --ladybug graph.lbug --node-key id
STATUS      SEVERITY   VIOLATIONS  TIME(ms)  RULE
failed      Violation           1        31  PersonShape.name.minCount
            - Person id=p3: Person needs a name
failed      Warning             1         3  PersonShape.name.pattern
            - Person id=p1: ex:PersonShape violates sh:pattern on ex:name (value: "ann")
failed      Violation           1         8  PersonShape.worksFor.maxCount
            - Person id=p1: ex:PersonShape violates sh:maxCount on ex:worksFor

3 rules: 0 passed, 3 failed, 0 timed out, 0 errors, 0 skipped, 0 guaranteed by schema; 3 violations in 42 ms
```

## Contents

- [Build](#build)
- [Quick start](#quick-start)
- [How shapes map to the graph](#how-shapes-map-to-the-graph)
- [The `s2c:` annotation vocabulary](#the-s2c-annotation-vocabulary)
- [Supported SHACL](#supported-shacl)
- [Output](#output)
- [Validation reports and exit codes](#validation-reports-and-exit-codes)
- [Development](#development)

## Build

Requires Rust 1.87 or newer. Database backends are optional cargo features, so a compile-only binary contains no database client code.

```sh
cargo build --release -p s2c-cli                          # compile only
cargo build --release -p s2c-cli --features neo4j         # + Neo4j (Bolt)
cargo build --release -p s2c-cli --features ladybug       # + LadybugDB (embedded; builds C++, needs cmake)
cargo build --release -p s2c-cli --features neo4j,ladybug
```

The binary is `target/release/shacl2cypher`.

## Quick start

The shapes file behind the report above:

```turtle
@prefix sh:  <http://www.w3.org/ns/shacl#> .
@prefix ex:  <http://example.org/> .

ex:PersonShape sh:targetClass ex:Person ;
    sh:property [ sh:path ex:name ; sh:minCount 1 ; sh:message "Person needs a name" ] ,
                [ sh:path ex:name ; sh:pattern "^[A-Z]" ; sh:severity sh:Warning ] ,
                [ sh:path ex:worksFor ; sh:maxCount 1 ] .
```

### Compile to queries

```sh
shacl2cypher compile shapes/*.ttl --dialect neo4j --node-key id -o out/
```

This writes `out/manifest.json` and `out/queries.cypher`. Each query in the `.cypher` file is headed by `// name: <name>` (summaries by `// name: <name>#summary`), so the file can be pasted into any Cypher client. Queries take two parameters, `$limit` and `$sampleSize`.

LadybugDB tables are typed, so compiling for it needs a schema snapshot:

```sh
shacl2cypher schema dump --ladybug ./graph.lbug -o schema.json
shacl2cypher compile shapes/*.ttl --dialect ladybug --schema schema.json --node-key id -o out/
```

### Validate a database

```sh
# Neo4j: every query runs in a transaction that is always rolled back
NEO4J_PASSWORD=secret shacl2cypher validate shapes/*.ttl \
    --connect bolt://localhost:7687 --node-key id

# LadybugDB: the file is opened read-only; the schema is read from it
shacl2cypher validate shapes/*.ttl --ladybug ./graph.lbug --node-key id --format sarif -o report.sarif

# Run a manifest compiled earlier, e.g. in CI
shacl2cypher validate --manifest out/manifest.json --connect bolt://localhost:7687 --format junit -o junit.xml
```

Useful options: `--timeout <seconds>` per query, `--limit` for violations listed per rule (`0` lists all), `--fail-on violation|warning|info`, and `--verbose` to include all focus properties in rows. Run `shacl2cypher <command> --help` for the full list.

## How shapes map to the graph

Resolution is convention first, then evidence from the schema snapshot, then explicit `s2c:` annotations, which always win.

- **Labels** use the class IRI's local name: `ex:Person` becomes `:Person`. Class targets and `sh:class` include `rdfs:subClassOf` subclasses from the shapes and `--ontology` files. With `--neo4j-labels inherited`, Neo4j matches only the class label, because nodes carry their superclass labels.
- **Properties** use the predicate's local name: `ex:name` becomes `name`.
- **Relationships** use UPPER_SNAKE_CASE: `ex:worksFor` becomes `WORKS_FOR`, outgoing. A path step is a relationship when:
  - it is inverse, repeated, or not the last step of a sequence, or
  - its property shape has `sh:class`, `sh:node`, or `sh:nodeKind sh:IRI`/`sh:BlankNode`/`sh:BlankNodeOrIRI`, or
  - the schema snapshot declares that relationship type and no such property.

  A property declared in the snapshot always beats a structural hint.
- **Value sets:** a scalar property is one value, a list property is one value per element, and `null`, absent and empty lists are the empty set.
- **IRI constants** in `sh:in` and `sh:hasValue` compare as local names on properties, and as the node key on relationships.
- **Focus identity:** rows identify nodes by the shape's `s2c:key`, else `--node-key`, plus the database element id.

`--strict` rejects any name chosen by convention alone. That keeps every mapping explicit and reviewable.

## The `s2c:` annotation vocabulary

Declare `@prefix s2c: <https://w3id.org/shacl2cypher#> .` Standard SHACL engines ignore these triples, so annotated shapes stay portable. Parsing is strict: an unknown `s2c:` predicate, an invalid value, or an annotation on the wrong kind of subject is a compile error.

| Annotation | Subject | Meaning | Example |
|---|---|---|---|
| `s2c:label` | class IRI or node shape | label / node table name | `ex:Person s2c:label "Human"` |
| `s2c:property` | predicate or property shape | property key | `ex:fullName s2c:property "name"` |
| `s2c:relationship` | predicate or property shape | relationship type / rel table | `ex:employer s2c:relationship "WORKS_FOR"` |
| `s2c:direction` | predicate or property shape | `"out"` or `"in"` | `ex:employs s2c:direction "in"` |
| `s2c:key` | node shape | identifying property of the focus label | `ex:PersonShape s2c:key "email"` |
| `s2c:datatype` | predicate or property shape | exact native type instead of the XSD mapping | `s2c:datatype "LOCAL_DATETIME"` |
| `s2c:collection` | predicate or property shape | `"list"` (one value per element) or `"scalar"` (the list is one value) | `s2c:collection "scalar"` |
| `s2c:iriAsString` | property shape | IRI constants as `"local"` names (default) or `"full"` IRIs | `s2c:iriAsString "full"` |
| `s2c:targetRelationship` | node shape | every relationship of this type is a focus, for relationship properties | `ex:KnowsShape s2c:targetRelationship "KNOWS"` |
| `s2c:name` | any shape | explicit rule name | `s2c:name "person-email"` |

Example: validating relationship properties with a shape.

```turtle
ex:KnowsShape s2c:targetRelationship "KNOWS" ;
    sh:property [ sh:path ex:since ; sh:datatype xsd:date ; sh:maxCount 1 ] .
```

## Supported SHACL

**Core constraints**

| Group | Features |
|---|---|
| Targets | `sh:targetClass`, implicit class targets, `sh:targetSubjectsOf`, `sh:targetObjectsOf`, `s2c:targetRelationship` |
| Cardinality | `sh:minCount`, `sh:maxCount` |
| Value type | `sh:datatype`, `sh:nodeKind`, `sh:class` |
| Ranges | `sh:minInclusive`, `sh:maxInclusive`, `sh:minExclusive`, `sh:maxExclusive` |
| Strings | `sh:minLength`, `sh:maxLength`, `sh:pattern` with `sh:flags` (`i`, `s`, `m`, `x`, `q`) |
| Other | `sh:in`, `sh:hasValue`, `sh:equals`, `sh:disjoint`, `sh:lessThan`, `sh:lessThanOrEquals`, `sh:closed`, `sh:ignoredProperties` |
| Metadata | `sh:severity`, `sh:message` (with `{$this}`/`{?value}`), `sh:deactivated` |

**Shape composition and paths**

| Group | Features |
|---|---|
| Logical | `sh:node`, `sh:not`, `sh:and`, `sh:or`, `sh:xone` |
| Qualified | `sh:qualifiedValueShape` with `sh:qualifiedMinCount`, `sh:qualifiedMaxCount`, `sh:qualifiedValueShapesDisjoint` |
| Paths | sequence, `sh:inversePath`, `sh:alternativePath`, `sh:zeroOrMorePath`, `sh:oneOrMorePath`, `sh:zeroOrOnePath` (unbounded repeats capped by `--max-path-depth`, default 10) |

**Rejected at compile time.** With `--lenient` these become `unsupported` rules instead:

- `sh:targetNode`
- SHACL-SPARQL
- `sh:languageIn`, `sh:uniqueLang` and `rdf:langString`
- recursive shapes

Behaviour differences from RDF engines:

- `sh:closed` also forbids undeclared outgoing relationships, and always allows the node key.
- Length constraints apply to strings and integers only.
- XSD regexes are parsed and rewritten for each engine. LadybugDB rejects back-references.
- Property shapes nested inside property shapes are not supported yet.

Conformance is checked with 24 fixtures on Neo4j and LadybugDB (23 of them also against pySHACL) and with the W3C SHACL Core suite on Neo4j. 27 W3C tests pass; every other outcome and its reason is recorded in [`tests/w3c/status.yaml`](tests/w3c/status.yaml).

## Output

`manifest.json` is the primary artifact:

- **Provenance:** compiler version, dialect, options, each input with its SHA-256, and the schema snapshot hash.
- **Rules**, one per constraint, each with:
  - `name` and structural `ruleId`, e.g. `ex:PersonShape/ex:name/sh:minCount`
  - `fingerprint`
  - `severity`
  - `status`: `compiled`, `guaranteed-by-schema`, `schema-mismatch`, `deactivated` or `unsupported`
  - `costClass`
  - `source` file and line
  - the detail and summary queries
- **Static diagnostics** and **recommended indexes**.

Output is deterministic: input order, triple order and blank-node labels never change names or queries.

Detail rows return `ruleId`, `shape`, `path`, `constraint`, `severity`, `focus`, `value`, `message` and `details`.

- `focus` is `{label, key, keyValue, elementId}`, or `{type, startKey, endKey, elementId}` for relationships.
- `details` lists the inner rules that failed for nested shapes, e.g. `["ex:AddressShape/ex:zip/sh:pattern"]`.

Summary rows return `ruleId`, `severity`, `violationCount` and a `sample` of focus objects, even when the count is zero.

## Validation reports and exit codes

| Format | Content |
|---|---|
| `table` | status, severity, count and time per rule, up to 10 violations each |
| `json` | the full report with every detail row and `durationMs` per rule |
| `junit` | one test case per rule; violations are failures, timeouts and query errors are errors |
| `sarif` | one result per violation, located at the constraint's shapes file and line |

| Exit code | Meaning |
|---|---|
| `0` | no rule at or above `--fail-on` has violations, and every query completed |
| `1` | a rule at or above `--fail-on` has violations |
| `2` | usage or setup error: bad arguments, compile errors, unreachable database, or no backend compiled in |
| `3` | nothing failed, but a query timed out or errored, so the result is incomplete |

## Development

The workspace has four crates:

- `s2c-core`: the pure compiler.
- `s2c-cli`: the `shacl2cypher` binary.
- `s2c-runner`: database execution, schema dumps and reports.
- `s2c-testkit`: conformance fixtures and the W3C harness.

```sh
cargo test --workspace                                       # compiler, CLI, reports
cargo test --workspace --features s2c-cli/ladybug            # + LadybugDB conformance fixtures and fuzzing
S2C_NEO4J_URI=bolt://localhost:7687 S2C_NEO4J_PASSWORD=secret \
  cargo test --workspace --features s2c-cli/neo4j            # + Neo4j fixtures, fuzzing, W3C suite
```

- **Fixtures** live in `tests/conformance/` as a YAML graph plus a shapes file and a hand-written `expect` block. Set `S2C_FIXTURE=<substring>` to run only matching fixtures.
- **pySHACL oracle:** `pip install -r tests/oracle/requirements.txt`, then `cargo build -p s2c-testkit --bin s2c-fixture` and run `python tests/oracle/pyshacl_oracle.py`.
- **Snapshots:** accept intended Cypher changes with `INSTA_UPDATE=always cargo test -p s2c-core --test snapshots`.
- **W3C suite:** after an intended change, rerun with `S2C_W3C_BLESS=1` (and a database) and review the diff of `tests/w3c/status.yaml`.

Design documentation lives in [`lat.md/`](lat.md/lat.md); check it with `lat check`. The change proposal and specs are in [`openspec/`](openspec/).
