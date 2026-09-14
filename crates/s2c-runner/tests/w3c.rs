//! Runs the vendored W3C SHACL Core tests against Neo4j and checks every outcome
//! against `tests/w3c/status.yaml`.
//!
//! Outcomes are `pass`, `fail: …` or `skip: <reason>`. Without `S2C_NEO4J_URI` only
//! static skips are checked. `S2C_W3C_BLESS=1` (with a database) rewrites the file.
#![cfg(feature = "neo4j")]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

use s2c_core::compile::{compile, CompileOptions, CompileRequest};
use s2c_core::render::Dialect;
use s2c_runner::executor::Executor;
use s2c_runner::neo4j::{Neo4jConfig, Neo4jExecutor};
use s2c_runner::validate::{validate, Status, ValidateOptions};
use s2c_testkit::w3c::{self, W3cTest};

fn config() -> Option<Neo4jConfig> {
    Some(Neo4jConfig {
        uri: std::env::var("S2C_NEO4J_URI").ok()?,
        user: std::env::var("S2C_NEO4J_USER").unwrap_or_else(|_| "neo4j".into()),
        password: std::env::var("S2C_NEO4J_PASSWORD").unwrap_or_default(),
        database: None,
    })
}

/// Replaces the database contents with `statements`.
fn load(config: &Neo4jConfig, statements: &[String]) -> Result<(), String> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())?;
    runtime.block_on(async {
        let graph = neo4rs::Graph::new(&config.uri, &config.user, &config.password)
            .map_err(|e| e.to_string())?;
        graph
            .run(neo4rs::query("MATCH (n) DETACH DELETE n"))
            .await
            .map_err(|e| e.to_string())?;
        for statement in statements {
            graph
                .run(neo4rs::query(statement))
                .await
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    })
}

fn first_line(text: &str, dir: &Path) -> String {
    let mut line = text.lines().next().unwrap_or_default().to_owned();
    // The canonical form (e.g. /private/var/... on macOS) contains the plain one.
    for prefix in [dir.canonicalize().unwrap_or_default(), dir.to_path_buf()] {
        let prefix = format!("{}/", prefix.display());
        line = line.replace(&prefix, "");
    }
    line
}

fn outcome(config: &Neo4jConfig, test: &W3cTest) -> String {
    let script = match w3c::neo4j_script(test) {
        Ok(script) => script,
        Err(reason) => return format!("skip: projection: {reason}"),
    };
    let dir = tempfile::tempdir().unwrap();
    if let Err(error) = load(config, &script) {
        return format!("skip: load: {}", first_line(&error, dir.path()));
    }
    let mut executor = Neo4jExecutor::connect(config.clone()).unwrap_or_else(|e| panic!("{e}"));
    let schema = executor
        .schema()
        .unwrap_or_else(|e| panic!("{e}"))
        .to_json();
    let shapes = [dir.path().join("shapes.nt")];
    std::fs::write(&shapes[0], w3c::ntriples(test)).unwrap();
    let request = CompileRequest {
        shapes: &shapes,
        ontologies: &[],
        schema: Some(&schema),
        fetcher: None,
    };
    let options = CompileOptions {
        dialect: Dialect::Neo4j,
        node_key: Some("id".into()),
        base_dir: Some(dir.path().to_path_buf()),
        ..CompileOptions::default()
    };
    let compilation = match compile(&request, &options) {
        Ok(compilation) => compilation,
        Err(error) => {
            return format!(
                "skip: compile: {}",
                first_line(&error.to_string(), dir.path())
            )
        }
    };
    let validate_options = ValidateOptions {
        limit: None,
        timeout: Some(Duration::from_secs(30)),
        ..ValidateOptions::default()
    };
    let report = validate(&compilation.manifest, &mut executor, &validate_options).unwrap();
    if let Some(rule) = report.rules.iter().find(|rule| {
        matches!(rule.status, Status::Error | Status::Timeout)
            || (rule.status == Status::Failed && rule.status_reason.is_some())
    }) {
        return format!("skip: runtime: {} {:?}", rule.constraint, rule.status);
    }
    let actual: BTreeSet<(String, String)> = report
        .rules
        .iter()
        .flat_map(|rule| {
            rule.violations.iter().map(|row| {
                (
                    row["focus"]["keyValue"].as_str().unwrap_or("?").to_owned(),
                    rule.constraint.clone(),
                )
            })
        })
        .collect();
    let expected: BTreeSet<(String, String)> = test
        .expected
        .iter()
        .map(|(focus, constraint)| (w3c::local_name(focus).to_owned(), constraint.clone()))
        .collect();
    if actual == expected {
        return "pass".into();
    }
    let describe = |items: Vec<&(String, String)>| {
        items
            .into_iter()
            .map(|(focus, constraint)| format!("{constraint}@{focus}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    format!(
        "fail: missing [{}]; unexpected [{}]",
        describe(expected.difference(&actual).collect()),
        describe(actual.difference(&expected).collect())
    )
}

// @lat: [[testing#W3C Test Suite]]
#[test]
fn w3c_core_suite_matches_recorded_status() {
    let root = w3c::suite_root();
    let status_path = root.parent().unwrap().join("status.yaml");
    let recorded: BTreeMap<String, String> = std::fs::read_to_string(&status_path)
        .ok()
        .map(|text| serde_yaml_ng::from_str(&text).unwrap())
        .unwrap_or_default();
    let config = config();
    let bless = std::env::var_os("S2C_W3C_BLESS").is_some();
    assert!(
        !bless || config.is_some(),
        "S2C_W3C_BLESS needs S2C_NEO4J_URI"
    );

    let mut observed = BTreeMap::new();
    let mut failures = Vec::new();
    for file in w3c::suite_files(&root) {
        let test = w3c::load(&root, &file).unwrap_or_else(|e| panic!("{e}"));
        if test.skip.as_deref() == Some(w3c::COMPANION) {
            continue;
        }
        let result = match (&test.skip, &config) {
            (Some(reason), _) => format!("skip: {reason}"),
            (None, Some(config)) => outcome(config, &test),
            // Without a database only static skips can be checked.
            (None, None) => match recorded.get(&test.name) {
                Some(recorded) => recorded.clone(),
                None => "unchecked".into(),
            },
        };
        match recorded.get(&test.name) {
            Some(want) if *want == result => {}
            Some(want) => failures.push(format!(
                "{}: recorded `{want}`, observed `{result}`",
                test.name
            )),
            None => failures.push(format!(
                "{}: not in status.yaml (observed `{result}`)",
                test.name
            )),
        }
        observed.insert(test.name, result);
    }

    let passing = observed.values().filter(|o| *o == "pass").count();
    eprintln!("w3c: {passing} of {} core tests pass", observed.len());
    if bless {
        let yaml = serde_yaml_ng::to_string(&observed).unwrap();
        std::fs::write(
            &status_path,
            format!("# Outcome of each W3C SHACL Core test on Neo4j.\n# Regenerate with S2C_W3C_BLESS=1 and S2C_NEO4J_URI set.\n{yaml}"),
        )
        .unwrap();
        return;
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
