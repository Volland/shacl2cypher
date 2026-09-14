//! Resolution of SHACL classes and property paths to LPG labels, property keys
//! and relationship types.

use oxrdf::NamedNodeRef;

use crate::ast::{
    Annotations, Constraint, Direction, Located, NodeKind, Path, Shape, Shapes, Target,
};
use crate::load::{ShapesGraph, SourceLocation};
use crate::schema::SchemaSnapshot;

/// Why a name was chosen; `--strict` rejects names chosen by convention alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Evidence {
    Annotation(SourceLocation),
    Schema,
    Convention,
}

/// An LPG name together with the evidence it was chosen on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    pub name: String,
    pub evidence: Evidence,
}

/// A property path resolved against the LPG.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LpgPath {
    /// Values of a property of the current node.
    Property(Resolved),
    /// Nodes reached over one relationship.
    Relationship {
        rel_type: Resolved,
        direction: Direction,
    },
    Sequence(Vec<LpgPath>),
    Alternative(Vec<LpgPath>),
    Repeat {
        path: Box<LpgPath>,
        min: u32,
        max: Option<u32>,
    },
}

impl LpgPath {
    /// Whether the path's values are nodes (as opposed to property values).
    pub fn yields_nodes(&self) -> bool {
        match self {
            LpgPath::Property(_) => false,
            LpgPath::Relationship { .. } | LpgPath::Repeat { .. } => true,
            LpgPath::Sequence(steps) => steps.last().is_some_and(LpgPath::yields_nodes),
            LpgPath::Alternative(options) => options.first().is_some_and(LpgPath::yields_nodes),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ResolveOptions {
    /// Reject names chosen by convention alone.
    pub strict: bool,
}

/// A resolution failure, rendered as `file:line: message` when a location is known.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct MappingError(pub String);

pub struct Resolver<'a> {
    graph: &'a ShapesGraph,
    shapes: &'a Shapes,
    schema: Option<&'a SchemaSnapshot>,
    options: ResolveOptions,
}

enum Kind {
    Property,
    Relationship,
}

struct StepContext<'s> {
    /// Shape-level annotations; only consulted for simple paths.
    annotations: Option<&'s Annotations>,
    /// The property shape's constraints imply node values.
    implies_nodes: bool,
    focus: &'s [String],
    location: SourceLocation,
}

impl<'a> Resolver<'a> {
    pub fn new(
        graph: &'a ShapesGraph,
        shapes: &'a Shapes,
        schema: Option<&'a SchemaSnapshot>,
        options: ResolveOptions,
    ) -> Self {
        Resolver {
            graph,
            shapes,
            schema,
            options,
        }
    }

    /// Label of a target class. `shape` is the node shape declaring the target and
    /// `location` the target triple.
    // @lat: [[mapping#Resolution]]
    pub fn class_label(
        &self,
        class: NamedNodeRef<'_>,
        shape: Option<&Shape>,
        location: SourceLocation,
    ) -> Result<Resolved, MappingError> {
        if let Some(label) = shape.and_then(|s| s.annotations.label.as_ref()) {
            let class_targets = shape
                .into_iter()
                .flat_map(|s| &s.targets)
                .filter(|t| matches!(t.value, Target::Class(_) | Target::ImplicitClass(_)))
                .count();
            if class_targets > 1 {
                return Err(self.error(
                    label.location,
                    format!(
                        "s2c:label on a node shape with {class_targets} class targets is ambiguous; annotate the class IRIs instead"
                    ),
                ));
            }
            return Ok(annotated(label));
        }
        if let Some(label) = self
            .shapes
            .iri_annotations(class)
            .and_then(|a| a.label.as_ref())
        {
            return Ok(annotated(label));
        }
        let name = self.local_name(class, location)?.to_owned();
        let evidence = if self.schema.is_some_and(|s| s.node_type(&name).is_some()) {
            Evidence::Schema
        } else {
            Evidence::Convention
        };
        let label = Resolved { name, evidence };
        self.check_strict(&label, class, "label", location)?;
        Ok(label)
    }

