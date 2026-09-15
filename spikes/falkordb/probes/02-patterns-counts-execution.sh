#!/usr/bin/env bash
# Probe batch 2: pattern predicates and comprehensions, distinct counts, CALL/UNION,
# label unions, parallel relationships, variable-length paths, parameters, LIMIT,
# zero-row aggregates, and GRAPH.RO_QUERY errors (writes, timeouts, missing graphs).
# One Cypher query per line on graph `probe2`; `#` lines are section headers.
# Lines starting with `!` are raw redis-cli argument lists, run verbatim through eval.
set -u
PODMAN=${PODMAN:-/opt/podman/bin/podman}
cli() { "$PODMAN" exec s2c-falkordb redis-cli "$@" 2>&1; }
q() {
  printf '\n>> %s\n' "$1"
  cli GRAPH.QUERY probe2 "$1"
}
cli GRAPH.DELETE probe2 >/dev/null
while IFS= read -r line; do
  case "$line" in
    '') ;;
    '#'*) printf '\n==== %s\n' "$line" ;;
    '!'*) printf '\n>> %s\n' "${line#!}"; eval "cli ${line#!}" ;;
    *) q "$line" ;;
  esac
done <<'EOF'
# setup: a has two parallel KNOWS to b, one to c; b KNOWS c; c has a WORKS_FOR to x; multi-label node m
CREATE (a:Person {id: 'a', name: 'Ann'}), (b:Person {id: 'b'}), (c:Person:Employee {id: 'c', name: 'Cy'}), (x:Company {id: 'x'}), (m:Employee {id: 'm'}), (a)-[:KNOWS {w: 1}]->(b), (a)-[:KNOWS {w: 2}]->(b), (a)-[:KNOWS]->(c), (b)-[:KNOWS]->(c), (c)-[:WORKS_FOR]->(x)
# 1.5 subqueries (expected unsupported)
MATCH (n:Person) WHERE EXISTS { MATCH (n)-[:KNOWS]->() } RETURN n.id
MATCH (n:Person) RETURN n.id, COUNT { MATCH (n)-[:KNOWS]->() }
# 1.5 pattern predicates
MATCH (n:Person) WHERE (n)-[:KNOWS]->() RETURN n.id ORDER BY n.id
MATCH (n:Person) WHERE NOT (n)-[:KNOWS]->() RETURN n.id ORDER BY n.id
MATCH (n:Person) WHERE (n)-[:KNOWS]->(:Employee) OR (n)<-[:KNOWS]-() RETURN n.id ORDER BY n.id
MATCH (n:Person) RETURN n.id, CASE WHEN (n)-[:KNOWS]->() THEN 1 ELSE 0 END ORDER BY n.id
# 1.5 pattern comprehensions
MATCH (n:Person) RETURN n.id, [(n)-[:KNOWS]->(y) | y.id] ORDER BY n.id
MATCH (n:Person) RETURN n.id, [(n)-[:KNOWS]->(y) WHERE y.name IS NOT NULL | y.id] ORDER BY n.id
MATCH (n:Person) RETURN n.id, [(n)-[r:KNOWS]->(y) | id(r)] ORDER BY n.id
MATCH (n:Person) RETURN n.id, size([(n)-[:KNOWS]->(y) | y]), size([(n)-[r:KNOWS]->(y) | r]) ORDER BY n.id
MATCH (n:Person) RETURN n.id, [(n)-[:KNOWS]->()-[:WORKS_FOR]->(z) | z.id] ORDER BY n.id
MATCH (n:Person) RETURN n.id, [(n)-[:KNOWS*1..3]->(y) | y.id] ORDER BY n.id
MATCH (n:Person) RETURN n.id, [(n)-[:KNOWS*0..2]->(y) | y.id] ORDER BY n.id
MATCH (n:Person) RETURN n.id, [(n)-[:KNOWS|WORKS_FOR]->(y) | y.id] ORDER BY n.id
MATCH (n:Person) RETURN n.id, [(n)<-[:KNOWS]-(y) | y.id] ORDER BY n.id
MATCH (n:Person) WHERE any(y IN [(n)-[:KNOWS]->(y) | y] WHERE y.name = 'Cy') RETURN n.id ORDER BY n.id
MATCH (n:Person) WHERE all(y IN [(n)-[:KNOWS]->(y) | y] WHERE size([(y)-[:KNOWS]->(z) | z]) > 0) RETURN n.id ORDER BY n.id
MATCH (n:Person) WHERE any(y IN [(n)-[:KNOWS]->(y) | y] WHERE any(z IN [(y)-[:KNOWS]->(z) | z] WHERE any(w IN [(z)-[:WORKS_FOR]->(w) | w] WHERE w.id = 'x'))) RETURN n.id ORDER BY n.id
MATCH (n:Person) RETURN n.id, reduce(acc = [], i IN [(n)-[:KNOWS]->(y) | id(y)] | CASE WHEN i IN acc THEN acc ELSE acc + [i] END) ORDER BY n.id
MATCH (n:Person) RETURN n.id, size(reduce(acc = [], i IN [(n)-[:KNOWS*1..3]->(y) | id(y)] | CASE WHEN i IN acc THEN acc ELSE acc + [i] END)) ORDER BY n.id
MATCH (n:Person) RETURN n.id, [x IN [1, null, 2] WHERE x IS NOT NULL | x * 2], reduce(s = 0, x IN [1, 2] | s + x) ORDER BY n.id
MATCH (n:Person) RETURN n.id, [(n)-[:KNOWS]->(y) | [(y)-[:KNOWS]->(z) | z.id]] ORDER BY n.id
MATCH (n:Person) RETURN n.id, [(n)-[:KNOWS]->(y) | count(y)] ORDER BY n.id
# 1.7 parallel relationships in MATCH with and without a referenced relationship variable
MATCH (n:Person {id: 'a'})-[:KNOWS]->(y) RETURN y.id ORDER BY y.id
MATCH (n:Person {id: 'a'})-[r:KNOWS]->(y) RETURN y.id ORDER BY y.id
MATCH (n:Person {id: 'a'})-[r:KNOWS]->(y) WHERE id(r) >= 0 RETURN y.id ORDER BY y.id
MATCH (n:Person {id: 'a'})-[:KNOWS]->(y) RETURN count(y), count(DISTINCT y)
MATCH (s)-[v0:KNOWS]->(e) RETURN s.id, e.id, v0.w ORDER BY s.id, e.id
MATCH (s)-[v0:KNOWS]->(e) RETURN count(*)
# label unions and multi-labels
MATCH (n:Person|Employee) RETURN n.id
MATCH (n) WHERE n:Person OR n:Employee RETURN n.id ORDER BY n.id
MATCH (n:Person:Employee) RETURN n.id
MATCH (n) WHERE n:Nope RETURN n.id
MATCH (n:Nope) RETURN n.id
RETURN labels(null)
# CALL subqueries and UNION
MATCH (n:Person) CALL { WITH n MATCH (n)-[:KNOWS]->(y) RETURN y.id AS v UNION WITH n MATCH (n)<-[:KNOWS]-(y) RETURN y.id AS v } RETURN n.id, v ORDER BY n.id, v
MATCH (n:Person) CALL { WITH n UNWIND [n.name] AS v RETURN v UNION ALL WITH n MATCH (n)-[:KNOWS]->(y) RETURN y.name AS v } WITH n, v WHERE v IS NOT NULL RETURN n.id, v ORDER BY n.id, v
MATCH (n:Person) CALL { WITH n RETURN 1 AS one } RETURN n.id, one ORDER BY n.id
# value normalization and unwind
RETURN [x IN CASE typeOf([1, 2]) WHEN 'Null' THEN [] WHEN 'List' THEN [1, 2] ELSE [[1, 2]] END | x]
UNWIND null AS v RETURN v
UNWIND 5 AS v RETURN v
MATCH (n:Person) UNWIND CASE typeOf(n.name) WHEN 'Null' THEN [] WHEN 'List' THEN n.name ELSE [n.name] END AS v RETURN n.id, v ORDER BY n.id
# zero-row aggregates, collect slicing, maps with nulls, message replace
MATCH (n:Nope) RETURN count(n), collect(n)
MATCH (n:Person) RETURN collect(n.id)[0..2], collect(n.id)[0..0]
MATCH (n:Person) WITH collect({id: n.id, name: n.name}) AS rows RETURN rows[0..10]
RETURN {a: null, b: 1}, replace('Hi {$this}', '{$this}', toString(5)), toString(1.5), toString(true), toString(id(null))
MATCH (n:Person {id: 'a'}) RETURN toString(id(n)), properties(n), keys(n)
MATCH (s)-[v0:KNOWS]->(e) RETURN type(v0), toString(id(v0)), startNode(v0).id, endNode(v0).id LIMIT 1
# parameters and LIMIT
CYPHER limit=2 MATCH (n:Person) RETURN n.id ORDER BY n.id LIMIT $limit
CYPHER limit=9223372036854775807 MATCH (n:Person) RETURN n.id ORDER BY n.id LIMIT $limit
CYPHER limit=null MATCH (n:Person) RETURN n.id ORDER BY n.id LIMIT coalesce($limit, 9223372036854775807)
CYPHER sampleSize=2 MATCH (n:Person) RETURN collect(n.id)[0..$sampleSize]
MATCH (n:Person) RETURN n.id LIMIT 9223372036854775807
# variable-length depth limits
MATCH (n:Person {id: 'a'}) RETURN size([(n)-[:KNOWS*1..30]->(y) | y])
MATCH (n:Person {id: 'a'}) RETURN size([(n)-[:KNOWS*1..1000]->(y) | y])
MATCH (n:Person {id: 'a'}) RETURN size([(n)-[:KNOWS*0..0]->(y) | y])
# closed-shape helpers
MATCH (n:Person {id: 'a'}) RETURN [k IN keys(n) WHERE NOT k IN ['id']], [(n)-[r]->() WHERE NOT type(r) IN ['X'] | type(r)]
# 1.7 RO_QUERY: write refusal, missing graph, timeouts, config
!GRAPH.RO_QUERY probe2 "CREATE (:W)"
!GRAPH.RO_QUERY probe2 "MATCH (n) SET n.x = 1"
!GRAPH.RO_QUERY missing_graph_s2c "RETURN 1"
!GRAPH.QUERY missing_graph_s2c2 "MATCH (n) RETURN n"
!GRAPH.LIST
!GRAPH.RO_QUERY probe2 "UNWIND range(1, 50000000) AS x WITH x WHERE x % 7 = 0 RETURN count(x)" TIMEOUT 50
!GRAPH.RO_QUERY probe2 "UNWIND range(1, 50000000) AS x WITH x WHERE x % 7 = 0 RETURN count(x)"
!GRAPH.CONFIG GET TIMEOUT_MAX
!GRAPH.CONFIG GET TIMEOUT_DEFAULT
!GRAPH.CONFIG GET TIMEOUT
!GRAPH.RO_QUERY probe2 "MATCH (n) RETURN n.id ORDER BY n.id" TIMEOUT 0
!GRAPH.RO_QUERY probe2 "RETURN db.labels()"
!GRAPH.RO_QUERY probe2 "CALL db.labels() YIELD label RETURN label ORDER BY label"
!GRAPH.RO_QUERY probe2 "CALL db.relationshipTypes() YIELD relationshipType RETURN relationshipType ORDER BY relationshipType"
!GRAPH.RO_QUERY probe2 "MATCH (a)-[r]->(b) RETURN DISTINCT labels(a), type(r), labels(b)"
!GRAPH.RO_QUERY probe2 "MATCH ()-[r]->() UNWIND keys(r) AS k RETURN type(r), k, collect(DISTINCT typeOf(r[k]))"
EOF
