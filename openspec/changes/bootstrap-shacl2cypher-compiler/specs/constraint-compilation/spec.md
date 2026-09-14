## Purpose

Defines which SHACL constraints compile, how nested shapes compose, and the value and null semantics every generated query must honor.

## ADDED Requirements

### Requirement: Tier 1 constraint support
The compiler SHALL compile `sh:targetClass`, implicit class targets, `sh:targetSubjectsOf`, `sh:targetObjectsOf`, `sh:minCount`, `sh:maxCount`, `sh:datatype`, `sh:nodeKind`, `sh:class`, `sh:minInclusive`, `sh:maxInclusive`, `sh:minExclusive`, `sh:maxExclusive`, `sh:minLength`, `sh:maxLength`, `sh:pattern`, `sh:flags`, `sh:in`, `sh:hasValue`, `sh:equals`, `sh:disjoint`, `sh:lessThan`, `sh:lessThanOrEquals`, `sh:closed`, `sh:ignoredProperties`, `sh:severity`, `sh:message` and `sh:deactivated`.

#### Scenario: Minimum count violation
- **WHEN** `ex:PersonShape` requires `sh:path ex:name ; sh:minCount 1` and a `Person` node has no `name`
- **THEN** the rule's detail query returns a row for that node

#### Scenario: Deactivated shape
- **WHEN** a shape has `sh:deactivated true`
- **THEN** its rules are listed with `status: deactivated` and no queries

#### Scenario: Closed shape
- **WHEN** a shape is `sh:closed true` with properties `name` and `age` and a node has property `email`
- **THEN** a violation row is returned with `value` identifying `email`

### Requirement: Tier 2 constraint support
The compiler SHALL compile `sh:node`, `sh:not`, `sh:and`, `sh:or`, `sh:xone`, `sh:qualifiedValueShape` with `sh:qualifiedMinCount`, `sh:qualifiedMaxCount` and `sh:qualifiedValueShapesDisjoint`, and sequence, inverse, alternative, zero-or-more, one-or-more and zero-or-one paths.

#### Scenario: Disjunction
- **WHEN** a shape requires `sh:or ( [sh:path ex:email ; sh:minCount 1] [sh:path ex:phone ; sh:minCount 1] )` and a node has neither
- **THEN** a violation row is returned for that node

#### Scenario: Unbounded path limit
- **WHEN** a path uses `sh:oneOrMorePath` and `--max-path-depth 5`
- **THEN** the generated traversal is bounded to depth 5 and the rule records the bound in the manifest

### Requirement: Rejected features
The compiler SHALL reject `sh:sparql` and SPARQL-based components, `sh:targetNode`, `sh:languageIn`, `sh:uniqueLang` and recursive shape references with compile errors, or with `--lenient` SHALL record them as `status: unsupported` in the manifest.

#### Scenario: Recursive shape
- **WHEN** `ex:PersonShape` references itself through `sh:property [ sh:path ex:knows ; sh:node ex:PersonShape ]`
- **THEN** compilation fails naming the cycle

#### Scenario: Lenient mode
- **WHEN** a shape uses `sh:sparql` and `--lenient` is set
- **THEN** compilation succeeds and the rule has `status: unsupported`

### Requirement: Explained nested failures
Violation rows for `sh:node`, logical and qualified constraints SHALL include `details` listing the rule ids of inner constraints that failed for the offending value.

#### Scenario: Nested node shape failure
- **WHEN** `ex:PersonShape` has `sh:path ex:address ; sh:node ex:AddressShape` and an address lacks `zip` required by `ex:AddressShape`
- **THEN** the row's `details` contains `ex:AddressShape/ex:zip/sh:minCount`

### Requirement: Empty value sets
Value constraints SHALL be satisfied by an empty value set; only `sh:minCount`, `sh:qualifiedMinCount` and `sh:hasValue` SHALL report absence.

#### Scenario: Missing optional value
- **WHEN** `sh:path ex:age ; sh:minInclusive 0` and a node has no `age`
- **THEN** no violation is reported

### Requirement: Null-safe predicates
A value that is null inside a comparison, or whose type is incompatible with the constraint, SHALL be treated as not conforming, and generated queries SHALL never drop a violation because a predicate evaluates to null.

#### Scenario: Wrong type in range
- **WHEN** `sh:path ex:age ; sh:minInclusive 18` and a node has `age: "twenty"`
- **THEN** a violation row is returned with `value` `"twenty"`

#### Scenario: Null inside negated logic
- **WHEN** `sh:not [ sh:path ex:age ; sh:minInclusive 65 ]` and a node's `age` is a string
- **THEN** the inner shape evaluates to false (not null), so `sh:not` is satisfied and no violation is reported

### Requirement: XSD regex semantics
`sh:pattern` SHALL use XSD regex substring-match semantics with `sh:flags` on every dialect; patterns that are invalid or not expressible in the target dialect SHALL be compile errors naming the source location.

#### Scenario: Unanchored pattern
- **WHEN** `sh:pattern "\\d+"` and a value is `"abc123"`
- **THEN** the value conforms on both Neo4j and LadybugDB

#### Scenario: Case-insensitive flag
- **WHEN** `sh:pattern "^abc$" ; sh:flags "i"` and a value is `"ABC"`
- **THEN** the value conforms

#### Scenario: Inexpressible construct
- **WHEN** a pattern uses a backreference and the dialect is LadybugDB
- **THEN** compilation fails naming the pattern location
