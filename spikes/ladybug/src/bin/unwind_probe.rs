// Isolates a LadybugDB issue: list values from one row leaking into rows whose
// list column is NULL when using coalesce/list_filter/UNWIND.

use lbug::{Connection, Database, SystemConfig};

fn probe(conn: &Connection, label: &str, q: &str) {
    match conn.query(q) {
        Ok(rows) => {
            let rows: Vec<String> = rows.map(|r| format!("{r:?}")).collect();
            println!("OK   [{label}] {}", rows.join(" | "));
        }
        Err(e) => println!("ERR  [{label}] {}", e.to_string().replace('\n', " ")),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::temp_dir().join("s2c-spike-ladybug-unwind");
    let _ = std::fs::remove_file(&path);
    let _ = std::fs::remove_file(path.with_extension("wal"));
    let _ = std::fs::remove_dir_all(&path);
    let db = Database::new(&path, SystemConfig::default())?;
    let conn = Connection::new(&db)?;
    for q in [
        "CREATE NODE TABLE Person(id STRING, tags STRING[], PRIMARY KEY(id))",
        "CREATE (:Person {id: 'p1', tags: ['a', 'z']})",
        "CREATE (:Person {id: 'p2'})",
        "CREATE (:Person {id: 'p3'})",
    ] {
        conn.query(q)?;
    }

    probe(&conn, "A raw column", "MATCH (v0:Person) RETURN v0.id, v0.tags ORDER BY v0.id");
    probe(&conn, "B coalesce", "MATCH (v0:Person) RETURN v0.id, coalesce(v0.tags, CAST([] AS STRING[])) ORDER BY v0.id");
    probe(&conn, "C list_filter(coalesce)", "MATCH (v0:Person) RETURN v0.id, list_filter(coalesce(v0.tags, CAST([] AS STRING[])), x -> x IS NOT NULL) ORDER BY v0.id");
    probe(&conn, "D CASE", "MATCH (v0:Person) RETURN v0.id, CASE WHEN v0.tags IS NULL THEN CAST([] AS STRING[]) ELSE v0.tags END ORDER BY v0.id");
    probe(&conn, "E UNWIND raw", "MATCH (v0:Person) UNWIND v0.tags AS v1 RETURN v0.id, v1 ORDER BY v0.id, v1");
    probe(&conn, "F UNWIND coalesce", "MATCH (v0:Person) UNWIND coalesce(v0.tags, CAST([] AS STRING[])) AS v1 RETURN v0.id, v1 ORDER BY v0.id, v1");
    probe(&conn, "G UNWIND list_filter(coalesce)", "MATCH (v0:Person) UNWIND list_filter(coalesce(v0.tags, CAST([] AS STRING[])), x -> x IS NOT NULL) AS v1 RETURN v0.id, v1 ORDER BY v0.id, v1");
    probe(&conn, "H UNWIND list_filter(raw)", "MATCH (v0:Person) UNWIND list_filter(v0.tags, x -> x IS NOT NULL) AS v1 RETURN v0.id, v1 ORDER BY v0.id, v1");
    probe(&conn, "I UNWIND CASE", "MATCH (v0:Person) UNWIND CASE WHEN v0.tags IS NULL THEN CAST([] AS STRING[]) ELSE v0.tags END AS v1 RETURN v0.id, v1 ORDER BY v0.id, v1");
    probe(&conn, "J WITH then UNWIND", "MATCH (v0:Person) WITH v0, list_filter(coalesce(v0.tags, CAST([] AS STRING[])), x -> x IS NOT NULL) AS l UNWIND l AS v1 RETURN v0.id, v1 ORDER BY v0.id, v1");
    probe(&conn, "K WHERE IS NOT NULL then UNWIND raw", "MATCH (v0:Person) WHERE v0.tags IS NOT NULL UNWIND v0.tags AS v1 RETURN v0.id, v1 ORDER BY v0.id, v1");
    probe(&conn, "L G + WITH DISTINCT", "MATCH (v0:Person) UNWIND list_filter(coalesce(v0.tags, CAST([] AS STRING[])), x -> x IS NOT NULL) AS v1 WITH DISTINCT v0, v1 RETURN v0.id, v1 ORDER BY v0.id, v1");
    probe(&conn, "M E + WITH DISTINCT + guard", "MATCH (v0:Person) UNWIND v0.tags AS v1 WITH DISTINCT v0, v1 WHERE v1 IS NOT NULL RETURN v0.id, v1 ORDER BY v0.id, v1");
    probe(&conn, "N list_distinct size", "MATCH (v0:Person) RETURN v0.id, size(list_distinct(list_filter(coalesce(v0.tags, CAST([] AS STRING[])), x -> x IS NOT NULL))) ORDER BY v0.id");
    probe(&conn, "O all() over list_filter", "MATCH (v0:Person) RETURN v0.id, all(y IN list_filter(coalesce(v0.tags, CAST([] AS STRING[])), x -> x IS NOT NULL) WHERE y = 'a') ORDER BY v0.id");
    Ok(())
}
