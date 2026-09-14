//! Round-trips random strings, identifiers and typed constants through the literal
//! renderer and both databases.
#![cfg(any(feature = "ladybug", feature = "neo4j"))]

use s2c_core::ir::Constant;
use s2c_core::render::{constant, ident, quote, Dialect};
use s2c_runner::executor::{Executor, Params, Row};
use serde_json::Value;

const PARAMS: Params = Params {
    limit: 1,
    sample_size: 1,
};

/// Deterministic xorshift generator so failures reproduce.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

const PIECES: &[&str] = &[
    "a", "Z", "0", " ", "'", "\"", "\\", "`", "``", "$", "{", "}", "\n", "\t", "\r", "é", "中",
    "😀", "\\'", "''", "/*", "*/", "//", "%", "_", "\u{2028}", "\\u0041", ";",
];

fn random_text(rng: &mut Rng) -> String {
    let len = rng.below(12);
    (0..len).map(|_| PIECES[rng.below(PIECES.len())]).collect()
}

fn single(executor: &mut dyn Executor, query: &str) -> Result<Row, String> {
    let rows = executor
        .run(query, PARAMS, None)
        .map_err(|e| format!("{query}: {e}"))?;
    rows.into_iter()
        .next()
        .ok_or_else(|| format!("{query}: no row"))
}

fn to_string_expr(dialect: Dialect, literal: &str) -> String {
    match dialect {
        Dialect::Neo4j => format!("toString({literal})"),
        Dialect::Ladybug => format!("CAST({literal} AS STRING)"),
    }
}

/// Returns a description of every value that did not round-trip.
fn round_trip(executor: &mut dyn Executor) -> Vec<String> {
    let dialect = executor.dialect();
    let mut rng = Rng(0x5eed_c0de_2026);
    let mut failures = Vec::new();
    let mut expect =
        |query: String, check: &dyn Fn(&Row) -> bool, executor: &mut dyn Executor| match single(
            executor, &query,
        ) {
            Ok(row) if check(&row) => {}
            Ok(row) => failures.push(format!("{query} returned {row:?}")),
            Err(error) => failures.push(error),
        };

    for _ in 0..150 {
        let text = random_text(&mut rng);
        let expected = Value::String(text.clone());
        expect(
            format!("RETURN {} AS v", quote(&text)),
            &|row| row["v"] == expected,
            executor,
        );
        let rendered = constant(dialect, &Constant::String(text.clone())).unwrap();
        expect(
            format!("RETURN {rendered} AS v"),
            &|row| row["v"] == expected,
            executor,
        );
    }

    let mut rejected = 0;
    for _ in 0..100 {
        let name = format!("c{}", random_text(&mut rng));
        // Neo4j decodes `\uXXXX` inside backticks, so the renderer rejects such names.
        if dialect == Dialect::Neo4j && s2c_core::render::neo4j::unrepresentable_identifier(&name) {
            rejected += 1;
            continue;
        }
        expect(
            format!("RETURN 1 AS {}", ident(&name)),
            &|row| row.contains_key(&name),
            executor,
        );
    }
    if dialect == Dialect::Neo4j {
        assert!(rejected > 0, "the generator should produce `\\uXXXX` names");
    }

    let mut integers = vec![0i64, 1, -1, i64::MAX, i64::MIN, i64::MIN + 1, 1 << 53];
    integers.extend((0..40).map(|_| rng.next() as i64));
    for n in integers {
        let rendered = constant(dialect, &Constant::Integer(i128::from(n))).unwrap();
        expect(
            format!("RETURN {rendered} AS v"),
            &|row| row["v"].as_i64() == Some(n),
            executor,
        );
    }

    let mut doubles = vec![0.0f64, -0.0, 1.5, -2.25e-300, 1e308, f64::MIN_POSITIVE, 0.1];
    doubles.extend(
        (0..40)
            .map(|_| f64::from_bits(rng.next()))
            .filter(|x| x.is_finite()),
    );
    for x in doubles {
        let rendered = constant(dialect, &Constant::Double(format!("{x:?}"))).unwrap();
        expect(
            format!("RETURN {rendered} AS v"),
            &|row| row["v"].as_f64() == Some(x),
            executor,
        );
    }

    for b in [true, false] {
        let rendered = constant(dialect, &Constant::Boolean(b)).unwrap();
        expect(
            format!("RETURN {rendered} AS v"),
            &|row| row["v"] == Value::Bool(b),
            executor,
        );
    }

    for _ in 0..20 {
        let date = format!(
            "{:04}-{:02}-{:02}",
            1 + rng.below(9998),
            1 + rng.below(12),
            1 + rng.below(28)
        );
        let rendered = constant(dialect, &Constant::Date(date.clone())).unwrap();
        let expected = Value::String(date);
        expect(
            format!("RETURN {} AS v", to_string_expr(dialect, &rendered)),
            &|row| row["v"] == expected,
            executor,
        );
    }
    failures
}

// @lat: [[testing#Literal Round-Trip Fuzzing]]
#[cfg(feature = "ladybug")]
#[test]
fn literals_round_trip_on_ladybug() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("empty.lbug");
    {
        let db = lbug::Database::new(&path, lbug::SystemConfig::default()).unwrap();
        let conn = lbug::Connection::new(&db).unwrap();
        conn.query("CREATE NODE TABLE T(id STRING, PRIMARY KEY(id))")
            .unwrap();
    }
    let mut executor = s2c_runner::ladybug::LadybugExecutor::open(&path).unwrap();
    let failures = round_trip(&mut executor);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[cfg(feature = "neo4j")]
#[test]
fn literals_round_trip_on_neo4j() {
    let Ok(uri) = std::env::var("S2C_NEO4J_URI") else {
        eprintln!("skipping: S2C_NEO4J_URI is not set");
        return;
    };
    let config = s2c_runner::neo4j::Neo4jConfig {
        uri,
        user: std::env::var("S2C_NEO4J_USER").unwrap_or_else(|_| "neo4j".into()),
        password: std::env::var("S2C_NEO4J_PASSWORD").unwrap_or_default(),
        database: None,
    };
    let mut executor = s2c_runner::neo4j::Neo4jExecutor::connect(config).unwrap();
    let failures = round_trip(&mut executor);
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
