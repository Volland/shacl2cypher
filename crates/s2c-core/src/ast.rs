//! SHACL shapes AST built from the union shapes graph.

use std::collections::{HashMap, HashSet};

use oxrdf::vocab::{rdf, rdfs, xsd};
use oxrdf::{
    BlankNode, Graph, Literal, NamedNode, NamedNodeRef, NamedOrBlankNode, NamedOrBlankNodeRef,
    Term, TermRef, TripleRef,
};

use crate::load::{ShapesGraph, SourceLocation};

mod annotations;

pub use annotations::{s2c, Annotations, Collection, Direction, IriAsString, NAMESPACE as S2C};

/// SHACL vocabulary terms used by the compiler.
pub mod sh {
    use oxrdf::NamedNodeRef;

    macro_rules! terms {
        ($($name:ident = $local:literal),* $(,)?) => {
            $(pub const $name: NamedNodeRef<'static> =
                NamedNodeRef::new_unchecked(concat!("http://www.w3.org/ns/shacl#", $local));)*
        };
    }

    terms! {
        NODE_SHAPE = "NodeShape", PROPERTY_SHAPE = "PropertyShape", PATH = "path",
        PROPERTY = "property", TARGET_CLASS = "targetClass", TARGET_NODE = "targetNode",
        TARGET_SUBJECTS_OF = "targetSubjectsOf", TARGET_OBJECTS_OF = "targetObjectsOf",
        SEVERITY = "severity", VIOLATION = "Violation", WARNING = "Warning", INFO = "Info",
        MESSAGE = "message", DEACTIVATED = "deactivated",
        MIN_COUNT = "minCount", MAX_COUNT = "maxCount", DATATYPE = "datatype",
        NODE_KIND = "nodeKind", CLASS = "class",
        MIN_INCLUSIVE = "minInclusive", MAX_INCLUSIVE = "maxInclusive",
        MIN_EXCLUSIVE = "minExclusive", MAX_EXCLUSIVE = "maxExclusive",
        MIN_LENGTH = "minLength", MAX_LENGTH = "maxLength", PATTERN = "pattern", FLAGS = "flags",
        IN = "in", HAS_VALUE = "hasValue", EQUALS = "equals", DISJOINT = "disjoint",
        LESS_THAN = "lessThan", LESS_THAN_OR_EQUALS = "lessThanOrEquals",
        CLOSED = "closed", IGNORED_PROPERTIES = "ignoredProperties",
        NODE = "node", NOT = "not", AND = "and", OR = "or", XONE = "xone",
        QUALIFIED_VALUE_SHAPE = "qualifiedValueShape", QUALIFIED_MIN_COUNT = "qualifiedMinCount",
        QUALIFIED_MAX_COUNT = "qualifiedMaxCount",
        QUALIFIED_VALUE_SHAPES_DISJOINT = "qualifiedValueShapesDisjoint",
        LANGUAGE_IN = "languageIn", UNIQUE_LANG = "uniqueLang", SPARQL = "sparql",
        INVERSE_PATH = "inversePath", ALTERNATIVE_PATH = "alternativePath",
        ZERO_OR_MORE_PATH = "zeroOrMorePath", ONE_OR_MORE_PATH = "oneOrMorePath",
        ZERO_OR_ONE_PATH = "zeroOrOnePath",
        IRI = "IRI", BLANK_NODE = "BlankNode", LITERAL = "Literal",
        BLANK_NODE_OR_IRI = "BlankNodeOrIRI", BLANK_NODE_OR_LITERAL = "BlankNodeOrLiteral",
        IRI_OR_LITERAL = "IRIOrLiteral",
    }
}

/// Predicates whose subjects are shapes.
const SHAPE_SUBJECT_PARAMETERS: &[NamedNodeRef<'static>] = &[
    sh::PATH,
    sh::PROPERTY,
    sh::TARGET_CLASS,
    sh::TARGET_NODE,
    sh::TARGET_SUBJECTS_OF,
    sh::TARGET_OBJECTS_OF,
    sh::MIN_COUNT,
    sh::MAX_COUNT,
    sh::DATATYPE,
    sh::NODE_KIND,
    sh::CLASS,
    sh::MIN_INCLUSIVE,
    sh::MAX_INCLUSIVE,
    sh::MIN_EXCLUSIVE,
    sh::MAX_EXCLUSIVE,
    sh::MIN_LENGTH,
    sh::MAX_LENGTH,
    sh::PATTERN,
    sh::IN,
    sh::HAS_VALUE,
    sh::EQUALS,
    sh::DISJOINT,
    sh::LESS_THAN,
    sh::LESS_THAN_OR_EQUALS,
    sh::CLOSED,
    sh::NODE,
    sh::NOT,
    sh::AND,
    sh::OR,
    sh::XONE,
    sh::QUALIFIED_VALUE_SHAPE,
    sh::LANGUAGE_IN,
    sh::UNIQUE_LANG,
    sh::SPARQL,
    s2c::TARGET_RELATIONSHIP,
];

/// A shape's identity in the shapes graph: its IRI or file-scoped blank node.
pub type ShapeId = NamedOrBlankNode;

/// A value together with the location of the triple it was read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Located<T> {
    pub value: T,
    pub location: SourceLocation,
}

