#!/usr/bin/env bash
# Probe batch 8: can nested quantifiers and counts be rendered as correlated CALL {}
# subqueries so every pattern is anchored at a row variable? Graph `probe2`:
# a -KNOWS x2-> b, a -KNOWS-> c, b -KNOWS-> c, c -WORKS_FOR-> x; c is Person:Employee.
# Expected: deep(any KNOWS . any KNOWS . any WORKS_FOR to x): a true, b false, c false.
#           allOut(all KNOWS targets have a KNOWS out): a false, b false, c true.
#           distinct KNOWS targets: a 2, b 1, c 0.
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
done <<'PROBES'
# OPTIONAL MATCH rows as anchors
MATCH (n:Person) OPTIONAL MATCH (n)-[r1:KNOWS]->(y) WHERE id(r1) >= 0 WITH n, y, size([(y)-[:KNOWS]->(z) | 1]) > 0 AS c RETURN n.id, y.id, c ORDER BY n.id, y.id
MATCH (n:Person) OPTIONAL MATCH (n)-[r1:KNOWS]->(y) WHERE id(r1) >= 0 WITH n, y, size([(y)-[:KNOWS]->(z) | 1]) > 0 AS c WITH n, collect(CASE WHEN y IS NULL THEN null ELSE c END) AS cs RETURN n.id, all(x IN cs WHERE x) AS allOut ORDER BY n.id
# single-level CALL with aggregation
MATCH (n:Person) CALL { WITH n OPTIONAL MATCH (n)-[r1:KNOWS]->(y) WHERE id(r1) >= 0 RETURN count(DISTINCT y) AS k } RETURN n.id, k ORDER BY n.id
MATCH (n:Person) CALL { WITH n OPTIONAL MATCH (n)-[r1:KNOWS]->(y) WHERE id(r1) >= 0 WITH y, size([(y)-[:KNOWS]->(z) | 1]) > 0 AS c RETURN all(x IN collect(CASE WHEN y IS NULL THEN null ELSE c END) WHERE x) AS allOut } RETURN n.id, allOut ORDER BY n.id
# nested CALL three levels deep
MATCH (n:Person) CALL { WITH n OPTIONAL MATCH (n)-[r1:KNOWS]->(y1) WHERE id(r1) >= 0 CALL { WITH y1 OPTIONAL MATCH (y1)-[r2:KNOWS]->(y2) WHERE id(r2) >= 0 WITH y2, size([(y2)-[r3:WORKS_FOR]->(y3) WHERE y3.id = 'x' | 1]) > 0 AS c3 RETURN any(x IN collect(CASE WHEN y2 IS NULL THEN null ELSE c3 END) WHERE x) AS c2 } RETURN any(x IN collect(CASE WHEN y1 IS NULL THEN null ELSE c2 END) WHERE x) AS deep } RETURN n.id, deep ORDER BY n.id
MATCH (n:Person) CALL { WITH n OPTIONAL MATCH (n)-[r1:KNOWS]->(y1) WHERE id(r1) >= 0 CALL { WITH y1 OPTIONAL MATCH (y1)-[r2:KNOWS]->(y2) WHERE id(r2) >= 0 CALL { WITH y2 OPTIONAL MATCH (y2)-[r3:WORKS_FOR]->(y3) WHERE id(r3) >= 0 RETURN any(x IN collect(CASE WHEN y3 IS NULL THEN null ELSE y3.id = 'x' END) WHERE x) AS c3 } RETURN any(x IN collect(CASE WHEN y2 IS NULL THEN null ELSE c3 END) WHERE x) AS c2 } RETURN any(x IN collect(CASE WHEN y1 IS NULL THEN null ELSE c2 END) WHERE x) AS deep } RETURN n.id, deep ORDER BY n.id
# conditions as columns combined in the outer WHERE
MATCH (v0:Person) CALL { WITH v0 OPTIONAL MATCH (v0)-[r1:KNOWS]->(y1) WHERE id(r1) >= 0 RETURN count(DISTINCT y1) AS k1 } CALL { WITH v0 OPTIONAL MATCH (v0)<-[r2:KNOWS]-(y2) WHERE id(r2) >= 0 RETURN count(DISTINCT y2) AS k2 } WITH v0, k1, k2 WHERE NOT (k1 >= 1 AND k2 >= 1) RETURN v0.id ORDER BY v0.id
MATCH (v0:Person) CALL { WITH v0 OPTIONAL MATCH (v0)-[r1:KNOWS]->(y1) WHERE id(r1) >= 0 RETURN count(DISTINCT y1) AS k1 } WITH v0, k1 WHERE NOT (k1 <= 1) RETURN count(v0) AS violations, collect({keyValue: toStringOrNull(v0.id), elementId: toString(id(v0))})[0..5] AS sample
# per-value rows over a relationship path with a nested test
MATCH (v0:Person) MATCH (v0)-[r0:KNOWS]->(v1) WHERE id(r0) >= 0 CALL { WITH v1 OPTIONAL MATCH (v1)-[r1:KNOWS]->(y) WHERE id(r1) >= 0 RETURN count(DISTINCT y) AS k } WITH v0, v1, k WHERE NOT (k > 0) RETURN v0.id, v1.id ORDER BY v0.id, v1.id
# alternative routes inside CALL, then a nested test per value
MATCH (v0:Person) CALL { WITH v0 MATCH (v0)-[r1:KNOWS]->(v1) WHERE id(r1) >= 0 RETURN v1 UNION WITH v0 MATCH (v0)<-[r2:KNOWS]-(v1) WHERE id(r2) >= 0 RETURN v1 } CALL { WITH v1 OPTIONAL MATCH (v1)-[r3:WORKS_FOR]->(y) WHERE id(r3) >= 0 RETURN count(DISTINCT y) AS k } WITH v0, v1, k WHERE NOT (k >= 1) RETURN v0.id, v1.id ORDER BY v0.id, v1.id
MATCH (v0:Person) CALL { WITH v0 CALL { WITH v0 OPTIONAL MATCH (v0)-[r1:KNOWS]->(v1) WHERE id(r1) >= 0 RETURN v1 UNION WITH v0 OPTIONAL MATCH (v0)<-[r2:KNOWS]-(v1) WHERE id(r2) >= 0 RETURN v1 } RETURN count(DISTINCT v1) AS k } RETURN v0.id, k ORDER BY v0.id
MATCH (v0:Person) CALL { WITH v0 CALL { WITH v0 OPTIONAL MATCH (v0)-[r1:KNOWS]->(v1) WHERE id(r1) >= 0 RETURN v1 UNION WITH v0 OPTIONAL MATCH (v0)<-[r2:KNOWS]-(v1) WHERE id(r2) >= 0 RETURN v1 } WITH v1, size([(v1)-[:WORKS_FOR]->() | 1]) > 0 AS c RETURN any(x IN collect(CASE WHEN v1 IS NULL THEN null ELSE c END) WHERE x) AS anyWorks } RETURN v0.id, anyWorks ORDER BY v0.id
# variable-length routes in CALL with path variables (distinct targets within 1..2 hops: a {b, c} = 2, b {c} = 1, c 0)
MATCH (v0:Person) CALL { WITH v0 OPTIONAL MATCH p1 = (v0)-[:KNOWS*1..2]->(y) RETURN count(DISTINCT y) AS k, count(p1) AS paths } RETURN v0.id, k, paths ORDER BY v0.id
MATCH (v0:Person) CALL { WITH v0 OPTIONAL MATCH p1 = (v0)-[:KNOWS*0..2]->(y) RETURN count(DISTINCT y) AS k } RETURN v0.id, k ORDER BY v0.id
# property values of neighbours in CALL (value sets of a sequence path)
MATCH (v0:Person) CALL { WITH v0 OPTIONAL MATCH (v0)-[r1:KNOWS]->(y) WHERE id(r1) >= 0 UNWIND CASE typeOf(y.name) WHEN 'Null' THEN [] WHEN 'List' THEN y.name ELSE [y.name] END AS v RETURN collect(DISTINCT v) AS names } RETURN v0.id, names ORDER BY v0.id
MATCH (v0:Person) OPTIONAL MATCH (v0)-[r1:KNOWS]->(y) WHERE id(r1) >= 0 UNWIND CASE typeOf(y.name) WHEN 'Null' THEN [] WHEN 'List' THEN y.name ELSE [y.name] END AS v1 WITH v0, v1 WHERE v1 IS NOT NULL AND NOT (CASE WHEN typeOf(v1) = 'String' THEN size(toStringOrNull(v1)) >= 3 ELSE false END) RETURN v0.id, v1 ORDER BY v0.id
# CALL returning zero rows must not drop the outer row
MATCH (v0:Person) CALL { WITH v0 MATCH (v0)-[r1:WORKS_FOR]->(y) WHERE id(r1) >= 0 RETURN count(y) AS k } RETURN v0.id, k ORDER BY v0.id
MATCH (v0:Person) CALL { WITH v0 MATCH (v0)-[r1:WORKS_FOR]->(y) WHERE id(r1) >= 0 RETURN y } RETURN v0.id, y.id ORDER BY v0.id
# pair constraints over relationship sets as CALL columns (sh:equals KNOWS vs incoming KNOWS)
MATCH (v0:Person) CALL { WITH v0 OPTIONAL MATCH (v0)-[r1:KNOWS]->(y) WHERE id(r1) >= 0 RETURN collect(DISTINCT id(y)) AS outs } CALL { WITH v0 OPTIONAL MATCH (v0)<-[r2:KNOWS]-(y) WHERE id(r2) >= 0 RETURN collect(DISTINCT id(y)) AS ins } WITH v0, outs, ins WHERE NOT (all(i IN outs WHERE i IN ins) AND all(i IN ins WHERE i IN outs)) RETURN v0.id, outs, ins ORDER BY v0.id
PROBES
