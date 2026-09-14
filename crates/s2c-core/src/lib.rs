//! Pure, deterministic SHACL-to-Cypher compiler core.
//!
//! Loads shapes, resolves them against the LPG mapping and schema snapshot,
//! lowers constraints into a dialect-neutral IR and renders named diagnostic
//! queries. This crate performs no database I/O.

pub mod ast;
pub mod cycles;
pub mod datatypes;
pub mod hierarchy;
pub mod load;
pub mod mapping;
pub mod schema;
pub mod schema_check;
