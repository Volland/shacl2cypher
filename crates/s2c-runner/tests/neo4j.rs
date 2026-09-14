//! Runs the conformance fixtures against a live Neo4j database.
//!
//! Needs `S2C_NEO4J_URI` (and `S2C_NEO4J_PASSWORD`, `S2C_NEO4J_USER`); the tests are
//! skipped when the URI is unset. The database is wiped for every fixture, so the
//! tests in this file must not run concurrently against the same server.
#![cfg(feature = "neo4j")]

mod common;

use std::sync::Mutex;
use std::time::Duration;

use s2c_core::render::Dialect;
use s2c_runner::executor::{ExecError, Executor, Params};
use s2c_runner::neo4j::{Neo4jConfig, Neo4jExecutor};
use s2c_testkit::fixture::Fixture;

static DATABASE: Mutex<()> = Mutex::new(());

pub fn config() -> Option<Neo4jConfig> {
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
                .map_err(|e| format!("  load: {statement}: {e}"))?;
        }
        Ok(())
    })
}

// @lat: [[testing#Runner Integration Tests]]
#[test]
fn conformance_fixtures_validate_on_neo4j() {
    let Some(config) = config() else {
        eprintln!("skipping: S2C_NEO4J_URI is not set");
        return;
    };
    let _guard = DATABASE.lock().unwrap_or_else(|e| e.into_inner());
    common::run_fixtures("neo4j", Dialect::Neo4j, |fixture: &Fixture| {
        load(&config, &s2c_testkit::lpg::neo4j_script(fixture))?;
        let executor = Neo4jExecutor::connect(config.clone()).map_err(|e| e.to_string())?;
        Ok(Box::new(executor) as Box<dyn Executor>)
    });
}

#[test]
fn queries_roll_back_and_time_out() {
    let Some(config) = config() else {
        eprintln!("skipping: S2C_NEO4J_URI is not set");
        return;
    };
    let _guard = DATABASE.lock().unwrap_or_else(|e| e.into_inner());
    load(&config, &["CREATE (:Person {id: 'p1'})".to_owned()]).unwrap();
    let mut executor = Neo4jExecutor::connect(config).unwrap();
    let params = Params {
        limit: 10,
        sample_size: 1,
    };
    let count = |executor: &mut Neo4jExecutor| {
        executor
            .run("MATCH (n) RETURN count(n) AS n", params, None)
            .unwrap()[0]["n"]
            .as_u64()
            .unwrap()
    };
    assert_eq!(count(&mut executor), 1);
    executor
        .run("CREATE (:Scratch {id: 'x'}) RETURN 1 AS ok", params, None)
        .unwrap();
    assert_eq!(count(&mut executor), 1);

    let slow = executor.run(
        "UNWIND range(1, 100000000) AS a WITH a WHERE a % 7 = 3 RETURN count(*) AS n",
        params,
        Some(Duration::from_millis(50)),
    );
    assert_eq!(slow, Err(ExecError::Timeout));
    assert_eq!(count(&mut executor), 1);
}
