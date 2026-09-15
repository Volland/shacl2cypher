//! The native addon behind the `shacl2cypher` npm package.
//!
//! `index.js` wraps these exports: it checks arguments, adds `write` to
//! compilations and rethrows errors as classes. Values cross the boundary as JSON
//! text, and errors as JSON messages `{"s2c": kind, "message": …, "errors": […]}`.

use std::path::PathBuf;
use std::sync::Arc;

use napi::bindgen_prelude::*;
use napi_derive::napi;
use serde::Deserialize;
use serde_json::{json, Value};
use shacl2cypher_core::compile::{compile as compile_request, CompileOptions, Manifest};
use shacl2cypher_core::load::{Document, RdfFormat};
use shacl2cypher_core::render::Dialect;
use shacl2cypher_core::schema::SchemaSnapshot;
use shacl2cypher_runner::backend::{BackendConfig, BackendError};
use shacl2cypher_runner::executor::ExecError;
use shacl2cypher_runner::report;
use shacl2cypher_runner::session::{self, Sources};
use shacl2cypher_runner::validate::{self, Report, ValidateOptions};
use shacl2cypher_runner::worker::{DatabaseWorker, Target, WorkerError};

fn failure(kind: &str, message: impl Into<String>) -> Error {
    Error::from_reason(json!({ "s2c": kind, "message": message.into() }).to_string())
}

fn compile_failure(errors: Vec<String>) -> Error {
    let payload = json!({ "s2c": "compile", "message": errors.join("\n"), "errors": errors });
    Error::from_reason(payload.to_string())
}

fn invalid(option: &str, value: &str, allowed: &str) -> Error {
    failure(
        "type",
        format!("{option} must be one of {allowed} (got {value:?})"),
    )
}

fn backend_failure(error: BackendError) -> Error {
    match error {
        BackendError::Unavailable { .. } => failure("backend-unavailable", error.to_string()),
        BackendError::Connection(message) => failure("connection", message),
    }
}

fn worker_failure(error: WorkerError) -> Error {
    match error {
        WorkerError::Closed => failure("closed", "the database is closed"),
        WorkerError::Manifest(message) => failure("manifest", message),
        WorkerError::Compile(errors) => compile_failure(errors),
        WorkerError::Exec(ExecError::Connection(message)) => failure("connection", message),
        WorkerError::Exec(other) => failure("database", other.to_string()),
    }
}

fn parse<T: for<'de> Deserialize<'de>>(text: &str, what: &str) -> Result<T> {
    serde_json::from_str(text).map_err(|e| failure("type", format!("invalid {what}: {e}")))
}

fn to_json(value: &impl serde::Serialize) -> String {
    serde_json::to_string(value).expect("results serialize to JSON")
}

/// A path, or an in-memory `{ name, text, format }` document.
#[derive(Deserialize)]
#[serde(untagged)]
enum SourceInput {
    Path(String),
    Document {
        name: String,
        text: String,
        #[serde(default)]
        format: Option<String>,
    },
}

