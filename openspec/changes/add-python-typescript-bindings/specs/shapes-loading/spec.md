## ADDED Requirements

### Requirement: In-memory shape documents
The compiler SHALL accept shapes and ontologies as named in-memory documents (a name, the text, and an optional format of Turtle, N-Triples or TriG) alongside files. A document SHALL behave exactly like a file of the same name placed in the base directory. That covers:
- its base IRI and the resolution of relative `owl:imports`
- per-document blank-node scoping
- format detection from the name's extension when no format is given
- ordering, which is independent of argument order
- the `inputs` entry with the SHA-256 of its UTF-8 bytes
- `name:line` source locations

Two documents with the same name, or a document whose name resolves to a path also given as a file, SHALL be a compile error.

#### Scenario: Same result as files
- **WHEN** `person.ttl` and `hr.ttl` are compiled as in-memory documents with base directory `shapes/`, and those files with the same contents are compiled from `shapes/`
- **THEN** the manifests are byte-identical

#### Scenario: Location in errors
- **WHEN** document `person.ttl` has `sh:minCount "two"` on line 42
- **THEN** the error message includes `person.ttl:42`

#### Scenario: Relative import from a document
- **WHEN** a document named `main.ttl` with base directory `shapes/` imports `<common.ttl>`
- **THEN** the file `shapes/common.ttl` is loaded and listed in the manifest `inputs`

#### Scenario: Duplicate names
- **WHEN** two documents are both named `person.ttl`
- **THEN** compilation fails with an error naming the duplicate document

#### Scenario: Unknown format
- **WHEN** a document is named `shapes.json` and no format is given
- **THEN** compilation fails with an error naming the document and the supported formats
