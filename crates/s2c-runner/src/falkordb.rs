//! FalkorDB backend over the Redis protocol. Queries only ever run through
//! `GRAPH.RO_QUERY`, which refuses writes and never creates a missing graph.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use falkordb::{
    FalkorClientBuilder, FalkorConnectionInfo, FalkorDBError, FalkorSyncClient, FalkorValue,
};
use serde_json::Value as Json;
use shacl2cypher_core::render::Dialect;
use shacl2cypher_core::schema::{
    Endpoint, NodeType, PropertyDef, RelType, SchemaSnapshot, ValueType,
};

use crate::executor::{ExecError, Executor, Params, Row};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FalkorDbConfig {
    /// `redis://[user:password@]host:port`; credentials may also come from
    /// `FALKORDB_USERNAME` and `FALKORDB_PASSWORD`.
    pub url: String,
    /// Name of an existing graph.
    pub graph: String,
}

pub struct FalkorDbExecutor {
    client: FalkorSyncClient,
    graph: String,
}

impl FalkorDbExecutor {
    /// Connects and checks that the graph exists, so a typo never validates an empty graph.
    // @lat: [[architecture#Runner#Backends]]
    pub fn connect(config: &FalkorDbConfig) -> Result<Self, ExecError> {
        let shown = redacted(&config.url);
        let url = with_env_credentials(
            &config.url,
            std::env::var("FALKORDB_USERNAME").ok().as_deref(),
            std::env::var("FALKORDB_PASSWORD").ok().as_deref(),
        )?;
        let info: FalkorConnectionInfo = url.as_str().try_into().map_err(|e: FalkorDBError| {
            ExecError::Connection(format!("{shown}: {}", message(&e)))
        })?;
        let client = FalkorClientBuilder::new()
            .with_connection_info(info)
            .build()
            .map_err(|e| ExecError::Connection(format!("{shown}: {}", message(&e))))?;
        let graphs = client
            .list_graphs()
            .map_err(|e| ExecError::Connection(format!("{shown}: {}", message(&e))))?;
        if !graphs.contains(&config.graph) {
            return Err(ExecError::Connection(format!(
                "graph `{}` does not exist on {shown}",
                config.graph
            )));
        }
        Ok(FalkorDbExecutor {
            client,
            graph: config.graph.clone(),
        })
    }

    fn rows(
        &mut self,
        query: &str,
        params: Option<Params>,
        timeout: Option<Duration>,
    ) -> Result<Vec<Row>, ExecError> {
        let mut graph = self.client.select_graph(&self.graph);
        let mut builder = graph.ro_query(query);
        if let Some(params) = params {
            // The amd64 server build returns no rows for some limits above u32::MAX
            // (including the i64::MAX sent for "no limit"), so parameters stay in i32.
            // @lat: [[dialects#Dialect Backends#FalkorDB]]
            let cap = i64::from(i32::MAX);
            builder = builder
                .with_param("limit", params.limit.min(cap))
                .with_param("sampleSize", params.sample_size.min(cap));
        }
        // Without an explicit TIMEOUT the server applies its own default (1000 ms).
        let milliseconds = timeout.map_or(0, |timeout| {
            i64::try_from(timeout.as_millis())
                .unwrap_or(i64::MAX)
                .max(1)
        });
        let result = builder
            .with_timeout(milliseconds)
            .execute()
            .map_err(|e| query_error(&e))?;
        let header = result.header.clone();
        let mut rows = Vec::new();
        for row in result.data {
            let values = row
                .map_err(|e| ExecError::Query(format!("unreadable row: {}", message(&e))))?
                .into_values();
            rows.push(
                header
                    .iter()
                    .cloned()
                    .zip(values.iter().map(to_json))
                    .collect(),
            );
        }
        Ok(rows)
    }

    fn strings(&mut self, query: &str, column: &str) -> Result<Vec<String>, ExecError> {
        Ok(self
            .rows(query, None, None)?
            .iter()
            .filter_map(|row| row.get(column).and_then(Json::as_str).map(str::to_owned))
            .collect())
    }

    /// Observed `typeOf` names per owner (label or relationship type) and property.
    fn property_types(
        &mut self,
        query: &str,
        owner_column: &str,
    ) -> Result<BTreeMap<String, BTreeMap<String, ValueType>>, ExecError> {
        let mut owners: BTreeMap<String, BTreeMap<String, ValueType>> = BTreeMap::new();
        for row in self.rows(query, None, None)? {
            let (Some(owner), Some(key)) = (
                row.get(owner_column).and_then(Json::as_str),
                row.get("key").and_then(Json::as_str),
            ) else {
                continue;
            };
            let types = strings(row.get("types"));
            let elements: BTreeSet<String> = row
                .get("elements")
                .and_then(Json::as_array)
                .into_iter()
                .flatten()
                .flat_map(|list| strings(Some(list)))
                .collect();
            owners
                .entry(owner.to_owned())
                .or_default()
                .insert(key.to_owned(), falkordb_type(&types, &elements));
        }
        Ok(owners)
    }
}

