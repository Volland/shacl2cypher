//! Embedded LadybugDB backend. Databases are always opened read-only.

use std::path::Path;
use std::time::Duration;

use lbug::{Connection, Database, SystemConfig, Value};
use s2c_core::render::{quote, Dialect};
use s2c_core::schema::{Endpoint, NodeType, PropertyDef, RelType, SchemaSnapshot, ValueType};
use serde_json::Value as Json;

use crate::executor::{ExecError, Executor, Params, Row};

pub struct LadybugExecutor {
    db: Database,
}

impl LadybugExecutor {
    /// Opens an existing database file read-only.
    // @lat: [[architecture#Runner#Backends]]
    pub fn open(path: &Path) -> Result<Self, ExecError> {
        if !path.exists() {
            return Err(ExecError::Connection(format!(
                "{}: no such LadybugDB database",
                path.display()
            )));
        }
        let db = Database::new(path, SystemConfig::default().read_only(true))
            .map_err(|e| ExecError::Connection(format!("{}: {e}", path.display())))?;
        Ok(LadybugExecutor { db })
    }

    fn rows(&self, query: &str) -> Result<Vec<Row>, ExecError> {
        let conn = Connection::new(&self.db).map_err(|e| ExecError::Connection(e.to_string()))?;
        let result = conn.query(query).map_err(query_error)?;
        let columns = result.get_column_names();
        Ok(result.map(|row| to_row(&columns, row)).collect())
    }
}

impl Executor for LadybugExecutor {
    fn dialect(&self) -> Dialect {
        Dialect::Ladybug
    }

    fn run(
        &mut self,
        query: &str,
        params: Params,
        timeout: Option<Duration>,
    ) -> Result<Vec<Row>, ExecError> {
        let conn = Connection::new(&self.db).map_err(|e| ExecError::Connection(e.to_string()))?;
        if let Some(timeout) = timeout {
            conn.set_query_timeout(
                u64::try_from(timeout.as_millis())
                    .unwrap_or(u64::MAX)
                    .max(1),
            );
        }
        let mut statement = conn.prepare(query).map_err(query_error)?;
        // Prepared statements reject parameters the query does not declare.
        let mut values = Vec::new();
        if query.contains("$limit") {
            values.push(("limit", Value::Int64(params.limit)));
        }
        if query.contains("$sampleSize") {
            values.push(("sampleSize", Value::Int64(params.sample_size)));
        }
        let result = conn.execute(&mut statement, values).map_err(query_error)?;
        let columns = result.get_column_names();
        Ok(result.map(|row| to_row(&columns, row)).collect())
    }

    /// Node and rel tables with their declared columns and FROM/TO pairs.
    // @lat: [[architecture#Runner#Schema Dump]]
    fn schema(&mut self) -> Result<SchemaSnapshot, ExecError> {
        let mut snapshot = SchemaSnapshot::default();
        let mut tables: Vec<(String, String)> = self
            .rows("CALL show_tables() RETURN *")?
            .iter()
            .filter_map(|row| Some((text(row, "name")?, text(row, "type")?)))
            .collect();
        tables.sort();
        for (name, kind) in tables {
            let properties = self.properties(&name)?;
            match kind.as_str() {
                "NODE" => snapshot.node_types.push(NodeType { name, properties }),
                "REL" => {
                    let mut endpoints: Vec<Endpoint> = self
                        .rows(&format!("CALL show_connection({}) RETURN *", quote(&name)))?
                        .iter()
                        .filter_map(|row| {
                            Some(Endpoint {
                                from: text(row, "source table name")?,
                                to: text(row, "destination table name")?,
                            })
                        })
                        .collect();
                    endpoints.sort_by(|a, b| (&a.from, &a.to).cmp(&(&b.from, &b.to)));
                    snapshot.rel_types.push(RelType {
                        name,
                        endpoints,
                        properties,
                    });
                }
                _ => {}
            }
        }
        Ok(snapshot)
    }
}

impl LadybugExecutor {
    fn properties(&self, table: &str) -> Result<Vec<PropertyDef>, ExecError> {
        Ok(self
            .rows(&format!("CALL table_info({}) RETURN *", quote(table)))?
            .iter()
            .filter_map(|row| {
                Some(PropertyDef {
                    name: text(row, "name")?,
                    value_type: column_type(&text(row, "type")?),
                })
            })
            .collect())
    }
}

fn text(row: &Row, column: &str) -> Option<String> {
    row.get(column).and_then(Json::as_str).map(str::to_owned)
}

