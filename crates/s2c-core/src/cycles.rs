//! Detection of recursive shape references, which cannot be compiled by inlining
//! `conforms` expressions.

use std::collections::{HashMap, HashSet, VecDeque};

use oxrdf::NamedOrBlankNode;

use crate::ast::{Constraint, ShapeId, Shapes};
use crate::load::{ShapesGraph, SourceLocation};

/// One reference from a shape to another shape.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reference {
    pub from: ShapeId,
    /// SHACL parameter carrying the reference, e.g. `sh:node`.
    pub via: &'static str,
    pub to: ShapeId,
    pub location: SourceLocation,
}

/// A shortest reference cycle, starting from the smallest shape id of its group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceCycle {
    pub steps: Vec<Reference>,
}

impl ReferenceCycle {
    /// `ex:A -[sh:node at a.ttl:3]-> ex:B -[sh:not at a.ttl:4]-> ex:A`
    pub fn describe(&self, graph: &ShapesGraph) -> String {
        let name = |id: &ShapeId| match id {
            NamedOrBlankNode::NamedNode(iri) => graph.compact(iri.as_str()),
            NamedOrBlankNode::BlankNode(_) => "[blank node shape]".to_string(),
        };
        let mut out = name(&self.steps[0].from);
        for step in &self.steps {
            out.push_str(&format!(
                " -[{} at {}]-> {}",
                step.via,
                graph.display_location(step.location),
                name(&step.to)
            ));
        }
        out
    }
}

/// Finds every group of mutually recursive shapes and reports each group once.
// @lat: [[architecture#Shapes AST]]
pub fn recursive_references(shapes: &Shapes) -> Vec<ReferenceCycle> {
    let ids: Vec<&ShapeId> = shapes.iter().map(|shape| &shape.id).collect();
    let index: HashMap<&ShapeId, usize> = ids.iter().enumerate().map(|(i, id)| (*id, i)).collect();

    let edges: Vec<Vec<Reference>> = shapes
        .iter()
        .map(|shape| {
            let mut references = references_of(shape);
            references.retain(|r| index.contains_key(&r.to));
            references.sort_by(|a, b| {
                index[&a.to]
                    .cmp(&index[&b.to])
                    .then(a.location.cmp(&b.location))
            });
            references
        })
        .collect();
    let adjacency: Vec<Vec<usize>> = edges
        .iter()
        .map(|references| references.iter().map(|r| index[&r.to]).collect())
        .collect();

    let mut cycles: Vec<(usize, ReferenceCycle)> = strongly_connected_components(&adjacency)
        .into_iter()
        .filter(|component| component.len() > 1 || adjacency[component[0]].contains(&component[0]))
        .map(|component| {
            let start = *component.iter().min().expect("components are non-empty");
            let members: HashSet<usize> = component.into_iter().collect();
            let steps = shortest_cycle(start, &members, &edges, &index);
            (start, ReferenceCycle { steps })
        })
        .collect();
    cycles.sort_by_key(|(start, _)| *start);
    cycles.into_iter().map(|(_, cycle)| cycle).collect()
}

fn references_of(shape: &crate::ast::Shape) -> Vec<Reference> {
    let reference = |via: &'static str, to: &ShapeId, location: SourceLocation| Reference {
        from: shape.id.clone(),
        via,
        to: to.clone(),
        location,
    };
    let mut references: Vec<Reference> = shape
        .properties
        .iter()
        .map(|property| reference("sh:property", &property.value, property.location))
        .collect();
    for constraint in &shape.constraints {
        let location = constraint.location;
        match &constraint.value {
            Constraint::Node(to) => references.push(reference("sh:node", to, location)),
            Constraint::Not(to) => references.push(reference("sh:not", to, location)),
            Constraint::QualifiedValueShape { shape: to, .. } => {
                references.push(reference("sh:qualifiedValueShape", to, location))
            }
            Constraint::And(members) => {
                references.extend(members.iter().map(|to| reference("sh:and", to, location)))
            }
            Constraint::Or(members) => {
                references.extend(members.iter().map(|to| reference("sh:or", to, location)))
            }
            Constraint::Xone(members) => {
                references.extend(members.iter().map(|to| reference("sh:xone", to, location)))
            }
            _ => {}
        }
    }
    references
}

/// Tarjan's algorithm; components are returned in discovery order.
fn strongly_connected_components(adjacency: &[Vec<usize>]) -> Vec<Vec<usize>> {
    struct State<'a> {
        adjacency: &'a [Vec<usize>],
        order: Vec<Option<usize>>,
        low: Vec<usize>,
        on_stack: Vec<bool>,
        stack: Vec<usize>,
        next: usize,
        components: Vec<Vec<usize>>,
    }

    impl State<'_> {
        fn visit(&mut self, v: usize) {
            self.order[v] = Some(self.next);
            self.low[v] = self.next;
            self.next += 1;
            self.stack.push(v);
            self.on_stack[v] = true;
            let adjacency = self.adjacency;
            for &w in &adjacency[v] {
                match self.order[w] {
                    None => {
                        self.visit(w);
                        self.low[v] = self.low[v].min(self.low[w]);
                    }
                    Some(order) if self.on_stack[w] => self.low[v] = self.low[v].min(order),
                    Some(_) => {}
                }
            }
            if Some(self.low[v]) == self.order[v] {
                let mut component = Vec::new();
                loop {
                    let w = self.stack.pop().expect("v is on the stack");
                    self.on_stack[w] = false;
                    component.push(w);
                    if w == v {
                        break;
                    }
                }
                self.components.push(component);
            }
        }
    }

    let n = adjacency.len();
    let mut state = State {
        adjacency,
        order: vec![None; n],
        low: vec![0; n],
        on_stack: vec![false; n],
        stack: Vec::new(),
        next: 0,
        components: Vec::new(),
    };
    for v in 0..n {
        if state.order[v].is_none() {
            state.visit(v);
        }
    }
    state.components
}

