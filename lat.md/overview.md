# Overview

shacl2cypher compiles a set of SHACL shape files into named, read-only Cypher diagnostic queries that find and explain rule violations in a labeled property graph.

## Purpose

Application-built property graphs (not RDF stores) have no native SHACL engine. This tool lets teams write standard SHACL and validate Neo4j or LadybugDB data with generated queries.

Every generated query targets exactly one constraint and returns rows that point at the broken rule, the offending node or relationship, and the offending value. See [[output#Manifest]].

## Pipeline

Compilation is a pure, deterministic pipeline: identical shapes, schema snapshot and options always produce a byte-identical manifest.

1. **Load** all shape files into one union graph — [[architecture#Input Assembly]].
2. **Parse** into an owned shapes AST with source spans and `s2c:` annotations — [[architecture#Shapes AST]].
3. **Resolve** IRIs to labels, properties and relationship types against the schema snapshot — [[mapping#Resolution]], [[dialects#Schema Awareness]].
4. **Lower** each constraint into the dialect-neutral IR — [[semantics#Violations and Conforms]].
5. **Render** IR to Cypher per dialect — [[dialects#Dialect Backends]].
6. **Emit** the manifest and `.cypher` file — [[output#Manifest]].

## Scope

Version 1 covers SHACL Core Tier 1 and Tier 2 features on two targets, Neo4j 5 and LadybugDB. See [[semantics#Supported Features]].

Out of scope: SHACL-SPARQL (`sh:sparql`, SPARQL-based constraint components), SHACL Advanced Features (rules, functions), `sh:targetNode`, RDF-backed graphs (neosemantics), and query partitioning/batching (deferred, see [[output#Cost Classes]]).

## Target Graph Model

The validated database is a native LPG built by applications, not imported RDF. SHACL acts as a schema language whose IRIs are mapped onto LPG elements by convention and annotation.

See [[mapping]] for the full mapping rules.
