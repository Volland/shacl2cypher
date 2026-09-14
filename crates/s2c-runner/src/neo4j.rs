//! Neo4j backend over Bolt. Every query runs in an explicit transaction that is
//! always rolled back, so nothing a query does can persist.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::Duration;

use neo4rs::{query, BoltMap, BoltType, ConfigBuilder, Graph};
use serde_json::Value as Json;
use shacl2cypher_core::render::{ident, Dialect};
use shacl2cypher_core::schema::{
    Endpoint, NodeType, PropertyDef, RelType, SchemaSnapshot, ValueType,
};
use tokio::runtime::Runtime;

use crate::executor::{ExecError, Executor, Params, Row};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Neo4jConfig {
    pub uri: String,
    pub user: String,
    pub password: String,
    /// Database name; the server default when absent.
    pub database: Option<String>,
}

pub struct Neo4jExecutor {
    config: Neo4jConfig,
    runtime: Runtime,
    graph: Graph,
}

impl Neo4jExecutor {
    /// Connects and checks the connection with `RETURN 1`.
    // @lat: [[architecture#Runner#Backends]]
    pub fn connect(config: Neo4jConfig) -> Result<Self, ExecError> {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|e| ExecError::Connection(e.to_string()))?;
        let graph = runtime.block_on(open(&config))?;
        let mut executor = Neo4jExecutor {
            config,
            runtime,
            graph,
        };
        executor
            .rows("RETURN 1 AS ok", None, None)
            .map_err(|e| ExecError::Connection(format!("{}: {e}", executor.config.uri)))?;
        Ok(executor)
    }

    fn rows(
        &mut self,
        text: &str,
        params: Option<Params>,
        timeout: Option<Duration>,
    ) -> Result<Vec<Row>, ExecError> {
        let mut q = query(text);
        if let Some(params) = params {
            q = q
                .param("limit", params.limit)
                .param("sampleSize", params.sample_size);
        }
        let graph = self.graph.clone();
        let database = self.config.database.clone();
        let work = async move {
            let mut txn = match database {
                Some(database) => graph.start_txn_on(database.as_str()).await,
                None => graph.start_txn().await,
            }
            .map_err(|e| ExecError::Connection(e.to_string()))?;
            let mut stream = txn.execute(q).await.map_err(query_error)?;
            let mut rows = Vec::new();
            while let Some(row) = stream.next(txn.handle()).await.map_err(query_error)? {
                let columns: HashMap<String, BoltType> = row
                    .to()
                    .map_err(|e| ExecError::Query(format!("unreadable row: {e}")))?;
                let row: Row = columns
                    .iter()
                    .map(|(key, value)| (key.clone(), bolt_to_json(value)))
                    .collect();
                rows.push(row);
            }
            txn.rollback()
                .await
                .map_err(|e| ExecError::Connection(e.to_string()))?;
            Ok(rows)
        };
        let outcome = match timeout {
            Some(timeout) => self
                .runtime
                .block_on(async { tokio::time::timeout(timeout, work).await })
                .unwrap_or(Err(ExecError::Timeout)),
            None => self.runtime.block_on(work),
        };
        if outcome == Err(ExecError::Timeout) {
            // The abandoned transaction may leave its connection mid-stream; start over.
            self.graph = self.runtime.block_on(open(&self.config))?;
        }
        outcome
    }
}

async fn open(config: &Neo4jConfig) -> Result<Graph, ExecError> {
    let builder = ConfigBuilder::default()
        .uri(config.uri.as_str())
        .user(config.user.as_str())
        .password(config.password.as_str());
    let builder = match &config.database {
        Some(database) => builder.db(database.as_str()),
        None => builder,
    };
    let built = builder
        .build()
        .map_err(|e| ExecError::Connection(e.to_string()))?;
    Graph::connect(built).map_err(|e| ExecError::Connection(format!("{}: {e}", config.uri)))
}

