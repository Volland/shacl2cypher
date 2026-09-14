## Why

Application-built property graphs in Neo4j and LadybugDB have no SHACL engine, so teams cannot declare graph integrity rules in a standard language and check existing data against them. A compiler that turns SHACL shape files into named Cypher diagnostic queries lets them validate data where it lives and pinpoint exactly which rule is broken and where.

## What Changes

- New Rust workspace (`s2c-core`, `s2c-cli`, `s2c-runner`) producing the `shacl2cypher` binary.
- Load a set of SHACL files (Turtle/N-Triples/TriG, local `owl:imports`) into one shapes graph with an owned AST carrying source spans and `s2c:` annotations.
- Map SHACL IRIs onto an LPG by naming convention overridden by `s2c:` annotations, validated against an optional (Neo4j) or required (LadybugDB) schema snapshot; `--strict` mode.
- Compile SHACL Core Tier 1 and Tier 2 constraints into a dialect-neutral IR with null-safe `violations`/`conforms` semantics, class-hierarchy expansion, relationship focus targets, and XSD regex translation.
- Render IR to Neo4j 5 and LadybugDB Cypher; one named query per constraint with `detail` and `summary` variants.
- Emit a deterministic JSON manifest (stable rule ids, fingerprints, cost classes, static diagnostics, index recommendations, provenance) and a `.cypher` file.
- Runner: `validate` and `schema dump` against Neo4j (Bolt) and LadybugDB (embedded), with table/JSON/JUnit/SARIF reports, severity exit codes and timeouts.
- Out of scope: SHACL-SPARQL, SHACL-AF, `sh:targetNode`, RDF/n10s graphs, batched execution.

## Capabilities

### New Capabilities
- `shapes-loading`: assembling multiple SHACL files into one shapes graph and AST, imports, conflicts, source spans.
- `lpg-mapping`: resolving classes/paths to labels, tables, properties, relationships; annotation vocabulary; datatypes; value sets; IRI constants; class hierarchy; relationship targets.
- `schema-snapshot`: dialect-neutral schema snapshot format, introspection, static diagnostics and guaranteed-by-schema resolution.
- `constraint-compilation`: supported/rejected SHACL features, violations/conforms IR, nested-failure details, null safety, regex translation.
- `dialect-rendering`: Neo4j 5 and LadybugDB renderers, literal and identifier escaping.
- `diagnostic-manifest`: manifest format, rule naming, query variants, row schema, focus identity, cost classes, determinism.
- `validation-runner`: executing manifests, reports, exit codes, timeouts, schema dump.

### Modified Capabilities
<!-- none: greenfield project -->

## Impact

- New codebase; no existing code affected.
- Dependencies: `oxrdf`/`oxttl` (RDF parsing), `regex-syntax` (regex AST), `serde`/`serde_json`, `clap`, `neo4rs` (feature `neo4j`), LadybugDB Rust bindings (feature `ladybug`, availability to verify), `insta`, `testcontainers`; dev oracles rudof `shacl_validation` and pySHACL (CI only).
- Design reference: `lat.md/` (overview, mapping, semantics, output, dialects, architecture, testing).
