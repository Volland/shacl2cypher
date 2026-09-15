//! Fixture projection CLI for external oracles and binding tests.
//!
//! `s2c-fixture oracle-input <fixture.yaml>` prints JSON with the shapes path, base
//! IRI, the RDF projection as N-Triples and the expected violations.
//!
//! `s2c-fixture ladybug-db <fixture.yaml> <out.lbug>` (feature `ladybug`) writes the
//! fixture's LPG projection into a new LadybugDB database file.

use std::path::Path;
use std::process::exit;

use s2c_testkit::{fixture::Fixture, rdf};

const USAGE: &str =
    "usage: s2c-fixture oracle-input <fixture.yaml>\n       s2c-fixture ladybug-db <fixture.yaml> <out.lbug>";

// @lat: [[testing#Conformance Fixtures]]
fn main() {
    let args: Vec<String> = std::env::args().collect();
    match args.as_slice() {
        [_, command, path] if command == "oracle-input" => oracle_input(&load(path)),
        [_, command, path, out] if command == "ladybug-db" => {
            ladybug_db(&load(path), Path::new(out))
        }
        _ => {
            eprintln!("{USAGE}");
            exit(2);
        }
    }
}

fn load(path: &str) -> Fixture {
    Fixture::load(Path::new(path)).unwrap_or_else(|e| {
        eprintln!("{e}");
        exit(1);
    })
}

fn oracle_input(fixture: &Fixture) {
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
        "ntriples": rdf::to_ntriples(fixture),
        "expect": expect,
        "oracleSkip": fixture.oracle_skip,
        "knownDifferences": fixture
            .known_differences
            .iter()
            .filter(|k| k.engine == "pyshacl")
            .map(|k| serde_json::json!({"rule": k.rule, "focus": k.focus, "reason": k.reason}))
            .collect::<Vec<_>>(),
    });
    println!("{out}");
}

#[cfg(feature = "ladybug")]
fn ladybug_db(fixture: &Fixture, out: &Path) {
    if out.exists() {
        eprintln!("{}: already exists", out.display());
        exit(1);
    }
    if let Err(e) = s2c_testkit::lpg::create_ladybug_database(fixture, out) {
        eprintln!("{e}");
        exit(1);
    }
}

#[cfg(not(feature = "ladybug"))]
fn ladybug_db(_fixture: &Fixture, _out: &Path) {
    eprintln!("s2c-fixture was built without the `ladybug` feature");
    exit(2);
}
