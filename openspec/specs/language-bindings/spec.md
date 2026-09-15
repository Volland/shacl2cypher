# language-bindings Specification

## Purpose

Lets Python and TypeScript programs compile SHACL shapes, validate Neo4j and LadybugDB databases, and dump schemas in-process, with the same results and JSON shapes as the `shacl2cypher` CLI.

## Requirements

### Requirement: Compile from files and in-memory sources
The Python function `shacl2cypher.compile` and the TypeScript functions `compile` (Promise) and `compileSync` SHALL accept shapes and ontologies as file paths, as in-memory documents (a name, the text, and an optional format), or as a mix of both. They SHALL return the manifest, the manifest JSON text and the Cypher text.

#### Scenario: Python compile from a file
- **WHEN** Python calls `shacl2cypher.compile(["shapes/person.ttl"], dialect="neo4j", node_key="id")`
- **THEN** the result exposes `manifest` (a dict), `manifest_json` (a str) and `cypher` (a str) containing the rules for `ex:PersonShape`

#### Scenario: TypeScript compile from an in-memory document
- **WHEN** TypeScript awaits `compile({ shapes: [{ name: "person.ttl", text }], dialect: "neo4j" })`
- **THEN** the result exposes `manifest`, `manifestJson` and `cypher`, and `manifest.inputs[0].path` is `person.ttl`

#### Scenario: No shapes
- **WHEN** compile is called with an empty shapes list
- **THEN** it fails with a compile error saying no shapes were given

### Requirement: Compile option parity
Each binding SHALL accept every `shacl2cypher compile` option, using snake_case names in Python and camelCase names in TypeScript. The options are dialect, schema, ontologies, node key, Neo4j label policy, strict, lenient, verbose, max path depth, allow remote imports, fail on schema mismatch, and base directory. Defaults SHALL match the CLI. The schema snapshot SHALL be accepted as JSON text or as an already-parsed object.

#### Scenario: Defaults match the CLI
- **WHEN** a fixture is compiled through a binding with only `dialect` set, and through `shacl2cypher compile --dialect` with no other flags from the same working directory
- **THEN** the binding's `manifest_json`/`manifestJson` equals the CLI's `manifest.json` byte for byte

#### Scenario: Schema as an object
- **WHEN** `schema` is passed as a parsed snapshot object instead of JSON text
- **THEN** compilation produces the same manifest as passing the equivalent JSON text

#### Scenario: Unknown option value
- **WHEN** `dialect` is `"postgres"` or `neo4j_labels`/`neo4jLabels` is `"all"`
- **THEN** the call fails before compiling with an error naming the option and its allowed values

### Requirement: Output parity with the CLI
The manifest JSON text and Cypher text returned by a binding SHALL be byte-identical to the `manifest.json` and `queries.cypher` the CLI writes for the same inputs, options and working directory. Manifest and report objects SHALL use the same camelCase field names as their JSON form in both languages.

#### Scenario: Cypher parity
- **WHEN** a conformance fixture is compiled through Python, through TypeScript and through the CLI
- **THEN** all three Cypher texts are identical

#### Scenario: Field names
- **WHEN** Python reads a rule from `compilation.manifest["rules"][0]`
- **THEN** the rule has the key `ruleId`, as in `manifest.json`

### Requirement: Writing compiled outputs
Each compilation result SHALL provide a method that writes `manifest.json` and `queries.cypher` into a directory, creating the directory if needed.

#### Scenario: Write to a new directory
- **WHEN** Python calls `compilation.write("out")` and `out` does not exist
- **THEN** `out/manifest.json` and `out/queries.cypher` exist with the result's manifest JSON and Cypher text

### Requirement: Typed compile errors
A failed compilation SHALL raise (Python) or reject/throw (TypeScript) a `CompileError` that carries every problem message as a list, including the `file:line` locations. It SHALL NOT write any file.

#### Scenario: Several problems
- **WHEN** a shapes document has `sh:minCount "two"` on line 4 and an unknown `s2c:` predicate on line 7
- **THEN** the error's `errors` list has both messages, containing `:4` and `:7`

#### Scenario: Error hierarchy
- **WHEN** Python code catches `shacl2cypher.Shacl2CypherError`
- **THEN** it catches `CompileError`, `ManifestError`, `DatabaseConnectionError`, `BackendUnavailableError` and `DatabaseClosedError`

### Requirement: Static diagnostics are returned, not raised
Static schema diagnostics SHALL be available on the compilation result as structured entries (code, message, source file and line). They SHALL NOT cause an error unless the fail-on-schema-mismatch option is set.

#### Scenario: Schema mismatch warning
- **WHEN** a shape references a property the schema snapshot does not declare and fail-on-schema-mismatch is off
- **THEN** compile succeeds and the result lists one diagnostic with its code and source location

### Requirement: Loading manifests
Each binding SHALL load a manifest from JSON text or from a manifest object. It SHALL reject manifests whose `schemaVersion` it does not support with a `ManifestError`.

#### Scenario: Unsupported schema version
- **WHEN** a manifest with `"schemaVersion": 99` is loaded
- **THEN** a `ManifestError` is raised naming the unsupported version

### Requirement: Database handles
Each binding SHALL open a Neo4j database by Bolt URI, user, password and optional database name, and a LadybugDB database by file path. The Neo4j connection SHALL be verified when opened. LadybugDB files SHALL be opened read-only, and a missing LadybugDB file SHALL be an error, not a new database. A handle SHALL be closable, and Python handles SHALL be context managers. Any call on a closed handle SHALL fail with `DatabaseClosedError`.

