# diagnostic-manifest Specification

## Purpose

Defines the compiler's output contract: a deterministic manifest of stably named diagnostic queries whose rows point to broken rules and offending data.

## Requirements

### Requirement: Manifest and cypher outputs
`shacl2cypher compile` SHALL write a JSON manifest and a `.cypher` file in which each query is preceded by a `// name: <name>` header.

#### Scenario: Compile outputs
- **WHEN** `shacl2cypher compile shapes/*.ttl --dialect neo4j -o out/` succeeds
- **THEN** `out/` contains `manifest.json` and `queries.cypher` with one header per query

### Requirement: Manifest provenance
The manifest SHALL record `schemaVersion`, `compilerVersion`, `dialect`, compile options, each input file with its SHA-256 digest, and the schema snapshot digest.

#### Scenario: Input changed
- **WHEN** one shape file changes and compilation is rerun
- **THEN** that file's digest in `inputs` changes

### Requirement: One query per constraint
The compiler SHALL emit one rule per constraint component instance, each with `detail` and `summary` query variants.

#### Scenario: Two constraints on one path
- **WHEN** a property shape declares `sh:minCount 1` and `sh:datatype xsd:string`
- **THEN** the manifest contains two rules, each with its own detail and summary query

### Requirement: Detail row schema
Detail queries SHALL return one row per violation with columns `ruleId`, `shape`, `path`, `constraint`, `severity`, `focus`, `value`, `message`, `details`, limited by `$limit`.

#### Scenario: Limit applied
- **WHEN** a rule has 100 violations and `$limit` is 10
- **THEN** exactly 10 rows are returned

### Requirement: Summary row schema
Summary queries SHALL return exactly one row with `ruleId`, `severity`, `violationCount` and `sample` of up to `$sampleSize` focus objects, including when there are zero violations.

#### Scenario: Passing rule
- **WHEN** a rule has no violations
- **THEN** the summary query returns one row with `violationCount` 0 and an empty `sample`

### Requirement: Focus identity
Node focus objects SHALL contain `label`, `key`, `keyValue` and `elementId`, where `key` is the shape's `s2c:key`, else `--node-key`, else null; relationship focus objects SHALL contain `type`, `startKey`, `endKey` and `elementId`; `--verbose` SHALL add all properties.

#### Scenario: Per-shape key
- **WHEN** `ex:PersonShape s2c:key "email"` and `--node-key id` is set
- **THEN** focus objects for that shape's rules use `key` `"email"`

#### Scenario: No key configured
- **WHEN** no `s2c:key` or `--node-key` applies
- **THEN** `key` and `keyValue` are null and `elementId` is populated

### Requirement: Stable rule identifiers
Rule ids SHALL be derived from node shape, path and component (with nested segments for logical branches), SHALL NOT depend on triple order or blank node labels, SHALL receive a canonical-content hash suffix only on collision, and SHALL be overridable with `s2c:name`; duplicate names SHALL be compile errors.

#### Scenario: Reordered properties
- **WHEN** two `sh:property` blocks swap order in the source file
- **THEN** all rule ids and names are unchanged

#### Scenario: Colliding structural ids
- **WHEN** two qualified value shapes share path `ex:address`
- **THEN** their rule ids differ by a hash suffix that is stable across recompilation

#### Scenario: Duplicate explicit names
- **WHEN** two shapes declare `s2c:name "person-check"`
- **THEN** compilation fails listing both locations

### Requirement: Rule fingerprint
Each rule SHALL carry a `fingerprint` that changes when its constraint, target or dialect changes, even if its id does not.

#### Scenario: Bound edited
- **WHEN** `sh:minCount 1` changes to `sh:minCount 2`
- **THEN** the rule id is unchanged and the fingerprint changes

### Requirement: Cost classes and index recommendations
Each rule SHALL record a `costClass` of `scan`, `scan+expand`, `quadratic` or `unbounded-path`, and the manifest SHALL list recommended indexes without creating them.

#### Scenario: Disjoint across multi-valued paths
- **WHEN** a shape uses `sh:disjoint` between two relationship paths
- **THEN** the rule's `costClass` is `quadratic`

### Requirement: Deterministic output
Compiling identical inputs, options and snapshot SHALL produce byte-identical outputs.

#### Scenario: Repeated compile
- **WHEN** compilation runs twice with the same inputs
- **THEN** both manifests and cypher files are byte-identical
