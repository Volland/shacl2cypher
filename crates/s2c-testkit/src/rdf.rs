//! RDF projection of a fixture graph, consumed by SHACL reference engines.
//!
//! Nodes become IRIs `{base}node/{id}`, labels become `rdf:type {base}{label}`,
//! properties become literal triples (one per list element, nulls dropped) and
//! edges become IRI triples. Relationship properties have no RDF counterpart, so
//! fixtures that constrain them cannot use an RDF oracle.

use std::collections::BTreeSet;

use crate::fixture::{Fixture, Value};

const RDF_TYPE: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#type";
const XSD: &str = "http://www.w3.org/2001/XMLSchema#";

pub fn node_iri(base: &str, id: &str) -> String {
    format!("{base}node/{id}")
}

/// Inverse of [`node_iri`], used to map oracle focus nodes back to fixture ids.
pub fn focus_from_iri<'a>(base: &str, iri: &'a str) -> Option<&'a str> {
    iri.strip_prefix(base)?.strip_prefix("node/")
}

/// Serializes the graph as sorted N-Triples so projections are deterministic.
// @lat: [[testing#Conformance Fixtures]]
pub fn to_ntriples(fixture: &Fixture) -> String {
    let base = fixture.base.as_str();
    let mut triples = BTreeSet::new();
    for node in &fixture.graph.nodes {
        let subject = iri(&node_iri(base, &node.id));
        for label in &node.labels {
            triples.insert(format!(
                "{subject} {} {} .",
                iri(RDF_TYPE),
                iri(&format!("{base}{label}"))
            ));
        }
        for (key, value) in &node.props {
            let predicate = iri(&format!("{base}{key}"));
            for object in literals(value) {
                triples.insert(format!("{subject} {predicate} {object} ."));
            }
        }
    }
    for edge in &fixture.graph.edges {
        triples.insert(format!(
            "{} {} {} .",
            iri(&node_iri(base, &edge.from)),
            iri(&format!("{base}{}", edge.pred)),
            iri(&node_iri(base, &edge.to)),
        ));
    }
    triples.into_iter().map(|t| t + "\n").collect()
}

fn iri(value: &str) -> String {
    format!("<{value}>")
}

fn literals(value: &Value) -> Vec<String> {
    match value {
        Value::Null => vec![],
        Value::Bool(b) => vec![typed(&b.to_string(), "boolean")],
        Value::Int(i) => vec![typed(&i.to_string(), "integer")],
        Value::Float(x) => vec![typed(&double_lexical(*x), "double")],
        Value::Str(s) => vec![format!("\"{}\"", escape(s))],
        Value::Date { date } => vec![typed(date, "date")],
        Value::List(items) => items.iter().flat_map(literals).collect(),
    }
}

fn typed(lexical: &str, xsd_type: &str) -> String {
    format!("\"{}\"^^<{XSD}{xsd_type}>", escape(lexical))
}

fn double_lexical(x: f64) -> String {
    if x.is_nan() {
        "NaN".into()
    } else if x.is_infinite() {
        if x > 0.0 {
            "INF".into()
        } else {
            "-INF".into()
        }
    } else {
        format!("{x:?}")
    }
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projects_nodes_props_and_edges() {
        let fixture: Fixture = serde_yaml_ng::from_str(
            r#"
shapes: s.ttl
graph:
  nodes:
    - {id: p1, labels: [Person], props: {name: "A \"q\"", tags: [x, null], age: 3}}
    - {id: p2, labels: [Person]}
  edges:
    - {from: p1, to: p2, pred: knows}
"#,
        )
        .unwrap();
        let nt = to_ntriples(&fixture);
        let ex = "http://example.org/";
        assert!(nt.contains(&format!("<{ex}node/p1> <{RDF_TYPE}> <{ex}Person> .")));
        assert!(nt.contains(&format!("<{ex}node/p1> <{ex}name> \"A \\\"q\\\"\" .")));
        assert!(nt.contains(&format!("<{ex}node/p1> <{ex}tags> \"x\" .")));
        assert!(nt.contains(&format!("<{ex}node/p1> <{ex}age> \"3\"^^<{XSD}integer> .")));
        assert!(nt.contains(&format!("<{ex}node/p1> <{ex}knows> <{ex}node/p2> .")));
        assert_eq!(nt.lines().count(), 6);
    }

    #[test]
    fn maps_focus_iri_back_to_fixture_id() {
        assert_eq!(
            focus_from_iri("http://example.org/", "http://example.org/node/p7"),
            Some("p7")
        );
        assert_eq!(
            focus_from_iri("http://example.org/", "http://other.org/node/p7"),
            None
        );
    }
}
