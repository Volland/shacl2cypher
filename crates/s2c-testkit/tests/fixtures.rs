//! Every conformance fixture must load, validate and project to RDF and to each
//! LPG dialect it declares, so broken fixtures fail fast before any engine runs.

use std::path::PathBuf;

use s2c_testkit::{fixture::Fixture, fixtures_dir, lpg, rdf};

fn fixture_files() -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut dirs = vec![fixtures_dir()];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|e| e == "yaml") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

#[test]
fn all_fixtures_load_and_project() {
    let files = fixture_files();
    assert!(
        !files.is_empty(),
        "no fixtures found under {}",
        fixtures_dir().display()
    );
    for path in files {
        let fixture = Fixture::load(&path).unwrap_or_else(|e| panic!("{e}"));
        assert!(
            fixture.shapes.is_file(),
            "{}: missing shapes file",
            path.display()
        );
        assert!(!rdf::to_ntriples(&fixture).is_empty() || fixture.graph.nodes.is_empty());
        if fixture.runs_on("neo4j") {
            lpg::neo4j_script(&fixture);
        }
        if fixture.runs_on("ladybug") {
            lpg::ladybug_script(&fixture).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        }
    }
}
