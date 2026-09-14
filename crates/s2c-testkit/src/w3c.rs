//! W3C SHACL Core test suite support.
//!
//! A Core test file holds a manifest entry plus its shapes and data, either inline or
//! in sibling `-shapes.ttl`/`-data.ttl` files. [`load`] separates the manifest from
//! the shapes-and-data graph and reads the expected results; [`neo4j_script`]
//! projects the graph to an LPG: `rdf:type` becomes labels, literals become
//! properties and IRI objects become relationships, and each node's IRI local name
//! is its `id`.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};

use oxrdf::vocab::rdf;
use oxrdf::{Literal, NamedOrBlankNode, Term, Triple};
use oxttl::TurtleParser;

use crate::fixture::{upper_snake, ID_PROPERTY};
use crate::lpg::{ident, quote};

const MF: &str = "http://www.w3.org/2001/sw/DataAccess/tests/test-manifest#";
const SHT: &str = "http://www.w3.org/ns/shacl-test#";
const SH: &str = "http://www.w3.org/ns/shacl#";
const XSD: &str = "http://www.w3.org/2001/XMLSchema#";

/// Skip reason of `-shapes.ttl`/`-data.ttl` files that only serve another test.
pub const COMPANION: &str = "companion file without a test entry";

/// The vendored copy of `data-shapes-test-suite/tests/core`.
pub fn suite_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/w3c/core")
}

/// Test files under `root`, sorted; manifests that only include other files are skipped.
pub fn suite_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|e| e == "ttl")
                && path.file_name().is_some_and(|n| n != "manifest.ttl")
            {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

#[derive(Debug)]
pub struct W3cTest {
    /// Path relative to the suite root, e.g. `node/and-001.ttl`.
    pub name: String,
    /// Shapes and data triples, without the manifest.
    pub graph: Vec<Triple>,
    /// Expected `(focus IRI, constraint)` pairs such as `(".../test#Alice", "sh:minCount")`.
    pub expected: BTreeSet<(String, String)>,
    /// Why the test cannot run on an LPG at all.
    pub skip: Option<String>,
}

/// Indexes triples by subject for manifest walking.
struct Index<'t> {
    by_subject: BTreeMap<String, Vec<&'t Triple>>,
}

impl<'t> Index<'t> {
    fn new(triples: &'t [Triple]) -> Self {
        let mut by_subject: BTreeMap<String, Vec<&Triple>> = BTreeMap::new();
        for triple in triples {
            by_subject
                .entry(triple.subject.to_string())
                .or_default()
                .push(triple);
        }
        Index { by_subject }
    }

    fn values(&self, subject: &str, predicate: &str) -> Vec<&'t Term> {
        self.by_subject
            .get(subject)
            .into_iter()
            .flatten()
            .filter(|t| t.predicate.as_str() == predicate)
            .map(|t| &t.object)
            .collect()
    }

    fn value(&self, subject: &str, predicate: &str) -> Option<&'t Term> {
        self.values(subject, predicate).into_iter().next()
    }

    fn list(&self, head: &Term, visited: &mut BTreeSet<String>) -> Vec<&'t Term> {
        let mut items = Vec::new();
        let mut node = head.to_string();
        while node != format!("<{}>", rdf::NIL.as_str()) && visited.insert(node.clone()) {
            if let Some(first) = self.value(&node, rdf::FIRST.as_str()) {
                items.push(first);
            }
            match self.value(&node, rdf::REST.as_str()) {
                Some(rest) => node = rest.to_string(),
                None => break,
            }
        }
        items
    }
}

/// Parses a Turtle file with its `file://` IRI as base.
fn parse(file: &Path, name: &str) -> Result<(String, Vec<Triple>), String> {
    let bytes = std::fs::read(file).map_err(|e| format!("{name}: {e}"))?;
    let absolute = file.canonicalize().map_err(|e| format!("{name}: {e}"))?;
    let base = format!("file://{}", absolute.display());
    let triples = TurtleParser::new()
        .with_base_iri(&base)
        .map_err(|e| format!("{name}: {e}"))?
        .for_slice(&bytes)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("{name}: {e}"))?;
    Ok((base, triples))
}

