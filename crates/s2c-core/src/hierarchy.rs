//! `rdfs:subClassOf` hierarchy used to expand class targets and `sh:class` checks.

use std::collections::{BTreeMap, BTreeSet};

use oxrdf::vocab::rdfs;
use oxrdf::{NamedNode, NamedNodeRef, NamedOrBlankNodeRef, TermRef};

use crate::load::ShapesGraph;

/// How Neo4j labels relate to the class hierarchy (`--neo4j-labels`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LabelPolicy {
    /// Nodes carry only their own class label, so targets expand to all subclasses.
    #[default]
    Explicit,
    /// Nodes already carry every superclass label, so no expansion is needed.
    Inherited,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct HierarchyError(pub String);

#[derive(Debug, Default)]
pub struct ClassHierarchy {
    /// Class IRI -> direct subclass IRIs.
    subclasses: BTreeMap<String, BTreeSet<String>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Visit {
    InProgress,
    Done,
}

impl ClassHierarchy {
    /// Collects `rdfs:subClassOf` statements between IRIs from the shapes graph and
    /// any ontology graphs, rejecting cycles.
    // @lat: [[mapping#Class Hierarchy]]
    pub fn build(graphs: &[&ShapesGraph]) -> Result<Self, HierarchyError> {
        let mut subclasses: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut statements: BTreeMap<(String, String), String> = BTreeMap::new();
        for graph in graphs {
            for triple in graph.graph.triples_for_predicate(rdfs::SUB_CLASS_OF) {
                let (NamedOrBlankNodeRef::NamedNode(sub), TermRef::NamedNode(sup)) =
                    (triple.subject, triple.object)
                else {
                    continue;
                };
                if sub == sup {
                    continue;
                }
                let location = graph
                    .locations(&triple.into_owned())
                    .iter()
                    .min()
                    .map(|location| graph.display_location(*location))
                    .unwrap_or_default();
                subclasses
                    .entry(sup.as_str().to_owned())
                    .or_default()
                    .insert(sub.as_str().to_owned());
                statements
                    .entry((sub.as_str().to_owned(), sup.as_str().to_owned()))
                    .or_insert(location);
            }
        }

        let hierarchy = ClassHierarchy { subclasses };
        if let Some(cycle) = hierarchy.find_cycle() {
            let steps: Vec<String> = cycle
                .windows(2)
                .map(|pair| {
                    let (sup, sub) = (&pair[0], &pair[1]);
                    let at = &statements[&(sub.clone(), sup.clone())];
                    format!("<{sub}> rdfs:subClassOf <{sup}> at {at}")
                })
                .collect();
            return Err(HierarchyError(format!(
                "rdfs:subClassOf cycle: {}",
                steps.join(", ")
            )));
        }
        Ok(hierarchy)
    }

    /// The class and all of its transitive subclasses, sorted by IRI.
    pub fn with_subclasses(&self, class: NamedNodeRef<'_>) -> Vec<NamedNode> {
        let mut seen = BTreeSet::new();
        let mut pending = vec![class.as_str().to_owned()];
        while let Some(current) = pending.pop() {
            if let Some(subclasses) = self.subclasses.get(&current) {
                pending.extend(subclasses.iter().filter(|s| !seen.contains(*s)).cloned());
            }
            seen.insert(current);
        }
        seen.into_iter().map(NamedNode::new_unchecked).collect()
    }

    /// Classes whose instances a target or `sh:class` check must match under a policy.
    pub fn expand(&self, class: NamedNodeRef<'_>, policy: LabelPolicy) -> Vec<NamedNode> {
        match policy {
            LabelPolicy::Explicit => self.with_subclasses(class),
            LabelPolicy::Inherited => vec![class.into_owned()],
        }
    }

    /// A superclass-to-subclass path that returns to its start, if any.
    fn find_cycle(&self) -> Option<Vec<String>> {
        let mut state: BTreeMap<&str, Visit> = BTreeMap::new();
        let mut stack: Vec<&str> = Vec::new();
        for class in self.subclasses.keys() {
            if !state.contains_key(class.as_str()) {
                if let Some(cycle) = self.visit(class, &mut state, &mut stack) {
                    return Some(cycle);
                }
            }
        }
        None
    }

