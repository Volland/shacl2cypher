//! The `shacl2cypher._native` extension module.
//!
//! The typed public API lives in `python/shacl2cypher/__init__.py`. This module only
//! converts values, errors and threads around `shacl2cypher-core` and
//! `shacl2cypher-runner`, releasing the GIL while Rust code runs.

use std::path::{Path, PathBuf};

use pyo3::exceptions::{PyException, PyOSError, PyValueError};
use pyo3::prelude::*;
use pythonize::{depythonize, pythonize};
use shacl2cypher_core::compile::{compile as compile_request, CompileOptions, Manifest};
use shacl2cypher_core::load::{Document, RdfFormat};
use shacl2cypher_core::schema::SchemaSnapshot;
use shacl2cypher_runner::backend::{BackendConfig, BackendError};
use shacl2cypher_runner::executor::ExecError;
use shacl2cypher_runner::report;
use shacl2cypher_runner::session::{self, Sources};
use shacl2cypher_runner::validate::{self, ValidateOptions};
use shacl2cypher_runner::worker::{DatabaseWorker, Target, WorkerError};

pyo3::create_exception!(
    _native,
    Shacl2CypherError,
    PyException,
    "Base class of shacl2cypher errors."
);
pyo3::create_exception!(
    _native,
    CompileError,
    Shacl2CypherError,
    "Compilation failed; `errors` lists every problem."
);
pyo3::create_exception!(
    _native,
    ManifestError,
    Shacl2CypherError,
    "A manifest is invalid, unsupported, or compiled for another dialect."
);
pyo3::create_exception!(
    _native,
    DatabaseConnectionError,
    Shacl2CypherError,
    "A database could not be reached or opened."
);
pyo3::create_exception!(
    _native,
    BackendUnavailableError,
    Shacl2CypherError,
    "The database backend is not built into this package."
);
pyo3::create_exception!(
    _native,
    DatabaseClosedError,
    Shacl2CypherError,
    "The database handle is closed."
);

fn compile_error(py: Python<'_>, errors: Vec<String>) -> PyErr {
    let error = CompileError::new_err(errors.join("\n"));
    if let Err(setattr_error) = error.value(py).setattr("errors", errors) {
        return setattr_error;
    }
    error
}

fn backend_error(error: BackendError) -> PyErr {
    match error {
        BackendError::Unavailable { .. } => BackendUnavailableError::new_err(error.to_string()),
        BackendError::Connection(message) => DatabaseConnectionError::new_err(message),
    }
}

fn worker_error(py: Python<'_>, error: WorkerError) -> PyErr {
    match error {
        WorkerError::Closed => DatabaseClosedError::new_err("the database is closed"),
        WorkerError::Manifest(message) => ManifestError::new_err(message),
        WorkerError::Compile(errors) => compile_error(py, errors),
        WorkerError::Exec(ExecError::Connection(message)) => {
            DatabaseConnectionError::new_err(message)
        }
        WorkerError::Exec(other) => Shacl2CypherError::new_err(other.to_string()),
    }
}

fn invalid(option: &str, value: &str, allowed: &str) -> PyErr {
    PyValueError::new_err(format!("{option} must be one of {allowed} (got {value:?})"))
}

/// A path, or a `Source(name, text, format)` tuple.
#[derive(FromPyObject)]
enum SourceArg {
    Path(PathBuf),
    Document(String, String, Option<String>),
}

