//! A database handle whose executor lives on its own thread.
//!
//! Executors are not `Send`, and language bindings must not block Python's GIL or
//! Node's event loop while queries run. The worker thread owns the executor and
//! serves jobs one at a time; each job hands its result to a reply callback.

use std::sync::{mpsc, Mutex, MutexGuard};
use std::thread::JoinHandle;

use shacl2cypher_core::compile::{CompileOptions, Manifest};
use shacl2cypher_core::render::Dialect;
use shacl2cypher_core::schema::SchemaSnapshot;

use crate::backend::{self, BackendConfig, BackendError};
use crate::executor::{ExecError, Executor};
use crate::session::{self, SessionError, Sources};
use crate::validate::{validate, Report, ValidateOptions};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WorkerError {
    #[error("the database is closed")]
    Closed,
    /// The manifest is unusable for this database.
    #[error("{0}")]
    Manifest(String),
    #[error("{}", .0.join("\n"))]
    Compile(Vec<String>),
    #[error("{0}")]
    Exec(ExecError),
}

/// What a validation runs.
#[derive(Debug, Clone)]
pub enum Target {
    Manifest(Box<Manifest>),
    /// Shapes compiled for the database's dialect first.
    Shapes(Box<Sources>, Box<CompileOptions>),
}

type Reply<T> = Box<dyn FnOnce(Result<T, WorkerError>) + Send>;

enum Job {
    Validate(Target, ValidateOptions, Reply<Report>),
    Schema(Reply<SchemaSnapshot>),
}

impl Job {
    fn fail(self, error: WorkerError) {
        match self {
            Job::Validate(_, _, reply) => reply(Err(error)),
            Job::Schema(reply) => reply(Err(error)),
        }
    }
}

