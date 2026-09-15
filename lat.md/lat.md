This directory defines the high-level concepts, business logic, and architecture of this project using markdown. It is managed by [lat.md](https://www.npmjs.com/package/lat.md) — a tool that anchors source code to these definitions. Install the `lat` command with `npm i -g lat.md` and run `lat --help`.

- [[overview]] — purpose, compilation pipeline, scope and target graph model
- [[mapping]] — SHACL IRIs to LPG labels, properties, relationships; `s2c:` annotations; datatypes; value sets; hierarchy
- [[semantics]] — supported SHACL features, violations/conforms composition, null safety, regex translation
- [[output]] — manifest, named query variants, row schema, focus identity, rule naming, cost classes
- [[dialects]] — IR, Neo4j and LadybugDB backends, schema snapshot, literal and identifier escaping
- [[architecture]] — Rust crates, input assembly, shapes AST, runner and reports
- [[bindings]] — Python and Node.js packages, database worker threads, data model, errors, packaging and release
- [[testing]] — differential conformance fixtures, W3C suite, snapshots, fuzzing, determinism
- [[tests]] — test specifications, each referenced by the test that covers it