fn split_sources(sources: Vec<SourceArg>) -> PyResult<(Vec<PathBuf>, Vec<Document>)> {
    let mut paths = Vec::new();
    let mut documents = Vec::new();
    for source in sources {
        match source {
            SourceArg::Path(path) => paths.push(path),
            SourceArg::Document(name, text, format) => {
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

/// Compile options as the dict built by `shacl2cypher._options`.
#[derive(FromPyObject)]
#[pyo3(from_item_all)]
struct CompileArgs {
    dialect: String,
    schema: Option<Py<PyAny>>,
    node_key: Option<String>,
    neo4j_labels: String,
    strict: bool,
    lenient: bool,
    verbose: bool,
    max_path_depth: u32,
    allow_remote_imports: bool,
    fail_on_schema_mismatch: bool,
    base_dir: Option<PathBuf>,
}

/// Owned sources and options, ready to compile without the GIL.
fn prepare(
    py: Python<'_>,
    shapes: Vec<SourceArg>,
    ontologies: Vec<SourceArg>,
    args: CompileArgs,
) -> PyResult<(Sources, CompileOptions)> {
    let dialect = session::dialect_named(&args.dialect)
        .ok_or_else(|| invalid("dialect", &args.dialect, "neo4j, ladybug"))?;
    let label_policy = session::label_policy_named(&args.neo4j_labels)
        .ok_or_else(|| invalid("neo4j_labels", &args.neo4j_labels, "explicit, inherited"))?;
    let schema = match args.schema {
        None => None,
        Some(schema) => {
            let schema = schema.bind(py);
            match schema.extract::<String>() {
                Ok(text) => Some(text),
                // A parsed snapshot is written as `schema dump` would, so it hashes
                // like the equivalent file.
                Err(_) => {
                    let snapshot: SchemaSnapshot = depythonize(schema).map_err(|e| {
                        compile_error(py, vec![format!("invalid schema snapshot: {e}")])
                    })?;
                    Some(session::snapshot_text(&snapshot))
                }
            }
        }
    };
    let (shapes, shape_documents) = split_sources(shapes)?;
    let (ontologies, ontology_documents) = split_sources(ontologies)?;
    let options = CompileOptions {
        dialect,
        node_key: args.node_key,
        label_policy,
        strict: args.strict,
        lenient: args.lenient,
        verbose: args.verbose,
        max_path_depth: args.max_path_depth,
        allow_remote_imports: args.allow_remote_imports,
        fail_on_schema_mismatch: args.fail_on_schema_mismatch,
        base_dir: args.base_dir.or_else(|| std::env::current_dir().ok()),
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

#[pyfunction]
fn compile(
    py: Python<'_>,
    shapes: Vec<SourceArg>,
    ontologies: Vec<SourceArg>,
    options: CompileArgs,
) -> PyResult<Compilation> {
    let (sources, options) = prepare(py, shapes, ontologies, options)?;
    let compilation = py
        .detach(|| compile_request(&sources.request(), &options))
        .map_err(|error| compile_error(py, error.0))?;
    Ok(Compilation {
        manifest_json: compilation.manifest_json(),
        manifest: compilation.manifest,
        cypher: compilation.cypher,
    })
}

#[pyfunction]
fn load_manifest<'py>(py: Python<'py>, text: &str) -> PyResult<Bound<'py, PyAny>> {
    let manifest = Manifest::from_json(text).map_err(ManifestError::new_err)?;
    Ok(pythonize(py, &manifest)?)
}

#[pyfunction]
fn available_backends() -> Vec<&'static str> {
    shacl2cypher_runner::available_backends()
}

#[pyclass(frozen, module = "shacl2cypher._native")]
struct Compilation {
    manifest: Manifest,
    manifest_json: String,
    cypher: String,
}

#[pymethods]
impl Compilation {
    #[getter]
    fn manifest<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        Ok(pythonize(py, &self.manifest)?)
    }

    #[getter]
    fn manifest_json(&self) -> String {
        self.manifest_json.clone()
    }

    #[getter]
    fn cypher(&self) -> String {
        self.cypher.clone()
    }

    #[getter]
    fn static_diagnostics<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        Ok(pythonize(py, &self.manifest.static_diagnostics)?)
    }

    /// Writes `manifest.json` and `queries.cypher` into `directory`, creating it.
    fn write(&self, py: Python<'_>, directory: PathBuf) -> PyResult<()> {
        py.detach(|| write_outputs(&directory, &self.manifest_json, &self.cypher))
            .map_err(PyOSError::new_err)
    }

    fn __repr__(&self) -> String {
        format!(
            "<Compilation of {} rules for {}>",
            self.manifest.rules.len(),
            self.manifest.dialect
        )
    }
}

fn write_outputs(directory: &Path, manifest_json: &str, cypher: &str) -> Result<(), String> {
    std::fs::create_dir_all(directory).map_err(|e| format!("{}: {e}", directory.display()))?;
    for (name, contents) in [("manifest.json", manifest_json), ("queries.cypher", cypher)] {
        let path = directory.join(name);
        std::fs::write(&path, contents).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(())
}

#[pyclass(frozen, module = "shacl2cypher._native")]
struct Report {
    report: validate::Report,
}

#[pymethods]
impl Report {
    /// Parses a report written with format `json`.
    #[staticmethod]
    fn from_json(text: &str) -> PyResult<Report> {
        serde_json::from_str(text)
            .map(|report| Report { report })
            .map_err(|e| PyValueError::new_err(format!("invalid report: {e}")))
    }

    #[getter]
    fn conforms(&self) -> bool {
        self.report.conforms
    }

    #[getter]
    fn complete(&self) -> bool {
        self.report.complete
    }

    #[getter]
    fn summary<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        Ok(pythonize(py, &self.report.summary)?)
    }

    #[getter]
    fn rules<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        Ok(pythonize(py, &self.report.rules)?)
    }

    fn to_dict<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyAny>> {
        Ok(pythonize(py, &self.report)?)
    }

    fn to_json(&self) -> String {
        report::render(&self.report, report::Format::Json)
    }

    fn render(&self, format: &str) -> PyResult<String> {
        let format = session::format_named(format)
            .ok_or_else(|| invalid("format", format, "table, json, junit, sarif"))?;
        Ok(report::render(&self.report, format))
    }

    #[pyo3(signature = (fail_on = "violation"))]
    fn exit_code(&self, fail_on: &str) -> PyResult<u8> {
        let fail_on = session::fail_on_named(fail_on)
            .ok_or_else(|| invalid("fail_on", fail_on, "violation, warning, info"))?;
        Ok(validate::exit_code(&self.report, fail_on))
    }

    fn __repr__(&self) -> String {
        let summary = &self.report.summary;
        format!(
            "<Report conforms={} complete={} rules={} failed={}>",
            self.report.conforms, self.report.complete, summary.rules, summary.failed
        )
    }
}

