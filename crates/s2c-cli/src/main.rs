use clap::Parser;

/// Compile SHACL shapes into named Cypher diagnostic queries.
#[derive(Parser)]
#[command(name = "shacl2cypher", version, about)]
struct Cli {}

fn main() {
    let _cli = Cli::parse();
    let backends = s2c_runner::available_backends();
    if backends.is_empty() {
        eprintln!("shacl2cypher: no subcommand given (database backends: none compiled in)");
    } else {
        eprintln!(
            "shacl2cypher: no subcommand given (database backends: {})",
            backends.join(", ")
        );
    }
}