/// Loads one test file.
// @lat: [[testing#W3C Test Suite]]
pub fn load(root: &Path, file: &Path) -> Result<W3cTest, String> {
    let name = file
        .strip_prefix(root)
        .unwrap_or(file)
        .to_string_lossy()
        .replace('\\', "/");
    let (base, triples) = parse(file, &name)?;
    let index = Index::new(&triples);

    let mut removed: BTreeSet<String> = BTreeSet::new();
    let mut entries = Vec::new();
    for triple in &triples {
        if triple.predicate == rdf::TYPE
            && matches!(&triple.object, Term::NamedNode(t) if t.as_str() == format!("{MF}Manifest"))
        {
            let manifest = triple.subject.to_string();
            removed.insert(manifest.clone());
            for head in index.values(&manifest, &format!("{MF}entries")) {
                entries.extend(index.list(head, &mut removed));
            }
        }
    }
    // Entries and everything hanging off them through blank nodes belong to the manifest,
    // except references back into the tested graph (a result's shape, focus or value).
    let references = ["sourceShape", "sourceConstraint", "focusNode", "value"]
        .map(|local| format!("{SH}{local}"));
    let mut stack: Vec<String> = entries.iter().map(|e| e.to_string()).collect();
    while let Some(subject) = stack.pop() {
        if !removed.insert(subject.clone()) {
            continue;
        }
        for triple in index.by_subject.get(&subject).into_iter().flatten() {
            if matches!(triple.object, Term::BlankNode(_))
                && !references.iter().any(|r| r == triple.predicate.as_str())
            {
                stack.push(triple.object.to_string());
            }
        }
    }
    let mut graph: Vec<Triple> = triples
        .iter()
        .filter(|t| !removed.contains(&t.subject.to_string()))
        .cloned()
        .collect();

    let mut expected = BTreeSet::new();
    let mut referenced: Vec<Triple> = Vec::new();
    let mut skip = (|| {
        let [entry] = entries.as_slice() else {
            return Some(if entries.is_empty() {
                COMPANION.to_owned()
            } else {
                format!("has {} test entries", entries.len())
            });
        };
        let entry = entry.to_string();
        let validate = format!("<{SHT}Validate>");
        if !index
            .values(&entry, rdf::TYPE.as_str())
            .iter()
            .any(|t| t.to_string() == validate)
        {
            return Some("not a validation test".into());
        }
        let action = index.value(&entry, &format!("{MF}action"))?.to_string();
        let mut loaded = HashSet::new();
        for graph in ["dataGraph", "shapesGraph"] {
            match index.value(&action, &format!("{SHT}{graph}")) {
                Some(Term::NamedNode(iri)) if iri.as_str() == base => {}
                Some(Term::NamedNode(iri)) => {
                    let Some(path) = iri.as_str().strip_prefix("file://") else {
                        return Some(format!("its {graph} is not a local file"));
                    };
                    if loaded.insert(path.to_owned()) {
                        match parse(Path::new(path), &name) {
                            Ok((_, triples)) => referenced.extend(triples),
                            Err(error) => return Some(format!("cannot load its {graph}: {error}")),
                        }
                    }
                }
                _ => return Some(format!("has no {graph}")),
            }
        }
        let Some(report) = index.value(&entry, &format!("{MF}result")) else {
            return Some("has no expected result".into());
        };
        if !matches!(report, Term::BlankNode(_)) {
            return Some(format!("expects {report} instead of a report"));
        }
        for result in index.values(&report.to_string(), &format!("{SH}result")) {
            let result = result.to_string();
            match index.value(&result, &format!("{SH}focusNode")) {
                Some(Term::NamedNode(focus)) => {
                    let component = index
                        .value(&result, &format!("{SH}sourceConstraintComponent"))
                        .map(component_name)
                        .unwrap_or_default();
                    expected.insert((focus.as_str().to_owned(), component));
                }
                Some(Term::Literal(_)) => return Some("expects a literal focus node".into()),
                _ => return Some("expects a blank-node focus".into()),
            }
        }
        None
    })();
    let mut seen = HashSet::new();
    graph.extend(referenced);
    graph.retain(|triple| seen.insert(triple.clone()));
    if skip.is_none() {
        skip = static_reason(&graph);
    }

    Ok(W3cTest {
        name,
        graph,
        expected,
        skip,
    })
}

/// Features with no LPG meaning or excluded by design.
fn static_reason(graph: &[Triple]) -> Option<String> {
    let uses = |local: &str| {
        let iri = format!("{SH}{local}");
        graph.iter().any(|t| t.predicate.as_str() == iri)
    };
    if uses("targetNode") {
        return Some("sh:targetNode is rejected by design".into());
    }
    if uses("sparql") || uses("select") || uses("ask") {
        return Some("SHACL-SPARQL is out of scope".into());
    }
    if uses("languageIn") || uses("uniqueLang") {
        return Some("language tags have no LPG form".into());
    }
    if graph
        .iter()
        .any(|t| matches!(&t.object, Term::Literal(l) if l.language().is_some()))
    {
        return Some("language-tagged literals have no LPG form".into());
    }
    None
}

