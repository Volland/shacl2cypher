//! Compilation must be deterministic: repeated runs, input order, triple order and
//! `sh:property` order never change rule names, ids, fingerprints or queries.

use std::path::{Path, PathBuf};

use s2c_core::compile::{compile, Compilation, CompileOptions, CompileRequest};
use s2c_core::render::Dialect;

const PREFIXES: &str = "@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix ex: <http://example.org/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
";

const PROPERTIES: &[&str] = &[
    "[ sh:path ex:name ; sh:minCount 1 ; sh:datatype xsd:string ; sh:pattern \"^[A-Z]\" ]",
    "[ sh:path ex:age ; sh:maxInclusive 150 ; sh:minInclusive 0 ]",
    "[ sh:path ex:worksFor ; sh:class ex:Company ; sh:maxCount 1 ]",
    "[ sh:path ex:address ; sh:node ex:AddressShape ]",
    "[ sh:path [ sh:alternativePath ( ex:email ex:phone ) ] ; sh:minCount 1 ]",
    "[ sh:path ( ex:worksFor ex:name ) ; sh:minLength 2 ]",
];

const REST: &str = "
ex:AddressShape sh:property [ sh:path ex:zip ; sh:minCount 1 ] , [ sh:path ex:city ; sh:maxCount 1 ] .
ex:CompanyShape sh:targetClass ex:Company ;
    sh:or ( [ sh:path ex:name ; sh:minCount 1 ] [ sh:path ex:code ; sh:minCount 1 ] ) .
";

const SCHEMA: &str = r#"{
  "nodeTypes": [
    {"name": "Person", "properties": [{"name": "id", "type": "STRING"}, {"name": "name", "type": "STRING"},
      {"name": "age", "type": "INT64"}, {"name": "email", "type": "STRING"}, {"name": "phone", "type": "STRING"}]},
    {"name": "Company", "properties": [{"name": "id", "type": "STRING"}, {"name": "name", "type": "STRING"},
      {"name": "code", "type": "STRING"}]},
    {"name": "Address", "properties": [{"name": "id", "type": "STRING"}, {"name": "zip", "type": "STRING"},
      {"name": "city", "type": "STRING"}]}
  ],
  "relTypes": [
    {"name": "WORKS_FOR", "endpoints": [{"from": "Person", "to": "Company"}]},
    {"name": "ADDRESS", "endpoints": [{"from": "Person", "to": "Address"}]}
  ]
}"#;

fn shapes(order: &[usize]) -> String {
    let properties: Vec<&str> = order.iter().map(|&i| PROPERTIES[i]).collect();
    format!(
        "{PREFIXES}ex:PersonShape sh:targetClass ex:Person ;\n    sh:property {} .\n{REST}",
        properties.join(" ,\n        ")
    )
}

fn compile_in(dir: &Path, files: &[&str], dialect: Dialect) -> Compilation {
    let shapes: Vec<PathBuf> = files.iter().map(|f| dir.join(f)).collect();
    let request = CompileRequest {
        shapes: &shapes,
        ontologies: &[],
        schema: (dialect == Dialect::Ladybug).then_some(SCHEMA),
        fetcher: None,
    };
    let options = CompileOptions {
        dialect,
        node_key: Some("id".into()),
        base_dir: Some(dir.to_path_buf()),
        ..CompileOptions::default()
    };
    compile(&request, &options).unwrap_or_else(|e| panic!("{e}"))
}

/// Everything except source positions, which legitimately move when triples do.
fn stable_view(compilation: &Compilation) -> String {
    let mut out = compilation.cypher.clone();
    for rule in &compilation.manifest.rules {
        out.push_str(&format!(
            "\n{} {} {} {} {} {:?}",
            rule.name, rule.rule_id, rule.fingerprint, rule.status, rule.cost_class, rule.queries
        ));
    }
    out
}

/// Deterministic xorshift generator for shuffles.
struct Rng(u64);

impl Rng {
    fn below(&mut self, n: usize) -> usize {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        (x % n as u64) as usize
    }
}

fn natural_order() -> Vec<usize> {
    (0..PROPERTIES.len()).collect()
}

// @lat: [[testing#Determinism]]
#[test]
fn repeated_compiles_are_byte_identical() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("shapes.ttl"), shapes(&natural_order())).unwrap();
    for dialect in [Dialect::Neo4j, Dialect::Ladybug] {
        let first = compile_in(dir.path(), &["shapes.ttl"], dialect);
        let second = compile_in(dir.path(), &["shapes.ttl"], dialect);
        assert_eq!(first.manifest_json(), second.manifest_json());
        assert_eq!(first.cypher, second.cypher);
    }
}

#[test]
fn input_file_order_does_not_matter() {
    let dir = tempfile::tempdir().unwrap();
    let text = shapes(&natural_order());
    let (person, rest) = text.split_at(text.find("\nex:AddressShape").unwrap());
    std::fs::write(dir.path().join("a.ttl"), person).unwrap();
    std::fs::write(dir.path().join("b.ttl"), format!("{PREFIXES}{rest}")).unwrap();
    for dialect in [Dialect::Neo4j, Dialect::Ladybug] {
        let ab = compile_in(dir.path(), &["a.ttl", "b.ttl"], dialect);
        let ba = compile_in(dir.path(), &["b.ttl", "a.ttl"], dialect);
        assert_eq!(ab.manifest_json(), ba.manifest_json());
        assert_eq!(ab.cypher, ba.cypher);
    }
}

#[test]
fn property_and_triple_order_keep_rules_stable() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("shapes.ttl"), shapes(&natural_order())).unwrap();
    let baseline: Vec<String> = [Dialect::Neo4j, Dialect::Ladybug]
        .iter()
        .map(|&d| stable_view(&compile_in(dir.path(), &["shapes.ttl"], d)))
        .collect();

    let mut rng = Rng(0xd37e_4a11);
    for round in 0..6 {
        // Reordered `sh:property` values in Turtle.
        let mut order = natural_order();
        for i in (1..order.len()).rev() {
            order.swap(i, rng.below(i + 1));
        }
        let turtle = shapes(&order);
        std::fs::write(dir.path().join("shapes.ttl"), &turtle).unwrap();
        for (dialect, expected) in [Dialect::Neo4j, Dialect::Ladybug].iter().zip(&baseline) {
            let view = stable_view(&compile_in(dir.path(), &["shapes.ttl"], *dialect));
            assert_eq!(&view, expected, "property order {order:?}");
        }

        // The same graph as shuffled triples (N-Triples lines under the Turtle prefixes).
        let mut triples: Vec<String> = oxttl::TurtleParser::new()
            .for_slice(turtle.as_bytes())
            .map(|triple| format!("{} .", triple.unwrap()))
            .collect();
        for i in (1..triples.len()).rev() {
            triples.swap(i, rng.below(i + 1));
        }
        let name = format!("shuffled-{round}.ttl");
        std::fs::write(
            dir.path().join(&name),
            format!("{PREFIXES}{}\n", triples.join("\n")),
        )
        .unwrap();
        for (dialect, expected) in [Dialect::Neo4j, Dialect::Ladybug].iter().zip(&baseline) {
            let view = stable_view(&compile_in(dir.path(), &[&name], *dialect));
            if let Some((got, want)) = view.lines().zip(expected.lines()).find(|(g, w)| g != w) {
                panic!("shuffled triples, round {round}:\n got: {got}\nwant: {want}");
            }
            assert_eq!(&view, expected, "shuffled triples, round {round}");
        }
        std::fs::remove_file(dir.path().join(&name)).unwrap();
    }
}
