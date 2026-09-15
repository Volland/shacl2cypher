use std::process::Command;

const SHAPES: &str = "@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix ex: <http://example.org/> .
ex:PersonShape sh:targetClass ex:Person ;
    sh:property [ sh:path ex:name ; sh:minCount 1 ] .
";

fn shacl2cypher(dir: &std::path::Path, args: &[&str]) -> (Option<i32>, String) {
    let output = Command::new(env!("CARGO_BIN_EXE_shacl2cypher"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap();
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

// @lat: [[tests#Runner#Compile-Only Builds]]
#[test]
#[cfg(not(any(feature = "neo4j", feature = "ladybug", feature = "falkordb")))]
fn compile_only_builds_report_no_backend() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("shapes.ttl"), SHAPES).unwrap();
    for args in [
        &["validate", "shapes.ttl", "--ladybug", "graph.lbug"][..],
        &[
            "validate",
            "shapes.ttl",
            "--connect",
            "bolt://localhost:7687",
        ][..],
        &[
            "validate",
            "shapes.ttl",
            "--falkordb",
            "redis://localhost:6379",
            "--graph",
            "g",
        ][..],
        &["schema", "dump", "--ladybug", "graph.lbug"][..],
    ] {
        let (code, stderr) = shacl2cypher(dir.path(), args);
        assert_eq!(code, Some(2), "{args:?}: {stderr}");
        assert!(
            stderr.contains("no database backend is available in this build"),
            "{stderr}"
        );
    }
}

#[test]
fn validate_argument_errors_are_usage_errors() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("shapes.ttl"), SHAPES).unwrap();
    let (code, _) = shacl2cypher(dir.path(), &["validate"]);
    assert_eq!(code, Some(2));
    let (code, stderr) = shacl2cypher(
        dir.path(),
        &[
            "validate",
            "shapes.ttl",
            "--connect",
            "bolt://x",
            "--ladybug",
            "graph.lbug",
        ],
    );
    assert_eq!(code, Some(2));
    assert!(stderr.contains("cannot be used with"), "{stderr}");
}

#[test]
#[cfg(feature = "ladybug")]
fn missing_ladybug_database_is_a_setup_error() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("shapes.ttl"), SHAPES).unwrap();
    let (code, stderr) = shacl2cypher(
        dir.path(),
        &["validate", "shapes.ttl", "--ladybug", "missing.lbug"],
    );
    assert_eq!(code, Some(2), "{stderr}");
    assert!(stderr.contains("no such LadybugDB database"), "{stderr}");
    assert!(!dir.path().join("missing.lbug").exists());
}

#[test]
fn falkordb_needs_a_graph_and_a_single_database() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("shapes.ttl"), SHAPES).unwrap();
    let (code, stderr) = shacl2cypher(
        dir.path(),
        &[
            "validate",
            "shapes.ttl",
            "--falkordb",
            "redis://localhost:6379",
        ],
    );
    assert_eq!(code, Some(2), "{stderr}");
    assert!(stderr.contains("--graph <NAME>"), "{stderr}");
    let (code, stderr) = shacl2cypher(dir.path(), &["schema", "dump", "--graph", "g"]);
    assert_eq!(code, Some(2), "{stderr}");
    assert!(stderr.contains("--falkordb <URL>"), "{stderr}");
    let (code, stderr) = shacl2cypher(
        dir.path(),
        &[
            "validate",
            "shapes.ttl",
            "--falkordb",
            "redis://localhost:6379",
            "--graph",
            "g",
            "--ladybug",
            "graph.lbug",
        ],
    );
    assert_eq!(code, Some(2), "{stderr}");
    assert!(stderr.contains("cannot be used with"), "{stderr}");
}

#[test]
#[cfg(all(any(feature = "neo4j", feature = "ladybug"), not(feature = "falkordb")))]
fn missing_falkordb_backend_is_a_setup_error() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("shapes.ttl"), SHAPES).unwrap();
    let (code, stderr) = shacl2cypher(
        dir.path(),
        &[
            "validate",
            "shapes.ttl",
            "--falkordb",
            "redis://localhost:6379",
            "--graph",
            "g",
        ],
    );
    assert_eq!(code, Some(2), "{stderr}");
    assert!(
        stderr.contains("the falkordb backend is not available in this build"),
        "{stderr}"
    );
}

// @lat: [[tests#Runner#FalkorDB Setup Errors]]
#[test]
#[cfg(feature = "falkordb")]
fn falkordb_connection_problems_are_setup_errors() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("shapes.ttl"), SHAPES).unwrap();
    let validate = |url: &str, graph: &str| {
        shacl2cypher(
            dir.path(),
            &[
                "validate",
                "shapes.ttl",
                "--falkordb",
                url,
                "--graph",
                graph,
            ],
        )
    };
    let (code, stderr) = validate("rediss://localhost:6379", "g");
    assert_eq!(code, Some(2), "{stderr}");
    assert!(
        stderr.contains("TLS connections are not supported"),
        "{stderr}"
    );
    let (code, stderr) = validate("redis://127.0.0.1:1", "g");
    assert_eq!(code, Some(2), "{stderr}");
    assert!(stderr.contains("redis://127.0.0.1:1"), "{stderr}");

    let Ok(url) = std::env::var("S2C_FALKORDB_URL") else {
        eprintln!("skipping the live part: S2C_FALKORDB_URL is not set");
        return;
    };
    let (code, stderr) = validate(&url, "s2c_graph_that_does_not_exist");
    assert_eq!(code, Some(2), "{stderr}");
    assert!(
        stderr.contains("graph `s2c_graph_that_does_not_exist` does not exist"),
        "{stderr}"
    );
}
