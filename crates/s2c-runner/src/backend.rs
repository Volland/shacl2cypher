//! Opens a database backend from its configuration, shared by the CLI and the
//! language bindings.

use std::path::PathBuf;

pub use crate::available_backends;
use crate::executor::Executor;

/// Which database to open and how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendConfig {
    Neo4j {
        uri: String,
        user: String,
        password: String,
        /// Database name; the server default when absent.
        database: Option<String>,
    },
    /// An existing LadybugDB database file, opened read-only.
    Ladybug { path: PathBuf },
}

impl BackendConfig {
    /// Backend name as reported by [`available_backends`].
    pub fn name(&self) -> &'static str {
        match self {
            BackendConfig::Neo4j { .. } => "neo4j",
            BackendConfig::Ladybug { .. } => "ladybug",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BackendError {
    /// The backend was not compiled into this build.
    #[error("{}", unavailable_message(.backend, .available))]
    Unavailable {
        backend: &'static str,
        available: Vec<&'static str>,
    },
    /// The database could not be reached or opened.
    #[error("{0}")]
    Connection(String),
}

fn unavailable_message(backend: &str, available: &[&str]) -> String {
    if available.is_empty() {
        "no database backend is available in this build; rebuild with `--features neo4j` or `--features ladybug`".into()
    } else {
        format!(
            "the {backend} backend is not available in this build (available: {})",
            available.join(", ")
        )
    }
}

/// Fails when this build has no database backend at all.
pub fn require_any_backend() -> Result<(), BackendError> {
    if available_backends().is_empty() {
        return Err(BackendError::Unavailable {
            backend: "",
            available: Vec::new(),
        });
    }
    Ok(())
}

/// Opens the configured database: Neo4j connections are verified, LadybugDB
/// files are opened read-only and must exist.
// @lat: [[architecture#Runner#Backends]]
pub fn open(config: &BackendConfig) -> Result<Box<dyn Executor>, BackendError> {
    require_any_backend()?;
    #[cfg(not(all(feature = "neo4j", feature = "ladybug")))]
    let unavailable = || BackendError::Unavailable {
        backend: config.name(),
        available: available_backends(),
    };
    match config {
        BackendConfig::Neo4j {
            uri,
            user,
            password,
            database,
        } => {
            #[cfg(feature = "neo4j")]
            {
                let executor = crate::neo4j::Neo4jExecutor::connect(crate::neo4j::Neo4jConfig {
                    uri: uri.clone(),
                    user: user.clone(),
                    password: password.clone(),
                    database: database.clone(),
                })
                .map_err(|e| BackendError::Connection(e.to_string()))?;
                Ok(Box::new(executor))
            }
            #[cfg(not(feature = "neo4j"))]
            {
                let _ = (uri, user, password, database);
                Err(unavailable())
            }
        }
        BackendConfig::Ladybug { path } => {
            #[cfg(feature = "ladybug")]
            {
                let executor = crate::ladybug::LadybugExecutor::open(path)
                    .map_err(|e| BackendError::Connection(e.to_string()))?;
                Ok(Box::new(executor))
            }
            #[cfg(not(feature = "ladybug"))]
            {
                let _ = path;
                Err(unavailable())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_messages_name_the_backends() {
        let none = BackendError::Unavailable {
            backend: "ladybug",
            available: Vec::new(),
        };
        assert!(none.to_string().starts_with("no database backend"));
        let some = BackendError::Unavailable {
            backend: "ladybug",
            available: vec!["neo4j"],
        };
        assert_eq!(
            some.to_string(),
            "the ladybug backend is not available in this build (available: neo4j)"
        );
    }

    #[cfg(not(any(feature = "neo4j", feature = "ladybug")))]
    #[test]
    fn compile_only_builds_open_nothing() {
        let config = BackendConfig::Ladybug {
            path: "graph.lbug".into(),
        };
        assert!(matches!(
            open(&config),
            Err(BackendError::Unavailable { .. })
        ));
    }
}