/// Converts a Bolt value to JSON without serde, whose integer path turns small
/// negative numbers into unsigned ones in neo4rs 0.8. Temporal values become strings.
fn bolt_to_json(value: &BoltType) -> Json {
    match value {
        BoltType::Null(_) => Json::Null,
        BoltType::Boolean(b) => Json::Bool(b.value),
        BoltType::Integer(i) => Json::from(i.value),
        BoltType::Float(f) => serde_json::Number::from_f64(f.value)
            .map_or_else(|| Json::String(f.value.to_string()), Json::Number),
        BoltType::String(s) => Json::String(s.value.clone()),
        BoltType::List(list) => Json::Array(list.value.iter().map(bolt_to_json).collect()),
        BoltType::Map(map) => Json::Object(map_to_json(map)),
        BoltType::Node(node) => {
            let mut object = map_to_json(&node.properties);
            object.insert(
                "_labels".into(),
                Json::Array(node.labels.value.iter().map(bolt_to_json).collect()),
            );
            Json::Object(object)
        }
        BoltType::Relation(rel) => {
            let mut object = map_to_json(&rel.properties);
            object.insert("_type".into(), Json::String(rel.typ.value.clone()));
            Json::Object(object)
        }
        BoltType::Bytes(bytes) => {
            Json::String(bytes.value.iter().map(|b| format!("{b:02x}")).collect())
        }
        other => Json::String(other.to_string()),
    }
}

fn map_to_json(map: &BoltMap) -> Row {
    map.value
        .iter()
        .map(|(key, value)| (key.value.clone(), bolt_to_json(value)))
        .collect()
}

fn query_error(error: neo4rs::Error) -> ExecError {
    ExecError::Query(error.to_string())
}

impl Executor for Neo4jExecutor {
    fn dialect(&self) -> Dialect {
        Dialect::Neo4j
    }

    fn run(
        &mut self,
        query: &str,
        params: Params,
        timeout: Option<Duration>,
    ) -> Result<Vec<Row>, ExecError> {
        self.rows(query, Some(params), timeout)
    }

    /// Labels and relationship types with observed property types; endpoints come
    /// from a distinct scan of each relationship type.
    // @lat: [[architecture#Runner#Schema Dump]]
    fn schema(&mut self) -> Result<SchemaSnapshot, ExecError> {
        let mut nodes: BTreeMap<String, BTreeMap<String, ValueType>> = BTreeMap::new();
        for label in self.strings("CALL db.labels() YIELD label RETURN label AS name")? {
            nodes.entry(label).or_default();
        }
        for row in self.rows(
            "CALL db.schema.nodeTypeProperties() YIELD nodeLabels, propertyName, propertyTypes RETURN nodeLabels, propertyName, propertyTypes",
            None,
            None,
        )? {
            let labels = strings(row.get("nodeLabels"));
            let property = row.get("propertyName").and_then(Json::as_str);
            let value_type = property_type(row.get("propertyTypes"));
            for label in labels {
                let properties = nodes.entry(label).or_default();
                if let Some(property) = property {
                    merge(properties, property, value_type.clone());
                }
            }
        }

        let mut rels: BTreeMap<String, BTreeMap<String, ValueType>> = BTreeMap::new();
        for rel_type in self.strings(
            "CALL db.relationshipTypes() YIELD relationshipType RETURN relationshipType AS name",
        )? {
            rels.entry(rel_type).or_default();
        }
        for row in self.rows(
            "CALL db.schema.relTypeProperties() YIELD relType, propertyName, propertyTypes RETURN relType, propertyName, propertyTypes",
            None,
            None,
        )? {
            let Some(rel_type) = row.get("relType").and_then(Json::as_str) else {
                continue;
            };
            let name = rel_type.trim_start_matches(':').trim_matches('`').replace("``", "`");
            let properties = rels.entry(name).or_default();
            if let Some(property) = row.get("propertyName").and_then(Json::as_str) {
                merge(properties, property, property_type(row.get("propertyTypes")));
            }
        }

        let mut rel_types = Vec::new();
        for (name, properties) in rels {
            let mut endpoints = BTreeSet::new();
            let scan = format!(
                "MATCH (a)-[:{}]->(b) UNWIND labels(a) AS from UNWIND labels(b) AS to RETURN DISTINCT from, to",
                ident(&name)
            );
            for row in self.rows(&scan, None, None)? {
                if let (Some(from), Some(to)) = (
                    row.get("from").and_then(Json::as_str),
                    row.get("to").and_then(Json::as_str),
                ) {
                    endpoints.insert((from.to_owned(), to.to_owned()));
                }
            }
            if endpoints.is_empty() {
                // A type without relationships (or only unlabeled endpoints) has nothing to declare.
                continue;
            }
            rel_types.push(RelType {
                name,
                endpoints: endpoints
                    .into_iter()
                    .map(|(from, to)| Endpoint { from, to })
                    .collect(),
                properties: definitions(properties),
            });
        }
        Ok(SchemaSnapshot {
            node_types: nodes
                .into_iter()
                .map(|(name, properties)| NodeType {
                    name,
                    properties: definitions(properties),
                })
                .collect(),
            rel_types,
        })
    }
}

