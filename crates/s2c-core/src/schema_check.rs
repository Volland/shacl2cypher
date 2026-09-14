//! Static comparison of resolved shapes with the schema snapshot: schema mismatch
//! diagnostics, and constraints the declared schema already decides.

use std::collections::BTreeSet;

use crate::ast::Direction;
use crate::datatypes::{ColumnStatus, DatatypeCheck};
use crate::load::{ShapesGraph, SourceLocation};
use crate::mapping::LpgPath;
use crate::schema::SchemaSnapshot;

pub const SCHEMA_MISMATCH: &str = "s2c:SchemaMismatch";

/// A problem found at compile time instead of by a query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StaticDiagnostic {
    pub code: &'static str,
    pub location: SourceLocation,
    pub message: String,
}

/// What the schema says about one constraint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SchemaStatus {
    /// A query must check the constraint.
    Check,
    /// The enforced schema guarantees the constraint; no query is generated.
    Guaranteed,
    /// The enforced schema contradicts the constraint.
    Contradicted(String),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct SchemaMismatchError(pub String);

pub struct SchemaChecker<'a> {
    schema: &'a SchemaSnapshot,
    /// The database enforces the declared schema (LadybugDB), so columns and
    /// endpoints are guarantees rather than observations.
    enforced: bool,
}

impl<'a> SchemaChecker<'a> {
    pub fn new(schema: &'a SchemaSnapshot, enforced: bool) -> Self {
        SchemaChecker { schema, enforced }
    }

    /// A mismatch when none of a target's labels (the class and its subclasses) exists.
    // @lat: [[output#Static Diagnostics]]
    pub fn check_target(
        &self,
        class: &str,
        labels: &[String],
        location: SourceLocation,
    ) -> Option<StaticDiagnostic> {
        if labels
            .iter()
            .any(|label| self.schema.node_type(label).is_some())
        {
            return None;
        }
        Some(mismatch(
            location,
            format!(
                "{class} maps to {}, which the schema snapshot does not declare",
                describe(labels)
            ),
        ))
    }

    /// Missing property keys, relationship types and endpoints along a resolved path.
    /// Steps whose focus labels are unknown are not checked.
    pub fn check_path(
        &self,
        path: &LpgPath,
        focus: &[String],
        location: SourceLocation,
    ) -> Vec<StaticDiagnostic> {
        let mut diagnostics = Vec::new();
        let focus: BTreeSet<String> = focus.iter().cloned().collect();
        self.walk(path, &focus, location, &mut diagnostics);
        diagnostics
    }

    /// Checks `path` from `focus` and returns the labels it reaches (empty if unknown).
    fn walk(
        &self,
        path: &LpgPath,
        focus: &BTreeSet<String>,
        location: SourceLocation,
        out: &mut Vec<StaticDiagnostic>,
    ) -> BTreeSet<String> {
        match path {
            LpgPath::Property(key) => {
                let declared = focus
                    .iter()
                    .filter_map(|label| self.schema.node_type(label))
                    .any(|node_type| node_type.property(&key.name).is_some());
                if !focus.is_empty() && !declared {
                    out.push(mismatch(
                        location,
                        format!(
                            "property `{}` is not declared on {}",
                            key.name,
                            describe(focus)
                        ),
                    ));
                }
                BTreeSet::new()
            }
            LpgPath::Relationship {
                rel_type,
                direction,
            } => {
                let Some(rel) = self.schema.rel_type(&rel_type.name) else {
                    out.push(mismatch(
                        location,
                        format!(
                            "relationship type `{}` is not declared in the schema snapshot",
                            rel_type.name
                        ),
                    ));
                    return BTreeSet::new();
                };
                let reached: BTreeSet<String> = rel
                    .endpoints
                    .iter()
                    .filter_map(|endpoint| {
                        let (near, far) = match direction {
                            Direction::Out => (&endpoint.from, &endpoint.to),
                            Direction::In => (&endpoint.to, &endpoint.from),
                        };
                        (focus.is_empty() || focus.contains(near)).then(|| far.clone())
                    })
                    .collect();
                if !focus.is_empty() && reached.is_empty() {
                    let way = match direction {
                        Direction::Out => "outgoing",
                        Direction::In => "incoming",
                    };
                    out.push(mismatch(
                        location,
                        format!(
                            "relationship type `{}` has no {way} endpoint on {}",
                            rel_type.name,
                            describe(focus)
                        ),
                    ));
                }
                reached
            }
            LpgPath::Sequence(steps) => {
                let mut current = focus.clone();
                for step in steps {
                    current = self.walk(step, &current, location, out);
                }
                current
            }
            LpgPath::Alternative(options) => options
                .iter()
                .flat_map(|option| self.walk(option, focus, location, out))
                .collect(),
            LpgPath::Repeat { path, min, .. } => {
                // Only the first hop is reported; later hops explore reachable labels.
                let mut later_hops = Vec::new();
                let mut reached = if *min == 0 {
                    focus.clone()
                } else {
                    BTreeSet::new()
                };
                let mut frontier = focus.clone();
                let mut first = true;
                while first || !frontier.is_empty() {
                    let sink = if first { &mut *out } else { &mut later_hops };
                    let next = self.walk(path, &frontier, location, sink);
                    frontier = next.difference(&reached).cloned().collect();
                    reached.extend(frontier.iter().cloned());
                    if focus.is_empty() {
                        break;
                    }
                    first = false;
                }
                reached
            }
        }
    }

