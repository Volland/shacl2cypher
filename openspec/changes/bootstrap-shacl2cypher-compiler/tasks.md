## 1. Spikes (de-risk dialects)

- [ ] 1.1 Verify LadybugDB Rust bindings: open DB in-process, create node/rel tables, run a query
- [ ] 1.2 Verify LadybugDB support for correlated `COUNT {}`/`EXISTS {}` subqueries and list comprehensions inside `WHERE`; document fallback shape if unsupported
- [ ] 1.3 Verify LadybugDB schema introspection calls and regex functions (`regexp_matches`, flags)
- [ ] 1.4 Verify Neo4j 5 `IS ::` behavior for scalars, lists and temporal types against the datatype table
- [ ] 1.5 Record spike findings in `lat.md/dialects.md` and adjust design if needed

## 2. Workspace and scaffolding

- [ ] 2.1 Create cargo workspace with `s2c-core`, `s2c-cli`, `s2c-runner` (features `neo4j`, `ladybug`)
- [ ] 2.2 Set up CI: fmt, clippy, tests, testcontainers Neo4j job, pySHACL oracle job
- [ ] 2.3 Add `insta` and the conformance fixture harness skeleton (YAML loader, RDF and LPG projections, `expect` comparison)

## 3. Shapes loading and AST

- [ ] 3.1 Load Turtle/N-Triples/TriG into a union graph with per-file blank-node scoping and source spans
- [ ] 3.2 Resolve local `owl:imports` with cycle handling; gate remote imports behind `--allow-remote-imports`
- [ ] 3.3 Build shapes AST: node/property shapes, targets, implicit class targets, RDF lists, path structures, defaults
- [ ] 3.4 Parse `s2c:` annotations into the AST
- [ ] 3.5 Detect conflicting single-valued settings and report all source spans
- [ ] 3.6 Detect recursive shape references

## 4. Mapping and schema snapshot

- [ ] 4.1 Define schema snapshot JSON format with serde and validation
- [ ] 4.2 Implement resolution: annotations > schema evidence > convention; relationship detection rule; `--strict`
- [ ] 4.3 Implement class hierarchy expansion from shapes and `--ontology`, subclass cycle detection, `--neo4j-labels`
- [ ] 4.4 Implement datatype mapping table and `s2c:datatype` overrides
- [ ] 4.5 Implement static diagnostics (`SchemaMismatch`) and `guaranteed-by-schema` resolution; `--fail-on-schema-mismatch`

## 5. IR and constraint lowering

- [ ] 5.1 Define IR: targets (label sets, subjects-of, objects-of, relationship), value sets, value/set/logical predicates
- [ ] 5.2 Implement null-safe predicate constructor and value-set normalization
- [ ] 5.3 Lower Tier 1 cardinality, value type, range, string, `sh:in`/`sh:hasValue` constraints
- [ ] 5.4 Lower property pair constraints and `sh:closed`
- [ ] 5.5 Implement XSD regex parsing and flag handling into regex AST
- [ ] 5.6 Implement `conforms` lowering and `sh:node`, `sh:and`, `sh:or`, `sh:not`, `sh:xone` with `details`
- [ ] 5.7 Lower `sh:qualifiedValueShape` constraints including disjointness
- [ ] 5.8 Lower complex paths (sequence, inverse, alternative, `*`/`+`/`?`) with `--max-path-depth`
- [ ] 5.9 Implement relationship focus targets and reject traversal constraints on them
- [ ] 5.10 Implement rejected-feature errors and `--lenient` unsupported entries

## 6. Dialect renderers

- [ ] 6.1 Implement typed literal renderer and identifier escaping per dialect
- [ ] 6.2 Implement Neo4j renderer for all IR constructs, detail and summary variants, message placeholders
- [ ] 6.3 Implement LadybugDB renderer, including fallback for nested subqueries per spike results
- [ ] 6.4 Implement per-dialect regex rendering and inexpressible-construct errors
- [ ] 6.5 Add lint test forbidding bare comparisons under `NOT` and write clauses in any rendered query

## 7. Manifest and CLI compile

- [ ] 7.1 Implement structural rule ids, collision hashing, `s2c:name` overrides, duplicate detection
- [ ] 7.2 Implement canonical constraint form and fingerprints
- [ ] 7.3 Implement cost classification and recommended indexes
- [ ] 7.4 Emit manifest (provenance, rules, statuses, static diagnostics) and `.cypher` file deterministically
- [ ] 7.5 Implement `shacl2cypher compile` CLI with all options (`--dialect`, `--schema`, `--ontology`, `--node-key`, `--strict`, `--lenient`, `--verbose`, `-o`)

## 8. Runner

- [ ] 8.1 Implement Neo4j executor (Bolt) with timeouts and read-only sessions
- [ ] 8.2 Implement LadybugDB executor (embedded, read-only open)
- [ ] 8.3 Implement `schema dump` for both backends
- [ ] 8.4 Implement `validate` flow: summaries, drill-down details, timings, `--fail-on` exit codes
- [ ] 8.5 Implement table, JSON, JUnit and SARIF reporters

## 9. Conformance and quality

- [ ] 9.1 Write fixtures for every Tier 1 constraint including null/absent/empty-list/wrong-type cases
- [ ] 9.2 Write fixtures for Tier 2 logical, qualified and path constraints including `details`
- [ ] 9.3 Write regex fixtures (anchoring, flags, Unicode classes, inexpressible constructs)
- [ ] 9.4 Integrate filtered W3C SHACL Core test suite with skip reasons
- [ ] 9.5 Add literal/identifier round-trip fuzz tests on both databases
- [ ] 9.6 Add determinism tests (repeat compile, shuffled input order, reordered properties)
- [ ] 9.7 Add `insta` snapshots of generated Cypher per dialect

## 10. Documentation

- [ ] 10.1 Add `lat.md/` test spec sections with `require-code-mention` and `@lat:` refs in tests
- [ ] 10.2 Link core code to `lat.md/` sections with `@lat:` comments; keep `lat check` green
- [ ] 10.3 Write README with `s2c:` vocabulary reference and compile/validate examples
