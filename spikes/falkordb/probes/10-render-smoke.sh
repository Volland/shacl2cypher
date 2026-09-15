#!/usr/bin/env bash
# Renderer smoke run (add-falkordb-dialect task 6.5): executes every query dumped by
# `S2C_RENDER_DUMP_FALKORDB=<dir> cargo test -p shacl2cypher-core --lib falkordb` against
# sample data covering the renderer test shapes, and reports errors and summary counts.
set -u
DUMP=${1:?usage: 10-render-smoke.sh <dump dir>}
PODMAN=${PODMAN:-/opt/podman/bin/podman}
GRAPH=smoke_s2c
cli() { "$PODMAN" exec s2c-falkordb redis-cli --no-raw "$@" 2>&1; }
cli GRAPH.DELETE "$GRAPH" >/dev/null
cli GRAPH.QUERY "$GRAPH" "CREATE (a:Person {id: 'a', name: 'Ann', age: 30, tags: ['a', 'c'], start: 1, end: 2, email: 'a@x'}),
 (b:Person {id: 'b', name: 'b', age: 'old', start: 5, end: 3, nick: 'x'}),
 (c:Person:Employee {id: 'c', name: ['Cy', 'Cyrus'], age: 99999, phone: '1'}),
 (co:Company {id: 'co', legalName: 'Co'}), (co2:Company {id: 'co2'}),
 (ad:Address {id: 'ad', zip: '1234'}), (ad2:Address {id: 'ad2', zip: 12345}),
 (t:Team {id: 't'}), (cl:Client {id: 'cl'}),
 (a)-[:WORKS_FOR]->(co), (a)-[:WORKS_FOR]->(co2), (b)-[:WORKS_FOR]->(co),
 (a)-[:ADDRESS]->(ad), (b)-[:ADDRESS]->(ad2), (a)-[:MANAGER]->(b), (c)-[:MANAGER]->(a),
 (a)-[:KNOWS {since: date('2020-01-01')}]->(b), (a)-[:KNOWS {since: 'yesterday'}]->(b), (b)-[:KNOWS]->(c), (b)-[:KNOWS]->(co),
 (t)-[:LEAD]->(a), (t)-[:OWNER]->(b), (t)-[:MEMBER]->(a),
 (cl)-[:SERVED_BY]->(a)" | tail -1
errors=0
for detail in "$DUMP"/*.detail.cypher; do
  stem=${detail%.detail.cypher}
  id=$(cat "$stem.id")
  for kind in detail summary; do
    query=$(cat "$stem.$kind.cypher")
    out=$(cli GRAPH.RO_QUERY "$GRAPH" "CYPHER limit=100 sampleSize=5 $query" TIMEOUT 0)
    if printf '%s' "$out" | grep -q '^(error)'; then
      errors=$((errors + 1))
      printf 'ERROR %s %s: %s\n' "$id" "$kind" "$(printf '%s' "$out" | grep '^(error)' | head -1)"
    elif [ "$kind" = summary ]; then
      count=$(printf '%s' "$out" | sed -n '3p' | tr -d ' ')
      printf 'ok    %-55s violations=%s\n' "$id" "$(printf '%s' "$out" | awk 'NR>1 && /\(integer\)/ {print $NF; exit}')"
    fi
  done
done
echo "errors: $errors"