impl Executor for FalkorDbExecutor {
    fn dialect(&self) -> Dialect {
        Dialect::FalkorDb
    }

    fn run(
        &mut self,
        query: &str,
        params: Params,
        timeout: Option<Duration>,
    ) -> Result<Vec<Row>, ExecError> {
        self.rows(query, Some(params), timeout)
    }

    /// Labels and relationship types with the value types observed on every node and
    /// relationship; FalkorDB has no schema procedures, so the dump scans the graph.
    // @lat: [[architecture#Runner#Schema Dump]]
    fn schema(&mut self) -> Result<SchemaSnapshot, ExecError> {
        let mut nodes = self.property_types(
            "MATCH (n) UNWIND labels(n) AS label UNWIND keys(n) AS key RETURN label, key, collect(DISTINCT typeOf(n[key])) AS types, collect(DISTINCT [e IN CASE WHEN typeOf(n[key]) = 'List' THEN n[key] ELSE [] END | typeOf(e)]) AS elements",
            "label",
        )?;
        for label in self.strings("CALL db.labels() YIELD label RETURN label", "label")? {
            nodes.entry(label).or_default();
        }
        let mut rels = self.property_types(
            "MATCH ()-[r]->() UNWIND keys(r) AS key RETURN type(r) AS relType, key, collect(DISTINCT typeOf(r[key])) AS types, collect(DISTINCT [e IN CASE WHEN typeOf(r[key]) = 'List' THEN r[key] ELSE [] END | typeOf(e)]) AS elements",
            "relType",
        )?;
        for rel_type in self.strings(
            "CALL db.relationshipTypes() YIELD relationshipType RETURN relationshipType",
            "relationshipType",
        )? {
            rels.entry(rel_type).or_default();
        }
        let mut endpoints: BTreeMap<String, BTreeSet<(String, String)>> = BTreeMap::new();
        for row in self.rows(
            "MATCH (a)-[r]->(b) UNWIND labels(a) AS source UNWIND labels(b) AS target RETURN DISTINCT type(r) AS relType, source, target",
            None,
            None,
        )? {
            if let (Some(rel_type), Some(source), Some(target)) = (
                row.get("relType").and_then(Json::as_str),
                row.get("source").and_then(Json::as_str),
                row.get("target").and_then(Json::as_str),
            ) {
                endpoints
                    .entry(rel_type.to_owned())
                    .or_default()
                    .insert((source.to_owned(), target.to_owned()));
            }
        }
        Ok(SchemaSnapshot {
            node_types: nodes
                .into_iter()
                .map(|(name, properties)| NodeType {
                    name,
                    properties: definitions(properties),
                })
                .collect(),
            rel_types: rels
                .into_iter()
                // A type without labeled endpoints has nothing to declare.
                .filter_map(|(name, properties)| {
                    let pairs = endpoints.remove(&name)?;
                    Some(RelType {
                        name,
                        endpoints: pairs
                            .into_iter()
                            .map(|(from, to)| Endpoint { from, to })
                            .collect(),
                        properties: definitions(properties),
                    })
                })
                .collect(),
        })
    }
}

/// The crate prefixes server errors with its own text and drops their first word
/// (`Query timed out` arrives as `timed out`); keep only the server's message.
fn message(error: &FalkorDBError) -> String {
    match error {
        FalkorDBError::RedisError(text) | FalkorDBError::RedisParsingError(text) => text.clone(),
        other => other.to_string(),
    }
}

// @lat: [[dialects#Dialect Backends#FalkorDB]]
fn query_error(error: &FalkorDBError) -> ExecError {
    let text = message(error);
    match error {
        FalkorDBError::RedisError(_) if text.trim_end().ends_with("timed out") => {
            ExecError::Timeout
        }
        FalkorDBError::ConnectionDown
        | FalkorDBError::NoConnection
        | FalkorDBError::EmptyConnection => ExecError::Connection(text),
        _ => ExecError::Query(text),
    }
}