/// `sh:MinCountConstraintComponent` -> `sh:minCount`.
fn component_name(term: &Term) -> String {
    let text = match term {
        Term::NamedNode(iri) => iri.as_str(),
        _ => return term.to_string(),
    };
    let local = text
        .strip_prefix(SH)
        .and_then(|l| l.strip_suffix("ConstraintComponent"))
        .unwrap_or(text);
    let mut chars = local.chars();
    match chars.next() {
        Some(first) => format!("sh:{}{}", first.to_lowercase(), chars.as_str()),
        None => text.to_owned(),
    }
}

/// The shapes-and-data graph as N-Triples, for the compiler.
pub fn ntriples(test: &W3cTest) -> String {
    test.graph.iter().map(|t| format!("{t} .\n")).collect()
}

/// The part of an IRI after its last `#`, `/` or `:`; the projected node id.
pub fn local_name(iri: &str) -> &str {
    iri.rsplit(['#', '/', ':']).next().unwrap_or(iri)
}

fn node_key(node: &NamedOrBlankNode) -> String {
    match node {
        NamedOrBlankNode::NamedNode(iri) => iri.as_str().to_owned(),
        NamedOrBlankNode::BlankNode(b) => format!("_:{}", b.as_str()),
    }
}

#[derive(Debug, Clone, PartialEq)]
enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    Date(String),
}

/// Typed literals become native values when well-formed; everything else stays a string.
fn value(literal: &Literal) -> Value {
    let lexical = literal.value();
    let text = || Value::Str(lexical.to_owned());
    match literal.datatype().as_str().strip_prefix(XSD) {
        Some(
            "integer" | "int" | "long" | "short" | "byte" | "nonNegativeInteger"
            | "positiveInteger" | "negativeInteger" | "nonPositiveInteger" | "unsignedInt"
            | "unsignedLong" | "unsignedShort" | "unsignedByte",
        ) => lexical.parse().map_or_else(|_| text(), Value::Int),
        Some("decimal" | "double" | "float") => lexical
            .parse::<f64>()
            .ok()
            .filter(|x| x.is_finite())
            .map_or_else(text, Value::Float),
        Some("boolean") => match lexical {
            "true" | "1" => Value::Bool(true),
            "false" | "0" => Value::Bool(false),
            _ => text(),
        },
        Some("date")
            if lexical.len() == 10
                && lexical.char_indices().all(|(i, c)| {
                    if i == 4 || i == 7 {
                        c == '-'
                    } else {
                        c.is_ascii_digit()
                    }
                }) =>
        {
            Value::Date(lexical.to_owned())
        }
        _ => text(),
    }
}

fn literal(value: &Value) -> String {
    match value {
        Value::Int(i) => i.to_string(),
        Value::Float(x) => {
            let s = format!("{x:?}");
            if s.contains(['.', 'e', 'E']) {
                s
            } else {
                format!("{s}.0")
            }
        }
        Value::Bool(b) => b.to_string(),
        Value::Str(s) => quote(s),
        Value::Date(d) => format!("date({})", quote(d)),
    }
}

