# shapes-loading Specification

## Purpose

Assembles a set of SHACL shape files into a single shapes graph with source locations, so constraints split across files compile as one model.

## Requirements

### Requirement: Multiple files form one shapes graph
The compiler SHALL parse all input shape files into a single union graph before interpreting shapes, so a shape defined in one file MAY be extended in another.

#### Scenario: Shape split across files
- **WHEN** `core.ttl` declares `ex:PersonShape sh:targetClass ex:Person` and `hr.ttl` adds `ex:PersonShape sh:property [ sh:path ex:salary ; sh:minCount 1 ]`
- **THEN** the manifest contains the rule `ex:PersonShape/ex:salary/sh:minCount` targeting `Person`

#### Scenario: Input order does not matter
- **WHEN** the same files are compiled in a different order
- **THEN** the manifests are byte-identical

### Requirement: Supported input formats
The compiler SHALL accept Turtle, N-Triples and TriG files; TriG named graphs SHALL be unioned into the shapes graph.

#### Scenario: Unsupported format
- **WHEN** an input file has an unrecognized extension or fails to parse
- **THEN** compilation fails with an error naming the file and the parse location

### Requirement: Blank nodes are file-scoped
Blank node labels SHALL be scoped per file so identical labels in different files denote different nodes.

#### Scenario: Same blank node label in two files
- **WHEN** `a.ttl` and `b.ttl` both use `_:p1` for different property shapes
- **THEN** two distinct property shapes are compiled

### Requirement: Local imports
The compiler SHALL follow `owl:imports` that resolve to local files relative to the importing file, detect import cycles, and SHALL NOT fetch remote imports unless `--allow-remote-imports` is given.

#### Scenario: Remote import without opt-in
- **WHEN** a shapes file imports an `https://` IRI and `--allow-remote-imports` is not set
- **THEN** compilation fails with an error naming the import

#### Scenario: Remote import with opt-in
- **WHEN** `--allow-remote-imports` is set
- **THEN** the imported document is loaded and recorded in the manifest `inputs` with its digest

#### Scenario: Import cycle
- **WHEN** `a.ttl` imports `b.ttl` and `b.ttl` imports `a.ttl`
- **THEN** each file is loaded once and compilation proceeds

### Requirement: Conflicting single-valued settings
The compiler SHALL fail when a shape receives conflicting values for a single-valued setting (`sh:severity`, `sh:deactivated`, `s2c:key`, `s2c:name`, `s2c:label`, `s2c:direction`, `s2c:datatype`, `s2c:collection`) and SHALL report every conflicting source location.

#### Scenario: Two severities
- **WHEN** `a.ttl` sets `ex:PersonShape sh:severity sh:Warning` and `b.ttl` sets `sh:severity sh:Violation`
- **THEN** compilation fails listing both file locations

### Requirement: Source locations in errors and manifest
Every compile error and every manifest rule SHALL reference the source file and line of the originating triple.

#### Scenario: Invalid constraint value
- **WHEN** `person.ttl` line 42 contains `sh:minCount "two"`
- **THEN** the error message includes `person.ttl:42`