/// Adds `FALKORDB_USERNAME`/`FALKORDB_PASSWORD` to a URL without credentials, and
/// rejects TLS URLs, which this build does not support.
fn with_env_credentials(
    url: &str,
    username: Option<&str>,
    password: Option<&str>,
) -> Result<String, ExecError> {
    let (scheme, rest) = url.split_once("://").unwrap_or(("redis", url));
    match scheme {
        "redis" | "falkor" => {}
        "rediss" | "falkors" => {
            return Err(ExecError::Connection(format!(
                "{}: TLS connections are not supported yet; use redis://",
                redacted(url)
            )))
        }
        other => {
            return Err(ExecError::Connection(format!(
                "{}: unsupported URL scheme `{other}`; use redis://host:port",
                redacted(url)
            )))
        }
    }
    let authority = rest.split('/').next().unwrap_or_default();
    if authority.contains('@') || (username.is_none() && password.is_none()) {
        return Ok(format!("redis://{rest}"));
    }
    let credentials = format!(
        "{}:{}",
        percent_encode(username.unwrap_or_default()),
        percent_encode(password.unwrap_or_default())
    );
    Ok(format!("redis://{credentials}@{rest}"))
}

/// The URL with any password replaced by `***`, for messages.
fn redacted(url: &str) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_owned();
    };
    let (authority, path) = rest.split_at(rest.find('/').unwrap_or(rest.len()));
    match authority.rsplit_once('@') {
        Some((credentials, host)) => {
            let user = credentials.split(':').next().unwrap_or_default();
            format!("{scheme}://{user}:***@{host}{path}")
        }
        None => url.to_owned(),
    }
}

