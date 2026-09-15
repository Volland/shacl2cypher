use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand, ValueEnum};
use shacl2cypher_core::compile::{compile, CompileOptions, CompileRequest, Manifest};
use shacl2cypher_core::hierarchy::LabelPolicy;
use shacl2cypher_core::render::Dialect;
use shacl2cypher_runner::backend::{self, BackendConfig};
use shacl2cypher_runner::executor::Executor;
use shacl2cypher_runner::remote::HttpFetcher;
use shacl2cypher_runner::report::{self, Format};
use shacl2cypher_runner::session;
use shacl2cypher_runner::validate::{exit_code, validate, FailOn, ValidateOptions};

/// Exit code for usage and setup errors of `validate` and `schema dump`.
const SETUP_ERROR: u8 = 2;

/// Compile SHACL shapes into named Cypher diagnostic queries.
#[derive(Parser)]
#[command(name = "shacl2cypher", version, about, arg_required_else_help = true)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Compile shapes files into `manifest.json` and `queries.cypher`.
    Compile(CompileArgs),
    /// Validate a database against shapes files or a compiled manifest.
    Validate(ValidateArgs),
    /// Database schema snapshots.
    #[command(subcommand)]
    Schema(SchemaCommand),
}

#[derive(Subcommand)]
enum SchemaCommand {
    /// Write a schema snapshot of a database, usable with `compile --schema`.
    Dump(DumpArgs),
}

#[derive(Args)]
struct CompileArgs {
    /// Shapes files (Turtle, N-Triples or TriG).
    #[arg(required = true)]
    shapes: Vec<PathBuf>,
    /// Target Cypher dialect.
    #[arg(long, value_enum)]
    dialect: DialectArg,
    #[command(flatten)]
    flags: CompileFlags,
    /// Output directory.
    #[arg(short, long, default_value = ".")]
    output: PathBuf,
}

#[derive(Args)]
struct CompileFlags {
    /// Schema snapshot JSON (required for ladybug when compiling).
    #[arg(long)]
    schema: Option<PathBuf>,
    /// Ontology files contributing `rdfs:subClassOf` statements.
    #[arg(long)]
    ontology: Vec<PathBuf>,
    /// Property identifying nodes when a shape has no `s2c:key`.
    #[arg(long)]
    node_key: Option<String>,
    /// How class targets match subclass labels on Neo4j.
    #[arg(long, value_enum, default_value = "explicit")]
    neo4j_labels: LabelsArg,
    /// Fail for classes and paths resolved by convention alone.
    #[arg(long)]
    strict: bool,
    /// Record unsupported features as `unsupported` rules instead of failing.
    #[arg(long)]
    lenient: bool,
    /// Include all focus properties in detail rows.
    #[arg(long)]
    verbose: bool,
    /// Upper bound for unbounded repeated paths.
    #[arg(long, default_value_t = 10)]
    max_path_depth: u32,
    /// Fetch `owl:imports` of http(s) IRIs.
    #[arg(long)]
    allow_remote_imports: bool,
    /// Turn static schema diagnostics into a compile error.
    #[arg(long)]
    fail_on_schema_mismatch: bool,
}

#[derive(Args)]
struct BackendArgs {
    /// Neo4j Bolt URI, e.g. bolt://localhost:7687.
    #[arg(long, value_name = "URI", conflicts_with = "ladybug")]
    connect: Option<String>,
    /// Neo4j user.
    #[arg(long, env = "NEO4J_USER", default_value = "neo4j")]
    user: String,
    /// Neo4j password.
    #[arg(
        long,
        env = "NEO4J_PASSWORD",
        default_value = "",
        hide_env_values = true
    )]
    password: String,
    /// Neo4j database name (server default when omitted).
    #[arg(long)]
    database: Option<String>,
    /// LadybugDB database file, opened read-only.
    #[arg(long, value_name = "FILE")]
    ladybug: Option<PathBuf>,
}

