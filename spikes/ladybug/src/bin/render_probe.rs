// Spike for task group 6: LadybugDB constructs the renderer will emit.

use lbug::{Connection, Database, SystemConfig, Value};

fn show(label: &str, result: Result<lbug::QueryResult<'_>, lbug::Error>) {
    match result {
        Ok(rows) => {
            let rows: Vec<String> = rows.map(|r| format!("{r:?}")).collect();
            let mut out = rows.join(" | ");
            out.truncate(300);
            println!("OK   [{label}] {out}");
        }
        Err(e) => {
            let mut msg = e.to_string().replace('\n', " ");
            msg.truncate(260);
            println!("ERR  [{label}] {msg}");
        }
    }
}

fn probe(conn: &Connection, label: &str, q: &str) {
    show(label, conn.query(q));
}

fn probe_params(conn: &Connection, label: &str, q: &str, params: Vec<(&str, Value)>) {
    match conn.prepare(q) {
        Ok(mut prepared) => show(label, conn.execute(&mut prepared, params)),
        Err(e) => println!("ERR  [{label}] prepare: {}", e.to_string().replace('\n', " ")),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join("s2c-spike-ladybug-render");
    let _ = std::fs::remove_dir_all(&dir);
    let db = Database::new(&dir, SystemConfig::default())?;
    let conn = Connection::new(&db)?;
    for q in [
        "CREATE NODE TABLE Person(id STRING, name STRING, tags STRING[], age INT64, PRIMARY KEY(id))",
        "CREATE NODE TABLE Company(id STRING, name STRING, PRIMARY KEY(id))",
        "CREATE REL TABLE WORKS_FOR(FROM Person TO Company)",
        "CREATE REL TABLE KNOWS(FROM Person TO Person, since DATE)",
        "CREATE (:Person {id: 'p1', name: 'Ann', tags: ['a', 'a', 'b', NULL], age: 30})",
        "CREATE (:Person {id: 'p2', name: 'Bob'})",
        "CREATE (:Company {id: 'c1', name: 'Acme'})",
        "MATCH (a:Person {id: 'p1'}), (c:Company {id: 'c1'}) CREATE (a)-[:WORKS_FOR]->(c)",
        "MATCH (a:Person {id: 'p1'}), (c:Company {id: 'c1'}) CREATE (a)-[:WORKS_FOR]->(c)",
        "MATCH (a:Person {id: 'p1'}), (b:Person {id: 'p2'}) CREATE (a)-[:KNOWS {since: date('2020-01-01')}]->(b)",
    ] {
        conn.query(q)?;
    }

    probe(&conn, "raw newline in literal", "RETURN 'a\nb' AS r");
    probe(&conn, "unicode escape", r"RETURN 'aA' AS r");

    probe(&conn, "distinct list count", "MATCH (n:Person) RETURN n.id, size(list_distinct(list_filter(coalesce(n.tags, CAST([] AS STRING[])), x -> x IS NOT NULL))) ORDER BY n.id");
    probe(&conn, "COUNT rel ends (no RETURN)", "MATCH (n:Person) RETURN n.id, COUNT { MATCH (n)-[:WORKS_FOR]->(m) } ORDER BY n.id");
    probe(&conn, "COUNT DISTINCT rel ends", "MATCH (n:Person) RETURN n.id, COUNT { MATCH (n)-[:WORKS_FOR]->(m) RETURN DISTINCT m } ORDER BY n.id");
    probe(&conn, "COUNT DISTINCT in WHERE", "MATCH (n:Person) WHERE COUNT { MATCH (n)-[:WORKS_FOR]->(m) RETURN DISTINCT m } >= 1 RETURN n.id");

    probe(&conn, "per-value unwind rows", "MATCH (n:Person) UNWIND list_filter(coalesce(n.tags, CAST([] AS STRING[])), x -> x IS NOT NULL) AS v WITH DISTINCT n, v WHERE NOT (size(v) >= 1 AND v <> '') RETURN n.id, v");
    probe(&conn, "per-value scalar rows", "MATCH (n:Person) WITH n, n.name AS v WHERE v IS NOT NULL AND NOT (size(v) >= 4) RETURN n.id, v");
    probe(&conn, "per-value rel rows distinct", "MATCH (n:Person)-[:WORKS_FOR]->(v) WITH DISTINCT n, v RETURN count(*)");

    probe(&conn, "label membership", "MATCH (n:Person:Company) RETURN label(n) IN ['Person', 'Employee'], count(*) ORDER BY count(*)");
    probe(&conn, "multi-table missing column", "MATCH (n:Person:Company) RETURN n.id, n.age ORDER BY n.id");

    probe(&conn, "timestamp literal", "RETURN timestamp('2020-01-01 10:00:00') AS r");
    probe(&conn, "timestamp ISO Z", "RETURN timestamp('2020-01-01T10:00:00Z') AS r");
    probe(&conn, "timestamp_tz cast", "RETURN CAST('2020-01-01T10:00:00+01:00' AS TIMESTAMP_TZ) AS r");
    probe(&conn, "interval ISO", "RETURN interval('P1DT2H') AS r");
    probe(&conn, "interval words", "RETURN interval('1 day 2 hours') AS r");
    probe(&conn, "decimal cast", "RETURN CAST(1.50 AS DECIMAL(18, 2)) AS r");
    probe(&conn, "exponent literal", "RETURN 1.5e3 AS r");

    probe(&conn, "regex hex escapes", r"RETURN regexp_matches('b', '[\\x{62}-\\x{64}]'), regexp_matches('Ł', '[\\x{141}]'), regexp_matches('a&b', '[\\&]')");
    probe(&conn, "regex (?m)", "RETURN regexp_matches('X\nabc', '(?m)^abc'), regexp_matches('X\nabc', '^abc')");
    probe(&conn, "regex (?s) default", "RETURN regexp_matches('a\nb', 'a.b'), regexp_matches('a\nb', '(?s)a.b')");

    probe(&conn, "pair lists via list_contains", "RETURN all(x IN [1, 2] WHERE list_contains([2, 1], x)) AS r");
    probe(&conn, "list comparison < across lists", "RETURN all(x IN [1, 2] WHERE all(y IN [3, 4] WHERE x < y)) AS r");

    probe(&conn, "element id as string", "MATCH (n:Person) RETURN CAST(id(n) AS STRING) ORDER BY n.id LIMIT 1");
    probe(&conn, "rel id as string", "MATCH ()-[r:KNOWS]->() RETURN CAST(id(r) AS STRING), r.since");
    probe(&conn, "cast values to string", "RETURN CAST(1.5 AS STRING), CAST(date('2020-01-01') AS STRING), CAST(true AS STRING), CAST(30 AS STRING)");
    probe(&conn, "struct with null field", "MATCH (n:Person) RETURN {keyValue: n.id, missing: NULL} ORDER BY n.id LIMIT 1");

    probe_params(
        &conn,
        "zero-row summary with sample param",
        "MATCH (n:Person) WHERE n.id = 'nope' WITH count(n) AS c, collect({keyValue: n.id}) AS s RETURN c, list_slice(coalesce(s, []), 1, $sampleSize)",
        vec![("sampleSize", Value::Int64(2))],
    );
    probe_params(
        &conn,
        "sample slice with param",
        "MATCH (n:Person) WITH count(n) AS c, collect(n.id) AS s RETURN c, s[1:$sampleSize]",
        vec![("sampleSize", Value::Int64(1))],
    );
    probe_params(
        &conn,
        "LIMIT with param",
        "MATCH (n:Person) RETURN n.id LIMIT $limit",
        vec![("limit", Value::Int64(1))],
    );
    probe_params(
        &conn,
        "LIMIT with null param coalesce",
        "MATCH (n:Person) RETURN n.id LIMIT coalesce($limit, 9223372036854775807)",
        vec![("limit", Value::Null(lbug::LogicalType::Int64))],
    );
    Ok(())
}