fn percent_encode(text: &str) -> String {
    text.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// Converts a FalkorDB value to JSON explicitly; temporal values become ISO 8601 strings.
fn to_json(value: &FalkorValue) -> Json {
    match value {
        FalkorValue::None => Json::Null,
        FalkorValue::Bool(b) => Json::Bool(*b),
        FalkorValue::I64(i) => Json::from(*i),
        FalkorValue::F64(f) => serde_json::Number::from_f64(*f)
            .map_or_else(|| Json::String(f.to_string()), Json::Number),
        FalkorValue::String(s) => Json::String(s.clone()),
        FalkorValue::Array(items) => Json::Array(items.iter().map(to_json).collect()),
        FalkorValue::Map(map) => Json::Object(
            map.iter()
                .map(|(key, value)| (key.clone(), to_json(value)))
                .collect(),
        ),
        FalkorValue::Node(node) => {
            let mut object = properties(&node.properties);
            object.insert(
                "_labels".into(),
                Json::Array(node.labels.iter().cloned().map(Json::String).collect()),
            );
            Json::Object(object)
        }
        FalkorValue::Edge(edge) => {
            let mut object = properties(&edge.properties);
            object.insert("_type".into(), Json::String(edge.relationship_type.clone()));
            Json::Object(object)
        }
        FalkorValue::Vec32(vector) => Json::Array(
            vector
                .values
                .iter()
                .map(|v| {
                    serde_json::Number::from_f64(f64::from(*v)).map_or(Json::Null, Json::Number)
                })
                .collect(),
        ),
        FalkorValue::Point(point) => serde_json::json!({
            "latitude": point.latitude,
            "longitude": point.longitude,
        }),
        FalkorValue::Date(date) => Json::String(iso_date(date.seconds().get())),
        FalkorValue::DateTime(datetime) => Json::String(iso_datetime(datetime.seconds().get())),
        FalkorValue::Time(time) => Json::String(iso_time(time.seconds().get())),
        FalkorValue::Duration(duration) => Json::String(iso_duration(duration.seconds().get())),
        FalkorValue::Unparseable(text) => Json::String(text.clone()),
        other => Json::String(format!("{other:?}")),
    }
}

fn properties(map: &std::collections::HashMap<String, FalkorValue>) -> Row {
    map.iter()
        .map(|(key, value)| (key.clone(), to_json(value)))
        .collect()
}

/// `YYYY-MM-DD` of a day count since the Unix epoch (proleptic Gregorian calendar).
fn civil_date(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

fn clock(seconds: i64) -> String {
    let seconds = seconds.rem_euclid(86_400);
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}

/// FalkorDB dates are seconds since the Unix epoch at UTC midnight.
fn iso_date(seconds: i64) -> String {
    civil_date(seconds.div_euclid(86_400))
}

fn iso_datetime(seconds: i64) -> String {
    format!("{}T{}", iso_date(seconds), clock(seconds))
}

/// FalkorDB local times are seconds since 1900-01-01; only the time of day matters.
fn iso_time(seconds: i64) -> String {
    clock(seconds)
}

fn iso_duration(seconds: i64) -> String {
    if seconds < 0 {
        format!("-PT{}S", seconds.unsigned_abs())
    } else {
        format!("PT{seconds}S")
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

/// `typeOf` names observed for one property to the neutral snapshot type: several
/// types, and types the snapshot cannot describe, become `ANY`.
pub fn falkordb_type(types: &[String], elements: &BTreeSet<String>) -> ValueType {
    let [single] = types else {
        return ValueType::Any;
    };
    match single.as_str() {
        "String" => ValueType::String,
        "Integer" => ValueType::Int64,
        "Float" => ValueType::Double,
        "Boolean" => ValueType::Boolean,
        "Date" => ValueType::Date,
        "Datetime" => ValueType::LocalDateTime,
        "Time" => ValueType::LocalTime,
        "Duration" => ValueType::Duration,
        "Point" => ValueType::Point,
        "List" => {
            let element = match elements.iter().collect::<Vec<_>>().as_slice() {
                [only] if only.as_str() != "List" => {
                    falkordb_type(std::slice::from_ref(*only), &BTreeSet::new())
                }
                _ => ValueType::Any,
            };
            ValueType::List(Box::new(element))
        }
        _ => ValueType::Any,
    }
}

fn definitions(properties: BTreeMap<String, ValueType>) -> Vec<PropertyDef> {
    properties
        .into_iter()
        .map(|(name, value_type)| PropertyDef { name, value_type })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_temporal_scalars_as_iso_strings() {
        assert_eq!(iso_date(1_577_836_800), "2020-01-01");
        assert_eq!(iso_date(-301_276_800), "1960-06-15");
        assert_eq!(iso_datetime(1_577_872_800), "2020-01-01T10:00:00");
        assert_eq!(iso_time(-2_208_950_985), "10:30:15");
        assert_eq!(iso_duration(93_600), "PT93600S");
        assert_eq!(iso_duration(-5), "-PT5S");
        assert_eq!(civil_date(11_016), "2000-02-29");
    }

    #[test]
    fn converts_values_to_json() {
        let node = FalkorValue::Node(falkordb::Node {
            entity_id: 1,
            labels: vec!["Person".into()],
            properties: [("id".to_owned(), FalkorValue::I64(-1))].into(),
        });
        assert_eq!(
            to_json(&node),
            serde_json::json!({"id": -1, "_labels": ["Person"]})
        );
        let list = FalkorValue::Array(vec![FalkorValue::None, FalkorValue::F64(1.5)]);
        assert_eq!(to_json(&list), serde_json::json!([null, 1.5]));
    }

    #[test]
    fn maps_observed_types() {
        let one = |name: &str| vec![name.to_owned()];
        let none = BTreeSet::new();
        assert_eq!(falkordb_type(&one("Integer"), &none), ValueType::Int64);
        assert_eq!(
            falkordb_type(&one("Datetime"), &none),
            ValueType::LocalDateTime
        );
        assert_eq!(
            falkordb_type(&["String".into(), "Integer".into()], &none),
            ValueType::Any
        );
        assert_eq!(
            falkordb_type(&one("List"), &BTreeSet::from(["String".to_owned()])),
            ValueType::List(Box::new(ValueType::String))
        );
        assert_eq!(
            falkordb_type(&one("List"), &none),
            ValueType::List(Box::new(ValueType::Any))
        );
        assert_eq!(falkordb_type(&one("Vectorf32"), &none), ValueType::Any);
    }

    #[test]
    fn builds_urls_with_credentials_and_rejects_tls() {
        assert_eq!(
            with_env_credentials("redis://localhost:6379", None, None).unwrap(),
            "redis://localhost:6379"
        );
        assert_eq!(
            with_env_credentials("localhost:6379", Some("app"), Some("p@ss:w")).unwrap(),
            "redis://app:p%40ss%3Aw@localhost:6379"
        );
        assert_eq!(
            with_env_credentials("redis://u:x@h:1", Some("other"), Some("y")).unwrap(),
            "redis://u:x@h:1"
        );
        let tls = with_env_credentials("rediss://h:1", None, None).unwrap_err();
        assert!(tls.to_string().contains("TLS"), "{tls}");
        assert_eq!(redacted("redis://u:secret@h:1/0"), "redis://u:***@h:1/0");
        assert_eq!(redacted("redis://h:1"), "redis://h:1");
    }

    #[test]
    fn maps_server_timeouts() {
        assert_eq!(
            query_error(&FalkorDBError::RedisError("timed out".into())),
            ExecError::Timeout
        );
        assert_eq!(
            query_error(&FalkorDBError::RedisError(
                "mismatch: expected List, String, or Null but was Integer".into()
            )),
            ExecError::Query("mismatch: expected List, String, or Null but was Integer".into())
        );
    }
}
