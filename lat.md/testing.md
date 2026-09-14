# Testing

Correctness is established by differential conformance testing against SHACL reference engines, plus snapshot, lint and fuzz tests on generated Cypher.

## Conformance Fixtures

Fixtures use a neutral YAML graph that projects to both RDF and LPG, with a hand-written expected-violations block as an independent anchor.

Each fixture lists `shapes`, `graph` (`nodes` with ids, labels, props; `edges`), and `expect` (rule id plus focus id). The RDF projection runs through rudof `shacl_validation` (in-process) and pySHACL (CI); the LPG projection runs through Neo4j (a CI service container) and LadybugDB (in-process, schema generated from the graph). All results must equal `expect`; oracle disagreements are annotated rather than failed.

Fixtures live in `tests/conformance/` and are handled by the `s2c-testkit` crate.

- An `expect` entry may list `details`: the inner rule ids LPG engines must report for that violation. Relationship focuses are written `from->to`.
- `known_differences` records a violation one engine (`pyshacl`, `neo4j`, `ladybug`) reports differently from `expect`, with the reason; only that engine ignores it.
- `dialects` restricts the LPG engines a fixture loads into, for data one engine cannot store, such as nulls inside lists on Neo4j or mixed column types on LadybugDB. pySHACL always runs.
- `S2C_FIXTURE=<substring>` limits the database integration tests to matching fixture files.

Projection conventions:

- Each node's fixture id is stored in the `id` property (the node key); fixtures may not set `id` themselves, and closed shapes always allow the node key.
- RDF: nodes are `{base}node/{id}`, labels are `rdf:type {base}{label}`, list elements become separate triples and nulls are dropped.
- Edges name the RDF predicate (`pred`) and optionally the LPG relationship type (`type`, default UPPER_SNAKE_CASE of `pred`), so the projection does not reuse the compiler's mapping code.
- Relationship properties have no RDF form, so fixtures constraining them run on LPG engines only.
- LadybugDB tables are inferred from the data: one label per node and one type per property, otherwise the fixture must be restricted with `dialects`.

