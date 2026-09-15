# dialect-rendering Specification

## Purpose

Produces executable, injection-safe Cypher for each supported database dialect from the same compiled constraints.

## Requirements

### Requirement: Supported dialects
The compiler SHALL render queries for `--dialect neo4j` (Neo4j 5, no APOC), `--dialect ladybug` (LadybugDB) and `--dialect falkordb` (FalkorDB), and SHALL produce equivalent diagnostic results on all of them for the same data.

#### Scenario: Same fixture on both dialects
- **WHEN** a conformance fixture is loaded into Neo4j, LadybugDB and FalkorDB and validated with each dialect's manifest
- **THEN** all three return the same set of `(ruleId, focus key)` violations, except where the fixture records a known difference for an engine

#### Scenario: No APOC dependency
- **WHEN** Neo4j queries are generated
- **THEN** no query calls an `apoc.*` procedure or function

### Requirement: Read-only queries
Generated queries SHALL NOT create, update or delete data, indexes or constraints.

#### Scenario: Write clause check
- **WHEN** any manifest is generated
- **THEN** no query contains `CREATE`, `MERGE`, `SET`, `DELETE`, `REMOVE` or index/constraint DDL

### Requirement: Literal escaping
All constants taken from shape files SHALL be rendered as correctly escaped literals such that the value observed by the database equals the value in the shape; non-finite numbers SHALL be compile errors.

#### Scenario: Quote in hasValue
- **WHEN** `sh:hasValue "O'Brien\\"; MATCH (n) DETACH DELETE n //"`
- **THEN** the query compares against exactly that string and executes no additional clauses

### Requirement: Identifier escaping
All labels, property keys and relationship types SHALL be rendered as escaped identifiers; names the target dialect cannot represent SHALL be compile errors.

#### Scenario: Backtick in label
- **WHEN** `s2c:label "Weird`Label"`
- **THEN** the Neo4j query references the label with the backtick escaped and matches nodes with that exact label

### Requirement: Runtime parameters
Queries SHALL use runtime parameters only for execution controls `$limit` and `$sampleSize`; shape constants SHALL NOT be parameters.

#### Scenario: Unlimited detail
- **WHEN** a detail query is executed with `$limit` null
- **THEN** all violations are returned

### Requirement: Message placeholders
`sh:message` values SHALL appear in rows with `{$this}` and `{?value}` placeholders substituted by the actual focus key and value at query time; without `sh:message` a generated message SHALL describe the constraint.

#### Scenario: Custom message
- **WHEN** `sh:message "Person {$this} has invalid age {?value}"` and focus key `p7` has `age: -3`
- **THEN** the row's `message` is `"Person p7 has invalid age -3"`

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