    /// Whether declared columns decide an `sh:datatype` constraint on a property path.
    pub fn datatype_status(
        &self,
        path: &LpgPath,
        focus: &[String],
        check: &DatatypeCheck,
    ) -> SchemaStatus {
        let LpgPath::Property(key) = path else {
            return SchemaStatus::Check;
        };
        if !self.enforced || focus.is_empty() {
            return SchemaStatus::Check;
        }
        let mut contradicted = Vec::new();
        let mut all_guaranteed = true;
        let mut declared_columns = 0;
        for label in focus {
            let Some(node_type) = self.schema.node_type(label) else {
                return SchemaStatus::Check;
            };
            // A label without the column holds no values, which satisfies the constraint.
            let Some(property) = node_type.property(&key.name) else {
                continue;
            };
            declared_columns += 1;
            match check.column_status(&property.value_type) {
                ColumnStatus::Guaranteed => {}
                ColumnStatus::Contradicted => {
                    all_guaranteed = false;
                    contradicted.push(format!("`{label}.{}` {}", key.name, property.value_type));
                }
                ColumnStatus::NeedsRangeCheck | ColumnStatus::NeedsCheck => all_guaranteed = false,
            }
        }
        if declared_columns > 0 && contradicted.len() == declared_columns {
            SchemaStatus::Contradicted(format!(
                "sh:datatype contradicts the declared column type of {}",
                contradicted.join(", ")
            ))
        } else if all_guaranteed {
            SchemaStatus::Guaranteed
        } else {
            SchemaStatus::Check
        }
    }

    /// Whether rel table endpoints decide an `sh:class` constraint on a single
    /// relationship path. `class_labels` are the class and its subclasses.
    pub fn class_status(
        &self,
        path: &LpgPath,
        focus: &[String],
        class_labels: &[String],
    ) -> SchemaStatus {
        let LpgPath::Relationship {
            rel_type,
            direction,
        } = path
        else {
            return SchemaStatus::Check;
        };
        if !self.enforced || focus.is_empty() {
            return SchemaStatus::Check;
        }
        let Some(rel) = self.schema.rel_type(&rel_type.name) else {
            return SchemaStatus::Check;
        };
        let reached: BTreeSet<&str> = rel
            .endpoints
            .iter()
            .filter_map(|endpoint| {
                let (near, far) = match direction {
                    Direction::Out => (&endpoint.from, &endpoint.to),
                    Direction::In => (&endpoint.to, &endpoint.from),
                };
                focus.contains(near).then_some(far.as_str())
            })
            .collect();
        let is_class = |label: &&str| class_labels.iter().any(|c| c == label);
        if reached.is_empty() {
            SchemaStatus::Check
        } else if reached.iter().all(is_class) {
            SchemaStatus::Guaranteed
        } else if !reached.iter().any(is_class) {
            let reached: Vec<String> = reached.iter().map(|l| (*l).to_owned()).collect();
            SchemaStatus::Contradicted(format!(
                "sh:class can never hold: relationship type `{}` only reaches {}",
                rel_type.name,
                describe(&reached)
            ))
        } else {
            SchemaStatus::Check
        }
    }
}