    /// Resolves a property shape's path for focus nodes with the given labels
    /// (empty when the focus labels are unknown).
    pub fn path(&self, shape: &Shape, focus: &[String]) -> Result<LpgPath, MappingError> {
        let Some(path) = &shape.path else {
            return Err(MappingError(
                "internal error: path resolution requested for a node shape".into(),
            ));
        };
        let simple = match &path.value {
            Path::Predicate(_) => true,
            Path::Inverse(inner) => matches!(**inner, Path::Predicate(_)),
            _ => false,
        };
        let context = StepContext {
            annotations: simple.then_some(&shape.annotations),
            implies_nodes: implies_node_values(shape),
            focus,
            location: path.location,
        };
        self.lower(&path.value, &context, false, true)
    }

    fn lower(
        &self,
        path: &Path,
        cx: &StepContext<'_>,
        inverted: bool,
        last: bool,
    ) -> Result<LpgPath, MappingError> {
        match path {
            Path::Predicate(predicate) => self.step(predicate.as_ref(), cx, inverted, last),
            Path::Inverse(inner) => self.lower(inner, cx, !inverted, last),
            Path::Sequence(items) => {
                let ordered: Vec<&Path> = if inverted {
                    items.iter().rev().collect()
                } else {
                    items.iter().collect()
                };
                let count = ordered.len();
                ordered
                    .into_iter()
                    .enumerate()
                    .map(|(i, item)| self.lower(item, cx, inverted, last && i + 1 == count))
                    .collect::<Result<Vec<_>, _>>()
                    .map(LpgPath::Sequence)
            }
            Path::Alternative(items) => {
                let options = items
                    .iter()
                    .map(|item| self.lower(item, cx, inverted, last))
                    .collect::<Result<Vec<_>, _>>()?;
                if options
                    .iter()
                    .any(|option| option.yields_nodes() != options[0].yields_nodes())
                {
                    return Err(self.error(
                        cx.location,
                        "alternative path mixes properties and relationships".into(),
                    ));
                }
                Ok(LpgPath::Alternative(options))
            }
            Path::ZeroOrMore(inner) => self.repeat(inner, cx, inverted, 0, None),
            Path::OneOrMore(inner) => self.repeat(inner, cx, inverted, 1, None),
            Path::ZeroOrOne(inner) => self.repeat(inner, cx, inverted, 0, Some(1)),
        }
    }

    fn repeat(
        &self,
        inner: &Path,
        cx: &StepContext<'_>,
        inverted: bool,
        min: u32,
        max: Option<u32>,
    ) -> Result<LpgPath, MappingError> {
        let path = self.lower(inner, cx, inverted, false)?;
        Ok(LpgPath::Repeat {
            path: Box::new(path),
            min,
            max,
        })
    }

    fn step(
        &self,
        predicate: NamedNodeRef<'_>,
        cx: &StepContext<'_>,
        inverted: bool,
        last: bool,
    ) -> Result<LpgPath, MappingError> {
        let shape_annotations = if last { cx.annotations } else { None };
        let iri_annotations = self.shapes.iri_annotations(predicate);
        let must_be_node = inverted || !last;

        let explicit = match self.explicit_kind(shape_annotations)? {
            Some(kind) => Some(kind),
            None => self.explicit_kind(iri_annotations)?,
        };
        let relationship = match explicit {
            Some(Kind::Property) if must_be_node => {
                return Err(self.error(
                    cx.location,
                    format!(
                        "{} is mapped to a property, but an inverse, repeated or non-final path step needs a relationship",
                        self.graph.compact(predicate.as_str())
                    ),
                ))
            }
            Some(Kind::Property) => false,
            Some(Kind::Relationship) => true,
            None => {
                must_be_node
                    || (last && cx.implies_nodes)
                    || self.schema_prefers_relationship(predicate, cx)?
            }
        };

        if relationship {
            let rel_type = match annotation(shape_annotations, iri_annotations, |a| {
                a.relationship.as_ref()
            }) {
                Some(name) => annotated(name),
                None => {
                    let name = upper_snake(self.local_name(predicate, cx.location)?);
                    let evidence = if self.schema.is_some_and(|s| s.rel_type(&name).is_some()) {
                        Evidence::Schema
                    } else {
                        Evidence::Convention
                    };
                    Resolved { name, evidence }
                }
            };
            self.check_strict(&rel_type, predicate, "relationship type", cx.location)?;
            let declared = shape_annotations
                .and_then(|a| a.direction.as_ref())
                .or_else(|| iri_annotations.and_then(|a| a.direction.as_ref()))
                .map_or(Direction::Out, |d| d.value);
            let direction = if inverted { flip(declared) } else { declared };
            Ok(LpgPath::Relationship {
                rel_type,
                direction,
            })
        } else {
            let key = match annotation(shape_annotations, iri_annotations, |a| a.property.as_ref())
            {
                Some(name) => annotated(name),
                None => {
                    let name = self.local_name(predicate, cx.location)?.to_owned();
                    let evidence = if self.schema_has_property(&name, cx.focus) {
                        Evidence::Schema
                    } else {
                        Evidence::Convention
                    };
                    Resolved { name, evidence }
                }
            };
            self.check_strict(&key, predicate, "property key", cx.location)?;
            Ok(LpgPath::Property(key))
        }
    }

