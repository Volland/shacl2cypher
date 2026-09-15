//! Database runner for shacl2cypher manifests.
//!
//! Backends are optional cargo features so compile-only builds carry no
//! database client code: `neo4j` (Bolt), `ladybug` (embedded) and `falkordb`
//! (Redis protocol). The `remote-imports` feature adds an HTTP(S) fetcher for
//! `owl:imports`.

// @lat: [[architecture#Crates]]
pub mod backend;
pub mod executor;
#[cfg(feature = "falkordb")]
pub mod falkordb;
#[cfg(feature = "ladybug")]
pub mod ladybug;
#[cfg(feature = "neo4j")]
pub mod neo4j;
#[cfg(feature = "remote-imports")]
pub mod remote;
pub mod report;
pub mod session;
pub mod validate;
pub mod worker;

/// Names of the database backends compiled into this build.
pub fn available_backends() -> Vec<&'static str> {
    let mut backends = Vec::new();
    if cfg!(feature = "neo4j") {
        backends.push("neo4j");
    }
    if cfg!(feature = "ladybug") {
        backends.push("ladybug");
    }
    if cfg!(feature = "falkordb") {
        backends.push("falkordb");
    }
    backends
}
