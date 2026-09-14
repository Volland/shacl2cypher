use std::path::Path;
use std::process::{Command, Output};

const SHAPES: &str = "@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix ex: <http://example.org/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .

ex:PersonShape sh:targetClass ex:Person ;
    sh:property [ sh:path ex:name ; sh:minCount 1 ; sh:datatype xsd:string ] .
";

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_shacl2cypher"))
        .current_dir(dir)
        .args(args)
        .output()
        .unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

// @lat: [[tests#Manifest#Compile Outputs]]
#[test]
fn compile_writes_manifest_and_queries() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("shapes")).unwrap();
    std::fs::write(dir.path().join("shapes/person.ttl"), SHAPES).unwrap();

    let output = run(
        dir.path(),
        &[
            "compile",
            "shapes/person.ttl",
            "--dialect",
            "neo4j",
            "--node-key",
            "id",
            "-o",
            "out/",
        ],
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(stderr(&output).contains("compiled 2 rules (2 with queries"));

    let manifest: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.path().join("out/manifest.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(manifest["dialect"], "neo4j");
    assert_eq!(manifest["inputs"][0]["path"], "shapes/person.ttl");
    assert_eq!(manifest["options"]["nodeKey"], "id");
    assert_eq!(manifest["rules"].as_array().unwrap().len(), 2);

    let cypher = std::fs::read_to_string(dir.path().join("out/queries.cypher")).unwrap();
    assert_eq!(cypher.matches("\n// name: ").count(), 4);
    assert!(cypher.contains("// name: PersonShape.name.minCount\n"));
    assert!(cypher.contains("// name: PersonShape.name.datatype#summary\n"));

    let again = run(
        dir.path(),
        &[
            "compile",
            "shapes/person.ttl",
            "--dialect",
            "neo4j",
            "--node-key",
            "id",
            "-o",
            "again",
        ],
    );
    assert!(again.status.success());
    for file in ["manifest.json", "queries.cypher"] {
        assert_eq!(
            std::fs::read(dir.path().join("out").join(file)).unwrap(),
            std::fs::read(dir.path().join("again").join(file)).unwrap()
        );
    }
}

// @lat: [[tests#Runner#Setup Error Exit Codes]]
#[test]
fn compile_errors_exit_non_zero() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("person.ttl"), SHAPES).unwrap();

    let ladybug = run(
        dir.path(),
        &["compile", "person.ttl", "--dialect", "ladybug"],
    );
    assert_eq!(ladybug.status.code(), Some(1));
    assert!(stderr(&ladybug).contains("error: the LadybugDB dialect requires a schema snapshot"));

    std::fs::write(
        dir.path().join("remote.ttl"),
        format!("{SHAPES}<> <http://www.w3.org/2002/07/owl#imports> <https://example.org/shapes.ttl> .\n"),
    )
    .unwrap();
    let remote = run(dir.path(), &["compile", "remote.ttl", "--dialect", "neo4j"]);
    assert_eq!(remote.status.code(), Some(1));
    assert!(
        stderr(&remote).contains("https://example.org/shapes.ttl"),
        "{}",
        stderr(&remote)
    );
    assert!(!dir.path().join("manifest.json").exists());

    let usage = run(dir.path(), &["compile", "person.ttl"]);
    assert_eq!(usage.status.code(), Some(2));
}

#[test]
fn ladybug_compile_reports_schema_diagnostics() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("person.ttl"), SHAPES).unwrap();
    std::fs::write(
        dir.path().join("schema.json"),
        r#"{"nodeTypes": [{"name": "Person", "properties": [{"name": "name", "type": "INT64"}]}]}"#,
    )
    .unwrap();
    let args = [
        "compile",
        "person.ttl",
        "--dialect",
        "ladybug",
        "--schema",
        "schema.json",
    ];

    let output = run(dir.path(), &args);
    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("warning: person.ttl:"),
        "{}",
        stderr(&output)
    );

    let failing = run(
        dir.path(),
        &[&args[..], &["--fail-on-schema-mismatch"]].concat(),
    );
    assert_eq!(failing.status.code(), Some(1));
}
