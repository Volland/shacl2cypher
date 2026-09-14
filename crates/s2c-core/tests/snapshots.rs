//! Snapshots of the generated `.cypher` file per dialect, so query-shape changes
//! show up in code review. Update with `INSTA_UPDATE=always cargo test -p s2c-core --test snapshots`.

use std::path::PathBuf;

use s2c_core::compile::{compile, CompileOptions, CompileRequest};
use s2c_core::render::Dialect;

fn data(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/data")
        .join(name)
}

fn cypher(dialect: Dialect, schema: Option<&str>) -> String {
    let shapes = [data("snapshot.ttl")];
    let request = CompileRequest {
        shapes: &shapes,
        ontologies: &[],
        schema,
        fetcher: None,
    };
    let options = CompileOptions {
        dialect,
        node_key: Some("id".into()),
        base_dir: Some(data("")),
        ..CompileOptions::default()
    };
    compile(&request, &options).unwrap().cypher
}

// @lat: [[testing#Cypher Snapshots]]
#[test]
fn neo4j_queries() {
    insta::assert_snapshot!("neo4j", cypher(Dialect::Neo4j, None));
}

// @lat: [[tests#Manifest#Cypher Snapshots]]
#[test]
fn ladybug_queries() {
    let schema = std::fs::read_to_string(data("snapshot-schema.json")).unwrap();
    insta::assert_snapshot!("ladybug", cypher(Dialect::Ladybug, Some(&schema)));
}
