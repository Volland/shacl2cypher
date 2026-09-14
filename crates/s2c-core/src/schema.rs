//! Dialect-neutral schema snapshot of the target database.

use std::collections::HashSet;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SchemaSnapshot {
    #[serde(default)]
    pub node_types: Vec<NodeType>,
    #[serde(default)]
    pub rel_types: Vec<RelType>,
}

/// A node label (Neo4j) or node table (LadybugDB).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeType {
    pub name: String,
    #[serde(default)]
    pub properties: Vec<PropertyDef>,
}

/// A relationship type (Neo4j) or rel table (LadybugDB) with its declared endpoints.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RelType {
    pub name: String,
    pub endpoints: Vec<Endpoint>,
    #[serde(default)]
    pub properties: Vec<PropertyDef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Endpoint {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PropertyDef {
    pub name: String,
    #[serde(rename = "type")]
    pub value_type: ValueType,
}

/// Neutral property value type; serialized as e.g. `INT64` or `LIST<STRING>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum ValueType {
    String,
    Int64,
    Int32,
    Int16,
    Int8,
    UInt64,
    UInt32,
    UInt16,
    UInt8,
    Double,
    Float,
    Decimal,
    Boolean,
    Date,
    LocalDateTime,
    ZonedDateTime,
    LocalTime,
    ZonedTime,
    Duration,
    Point,
    Blob,
    /// Mixed or unknown types, e.g. a Neo4j property observed with several types.
    Any,
    List(Box<ValueType>),
}

const SCALAR_TYPES: &[(&str, ValueType)] = &[
    ("STRING", ValueType::String),
    ("INT64", ValueType::Int64),
    ("INT32", ValueType::Int32),
    ("INT16", ValueType::Int16),
    ("INT8", ValueType::Int8),
    ("UINT64", ValueType::UInt64),
    ("UINT32", ValueType::UInt32),
    ("UINT16", ValueType::UInt16),
    ("UINT8", ValueType::UInt8),
    ("DOUBLE", ValueType::Double),
    ("FLOAT", ValueType::Float),
    ("DECIMAL", ValueType::Decimal),
    ("BOOLEAN", ValueType::Boolean),
    ("DATE", ValueType::Date),
    ("LOCAL_DATETIME", ValueType::LocalDateTime),
    ("ZONED_DATETIME", ValueType::ZonedDateTime),
    ("LOCAL_TIME", ValueType::LocalTime),
    ("ZONED_TIME", ValueType::ZonedTime),
    ("DURATION", ValueType::Duration),
    ("POINT", ValueType::Point),
    ("BLOB", ValueType::Blob),
    ("ANY", ValueType::Any),
];

impl FromStr for ValueType {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        let text = text.trim();
        if let Some(inner) = text.strip_prefix("LIST<").and_then(|t| t.strip_suffix('>')) {
            return Ok(ValueType::List(Box::new(inner.parse()?)));
        }
        SCALAR_TYPES
            .iter()
            .find(|(name, _)| *name == text)
            .map(|(_, value_type)| value_type.clone())
            .ok_or_else(|| format!("unknown property type `{text}`"))
    }
}

impl fmt::Display for ValueType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let ValueType::List(inner) = self {
            return write!(f, "LIST<{inner}>");
        }
        let name = SCALAR_TYPES
            .iter()
            .find(|(_, value_type)| value_type == self)
            .map(|(name, _)| *name)
            .expect("every scalar type has a name");
        f.write_str(name)
    }
}

impl Serialize for ValueType {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ValueType {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SchemaError {
    #[error("invalid schema snapshot: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid schema snapshot: {0}")]
    Invalid(String),
}

impl SchemaSnapshot {
    /// Parses and validates a snapshot.
    // @lat: [[dialects#Schema Awareness]]
    pub fn from_json(text: &str) -> Result<Self, SchemaError> {
        let snapshot: SchemaSnapshot = serde_json::from_str(text)?;
        snapshot.validate().map_err(SchemaError::Invalid)?;
        Ok(snapshot)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("schema snapshots always serialize")
    }

    pub fn node_type(&self, name: &str) -> Option<&NodeType> {
        self.node_types.iter().find(|t| t.name == name)
    }

    pub fn rel_type(&self, name: &str) -> Option<&RelType> {
        self.rel_types.iter().find(|t| t.name == name)
    }

