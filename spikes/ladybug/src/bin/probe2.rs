// Spike follow-up for task 1.2: confirm LadybugDB replacements for constructs the
// Neo4j renderer uses (list comprehensions, pattern comprehensions, CALL {}).

use lbug::{Connection, Database, SystemConfig};

fn probe(conn: &Connection, label: &str, q: &str) {
    match conn.query(q) {
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join("s2c-spike-ladybug-2");
    let _ = std::fs::remove_dir_all(&dir);
    let db = Database::new(&dir, SystemConfig::default())?;
    let conn = Connection::new(&db)?;
    for q in [
        "CREATE NODE TABLE Person(id STRING, name STRING, tags STRING[], PRIMARY KEY(id))",
        "CREATE NODE TABLE Address(id STRING, zip STRING, PRIMARY KEY(id))",
        "CREATE REL TABLE LIVES_AT(FROM Person TO Address)",
        "CREATE REL TABLE KNOWS(FROM Person TO Person)",
        "CREATE (:Person {id: 'p1', name: 'Ann', tags: ['a', NULL, '']})",
        "CREATE (:Person {id: 'p2'})",
        "CREATE (:Address {id: 'a1', zip: '12345'})",
        "CREATE (:Address {id: 'a2'})",
        "MATCH (p:Person {id: 'p1'}), (a:Address) CREATE (p)-[:LIVES_AT]->(a)",
        "MATCH (p:Person {id: 'p1'}), (q:Person {id: 'p2'}) CREATE (p)-[:KNOWS]->(q)",
    ] {
        conn.query(q)?;
    }

    probe(&conn, "xone via size(list_filter)", "RETURN size(list_filter([true, false, true], b -> b)) = 1 AS xone");
    probe(&conn, "details list building", "RETURN list_filter([CASE WHEN false THEN 'r1' END, CASE WHEN true THEN 'r2' END], x -> x IS NOT NULL) AS d");
    probe(&conn, "value set drop nulls", "MATCH (n:Person) RETURN n.id, list_filter(n.tags, x -> x IS NOT NULL) ORDER BY n.id");
    probe(&conn, "all() over filtered value set", "MATCH (n:Person) RETURN n.id, all(v IN list_filter(n.tags, x -> x IS NOT NULL) WHERE coalesce(size(v) >= 1, false)) ORDER BY n.id");
    probe(&conn, "all() over NULL list", "MATCH (n:Person) RETURN n.id, all(v IN coalesce(n.tags, CAST([] AS STRING[])) WHERE size(v) >= 1) ORDER BY n.id");
    probe(&conn, "typed empty list cast", "RETURN CAST([] AS STRING[]) AS l");
    probe(&conn, "size on NULL string", "MATCH (n:Person) RETURN n.id, coalesce(size(n.name) >= 1, false) ORDER BY n.id");
    probe(&conn, "EXISTS inside CASE", "MATCH (n:Person) RETURN n.id, CASE WHEN EXISTS { MATCH (n)-[:KNOWS]->() } THEN 'yes' ELSE 'no' END ORDER BY n.id");
    probe(&conn, "conforms inlined in COUNT (2 levels)", "MATCH (n:Person) RETURN n.id, COUNT { MATCH (n)-[:LIVES_AT]->(a:Address) WHERE NOT (coalesce(a.zip =~ '[0-9]{5}', false) AND EXISTS { MATCH (a)<-[:LIVES_AT]-(q:Person) WHERE q.name IS NOT NULL }) } ORDER BY n.id");
    probe(&conn, "3-level nested subquery", "MATCH (n:Person) WHERE EXISTS { MATCH (n)-[:KNOWS]->(m) WHERE NOT EXISTS { MATCH (m)-[:LIVES_AT]->(a) WHERE COUNT { MATCH (a)<-[:LIVES_AT]-() } >= 1 } } RETURN n.id");
    probe(&conn, "subquery inside list literal (details)", "MATCH (n:Person) RETURN n.id, list_filter([CASE WHEN NOT EXISTS { MATCH (n)-[:LIVES_AT]->() } THEN 'r1' END], x -> x IS NOT NULL) ORDER BY n.id");
    probe(&conn, "coalesce(collect) on zero rows", "MATCH (n:Person) WHERE n.id = 'nope' WITH count(*) AS c, coalesce(collect(n.id), CAST([] AS STRING[])) AS s RETURN c, s");
    probe(&conn, "collect struct sample slice", "MATCH (n:Person) WITH count(*) AS c, collect({keyValue: n.id, elementId: id(n)}) AS s RETURN c, s[1:1]");
    probe(&conn, "var-length *1..30", "MATCH (n:Person {id: 'p1'})-[:KNOWS*1..30]->(m) RETURN count(m)");
    probe(&conn, "var-length *1..100", "MATCH (n:Person {id: 'p1'})-[:KNOWS*1..100]->(m) RETURN count(m)");
    probe(&conn, "zero-or-more *0..10", "MATCH (n:Person {id: 'p1'})-[:KNOWS*0..10]->(m) RETURN count(m)");
    probe(&conn, "label() for closed/focus", "MATCH (n) RETURN label(n), count(*) ORDER BY label(n)");
    probe(&conn, "multiline flag (?m)", "RETURN regexp_matches('x\nabc', '(?m)^abc') AS m");
    probe(&conn, "string replace for message", "RETURN replace(replace('Person {$this} {?value}', '{$this}', 'p7'), '{?value}', CAST(-3 AS STRING)) AS m");
    probe(&conn, "backtick table name with space", "CREATE NODE TABLE `Weird Name`(id STRING, PRIMARY KEY(id))");
    probe(&conn, "backtick table name with backtick", "CREATE NODE TABLE `We``ird`(id STRING, PRIMARY KEY(id))");

    drop(conn);
    drop(db);
    let ro = Database::new(&dir, SystemConfig::default().read_only(true))?;
    let ro_conn = Connection::new(&ro)?;
    probe(&ro_conn, "read-only open: read", "MATCH (n:Person) RETURN count(*)");
    probe(&ro_conn, "read-only open: write rejected", "CREATE (:Person {id: 'p9'})");
    Ok(())
}
