## Purpose

Makes compilation aware of the target database schema so structural mismatches are diagnosed statically and schema-guaranteed constraints are not re-checked.

## ADDED Requirements

### Requirement: Dialect-neutral snapshot format
The compiler SHALL accept a JSON schema snapshot listing node types with typed properties and relationship types with FROM/TO node types and typed properties, independent of the dialect it came from.

#### Scenario: Invalid snapshot
- **WHEN** the snapshot file is malformed or references an unknown property type
- **THEN** compilation fails with an error identifying the problem

### Requirement: Snapshot requirement per dialect
The snapshot SHALL be required for the LadybugDB dialect and optional for the Neo4j dialect.

#### Scenario: LadybugDB without snapshot
- **WHEN** compiling with `--dialect ladybug` and no `--schema`
- **THEN** compilation fails explaining that a schema snapshot is required

#### Scenario: Neo4j without snapshot
- **WHEN** compiling with `--dialect neo4j` and no `--schema`
- **THEN** compilation succeeds using convention and annotations only

### Requirement: Static schema mismatch diagnostics
When a snapshot is present, a target type, property or relationship type referenced by a shape but absent from the snapshot SHALL produce a static diagnostic in the manifest instead of a query; `--fail-on-schema-mismatch` SHALL turn static diagnostics into compile errors.

#### Scenario: Missing column
- **WHEN** a shape constrains `ex:nickname` on `Person` and the snapshot's `Person` has no `nickname` property
- **THEN** the manifest contains a `s2c:SchemaMismatch` static diagnostic with the source location and no query for that rule

#### Scenario: Fail on mismatch
- **WHEN** the same input is compiled with `--fail-on-schema-mismatch`
- **THEN** compilation exits non-zero

### Requirement: Schema-guaranteed constraints
Constraints provably satisfied by the declared schema (a matching declared column type for `sh:datatype`, rel table endpoints covering `sh:class`) SHALL be recorded with `status: guaranteed-by-schema` and no query; contradicted constraints SHALL produce static diagnostics.

#### Scenario: Declared column type matches
- **WHEN** on LadybugDB `sh:datatype xsd:date` targets a `DATE` column
- **THEN** the rule has `status: guaranteed-by-schema`

#### Scenario: Declared column type contradicts
- **WHEN** `sh:datatype xsd:integer` targets a `STRING` column
- **THEN** a static diagnostic is emitted for the rule

### Requirement: Schema introspection
The runner SHALL produce a snapshot from a live Neo4j database and from a LadybugDB database via `shacl2cypher schema dump`.

#### Scenario: Dump and reuse
- **WHEN** a user runs `schema dump` and passes the output to `compile --schema`
- **THEN** compilation accepts it without modification