The pySHACL oracle (`tests/oracle/pyshacl_oracle.py`, pinned in `requirements.txt`) gets each fixture's RDF projection and expectations from the testkit's `s2c-fixture oracle-input` binary. It maps pySHACL results to `(ruleId, focus)` using the structural scheme in [[output#Rule Naming]]. Results it cannot name (blank-node node shapes, complex paths, unknown components) count as failures, never silent passes.

CI (`.github/workflows/ci.yml`) runs fmt, clippy with `-D warnings` and workspace tests on the pinned toolchain, plus separate LadybugDB-feature, Neo4j-feature (with a Neo4j 5 service container) and pySHACL oracle jobs.

## W3C Test Suite

The W3C `data-shapes-test-suite` Core tests, projected RDF→LPG, form a secondary suite; tests without LPG meaning are skipped with a recorded reason.

`tests/w3c/core` vendors the suite (source commit and license in `tests/w3c/README.md`). `s2c-testkit`'s `w3c` module loads each test:

- It strips the manifest and reads the expected top-level `(focus, component)` results.
- It unions sibling `-shapes.ttl`/`-data.ttl` files and leaves companion files out of the suite.
- It projects the graph to Neo4j: `rdf:type` becomes labels, literals become properties (ill-formed typed literals stay strings), IRI objects become relationships, and each node's IRI local name is its `id`.

`crates/s2c-runner/tests/w3c.rs` loads every runnable test into Neo4j, dumps the schema, compiles the shapes as N-Triples, validates, and compares `(focus, constraint)` sets. Every outcome is recorded in `tests/w3c/status.yaml` and checked by the test:

- `pass`, or `fail: missing [...]; unexpected [...]` for a known divergence.
- `skip: <reason>` for static reasons (`sh:targetNode`, SPARQL, language tags, literal or blank focus nodes), or for a projection, compile or runtime stage that cannot run the test.

Without `S2C_NEO4J_URI` only static skips are checked. After an intended change, rerun with `S2C_W3C_BLESS=1` and review the diff of `status.yaml`.

Current status of the 98 Core tests:

- 27 pass.
- 46 are skipped for `sh:targetNode`, 19 for literal focus nodes and 2 for language tags.
- 2 are skipped because one predicate has both literal and IRI objects, which has no LPG form.
- 1 is a compile skip: qualified counts without `sh:qualifiedValueShape` are rejected rather than ignored.
- 1 fails: `property/property-001` needs property shapes nested inside property shapes, which the compiler does not support yet.

## Runner Integration Tests

Every conformance fixture is loaded into a real database, validated end to end through the runner, and its violations compared with `expect`.

- LadybugDB (`--features ladybug`): each fixture gets a new database file. The schema is dumped from it, round-tripped through JSON and used to compile. Tests also check that writes are refused and slow queries time out.
- Neo4j (`--features neo4j`): runs only when `S2C_NEO4J_URI` is set. The database is wiped per fixture; tests also check that writes roll back and timeouts recover.

## Null and Type Edge Cases

For every constraint kind, fixtures cover null, absent, empty list, list with nulls, and wrong-type values, enforcing [[semantics#Null Safety]].

## Regex Fixtures

Dedicated fixtures cover anchoring, `sh:flags` (`i`, `s`, `m`, `x`, `q`), Unicode categories and dialect-inexpressible constructs, enforcing [[semantics#Regex Translation]].

## Null-Safety Lint

A test-only lint scans every rendered query of both dialects for comparisons directly under `NOT (…)`, enforcing [[semantics#Null Safety]].

It is aware of string literals and bracket depth. It splits `NOT` groups on top-level `AND`/`OR` and flags operands that compare without a wrapping call or explicit `IS NULL` handling.

## Renderer Smoke Runs

Renderer tests can dump every rendered query (`S2C_RENDER_DUMP`, `S2C_RENDER_DUMP_LADYBUG`) for execution against sample data on the real engines, for queries no fixture covers yet.

The Neo4j dump runs through `cypher-shell` in a container. The LadybugDB dump runs through `spikes/ladybug/src/bin/run_dump.rs`. Both engines must execute every query, and their violation counts must agree except where typed columns make a violation impossible. These runs found the LadybugDB `UNWIND list_filter(coalesce(…))` row leak described in [[dialects#Dialect Backends#LadybugDB]].

## Cypher Snapshots

`insta` snapshot tests capture generated Cypher per dialect so query-shape changes are visible in code review.

`crates/s2c-core/tests/snapshots.rs` compiles `tests/data/snapshot.ttl` for Neo4j and, with `snapshot-schema.json`, for LadybugDB. Accept intended changes with `INSTA_UPDATE=always cargo test -p shacl2cypher-core --test snapshots`.

## Literal Round-Trip Fuzzing

Random strings and identifiers are rendered through [[dialects#Literals and Identifiers]], executed on both databases, and must round-trip unchanged.

The seeded generator mixes quotes, backslashes, backticks, control characters, surrogate-pair emoji and `\uXXXX` text. Strings, identifiers, 64-bit integer edges, finite doubles, booleans and dates are each returned by `RETURN <literal>` and compared with the input.

The test found two real defects. Neo4j decodes `\uXXXX` inside backticked identifiers, and neo4rs 0.8 decoded −16…−1 as 240…255.

## Determinism

Compiling the same inputs twice, and with input files in different order, must produce byte-identical manifests and stable rule names per [[output#Rule Naming]].

`crates/s2c-core/tests/determinism.rs` also reorders `sh:property` values and shuffles every triple of the shapes graph. Rule names, ids, fingerprints and queries must not change; only source lines may.

The shuffle exposed ordering by source position and blank-node label, now replaced by content ordering ([[architecture#Shapes AST]]).
