## MODIFIED Requirements

### Requirement: Supported dialects
The compiler SHALL render queries for `--dialect neo4j` (Neo4j 5, no APOC), `--dialect ladybug` (LadybugDB) and `--dialect falkordb` (FalkorDB), and SHALL produce equivalent diagnostic results on all of them for the same data.

#### Scenario: Same fixture on both dialects
- **WHEN** a conformance fixture is loaded into Neo4j, LadybugDB and FalkorDB and validated with each dialect's manifest
- **THEN** all three return the same set of `(ruleId, focus key)` violations, except where the fixture records a known difference for an engine

#### Scenario: No APOC dependency
- **WHEN** Neo4j queries are generated
- **THEN** no query calls an `apoc.*` procedure or function

## ADDED Requirements

### Requirement: FalkorDB rule coverage
The FalkorDB dialect SHALL render every rule kind the compiler supports: node-focused rules from class, implicit class, subjects-of and objects-of targets; relationship-focused rules from `s2c:targetRelationship`; every Tier 1 constraint; and Tier 2 logical, qualified and complex-path constraints. A construct FalkorDB cannot express SHALL be a compile error naming the construct and its source location, or a rule with `status: unsupported` under `--lenient`.

#### Scenario: Node shape on FalkorDB
- **WHEN** `ex:PersonShape sh:targetClass ex:Person; sh:property [sh:path ex:name; sh:minCount 1; sh:datatype xsd:string]` is compiled with `--dialect falkordb` and validated against a graph with a `Person` node lacking `name` and another whose `name` is `42`
- **THEN** the `sh:minCount` rule reports the first node and the `sh:datatype` rule reports the second node with value `42`

#### Scenario: Relationship shape on FalkorDB
- **WHEN** a node shape with `s2c:targetRelationship "WORKS_FOR"` requires `since` to be an `xsd:date` and a `WORKS_FOR` relationship has `since: 'yesterday'`
- **THEN** the rule reports that relationship with its type, start key, end key and element id

#### Scenario: Nested shape on FalkorDB
- **WHEN** a shape uses `sh:or` over two property shapes and a node satisfies neither
- **THEN** the violation row lists both inner rule ids in `details`

#### Scenario: Inexpressible construct
- **WHEN** a shape uses a construct the FalkorDB renderer does not support
- **THEN** compilation fails with an error naming FalkorDB and the constraint's `file:line`, and with `--lenient` the rule has `status: unsupported` instead

### Requirement: Null safety on FalkorDB
FalkorDB queries SHALL treat null, absent and empty-list values as empty value sets, SHALL report wrong-type values as violations of value tests, and SHALL NOT raise runtime errors for values of an unexpected type.

#### Scenario: Wrong-type value
- **WHEN** `sh:minLength 3` applies to `name` and a node has `name: 12` while another has `name: ['ab']`
- **THEN** the query executes without error and reports both values as violations

#### Scenario: Null and absent
- **WHEN** `sh:pattern "^a"` applies to `code` and one node has no `code` while another has an empty list
- **THEN** neither node is reported
