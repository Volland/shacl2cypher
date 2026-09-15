#!/usr/bin/env bash
# Probe batch 6: nested quantifiers computed in WITH projections, pattern-comprehension
# variable collisions and null projection, UNWIND over comprehensions for per-value rows,
# variable-length MATCH collapsing, CALL {} with relationship variables, and the parameter
# probes batch 4 lost. Uses graph `probe2` (a -KNOWS x2-> b, a -KNOWS-> c, b -KNOWS-> c,
# c -WORKS_FOR-> x; c is Person:Employee with name 'Cy'; a has name 'Ann').
# One read-only query per line; `#` headers. No eval, so `$params` stay literal.
set -u
PODMAN=${PODMAN:-/opt/podman/bin/podman}
q() {
  printf '\n>> %s\n' "$1"
  "$PODMAN" exec s2c-falkordb redis-cli GRAPH.RO_QUERY probe2 "$1" 2>&1
}
while IFS= read -r line; do
  case "$line" in
    '') ;;
    '#'*) printf '\n==== %s\n' "$line" ;;
    *) q "$line" ;;
  esac
done <<'EOF'
# nested quantifiers in projection context (expected allOut: a false (c has no KNOWS out), b false, c true (vacuous))
MATCH (n:Person) WITH n, all(y IN [(n)-[:KNOWS]->(y) | y] WHERE size([(y)-[:KNOWS]->(z) | 1]) > 0) AS ok RETURN n.id, ok ORDER BY n.id
MATCH (n:Person) WITH n, all(y IN [(n)-[:KNOWS]->(y) | y] WHERE size([(y)-[:KNOWS]->(z) | 1]) > 0) AS ok WHERE NOT ok RETURN n.id ORDER BY n.id
MATCH (n:Person) RETURN n.id, any(y IN [(n)-[:KNOWS]->(y) | y] WHERE any(z IN [(y)-[:KNOWS]->(z) | z] WHERE any(w IN [(z)-[:WORKS_FOR]->(w) | w] WHERE w.id = 'x'))) AS deep ORDER BY n.id
MATCH (n:Person) WITH n, any(y1 IN [(n)-[:KNOWS]->(x1) | x1] WHERE any(y2 IN [(y1)-[:KNOWS]->(x2) | x2] WHERE any(y3 IN [(y2)-[:WORKS_FOR]->(x3) | x3] WHERE y3.id = 'x'))) AS c WHERE NOT c RETURN n.id ORDER BY n.id
MATCH (n:Person) RETURN n.id, CASE WHEN any(y IN [(n)-[:KNOWS]->(y) | y] WHERE y.name = 'Cy') THEN 'yes' ELSE 'no' END AS c ORDER BY n.id
MATCH (n:Person) RETURN n.id, size([y IN [(n)-[:KNOWS]->(y) | y] WHERE size([(y)-[:KNOWS]->(z) | 1]) > 0]) AS k ORDER BY n.id
MATCH (n:Person) RETURN n.id, reduce(acc = 0, y IN [(n)-[:KNOWS]->(y) | y] | acc + size([(y)-[:KNOWS]->(z) | 1])) AS k ORDER BY n.id
MATCH (n:Person) WITH n, [(n)-[:KNOWS]->(y) | y] AS ys WITH n, ys, all(y IN ys WHERE size([(y)-[:KNOWS]->(z) | 1]) > 0) AS ok RETURN n.id, ok ORDER BY n.id
MATCH (n:Person) WITH n, size([(n)-[:KNOWS]->(y) WHERE y.name IS NOT NULL | 1]) > 0 AND n.name IS NOT NULL AS c WHERE NOT c RETURN n.id ORDER BY n.id
MATCH (n:Person) RETURN n.id, size([x IN [any(y IN [(n)-[:KNOWS]->(y) | y] WHERE y.name = 'Cy'), n.name IS NOT NULL] WHERE x]) = 1 AS xone ORDER BY n.id
# variable collisions and null projection
MATCH (n:Person) RETURN n.id, [(n)-[r:KNOWS]->(y) | r.w] AS ws ORDER BY n.id
MATCH (n:Person) RETURN n.id, [(n)-[r:KNOWS]->(y) | type(r)] AS ts ORDER BY n.id
MATCH (n:Person) RETURN n.id, [(n)-[r1:KNOWS]->(y1) | r1.w] AS ws, [(n)-[r2:KNOWS]->(y2) | type(r2)] AS ts ORDER BY n.id
MATCH (n:Person) RETURN n.id, [(n)-[r:KNOWS]->(y) | y.name] AS names ORDER BY n.id
MATCH (n:Person) RETURN n.id, [(n)-[r:KNOWS]->(y) | [y.name]] AS names ORDER BY n.id
MATCH (n:Person) RETURN n.id, size([(n)-[r:KNOWS]->(y) | r]) AS a, size([(n)-[r2:KNOWS]->(y2) | id(r2)]) AS b ORDER BY n.id
MATCH (n:Person) RETURN n.id, size([(n)-[:KNOWS]->(y) | y]) AS a, size([(n)-[:KNOWS]->(y2) | y2]) AS b ORDER BY n.id
# per-value rows from comprehensions (a: b, b, c; b: c)
MATCH (n:Person) UNWIND [(n)-[:KNOWS]->(y) | y] AS v1 WITH n, v1 WHERE v1.name IS NULL RETURN n.id, v1.id ORDER BY n.id, v1.id
MATCH (n:Person) UNWIND [(n)-[:KNOWS]->(y) | y] AS v1 WITH n, v1 RETURN n.id, v1.id, labels(v1), toString(id(v1)) ORDER BY n.id, v1.id
MATCH (n:Person) UNWIND [(n)-[:KNOWS]->()-[:WORKS_FOR]->(y) | y] + [(n)<-[:KNOWS]-(y) | y] AS v1 RETURN n.id, v1.id ORDER BY n.id, v1.id
MATCH (n:Person) WITH n, [(n)-[:KNOWS]->(y) | y] AS vs UNWIND vs AS v1 WITH n, v1 WHERE NOT size([(v1)-[:KNOWS]->(z) | 1]) > 0 RETURN n.id, v1.id ORDER BY n.id, v1.id
MATCH (n:Person) UNWIND [(n)-[:KNOWS*1..2]->(y) | y] AS v1 RETURN n.id, collect(v1.id) ORDER BY n.id
# variable-length MATCH collapsing
MATCH p = (n:Person {id: 'a'})-[:KNOWS*1..1]->(y) RETURN y.id ORDER BY y.id
MATCH p = (n:Person {id: 'a'})-[:KNOWS*1..1]->(y) WHERE length(p) >= 0 RETURN y.id ORDER BY y.id
MATCH (n:Person {id: 'a'})-[r:KNOWS*1..1]->(y) WHERE size(r) >= 0 RETURN y.id ORDER BY y.id
MATCH (n:Person {id: 'a'})-[r:KNOWS*1..2]->(y) RETURN y.id, size(r) ORDER BY y.id
# CALL with relationship variables
MATCH (n:Person {id: 'a'}) CALL { WITH n MATCH (n)-[r:KNOWS]->(y) WHERE id(r) >= 0 RETURN y AS v1 UNION ALL WITH n MATCH (n)<-[r:KNOWS]-(y) WHERE id(r) >= 0 RETURN y AS v1 } RETURN v1.id ORDER BY v1.id
MATCH (n:Person {id: 'a'}) CALL { WITH n UNWIND [(n)-[:KNOWS]->(y) | y] AS v1 RETURN v1 UNION ALL WITH n UNWIND [(n)<-[:KNOWS]-(y) | y] AS v1 RETURN v1 } RETURN v1.id ORDER BY v1.id
# relationship focus summaries and details
MATCH (s0)-[v0:KNOWS]->(e0) WITH s0, v0, e0 WHERE v0.w IS NULL RETURN count(v0) AS violations, collect({type: type(v0), startKey: s0.id, endKey: e0.id, elementId: toString(id(v0))})[0..5] AS sample
MATCH (s0)-[v0:KNOWS]->(e0) WITH s0, v0, e0 WHERE v0.w IS NULL RETURN {type: type(v0), startKey: s0.id, endKey: e0.id, elementId: toString(id(v0))} AS focus ORDER BY focus.endKey
MATCH (v0:Person) WITH v0 WHERE v0.name IS NULL RETURN count(v0), collect({label: 'Person', key: 'id', keyValue: toStringOrNull(v0.id), elementId: toString(id(v0))})[0..5]
# parameters lost in batch 4
CYPHER limit=0 MATCH (n) RETURN n.id LIMIT $limit
CYPHER a='x' b=1.5 c=[1,2] RETURN $a, $b, $c
CYPHER limit=9223372036854775807 sampleSize=5 MATCH (n:Person) WITH collect(n.id) AS ids RETURN ids[0..$sampleSize] LIMIT $limit
EOF