    fn validate(&self) -> Result<(), String> {
        let mut node_names = HashSet::new();
        for node_type in &self.node_types {
            check_name("node type", &node_type.name)?;
            if !node_names.insert(node_type.name.as_str()) {
                return Err(format!("duplicate node type `{}`", node_type.name));
            }
            check_properties(
                &format!("node type `{}`", node_type.name),
                &node_type.properties,
            )?;
        }
        let mut rel_names = HashSet::new();
        for rel_type in &self.rel_types {
            check_name("relationship type", &rel_type.name)?;
            if !rel_names.insert(rel_type.name.as_str()) {
                return Err(format!("duplicate relationship type `{}`", rel_type.name));
            }
            if rel_type.endpoints.is_empty() {
                return Err(format!(
                    "relationship type `{}` declares no endpoints",
                    rel_type.name
                ));
            }
            for endpoint in &rel_type.endpoints {
                for end in [&endpoint.from, &endpoint.to] {
                    if !node_names.contains(end.as_str()) {
                        return Err(format!(
                            "relationship type `{}` references undeclared node type `{end}`",
                            rel_type.name
                        ));
                    }
                }
            }
            check_properties(
                &format!("relationship type `{}`", rel_type.name),
                &rel_type.properties,
            )?;
        }
        Ok(())
    }
}

impl NodeType {
    pub fn property(&self, name: &str) -> Option<&PropertyDef> {
        self.properties.iter().find(|p| p.name == name)
    }
}

impl RelType {
    pub fn property(&self, name: &str) -> Option<&PropertyDef> {
        self.properties.iter().find(|p| p.name == name)
    }
}

fn check_name(kind: &str, name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err(format!("{kind} with an empty name"));
    }
    Ok(())
}

fn check_properties(owner: &str, properties: &[PropertyDef]) -> Result<(), String> {
    let mut names = HashSet::new();
    for property in properties {
        check_name(&format!("property of {owner}"), &property.name)?;
        if !names.insert(property.name.as_str()) {
            return Err(format!(
                "{owner} declares property `{}` twice",
                property.name
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SNAPSHOT: &str = r#"{
  "nodeTypes": [
    {"name": "Person", "properties": [
      {"name": "id", "type": "STRING"},
      {"name": "tags", "type": "LIST<STRING>"},
      {"name": "born", "type": "DATE"}
    ]},
    {"name": "Company"}
  ],
  "relTypes": [
    {"name": "WORKS_FOR",
     "endpoints": [{"from": "Person", "to": "Company"}],
     "properties": [{"name": "since", "type": "DATE"}]}
  ]
}"#;

    fn invalid(json: &str) -> String {
        SchemaSnapshot::from_json(json).unwrap_err().to_string()
    }

    #[test]
    fn parses_looks_up_and_round_trips() {
        let snapshot = SchemaSnapshot::from_json(SNAPSHOT).unwrap();
        let person = snapshot.node_type("Person").unwrap();
        assert_eq!(
            person.property("tags").unwrap().value_type,
            ValueType::List(Box::new(ValueType::String))
        );
        assert!(snapshot.node_type("Company").unwrap().properties.is_empty());
        let works_for = snapshot.rel_type("WORKS_FOR").unwrap();
        assert_eq!(works_for.endpoints[0].to, "Company");
        assert_eq!(
            works_for.property("since").unwrap().value_type,
            ValueType::Date
        );
        assert_eq!(
            SchemaSnapshot::from_json(&snapshot.to_json()).unwrap(),
            snapshot
        );
    }

    #[test]
    fn value_types_round_trip_through_their_names() {
        let mut types: Vec<ValueType> = SCALAR_TYPES.iter().map(|(_, t)| t.clone()).collect();
        types.push(ValueType::List(Box::new(ValueType::List(Box::new(
            ValueType::Int64,
        )))));
        for value_type in types {
            assert_eq!(value_type.to_string().parse::<ValueType>(), Ok(value_type));
        }
        assert_eq!(
            "LIST<LIST<INT64>>"
                .parse::<ValueType>()
                .unwrap()
                .to_string(),
            "LIST<LIST<INT64>>"
        );
    }

    #[test]
    fn rejects_unknown_property_types() {
        let message = invalid(
            r#"{"nodeTypes": [{"name": "T", "properties": [{"name": "x", "type": "VARCHAR"}]}]}"#,
        );
        assert!(
            message.contains("unknown property type `VARCHAR`"),
            "{message}"
        );
    }

    #[test]
    fn rejects_malformed_json_and_unknown_fields() {
        assert!(invalid("{").starts_with("invalid schema snapshot"));
        let message = invalid(r#"{"nodeTypez": []}"#);
        assert!(message.contains("nodeTypez"), "{message}");
    }

    #[test]
    fn rejects_structural_problems() {
        let cases = [
            (
                r#"{"nodeTypes": [{"name": "T"}, {"name": "T"}]}"#,
                "duplicate node type `T`",
            ),
            (
                r#"{"nodeTypes": [{"name": "T", "properties": [{"name": "x", "type": "STRING"}, {"name": "x", "type": "INT64"}]}]}"#,
                "declares property `x` twice",
            ),
            (
                r#"{"nodeTypes": [{"name": "A"}], "relTypes": [{"name": "R", "endpoints": [{"from": "A", "to": "B"}]}]}"#,
                "undeclared node type `B`",
            ),
            (
                r#"{"nodeTypes": [{"name": "A"}], "relTypes": [{"name": "R", "endpoints": []}]}"#,
                "declares no endpoints",
            ),
            (r#"{"nodeTypes": [{"name": ""}]}"#, "empty name"),
        ];
        for (json, expected) in cases {
            let message = invalid(json);
            assert!(message.contains(expected), "{message}");
        }
    }
}
