## Purpose

Defines how SHACL classes, paths, literals and IRIs map onto labeled property graph labels, tables, properties, relationships and values.

## ADDED Requirements

### Requirement: Convention-based resolution
Without annotations or schema evidence, class local names SHALL map to labels, property-path local names SHALL map to property keys verbatim, and relationship-path local names SHALL map to UPPER_SNAKE_CASE relationship types with outgoing direction.

#### Scenario: Property path by convention
- **WHEN** a property shape has `sh:path ex:firstName ; sh:datatype xsd:string`
- **THEN** the rule checks property `firstName`

#### Scenario: Relationship path by convention
- **WHEN** a property shape has `sh:path ex:worksFor ; sh:class ex:Company`
- **THEN** the rule traverses outgoing relationships of type `WORKS_FOR` to nodes labeled `Company`

### Requirement: Relationship detection rule
A path SHALL be treated as a relationship only if its property shape declares `sh:class`, `sh:node`, or `sh:nodeKind` of `sh:IRI`, `sh:BlankNode` or `sh:BlankNodeOrIRI`, or is annotated `s2c:relationship`, or the schema snapshot contains a matching relationship type and no matching property; otherwise it SHALL be a property.

#### Scenario: Schema disambiguates
- **WHEN** a property shape has `sh:path ex:manager ; sh:minCount 1` with no `sh:class` and the schema snapshot has relationship type `MANAGER` from `Person` but no `manager` property
- **THEN** the rule counts `MANAGER` relationships

### Requirement: Annotation overrides
`s2c:label`, `s2c:property`, `s2c:relationship` and `s2c:direction` annotations on class IRIs, predicate IRIs, or shapes SHALL override convention, with shape-level annotations taking precedence over IRI-level ones.

#### Scenario: Explicit relationship type and direction
- **WHEN** a property shape has `sh:path ex:employs ; s2c:relationship "WORKS_FOR" ; s2c:direction "in"`
- **THEN** the rule traverses incoming `WORKS_FOR` relationships

### Requirement: Strict mode
With `--strict`, the compiler SHALL fail for any class or path resolved by convention alone.

#### Scenario: Unannotated path in strict mode
- **WHEN** `--strict` is set and `sh:path ex:nickname` has no annotation and no schema evidence
- **THEN** compilation fails naming the path and its source location

### Requirement: Datatype mapping
The compiler SHALL map XSD datatypes to native types per dialect as defined in the design reference, SHALL honor `s2c:datatype` overrides, and SHALL reject `rdf:langString`.

#### Scenario: Narrow integer type
- **WHEN** a property shape has `sh:datatype xsd:short` on Neo4j
- **THEN** values that are not integers or fall outside −32768..32767 are violations

#### Scenario: Language-tagged strings
- **WHEN** a shape uses `sh:datatype rdf:langString`
- **THEN** compilation fails with an unsupported-feature error

### Requirement: Value sets
Each path SHALL evaluate to a value set where a scalar is one value, a list property contributes one value per non-null element, and null, absent or empty list is the empty set; `s2c:collection "scalar"` SHALL treat a list as a single value.

#### Scenario: List counts as multiple values
- **WHEN** `sh:path ex:tags ; sh:maxCount 1` and a node has `tags: ["a", "b"]`
- **THEN** the node is reported as a violation

#### Scenario: Scalar collection override
- **WHEN** the same shape is annotated `s2c:collection "scalar"`
- **THEN** the node is not reported by the `sh:maxCount` rule

### Requirement: IRI constants
IRI constants in `sh:in` and `sh:hasValue` SHALL render as local-name strings on property paths (full IRI with `s2c:iriAsString "full"`) and SHALL match the end node's identifying key on relationship paths.

#### Scenario: Enumeration on a property
- **WHEN** `sh:path ex:status ; sh:in (ex:Active ex:Closed)`
- **THEN** values other than `"Active"` and `"Closed"` are violations

### Requirement: Class hierarchy expansion
`rdfs:subClassOf` triples from shape files and `--ontology` files SHALL expand targets and `sh:class` checks to all subclasses; on Neo4j `--neo4j-labels inherited` SHALL disable expansion; subclass cycles SHALL be compile errors.

#### Scenario: Subclass instances targeted
- **WHEN** `ex:Employee rdfs:subClassOf ex:Person` and a shape targets `ex:Person` with default options
- **THEN** nodes labeled only `Employee` are validated by that shape

#### Scenario: Subclass cycle
- **WHEN** `ex:A rdfs:subClassOf ex:B` and `ex:B rdfs:subClassOf ex:A`
- **THEN** compilation fails

### Requirement: Relationship focus targets
A node shape with `s2c:targetRelationship "TYPE"` SHALL validate each relationship of that type as a focus, SHALL allow only property-kind constraints, and SHALL reject traversal constraints with a compile error.

#### Scenario: Relationship property validated
- **WHEN** `s2c:targetRelationship "KNOWS"` with `sh:property [ sh:path ex:since ; sh:minCount 1 ]` and a `KNOWS` relationship lacks `since`
- **THEN** a violation row is produced whose focus identifies that relationship

#### Scenario: Traversal on relationship focus
- **WHEN** such a shape declares `sh:class` on a property
- **THEN** compilation fails