/// Compile options as `index.js` sends them; absent values take the CLI defaults.
#[derive(Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
struct CompileFields {
    shapes: Vec<SourceInput>,
    ontologies: Vec<SourceInput>,
    dialect: Option<String>,
    schema: Option<Value>,
    node_key: Option<String>,
    neo4j_labels: Option<String>,
    strict: bool,
    lenient: bool,
    verbose: bool,
    max_path_depth: Option<u32>,
    allow_remote_imports: bool,
    fail_on_schema_mismatch: bool,
    base_dir: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ValidateInput {
    #[serde(default)]
    manifest: Option<String>,
    #[serde(default)]
    limit: Option<u64>,
    #[serde(default)]
    sample_size: Option<u32>,
    #[serde(default)]
    timeout_ms: Option<f64>,
    #[serde(flatten)]
    compile: CompileFields,
}

fn split_sources(sources: Vec<SourceInput>) -> Result<(Vec<PathBuf>, Vec<Document>)> {
    let mut paths = Vec::new();
    let mut documents = Vec::new();
    for source in sources {
        match source {
            SourceInput::Path(path) => paths.push(PathBuf::from(path)),
            SourceInput::Document { name, text, format } => {
                let format = format
                    .map(|format| {
                        RdfFormat::from_name(&format)
                            .ok_or_else(|| invalid("format", &format, "turtle, ntriples, trig"))
                    })
                    .transpose()?;
                documents.push(Document { name, text, format });
            }
        }
    }
    Ok((paths, documents))
}

/// Owned sources and options; `dialect` overrides the requested one (a database's).
fn prepare(fields: CompileFields, dialect: Option<Dialect>) -> Result<(Sources, CompileOptions)> {
    let dialect = match (dialect, fields.dialect) {
        (Some(dialect), _) => dialect,
        (None, Some(name)) => session::dialect_named(&name)
            .ok_or_else(|| invalid("dialect", &name, "neo4j, ladybug"))?,
        (None, None) => return Err(failure("type", "dialect is required")),
    };
    let labels = fields.neo4j_labels.as_deref().unwrap_or("explicit");
    let label_policy = session::label_policy_named(labels)
        .ok_or_else(|| invalid("neo4jLabels", labels, "explicit, inherited"))?;
    let schema = match fields.schema {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) => Some(text),
        // A parsed snapshot is written as `schema dump` would, so it hashes like
        // the equivalent file.
        Some(object) => {
            let snapshot: SchemaSnapshot = serde_json::from_value(object)
                .map_err(|e| compile_failure(vec![format!("invalid schema snapshot: {e}")]))?;
            Some(session::snapshot_text(&snapshot))
        }
    };
    let (shapes, shape_documents) = split_sources(fields.shapes)?;
    let (ontologies, ontology_documents) = split_sources(fields.ontologies)?;
    let options = CompileOptions {
        dialect,
        node_key: fields.node_key,
        label_policy,
        strict: fields.strict,
        lenient: fields.lenient,
        verbose: fields.verbose,
        max_path_depth: fields.max_path_depth.unwrap_or(10),
        allow_remote_imports: fields.allow_remote_imports,
        fail_on_schema_mismatch: fields.fail_on_schema_mismatch,
        base_dir: fields
            .base_dir
            .map(PathBuf::from)
            .or_else(|| std::env::current_dir().ok()),
    };
    let sources = Sources {
        shapes,
        shape_documents,
        ontologies,
        ontology_documents,
        schema,
    };
    Ok((sources, options))
}

/// `{ manifestJson, cypher }` as JSON text.
fn run_compile(input: &str) -> Result<String> {
    let (sources, options) = prepare(parse(input, "compile options")?, None)?;
    let compilation =
        compile_request(&sources.request(), &options).map_err(|error| compile_failure(error.0))?;
    Ok(
        json!({ "manifestJson": compilation.manifest_json(), "cypher": compilation.cypher })
            .to_string(),
    )
}

#[napi(js_name = "compile")]
pub async fn compile_async(input: String) -> Result<String> {
    tokio::task::spawn_blocking(move || run_compile(&input))
        .await
        .map_err(|e| failure("internal", e.to_string()))?
}

#[napi]
pub fn compile_sync(input: String) -> Result<String> {
    run_compile(&input)
}

/// The manifest re-serialized after version checks.
#[napi]
pub fn load_manifest(text: String) -> Result<String> {
    let manifest = Manifest::from_json(&text).map_err(|e| failure("manifest", e))?;
    Ok(to_json(&manifest))
}

#[napi]
pub fn render_report(report: String, format: String) -> Result<String> {
    let report: Report = parse(&report, "report")?;
    let format = session::format_named(&format)
        .ok_or_else(|| invalid("format", &format, "table, json, junit, sarif"))?;
    Ok(report::render(&report, format))
}

#[napi]
pub fn exit_code(report: String, fail_on: String) -> Result<u32> {
    let report: Report = parse(&report, "report")?;
    let fail_on = session::fail_on_named(&fail_on)
        .ok_or_else(|| invalid("failOn", &fail_on, "violation, warning, info"))?;
    Ok(u32::from(validate::exit_code(&report, fail_on)))
}

