## Purpose

Produces executable, injection-safe Cypher for each supported database dialect from the same compiled constraints.

## ADDED Requirements

### Requirement: Supported dialects
The compiler SHALL render queries for `--dialect neo4j` (Neo4j 5, no APOC) and `--dialect ladybug` (LadybugDB), and SHALL produce equivalent diagnostic results on both for the same data.

#### Scenario: Same fixture on both dialects
- **WHEN** a conformance fixture is loaded into Neo4j and LadybugDB and validated with each dialect's manifest
- **THEN** both return the same set of `(ruleId, focus key)` violations

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
