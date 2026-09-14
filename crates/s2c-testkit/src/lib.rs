//! Test-only toolkit for conformance fixtures.
//!
//! A fixture is a neutral YAML graph plus a SHACL shapes file and a hand-written
//! `expect` block. The graph projects to RDF (for SHACL reference engines) and to
//! LPG load scripts (for Neo4j and LadybugDB); every engine's violations are then
//! compared against `expect`.

// @lat: [[testing#Conformance Fixtures]]
pub mod compare;
pub mod fixture;
pub mod lpg;
pub mod rdf;
pub mod w3c;

/// Directory holding the repository's conformance fixtures.
pub fn fixtures_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/conformance")
}