/// LadybugDB column type to the neutral snapshot type.
pub fn column_type(name: &str) -> ValueType {
    let name = name.trim();
    if let Some(open) = name.rfind('[').filter(|_| name.ends_with(']')) {
        return ValueType::List(Box::new(column_type(&name[..open])));
    }
    let base = name.split('(').next().unwrap_or(name).trim();
    match base {
        "STRING" => ValueType::String,
        "INT64" | "SERIAL" => ValueType::Int64,
        "INT32" => ValueType::Int32,
        "INT16" => ValueType::Int16,
        "INT8" => ValueType::Int8,
        "UINT64" => ValueType::UInt64,
        "UINT32" => ValueType::UInt32,
        "UINT16" => ValueType::UInt16,
        "UINT8" => ValueType::UInt8,
        "DOUBLE" => ValueType::Double,
        "FLOAT" => ValueType::Float,
        "DECIMAL" => ValueType::Decimal,
        "BOOL" | "BOOLEAN" => ValueType::Boolean,
        "DATE" => ValueType::Date,
        "TIMESTAMP" | "TIMESTAMP_NS" | "TIMESTAMP_MS" | "TIMESTAMP_SEC" => ValueType::LocalDateTime,
        "TIMESTAMP_TZ" => ValueType::ZonedDateTime,
        "INTERVAL" => ValueType::Duration,
        "BLOB" => ValueType::Blob,
        _ => ValueType::Any,
    }
}

fn query_error(error: lbug::Error) -> ExecError {
    let message = error.to_string();
    if message.contains("nterrupt") || message.to_lowercase().contains("timeout") {
        ExecError::Timeout
    } else {
        ExecError::Query(message)
    }
}

fn to_row(columns: &[String], values: Vec<Value>) -> Row {
    columns
        .iter()
        .cloned()
        .zip(values.into_iter().map(to_json))
        .collect()
}

fn number(value: f64) -> Json {
    serde_json::Number::from_f64(value)
        .map_or_else(|| Json::String(value.to_string()), Json::Number)
}

fn properties(label_key: &str, label: &str, properties: &[(String, Value)]) -> Json {
    let mut object = serde_json::Map::new();
    object.insert(label_key.into(), Json::String(label.into()));
    for (key, value) in properties {
        object.insert(key.clone(), to_json(value.clone()));
    }
    Json::Object(object)
}

/// Converts a LadybugDB value to JSON; temporal and other exotic values become strings.
pub fn to_json(value: Value) -> Json {
    match value {
        Value::Null(_) => Json::Null,
        Value::Bool(b) => Json::Bool(b),
        Value::Int64(x) => x.into(),
        Value::Int32(x) => x.into(),
        Value::Int16(x) => x.into(),
        Value::Int8(x) => x.into(),
        Value::UInt64(x) => x.into(),
        Value::UInt32(x) => x.into(),
        Value::UInt16(x) => x.into(),
        Value::UInt8(x) => x.into(),
        Value::Int128(x) => {
            i64::try_from(x).map_or_else(|_| Json::String(x.to_string()), Json::from)
        }
        Value::Double(x) => number(x),
        Value::Float(x) => number(f64::from(x)),
        Value::String(s) => Json::String(s),
        Value::Json(json) => json,
        Value::Blob(bytes) => Json::String(bytes.iter().map(|b| format!("{b:02x}")).collect()),
        Value::List(_, items) | Value::Array(_, items) => {
            Json::Array(items.into_iter().map(to_json).collect())
        }
        Value::Struct(fields) => Json::Object(
            fields
                .into_iter()
                .map(|(key, value)| (key, to_json(value)))
                .collect(),
        ),
        Value::Node(node) => properties("_label", node.get_label_name(), node.get_properties()),
        Value::Rel(rel) => properties("_type", rel.get_label_name(), rel.get_properties()),
        Value::Map(_, entries) => Json::Array(
            entries
                .into_iter()
                .map(|(key, value)| Json::Array(vec![to_json(key), to_json(value)]))
                .collect(),
        ),
        Value::Union { value, .. } => to_json(*value),
        other => Json::String(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_column_types() {
        assert_eq!(column_type("STRING"), ValueType::String);
        assert_eq!(column_type("BOOL"), ValueType::Boolean);
        assert_eq!(column_type("TIMESTAMP"), ValueType::LocalDateTime);
        assert_eq!(column_type("DECIMAL(18, 3)"), ValueType::Decimal);
        assert_eq!(
            column_type("INT64[]"),
            ValueType::List(Box::new(ValueType::Int64))
        );
        assert_eq!(
            column_type("STRING[3]"),
            ValueType::List(Box::new(ValueType::String))
        );
        assert_eq!(column_type("STRUCT(a INT64)"), ValueType::Any);
    }
}