#[derive(Args)]
struct ValidateArgs {
    /// Shapes files to compile for the connected database.
    #[arg(required_unless_present = "manifest", conflicts_with = "manifest")]
    shapes: Vec<PathBuf>,
    /// Run a manifest written by `compile` instead of compiling shapes.
    #[arg(long)]
    manifest: Option<PathBuf>,
    #[command(flatten)]
    flags: CompileFlags,
    #[command(flatten)]
    backend: BackendArgs,
    /// Report format.
    #[arg(long, value_enum, default_value = "table")]
    format: FormatArg,
    /// Write the report to a file instead of stdout.
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Maximum violations listed per rule; 0 lists all.
    #[arg(long, default_value_t = 100)]
    limit: u64,
    /// Focus samples per summary row.
    #[arg(long, default_value_t = 5)]
    sample_size: u32,
    /// Per-query timeout in seconds.
    #[arg(long, value_name = "SECONDS")]
    timeout: Option<f64>,
    /// Lowest severity whose violations fail the run.
    #[arg(long, value_enum, default_value = "violation")]
    fail_on: FailOnArg,
}

#[derive(Args)]
struct DumpArgs {
    #[command(flatten)]
    backend: BackendArgs,
    /// Write the snapshot to a file instead of stdout.
    #[arg(short, long)]
    output: Option<PathBuf>,
}

#[derive(Clone, Copy, ValueEnum)]
enum DialectArg {
    Neo4j,
    Ladybug,
}

#[derive(Clone, Copy, ValueEnum)]
enum LabelsArg {
    Explicit,
    Inherited,
}

#[derive(Clone, Copy, ValueEnum)]
enum FormatArg {
    Table,
    Json,
    Junit,
    Sarif,
}

#[derive(Clone, Copy, ValueEnum)]
enum FailOnArg {
    Violation,
    Warning,
    Info,
}

/// An error message with the exit code it ends the process with.
struct Failure {
    code: u8,
    message: String,
}

