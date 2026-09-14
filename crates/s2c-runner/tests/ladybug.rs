//! Runs the conformance fixtures against embedded LadybugDB databases.
#![cfg(feature = "ladybug")]

mod common;

use std::path::{Path, PathBuf};
use std::time::Duration;

use s2c_core::render::Dialect;
use s2c_runner::executor::{ExecError, Executor, Params};
use s2c_runner::ladybug::LadybugExecutor;
use s2c_testkit::fixture::Fixture;

/// Writes the fixture's LPG projection into a new database file.
fn create_database(fixture: &Fixture, dir: &Path) -> Result<PathBuf, String> {
    let path = dir.join("graph.lbug");
    let db =
        lbug::Database::new(&path, lbug::SystemConfig::default()).map_err(|e| e.to_string())?;
    let conn = lbug::Connection::new(&db).map_err(|e| e.to_string())?;
    for statement in s2c_testkit::lpg::ladybug_script(fixture)? {
        conn.query(&statement)
            .map_err(|e| format!("  load: {statement}: {e}"))?;
    }
    Ok(path)
}

// @lat: [[tests#Conformance#Fixtures on LadybugDB]]
// @lat: [[testing#Runner Integration Tests]]
#[test]
fn conformance_fixtures_validate_on_ladybug() {
    let mut dirs = Vec::new();
    common::run_fixtures("ladybug", Dialect::Ladybug, |fixture| {
        let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
        let path = create_database(fixture, dir.path())?;
        dirs.push(dir);
        let executor = LadybugExecutor::open(&path).map_err(|e| e.to_string())?;
        Ok(Box::new(executor) as Box<dyn Executor>)
    });
}

// @lat: [[tests#Runner#Read-Only LadybugDB]]
#[test]
fn databases_are_opened_read_only_and_queries_time_out() {
    let fixture =
        Fixture::load(&s2c_testkit::fixtures_dir().join("core/min-count-basic.yaml")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut executor =
        LadybugExecutor::open(&create_database(&fixture, dir.path()).unwrap()).unwrap();
    let params = Params {
        limit: 10,
        sample_size: 1,
    };

    let write = executor.run("CREATE (:Person {id: 'x'})", params, None);
    assert!(matches!(write, Err(ExecError::Query(_))), "{write:?}");

    let slow = executor.run(
        "UNWIND range(1, 100000) AS a UNWIND range(1, 100000) AS b RETURN count(*) AS n",
        params,
        Some(Duration::from_millis(50)),
    );
    assert_eq!(slow, Err(ExecError::Timeout));

    let rows = executor
        .run(
            "MATCH (p:Person) RETURN p.id AS id ORDER BY id LIMIT $limit",
            Params {
                limit: 2,
                sample_size: 1,
            },
            Some(Duration::from_secs(10)),
        )
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0]["id"], "p1");

    let missing = LadybugExecutor::open(&dir.path().join("missing.lbug"));
    assert!(matches!(missing, Err(ExecError::Connection(_))));
    assert!(!dir.path().join("missing.lbug").exists());

    let schema = executor.schema().unwrap();
    assert_eq!(schema.node_types.len(), 1);
    assert_eq!(schema.node_types[0].name, "Person");
}
