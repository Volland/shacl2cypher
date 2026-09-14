# validation-runner Specification

## Purpose

Executes compiled diagnostic queries against Neo4j or LadybugDB and reports broken rules in human and CI-friendly formats.

## Requirements

### Requirement: Validate command
`shacl2cypher validate` SHALL compile shapes (or load a manifest), run every summary query, run detail queries for rules with violations, and report results.

#### Scenario: Neo4j validation
- **WHEN** `shacl2cypher validate shapes/*.ttl --connect bolt://localhost:7687` runs against a database with one violating node
- **THEN** the report lists the violated rule with its count and the offending focus

#### Scenario: LadybugDB validation
- **WHEN** `shacl2cypher validate shapes/*.ttl --ladybug ./graph.lbug` runs
- **THEN** the database is opened read-only and the same report structure is produced

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
Database drivers SHALL be optional build features so a compile-only binary contains no database client code.

#### Scenario: Compile-only build
- **WHEN** the CLI is built without runner features
- **THEN** `compile` works and `validate` reports that no database backend is available
