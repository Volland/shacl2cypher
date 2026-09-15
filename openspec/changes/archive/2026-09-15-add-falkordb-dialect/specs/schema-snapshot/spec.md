## MODIFIED Requirements

### Requirement: Snapshot requirement per dialect
The snapshot SHALL be required for the LadybugDB dialect and optional for the Neo4j and FalkorDB dialects.

#### Scenario: LadybugDB without snapshot
- **WHEN** compiling with `--dialect ladybug` and no `--schema`
- **THEN** compilation fails explaining that a schema snapshot is required

#### Scenario: Neo4j without snapshot
- **WHEN** compiling with `--dialect neo4j` and no `--schema`
- **THEN** compilation succeeds using convention and annotations only

#### Scenario: FalkorDB without snapshot
- **WHEN** compiling with `--dialect falkordb` and no `--schema`
- **THEN** compilation succeeds using convention and annotations only

### Requirement: Schema introspection
The runner SHALL produce a snapshot from a live Neo4j database, from a LadybugDB database and from a FalkorDB graph via `shacl2cypher schema dump`.

#### Scenario: Dump and reuse
- **WHEN** a user runs `schema dump` and passes the output to `compile --schema`
- **THEN** compilation accepts it without modification

#### Scenario: FalkorDB dump
- **WHEN** `schema dump` runs against a FalkorDB graph with `Person` nodes whose `age` values are all integers and `WORKS_FOR` relationships from `Person` to `Company`
- **THEN** the snapshot lists `Person` with an integer `age` property and `WORKS_FOR` with a `Person` to `Company` endpoint, using neutral type names
