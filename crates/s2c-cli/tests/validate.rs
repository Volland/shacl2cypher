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

#[test]
#[cfg(not(any(feature = "neo4j", feature = "ladybug")))]
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
