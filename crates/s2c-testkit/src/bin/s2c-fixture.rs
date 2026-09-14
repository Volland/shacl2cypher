//! Fixture projection CLI for external oracles such as pySHACL.
//!
//! `s2c-fixture oracle-input <fixture.yaml>` prints JSON with the shapes path, base
//! IRI, the RDF projection as N-Triples and the expected violations.

use std::path::Path;
use std::process::exit;

use s2c_testkit::{fixture::Fixture, rdf};

const USAGE: &str = "usage: s2c-fixture oracle-input <fixture.yaml>";

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let [_, command, path] = args.as_slice() else {
        eprintln!("{USAGE}");
        exit(2);
    };
    if command != "oracle-input" {
        eprintln!("{USAGE}");
        exit(2);
    }
    let fixture = Fixture::load(Path::new(path)).unwrap_or_else(|e| {
        eprintln!("{e}");
        exit(1);
    });
    let expect: Vec<_> = fixture
        .expect
        .iter()
        .map(|v| serde_json::json!({"rule": v.rule, "focus": v.focus}))
        .collect();
    let out = serde_json::json!({
        "shapes": fixture.shapes.display().to_string(),
        "base": fixture.base,
        "dialects": fixture.dialects,
        "notes": fixture.notes,
        "ntriples": rdf::to_ntriples(&fixture),
        "expect": expect,
    });
    println!("{out}");
}