/// `--fail-on-schema-mismatch`: any static diagnostic becomes a compile error.
pub fn fail_on_mismatch(
    diagnostics: &[StaticDiagnostic],
    graph: &ShapesGraph,
) -> Result<(), SchemaMismatchError> {
    if diagnostics.is_empty() {
        return Ok(());
    }
    let lines: Vec<String> = diagnostics
        .iter()
        .map(|d| {
            format!(
                "{}: {}: {}",
                graph.display_location(d.location),
                d.code,
                d.message
            )
        })
        .collect();
    Err(SchemaMismatchError(format!(
        "schema mismatches (--fail-on-schema-mismatch):\n{}",
        lines.join("\n")
    )))
}

fn mismatch(location: SourceLocation, message: String) -> StaticDiagnostic {
    StaticDiagnostic {
        code: SCHEMA_MISMATCH,
        location,
        message,
    }
}

fn describe<'l>(labels: impl IntoIterator<Item = &'l String>) -> String {
    let quoted: Vec<String> = labels.into_iter().map(|l| format!("`{l}`")).collect();
    match quoted.as_slice() {
        [single] => format!("label {single}"),
        _ => format!("labels {}", quoted.join(", ")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapping::{Evidence, Resolved};
    use oxrdf::NamedNode;

    const SCHEMA: &str = r#"{
        "nodeTypes": [
            {"name": "Person", "properties": [
                {"name": "name", "type": "STRING"},
                {"name": "age", "type": "INT64"}
            ]},
            {"name": "Company", "properties": [{"name": "name", "type": "STRING"}]},
            {"name": "Team"}
        ],
        "relTypes": [
            {"name": "WORKS_FOR", "endpoints": [{"from": "Person", "to": "Company"}]},
            {"name": "KNOWS", "endpoints": [{"from": "Person", "to": "Person"}]}
        ]
    }"#;

    fn schema() -> SchemaSnapshot {
        SchemaSnapshot::from_json(SCHEMA).unwrap()
    }

    const AT: SourceLocation = SourceLocation { file: 0, line: 3 };

    fn labels(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| (*n).to_owned()).collect()
    }

    fn resolved(name: &str) -> Resolved {
        Resolved {
            name: name.into(),
            evidence: Evidence::Convention,
        }
    }

    fn property(name: &str) -> LpgPath {
        LpgPath::Property(resolved(name))
    }

    fn relationship(name: &str, direction: Direction) -> LpgPath {
        LpgPath::Relationship {
            rel_type: resolved(name),
            direction,
        }
    }

    fn messages(diagnostics: &[StaticDiagnostic]) -> Vec<&str> {
        diagnostics.iter().map(|d| d.message.as_str()).collect()
    }

    fn xsd(local: &str) -> DatatypeCheck {
        let iri = NamedNode::new_unchecked(format!("http://www.w3.org/2001/XMLSchema#{local}"));
        DatatypeCheck::for_datatype(iri.as_ref(), None).unwrap()
    }

    #[test]
    fn reports_target_classes_missing_from_the_schema() {
        let schema = schema();
        let checker = SchemaChecker::new(&schema, true);
        assert_eq!(
            checker.check_target("ex:Person", &labels(&["Person"]), AT),
            None
        );
        let diagnostic = checker
            .check_target("ex:Ghost", &labels(&["Ghost"]), AT)
            .unwrap();
        assert_eq!(diagnostic.code, SCHEMA_MISMATCH);
        assert_eq!(
            diagnostic.message,
            "ex:Ghost maps to label `Ghost`, which the schema snapshot does not declare"
        );
        assert_eq!(
            checker.check_target("ex:Ghost", &labels(&["Ghost", "Person"]), AT),
            None
        );
    }

    #[test]
    fn reports_missing_properties_relationships_and_endpoints() {
        let schema = schema();
        let checker = SchemaChecker::new(&schema, false);
        let person = labels(&["Person"]);
        assert_eq!(
            messages(&checker.check_path(&property("nickname"), &person, AT)),
            vec!["property `nickname` is not declared on label `Person`"]
        );
        assert_eq!(
            messages(&checker.check_path(&relationship("MANAGES", Direction::Out), &person, AT)),
            vec!["relationship type `MANAGES` is not declared in the schema snapshot"]
        );
        assert_eq!(
            messages(&checker.check_path(
                &relationship("WORKS_FOR", Direction::Out),
                &labels(&["Company"]),
                AT
            )),
            vec!["relationship type `WORKS_FOR` has no outgoing endpoint on label `Company`"]
        );
        assert!(checker
            .check_path(
                &relationship("WORKS_FOR", Direction::In),
                &labels(&["Company"]),
                AT
            )
            .is_empty());
        assert!(checker
            .check_path(&property("nickname"), &[], AT)
            .is_empty());
    }

    #[test]
    fn follows_sequences_and_repeats_through_declared_endpoints() {
        let schema = schema();
        let checker = SchemaChecker::new(&schema, true);
        let person = labels(&["Person"]);
        let works_for = relationship("WORKS_FOR", Direction::Out);
        assert!(checker
            .check_path(
                &LpgPath::Sequence(vec![works_for.clone(), property("name")]),
                &person,
                AT
            )
            .is_empty());
        assert_eq!(
            messages(&checker.check_path(
                &LpgPath::Sequence(vec![works_for.clone(), property("age")]),
                &person,
                AT
            )),
            vec!["property `age` is not declared on label `Company`"]
        );
        for inner in [relationship("KNOWS", Direction::Out), works_for] {
            let repeat = LpgPath::Repeat {
                path: Box::new(inner),
                min: 1,
                max: None,
            };
            assert!(checker.check_path(&repeat, &person, AT).is_empty());
        }
    }

    #[test]
    fn enforced_columns_decide_datatype_constraints() {
        let schema = schema();
        let enforced = SchemaChecker::new(&schema, true);
        let person = labels(&["Person"]);
        assert_eq!(
            enforced.datatype_status(&property("name"), &person, &xsd("string")),
            SchemaStatus::Guaranteed
        );
        let SchemaStatus::Contradicted(message) =
            enforced.datatype_status(&property("name"), &person, &xsd("integer"))
        else {
            panic!("expected a contradiction");
        };
        assert!(message.contains("`Person.name` STRING"), "{message}");
        assert_eq!(
            enforced.datatype_status(&property("age"), &person, &xsd("short")),
            SchemaStatus::Check
        );
        assert_eq!(
            enforced.datatype_status(
                &property("name"),
                &labels(&["Person", "Team"]),
                &xsd("string")
            ),
            SchemaStatus::Guaranteed
        );
        assert_eq!(
            enforced.datatype_status(
                &LpgPath::Sequence(vec![
                    relationship("WORKS_FOR", Direction::Out),
                    property("name")
                ]),
                &person,
                &xsd("string")
            ),
            SchemaStatus::Check
        );
        assert_eq!(
            SchemaChecker::new(&schema, false).datatype_status(
                &property("name"),
                &person,
                &xsd("string")
            ),
            SchemaStatus::Check
        );
    }

    #[test]
    fn enforced_endpoints_decide_class_constraints() {
        let schema = schema();
        let enforced = SchemaChecker::new(&schema, true);
        let person = labels(&["Person"]);
        let works_for = relationship("WORKS_FOR", Direction::Out);
        assert_eq!(
            enforced.class_status(&works_for, &person, &labels(&["Company"])),
            SchemaStatus::Guaranteed
        );
        assert!(matches!(
            enforced.class_status(&works_for, &person, &labels(&["Person"])),
            SchemaStatus::Contradicted(_)
        ));
        assert_eq!(
            enforced.class_status(
                &relationship("KNOWS", Direction::Out),
                &person,
                &labels(&["Employee", "Person"])
            ),
            SchemaStatus::Guaranteed
        );
        assert_eq!(
            SchemaChecker::new(&schema, false).class_status(
                &works_for,
                &person,
                &labels(&["Company"])
            ),
            SchemaStatus::Check
        );
    }

    #[test]
    fn fail_on_mismatch_turns_diagnostics_into_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shapes.ttl");
        std::fs::write(&path, "").unwrap();
        let graph = ShapesGraph::load(&[path]).unwrap();
        assert!(fail_on_mismatch(&[], &graph).is_ok());
        let diagnostic = mismatch(AT, "property `x` is not declared on label `T`".into());
        let message = fail_on_mismatch(&[diagnostic], &graph)
            .unwrap_err()
            .to_string();
        assert!(message.contains("--fail-on-schema-mismatch"), "{message}");
        assert!(
            message.contains("shapes.ttl:3: s2c:SchemaMismatch: property `x`"),
            "{message}"
        );
    }
}
