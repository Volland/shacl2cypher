# validation-runner Specification

## Purpose

Executes compiled diagnostic queries against Neo4j, LadybugDB or FalkorDB and reports broken rules in human and CI-friendly formats.

## Requirements

### Requirement: Validate command
`shacl2cypher validate` SHALL compile shapes (or load a manifest), run every summary query, run detail queries for rules with violations, and report results.

#### Scenario: Neo4j validation
- **WHEN** `shacl2cypher validate shapes/*.ttl --connect bolt://localhost:7687` runs against a database with one violating node
- **THEN** the report lists the violated rule with its count and the offending focus

#### Scenario: LadybugDB validation
- **WHEN** `shacl2cypher validate shapes/*.ttl --ladybug ./graph.lbug` runs
- **THEN** the database is opened read-only and the same report structure is produced

#### Scenario: FalkorDB validation
- **WHEN** `shacl2cypher validate shapes/*.ttl --falkordb redis://localhost:6379 --graph social` runs against a graph with one violating node
- **THEN** the shapes are compiled for the FalkorDB dialect, queries run read-only against graph `social`, and the same report structure is produced

#### Scenario: Manifest for another dialect
- **WHEN** a manifest compiled with `--dialect neo4j` is validated against FalkorDB
- **THEN** validation exits with code 2 and an error naming both dialects

### Requirement: Report formats
The runner SHALL support `--format table|json|junit|sarif`; JUnit SHALL represent each rule as a test case and SARIF results SHALL reference the rule's shape source location.

#### Scenario: JUnit output
- **WHEN** `--format junit` is used and 3 of 10 rules fail
- **THEN** the output has 10 test cases with 3 failures

#### Scenario: SARIF location
- **WHEN** `--format sarif` is used
- **THEN** each result's location points to the `.ttl` file and line of the violated constraint

### Requirement: Severity exit codes
The runner SHALL exit non-zero when any rule at or above the `--fail-on` severity (default `violation`) has violations, and zero otherwise.

#### Scenario: Warnings only
- **WHEN** only `sh:Warning` rules have violations and `--fail-on violation`
- **THEN** the exit code is zero

#### Scenario: Fail on warning
- **WHEN** the same run uses `--fail-on warning`
- **THEN** the exit code is non-zero

### Requirement: Query timeouts
The runner SHALL apply `--timeout` per query and SHALL report timed-out rules with `status: timeout` without aborting remaining rules.

#### Scenario: Slow rule
- **WHEN** one rule exceeds the timeout
- **THEN** the report marks it `timeout` and other rules still complete

### Requirement: Timings
Reports SHALL include execution time per rule.

#### Scenario: JSON timings
- **WHEN** `--format json` is used
- **THEN** each rule result includes its duration in milliseconds

### Requirement: Optional database drivers
Database drivers SHALL be optional build features (`neo4j`, `ladybug`, `falkordb`) so a compile-only binary contains no database client code.

#### Scenario: Compile-only build
- **WHEN** the CLI is built without runner features
- **THEN** `compile` works and `validate` reports that no database backend is available

#### Scenario: Backend missing from build
- **WHEN** the CLI is built with only the `neo4j` feature and `validate --falkordb redis://localhost:6379 --graph g` runs
- **THEN** the command exits with code 2 and reports that the falkordb backend is not available, listing the available backends

### Requirement: FalkorDB read-only execution
The runner SHALL execute FalkorDB queries so that no query can modify the graph, and SHALL apply `--timeout` to each FalkorDB query.

#### Scenario: Write attempt refused
- **WHEN** the FalkorDB executor is given a query containing `CREATE`
- **THEN** the query fails and the graph is unchanged

#### Scenario: FalkorDB timeout
- **WHEN** a FalkorDB query runs longer than `--timeout`
- **THEN** the rule is reported with `status: timeout` and later rules still run

### Requirement: FalkorDB connection errors
The runner SHALL verify the FalkorDB connection and the existence of the named graph before running rules, and SHALL report failures as setup errors.

#### Scenario: Unreachable server
- **WHEN** `validate --falkordb redis://127.0.0.1:1 --graph g` runs
- **THEN** the command exits with code 2 and reports the connection failure

#### Scenario: Missing graph
- **WHEN** `validate` names a graph that does not exist on the server
- **THEN** the command exits with code 2 and reports that the graph does not exist, rather than validating an empty graph