fn setup(message: impl std::fmt::Display) -> Failure {
    Failure {
        code: SETUP_ERROR,
        message: message.to_string(),
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let result = match cli.command {
        Command::Compile(args) => run_compile(args).map_err(|message| Failure { code: 1, message }),
        Command::Validate(args) => run_validate(args),
        Command::Schema(SchemaCommand::Dump(args)) => run_dump(args).map(|()| 0),
    };
    match result {
        Ok(code) => ExitCode::from(code),
        Err(failure) => {
            for line in failure.message.lines() {
                eprintln!("error: {line}");
            }
            ExitCode::from(failure.code)
        }
    }
}

fn compile_options(flags: &CompileFlags, dialect: Dialect) -> CompileOptions {
    CompileOptions {
        dialect,
        node_key: flags.node_key.clone(),
        label_policy: match flags.neo4j_labels {
            LabelsArg::Explicit => LabelPolicy::Explicit,
            LabelsArg::Inherited => LabelPolicy::Inherited,
        },
        strict: flags.strict,
        lenient: flags.lenient,
        verbose: flags.verbose,
        max_path_depth: flags.max_path_depth,
        allow_remote_imports: flags.allow_remote_imports,
        fail_on_schema_mismatch: flags.fail_on_schema_mismatch,
        base_dir: std::env::current_dir().ok(),
    }
}

fn read(path: &PathBuf) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

// @lat: [[architecture#CLI]]
fn run_compile(args: CompileArgs) -> Result<u8, String> {
    let schema = args.flags.schema.as_ref().map(read).transpose()?;
    let dialect = match args.dialect {
        DialectArg::Neo4j => Dialect::Neo4j,
        DialectArg::Ladybug => Dialect::Ladybug,
    };
    let request = CompileRequest {
        shapes: &args.shapes,
        ontologies: &args.flags.ontology,
        schema: schema.as_deref(),
        fetcher: Some(&HttpFetcher),
        ..CompileRequest::default()
    };
    let compilation =
        compile(&request, &compile_options(&args.flags, dialect)).map_err(|e| e.to_string())?;

    let out = &args.output;
    let write = |name: &str, contents: &str| {
        let path = out.join(name);
        std::fs::write(&path, contents).map_err(|e| format!("{}: {e}", path.display()))
    };
    std::fs::create_dir_all(out).map_err(|e| format!("{}: {e}", out.display()))?;
    write("manifest.json", &compilation.manifest_json())?;
    write("queries.cypher", &compilation.cypher)?;

    let manifest = &compilation.manifest;
    let with_queries = manifest
        .rules
        .iter()
        .filter(|rule| rule.queries.is_some())
        .count();
    eprintln!(
        "compiled {} rules ({with_queries} with queries, {} static diagnostics) for {} into {}",
        manifest.rules.len(),
        manifest.static_diagnostics.len(),
        manifest.dialect,
        out.display()
    );
    print_diagnostics(manifest);
    Ok(0)
}

fn print_diagnostics(manifest: &Manifest) {
    for diagnostic in &manifest.static_diagnostics {
        eprintln!(
            "warning: {}:{}: {}: {}",
            diagnostic.source.file, diagnostic.source.line, diagnostic.code, diagnostic.message
        );
    }
}

/// Opens the database named by `--connect` or `--ladybug`.
// @lat: [[architecture#Runner#Backends]]
fn open_backend(args: &BackendArgs) -> Result<Box<dyn Executor>, Failure> {
    backend::require_any_backend().map_err(setup)?;
    let config = match (&args.connect, &args.ladybug) {
        (Some(uri), _) => BackendConfig::Neo4j {
            uri: uri.clone(),
            user: args.user.clone(),
            password: args.password.clone(),
            database: args.database.clone(),
        },
        (None, Some(path)) => BackendConfig::Ladybug { path: path.clone() },
        (None, None) => {
            return Err(setup(
                "name a database with `--connect <bolt-uri>` or `--ladybug <database file>`",
            ))
        }
    };
    backend::open(&config).map_err(setup)
}

fn write_output(output: Option<&PathBuf>, contents: &str) -> Result<(), Failure> {
    match output {
        Some(path) => {
            std::fs::write(path, contents).map_err(|e| setup(format!("{}: {e}", path.display())))
        }
        None => {
            print!("{contents}");
            Ok(())
        }
    }
}

// @lat: [[architecture#Runner#Validate Flow]]
fn run_validate(args: ValidateArgs) -> Result<u8, Failure> {
    let timeout = match args.timeout {
        Some(seconds) => Some(
            session::positive_timeout(seconds)
                .ok_or_else(|| setup("--timeout must be a positive number of seconds"))?,
        ),
        None => None,
    };
    let mut executor = open_backend(&args.backend)?;
    let dialect = executor.dialect();

    let manifest = match &args.manifest {
        Some(path) => Manifest::from_json(&read(path).map_err(setup)?)
            .map_err(|e| setup(format!("{}: {e}", path.display())))?,
        None => {
            let schema = args
                .flags
                .schema
                .as_ref()
                .map(read)
                .transpose()
                .map_err(setup)?;
            let request = CompileRequest {
                shapes: &args.shapes,
                ontologies: &args.flags.ontology,
                schema: schema.as_deref(),
                fetcher: Some(&HttpFetcher),
                ..CompileRequest::default()
            };
            session::compile_for(
                executor.as_mut(),
                &request,
                &compile_options(&args.flags, dialect),
            )
            .map_err(setup)?
        }
    };
    print_diagnostics(&manifest);

    let options = ValidateOptions {
        limit: (args.limit > 0).then_some(args.limit),
        sample_size: args.sample_size,
        timeout,
    };
    let report = validate(&manifest, executor.as_mut(), &options).map_err(setup)?;
    let format = match args.format {
        FormatArg::Table => Format::Table,
        FormatArg::Json => Format::Json,
        FormatArg::Junit => Format::Junit,
        FormatArg::Sarif => Format::Sarif,
    };
    write_output(args.output.as_ref(), &report::render(&report, format))?;
    let fail_on = match args.fail_on {
        FailOnArg::Violation => FailOn::Violation,
        FailOnArg::Warning => FailOn::Warning,
        FailOnArg::Info => FailOn::Info,
    };
    Ok(exit_code(&report, fail_on))
}

// @lat: [[architecture#Runner#Schema Dump]]
fn run_dump(args: DumpArgs) -> Result<(), Failure> {
    let mut executor = open_backend(&args.backend)?;
    let mut json = executor.schema().map_err(setup)?.to_json();
    json.push('\n');
    write_output(args.output.as_ref(), &json)
}