    fn explicit_kind(
        &self,
        annotations: Option<&Annotations>,
    ) -> Result<Option<Kind>, MappingError> {
        let Some(annotations) = annotations else {
            return Ok(None);
        };
        let relationship = annotations.relationship.is_some() || annotations.direction.is_some();
        match (&annotations.property, relationship) {
            (Some(property), true) => Err(self.error(
                property.location,
                "s2c:property cannot be combined with s2c:relationship or s2c:direction".into(),
            )),
            (Some(_), false) => Ok(Some(Kind::Property)),
            (None, true) => Ok(Some(Kind::Relationship)),
            (None, false) => Ok(None),
        }
    }

    fn schema_prefers_relationship(
        &self,
        predicate: NamedNodeRef<'_>,
        cx: &StepContext<'_>,
    ) -> Result<bool, MappingError> {
        let Some(schema) = self.schema else {
            return Ok(false);
        };
        let local = self.local_name(predicate, cx.location)?;
        Ok(schema.rel_type(&upper_snake(local)).is_some()
            && !self.schema_has_property(local, cx.focus))
    }

    /// Whether a focus label (or, when unknown, any node type) declares the property.
    fn schema_has_property(&self, key: &str, focus: &[String]) -> bool {
        let Some(schema) = self.schema else {
            return false;
        };
        if focus.is_empty() {
            schema.node_types.iter().any(|t| t.property(key).is_some())
        } else {
            focus
                .iter()
                .filter_map(|label| schema.node_type(label))
                .any(|t| t.property(key).is_some())
        }
    }

    fn local_name<'i>(
        &self,
        iri: NamedNodeRef<'i>,
        location: SourceLocation,
    ) -> Result<&'i str, MappingError> {
        let local = local_name(iri.as_str());
        if local.is_empty() {
            return Err(self.error(
                location,
                format!(
                    "cannot derive an LPG name from <{}>; add an s2c: annotation",
                    iri.as_str()
                ),
            ));
        }
        Ok(local)
    }

    fn check_strict(
        &self,
        resolved: &Resolved,
        subject: NamedNodeRef<'_>,
        what: &str,
        location: SourceLocation,
    ) -> Result<(), MappingError> {
        if self.options.strict && resolved.evidence == Evidence::Convention {
            return Err(self.error(
                location,
                format!(
                    "{} resolves to {what} `{}` by convention only; add an s2c: annotation or a schema snapshot (--strict)",
                    self.graph.compact(subject.as_str()),
                    resolved.name
                ),
            ));
        }
        Ok(())
    }

    fn error(&self, location: SourceLocation, message: String) -> MappingError {
        MappingError(format!(
            "{}: {message}",
            self.graph.display_location(location)
        ))
    }
}

fn annotated(name: &Located<String>) -> Resolved {
    Resolved {
        name: name.value.clone(),
        evidence: Evidence::Annotation(name.location),
    }
}

/// A string annotation, preferring the shape-level declaration over the IRI-level one.
fn annotation<'x>(
    shape: Option<&'x Annotations>,
    iri: Option<&'x Annotations>,
    get: impl Fn(&'x Annotations) -> Option<&'x Located<String>>,
) -> Option<&'x Located<String>> {
    shape.and_then(&get).or_else(|| iri.and_then(&get))
}

