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

- [Install](#install)
- [Build](#build)
- [Quick start](#quick-start)
- [Python and TypeScript](#python-and-typescript)
- [How shapes map to the graph](#how-shapes-map-to-the-graph)
- [The `s2c:` annotation vocabulary](#the-s2c-annotation-vocabulary)
- [Supported SHACL](#supported-shacl)
- [Output](#output)
- [Validation reports and exit codes](#validation-reports-and-exit-codes)
- [Development](#development)

## Install

Prebuilt binaries with both database backends are attached to each [GitHub release](https://github.com/Volland/shacl2cypher/releases). They cover Linux x86_64, Linux arm64 and macOS arm64, and each tarball has a SHA-256 checksum next to it.

With cargo:

```sh
cargo install --locked shacl2cypher                              # compile only
cargo install --locked shacl2cypher --features neo4j,ladybug     # with database backends (LadybugDB builds C++, needs cmake)
```

`--locked` installs the dependency versions the release was tested with. It is required on Rust 1.87, where the newest versions of some transitive dependencies need a newer compiler.

The compiler and runner are also published as libraries: [`shacl2cypher-core`](https://crates.io/crates/shacl2cypher-core) and [`shacl2cypher-runner`](https://crates.io/crates/shacl2cypher-runner).

Python and Node.js packages bundle both database backends — see [Python and TypeScript](#python-and-typescript):

```sh
pip install shacl2cypher      # CPython 3.9+
npm install shacl2cypher      # Node.js 18+
```

## Build

Requires Rust 1.87 or newer. Database backends are optional cargo features, so a compile-only binary contains no database client code.

```sh
cargo build --release -p shacl2cypher                          # compile only
cargo build --release -p shacl2cypher --features neo4j         # + Neo4j (Bolt)
cargo build --release -p shacl2cypher --features ladybug       # + LadybugDB (embedded; builds C++, needs cmake)
cargo build --release -p shacl2cypher --features neo4j,ladybug
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

## Python and TypeScript

The `shacl2cypher` packages on PyPI and npm run the same compiler and runner in-process, without the CLI. Manifests, queries and reports are byte-identical to the CLI's, and their data uses the camelCase keys of `manifest.json` and `--format json`.

### Python

```python
import shacl2cypher

compilation = shacl2cypher.compile(["shapes/person.ttl"], dialect="neo4j", node_key="id")
compilation.write("out")                        # manifest.json and queries.cypher
print(compilation.manifest["rules"][0]["ruleId"])

with shacl2cypher.Ladybug("graph.lbug") as db:  # or shacl2cypher.Neo4j("bolt://localhost:7687", password="...")
    report = db.validate(["shapes/person.ttl"], node_key="id", timeout=30)
    print(report.render("table"))               # also "json", "junit", "sarif"
    raise SystemExit(report.exit_code("violation"))
```

- Shapes are paths or in-memory `shacl2cypher.Source(name, text)` documents. A document behaves like the file `name` in `base_dir` (default: the working directory).
- `schema` takes snapshot JSON text or a parsed snapshot, such as `db.schema()`. `db.validate(manifest=...)` runs a manifest compiled earlier.
- Errors derive from `Shacl2CypherError`: `CompileError` (with `.errors`), `ManifestError`, `DatabaseConnectionError`, `BackendUnavailableError` and `DatabaseClosedError`. Invalid option values raise `ValueError`.
- Calls release the GIL. A handle runs one call at a time on its own thread.

### TypeScript

```ts
import { compile, Ladybug, renderReport, exitCode } from 'shacl2cypher';

const compilation = await compile({ shapes: ['shapes/person.ttl'], dialect: 'neo4j', nodeKey: 'id' });
compilation.write('out');

const db = await Ladybug.open('graph.lbug');   // or await Neo4j.connect({ uri: 'bolt://localhost:7687', password: '...' })
try {
  const report = await db.validate({ shapes: ['shapes/person.ttl'], nodeKey: 'id', timeoutMs: 30_000 });
  console.log(renderReport(report, 'table'));
  process.exitCode = exitCode(report, 'violation');
} finally {
  await db.close();
}
```

- Options are camelCase objects, and unknown keys throw `TypeError`. `limit: 0` or `null` lists every violation.
- `compile`, `validate`, `schema` and `close` return Promises and never block the event loop. `compileSync` compiles on the calling thread.
- The error classes match Python's. Invalid option values throw `TypeError`, and a non-positive `timeoutMs` throws `RangeError`. Types for every option, manifest, report and snapshot ship in `index.d.ts`.

### Platforms and source builds

Wheels (one abi3 wheel for CPython 3.9+) and npm addons are published for Linux x86_64 and arm64 (glibc 2.28+) and macOS arm64, each with the Neo4j and LadybugDB backends. Other platforms can install the Python source distribution, which builds both backends and needs Rust 1.87+, cmake and a C++ compiler. A compile-only build needs neither cmake nor C++:

```sh
MATURIN_PEP517_ARGS="--no-default-features --features remote-imports" pip install --no-binary shacl2cypher shacl2cypher
```

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

- `shacl2cypher-core`: the pure compiler.
- `shacl2cypher`: the `shacl2cypher` binary.
- `shacl2cypher-runner`: database execution, schema dumps and reports.
- `s2c-testkit`: conformance fixtures and the W3C harness.

```sh
cargo test --workspace                                       # compiler, CLI, reports
cargo test --workspace --features shacl2cypher/ladybug            # + LadybugDB conformance fixtures and fuzzing
S2C_NEO4J_URI=bolt://localhost:7687 S2C_NEO4J_PASSWORD=secret \
  cargo test --workspace --features shacl2cypher/neo4j            # + Neo4j fixtures, fuzzing, W3C suite
```

- **Fixtures** live in `tests/conformance/` as a YAML graph plus a shapes file and a hand-written `expect` block. Set `S2C_FIXTURE=<substring>` to run only matching fixtures.
- **pySHACL oracle:** `pip install -r tests/oracle/requirements.txt`, then `cargo build -p s2c-testkit --bin s2c-fixture` and run `python tests/oracle/pyshacl_oracle.py`.
- **Snapshots:** accept intended Cypher changes with `INSTA_UPDATE=always cargo test -p shacl2cypher-core --test snapshots`.
- **W3C suite:** after an intended change, rerun with `S2C_W3C_BLESS=1` (and a database) and review the diff of `tests/w3c/status.yaml`.

shacl2cypher is licensed under the [MIT License](LICENSE).

Design documentation lives in [`lat.md/`](lat.md/lat.md); check it with `lat check`. The change proposal and specs are in [`openspec/`](openspec/).
