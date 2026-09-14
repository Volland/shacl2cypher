# Testing

Correctness is established by differential conformance testing against SHACL reference engines, plus snapshot, lint and fuzz tests on generated Cypher.

## Conformance Fixtures

Fixtures use a neutral YAML graph that projects to both RDF and LPG, with a hand-written expected-violations block as an independent anchor.

Each fixture lists `shapes`, `graph` (`nodes` with ids, labels, props; `edges`), and `expect` (rule id plus focus id). The RDF projection runs through rudof `shacl_validation` (in-process) and pySHACL (CI); the LPG projection runs through Neo4j (testcontainers) and LadybugDB (in-process, schema generated from the graph). All results must equal `expect`; oracle disagreements are annotated rather than failed.

## W3C Test Suite

The W3C `data-shapes-test-suite` Core tests, projected RDF→LPG, form a secondary suite; tests without LPG meaning are skipped with a recorded reason.

## Null and Type Edge Cases

For every constraint kind, fixtures cover null, absent, empty list, list with nulls, and wrong-type values, enforcing [[semantics#Null Safety]].

## Regex Fixtures

Dedicated fixtures cover anchoring, `sh:flags` (`i`, `s`, `m`, `x`, `q`), Unicode categories and dialect-inexpressible constructs, enforcing [[semantics#Regex Translation]].

## Cypher Snapshots

`insta` snapshot tests capture generated Cypher per dialect so query-shape changes are visible in code review.

## Literal Round-Trip Fuzzing

Random strings and identifiers are rendered through [[dialects#Literals and Identifiers]], executed on both databases, and must round-trip unchanged.

## Determinism

Compiling the same inputs twice, and with input files in different order, must produce byte-identical manifests and stable rule names per [[output#Rule Naming]].
