#!/usr/bin/env bash
# Probe batch 4: where pattern predicates are evaluated correctly, replacements for
# `any(x IN [pattern comprehension] WHERE ...)`, relationship-variable collapsing in
# aggregates, and TIMEOUT 0 semantics. Uses graph `probe2` from batch 2 (a -KNOWS x2-> b,
# a -KNOWS-> c, b -KNOWS-> c, c -WORKS_FOR-> x; c is Person:Employee, m is Employee).
# One query per line; `#` headers; `!` lines are raw redis-cli arguments.
set -u
PODMAN=${PODMAN:-/opt/podman/bin/podman}
cli() { "$PODMAN" exec s2c-falkordb redis-cli "$@" 2>&1; }
q() {
  printf '\n>> %s\n' "$1"
  cli GRAPH.RO_QUERY probe2 "$1"
}
cli GRAPH.DELETE missing_graph_s2c2 >/dev/null
while IFS= read -r line; do
  case "$line" in
    '') ;;
    '#'*) printf '\n==== %s\n' "$line" ;;
    '!'*) printf '\n>> %s\n' "${line#!}"; eval "cli ${line#!}" ;;
    *) q "$line" ;;
  esac
done <<'EOF'
# expected: outgoing KNOWS for a, b; not c
# pattern predicates outside plain WHERE
MATCH (n:Person) RETURN n.id, CASE WHEN (n)-[:KNOWS]->() THEN 1 ELSE 0 END AS c ORDER BY n.id
MATCH (n:Person) RETURN n.id, (n)-[:KNOWS]->() AS p ORDER BY n.id
MATCH (n:Person) WITH n, (n)-[:KNOWS]->() AS p RETURN n.id, p ORDER BY n.id
MATCH (n:Person) WITH n WHERE (n)-[:KNOWS]->() RETURN n.id ORDER BY n.id
MATCH (n:Person) WHERE NOT ((n)-[:KNOWS]->() AND n.id <> 'zz') RETURN n.id ORDER BY n.id
MATCH (n:Person) WHERE NOT (NOT (n)-[:KNOWS]->() OR n.id = 'b') RETURN n.id ORDER BY n.id
MATCH (n:Person) WHERE ((n)-[:KNOWS]->() AND NOT (n)<-[:KNOWS]-()) OR n.id = 'c' RETURN n.id ORDER BY n.id
MATCH (n:Person) WHERE (n)-[:KNOWS*2..2]->(:Employee) RETURN n.id ORDER BY n.id
MATCH (n:Person) WHERE (n)-[:KNOWS]->({name: 'Cy'}) RETURN n.id ORDER BY n.id
MATCH (n:Person) RETURN n.id, size([(n)-[:KNOWS]->() | 1]) > 0 AS p ORDER BY n.id
MATCH (n:Person) RETURN n.id, CASE WHEN size([(n)-[:KNOWS]->() | 1]) > 0 THEN 1 ELSE 0 END AS c ORDER BY n.id
MATCH (n:Person) RETURN n.id, [(n)-[:KNOWS]->(y) | 1] AS l ORDER BY n.id
# quantifier replacements over pattern comprehensions
MATCH (n:Person) WITH n, [(n)-[:KNOWS]->(y) | y] AS ys WHERE any(y IN ys WHERE y.name = 'Cy') RETURN n.id ORDER BY n.id
MATCH (n:Person) RETURN n.id, any(y IN [(n)-[:KNOWS]->(y) | y] WHERE y.name = 'Cy') AS p ORDER BY n.id
MATCH (n:Person) RETURN n.id, any(z IN [(n)-[:KNOWS]->(y) | y] WHERE z.name = 'Cy') AS p ORDER BY n.id
MATCH (n:Person) WHERE any(z IN [(n)-[:KNOWS]->(y) | y] WHERE z.name = 'Cy') RETURN n.id ORDER BY n.id
MATCH (n:Person) RETURN n.id, size([(n)-[:KNOWS]->(y) WHERE y.name = 'Cy' | 1]) > 0 AS p ORDER BY n.id
MATCH (n:Person) RETURN n.id, size([(n)-[:KNOWS]->(y) WHERE NOT (size([(y)-[:KNOWS]->(z) | 1]) > 0) | 1]) = 0 AS allHaveOut ORDER BY n.id
MATCH (n:Person) RETURN n.id, size([(n)-[:KNOWS]->(y) WHERE size([(y)-[:KNOWS]->(z) WHERE size([(z)-[:WORKS_FOR]->(w) WHERE w.id = 'x' | 1]) > 0 | 1]) > 0 | 1]) > 0 AS deep ORDER BY n.id
MATCH (n:Person) RETURN n.id, [x IN [(n)-[:KNOWS]->(y) | y] WHERE x.name = 'Cy' | x.id] AS l ORDER BY n.id
MATCH (n:Person) RETURN n.id, all(x IN [(n)-[:KNOWS]->(y) | y.name] WHERE x IS NOT NULL) AS p ORDER BY n.id
MATCH (n:Person) RETURN n.id, any(x IN [(n)-[:KNOWS]->(y) | y.name] WHERE x = 'Cy') AS p ORDER BY n.id
MATCH (n:Person) RETURN n.id, [(n)-[:KNOWS]->(y) WHERE any(k IN keys(y) WHERE k = 'name') | y.id] AS l ORDER BY n.id
MATCH (n:Person) RETURN n.id, [(n)-[:KNOWS]->(y) WHERE (y)-[:WORKS_FOR]->() | y.id] AS l ORDER BY n.id
MATCH (n:Person) RETURN n.id, [(n)-[:KNOWS]->(y) WHERE y <> n | id(y)] AS l ORDER BY n.id
MATCH (n:Person) RETURN n.id, [(n)-[:KNOWS]->(y) WHERE y.id IN [(n)-[:KNOWS]->(z) | z.id] | y.id] AS l ORDER BY n.id
# relationship values in comprehensions
MATCH (n:Person) RETURN n.id, [(n)-[r:KNOWS]->(y) | r] AS rs ORDER BY n.id
MATCH (n:Person) RETURN n.id, [(n)-[r:KNOWS]->(y) | r.w] AS ws, [(n)-[r:KNOWS]->(y) | type(r)] AS ts ORDER BY n.id
MATCH (n:Person) RETURN n.id, [(n)-[r:KNOWS]->(y) | [r]] AS rs ORDER BY n.id
# relationship-variable collapsing in aggregates and summaries
MATCH (s)-[v0:KNOWS]->(e) WHERE id(v0) >= 0 RETURN count(v0), collect(toString(id(v0)))[0..5]
MATCH (s)-[v0:KNOWS]->(e) RETURN count(v0)
MATCH (s)-[v0:KNOWS]->(e) WITH s, v0, e RETURN count(*)
MATCH (s)-[v0:KNOWS]->(e) WITH v0 WHERE v0.w IS NULL RETURN count(v0)
MATCH (n:Person {id: 'a'})-[:KNOWS]->(y) RETURN count(DISTINCT id(y)), count(id(y))
MATCH (n:Person {id: 'a'}) OPTIONAL MATCH (n)-[r:KNOWS]->(y) RETURN count(r), count(DISTINCT y)
MATCH (n:Person {id: 'a'}) OPTIONAL MATCH (n)-[r:KNOWS]->(y) WHERE id(r) >= 0 RETURN count(r), count(DISTINCT y)
MATCH (n:Person {id: 'a'}) MATCH (n)-[:KNOWS]->(y) RETURN y.id ORDER BY y.id
MATCH (n:Person {id: 'a'}) MATCH (n)-[r:KNOWS]->(y) WHERE id(r) >= 0 RETURN y.id ORDER BY y.id
MATCH (n:Person {id: 'a'}) MATCH (n)-[r:KNOWS*1..1]->(y) RETURN y.id ORDER BY y.id
# property-value distinct counts and lists
MATCH (n:Person) RETURN n.id, size(reduce(acc = [], x IN CASE typeOf(n.name) WHEN 'Null' THEN [] WHEN 'List' THEN n.name ELSE [n.name] END | CASE WHEN x IN acc THEN acc ELSE acc + [x] END)) ORDER BY n.id
RETURN reduce(acc = [], x IN [1, 1.0, '1', date('2020-01-01'), date('2020-01-01')] | CASE WHEN x IN acc THEN acc ELSE acc + [x] END)
# TIMEOUT semantics
!GRAPH.RO_QUERY probe2 "UNWIND range(1, 20000000) AS x WITH x WHERE x % 7 = 0 RETURN count(x)" TIMEOUT 0
!GRAPH.RO_QUERY probe2 "UNWIND range(1, 20000000) AS x WITH x WHERE x % 7 = 0 RETURN count(x)" TIMEOUT 600000
!GRAPH.RO_QUERY probe2 "UNWIND range(1, 20000000) AS x WITH x WHERE x % 7 = 0 RETURN count(x)" TIMEOUT 9223372036854775807
!GRAPH.RO_QUERY probe2 "RETURN 1" TIMEOUT -1
!GRAPH.RO_QUERY probe2 "CYPHER limit=0 MATCH (n) RETURN n.id LIMIT $limit"
!GRAPH.RO_QUERY probe2 "CYPHER a='x' b=1.5 c=[1,2] RETURN $a, $b, $c"
!GRAPH.RO_QUERY probe2 "MATCH (n) RETURN n LIMIT 1" --compact
EOF
