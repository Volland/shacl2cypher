//! Database runner for shacl2cypher manifests.
//!
//! Backends are optional cargo features so compile-only builds carry no
//! database client code: `neo4j` (Bolt) and `ladybug` (embedded).

/// Names of the database backends compiled into this build.
pub fn available_backends() -> Vec<&'static str> {
    let mut backends = Vec::new();
    if cfg!(feature = "neo4j") {
        backends.push("neo4j");
    }
    if cfg!(feature = "ladybug") {
        backends.push("ladybug");
    }
    backends
}
