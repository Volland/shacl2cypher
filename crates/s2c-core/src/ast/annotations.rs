//! `s2c:` annotations: LPG mapping hints attached to shapes and IRIs.

use std::collections::HashMap;

use oxrdf::{NamedNode, NamedNodeRef, NamedOrBlankNode, NamedOrBlankNodeRef};

use super::{Builder, Located, Shape};
use crate::load::SourceLocation;

pub const NAMESPACE: &str = "https://w3id.org/shacl2cypher#";

/// shacl2cypher vocabulary terms.
pub mod s2c {
    use oxrdf::NamedNodeRef;

    macro_rules! terms {
        ($($name:ident = $local:literal),* $(,)?) => {
            $(pub const $name: NamedNodeRef<'static> =
                NamedNodeRef::new_unchecked(concat!("https://w3id.org/shacl2cypher#", $local));)*
        };
    }

    terms! {
        LABEL = "label", PROPERTY = "property", RELATIONSHIP = "relationship",
        DIRECTION = "direction", KEY = "key", DATATYPE = "datatype", COLLECTION = "collection",
        IRI_AS_STRING = "iriAsString", TARGET_RELATIONSHIP = "targetRelationship", NAME = "name",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Out,
    In,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Collection {
    List,
    Scalar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IriAsString {
    Local,
    Full,
}

/// Mapping hints declared on one subject; `None` means not declared.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Annotations {
    pub label: Option<Located<String>>,
    pub property: Option<Located<String>>,
    pub relationship: Option<Located<String>>,
    pub direction: Option<Located<Direction>>,
    pub key: Option<Located<String>>,
    pub datatype: Option<Located<String>>,
    pub collection: Option<Located<Collection>>,
    pub iri_as_string: Option<Located<IriAsString>>,
    pub name: Option<Located<String>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SubjectKind {
    NodeShape,
    PropertyShape,
    Iri,
}

impl SubjectKind {
    fn describe(self) -> &'static str {
        match self {
            SubjectKind::NodeShape => "a node shape",
            SubjectKind::PropertyShape => "a property shape",
            SubjectKind::Iri => "a class or predicate IRI",
        }
    }
}

/// Every annotation and the kinds of subject it may be declared on.
const USAGE: &[(NamedNodeRef<'static>, &[SubjectKind])] = &[
    (s2c::LABEL, &[SubjectKind::NodeShape, SubjectKind::Iri]),
    (
        s2c::PROPERTY,
        &[SubjectKind::PropertyShape, SubjectKind::Iri],
    ),
    (
        s2c::RELATIONSHIP,
        &[SubjectKind::PropertyShape, SubjectKind::Iri],
    ),
    (
        s2c::DIRECTION,
        &[SubjectKind::PropertyShape, SubjectKind::Iri],
    ),
    (s2c::KEY, &[SubjectKind::NodeShape]),
    (
        s2c::DATATYPE,
        &[SubjectKind::PropertyShape, SubjectKind::Iri],
    ),
    (
        s2c::COLLECTION,
        &[SubjectKind::PropertyShape, SubjectKind::Iri],
    ),
    (s2c::IRI_AS_STRING, &[SubjectKind::PropertyShape]),
    (s2c::TARGET_RELATIONSHIP, &[SubjectKind::NodeShape]),
    (
        s2c::NAME,
        &[SubjectKind::NodeShape, SubjectKind::PropertyShape],
    ),
];

impl<'g> Builder<'g> {
    /// Reads every annotation of a subject; placement is validated separately.
    // @lat: [[mapping#Annotation Vocabulary]]
    pub(super) fn annotations(&mut self, s: NamedOrBlankNodeRef<'_>) -> Annotations {
        Annotations {
            label: self.annotation_string(s, s2c::LABEL),
            property: self.annotation_string(s, s2c::PROPERTY),
            relationship: self.annotation_string(s, s2c::RELATIONSHIP),
            direction: self.annotation_enum(
                s,
                s2c::DIRECTION,
                &[("out", Direction::Out), ("in", Direction::In)],
            ),
            key: self.annotation_string(s, s2c::KEY),
            datatype: self.annotation_string(s, s2c::DATATYPE),
            collection: self.annotation_enum(
                s,
                s2c::COLLECTION,
                &[("list", Collection::List), ("scalar", Collection::Scalar)],
            ),
            iri_as_string: self.annotation_enum(
                s,
                s2c::IRI_AS_STRING,
                &[("local", IriAsString::Local), ("full", IriAsString::Full)],
            ),
            name: self.annotation_string(s, s2c::NAME),
        }
    }

    pub(super) fn target_relationships(
        &mut self,
        s: NamedOrBlankNodeRef<'_>,
    ) -> Vec<Located<String>> {
        self.values(s, s2c::TARGET_RELATIONSHIP)
            .into_iter()
            .filter_map(|(term, location)| {
                self.string(s2c::TARGET_RELATIONSHIP, term, location)
                    .map(|value| Located { value, location })
            })
            .collect()
    }

    /// Validates every `s2c:` triple (known annotation, allowed subject) and reads
    /// the annotations of class and predicate IRIs that are not themselves shapes.
    pub(super) fn iri_annotations(&mut self, shapes: &[Shape]) -> HashMap<NamedNode, Annotations> {
        let kinds: HashMap<NamedOrBlankNode, SubjectKind> = shapes
            .iter()
            .map(|shape| {
                let kind = if shape.path.is_some() {
                    SubjectKind::PropertyShape
                } else {
                    SubjectKind::NodeShape
                };
                (shape.id.clone(), kind)
            })
            .collect();

        let graph = self.graph();
        let mut triples: Vec<(NamedOrBlankNode, NamedNode, SourceLocation)> = graph
            .iter()
            .filter(|t| t.predicate.as_str().starts_with(NAMESPACE))
            .map(|t| {
                (
                    t.subject.into_owned(),
                    t.predicate.into_owned(),
                    self.location(t),
                )
            })
            .collect();
        triples.sort_by(|a, b| a.2.cmp(&b.2).then_with(|| a.1.as_str().cmp(b.1.as_str())));

        let mut annotated_iris = Vec::new();
        for (subject, predicate, location) in triples {
            let name = self.compact(predicate.as_str());
            let Some((_, allowed)) = USAGE.iter().find(|(p, _)| *p == predicate.as_ref()) else {
                self.problem(location, format!("unknown annotation {name}"));
                continue;
            };
            let kind = match (&subject, kinds.get(&subject)) {
                (_, Some(kind)) => *kind,
                (NamedOrBlankNode::NamedNode(iri), None) => {
                    annotated_iris.push(iri.clone());
                    SubjectKind::Iri
                }
                (NamedOrBlankNode::BlankNode(_), None) => {
                    self.problem(
                        location,
                        format!("{name} must annotate a shape, a class IRI or a predicate IRI"),
                    );
                    continue;
                }
            };
            if !allowed.contains(&kind) {
                let applies_to: Vec<&str> = allowed.iter().map(|k| k.describe()).collect();
                self.problem(
                    location,
                    format!(
                        "{name} is not allowed on {}; it applies to {}",
                        kind.describe(),
                        applies_to.join(" or ")
                    ),
                );
            }
        }

        annotated_iris.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        annotated_iris.dedup();
        annotated_iris
            .into_iter()
            .map(|iri| {
                let annotations = self.annotations(iri.as_ref().into());
                (iri, annotations)
            })
            .collect()
    }

    fn annotation_string(
        &mut self,
        s: NamedOrBlankNodeRef<'_>,
        predicate: NamedNodeRef<'_>,
    ) -> Option<Located<String>> {
        let (term, location) = self.single(s, predicate)?;
        let value = self.string(predicate, term, location)?;
        if value.is_empty() {
            let name = self.compact(predicate.as_str());
            self.problem(location, format!("{name} must not be empty"));
            return None;
        }
        Some(Located { value, location })
    }

    fn annotation_enum<T: Copy>(
        &mut self,
        s: NamedOrBlankNodeRef<'_>,
        predicate: NamedNodeRef<'_>,
        allowed: &[(&str, T)],
    ) -> Option<Located<T>> {
        let (term, location) = self.single(s, predicate)?;
        let value = self.string(predicate, term, location)?;
        if let Some((_, variant)) = allowed.iter().find(|(text, _)| *text == value) {
            return Some(Located {
                value: *variant,
                location,
            });
        }
        let name = self.compact(predicate.as_str());
        let expected: Vec<String> = allowed
            .iter()
            .map(|(text, _)| format!("\"{text}\""))
            .collect();
        self.problem(
            location,
            format!(
                "{name} must be {}, found \"{value}\"",
                expected.join(" or ")
            ),
        );
        None
    }
}

#[cfg(test)]
mod tests {
    use super::super::{ShapeId, Shapes, Target};
    use super::*;
    use crate::ast::AstErrors;
    use crate::load::ShapesGraph;

    const PREFIXES: &str = "@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix ex: <http://example.org/> .
@prefix s2c: <https://w3id.org/shacl2cypher#> .
";

    fn parse_files(files: &[(&str, &str)]) -> Result<Shapes, AstErrors> {
        let dir = tempfile::tempdir().unwrap();
        let paths: Vec<_> = files
            .iter()
            .map(|(name, body)| {
                let path = dir.path().join(name);
                std::fs::write(&path, format!("{PREFIXES}{body}")).unwrap();
                path
            })
            .collect();
        Shapes::from_graph(&ShapesGraph::load(&paths).unwrap())
    }

    fn parse(body: &str) -> Result<Shapes, AstErrors> {
        parse_files(&[("shapes.ttl", body)])
    }

    fn ex(local: &str) -> NamedNode {
        NamedNode::new_unchecked(format!("http://example.org/{local}"))
    }

    fn value<T: Clone>(annotation: &Option<Located<T>>) -> Option<T> {
        annotation.as_ref().map(|a| a.value.clone())
    }

    #[test]
    fn parses_shape_and_iri_annotations() {
        let shapes = parse(
            "ex:PersonShape a sh:NodeShape ;
    sh:targetClass ex:Person ;
    s2c:label \"Human\" ;
    s2c:key \"email\" ;
    s2c:name \"person\" ;
    sh:property [
        sh:path ex:employer ;
        sh:class ex:Company ;
        s2c:relationship \"WORKS_FOR\" ;
        s2c:direction \"in\" ;
        s2c:collection \"list\" ;
        s2c:iriAsString \"full\" ;
    ] .
ex:Company s2c:label \"Organisation\" .
ex:createdAt s2c:property \"created_at\" ;
    s2c:datatype \"LOCAL DATETIME\" .
",
        )
        .unwrap();
        let person = shapes.get(&ShapeId::from(ex("PersonShape"))).unwrap();
        assert_eq!(value(&person.annotations.label).as_deref(), Some("Human"));
        assert_eq!(value(&person.annotations.key).as_deref(), Some("email"));
        assert_eq!(value(&person.annotations.name).as_deref(), Some("person"));

        let employer = shapes.get(&person.properties[0].value).unwrap();
        let annotations = &employer.annotations;
        assert_eq!(
            value(&annotations.relationship).as_deref(),
            Some("WORKS_FOR")
        );
        assert_eq!(value(&annotations.direction), Some(Direction::In));
        assert_eq!(value(&annotations.collection), Some(Collection::List));
        assert_eq!(value(&annotations.iri_as_string), Some(IriAsString::Full));

        let company = shapes.iri_annotations(ex("Company").as_ref()).unwrap();
        assert_eq!(value(&company.label).as_deref(), Some("Organisation"));
        let created_at = shapes.iri_annotations(ex("createdAt").as_ref()).unwrap();
        assert_eq!(value(&created_at.property).as_deref(), Some("created_at"));
        assert_eq!(
            value(&created_at.datatype).as_deref(),
            Some("LOCAL DATETIME")
        );
        assert!(shapes.iri_annotations(ex("Person").as_ref()).is_none());
    }

    #[test]
    fn relationship_target_declares_a_shape() {
        let shapes = parse("ex:LikesShape s2c:targetRelationship \"LIKES\" .\n").unwrap();
        let likes = shapes.get(&ShapeId::from(ex("LikesShape"))).unwrap();
        let targets: Vec<Target> = likes.targets.iter().map(|t| t.value.clone()).collect();
        assert_eq!(targets, vec![Target::Relationship("LIKES".into())]);
    }

    #[test]
    fn rejects_invalid_enumerated_values() {
        let err = parse("ex:P sh:path ex:p ;\n    s2c:direction \"sideways\" .\n").unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains("s2c:direction must be \"out\" or \"in\", found \"sideways\""),
            "{message}"
        );
    }

    #[test]
    fn rejects_unknown_annotations() {
        let err = parse("ex:S a sh:NodeShape ;\n    s2c:lable \"Person\" .\n").unwrap_err();
        assert!(
            err.to_string().contains("unknown annotation s2c:lable"),
            "{err}"
        );
    }

    #[test]
    fn rejects_annotations_on_the_wrong_subject() {
        let err = parse(
            "ex:S a sh:NodeShape ;
    s2c:direction \"out\" ;
    sh:property [ sh:path ex:p ; s2c:key \"id\" ] .
ex:Company s2c:key \"id\" .
",
        )
        .unwrap_err();
        assert_eq!(err.messages.len(), 3, "{err}");
        assert!(err
            .messages
            .iter()
            .all(|message| message.contains("is not allowed on")));
        assert!(err
            .to_string()
            .contains("s2c:key is not allowed on a property shape"));
        assert!(err
            .to_string()
            .contains("s2c:direction is not allowed on a node shape"));
        assert!(err
            .to_string()
            .contains("s2c:key is not allowed on a class or predicate IRI"));
    }

    // @lat: [[tests#Loading#Conflicting Settings List Every Location]]
    #[test]
    fn conflicting_single_valued_settings_across_files_list_both_locations() {
        let cases = [
            (
                "ex:S a sh:NodeShape ;\n    sh:severity sh:Warning .\n",
                "ex:S sh:severity sh:Violation .\n",
            ),
            (
                "ex:S a sh:NodeShape ;\n    sh:deactivated true .\n",
                "ex:S sh:deactivated false .\n",
            ),
            (
                "ex:S a sh:NodeShape ;\n    s2c:key \"id\" .\n",
                "ex:S s2c:key \"email\" .\n",
            ),
            (
                "ex:S a sh:NodeShape ;\n    s2c:name \"a\" .\n",
                "ex:S s2c:name \"b\" .\n",
            ),
            (
                "ex:S a sh:NodeShape ;\n    s2c:label \"A\" .\n",
                "ex:S s2c:label \"B\" .\n",
            ),
            (
                "ex:P sh:path ex:p ;\n    s2c:direction \"out\" .\n",
                "ex:P s2c:direction \"in\" .\n",
            ),
            (
                "ex:p s2c:datatype \"STRING\" .\n",
                "ex:p s2c:datatype \"INT64\" .\n",
            ),
            (
                "ex:p s2c:collection \"list\" .\n",
                "ex:p s2c:collection \"scalar\" .\n",
            ),
        ];
        for (a, b) in cases {
            let err = parse_files(&[("a.ttl", a), ("b.ttl", b)]).unwrap_err();
            let message = err.to_string();
            assert!(message.contains("expected at most one"), "{message}");
            assert!(message.contains("a.ttl:"), "{message}");
            assert!(message.contains("b.ttl:"), "{message}");
        }
    }

    #[test]
    fn identical_settings_in_two_files_do_not_conflict() {
        let body = "ex:S a sh:NodeShape ;\n    s2c:key \"id\" .\n";
        let shapes = parse_files(&[("a.ttl", body), ("b.ttl", body)]).unwrap();
        let s = shapes.get(&ShapeId::from(ex("S"))).unwrap();
        assert_eq!(value(&s.annotations.key).as_deref(), Some("id"));
    }
}