#[pyclass(frozen, module = "shacl2cypher._native")]
struct Database {
    worker: DatabaseWorker,
}

fn open(py: Python<'_>, config: BackendConfig) -> PyResult<Database> {
    py.detach(|| DatabaseWorker::open(config))
        .map(|worker| Database { worker })
        .map_err(backend_error)
}

fn validate_options(
    limit: Option<u64>,
    sample_size: u32,
    timeout: Option<f64>,
) -> PyResult<ValidateOptions> {
    let timeout = timeout
        .map(|seconds| {
            session::positive_timeout(seconds).ok_or_else(|| {
                PyValueError::new_err("timeout must be a positive number of seconds")
            })
        })
        .transpose()?;
    Ok(ValidateOptions {
        limit: limit.filter(|limit| *limit > 0),
        sample_size,
        timeout,
    })
}

#[pymethods]
impl Database {
    #[staticmethod]
    fn neo4j(
        py: Python<'_>,
        uri: String,
        user: String,
        password: String,
        database: Option<String>,
    ) -> PyResult<Database> {
        open(
            py,
            BackendConfig::Neo4j {
                uri,
                user,
                password,
                database,
            },
        )
    }

    #[staticmethod]
    fn ladybug(py: Python<'_>, path: PathBuf) -> PyResult<Database> {
        open(py, BackendConfig::Ladybug { path })
    }

    #[getter]
    fn dialect(&self) -> &'static str {
        self.worker.dialect().name()
    }

    #[getter]
    fn closed(&self) -> bool {
        self.worker.is_closed()
    }

    fn validate_manifest(
        &self,
        py: Python<'_>,
        manifest_json: &str,
        limit: Option<u64>,
        sample_size: u32,
        timeout: Option<f64>,
    ) -> PyResult<Report> {
        let options = validate_options(limit, sample_size, timeout)?;
        let manifest = Manifest::from_json(manifest_json).map_err(ManifestError::new_err)?;
        session::check_dialect(&manifest, self.worker.dialect()).map_err(ManifestError::new_err)?;
        self.run(py, Target::Manifest(Box::new(manifest)), options)
    }

    #[allow(clippy::too_many_arguments)]
    fn validate_shapes(
        &self,
        py: Python<'_>,
        shapes: Vec<SourceArg>,
        ontologies: Vec<SourceArg>,
        options: CompileArgs,
        limit: Option<u64>,
        sample_size: u32,
        timeout: Option<f64>,
    ) -> PyResult<Report> {
        let validate_options = validate_options(limit, sample_size, timeout)?;
        let (sources, compile_options) = prepare(py, shapes, ontologies, options)?;
        let target = Target::Shapes(Box::new(sources), Box::new(compile_options));
        self.run(py, target, validate_options)
    }

    fn schema_json(&self, py: Python<'_>) -> PyResult<String> {
        py.detach(|| self.worker.schema_blocking())
            .map(|snapshot| session::snapshot_text(&snapshot))
            .map_err(|error| worker_error(py, error))
    }

    fn close(&self, py: Python<'_>) {
        py.detach(|| self.worker.close());
    }
}

impl Database {
    fn run(&self, py: Python<'_>, target: Target, options: ValidateOptions) -> PyResult<Report> {
        py.detach(|| self.worker.validate_blocking(target, options))
            .map(|report| Report { report })
            .map_err(|error| worker_error(py, error))
    }
}

#[pymodule]
fn _native(module: &Bound<'_, PyModule>) -> PyResult<()> {
    let py = module.py();
    module.add("Shacl2CypherError", py.get_type::<Shacl2CypherError>())?;
    module.add("CompileError", py.get_type::<CompileError>())?;
    module.add("ManifestError", py.get_type::<ManifestError>())?;
    module.add(
        "DatabaseConnectionError",
        py.get_type::<DatabaseConnectionError>(),
    )?;
    module.add(
        "BackendUnavailableError",
        py.get_type::<BackendUnavailableError>(),
    )?;
    module.add("DatabaseClosedError", py.get_type::<DatabaseClosedError>())?;
    module.add("__version__", env!("CARGO_PKG_VERSION"))?;
    module.add_function(wrap_pyfunction!(compile, module)?)?;
    module.add_function(wrap_pyfunction!(load_manifest, module)?)?;
    module.add_function(wrap_pyfunction!(available_backends, module)?)?;
    module.add_class::<Compilation>()?;
    module.add_class::<Report>()?;
    module.add_class::<Database>()?;
    Ok(())
}