#[napi]
pub fn available_backends() -> Vec<String> {
    shacl2cypher_runner::available_backends()
        .into_iter()
        .map(String::from)
        .collect()
}

#[napi]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").into()
}

#[derive(Deserialize)]
struct Neo4jInput {
    uri: String,
    user: String,
    password: String,
    #[serde(default)]
    database: Option<String>,
}

async fn open(config: BackendConfig) -> Result<NativeDatabase> {
    let worker = tokio::task::spawn_blocking(move || DatabaseWorker::open(config))
        .await
        .map_err(|e| failure("internal", e.to_string()))?
        .map_err(backend_failure)?;
    Ok(NativeDatabase {
        worker: Arc::new(worker),
    })
}

// napi-rs would name this `openNeo4J`.
#[napi(js_name = "openNeo4j")]
pub async fn open_neo4j(input: String) -> Result<NativeDatabase> {
    let input: Neo4jInput = parse(&input, "Neo4j options")?;
    open(BackendConfig::Neo4j {
        uri: input.uri,
        user: input.user,
        password: input.password,
        database: input.database,
    })
    .await
}

#[napi]
pub async fn open_ladybug(path: String) -> Result<NativeDatabase> {
    open(BackendConfig::Ladybug {
        path: PathBuf::from(path),
    })
    .await
}

/// A database handle; its executor runs on the worker's own thread, and methods
/// await the worker's reply without occupying libuv threads.
#[napi]
pub struct NativeDatabase {
    worker: Arc<DatabaseWorker>,
}

#[napi]
impl NativeDatabase {
    #[napi(getter)]
    pub fn dialect(&self) -> String {
        self.worker.dialect().name().into()
    }

    #[napi(getter)]
    pub fn closed(&self) -> bool {
        self.worker.is_closed()
    }

    /// Runs a validation; the report as JSON text.
    #[napi]
    pub async fn validate(&self, input: String) -> Result<String> {
        let input: ValidateInput = parse(&input, "validate options")?;
        let timeout = input
            .timeout_ms
            .map(|ms| {
                session::positive_timeout(ms / 1000.0).ok_or_else(|| {
                    failure(
                        "range",
                        "timeoutMs must be a positive number of milliseconds",
                    )
                })
            })
            .transpose()?;
        let options = ValidateOptions {
            limit: match input.limit {
                None => Some(100),
                Some(0) => None,
                Some(limit) => Some(limit),
            },
            sample_size: input.sample_size.unwrap_or(5),
            timeout,
        };
        let target = match input.manifest {
            Some(text) => {
                let manifest = Manifest::from_json(&text).map_err(|e| failure("manifest", e))?;
                session::check_dialect(&manifest, self.worker.dialect())
                    .map_err(|e| failure("manifest", e))?;
                Target::Manifest(Box::new(manifest))
            }
            None => {
                let (sources, compile_options) =
                    prepare(input.compile, Some(self.worker.dialect()))?;
                Target::Shapes(Box::new(sources), Box::new(compile_options))
            }
        };
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.worker.validate(target, options, move |result| {
            let _ = sender.send(result);
        });
        let report = receiver
            .await
            .unwrap_or(Err(WorkerError::Closed))
            .map_err(worker_failure)?;
        Ok(to_json(&report))
    }

    /// The schema snapshot as `schema dump` writes it.
    #[napi]
    pub async fn schema(&self) -> Result<String> {
        let (sender, receiver) = tokio::sync::oneshot::channel();
        self.worker.schema(move |result| {
            let _ = sender.send(result);
        });
        let snapshot = receiver
            .await
            .unwrap_or(Err(WorkerError::Closed))
            .map_err(worker_failure)?;
        Ok(session::snapshot_text(&snapshot))
    }

    /// Waits for queued calls, then closes the database.
    #[napi]
    pub async fn close(&self) -> Result<()> {
        let worker = Arc::clone(&self.worker);
        tokio::task::spawn_blocking(move || worker.close())
            .await
            .map_err(|e| failure("internal", e.to_string()))
    }
}
