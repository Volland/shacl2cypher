# Testing

Correctness is established by differential conformance testing against SHACL reference engines, plus snapshot, lint and fuzz tests on generated Cypher.

## Conformance Fixtures

Fixtures use a neutral YAML graph that projects to both RDF and LPG, with a hand-written expected-violations block as an independent anchor.

Each fixture lists `shapes`, `graph` (`nodes` with ids, labels, props; `edges`), and `expect` (rule id plus focus id). The RDF projection runs through rudof `shacl_validation` (in-process) and pySHACL (CI); the LPG projection runs through Neo4j (testcontainers) and LadybugDB (in-process, schema generated from the graph). All results must equal `expect`; oracle disagreements are annotated rather than failed.

Fixtures live in `tests/conformance/` and are handled by the `s2c-testkit` crate. Projection conventions:

- Each node's fixture id is stored in the `id` property (the node key); fixtures may not set `id` themselves, and closed-shape fixtures list `ex:id` in `sh:ignoredProperties`.
- RDF: nodes are `{base}node/{id}`, labels are `rdf:type {base}{label}`, list elements become separate triples and nulls are dropped.
- Edges name the RDF predicate (`pred`) and optionally the LPG relationship type (`type`, default UPPER_SNAKE_CASE of `pred`), so the projection does not reuse the compiler's mapping code.
- Relationship properties have no RDF form, so fixtures constraining them run on LPG engines only.
- LadybugDB tables are inferred from the data: one label per node and one type per property, otherwise the fixture must be restricted with `dialects`.

The pySHACL oracle (`tests/oracle/pyshacl_oracle.py`, pinned in `requirements.txt`) gets each fixture's RDF projection and expectations from the testkit's `s2c-fixture oracle-input` binary. It maps pySHACL results to `(ruleId, focus)` using the structural scheme in [[output#Rule Naming]]. Results it cannot name (blank-node node shapes, complex paths, unknown components) count as failures, never silent passes.

CI (`.github/workflows/ci.yml`) runs fmt, clippy with `-D warnings` and workspace tests on the pinned toolchain, plus separate LadybugDB-feature, Neo4j-feature and pySHACL oracle jobs.

## W3C Test Suite

The W3C `data-shapes-test-suite` Core tests, projected RDF→LPG, form a secondary suite; tests without LPG meaning are skipped with a recorded reason.

## Null and Type Edge Cases

For every constraint kind, fixtures cover null, absent, empty list, list with nulls, and wrong-type values, enforcing [[semantics#Null Safety]].

## Regex Fixtures

Dedicated fixtures cover anchoring, `sh:flags` (`i`, `s`, `m`, `x`, `q`), Unicode categories and dialect-inexpressible constructs, enforcing [[semantics#Regex Translation]].

## Null-Safety Lint

A test-only lint scans every rendered query of both dialects for comparisons directly under `NOT (…)`, enforcing [[semantics#Null Safety]].

It is aware of string literals and bracket depth. It splits `NOT` groups on top-level `AND`/`OR` and flags operands that compare without a wrapping call or explicit `IS NULL` handling.

## Renderer Smoke Runs

Renderer tests can dump every rendered query (`S2C_RENDER_DUMP`, `S2C_RENDER_DUMP_LADYBUG`) for execution against sample data on the real engines, until the runner's integration tests exist.

The Neo4j dump runs through `cypher-shell` in a container. The LadybugDB dump runs through `spikes/ladybug/src/bin/run_dump.rs`. Both engines must execute every query, and their violation counts must agree except where typed columns make a violation impossible. These runs found the LadybugDB `UNWIND list_filter(coalesce(…))` row leak described in [[dialects#Dialect Backends#LadybugDB]].

## Cypher Snapshots

`insta` snapshot tests capture generated Cypher per dialect so query-shape changes are visible in code review.

## Literal Round-Trip Fuzzing

Random strings and identifiers are rendered through [[dialects#Literals and Identifiers]], executed on both databases, and must round-trip unchanged.

## Determinism

Compiling the same inputs twice, and with input files in different order, must produce byte-identical manifests and stable rule names per [[output#Rule Naming]].
