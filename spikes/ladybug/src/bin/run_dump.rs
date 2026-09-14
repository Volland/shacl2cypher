// Runs rendered LadybugDB queries (written by the renderer tests to
// S2C_RENDER_DUMP_LADYBUG) against the same sample data as the Neo4j smoke run.

use lbug::{Connection, Database, SystemConfig, Value};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dump = std::path::PathBuf::from(std::env::args().nth(1).expect("dump directory"));
    // Optional second argument: print every detail row of this rule id.
    let show = std::env::args().nth(2);
    let dir = std::env::temp_dir().join("s2c-spike-ladybug-dump");
    // A LadybugDB database is a single file plus a write-ahead log.
    let _ = std::fs::remove_file(&dir);
    let _ = std::fs::remove_file(dir.with_extension("wal"));
    let _ = std::fs::remove_dir_all(&dir);
    let db = Database::new(&dir, SystemConfig::default())?;
    let conn = Connection::new(&db)?;
    for statement in [
        "CREATE NODE TABLE Person(id STRING, name STRING, age INT64, tags STRING[], start INT64, `end` INT64, email STRING, phone STRING, extra BOOL, PRIMARY KEY(id))",
        "CREATE NODE TABLE Company(id STRING, PRIMARY KEY(id))",
        "CREATE NODE TABLE Location(id STRING, zip STRING, PRIMARY KEY(id))",
        "CREATE REL TABLE WORKS_FOR(FROM Person TO Company, FROM Person TO Person)",
        "CREATE REL TABLE ADDRESS(FROM Person TO Location)",
        "CREATE REL TABLE KNOWS(FROM Person TO Person, since DATE)",
        "CREATE (:Person {id: 'p1', name: 'ann', age: 200, tags: ['a', 'z'], start: 5, `end`: 3, email: 'a@x'})",
        "CREATE (:Person {id: 'p2', name: 'Bob', age: 30, phone: '1'})",
        "CREATE (:Person {id: 'p3', extra: true})",
        "CREATE (:Company {id: 'c1'})",
        "CREATE (:Company {id: 'c2'})",
        "CREATE (:Company {id: 'c3'})",
        "CREATE (:Location {id: 'ad1', zip: '1234'})",
        "MATCH (a:Person {id: 'p1'}), (c:Company {id: 'c1'}) CREATE (a)-[:WORKS_FOR]->(c)",
        "MATCH (a:Person {id: 'p1'}), (c:Company {id: 'c2'}) CREATE (a)-[:WORKS_FOR]->(c)",
        "MATCH (a:Person {id: 'p1'}), (d:Location {id: 'ad1'}) CREATE (a)-[:ADDRESS]->(d)",
        "MATCH (a:Person {id: 'p1'}), (b:Person {id: 'p2'}) CREATE (a)-[:KNOWS {since: date('2020-01-01')}]->(b)",
        "MATCH (b:Person {id: 'p2'}), (c:Person {id: 'p3'}) CREATE (b)-[:KNOWS]->(c)",
        "MATCH (b:Person {id: 'p2'}) CREATE (b)-[:WORKS_FOR]->(b)",
    ] {
        conn.query(statement)?;
    }

    let mut ids: Vec<_> = std::fs::read_dir(&dump)?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.extension().is_some_and(|e| e == "id"))
        .collect();
    ids.sort();
    for id_path in ids {
        let id = std::fs::read_to_string(&id_path)?;
        for kind in ["detail", "summary"] {
            let query = std::fs::read_to_string(id_path.with_extension(format!("{kind}.cypher")))?;
            let mut params = Vec::new();
            if query.contains("$limit") {
                params.push(("limit", Value::Int64(100)));
            }
            if query.contains("$sampleSize") {
                params.push(("sampleSize", Value::Int64(3)));
            }
            let outcome = conn
                .prepare(&query)
                .and_then(|mut prepared| conn.execute(&mut prepared, params));
            match outcome {
                Ok(rows) => {
                    let rows: Vec<Vec<Value>> = rows.collect();
                    if kind == "summary" {
                        let mut first = format!("{:?}", rows.first());
                        first.truncate(140);
                        println!("ok   {id:<52} {kind} {first}");
                    } else {
                        println!("ok   {id:<52} {kind} rows={}", rows.len());
                    }
                    if show.as_deref() == Some(id.as_str()) && kind == "detail" {
                        for row in &rows {
                            println!("       {row:?}");
                        }
                    }
                }
                Err(error) => {
                    let mut message = error.to_string().replace('\n', " ");
                    message.truncate(260);
                    println!("ERR  {id:<52} {kind} {message}");
                }
            }
        }
    }
    Ok(())
}