/// Breadth-first search for the shortest cycle through `start` inside its component.
fn shortest_cycle(
    start: usize,
    members: &HashSet<usize>,
    edges: &[Vec<Reference>],
    index: &HashMap<&ShapeId, usize>,
) -> Vec<Reference> {
    let mut previous: HashMap<usize, (usize, usize)> = HashMap::new();
    let mut visited = HashSet::from([start]);
    let mut queue = VecDeque::from([start]);
    while let Some(v) = queue.pop_front() {
        for (k, edge) in edges[v].iter().enumerate() {
            let w = index[&edge.to];
            if !members.contains(&w) {
                continue;
            }
            if w == start {
                let mut steps = vec![edge.clone()];
                let mut node = v;
                while node != start {
                    let (from, edge_index) = previous[&node];
                    steps.push(edges[from][edge_index].clone());
                    node = from;
                }
                steps.reverse();
                return steps;
            }
            if visited.insert(w) {
                previous.insert(w, (v, k));
                queue.push_back(w);
            }
        }
    }
    unreachable!("a recursive component has a cycle through each of its members")
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::NamedNode;

    const PREFIXES: &str =
        "@prefix sh: <http://www.w3.org/ns/shacl#> .\n@prefix ex: <http://example.org/> .\n";

    fn parse(body: &str) -> (ShapesGraph, Shapes) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shapes.ttl");
        std::fs::write(&path, format!("{PREFIXES}{body}")).unwrap();
        let graph = ShapesGraph::load(&[path]).unwrap();
        let shapes = Shapes::from_graph(&graph).unwrap();
        (graph, shapes)
    }

    fn shape_id(local: &str) -> ShapeId {
        NamedNode::new_unchecked(format!("http://example.org/{local}")).into()
    }

    fn vias(cycle: &ReferenceCycle) -> Vec<&'static str> {
        cycle.steps.iter().map(|step| step.via).collect()
    }

    #[test]
    fn detects_recursion_through_a_property_shape() {
        let (graph, shapes) = parse(
            "ex:PersonShape a sh:NodeShape ;
    sh:property [ sh:path ex:knows ; sh:node ex:PersonShape ] .
",
        );
        let cycles = recursive_references(&shapes);
        assert_eq!(cycles.len(), 1);
        assert_eq!(vias(&cycles[0]), vec!["sh:property", "sh:node"]);
        assert_eq!(cycles[0].steps[0].from, shape_id("PersonShape"));
        let description = cycles[0].describe(&graph);
        assert!(
            description.starts_with("ex:PersonShape -[sh:property at "),
            "{description}"
        );
        assert!(description.contains("shapes.ttl:4]->"), "{description}");
        assert!(description.ends_with("-> ex:PersonShape"), "{description}");
    }

    #[test]
    fn detects_self_reference() {
        let (_, shapes) = parse("ex:A sh:node ex:A .\n");
        let cycles = recursive_references(&shapes);
        assert_eq!(cycles.len(), 1);
        assert_eq!(vias(&cycles[0]), vec!["sh:node"]);
    }

    #[test]
    fn detects_indirect_recursion_through_logical_and_qualified_constraints() {
        let (_, shapes) = parse(
            "ex:A sh:and ( ex:B ) .
ex:B sh:path ex:p ;
    sh:qualifiedValueShape ex:C ;
    sh:qualifiedMinCount 1 .
ex:C sh:not ex:A .
",
        );
        let cycles = recursive_references(&shapes);
        assert_eq!(cycles.len(), 1);
        assert_eq!(
            vias(&cycles[0]),
            vec!["sh:and", "sh:qualifiedValueShape", "sh:not"]
        );
        let visited: Vec<ShapeId> = cycles[0].steps.iter().map(|s| s.to.clone()).collect();
        assert_eq!(visited, vec![shape_id("B"), shape_id("C"), shape_id("A")]);
    }

    #[test]
    fn shared_references_are_not_recursion() {
        let (_, shapes) = parse(
            "ex:A sh:node ex:B, ex:C .
ex:B sh:node ex:D .
ex:C sh:node ex:D .
ex:D sh:minCount 1 .
",
        );
        assert!(recursive_references(&shapes).is_empty());
    }

    #[test]
    fn reports_each_recursive_group_once_in_id_order() {
        let (_, shapes) = parse(
            "ex:X sh:node ex:Y .
ex:Y sh:node ex:X .
ex:A sh:node ex:B .
ex:B sh:node ex:A ;
    sh:or ( ex:A ) .
",
        );
        let cycles = recursive_references(&shapes);
        assert_eq!(cycles.len(), 2);
        assert_eq!(cycles[0].steps[0].from, shape_id("A"));
        assert_eq!(cycles[1].steps[0].from, shape_id("X"));
        assert_eq!(cycles[0].steps.len(), 2);
    }
}