fn implies_node_values(shape: &Shape) -> bool {
    shape.constraints.iter().any(|constraint| {
        matches!(
            constraint.value,
            Constraint::Class(_)
                | Constraint::Node(_)
                | Constraint::NodeKind(
                    NodeKind::Iri | NodeKind::BlankNode | NodeKind::BlankNodeOrIri
                )
        )
    })
}

fn flip(direction: Direction) -> Direction {
    match direction {
        Direction::Out => Direction::In,
        Direction::In => Direction::Out,
    }
}

/// The part of an IRI after its last `#`, `/` or `:`.
pub fn local_name(iri: &str) -> &str {
    match iri.rfind(['#', '/', ':']) {
        Some(i) => &iri[i + 1..],
        None => iri,
    }
}

/// `worksFor` -> `WORKS_FOR`, the relationship-type naming convention.
pub fn upper_snake(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    let mut previous_lower_or_digit = false;
    for c in name.chars() {
        if matches!(c, '-' | ' ' | '_' | '.') {
            out.push('_');
            previous_lower_or_digit = false;
        } else if c.is_uppercase() {
            if previous_lower_or_digit {
                out.push('_');
            }
            out.extend(c.to_uppercase());
            previous_lower_or_digit = false;
        } else {
            out.extend(c.to_uppercase());
            previous_lower_or_digit = c.is_lowercase() || c.is_ascii_digit();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::ShapeId;
    use oxrdf::NamedNode;

    /// Four prefix lines, so the first body line is line 5 of `shapes.ttl`.
    const PREFIXES: &str = "@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix ex: <http://example.org/> .
@prefix s2c: <https://w3id.org/shacl2cypher#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
";

    struct Setup {
        graph: ShapesGraph,
        shapes: Shapes,
        schema: Option<SchemaSnapshot>,
    }

    impl Setup {
        fn new(body: &str, schema: Option<&str>) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("shapes.ttl");
            std::fs::write(&path, format!("{PREFIXES}{body}")).unwrap();
            let graph = ShapesGraph::load(&[path]).unwrap();
            let shapes = Shapes::from_graph(&graph).unwrap();
            let schema = schema.map(|json| SchemaSnapshot::from_json(json).unwrap());
            Setup {
                graph,
                shapes,
                schema,
            }
        }

        fn resolver(&self, strict: bool) -> Resolver<'_> {
            Resolver::new(
                &self.graph,
                &self.shapes,
                self.schema.as_ref(),
                ResolveOptions { strict },
            )
        }

        fn shape(&self, local: &str) -> &Shape {
            self.shapes.get(&ShapeId::from(ex(local))).unwrap()
        }

        /// The `index`-th property shape of a node shape, in source order.
        fn property(&self, node_shape: &str, index: usize) -> &Shape {
            let id = &self.shape(node_shape).properties[index].value;
            self.shapes.get(id).unwrap()
        }
    }

    fn ex(local: &str) -> NamedNode {
        NamedNode::new_unchecked(format!("http://example.org/{local}"))
    }

    fn by(evidence: Evidence, name: &str) -> Resolved {
        Resolved {
            name: name.into(),
            evidence,
        }
    }

    fn convention(name: &str) -> Resolved {
        by(Evidence::Convention, name)
    }

    fn person() -> Vec<String> {
        vec!["Person".into()]
    }

    #[test]
    fn resolves_properties_and_relationships_by_convention() {
        let setup = Setup::new(
            "ex:PersonShape sh:targetClass ex:Person ;
    sh:property [ sh:path ex:firstName ; sh:datatype xsd:string ] ,
                [ sh:path ex:worksFor ; sh:class ex:Company ] .
",
            None,
        );
        let resolver = setup.resolver(false);
        assert_eq!(
            resolver
                .path(setup.property("PersonShape", 0), &person())
                .unwrap(),
            LpgPath::Property(convention("firstName"))
        );
        assert_eq!(
            resolver
                .path(setup.property("PersonShape", 1), &person())
                .unwrap(),
            LpgPath::Relationship {
                rel_type: convention("WORKS_FOR"),
                direction: Direction::Out
            }
        );
    }

    #[test]
    fn shape_annotations_override_iri_annotations() {
        let setup = Setup::new(
            "ex:employs s2c:relationship \"EMPLOYS\" .
ex:PersonShape sh:targetClass ex:Person ;
    sh:property [ sh:path ex:employs ; s2c:relationship \"WORKS_FOR\" ; s2c:direction \"in\" ] .
ex:OrgShape sh:targetClass ex:Company ;
    sh:property [ sh:path ex:employs ; sh:minCount 1 ] .
",
            None,
        );
        let resolver = setup.resolver(false);
        let LpgPath::Relationship {
            rel_type,
            direction,
        } = resolver
            .path(setup.property("PersonShape", 0), &person())
            .unwrap()
        else {
            panic!("expected a relationship");
        };
        assert_eq!(rel_type.name, "WORKS_FOR");
        assert!(matches!(rel_type.evidence, Evidence::Annotation(_)));
        assert_eq!(direction, Direction::In);

        let LpgPath::Relationship {
            rel_type,
            direction,
        } = resolver
            .path(setup.property("OrgShape", 0), &["Company".into()])
            .unwrap()
        else {
            panic!("expected a relationship");
        };
        assert_eq!(rel_type.name, "EMPLOYS");
        assert_eq!(direction, Direction::Out);
    }

    #[test]
    fn schema_disambiguates_relationships_from_properties() {
        let schema = r#"{
            "nodeTypes": [{"name": "Person", "properties": [{"name": "name", "type": "STRING"}]}],
            "relTypes": [{"name": "MANAGER", "endpoints": [{"from": "Person", "to": "Person"}]}]
        }"#;
        let setup = Setup::new(
            "ex:PersonShape sh:targetClass ex:Person ;
    sh:property [ sh:path ex:manager ; sh:minCount 1 ] ,
                [ sh:path ex:name ; sh:minCount 1 ] .
",
            Some(schema),
        );
        let resolver = setup.resolver(true);
        assert_eq!(
            resolver
                .path(setup.property("PersonShape", 0), &person())
                .unwrap(),
            LpgPath::Relationship {
                rel_type: by(Evidence::Schema, "MANAGER"),
                direction: Direction::Out
            }
        );
        assert_eq!(
            resolver
                .path(setup.property("PersonShape", 1), &person())
                .unwrap(),
            LpgPath::Property(by(Evidence::Schema, "name"))
        );
    }

    #[test]
    fn strict_mode_rejects_names_chosen_by_convention() {
        let setup = Setup::new(
            "ex:PersonShape sh:targetClass ex:Person ;
    sh:property [ sh:path ex:nickname ; sh:maxCount 1 ] .
",
            None,
        );
        let shape = setup.property("PersonShape", 0);
        let message = setup
            .resolver(true)
            .path(shape, &person())
            .unwrap_err()
            .to_string();
        assert!(
            message.contains(
                "shapes.ttl:6: ex:nickname resolves to property key `nickname` by convention only"
            ),
            "{message}"
        );
        assert!(message.contains("--strict"), "{message}");
        assert!(setup.resolver(false).path(shape, &person()).is_ok());
    }

    #[test]
    fn class_labels_come_from_annotations_schema_or_convention() {
        let setup = Setup::new(
            "ex:Person s2c:label \"Human\" .
ex:PersonShape sh:targetClass ex:Person .
ex:CompanyShape sh:targetClass ex:Company .
ex:TeamShape sh:targetClass ex:Team ;
    s2c:label \"Squad\" .
ex:MixedShape sh:targetClass ex:A, ex:B ;
    s2c:label \"AB\" .
ex:GhostShape sh:targetClass ex:Ghost .
",
            Some(r#"{"nodeTypes": [{"name": "Company"}]}"#),
        );
        let label = |shape: &str, class: &str, strict: bool| {
            let shape = setup.shape(shape);
            let target = shape
                .targets
                .iter()
                .find(|t| t.value == Target::Class(ex(class)))
                .unwrap();
            setup
                .resolver(strict)
                .class_label(ex(class).as_ref(), Some(shape), target.location)
        };
        let human = label("PersonShape", "Person", true).unwrap();
        assert_eq!(human.name, "Human");
        assert!(matches!(human.evidence, Evidence::Annotation(_)));
        assert_eq!(
            label("CompanyShape", "Company", true).unwrap(),
            by(Evidence::Schema, "Company")
        );
        assert_eq!(label("TeamShape", "Team", true).unwrap().name, "Squad");
        assert!(label("MixedShape", "A", false)
            .unwrap_err()
            .to_string()
            .contains("ambiguous"));
        assert_eq!(
            label("GhostShape", "Ghost", false).unwrap(),
            convention("Ghost")
        );
        assert!(label("GhostShape", "Ghost", true)
            .unwrap_err()
            .to_string()
            .contains("label `Ghost` by convention only"));
    }

    #[test]
    fn inverse_paths_flip_direction_and_reject_properties() {
        let setup = Setup::new(
            "ex:CompanyShape sh:targetClass ex:Company ;
    sh:property [ sh:path [ sh:inversePath ex:worksFor ] ; sh:class ex:Person ] .
ex:PersonShape sh:targetClass ex:Person ;
    sh:property [ sh:path [ sh:inversePath ex:name ] ; s2c:property \"name\" ] .
",
            None,
        );
        let resolver = setup.resolver(false);
        assert_eq!(
            resolver
                .path(setup.property("CompanyShape", 0), &["Company".into()])
                .unwrap(),
            LpgPath::Relationship {
                rel_type: convention("WORKS_FOR"),
                direction: Direction::In
            }
        );
        let message = resolver
            .path(setup.property("PersonShape", 0), &person())
            .unwrap_err()
            .to_string();
        assert!(message.contains("needs a relationship"), "{message}");
    }

    #[test]
    fn lowers_sequences_alternatives_and_repeats() {
        let setup = Setup::new(
            "ex:S sh:targetClass ex:Person ;
    sh:property [ sh:path ( ex:worksFor ex:name ) ; sh:minCount 1 ] ,
                [ sh:path [ sh:alternativePath ( ex:email ex:phone ) ] ; sh:minCount 1 ] ,
                [ sh:path [ sh:oneOrMorePath ex:knows ] ; sh:class ex:Person ] .
",
            None,
        );
        let resolver = setup.resolver(false);
        assert_eq!(
            resolver.path(setup.property("S", 0), &person()).unwrap(),
            LpgPath::Sequence(vec![
                LpgPath::Relationship {
                    rel_type: convention("WORKS_FOR"),
                    direction: Direction::Out
                },
                LpgPath::Property(convention("name")),
            ])
        );
        assert_eq!(
            resolver.path(setup.property("S", 1), &person()).unwrap(),
            LpgPath::Alternative(vec![
                LpgPath::Property(convention("email")),
                LpgPath::Property(convention("phone")),
            ])
        );
        assert_eq!(
            resolver.path(setup.property("S", 2), &person()).unwrap(),
            LpgPath::Repeat {
                path: Box::new(LpgPath::Relationship {
                    rel_type: convention("KNOWS"),
                    direction: Direction::Out
                }),
                min: 1,
                max: None
            }
        );
    }

    #[test]
    fn rejects_properties_in_node_positions_and_mixed_alternatives() {
        let setup = Setup::new(
            "ex:name s2c:property \"name\" .
ex:knows s2c:relationship \"KNOWS\" .
ex:Bad sh:targetClass ex:Person ;
    sh:property [ sh:path ( ex:name ex:worksFor ) ; sh:minCount 1 ] ,
                [ sh:path [ sh:alternativePath ( ex:email ex:knows ) ] ; sh:minCount 1 ] .
",
            None,
        );
        let resolver = setup.resolver(false);
        let sequence = resolver
            .path(setup.property("Bad", 0), &person())
            .unwrap_err()
            .to_string();
        assert!(sequence.contains("needs a relationship"), "{sequence}");
        let alternative = resolver
            .path(setup.property("Bad", 1), &person())
            .unwrap_err()
            .to_string();
        assert!(
            alternative.contains("mixes properties and relationships"),
            "{alternative}"
        );
    }

    #[test]
    fn derives_local_names_and_relationship_types() {
        assert_eq!(local_name("http://example.org/name"), "name");
        assert_eq!(local_name("http://example.org/ns#worksFor"), "worksFor");
        assert_eq!(local_name("urn:acme:employee"), "employee");
        assert_eq!(upper_snake("worksFor"), "WORKS_FOR");
        assert_eq!(upper_snake("line2Item"), "LINE2_ITEM");
        assert_eq!(upper_snake("has-part"), "HAS_PART");
    }
}
