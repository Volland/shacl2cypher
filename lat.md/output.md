# Output

The compiler's output contract: a JSON manifest of named queries, their stable names, the diagnostic row schema, and metadata for runners.

## Manifest

The JSON manifest is the primary artifact; the `.cypher` file is generated from it for humans and copy-paste use.

Top-level fields: `schemaVersion`, `compilerVersion`, `dialect`, `inputs` (`[{path, sha256}]`), `schemaSnapshotHash`, `options`, `rules`, `staticDiagnostics`, `recommendedIndexes`.

Each rule entry: `name`, `ruleId`, `fingerprint`, `shape`, `path`, `constraint`, `severity`, `status` (`compiled`, `guaranteed-by-schema`, `unsupported`), `costClass`, `source` (file and span), and `queries` with `detail` and `summary` variants.

Queries only read. The `.cypher` file prefixes each query with a `// name: <name>` header.

## Query Granularity

There is one named query per constraint component instance, never one per shape or one per shapes graph.

Each query can be run, timed, and baselined independently. Runners may `UNION ALL` queries since every variant returns the same columns.

## Query Variants

Each rule compiles into a detail query and a summary query that share the same violation pattern.

### Detail

Returns one row per violation, capped by the runtime parameter `$limit` (null means unlimited).

Row schema: `ruleId`, `shape`, `path`, `constraint`, `severity`, `focus` ([[output#Focus Identity]]), `value` (offending value or node, may be null), `message`, `details` (nested failures, [[semantics#Violations and Conforms#Explained Nested Failures]]).

### Summary

Returns exactly one row per rule, even with zero violations, so a passing rule is distinguishable from a rule that never ran.

Row schema: `ruleId`, `severity`, `violationCount`, `sample` (up to `$sampleSize` focus objects).

## Focus Identity

Rows identify the focus node by per-shape key, falling back to a global key, always accompanied by the database element id.

Node focus: `{label, key, keyValue, elementId}` where `key` is the shape's `s2c:key`, else `--node-key`, else null. Relationship focus: `{type, startKey, endKey, elementId}`. `properties(n)` is included only with `--verbose`.

## Rule Naming

Rule ids are structural and deterministic so names survive reordering and unrelated edits.

- `ruleId` = `{nodeShape}/{path}/{component}`, nested segments for logical branches, e.g. `ex:PersonShape/ex:address/sh:or[0]/ex:street/sh:minCount`.
- On collision, a short hash of the constraint's canonical form is appended: `…/sh:qualifiedMinCount~a3f9`.
- `s2c:name` on any shape overrides the generated name.
- `name` is the compacted, sanitized form (`PersonShape.name.minCount`); duplicate names after overrides are compile errors.
- `fingerprint` hashes canonical constraint, target and dialect, so CI detects a rule that changed under the same name.

## Static Diagnostics

Problems detectable at compile time are reported in the manifest instead of generating queries that would fail or be meaningless.

Examples: a target table or property column missing from the schema snapshot (`s2c:SchemaMismatch`), a datatype constraint contradicting a declared column type. `--fail-on-schema-mismatch` turns them into compile errors for CI.

## Cost Classes

Each rule records a static cost class so runners can order, warn about, or skip expensive checks.

Classes: `scan`, `scan+expand`, `quadratic` (e.g. `sh:disjoint` across multi-valued paths), `unbounded-path`. Variable-length paths are bounded by `--max-path-depth`; reaching the bound is reported, not silently truncated. `recommendedIndexes` lists indexes the compiler suggests but never creates. Partitioned execution (`$skip`/`$batch`) is deferred.
