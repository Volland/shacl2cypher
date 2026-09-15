//! Validate-flow rules shared by the CLI and the language bindings: compiling
//! shapes for a connected database, dialect checks and timeout parsing.

use std::path::PathBuf;
use std::time::Duration;

use shacl2cypher_core::compile::{compile, CompileError, CompileOptions, CompileRequest, Manifest};
use shacl2cypher_core::hierarchy::LabelPolicy;
use shacl2cypher_core::load::{Document, RemoteFetcher};
use shacl2cypher_core::render::Dialect;
use shacl2cypher_core::schema::SchemaSnapshot;

use crate::executor::{ExecError, Executor};
use crate::report::Format;
use crate::validate::FailOn;

/// Owned shapes, ontologies and schema text, for compiling later or on another
/// thread (a language binding's database worker).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Sources {
    pub shapes: Vec<PathBuf>,
    pub shape_documents: Vec<Document>,
    pub ontologies: Vec<PathBuf>,
    pub ontology_documents: Vec<Document>,
    /// Schema snapshot JSON text.
    pub schema: Option<String>,
}

impl Sources {
    /// A request over these sources using [`default_fetcher`].
    pub fn request(&self) -> CompileRequest<'_> {
        CompileRequest {
            shapes: &self.shapes,
            ontologies: &self.ontologies,
            shape_documents: &self.shape_documents,
            ontology_documents: &self.ontology_documents,
            schema: self.schema.as_deref(),
            fetcher: default_fetcher(),
        }
    }
}

/// The HTTP(S) fetcher when built with `remote-imports`, else none.
pub fn default_fetcher() -> Option<&'static dyn RemoteFetcher> {
    #[cfg(feature = "remote-imports")]
    return Some(&crate::remote::HttpFetcher);
    #[cfg(not(feature = "remote-imports"))]
    return None;
}

/// A schema snapshot as `schema dump` writes it, including the final newline.
pub fn snapshot_text(snapshot: &SchemaSnapshot) -> String {
    let mut text = snapshot.to_json();
    text.push('\n');
    text
}

/// `neo4j`, `ladybug` or `falkordb`.
pub fn dialect_named(name: &str) -> Option<Dialect> {
    match name {
        "neo4j" => Some(Dialect::Neo4j),
        "ladybug" => Some(Dialect::Ladybug),
        "falkordb" => Some(Dialect::FalkorDb),
        _ => None,
    }
}

/// `explicit` or `inherited`, as for `--neo4j-labels`.
pub fn label_policy_named(name: &str) -> Option<LabelPolicy> {
    match name {
        "explicit" => Some(LabelPolicy::Explicit),
        "inherited" => Some(LabelPolicy::Inherited),
        _ => None,
    }
}

/// `table`, `json`, `junit` or `sarif`, as for `--format`.
pub fn format_named(name: &str) -> Option<Format> {
    match name {
        "table" => Some(Format::Table),
        "json" => Some(Format::Json),
        "junit" => Some(Format::Junit),
        "sarif" => Some(Format::Sarif),
        _ => None,
    }
}

/// `violation`, `warning` or `info`, as for `--fail-on`.
pub fn fail_on_named(name: &str) -> Option<FailOn> {
    match name {
        "violation" => Some(FailOn::Violation),
        "warning" => Some(FailOn::Warning),
        "info" => Some(FailOn::Info),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SessionError {
    /// Reading the schema snapshot from the database failed.
    #[error("{0}")]
    Schema(ExecError),
    #[error("{0}")]
    Compile(CompileError),
}

/// Compiles shapes for the executor's dialect. On LadybugDB without a schema in
/// the request, the schema is read from the database first.
// @lat: [[architecture#Runner#Validate Flow]]
pub fn compile_for(
    executor: &mut dyn Executor,
    request: &CompileRequest<'_>,
    options: &CompileOptions,
) -> Result<Manifest, SessionError> {
    let dialect = executor.dialect();
    let dumped = match (request.schema, dialect) {
        (None, Dialect::Ladybug) => {
            Some(executor.schema().map_err(SessionError::Schema)?.to_json())
        }
        _ => None,
    };
    let request = CompileRequest {
        shapes: request.shapes,
        ontologies: request.ontologies,
        shape_documents: request.shape_documents,
        ontology_documents: request.ontology_documents,
        schema: request.schema.or(dumped.as_deref()),
        fetcher: request.fetcher,
    };
    let options = CompileOptions {
        dialect,
        ..options.clone()
    };
    compile(&request, &options)
        .map(|compilation| compilation.manifest)
        .map_err(SessionError::Compile)
}

/// Rejects a manifest compiled for another dialect than the database's.
pub fn check_dialect(manifest: &Manifest, dialect: Dialect) -> Result<(), String> {
    let backend = dialect.name();
    if manifest.dialect != backend {
        return Err(format!(
            "the manifest was compiled for {} but the database is {backend}",
            manifest.dialect
        ));
    }
    Ok(())
}

/// A per-query timeout of `seconds`; `None` unless finite and positive.
pub fn positive_timeout(seconds: f64) -> Option<Duration> {
    (seconds.is_finite() && seconds > 0.0).then(|| Duration::from_secs_f64(seconds))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_parse_like_cli_values() {
        assert_eq!(dialect_named("ladybug"), Some(Dialect::Ladybug));
        assert_eq!(dialect_named("falkordb"), Some(Dialect::FalkorDb));
        assert_eq!(dialect_named("postgres"), None);
        assert_eq!(
            label_policy_named("inherited"),
            Some(LabelPolicy::Inherited)
        );
        assert_eq!(format_named("sarif"), Some(Format::Sarif));
        assert_eq!(fail_on_named("warning"), Some(FailOn::Warning));
        assert_eq!(fail_on_named("Warning"), None);
    }

    #[test]
    fn timeouts_must_be_positive_and_finite() {
        assert_eq!(positive_timeout(1.5), Some(Duration::from_millis(1500)));
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(positive_timeout(bad), None, "{bad}");
        }
    }
}