/// Neo4j load script for the test graph; `Err` explains why the graph has no faithful LPG form.
pub fn neo4j_script(test: &W3cTest) -> Result<Vec<String>, String> {
    let mut labels: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut props: BTreeMap<String, BTreeMap<String, Vec<Value>>> = BTreeMap::new();
    let mut edges: BTreeSet<(String, String, String)> = BTreeSet::new();
    let mut literal_predicates = BTreeSet::new();
    let mut node_predicates = BTreeSet::new();
    for triple in &test.graph {
        let subject = node_key(&triple.subject);
        labels.entry(subject.clone()).or_default();
        let predicate = triple.predicate.as_str();
        if triple.predicate == rdf::TYPE {
            if let Term::NamedNode(class) = &triple.object {
                labels
                    .entry(subject)
                    .or_default()
                    .insert(local_name(class.as_str()).to_owned());
                continue;
            }
        }
        let key = local_name(predicate);
        if key.is_empty() {
            return Err(format!("<{predicate}> has no local name"));
        }
        let object = match &triple.object {
            Term::Literal(l) => {
                if key == ID_PROPERTY {
                    return Err("the data uses the reserved `id` key".into());
                }
                literal_predicates.insert(predicate.to_owned());
                props
                    .entry(subject)
                    .or_default()
                    .entry(key.to_owned())
                    .or_default()
                    .push(value(l));
                continue;
            }
            Term::NamedNode(iri) => iri.as_str().to_owned(),
            Term::BlankNode(b) => format!("_:{}", b.as_str()),
            #[allow(unreachable_patterns)]
            _ => return Err("RDF-star triples".into()),
        };
        node_predicates.insert(predicate.to_owned());
        labels.entry(object.clone()).or_default();
        edges.insert((subject, upper_snake(key), object));
    }
    if let Some(predicate) = literal_predicates.intersection(&node_predicates).next() {
        return Err(format!("<{predicate}> has both literal and node objects"));
    }

    // Node ids are IRI local names, which is how the compiler matches IRI constants.
    let mut ids: HashMap<&str, String> = HashMap::new();
    let mut owners: HashMap<String, &str> = HashMap::new();
    for node in labels.keys() {
        let id = if node.starts_with("_:") || local_name(node).is_empty() {
            node.clone()
        } else {
            local_name(node).to_owned()
        };
        if let Some(other) = owners.insert(id.clone(), node) {
            return Err(format!(
                "<{other}> and <{node}> share the local name `{id}`"
            ));
        }
        ids.insert(node, id);
    }

    let mut statements = Vec::new();
    for (node, node_labels) in &labels {
        let label_text: String = node_labels
            .iter()
            .map(|l| format!(":{}", ident(l)))
            .collect();
        let mut entries = vec![format!(
            "{}: {}",
            ident(ID_PROPERTY),
            quote(&ids[node.as_str()])
        )];
        for (key, values) in props.get(node).into_iter().flatten() {
            let rendered = match values.as_slice() {
                [single] => literal(single),
                many => {
                    if many
                        .iter()
                        .any(|v| std::mem::discriminant(v) != std::mem::discriminant(&many[0]))
                    {
                        return Err(format!("<{node}> has mixed-type values for `{key}`"));
                    }
                    let items: Vec<String> = many.iter().map(literal).collect();
                    format!("[{}]", items.join(", "))
                }
            };
            entries.push(format!("{}: {rendered}", ident(key)));
        }
        statements.push(format!("CREATE ({label_text} {{{}}})", entries.join(", ")));
    }
    let id = ident(ID_PROPERTY);
    for (from, rel_type, to) in &edges {
        statements.push(format!(
            "MATCH (a {{{id}: {}}}), (b {{{id}: {}}}) CREATE (a)-[:{}]->(b)",
            quote(&ids[from.as_str()]),
            quote(&ids[to.as_str()]),
            ident(rel_type)
        ));
    }
    Ok(statements)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test(name: &str) -> W3cTest {
        let root = suite_root();
        load(&root, &root.join(name)).unwrap()
    }

    #[test]
    fn separates_manifest_and_reads_expected_results() {
        let t = test("property/minCount-001.ttl");
        assert_eq!(
            t.skip.as_deref(),
            Some("sh:targetNode is rejected by design")
        );
        assert_eq!(t.expected.len(), 1);
        let (focus, component) = t.expected.iter().next().unwrap();
        assert!(focus.ends_with("#InvalidPerson"), "{focus}");
        assert_eq!(component, "sh:minCount");
        assert!(t
            .graph
            .iter()
            .all(|t| !t.predicate.as_str().starts_with(MF)));
        assert!(t
            .graph
            .iter()
            .all(|t| t.predicate.as_str() != format!("{SH}resultSeverity")));

        let script = neo4j_script(&t).unwrap();
        assert!(script
            .iter()
            .any(|s| s.starts_with("CREATE (:`Person` {`id`: 'InvalidPerson'")));
        assert!(ntriples(&t).contains("<http://www.w3.org/ns/shacl#minCount>"));
    }

    #[test]
    fn loads_separate_shapes_and_data_files() {
        let t = test("node/xone-duplicate.ttl");
        assert_eq!(t.skip, None);
        assert_eq!(t.expected.len(), 2);
        assert!(t
            .graph
            .iter()
            .any(|t| t.predicate.as_str() == format!("{SH}xone")));
        assert_eq!(
            test("node/xone-duplicate-data.ttl").skip.as_deref(),
            Some(COMPANION)
        );
    }

    #[test]
    fn records_static_skip_reasons() {
        assert_eq!(
            test("node/minInclusive-001.ttl").skip.as_deref(),
            Some("expects a literal focus node")
        );
        assert_eq!(
            component_name(&Term::NamedNode(oxrdf::NamedNode::new_unchecked(
                "http://www.w3.org/ns/shacl#QualifiedMinCountConstraintComponent"
            ))),
            "sh:qualifiedMinCount"
        );
    }
}