#### Scenario: Unreachable Neo4j
- **WHEN** Python calls `shacl2cypher.Neo4j("bolt://127.0.0.1:1")`
- **THEN** it raises `DatabaseConnectionError` naming the URI

#### Scenario: Missing LadybugDB file
- **WHEN** TypeScript awaits `Ladybug.open("missing.lbug")`
- **THEN** the Promise rejects with a `DatabaseConnectionError` and no file is created

#### Scenario: Use after close
- **WHEN** a handle is closed and `validate` is then called on it
- **THEN** the call fails with `DatabaseClosedError`

### Requirement: Validate through a binding
A database handle's `validate` SHALL accept either shapes (with compile options) or a manifest, and run it as `shacl2cypher validate` does. It SHALL accept the limit (default 100; `0` or no limit lists every violation), sample size (default 5) and per-query timeout options. It SHALL return a report with `conforms`, `complete`, `summary` and per-rule results using the CLI's statuses. When shapes are validated on LadybugDB without a schema, the schema SHALL be read from the database. A manifest compiled for another dialect SHALL be rejected.

#### Scenario: LadybugDB violations
- **WHEN** Python calls `db.validate(["shapes.ttl"], node_key="id")` on a LadybugDB fixture with one node missing a required name
- **THEN** the report has `conforms` false, and the rule `PersonShape.name.minCount` has status `failed`, one violation and the offending focus in its violations

#### Scenario: Dialect mismatch
- **WHEN** a manifest compiled for `neo4j` is validated on a LadybugDB handle
- **THEN** the call fails with a `ManifestError` naming both dialects

#### Scenario: Invalid timeout
- **WHEN** Python passes `timeout=0` or TypeScript passes `timeoutMs: -5`
- **THEN** the call fails before running any query with an error saying the timeout must be positive

### Requirement: Report rendering and exit codes
Each binding SHALL render a report as `table`, `json`, `junit` or `sarif` text identical to the CLI's output for that format. It SHALL compute the CLI's exit code (0, 1 or 3) for a report and a fail-on severity (`violation` default, `warning`, `info`).

#### Scenario: SARIF parity
- **WHEN** the same report is rendered with format `sarif` by a binding and written by `shacl2cypher validate --format sarif`
- **THEN** the texts are identical, apart from per-rule `durationMs` timing values

#### Scenario: Warnings only
- **WHEN** only `sh:Warning` rules failed and the exit code is computed with fail-on `violation`
- **THEN** the result is 0, and with fail-on `warning` it is 1

### Requirement: Schema dump through a binding
A database handle SHALL return its schema snapshot both as an object and as JSON text. The JSON SHALL be accepted unchanged as the `schema` compile option.

#### Scenario: Round trip
- **WHEN** TypeScript passes `await db.schema()` as `schema` to `compile` with dialect `ladybug`
- **THEN** compilation succeeds with the same manifest as passing the CLI's `schema dump` output

### Requirement: Non-blocking execution
Compilation, validation and schema dump SHALL NOT hold Python's global interpreter lock while running Rust code. In TypeScript, `compile`, `validate`, `schema`, `Neo4j.connect` and `Ladybug.open` SHALL return Promises and SHALL NOT block the Node.js event loop. Concurrent calls on one database handle SHALL be serialized and each SHALL complete.

#### Scenario: Event loop stays responsive
- **WHEN** TypeScript starts a validation whose query sleeps for 500 ms and a 10 ms timer is scheduled right after
- **THEN** the timer fires before the validation Promise settles

#### Scenario: Python threads progress
- **WHEN** one Python thread runs `validate` on a slow query while another thread increments a counter
- **THEN** the counter advances during the validation

#### Scenario: Concurrent calls on one handle
- **WHEN** two `validate` calls are started on the same handle without awaiting the first
- **THEN** both resolve with complete reports

### Requirement: Backend availability
Each binding SHALL report which database backends it was built with. Opening a backend that was not built in SHALL fail with `BackendUnavailableError` listing the available backends.

#### Scenario: Compile-only source build
- **WHEN** the Python package is built from source without backends and `shacl2cypher.Ladybug("g.lbug")` is called
- **THEN** it raises `BackendUnavailableError`, and `shacl2cypher.available_backends()` returns an empty list

### Requirement: Remote imports in bindings
With allow-remote-imports enabled, bindings SHALL fetch `http(s)` `owl:imports` using the CLI's limits (30-second timeout, 16 MB cap). Without it, remote imports SHALL fail as they do in the CLI.

#### Scenario: Oversized remote import
- **WHEN** a remote import returns more than 16 MB with allow-remote-imports enabled
- **THEN** compilation fails with an error naming the import IRI and the size limit

### Requirement: Published type information
The npm package SHALL include TypeScript declarations for every exported function, class, option object, manifest, report and schema snapshot. The Python package SHALL include `py.typed` and type annotations, with `TypedDict` definitions for manifest, report and schema snapshot data.

#### Scenario: Strict TypeScript consumer
- **WHEN** an example program using compile, validate and report rendering is type-checked with `tsc --strict --noEmit`
- **THEN** type checking passes, and accessing `report.rules[0].violationCount` is typed `number | null`

#### Scenario: Strict Python consumer
- **WHEN** the equivalent Python example is checked with `mypy --strict`
- **THEN** type checking passes