impl<T> Located<T> {
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Located<U> {
        Located {
            value: f(self.value),
            location: self.location,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Severity {
    Info,
    Warning,
    Violation,
    Other(NamedNode),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Class(NamedNode),
    /// The shape IRI is also an `rdfs:Class`.
    ImplicitClass(NamedNode),
    SubjectsOf(NamedNode),
    ObjectsOf(NamedNode),
    Node(Term),
    /// `s2c:targetRelationship`: every relationship of this type is a focus.
    // @lat: [[mapping#Relationship Targets]]
    Relationship(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
// @lat: [[semantics#Supported Features#Tier 2]]
pub enum Path {
    Predicate(NamedNode),
    Inverse(Box<Path>),
    Sequence(Vec<Path>),
    Alternative(Vec<Path>),
    ZeroOrMore(Box<Path>),
    OneOrMore(Box<Path>),
    ZeroOrOne(Box<Path>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    Iri,
    BlankNode,
    Literal,
    BlankNodeOrIri,
    BlankNodeOrLiteral,
    IriOrLiteral,
}

#[derive(Debug, Clone, PartialEq, Eq)]
// @lat: [[semantics#Supported Features#Tier 1]]
pub enum Constraint {
    MinCount(u64),
    MaxCount(u64),
    Datatype(NamedNode),
    NodeKind(NodeKind),
    Class(NamedNode),
    MinInclusive(Literal),
    MaxInclusive(Literal),
    MinExclusive(Literal),
    MaxExclusive(Literal),
    MinLength(u64),
    MaxLength(u64),
    Pattern {
        pattern: String,
        flags: Option<String>,
    },
    In(Vec<Term>),
    HasValue(Term),
    Equals(NamedNode),
    Disjoint(NamedNode),
    LessThan(NamedNode),
    LessThanOrEquals(NamedNode),
    Closed {
        ignored_properties: Vec<NamedNode>,
    },
    Node(ShapeId),
    Not(ShapeId),
    And(Vec<ShapeId>),
    Or(Vec<ShapeId>),
    Xone(Vec<ShapeId>),
    QualifiedValueShape {
        shape: ShapeId,
        min_count: Option<u64>,
        max_count: Option<u64>,
        disjoint: bool,
    },
    LanguageIn(Vec<String>),
    UniqueLang(bool),
    Sparql(Term),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shape {
    pub id: ShapeId,
    /// Present for property shapes.
    pub path: Option<Located<Path>>,
    pub targets: Vec<Located<Target>>,
    pub constraints: Vec<Located<Constraint>>,
    /// Shapes attached with `sh:property`.
    pub properties: Vec<Located<ShapeId>>,
    pub severity: Severity,
    pub messages: Vec<Literal>,
    pub deactivated: bool,
    pub annotations: Annotations,
    /// Earliest triple with the shape as subject; `None` for shapes that are only referenced.
    pub location: Option<SourceLocation>,
}

/// Every problem found while building the AST, rendered as `file:line: message`.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{}", .messages.join("\n"))]
pub struct AstErrors {
    pub messages: Vec<String>,
}

/// All shapes of a shapes graph, ordered by identity.
#[derive(Debug, Default)]
pub struct Shapes {
    shapes: Vec<Shape>,
    index: HashMap<ShapeId, usize>,
    iri_annotations: HashMap<NamedNode, Annotations>,
}

impl Shapes {
    // @lat: [[architecture#Shapes AST]]
    pub fn from_graph(shapes_graph: &ShapesGraph) -> Result<Self, AstErrors> {
        let mut builder = Builder {
            sg: shapes_graph,
            problems: Vec::new(),
        };
        let ids = builder.shape_ids();
        let mut shapes: Vec<Shape> = ids.iter().map(|id| builder.shape(id)).collect();
        let iri_annotations = builder.iri_annotations(&shapes);
        if !builder.problems.is_empty() {
            let mut problems = builder.problems;
            problems.sort();
            problems.dedup();
            let messages = problems
                .into_iter()
                .map(|(location, message)| match location {
                    Some(location) => {
                        format!("{}: {message}", shapes_graph.display_location(location))
                    }
                    None => message,
                })
                .collect();
            return Err(AstErrors { messages });
        }
        canonical_order(&mut shapes);
        let index = shapes
            .iter()
            .enumerate()
            .map(|(i, shape)| (shape.id.clone(), i))
            .collect();
        Ok(Shapes {
            shapes,
            index,
            iri_annotations,
        })
    }

    pub fn get(&self, id: &ShapeId) -> Option<&Shape> {
        self.index.get(id).map(|&i| &self.shapes[i])
    }

    pub fn iter(&self) -> impl Iterator<Item = &Shape> {
        self.shapes.iter()
    }

    pub fn len(&self) -> usize {
        self.shapes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.shapes.is_empty()
    }

    /// Annotations declared on a class or predicate IRI that is not itself a shape.
    pub fn iri_annotations(&self, iri: NamedNodeRef<'_>) -> Option<&Annotations> {
        self.iri_annotations.get(&iri.into_owned())
    }
}

/// Orders every multi-valued shape setting by content instead of source position,
/// so reordered triples or renamed blank nodes never change the compiled output.
/// RDF lists (`sh:or`, `sh:in`, …) keep their order.
// @lat: [[architecture#Shapes AST]]
fn canonical_order(shapes: &mut [Shape]) {
    let keys: HashMap<ShapeId, String> = {
        let by_id: HashMap<&ShapeId, &Shape> = shapes.iter().map(|s| (&s.id, s)).collect();
        shapes
            .iter()
            .map(|s| {
                (
                    s.id.clone(),
                    canonical_shape(&s.id, &by_id, &mut Vec::new()),
                )
            })
            .collect()
    };
    let key_of = |id: &ShapeId| match id {
        NamedOrBlankNode::NamedNode(iri) => iri.to_string(),
        NamedOrBlankNode::BlankNode(_) => keys.get(id).cloned().unwrap_or_else(|| "[]".into()),
    };
    for shape in shapes.iter_mut() {
        shape.properties.sort_by_cached_key(|p| key_of(&p.value));
        shape
            .constraints
            .sort_by_cached_key(|c| constraint_text(&c.value, &mut |s| key_of(s)));
        shape
            .targets
            .sort_by_cached_key(|t| format!("{:?}", t.value));
        shape.messages.sort_by_cached_key(|m| m.to_string());
    }
}

/// A blank shape's content with nested shapes expanded; named shapes are their IRI.
fn canonical_shape(
    id: &ShapeId,
    by_id: &HashMap<&ShapeId, &Shape>,
    seen: &mut Vec<ShapeId>,
) -> String {
    if let NamedOrBlankNode::NamedNode(iri) = id {
        return iri.to_string();
    }
    let Some(shape) = by_id.get(id) else {
        return "[]".into();
    };
    if seen.contains(id) {
        return "[cycle]".into();
    }
    seen.push(id.clone());
    let mut properties: Vec<String> = shape
        .properties
        .iter()
        .map(|p| canonical_shape(&p.value, by_id, seen))
        .collect();
    properties.sort();
    let mut constraints: Vec<String> = shape
        .constraints
        .iter()
        .map(|c| constraint_text(&c.value, &mut |s| canonical_shape(s, by_id, seen)))
        .collect();
    constraints.sort();
    seen.pop();
    let mut targets: Vec<String> = shape
        .targets
        .iter()
        .map(|t| format!("{:?}", t.value))
        .collect();
    targets.sort();
    let mut messages: Vec<String> = shape.messages.iter().map(|m| m.to_string()).collect();
    messages.sort();
    format!(
        "[path={:?};targets={targets:?};constraints={constraints:?};properties={properties:?};severity={:?};messages={messages:?};deactivated={}]",
        shape.path.as_ref().map(|p| &p.value),
        shape.severity,
        shape.deactivated
    )
}

fn constraint_text(constraint: &Constraint, shape: &mut dyn FnMut(&ShapeId) -> String) -> String {
    let mut list = |items: &[ShapeId]| items.iter().map(&mut *shape).collect::<Vec<_>>();
    match constraint {
        Constraint::And(items) => format!("And({:?})", list(items)),
        Constraint::Or(items) => format!("Or({:?})", list(items)),
        Constraint::Xone(items) => format!("Xone({:?})", list(items)),
        Constraint::Node(s) => format!("Node({:?})", list(std::slice::from_ref(s))),
        Constraint::Not(s) => format!("Not({:?})", list(std::slice::from_ref(s))),
        Constraint::QualifiedValueShape {
            shape: s,
            min_count,
            max_count,
            disjoint,
        } => format!(
            "QualifiedValueShape({:?},{min_count:?},{max_count:?},{disjoint})",
            list(std::slice::from_ref(s))
        ),
        other => format!("{other:?}"),
    }
}

struct Builder<'g> {
    sg: &'g ShapesGraph,
    problems: Vec<(Option<SourceLocation>, String)>,
}

impl<'g> Builder<'g> {
    fn graph(&self) -> &'g Graph {
        &self.sg.graph
    }

    fn problem(&mut self, location: SourceLocation, message: String) {
        self.problems.push((Some(location), message));
    }

    fn shape_ids(&self) -> Vec<ShapeId> {
        let graph = self.graph();
        let mut ids: HashSet<ShapeId> = HashSet::new();
        for class in [sh::NODE_SHAPE, sh::PROPERTY_SHAPE] {
            ids.extend(
                graph
                    .subjects_for_predicate_object(rdf::TYPE, class)
                    .map(NamedOrBlankNodeRef::into_owned),
            );
        }
        for &predicate in SHAPE_SUBJECT_PARAMETERS {
            ids.extend(
                graph
                    .triples_for_predicate(predicate)
                    .map(|t| t.subject.into_owned()),
            );
        }
        for predicate in [sh::PROPERTY, sh::NODE, sh::NOT, sh::QUALIFIED_VALUE_SHAPE] {
            ids.extend(
                graph
                    .triples_for_predicate(predicate)
                    .filter_map(|t| as_node(t.object)),
            );
        }
        for predicate in [sh::AND, sh::OR, sh::XONE] {
            for triple in graph.triples_for_predicate(predicate) {
                if let Ok(items) = self.list(triple.object) {
                    ids.extend(items.into_iter().filter_map(as_node));
                }
            }
        }
        let mut ids: Vec<ShapeId> = ids.into_iter().collect();
        ids.sort_by_cached_key(|id| id.to_string());
        ids
    }

    fn shape(&mut self, id: &ShapeId) -> Shape {
        let s = id.as_ref();
        let graph = self.graph();
        let location = graph.triples_for_subject(s).map(|t| self.location(t)).min();

        let path = self.single(s, sh::PATH).and_then(|(term, location)| {
            let mut visiting = HashSet::new();
            self.path(term, location, &mut visiting)
                .map(|value| Located { value, location })
        });

        let mut targets = Vec::new();
        for class in self.iris(s, sh::TARGET_CLASS) {
            targets.push(class.map(Target::Class));
        }
        if let NamedOrBlankNodeRef::NamedNode(iri) = s {
            let typed_class = TripleRef::new(iri, rdf::TYPE, rdfs::CLASS);
            if graph.contains(typed_class) {
                targets.push(Located {
                    value: Target::ImplicitClass(iri.into_owned()),
                    location: self.location(typed_class),
                });
            }
        }
        for predicate in self.iris(s, sh::TARGET_SUBJECTS_OF) {
            targets.push(predicate.map(Target::SubjectsOf));
        }
        for predicate in self.iris(s, sh::TARGET_OBJECTS_OF) {
            targets.push(predicate.map(Target::ObjectsOf));
        }
        for (term, location) in self.values(s, sh::TARGET_NODE) {
            targets.push(Located {
                value: Target::Node(term.into_owned()),
                location,
            });
        }
        for relationship in self.target_relationships(s) {
            targets.push(relationship.map(Target::Relationship));
        }

        let properties = self
            .values(s, sh::PROPERTY)
            .into_iter()
            .filter_map(|(term, location)| {
                self.shape_ref(sh::PROPERTY, term, location)
                    .map(|value| Located { value, location })
            })
            .collect();

        let severity = match self.single(s, sh::SEVERITY) {
            None => Severity::Violation,
            Some((TermRef::NamedNode(iri), _)) if iri == sh::VIOLATION => Severity::Violation,
            Some((TermRef::NamedNode(iri), _)) if iri == sh::WARNING => Severity::Warning,
            Some((TermRef::NamedNode(iri), _)) if iri == sh::INFO => Severity::Info,
            Some((TermRef::NamedNode(iri), _)) => Severity::Other(iri.into_owned()),
            Some((term, location)) => {
                self.problem(
                    location,
                    format!("sh:severity must be an IRI, found {term}"),
                );
                Severity::Violation
            }
        };

        let mut messages = Vec::new();
        for (term, location) in self.values(s, sh::MESSAGE) {
            match term {
                TermRef::Literal(literal) => messages.push(literal.into_owned()),
                _ => self.problem(
                    location,
                    format!("sh:message must be a literal, found {term}"),
                ),
            }
        }

        let deactivated = self
            .boolean(s, sh::DEACTIVATED)
            .is_some_and(|flag| flag.value);
        let annotations = self.annotations(s);
        let constraints = self.constraints(s);

        Shape {
            id: id.clone(),
            path,
            targets,
            constraints,
            properties,
            severity,
            messages,
            deactivated,
            annotations,
            location,
        }
    }

    fn constraints(&mut self, s: NamedOrBlankNodeRef<'_>) -> Vec<Located<Constraint>> {
        let mut out = Vec::new();
        if let Some(n) = self.count(s, sh::MIN_COUNT) {
            out.push(n.map(Constraint::MinCount));
        }
        if let Some(n) = self.count(s, sh::MAX_COUNT) {
            out.push(n.map(Constraint::MaxCount));
        }
        if let Some(iri) = self.iri(s, sh::DATATYPE) {
            out.push(iri.map(Constraint::Datatype));
        }
        if let Some(kind) = self.node_kind(s) {
            out.push(kind.map(Constraint::NodeKind));
        }
        for class in self.iris(s, sh::CLASS) {
            out.push(class.map(Constraint::Class));
        }
        for (predicate, make) in [
            (
                sh::MIN_INCLUSIVE,
                Constraint::MinInclusive as fn(Literal) -> Constraint,
            ),
            (sh::MAX_INCLUSIVE, Constraint::MaxInclusive),
            (sh::MIN_EXCLUSIVE, Constraint::MinExclusive),
            (sh::MAX_EXCLUSIVE, Constraint::MaxExclusive),
        ] {
            if let Some(literal) = self.literal(s, predicate) {
                out.push(literal.map(make));
            }
        }
        if let Some(n) = self.count(s, sh::MIN_LENGTH) {
            out.push(n.map(Constraint::MinLength));
        }
        if let Some(n) = self.count(s, sh::MAX_LENGTH) {
            out.push(n.map(Constraint::MaxLength));
        }
        let flags = self
            .single(s, sh::FLAGS)
            .and_then(|(term, location)| self.string(sh::FLAGS, term, location));
        for (term, location) in self.values(s, sh::PATTERN) {
            if let Some(pattern) = self.string(sh::PATTERN, term, location) {
                out.push(Located {
                    value: Constraint::Pattern {
                        pattern,
                        flags: flags.clone(),
                    },
                    location,
                });
            }
        }
        if let Some((term, location)) = self.single(s, sh::IN) {
            if let Some(items) = self.list_at(sh::IN, term, location) {
                out.push(Located {
                    value: Constraint::In(items.into_iter().map(TermRef::into_owned).collect()),
                    location,
                });
            }
        }
        for (term, location) in self.values(s, sh::HAS_VALUE) {
            out.push(Located {
                value: Constraint::HasValue(term.into_owned()),
                location,
            });
        }
        for (predicate, make) in [
            (
                sh::EQUALS,
                Constraint::Equals as fn(NamedNode) -> Constraint,
            ),
            (sh::DISJOINT, Constraint::Disjoint),
            (sh::LESS_THAN, Constraint::LessThan),
            (sh::LESS_THAN_OR_EQUALS, Constraint::LessThanOrEquals),
        ] {
            for iri in self.iris(s, predicate) {
                out.push(iri.map(make));
            }
        }
        if let Some(closed) = self.boolean(s, sh::CLOSED) {
            if closed.value {
                let ignored_properties = self.ignored_properties(s);
                out.push(closed.map(|_| Constraint::Closed { ignored_properties }));
            }
        }
        for (predicate, make) in [
            (sh::NODE, Constraint::Node as fn(ShapeId) -> Constraint),
            (sh::NOT, Constraint::Not),
        ] {
            for (term, location) in self.values(s, predicate) {
                if let Some(shape) = self.shape_ref(predicate, term, location) {
                    out.push(Located {
                        value: make(shape),
                        location,
                    });
                }
            }
        }
        for (predicate, make) in [
            (sh::AND, Constraint::And as fn(Vec<ShapeId>) -> Constraint),
            (sh::OR, Constraint::Or),
            (sh::XONE, Constraint::Xone),
        ] {
            for (term, location) in self.values(s, predicate) {
                let Some(items) = self.list_at(predicate, term, location) else {
                    continue;
                };
                let members: Option<Vec<ShapeId>> = items
                    .into_iter()
                    .map(|item| self.shape_ref(predicate, item, location))
                    .collect();
                if let Some(members) = members {
                    out.push(Located {
                        value: make(members),
                        location,
                    });
                }
            }
        }
        self.qualified(s, &mut out);
        if let Some((term, location)) = self.single(s, sh::LANGUAGE_IN) {
            if let Some(items) = self.list_at(sh::LANGUAGE_IN, term, location) {
                let tags: Option<Vec<String>> = items
                    .into_iter()
                    .map(|item| self.string(sh::LANGUAGE_IN, item, location))
                    .collect();
                if let Some(tags) = tags {
                    out.push(Located {
                        value: Constraint::LanguageIn(tags),
                        location,
                    });
                }
            }
        }
        if let Some(unique) = self.boolean(s, sh::UNIQUE_LANG) {
            if unique.value {
                out.push(unique.map(Constraint::UniqueLang));
            }
        }
        for (term, location) in self.values(s, sh::SPARQL) {
            out.push(Located {
                value: Constraint::Sparql(term.into_owned()),
                location,
            });
        }
        out
    }

    fn qualified(&mut self, s: NamedOrBlankNodeRef<'_>, out: &mut Vec<Located<Constraint>>) {
        let shapes = self.values(s, sh::QUALIFIED_VALUE_SHAPE);
        let min = self.count(s, sh::QUALIFIED_MIN_COUNT);
        let max = self.count(s, sh::QUALIFIED_MAX_COUNT);
        let disjoint = self
            .boolean(s, sh::QUALIFIED_VALUE_SHAPES_DISJOINT)
            .is_some_and(|flag| flag.value);
        if shapes.is_empty() {
            if let Some(count) = min.as_ref().or(max.as_ref()) {
                self.problem(
                    count.location,
                    "sh:qualifiedMinCount and sh:qualifiedMaxCount require sh:qualifiedValueShape"
                        .into(),
                );
            }
            return;
        }
        if min.is_none() && max.is_none() {
            self.problem(
                shapes[0].1,
                "sh:qualifiedValueShape requires sh:qualifiedMinCount or sh:qualifiedMaxCount"
                    .into(),
            );
            return;
        }
        for (term, location) in shapes {
            if let Some(shape) = self.shape_ref(sh::QUALIFIED_VALUE_SHAPE, term, location) {
                out.push(Located {
                    value: Constraint::QualifiedValueShape {
                        shape,
                        min_count: min.as_ref().map(|c| c.value),
                        max_count: max.as_ref().map(|c| c.value),
                        disjoint,
                    },
                    location,
                });
            }
        }
    }

    fn ignored_properties(&mut self, s: NamedOrBlankNodeRef<'_>) -> Vec<NamedNode> {
        let Some((term, location)) = self.single(s, sh::IGNORED_PROPERTIES) else {
            return Vec::new();
        };
        let Some(items) = self.list_at(sh::IGNORED_PROPERTIES, term, location) else {
            return Vec::new();
        };
        items
            .into_iter()
            .filter_map(|item| self.iri_value(sh::IGNORED_PROPERTIES, item, location))
            .collect()
    }

    fn node_kind(&mut self, s: NamedOrBlankNodeRef<'_>) -> Option<Located<NodeKind>> {
        let (term, location) = self.single(s, sh::NODE_KIND)?;
        let kind = match term {
            TermRef::NamedNode(iri) if iri == sh::IRI => NodeKind::Iri,
            TermRef::NamedNode(iri) if iri == sh::BLANK_NODE => NodeKind::BlankNode,
            TermRef::NamedNode(iri) if iri == sh::LITERAL => NodeKind::Literal,
            TermRef::NamedNode(iri) if iri == sh::BLANK_NODE_OR_IRI => NodeKind::BlankNodeOrIri,
            TermRef::NamedNode(iri) if iri == sh::BLANK_NODE_OR_LITERAL => {
                NodeKind::BlankNodeOrLiteral
            }
            TermRef::NamedNode(iri) if iri == sh::IRI_OR_LITERAL => NodeKind::IriOrLiteral,
            _ => {
                self.problem(location, format!("sh:nodeKind has unknown value {term}"));
                return None;
            }
        };
        Some(Located {
            value: kind,
            location,
        })
    }

    fn path(
        &mut self,
        term: TermRef<'g>,
        location: SourceLocation,
        visiting: &mut HashSet<BlankNode>,
    ) -> Option<Path> {
        let node = match term {
            TermRef::NamedNode(iri) if iri != rdf::NIL => {
                return Some(Path::Predicate(iri.into_owned()))
            }
            TermRef::BlankNode(node) => node,
            _ => {
                self.problem(
                    location,
                    format!("sh:path must be an IRI or a blank node path, found {term}"),
                );
                return None;
            }
        };
        if !visiting.insert(node.into_owned()) {
            self.problem(location, "cyclic property path".into());
            return None;
        }
        let graph = self.graph();
        let result = if graph
            .object_for_subject_predicate(node, rdf::FIRST)
            .is_some()
        {
            self.path_list(term, location, visiting, "sequence")
                .map(Path::Sequence)
        } else if let Some((inner, _)) = self.single(node.into(), sh::INVERSE_PATH) {
            self.path(inner, location, visiting)
                .map(|p| Path::Inverse(Box::new(p)))
        } else if let Some((list, _)) = self.single(node.into(), sh::ALTERNATIVE_PATH) {
            self.path_list(list, location, visiting, "alternative")
                .map(Path::Alternative)
        } else if let Some((inner, _)) = self.single(node.into(), sh::ZERO_OR_MORE_PATH) {
            self.path(inner, location, visiting)
                .map(|p| Path::ZeroOrMore(Box::new(p)))
        } else if let Some((inner, _)) = self.single(node.into(), sh::ONE_OR_MORE_PATH) {
            self.path(inner, location, visiting)
                .map(|p| Path::OneOrMore(Box::new(p)))
        } else if let Some((inner, _)) = self.single(node.into(), sh::ZERO_OR_ONE_PATH) {
            self.path(inner, location, visiting)
                .map(|p| Path::ZeroOrOne(Box::new(p)))
        } else {
            self.problem(location, "unrecognized property path".into());
            None
        };
        visiting.remove(&node.into_owned());
        result
    }

    fn path_list(
        &mut self,
        list: TermRef<'g>,
        location: SourceLocation,
        visiting: &mut HashSet<BlankNode>,
        kind: &str,
    ) -> Option<Vec<Path>> {
        let items = match self.list(list) {
            Ok(items) if items.len() >= 2 => items,
            Ok(_) => {
                self.problem(location, format!("{kind} path needs at least two members"));
                return None;
            }
            Err(message) => {
                self.problem(location, format!("sh:path: {message}"));
                return None;
            }
        };
        items
            .into_iter()
            .map(|item| self.path(item, location, visiting))
            .collect()
    }

    /// Items of a well-formed RDF list.
    fn list(&self, head: TermRef<'g>) -> Result<Vec<TermRef<'g>>, String> {
        let graph = self.graph();
        let mut items = Vec::new();
        let mut seen = HashSet::new();
        let mut node = head;
        loop {
            let subject: NamedOrBlankNodeRef<'g> = match node {
                TermRef::NamedNode(iri) if iri == rdf::NIL => return Ok(items),
                TermRef::NamedNode(iri) => iri.into(),
                TermRef::BlankNode(blank) => blank.into(),
                _ => return Err(format!("expected an RDF list, found {node}")),
            };
            if !seen.insert(subject.into_owned()) {
                return Err("cyclic RDF list".into());
            }
            let firsts: Vec<_> = graph
                .objects_for_subject_predicate(subject, rdf::FIRST)
                .collect();
            let rests: Vec<_> = graph
                .objects_for_subject_predicate(subject, rdf::REST)
                .collect();
            let ([first], [rest]) = (firsts.as_slice(), rests.as_slice()) else {
                return Err(format!(
                    "malformed RDF list at {node} (each list node needs exactly one rdf:first and one rdf:rest)"
                ));
            };
            items.push(*first);
            node = *rest;
        }
    }

    fn list_at(
        &mut self,
        predicate: NamedNodeRef<'_>,
        term: TermRef<'g>,
        location: SourceLocation,
    ) -> Option<Vec<TermRef<'g>>> {
        match self.list(term) {
            Ok(items) => Some(items),
            Err(message) => {
                let name = self.compact(predicate.as_str());
                self.problem(location, format!("{name}: {message}"));
                None
            }
        }
    }

    fn location(&self, triple: TripleRef<'_>) -> SourceLocation {
        self.sg
            .locations(&triple.into_owned())
            .iter()
            .min()
            .copied()
            .expect("every loaded triple has a location")
    }

    /// All values of a parameter, ordered by source location.
    fn values(
        &self,
        subject: NamedOrBlankNodeRef<'_>,
        predicate: NamedNodeRef<'_>,
    ) -> Vec<(TermRef<'g>, SourceLocation)> {
        let graph = self.graph();
        let mut values: Vec<(TermRef<'g>, SourceLocation)> = graph
            .objects_for_subject_predicate(subject, predicate)
            .map(|object| {
                let location = self.location(TripleRef::new(subject, predicate, object));
                (object, location)
            })
            .collect();
        values.sort_by(|a, b| {
            a.1.cmp(&b.1)
                .then_with(|| a.0.to_string().cmp(&b.0.to_string()))
        });
        values
    }

    /// The value of a single-valued parameter; more than one value is a problem.
    fn single(
        &mut self,
        subject: NamedOrBlankNodeRef<'_>,
        predicate: NamedNodeRef<'_>,
    ) -> Option<(TermRef<'g>, SourceLocation)> {
        let values = self.values(subject, predicate);
        if values.len() > 1 {
            let at: Vec<String> = values
                .iter()
                .map(|(_, location)| self.sg.display_location(*location))
                .collect();
            let message = format!(
                "{} has {} values for {}, expected at most one (at {})",
                self.node_name(subject),
                values.len(),
                self.compact(predicate.as_str()),
                at.join(", ")
            );
            self.problem(values[0].1, message);
        }
        values.into_iter().next()
    }

    fn count(
        &mut self,
        subject: NamedOrBlankNodeRef<'_>,
        predicate: NamedNodeRef<'_>,
    ) -> Option<Located<u64>> {
        let (term, location) = self.single(subject, predicate)?;
        if let TermRef::Literal(literal) = term {
            if is_integer_datatype(literal.datatype()) {
                if let Ok(value) = literal.value().parse::<u64>() {
                    return Some(Located { value, location });
                }
            }
        }
        let name = self.compact(predicate.as_str());
        self.problem(
            location,
            format!("{name} must be a non-negative integer, found {term}"),
        );
        None
    }

    fn boolean(
        &mut self,
        subject: NamedOrBlankNodeRef<'_>,
        predicate: NamedNodeRef<'_>,
    ) -> Option<Located<bool>> {
        let (term, location) = self.single(subject, predicate)?;
        if let TermRef::Literal(literal) = term {
            if literal.datatype() == xsd::BOOLEAN {
                match literal.value() {
                    "true" | "1" => {
                        return Some(Located {
                            value: true,
                            location,
                        })
                    }
                    "false" | "0" => {
                        return Some(Located {
                            value: false,
                            location,
                        })
                    }
                    _ => {}
                }
            }
        }
        let name = self.compact(predicate.as_str());
        self.problem(location, format!("{name} must be a boolean, found {term}"));
        None
    }

    fn literal(
        &mut self,
        subject: NamedOrBlankNodeRef<'_>,
        predicate: NamedNodeRef<'_>,
    ) -> Option<Located<Literal>> {
        let (term, location) = self.single(subject, predicate)?;
        if let TermRef::Literal(literal) = term {
            return Some(Located {
                value: literal.into_owned(),
                location,
            });
        }
        let name = self.compact(predicate.as_str());
        self.problem(location, format!("{name} must be a literal, found {term}"));
        None
    }

    fn string(
        &mut self,
        predicate: NamedNodeRef<'_>,
        term: TermRef<'g>,
        location: SourceLocation,
    ) -> Option<String> {
        if let TermRef::Literal(literal) = term {
            if literal.datatype() == xsd::STRING {
                return Some(literal.value().to_owned());
            }
        }
        let name = self.compact(predicate.as_str());
        self.problem(location, format!("{name} must be a string, found {term}"));
        None
    }

    fn iri(
        &mut self,
        subject: NamedOrBlankNodeRef<'_>,
        predicate: NamedNodeRef<'_>,
    ) -> Option<Located<NamedNode>> {
        let (term, location) = self.single(subject, predicate)?;
        self.iri_value(predicate, term, location)
            .map(|value| Located { value, location })
    }

    fn iris(
        &mut self,
        subject: NamedOrBlankNodeRef<'_>,
        predicate: NamedNodeRef<'_>,
    ) -> Vec<Located<NamedNode>> {
        self.values(subject, predicate)
            .into_iter()
            .filter_map(|(term, location)| {
                self.iri_value(predicate, term, location)
                    .map(|value| Located { value, location })
            })
            .collect()
    }

    fn iri_value(
        &mut self,
        predicate: NamedNodeRef<'_>,
        term: TermRef<'g>,
        location: SourceLocation,
    ) -> Option<NamedNode> {
        if let TermRef::NamedNode(iri) = term {
            return Some(iri.into_owned());
        }
        let name = self.compact(predicate.as_str());
        self.problem(location, format!("{name} must be an IRI, found {term}"));
        None
    }

    fn shape_ref(
        &mut self,
        predicate: NamedNodeRef<'_>,
        term: TermRef<'g>,
        location: SourceLocation,
    ) -> Option<ShapeId> {
        let shape = as_node(term);
        if shape.is_none() {
            let name = self.compact(predicate.as_str());
            self.problem(
                location,
                format!("{name} must reference a shape (IRI or blank node), found {term}"),
            );
        }
        shape
    }

    fn node_name(&self, node: NamedOrBlankNodeRef<'_>) -> String {
        match node {
            NamedOrBlankNodeRef::NamedNode(iri) => self.compact(iri.as_str()),
            NamedOrBlankNodeRef::BlankNode(_) => "blank node shape".into(),
        }
    }

    fn compact(&self, iri: &str) -> String {
        self.sg.compact(iri)
    }
}

fn as_node(term: TermRef<'_>) -> Option<ShapeId> {
    match term {
        TermRef::NamedNode(iri) => Some(iri.into_owned().into()),
        TermRef::BlankNode(node) => Some(node.into_owned().into()),
        _ => None,
    }
}

fn is_integer_datatype(datatype: NamedNodeRef<'_>) -> bool {
    datatype
        .as_str()
        .strip_prefix("http://www.w3.org/2001/XMLSchema#")
        .is_some_and(|local| {
            matches!(
                local,
                "integer"
                    | "nonNegativeInteger"
                    | "positiveInteger"
                    | "long"
                    | "int"
                    | "short"
                    | "byte"
                    | "unsignedLong"
                    | "unsignedInt"
                    | "unsignedShort"
                    | "unsignedByte"
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Five prefix lines, so the first body line is line 6 of `person.ttl`.
    const PREFIXES: &str = "@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix ex: <http://example.org/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
";

    fn parse(body: &str) -> Result<Shapes, AstErrors> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("person.ttl");
        std::fs::write(&path, format!("{PREFIXES}{body}")).unwrap();
        let graph = ShapesGraph::load(&[path]).unwrap();
        Shapes::from_graph(&graph)
    }

    fn ex(local: &str) -> NamedNode {
        NamedNode::new_unchecked(format!("http://example.org/{local}"))
    }

    fn shape_id(local: &str) -> ShapeId {
        ex(local).into()
    }

    fn values<T: Clone>(items: &[Located<T>]) -> Vec<T> {
        items.iter().map(|item| item.value.clone()).collect()
    }

    fn property_with_path<'a>(shapes: &'a Shapes, local: &str) -> &'a Shape {
        shapes
            .iter()
            .find(|shape| {
                shape
                    .path
                    .as_ref()
                    .is_some_and(|path| path.value == Path::Predicate(ex(local)))
            })
            .unwrap_or_else(|| panic!("no property shape with path ex:{local}"))
    }

    #[test]
    fn parses_node_and_property_shapes_with_defaults() {
        let shapes = parse(
            "ex:PersonShape a sh:NodeShape ;
    sh:targetClass ex:Person ;
    sh:closed true ;
    sh:ignoredProperties ( ex:id ) ;
    sh:property [
        sh:path ex:name ;
        sh:minCount 1 ;
        sh:maxCount 1 ;
        sh:datatype xsd:string ;
    ] .
",
        )
        .unwrap();
        assert_eq!(shapes.len(), 2);
        let person = shapes.get(&shape_id("PersonShape")).unwrap();
        assert!(person.path.is_none());
        assert_eq!(values(&person.targets), vec![Target::Class(ex("Person"))]);
        assert_eq!(
            values(&person.constraints),
            vec![Constraint::Closed {
                ignored_properties: vec![ex("id")]
            }]
        );
        assert_eq!(person.severity, Severity::Violation);
        assert!(!person.deactivated);
        assert_eq!(person.annotations, Annotations::default());
        assert_eq!(person.location.map(|l| l.line), Some(6));

        let name = shapes.get(&person.properties[0].value).unwrap();
        assert_eq!(
            name.path.as_ref().unwrap().value,
            Path::Predicate(ex("name"))
        );
        assert_eq!(
            values(&name.constraints),
            vec![
                Constraint::Datatype(xsd::STRING.into_owned()),
                Constraint::MaxCount(1),
                Constraint::MinCount(1),
            ]
        );
        assert_eq!(name.constraints[2].location.line, 12);
    }

    #[test]
    fn shape_iri_that_is_a_class_gets_an_implicit_target() {
        let shapes = parse(
            "ex:Person a rdfs:Class, sh:NodeShape ;
    sh:property [ sh:path ex:name ; sh:minCount 1 ] .
",
        )
        .unwrap();
        let person = shapes.get(&shape_id("Person")).unwrap();
        assert_eq!(
            values(&person.targets),
            vec![Target::ImplicitClass(ex("Person"))]
        );
    }

    #[test]
    fn recognizes_referenced_and_list_member_shapes() {
        let shapes = parse(
            "ex:S sh:node ex:T ;
    sh:or ( ex:A [ sh:minCount 1 ] ) .
",
        )
        .unwrap();
        assert_eq!(shapes.len(), 4);
        assert!(shapes.get(&shape_id("T")).unwrap().location.is_none());
        let s = shapes.get(&shape_id("S")).unwrap();
        let constraints = values(&s.constraints);
        assert_eq!(constraints[0], Constraint::Node(shape_id("T")));
        let Constraint::Or(members) = &constraints[1] else {
            panic!("expected sh:or, got {:?}", constraints[1]);
        };
        assert_eq!(members.len(), 2);
        assert_eq!(members[0], shape_id("A"));
        assert_eq!(
            values(&shapes.get(&members[1]).unwrap().constraints),
            vec![Constraint::MinCount(1)]
        );
    }

    #[test]
    fn parses_complex_property_paths() {
        let shapes = parse(
            "ex:S sh:property [
    sh:path ( ex:a [ sh:inversePath ex:b ] [ sh:alternativePath ( ex:c [ sh:zeroOrMorePath ex:d ] ) ] ) ;
] , [
    sh:path [ sh:oneOrMorePath ex:e ] ;
] , [
    sh:path [ sh:zeroOrOnePath ex:f ] ;
] .
",
        )
        .unwrap();
        let paths: Vec<Path> = shapes
            .iter()
            .filter_map(|shape| shape.path.as_ref().map(|p| p.value.clone()))
            .collect();
        let predicate = |local| Path::Predicate(ex(local));
        assert!(paths.contains(&Path::Sequence(vec![
            predicate("a"),
            Path::Inverse(Box::new(predicate("b"))),
            Path::Alternative(vec![
                predicate("c"),
                Path::ZeroOrMore(Box::new(predicate("d"))),
            ]),
        ])));
        assert!(paths.contains(&Path::OneOrMore(Box::new(predicate("e")))));
        assert!(paths.contains(&Path::ZeroOrOne(Box::new(predicate("f")))));
    }

    #[test]
    fn parses_targets_values_and_rejected_constructs() {
        let shapes = parse(
            "ex:S a sh:NodeShape ;
    sh:targetNode ex:alice ;
    sh:targetSubjectsOf ex:knows ;
    sh:targetObjectsOf ex:knows ;
    sh:property [
        sh:path ex:status ;
        sh:in ( \"a\" 1 ex:X ) ;
        sh:hasValue \"a\" ;
        sh:pattern \"^a\" ;
        sh:flags \"i\" ;
        sh:minInclusive 0 ;
        sh:languageIn ( \"en\" ) ;
        sh:uniqueLang true ;
        sh:nodeKind sh:IRI ;
        sh:lessThan ex:other ;
    ] , [
        sh:path ex:address ;
        sh:qualifiedValueShape ex:AddressShape ;
        sh:qualifiedMinCount 1 ;
    ] ;
    sh:sparql [ sh:select \"SELECT $this WHERE {}\" ] .
",
        )
        .unwrap();
        let s = shapes.get(&shape_id("S")).unwrap();
        assert_eq!(
            values(&s.targets),
            vec![
                Target::Node(ex("alice").into()),
                Target::ObjectsOf(ex("knows")),
                Target::SubjectsOf(ex("knows")),
            ]
        );
        assert!(matches!(
            values(&s.constraints).as_slice(),
            [Constraint::Sparql(_)]
        ));

        let integer = |v: &str| Literal::new_typed_literal(v, xsd::INTEGER);
        let status = values(&property_with_path(&shapes, "status").constraints);
        for expected in [
            Constraint::In(vec![
                Literal::new_simple_literal("a").into(),
                integer("1").into(),
                ex("X").into(),
            ]),
            Constraint::HasValue(Literal::new_simple_literal("a").into()),
            Constraint::Pattern {
                pattern: "^a".into(),
                flags: Some("i".into()),
            },
            Constraint::MinInclusive(integer("0")),
            Constraint::LanguageIn(vec!["en".into()]),
            Constraint::UniqueLang(true),
            Constraint::NodeKind(NodeKind::Iri),
            Constraint::LessThan(ex("other")),
        ] {
            assert!(status.contains(&expected), "missing {expected:?}");
        }
        assert_eq!(
            values(&property_with_path(&shapes, "address").constraints),
            vec![Constraint::QualifiedValueShape {
                shape: shape_id("AddressShape"),
                min_count: Some(1),
                max_count: None,
                disjoint: false,
            }]
        );
    }

    #[test]
    fn parses_severity_deactivation_and_messages() {
        let shapes = parse(
            "ex:S a sh:NodeShape ;
    sh:severity sh:Warning ;
    sh:deactivated true ;
    sh:message \"Broken\"@en, \"Kaputt\"@de .
",
        )
        .unwrap();
        let s = shapes.get(&shape_id("S")).unwrap();
        assert_eq!(s.severity, Severity::Warning);
        assert!(s.deactivated);
        assert_eq!(s.messages.len(), 2);
    }

    #[test]
    fn malformed_count_reports_file_and_line() {
        let err = parse(
            "ex:S a sh:NodeShape ;
    sh:property [
        sh:path ex:name ;
        sh:minCount \"two\" ;
    ] .
",
        )
        .unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains("person.ttl:9: sh:minCount must be a non-negative integer"),
            "{message}"
        );
    }

    #[test]
    fn multiple_values_for_single_valued_parameter_list_all_locations() {
        let err = parse(
            "ex:S sh:path ex:p ;
    sh:maxCount 1 ;
    sh:maxCount 2 .
",
        )
        .unwrap_err();
        let message = err.to_string();
        assert!(message.contains("expected at most one"), "{message}");
        assert!(message.contains("person.ttl:7"), "{message}");
        assert!(message.contains("person.ttl:8"), "{message}");
    }

    #[test]
    fn collects_every_problem() {
        let err = parse(
            "ex:S sh:path ex:p ;
    sh:minCount -1 ;
    sh:datatype \"string\" ;
    sh:node \"not a shape\" .
",
        )
        .unwrap_err();
        assert_eq!(err.messages.len(), 3, "{err}");
    }

    #[test]
    fn rejects_malformed_and_cyclic_lists() {
        let malformed = parse("ex:S a sh:NodeShape ;\n    sh:in ex:notAList .\n").unwrap_err();
        assert!(
            malformed.to_string().contains("malformed RDF list"),
            "{malformed}"
        );

        let cyclic =
            parse("ex:S a sh:NodeShape ;\n    sh:in _:l .\n_:l rdf:first ex:a ; rdf:rest _:l .\n")
                .unwrap_err();
        assert!(cyclic.to_string().contains("cyclic RDF list"), "{cyclic}");
    }

    #[test]
    fn qualified_value_shape_requires_a_count() {
        let err = parse("ex:S sh:path ex:p ;\n    sh:qualifiedValueShape ex:T .\n").unwrap_err();
        assert!(err.to_string().contains("sh:qualifiedMinCount"), "{err}");
    }
}