impl Neo4jExecutor {
    fn strings(&mut self, text: &str) -> Result<Vec<String>, ExecError> {
        Ok(self
            .rows(text, None, None)?
            .iter()
            .filter_map(|row| row.get("name").and_then(Json::as_str).map(str::to_owned))
            .collect())
    }
}

fn strings(value: Option<&Json>) -> Vec<String> {
    value
        .and_then(Json::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

/// Properties seen with different types on different label combinations become `ANY`.
fn merge(properties: &mut BTreeMap<String, ValueType>, name: &str, value_type: ValueType) {
    properties
        .entry(name.to_owned())
        .and_modify(|existing| {
            if *existing != value_type {
                *existing = ValueType::Any;
            }
        })
        .or_insert(value_type);
}

fn definitions(properties: BTreeMap<String, ValueType>) -> Vec<PropertyDef> {
    properties
        .into_iter()
        .map(|(name, value_type)| PropertyDef { name, value_type })
        .collect()
}

/// `propertyTypes` of the schema procedures; several observed types become `ANY`.
fn property_type(types: Option<&Json>) -> ValueType {
    match strings(types).as_slice() {
        [single] => neo4j_type(single),
        _ => ValueType::Any,
    }
}

/// Neo4j schema procedure type name to the neutral snapshot type.
pub fn neo4j_type(name: &str) -> ValueType {
    if let Some(element) = name.strip_suffix("Array").filter(|e| *e != "Byte") {
        return ValueType::List(Box::new(neo4j_type(element)));
    }
    match name {
        "String" => ValueType::String,
        "Long" => ValueType::Int64,
        "Double" => ValueType::Double,
        "Boolean" => ValueType::Boolean,
        "Date" => ValueType::Date,
        "LocalDateTime" => ValueType::LocalDateTime,
        "DateTime" => ValueType::ZonedDateTime,
        "LocalTime" => ValueType::LocalTime,
        "Time" => ValueType::ZonedTime,
        "Duration" => ValueType::Duration,
        "Point" => ValueType::Point,
        "ByteArray" => ValueType::Blob,
        _ => ValueType::Any,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_bolt_values_without_losing_signs() {
        use neo4rs::{BoltInteger, BoltList, BoltString};
        assert_eq!(
            bolt_to_json(&BoltType::Integer(BoltInteger::new(-1))),
            serde_json::json!(-1)
        );
        let list = BoltType::List(BoltList::from(vec![
            BoltType::Integer(BoltInteger::new(-128)),
            BoltType::String(BoltString::new("a")),
        ]));
        assert_eq!(bolt_to_json(&list), serde_json::json!([-128, "a"]));
    }

    #[test]
    fn maps_procedure_types() {
        assert_eq!(neo4j_type("Long"), ValueType::Int64);
        assert_eq!(neo4j_type("DateTime"), ValueType::ZonedDateTime);
        assert_eq!(neo4j_type("ByteArray"), ValueType::Blob);
        assert_eq!(
            neo4j_type("StringArray"),
            ValueType::List(Box::new(ValueType::String))
        );
        assert_eq!(
            property_type(Some(&serde_json::json!(["String", "Long"]))),
            ValueType::Any
        );
    }
}
