//! The interface every database backend implements.

use std::time::Duration;

use s2c_core::render::Dialect;
use s2c_core::schema::SchemaSnapshot;

/// One result row: column name to JSON value.
pub type Row = serde_json::Map<String, serde_json::Value>;

/// Values for the runtime parameters of generated queries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Params {
    /// `$limit`: maximum detail rows.
    pub limit: i64,
    /// `$sampleSize`: maximum focus samples in a summary row.
    pub sample_size: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExecError {
    #[error("query timed out")]
    Timeout,
    #[error("{0}")]
    Query(String),
    #[error("{0}")]
    Connection(String),
}

/// Runs read-only queries and introspects the schema of one database.
// @lat: [[architecture#Runner]]
pub trait Executor {
    fn dialect(&self) -> Dialect;

    /// Runs one query; `timeout` bounds its execution.
    fn run(
        &mut self,
        query: &str,
        params: Params,
        timeout: Option<Duration>,
    ) -> Result<Vec<Row>, ExecError>;

    /// Produces a schema snapshot of the database (`schema dump`).
    fn schema(&mut self) -> Result<SchemaSnapshot, ExecError>;
}
