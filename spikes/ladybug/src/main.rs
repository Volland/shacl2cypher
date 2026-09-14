// Spike for tasks 1.1-1.3: LadybugDB Rust bindings, nested subqueries inside
// expressions, schema introspection and regex behavior. Each probe prints OK/ERR
// so one unsupported construct does not stop the run.

use lbug::{Connection, Database, SystemConfig};

fn probe(conn: &Connection, label: &str, q: &str) {
    match conn.query(q) {
        Ok(rows) => {
            let rows: Vec<String> = rows.map(|r| format!("{r:?}")).collect();
            let mut out = rows.join(" | ");
            if out.len() > 400 {
                out.truncate(400);
                out.push('…');
            }
            println!("OK   [{label}] {out}");
        }
        Err(e) => {
            let mut msg = e.to_string().replace('\n', " ");
            msg.truncate(300);
            println!("ERR  [{label}] {msg}");
        }
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join("s2c-spike-ladybug");
    let _ = std::fs::remove_dir_all(&dir);
    let db = Database::new(&dir, SystemConfig::default())?;
    let conn = Connection::new(&db)?;

    // 1.1 setup: typed node/rel tables and a small graph.
    for q in [
        "CREATE NODE TABLE Person(id STRING, name STRING, age INT64, tags STRING[], PRIMARY KEY(id))",
        "CREATE NODE TABLE Address(id STRING, zip STRING, PRIMARY KEY(id))",
        "CREATE REL TABLE LIVES_AT(FROM Person TO Address)",
        "CREATE REL TABLE KNOWS(FROM Person TO Person, since DATE)",
        "CREATE (:Person {id: 'p1', name: 'Ann', age: 30, tags: ['a', 'b']})",
        "CREATE (:Person {id: 'p2', age: 10, tags: []})",
        "CREATE (:Person {id: 'p3', name: 'Bob'})",
        "CREATE (:Address {id: 'a1', zip: '12345'})",
        "CREATE (:Address {id: 'a2'})",
        "MATCH (p:Person {id: 'p1'}), (a:Address {id: 'a1'}) CREATE (p)-[:LIVES_AT]->(a)",
        "MATCH (p:Person {id: 'p2'}), (a:Address {id: 'a2'}) CREATE (p)-[:LIVES_AT]->(a)",
        "MATCH (p:Person {id: 'p1'}), (q:Person {id: 'p2'}) CREATE (p)-[:KNOWS {since: date('2020-01-01')}]->(q)",
        "MATCH (p:Person {id: 'p2'}), (q:Person {id: 'p3'}) CREATE (p)-[:KNOWS]->(q)",
    ] {
        conn.query(q)?;
    }
    probe(&conn, "1.1 basic", "MATCH (n:Person) RETURN n.id, n.name ORDER BY n.id");

    println!("\n== 1.2 subqueries and expressions");
    probe(&conn, "COUNT{MATCH} in WHERE", "MATCH (n:Person) WHERE COUNT { MATCH (n)-[:LIVES_AT]->(:Address) } >= 1 RETURN n.id ORDER BY n.id");
    probe(&conn, "COUNT{pattern} neo4j-style", "MATCH (n:Person) WHERE COUNT { (n)-[:LIVES_AT]->() } >= 1 RETURN n.id");
    probe(&conn, "EXISTS{MATCH WHERE}", "MATCH (n:Person) WHERE EXISTS { MATCH (n)-[:LIVES_AT]->(a:Address) WHERE a.zip IS NULL } RETURN n.id");
    probe(&conn, "nested EXISTS", "MATCH (n:Person) WHERE EXISTS { MATCH (n)-[:LIVES_AT]->(a:Address) WHERE NOT EXISTS { MATCH (a)<-[:LIVES_AT]-(q:Person) WHERE q.name IS NOT NULL } } RETURN n.id");
    probe(&conn, "COUNT{} in RETURN bool", "MATCH (n:Person) RETURN n.id, (COUNT { MATCH (n)-[:KNOWS]->() } = 1) AS b ORDER BY n.id");
    probe(&conn, "subquery in OR/NOT", "MATCH (n:Person) WHERE NOT (n.name IS NOT NULL OR COUNT { MATCH (n)-[:KNOWS]->() } >= 1) RETURN n.id");
    probe(&conn, "qualified: COUNT with nested predicate", "MATCH (n:Person) RETURN n.id, COUNT { MATCH (n)-[:LIVES_AT]->(a:Address) WHERE coalesce(a.zip =~ '[0-9]{5}', false) } AS c ORDER BY n.id");
    probe(&conn, "list comprehension filter", "MATCH (n:Person) RETURN n.id, [x IN n.tags WHERE x IS NOT NULL] ORDER BY n.id");
    probe(&conn, "all() quantifier", "MATCH (n:Person) RETURN n.id, all(x IN n.tags WHERE x = 'a') ORDER BY n.id");
    probe(&conn, "size of filtered bool list (xone)", "RETURN size([b IN [true, false, true] WHERE b]) AS c");
    probe(&conn, "list_filter lambda", "RETURN list_filter([true, false, true], x -> x) AS l");
    probe(&conn, "pattern comprehension", "MATCH (n:Person) RETURN n.id, [(n)-[:KNOWS]->(m) | m.id] ORDER BY n.id");
    probe(&conn, "coalesce(cmp, false)", "MATCH (n:Person) RETURN n.id, coalesce(n.age >= 18, false) ORDER BY n.id");
    probe(&conn, "string vs int compare", "RETURN coalesce('abc' >= 18, false)");
    probe(&conn, "CASE list normalization", "MATCH (n:Person) RETURN n.id, CASE WHEN n.name IS NULL THEN [] ELSE [n.name] END ORDER BY n.id");
    probe(&conn, "CALL {} subquery", "MATCH (n:Person) CALL { WITH n MATCH (n)-[:KNOWS]->(m) RETURN count(m) AS c } RETURN n.id, c");
    probe(&conn, "OPTIONAL MATCH hoisting", "MATCH (n:Person) OPTIONAL MATCH (n)-[:KNOWS]->(m) WITH n, count(m) AS c RETURN n.id, c ORDER BY n.id");
    probe(&conn, "multi-table match", "MATCH (n:Person:Address) RETURN label(n), count(*) ORDER BY label(n)");
    probe(&conn, "label expr with |", "MATCH (n:Person|Address) RETURN count(*)");
    probe(&conn, "struct/map literal row", "MATCH (n:Person) RETURN {ruleId: 'r1', focus: {label: label(n), keyValue: n.id, elementId: id(n)}} AS v LIMIT 1");
    probe(&conn, "collect slice sample", "MATCH (n:Person) WITH count(*) AS c, collect(n.id) AS ids RETURN c, ids[1:2]");
    probe(&conn, "collect slice neo4j-style [0..2]", "MATCH (n:Person) WITH collect(n.id) AS ids RETURN ids[0..2]");
    probe(&conn, "zero-row summary aggregate", "MATCH (n:Person) WHERE n.id = 'nope' WITH count(*) AS c, collect(n.id) AS s RETURN 'r' AS ruleId, c, s");
    probe(&conn, "var-length bounded", "MATCH (n:Person {id: 'p1'})-[:KNOWS*1..10]->(m) RETURN m.id");
    probe(&conn, "inverse direction", "MATCH (a:Address)<-[:LIVES_AT]-(p:Person) RETURN a.id, p.id ORDER BY a.id");
    probe(&conn, "undeclared property (binder)", "MATCH (n:Person) RETURN n.nickname");
    probe(&conn, "undeclared table (binder)", "MATCH (n:Company) RETURN n");
    probe(&conn, "rel property null", "MATCH ()-[r:KNOWS]->() RETURN r.since IS NULL");
    probe(&conn, "LIMIT with param-like coalesce", "MATCH (n:Person) RETURN n.id LIMIT coalesce(NULL, 9223372036854775807)");
    probe(&conn, "string escape backslash+quote", r"RETURN 'O\'Brien \\ x' AS s");
    probe(&conn, "backtick identifier", "RETURN 1 AS `we``ird`");

    println!("\n== 1.3 introspection");
    probe(&conn, "show_tables", "CALL show_tables() RETURN *");
    probe(&conn, "table_info Person", "CALL table_info('Person') RETURN *");
    probe(&conn, "table_info KNOWS", "CALL table_info('KNOWS') RETURN *");
    probe(&conn, "show_connection", "CALL show_connection('LIVES_AT') RETURN *");

    println!("\n== 1.3 regex");
    probe(&conn, "=~ anchored?", "RETURN 'abc123' =~ '[0-9]+' AS m");
    probe(&conn, "regexp_matches substring", "RETURN regexp_matches('abc123', '[0-9]+') AS m");
    probe(&conn, "regexp_full_match", "RETURN regexp_full_match('abc123', '[0-9]+') AS m");
    probe(&conn, "backslash-d single escape", r"RETURN regexp_matches('abc123', '\d+') AS m");
    probe(&conn, "backslash-d double escape", r"RETURN regexp_matches('abc123', '\\d+') AS m");
    probe(&conn, "RE2 inline (?i)", "RETURN regexp_matches('ABC', '(?i)^abc$') AS m");
    probe(&conn, "RE2 (?s) dotall", "RETURN regexp_matches('a\nb', '(?s)a.b') AS m");
    probe(&conn, "unicode class", r"RETURN regexp_matches('Łódź', '^\\p{L}+$') AS m");
    probe(&conn, "backreference (expect error)", r"RETURN regexp_matches('aa', '(a)\\1') AS m");
    probe(&conn, "regex on NULL", "MATCH (n:Person) RETURN n.id, regexp_matches(n.name, 'A') ORDER BY n.id");

    println!("\n== types");
    probe(&conn, "typeof-like", "MATCH (n:Person) RETURN typeof(n.age) LIMIT 1");
    probe(&conn, "IS :: syntax", "MATCH (n:Person) RETURN n.age IS :: INT64 LIMIT 1");
    Ok(())
}
