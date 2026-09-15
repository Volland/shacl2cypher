//! Runs the conformance fixtures against a live FalkorDB server.
//!
//! Needs `S2C_FALKORDB_URL` (e.g. `redis://localhost:6379`); the tests are skipped when
//! it is unset. Every fixture loads into its own graph, deleted when the test ends.
#![cfg(feature = "falkordb")]

mod common;

use std::time::Duration;

use falkordb::{FalkorClientBuilder, FalkorConnectionInfo, FalkorSyncClient};
use s2c_testkit::fixture::Fixture;
use shacl2cypher_core::compile::{compile, CompileOptions, CompileRequest};
use shacl2cypher_core::render::Dialect;
use shacl2cypher_core::schema::SchemaSnapshot;
use shacl2cypher_runner::executor::{ExecError, Executor, Params};
use shacl2cypher_runner::falkordb::{FalkorDbConfig, FalkorDbExecutor};
use shacl2cypher_runner::session;

fn url() -> Option<String> {
    std::env::var("S2C_FALKORDB_URL").ok()
}

/// Graphs created by a test, deleted when it ends, even after a failed assertion.
struct Graphs {
    client: FalkorSyncClient,
    names: Vec<String>,
}

impl Graphs {
    fn new(url: &str) -> Self {
        let info: FalkorConnectionInfo = url.try_into().unwrap();
        let client = FalkorClientBuilder::new()
            .with_connection_info(info)
            .build()
            .unwrap();
        Graphs {
            client,
            names: Vec::new(),
        }
    }

    /// Creates a new graph holding `statements`; returns its name, unique across the
    /// tests of this process, which run concurrently against one server.
    fn load(&mut self, statements: &[String]) -> Result<String, String> {
        static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let index = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let name = format!("s2c_test_{}_{index}", std::process::id());
        self.names.push(name.clone());
        let mut graph = self.client.select_graph(&name);
        // GRAPH.QUERY creates the graph even when the fixture has no nodes.
        graph
            .query("MATCH (n) RETURN count(n)")
            .execute()
            .map(|_| ())
            .map_err(|e| format!("  create graph: {e}"))?;
        for statement in statements {
            graph
                .query(statement.as_str())
                .execute()
                .map(|_| ())
                .map_err(|e| format!("  load: {statement}: {e}"))?;
        }
        Ok(name)
    }
}

impl Drop for Graphs {
    fn drop(&mut self) {
        for name in &self.names {
            let _ = self.client.select_graph(name).delete();
        }
    }
}

// @lat: [[tests#Conformance#Fixtures on FalkorDB]]
#[test]
fn conformance_fixtures_validate_on_falkordb() {
    let Some(url) = url() else {
        eprintln!("skipping: S2C_FALKORDB_URL is not set");
        return;
    };
    let mut graphs = Graphs::new(&url);
    common::run_fixtures("falkordb", Dialect::FalkorDb, |fixture: &Fixture| {
        let graph = graphs.load(&s2c_testkit::lpg::falkordb_script(fixture)?)?;
        let executor = FalkorDbExecutor::connect(&FalkorDbConfig {
            url: url.clone(),
            graph,
        })
        .map_err(|e| e.to_string())?;
        Ok(Box::new(executor) as Box<dyn Executor>)
    });
}

// @lat: [[tests#Runner#Read-Only FalkorDB]]
#[test]
fn graphs_are_read_only_and_queries_time_out() {
    let Some(url) = url() else {
        eprintln!("skipping: S2C_FALKORDB_URL is not set");
        return;
    };
    let fixture =
        Fixture::load(&s2c_testkit::fixtures_dir().join("core/min-count-basic.yaml")).unwrap();
    let mut graphs = Graphs::new(&url);
    let graph = graphs
        .load(&s2c_testkit::lpg::falkordb_script(&fixture).unwrap())
        .unwrap();
    let mut executor = FalkorDbExecutor::connect(&FalkorDbConfig {
        url: url.clone(),
        graph: graph.clone(),
    })
    .unwrap();
    let params = Params {
        limit: 10,
        sample_size: 1,
    };
    let count = |executor: &mut FalkorDbExecutor| {
        executor
            .run("MATCH (n) RETURN count(n) AS n", params, None)
            .unwrap()[0]["n"]
            .as_u64()
            .unwrap()
    };
    let before = count(&mut executor);
    let write = executor.run("CREATE (:Scratch {id: 'x'})", params, None);
    assert!(matches!(write, Err(ExecError::Query(_))), "{write:?}");
    assert_eq!(count(&mut executor), before);

    // Large enough to outlast the timeout, small enough not to exhaust server memory.
    let slow = executor.run(
        "UNWIND range(1, 20000000) AS a WITH a WHERE a % 7 = 3 RETURN count(*) AS n",
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

    let missing_name = format!("s2c_missing_{}", std::process::id());
    let missing = FalkorDbExecutor::connect(&FalkorDbConfig {
        url: url.clone(),
        graph: missing_name.clone(),
    });
    assert!(
        matches!(&missing, Err(ExecError::Connection(message)) if message.contains("does not exist")),
        "{:?}",
        missing.err()
    );
    assert!(!graphs.client.list_graphs().unwrap().contains(&missing_name));

    let schema = executor.schema().unwrap();
    assert!(schema.node_types.iter().any(|node| node.name == "Person"));
    assert_eq!(
        SchemaSnapshot::from_json(&schema.to_json()).unwrap(),
        schema
    );

    let shapes = [fixture.shapes.clone()];
    let neo4j = compile(
        &CompileRequest {
            shapes: &shapes,
            ..CompileRequest::default()
        },
        &CompileOptions::default(),
    )
    .unwrap()
    .manifest;
    let mismatch = session::check_dialect(&neo4j, executor.dialect()).unwrap_err();
    assert!(
        mismatch.contains("neo4j") && mismatch.contains("falkordb"),
        "{mismatch}"
    );
}