    fn visit<'s>(
        &'s self,
        class: &'s str,
        state: &mut BTreeMap<&'s str, Visit>,
        stack: &mut Vec<&'s str>,
    ) -> Option<Vec<String>> {
        state.insert(class, Visit::InProgress);
        stack.push(class);
        for subclass in self.subclasses.get(class).into_iter().flatten() {
            match state.get(subclass.as_str()) {
                Some(Visit::InProgress) => {
                    let start = stack
                        .iter()
                        .position(|c| *c == subclass.as_str())
                        .expect("in-progress classes are on the stack");
                    let mut cycle: Vec<String> =
                        stack[start..].iter().map(|c| (*c).to_owned()).collect();
                    cycle.push(subclass.clone());
                    return Some(cycle);
                }
                Some(Visit::Done) => {}
                None => {
                    if let Some(cycle) = self.visit(subclass, state, stack) {
                        return Some(cycle);
                    }
                }
            }
        }
        stack.pop();
        state.insert(class, Visit::Done);
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two prefix lines, so the first body line is line 3.
    const PREFIXES: &str = "@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix ex: <http://example.org/> .
";

    fn load(name: &str, body: &str) -> ShapesGraph {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(name);
        std::fs::write(&path, format!("{PREFIXES}{body}")).unwrap();
        ShapesGraph::load(&[path]).unwrap()
    }

    fn ex(local: &str) -> NamedNode {
        NamedNode::new_unchecked(format!("http://example.org/{local}"))
    }

    fn locals(classes: &[NamedNode]) -> Vec<&str> {
        classes
            .iter()
            .map(|c| c.as_str().trim_start_matches("http://example.org/"))
            .collect()
    }

    // @lat: [[tests#Mapping#Subclass Expansion]]
    #[test]
    fn expands_transitive_subclasses_in_iri_order() {
        let shapes = load(
            "shapes.ttl",
            "ex:Employee rdfs:subClassOf ex:Person .
ex:Manager rdfs:subClassOf ex:Employee .
ex:Intern rdfs:subClassOf ex:Employee .
",
        );
        let hierarchy = ClassHierarchy::build(&[&shapes]).unwrap();
        assert_eq!(
            locals(&hierarchy.with_subclasses(ex("Person").as_ref())),
            vec!["Employee", "Intern", "Manager", "Person"]
        );
        assert_eq!(
            locals(&hierarchy.with_subclasses(ex("Unrelated").as_ref())),
            vec!["Unrelated"]
        );
    }

    #[test]
    fn ontology_graphs_contribute_and_policy_controls_expansion() {
        let shapes = load("shapes.ttl", "ex:S a ex:Thing .\n");
        let ontology = load(
            "ontology.ttl",
            "ex:Contractor rdfs:subClassOf ex:Person .\n",
        );
        let hierarchy = ClassHierarchy::build(&[&shapes, &ontology]).unwrap();
        let person = ex("Person");
        assert_eq!(
            locals(&hierarchy.expand(person.as_ref(), LabelPolicy::Explicit)),
            vec!["Contractor", "Person"]
        );
        assert_eq!(
            locals(&hierarchy.expand(person.as_ref(), LabelPolicy::Inherited)),
            vec!["Person"]
        );
    }

    #[test]
    fn ignores_reflexive_statements_and_deduplicates_diamonds() {
        let shapes = load(
            "shapes.ttl",
            "ex:A rdfs:subClassOf ex:A .
ex:B rdfs:subClassOf ex:A .
ex:C rdfs:subClassOf ex:A .
ex:D rdfs:subClassOf ex:B, ex:C .
",
        );
        let hierarchy = ClassHierarchy::build(&[&shapes]).unwrap();
        assert_eq!(
            locals(&hierarchy.with_subclasses(ex("A").as_ref())),
            vec!["A", "B", "C", "D"]
        );
    }

    #[test]
    fn rejects_cycles_with_every_statement_location() {
        let shapes = load(
            "a.ttl",
            "ex:A rdfs:subClassOf ex:B .
ex:B rdfs:subClassOf ex:A .
",
        );
        let message = ClassHierarchy::build(&[&shapes]).unwrap_err().to_string();
        assert!(message.starts_with("rdfs:subClassOf cycle:"), "{message}");
        assert!(message.contains("a.ttl:3"), "{message}");
        assert!(message.contains("a.ttl:4"), "{message}");
    }
}