// @lat: [[architecture#Runner]]
pub struct DatabaseWorker {
    dialect: Dialect,
    sender: Mutex<Option<mpsc::Sender<Job>>>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl DatabaseWorker {
    /// Opens the configured backend on a new worker thread, waiting until it is open.
    pub fn open(config: BackendConfig) -> Result<Self, BackendError> {
        Self::spawn(move || backend::open(&config))
    }

    /// Runs `open` on a new worker thread, which then owns the executor.
    pub fn spawn(
        open: impl FnOnce() -> Result<Box<dyn Executor>, BackendError> + Send + 'static,
    ) -> Result<Self, BackendError> {
        let (ready, opened) = mpsc::channel();
        let (sender, jobs) = mpsc::channel::<Job>();
        let thread = std::thread::Builder::new()
            .name("shacl2cypher-database".into())
            .spawn(move || {
                let mut executor = match open() {
                    Ok(executor) => executor,
                    Err(error) => {
                        let _ = ready.send(Err(error));
                        return;
                    }
                };
                let _ = ready.send(Ok(executor.dialect()));
                for job in jobs {
                    run(executor.as_mut(), job);
                }
            })
            .map_err(|e| {
                BackendError::Connection(format!("cannot start a database thread: {e}"))
            })?;
        let dialect = match opened.recv() {
            Ok(Ok(dialect)) => dialect,
            Ok(Err(error)) => {
                let _ = thread.join();
                return Err(error);
            }
            Err(_) => {
                let _ = thread.join();
                return Err(BackendError::Connection(
                    "the database thread stopped while opening".into(),
                ));
            }
        };
        Ok(DatabaseWorker {
            dialect,
            sender: Mutex::new(Some(sender)),
            thread: Mutex::new(Some(thread)),
        })
    }

    pub fn dialect(&self) -> Dialect {
        self.dialect
    }

    pub fn is_closed(&self) -> bool {
        lock(&self.sender).is_none()
    }

    /// Queues a validation; `reply` runs on the worker thread.
    pub fn validate(
        &self,
        target: Target,
        options: ValidateOptions,
        reply: impl FnOnce(Result<Report, WorkerError>) + Send + 'static,
    ) {
        self.submit(Job::Validate(target, options, Box::new(reply)));
    }

    /// Queues a schema dump; `reply` runs on the worker thread.
    pub fn schema(&self, reply: impl FnOnce(Result<SchemaSnapshot, WorkerError>) + Send + 'static) {
        self.submit(Job::Schema(Box::new(reply)));
    }

    /// [`DatabaseWorker::validate`], waiting for the result on the calling thread.
    pub fn validate_blocking(
        &self,
        target: Target,
        options: ValidateOptions,
    ) -> Result<Report, WorkerError> {
        let (sender, receiver) = mpsc::channel();
        self.validate(target, options, move |result| {
            let _ = sender.send(result);
        });
        receiver.recv().unwrap_or(Err(WorkerError::Closed))
    }

    /// [`DatabaseWorker::schema`], waiting for the result on the calling thread.
    pub fn schema_blocking(&self) -> Result<SchemaSnapshot, WorkerError> {
        let (sender, receiver) = mpsc::channel();
        self.schema(move |result| {
            let _ = sender.send(result);
        });
        receiver.recv().unwrap_or(Err(WorkerError::Closed))
    }

    /// Stops accepting jobs, lets queued jobs finish and closes the database.
    /// Closing again is a no-op.
    pub fn close(&self) {
        drop(lock(&self.sender).take());
        if let Some(thread) = lock(&self.thread).take() {
            let _ = thread.join();
        }
    }

    fn submit(&self, job: Job) {
        let sender = lock(&self.sender);
        let job = match sender.as_ref() {
            Some(sender) => match sender.send(job) {
                Ok(()) => return,
                Err(mpsc::SendError(job)) => job,
            },
            None => job,
        };
        drop(sender);
        job.fail(WorkerError::Closed);
    }
}

impl Drop for DatabaseWorker {
    fn drop(&mut self) {
        self.close();
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn run(executor: &mut dyn Executor, job: Job) {
    match job {
        Job::Validate(target, options, reply) => reply(run_validate(executor, target, &options)),
        Job::Schema(reply) => reply(executor.schema().map_err(WorkerError::Exec)),
    }
}

fn run_validate(
    executor: &mut dyn Executor,
    target: Target,
    options: &ValidateOptions,
) -> Result<Report, WorkerError> {
    let manifest = match target {
        Target::Manifest(manifest) => *manifest,
        Target::Shapes(sources, compile_options) => {
            session::compile_for(executor, &sources.request(), &compile_options).map_err(
                |error| match error {
                    SessionError::Schema(error) => WorkerError::Exec(error),
                    SessionError::Compile(error) => WorkerError::Compile(error.0),
                },
            )?
        }
    };
    session::check_dialect(&manifest, executor.dialect()).map_err(WorkerError::Manifest)?;
    validate(&manifest, executor, options).map_err(WorkerError::Manifest)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::validate::tests::{executor, manifest, rule};
    use crate::validate::Status;

    fn worker() -> DatabaseWorker {
        DatabaseWorker::spawn(|| Ok(Box::new(executor(&[("a", Some(0))])) as Box<dyn Executor>))
            .unwrap()
    }

    fn target() -> Target {
        Target::Manifest(Box::new(manifest(vec![rule("a", "Violation", "compiled")])))
    }

    #[test]
    fn validates_and_dumps_on_the_worker_thread() {
        let worker = worker();
        assert_eq!(worker.dialect(), Dialect::Neo4j);
        let report = worker
            .validate_blocking(target(), ValidateOptions::default())
            .unwrap();
        assert_eq!(report.rules[0].status, Status::Passed);
        assert_eq!(worker.schema_blocking().unwrap(), SchemaSnapshot::default());
    }

    // @lat: [[tests#Runner#Database Worker]]
    #[test]
    fn concurrent_calls_all_complete() {
        let worker = Arc::new(worker());
        let threads: Vec<_> = (0..4)
            .map(|_| {
                let worker = Arc::clone(&worker);
                std::thread::spawn(move || {
                    worker.validate_blocking(target(), ValidateOptions::default())
                })
            })
            .collect();
        for thread in threads {
            assert!(thread.join().unwrap().is_ok());
        }
    }

    #[test]
    fn closed_workers_reject_jobs() {
        let worker = worker();
        worker.close();
        worker.close();
        assert!(worker.is_closed());
        assert_eq!(
            worker.validate_blocking(target(), ValidateOptions::default()),
            Err(WorkerError::Closed)
        );
    }

    #[test]
    fn open_errors_are_returned() {
        let error = DatabaseWorker::spawn(|| Err(BackendError::Connection("nope".into())));
        assert_eq!(error.err(), Some(BackendError::Connection("nope".into())));
    }

    #[test]
    fn manifests_for_another_dialect_are_rejected() {
        let mut ladybug = manifest(vec![rule("a", "Violation", "compiled")]);
        ladybug.dialect = "ladybug".into();
        let result = worker().validate_blocking(
            Target::Manifest(Box::new(ladybug)),
            ValidateOptions::default(),
        );
        assert!(
            matches!(result, Err(WorkerError::Manifest(message)) if message.contains("ladybug"))
        );
    }
}
