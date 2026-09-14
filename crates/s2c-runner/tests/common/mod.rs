//! Conformance harness shared by the database integration tests.
//!
//! Set `S2C_FIXTURE=<substring>` to run only matching fixture files.

use std::path::{Path, PathBuf};

use s2c_testkit::compare::{compare, detail_mismatches, without_known};
use s2c_testkit::fixture::{Expected, Fixture, ID_PROPERTY};
use serde_json::Value;
use shacl2cypher_core::compile::{compile, CompileOptions, CompileRequest};
use shacl2cypher_core::render::Dialect;
use shacl2cypher_core::schema::SchemaSnapshot;
use shacl2cypher_runner::executor::Executor;
use shacl2cypher_runner::validate::{validate, Status, ValidateOptions};

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            collect(&path, out);
        } else if path.extension().is_some_and(|e| e == "yaml") {
            out.push(path);
        }
    }
}

pub fn fixture_paths() -> Vec<PathBuf> {
    let filter = std::env::var("S2C_FIXTURE").ok();
    let mut paths = Vec::new();
    collect(&s2c_testkit::fixtures_dir(), &mut paths);
    paths.retain(|path| {
        filter
            .as_deref()
            .is_none_or(|f| path.to_string_lossy().contains(f))
    });
    paths.sort();
    paths
}

/// Runs every fixture for `engine`; `open` loads the fixture graph into a fresh
/// database and returns an executor on it.
pub fn run_fixtures(
    engine: &str,
    dialect: Dialect,
    mut open: impl FnMut(&Fixture) -> Result<Box<dyn Executor>, String>,
) {
    let mut failures = Vec::new();
    let mut ran = 0;
    for path in fixture_paths() {
        let fixture = Fixture::load(&path).unwrap_or_else(|e| panic!("{e}"));
        if !fixture.runs_on(engine) {
            continue;
        }
        ran += 1;
        if let Err(message) = check(&fixture, engine, dialect, &mut open) {
            failures.push(format!("{}:\n{message}", path.display()));
        }
    }
    eprintln!(
        "{engine}: {} of {ran} fixtures agree with `expect`",
        ran - failures.len()
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

fn check(
    fixture: &Fixture,
    engine: &str,
    dialect: Dialect,
    open: &mut impl FnMut(&Fixture) -> Result<Box<dyn Executor>, String>,
) -> Result<(), String> {
    let mut executor = open(fixture)?;
    let schema = executor
        .schema()
        .map_err(|e| format!("  schema dump: {e}"))?;
    let schema_json = schema.to_json();
    if SchemaSnapshot::from_json(&schema_json).map_err(|e| format!("  schema: {e}"))? != schema {
        return Err("  the schema snapshot does not round-trip".into());
    }
    let shapes = [fixture.shapes.clone()];
    let compilation = compile(
        &CompileRequest {
            shapes: &shapes,
            ontologies: &[],
            schema: Some(&schema_json),
            fetcher: None,
        },
        &CompileOptions {
            dialect,
            node_key: Some(ID_PROPERTY.into()),
            ..CompileOptions::default()
        },
    )
    .map_err(|e| format!("  compile: {e}"))?;
    let options = ValidateOptions {
        limit: None,
        ..ValidateOptions::default()
    };
    let report = validate(&compilation.manifest, executor.as_mut(), &options)?;

    let mut problems: Vec<String> = report
        .rules
        .iter()
        .filter(|rule| {
            matches!(rule.status, Status::Timeout | Status::Error)
                || rule.status_reason.is_some() && rule.status == Status::Failed
        })
        .map(|rule| {
            format!(
                "  {} {:?}: {}",
                rule.rule_id,
                rule.status,
                rule.status_reason.as_deref().unwrap_or("")
            )
        })
        .collect();
    let actual: Vec<Expected> = report
        .rules
        .iter()
        .flat_map(|rule| {
            rule.violations.iter().map(|row| Expected {
                rule: rule.rule_id.clone(),
                focus: focus_id(&row["focus"]),
                details: Some(strings(&row["details"])),
            })
        })
        .collect();
    let diff = without_known(
        compare(&fixture.expect, &actual),
        &fixture.known_differences,
        engine,
    );
    if !diff.is_empty() {
        problems.push(diff.to_string().trim_end().to_owned());
    }
    problems.extend(
        detail_mismatches(&fixture.expect, &actual)
            .into_iter()
            .map(|m| format!("  details: {m}")),
    );
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("\n"))
    }
}

/// Fixture id of a node focus, or `from->to` of a relationship focus.
fn focus_id(focus: &Value) -> String {
    let text = |key: &str| focus.get(key).and_then(Value::as_str).unwrap_or("?");
    if focus.get("type").is_some() {
        format!("{}->{}", text("startKey"), text("endKey"))
    } else {
        text("keyValue").to_owned()
    }
}

fn strings(value: &Value) -> Vec<String> {
    match value {
        Value::Array(items) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map_or_else(|| item.to_string(), str::to_owned)
            })
            .collect(),
        _ => Vec::new(),
    }
}
