use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;

pub const DEFAULT_BASE: &str = "http://example.org/";

/// Property every projected node carries with its fixture id; used as the node key.
pub const ID_PROPERTY: &str = "id";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
// @lat: [[testing#Conformance Fixtures]]
pub struct Fixture {
    /// Shapes file, resolved relative to the fixture file.
    pub shapes: PathBuf,
    /// Namespace for labels, predicates and node IRIs in the RDF projection.
    #[serde(default = "default_base")]
    pub base: String,
    /// Dialects the fixture applies to; all when absent.
    #[serde(default)]
    pub dialects: Option<Vec<String>>,
    pub graph: Graph,
    #[serde(default)]
    pub expect: Vec<Expected>,
    /// Violations an engine is known to report differently, with the reason.
    #[serde(default)]
    pub known_differences: Vec<KnownDifference>,
    /// Why pySHACL cannot validate this fixture at all, e.g. unsupported regex syntax.
    #[serde(default)]
    pub oracle_skip: Option<String>,
    /// Free-form notes, e.g. known disagreements between reference engines.
    #[serde(default)]
    pub notes: Option<String>,
}

fn default_base() -> String {
    DEFAULT_BASE.to_string()
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Graph {
    #[serde(default)]
    pub nodes: Vec<Node>,
    #[serde(default)]
    pub edges: Vec<Edge>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub id: String,
    pub labels: Vec<String>,
    #[serde(default)]
    pub props: BTreeMap<String, Value>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Edge {
    pub from: String,
    pub to: String,
    /// Predicate local name used in the RDF projection.
    pub pred: String,
    /// Relationship type in the LPG projection; UPPER_SNAKE_CASE of `pred` when absent.
    #[serde(rename = "type", default)]
    pub rel_type: Option<String>,
    #[serde(default)]
    pub props: BTreeMap<String, Value>,
}

impl Edge {
    pub fn rel_type(&self) -> String {
        self.rel_type
            .clone()
            .unwrap_or_else(|| upper_snake(&self.pred))
    }
}

/// Property value. Lists are multi-valued; `null` and empty lists are empty value sets.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Date { date: String },
    List(Vec<Value>),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Expected {
    pub rule: String,
    /// Fixture node id, or `from->to` for a relationship focus.
    pub focus: String,
    /// Rule ids of failed inner constraints; checked on LPG engines when given.
    #[serde(default)]
    pub details: Option<Vec<String>>,
}

/// Engines a fixture can run on.
pub const ENGINES: &[&str] = &["pyshacl", "neo4j", "ladybug"];

/// A violation one engine reports differently from `expect`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KnownDifference {
    pub engine: String,
    pub rule: String,
    pub focus: String,
    pub reason: String,
}

impl Fixture {
    pub fn load(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let text = std::fs::read_to_string(path)?;
        let mut fixture: Fixture =
            serde_yaml_ng::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        if fixture.shapes.is_relative() {
            if let Some(dir) = path.parent() {
                fixture.shapes = dir.join(&fixture.shapes);
            }
        }
        fixture
            .validate()
            .map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(fixture)
    }

    pub fn runs_on(&self, dialect: &str) -> bool {
        self.dialects
            .as_ref()
            .is_none_or(|dialects| dialects.iter().any(|d| d == dialect))
    }

    fn validate(&self) -> Result<(), String> {
        let mut ids = BTreeSet::new();
        for node in &self.graph.nodes {
            if !ids.insert(node.id.as_str()) {
                return Err(format!("duplicate node id `{}`", node.id));
            }
            if node.props.contains_key(ID_PROPERTY) {
                return Err(format!(
                    "node `{}` must not set `{ID_PROPERTY}`; the fixture id is projected into it",
                    node.id
                ));
            }
        }
        for edge in &self.graph.edges {
            for end in [&edge.from, &edge.to] {
                if !ids.contains(end.as_str()) {
                    return Err(format!(
                        "edge `{}` references unknown node `{end}`",
                        edge.pred
                    ));
                }
            }
        }
        let known_focus = |focus: &str| match focus.split_once("->") {
            Some((from, to)) => ids.contains(from) && ids.contains(to),
            None => ids.contains(focus),
        };
        for expected in &self.expect {
            if !known_focus(&expected.focus) {
                return Err(format!(
                    "expected violation of `{}` has unknown focus `{}`",
                    expected.rule, expected.focus
                ));
            }
        }
        for difference in &self.known_differences {
            if !ENGINES.contains(&difference.engine.as_str()) {
                return Err(format!(
                    "known difference names unknown engine `{}`",
                    difference.engine
                ));
            }
            if !known_focus(&difference.focus) {
                return Err(format!(
                    "known difference has unknown focus `{}`",
                    difference.focus
                ));
            }
        }
        Ok(())
    }
}

/// `worksFor` -> `WORKS_FOR`, matching the relationship-type naming convention.
pub fn upper_snake(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 4);
    let mut prev_lower_or_digit = false;
    for c in name.chars() {
        if c == '-' || c == ' ' || c == '_' {
            out.push('_');
            prev_lower_or_digit = false;
        } else if c.is_uppercase() {
            if prev_lower_or_digit {
                out.push('_');
            }
            out.extend(c.to_uppercase());
            prev_lower_or_digit = false;
        } else {
            out.extend(c.to_uppercase());
            prev_lower_or_digit = c.is_lowercase() || c.is_ascii_digit();
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upper_snake_converts_camel_case() {
        assert_eq!(upper_snake("worksFor"), "WORKS_FOR");
        assert_eq!(upper_snake("knows"), "KNOWS");
        assert_eq!(upper_snake("hasHTTPUrl"), "HAS_HTTPURL");
        assert_eq!(upper_snake("line2Item"), "LINE2_ITEM");
    }

    #[test]
    fn parses_values_by_shape() {
        let props: BTreeMap<String, Value> = serde_yaml_ng::from_str(
            "a: null\nb: true\nc: 3\nd: 1.5\ne: x\nf: {date: '2020-01-01'}\ng: [1, null]",
        )
        .unwrap();
        assert_eq!(props["a"], Value::Null);
        assert_eq!(props["b"], Value::Bool(true));
        assert_eq!(props["c"], Value::Int(3));
        assert_eq!(props["d"], Value::Float(1.5));
        assert_eq!(props["e"], Value::Str("x".into()));
        assert_eq!(
            props["f"],
            Value::Date {
                date: "2020-01-01".into()
            }
        );
        assert_eq!(props["g"], Value::List(vec![Value::Int(1), Value::Null]));
    }
}
