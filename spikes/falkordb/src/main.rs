//! Probes the `falkordb` crate against a local FalkorDB (add-falkordb-dialect task 1.8):
//! read-only queries with parameters and timeouts, value decoding, and how server and
//! connection errors surface. Expects the probe graphs from `probes/02` and `probes/03`.

use std::time::{Duration, Instant};

use falkordb::{FalkorClientBuilder, FalkorConnectionInfo, FalkorSyncClient};

const URL: &str = "redis://127.0.0.1:6379";
const SLOW: &str = "UNWIND range(1, 20000000) AS x WITH x WHERE x % 7 = 0 RETURN count(x)";

fn client(url: &str, response_timeout: Option<Duration>) -> Result<FalkorSyncClient, String> {
    let info: FalkorConnectionInfo = url.try_into().map_err(|e| format!("url: {e:?}"))?;
    FalkorClientBuilder::new()
        .with_connection_info(info)
        .with_response_timeout(response_timeout)
        .build()
        .map_err(|e| format!("build: {e:?} / {e}"))
}

fn probe(client: &FalkorSyncClient, graph: &str, label: &str, query: &str, timeout: Option<i64>) {
    let mut graph = client.select_graph(graph);
    let started = Instant::now();
    let builder = graph
        .ro_query(query)
        .with_param("limit", i64::MAX)
        .with_param("sampleSize", 5i64);
    let builder = match timeout {
        Some(ms) => builder.with_timeout(ms),
        None => builder,
    };
    println!("\n== {label}: {query} (timeout {timeout:?})");
    match builder.execute() {
        Ok(result) => {
            println!("header: {:?}", result.header);
            for row in result.data {
                match row {
                    Ok(row) => println!("row: {:?}", row.into_values()),
                    Err(e) => println!("row error: {e:?}"),
                }
            }
        }
        Err(e) => println!("error: {e:?}\ndisplay: {e}"),
    }
    println!("elapsed: {:?}", started.elapsed());
}

fn main() {
    let c = match client(URL, None) {
        Ok(c) => c,
        Err(e) => {
            println!("connect error: {e}");
            return;
        }
    };
    println!("graphs: {:?}", c.list_graphs());
    probe(&c, "probe2", "params", "RETURN $limit AS l, $sampleSize AS s", Some(0));
    probe(&c, "probe2", "unused params", "RETURN 1 AS one", Some(0));
    probe(&c, "probe2", "nodes", "MATCH (n:Person) RETURN n ORDER BY n.id LIMIT $limit", Some(0));
    probe(&c, "probe2", "edge", "MATCH (s)-[r:KNOWS]->(e) WHERE id(r) >= 0 RETURN r, type(r), toString(id(r)) LIMIT 1", Some(0));
    probe(&c, "probe2", "path", "MATCH p = (a:Person {id: 'a'})-[:KNOWS]->(b) RETURN p LIMIT 1", Some(0));
    probe(&c, "probe2", "scalars", "RETURN null AS n, [1, 'a', null] AS l, {a: 1, b: [true]} AS m, 1.5 AS f, -9223372036854775808 AS min, 9223372036854775807 AS max, 'é😀\n' AS s", Some(0));
    probe(&c, "probe2", "temporals", "RETURN date('2020-01-01') AS d, date('1960-06-15') AS old, localdatetime('2020-01-01T10:00:00') AS ldt, localtime('10:30:15') AS lt, duration('P1DT2H') AS du, point({latitude: 1.5, longitude: 2.5}) AS p, vecf32([1.0, 2.0]) AS v", Some(0));
    probe(&c, "probe2", "summary", "MATCH (s0)-[v0:KNOWS]->(e0) WITH s0, v0, e0 RETURN count(v0) AS violations, collect({type: type(v0), elementId: toString(id(v0))})[0..$sampleSize] AS sample", Some(0));
    probe(&c, "probe2", "server timeout", SLOW, Some(50));
    probe(&c, "probe2", "timeout 0", SLOW, Some(0));
    probe(&c, "probe2", "no timeout argument", SLOW, None);
    probe(&c, "probe2", "write refused", "CREATE (:W)", Some(0));
    probe(&c, "probe2", "syntax error", "MATCH (n:A|B) RETURN n", Some(0));
    probe(&c, "probe2", "invalid regex", "RETURN string.matchRegEx('a', '(')", Some(0));
    probe(&c, "probe3", "type error", "MATCH (n:V) RETURN size(n.v)", Some(0));
    probe(&c, "missing_graph_rust", "missing graph", "RETURN 1", Some(0));
    println!("\ngraphs after missing-graph probe: {:?}", c.list_graphs());
    match client(URL, Some(Duration::from_millis(300))) {
        Ok(fast) => probe(&fast, "probe2", "client response timeout", SLOW, Some(0)),
        Err(e) => println!("\n== client response timeout: {e}"),
    }
    for (label, url) in [
        ("unreachable", "redis://127.0.0.1:1"),
        ("bad auth", "redis://nobody:secret@127.0.0.1:6379"),
        ("tls url", "rediss://127.0.0.1:6379"),
    ] {
        match client(url, None) {
            Ok(c) => probe(&c, "probe2", label, "RETURN 1", Some(0)),
            Err(e) => println!("\n== {label}: {e}"),
        }
    }
}
