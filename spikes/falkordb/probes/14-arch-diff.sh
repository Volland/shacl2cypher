#!/usr/bin/env bash
# Probe batch 14: the same queries on the arm64 and amd64 builds of falkordb:v4.20.4,
# because the conformance fixtures pass on arm64 but report no violations on amd64 (CI).
set -u
PODMAN=${PODMAN:-/opt/podman/bin/podman}
SUMMARY=$(cat "$1")
run() { "$PODMAN" exec "$1" redis-cli GRAPH.RO_QUERY arch_probe "$2" 2>&1 | grep -v "execution time\|Cached execution"; }
for c in s2c-falkordb s2c-falkordb-amd64; do
  echo "######## $c ($("$PODMAN" exec $c uname -m))"
  "$PODMAN" exec $c redis-cli GRAPH.DELETE arch_probe >/dev/null 2>&1
  for s in "MATCH (n) RETURN count(n)" "CREATE (:\`Person\` {\`id\`: 'p1', \`name\`: 'Ann'})" "CREATE (:\`Person\` {\`id\`: 'p2'})" "CREATE (:\`Person\` {\`id\`: 'p3'})"; do
    "$PODMAN" exec $c redis-cli GRAPH.QUERY arch_probe "$s" >/dev/null
  done
  while IFS= read -r q; do
    [ -z "$q" ] && continue
    printf '\n>> %s\n' "$q"; run $c "$q"
  done <<'PROBES'
MATCH (v0:Person) RETURN v0.id, typeOf(v0.name), v0.name IS NULL ORDER BY v0.id
MATCH (v0:Person) RETURN v0.id, CASE typeOf(v0.name) WHEN 'Null' THEN [] WHEN 'List' THEN v0.name ELSE [v0.name] END AS l ORDER BY v0.id
MATCH (v0:Person) RETURN v0.id, CASE typeOf(v0.name) WHEN 'Null' THEN 'null' ELSE 'other' END AS simple, CASE WHEN typeOf(v0.name) = 'Null' THEN 'null' ELSE 'other' END AS searched ORDER BY v0.id
MATCH (v0:Person) RETURN v0.id, size(reduce(x2 = [], x1 IN CASE typeOf(v0.name) WHEN 'Null' THEN [] WHEN 'List' THEN v0.name ELSE [v0.name] END | CASE WHEN x1 IS NULL OR x1 IN x2 THEN x2 ELSE x2 + [x1] END)) AS n ORDER BY v0.id
MATCH (v0:Person) RETURN v0.id, reduce(acc = [], x IN ['a', 'b'] | acc + [x]) AS r, size([x IN coalesce(v0.name, []) | x]) AS c ORDER BY v0.id
MATCH (v0:Person) WITH v0 WHERE NOT (size(reduce(x2 = [], x1 IN CASE typeOf(v0.name) WHEN 'Null' THEN [] WHEN 'List' THEN v0.name ELSE [v0.name] END | CASE WHEN x1 IS NULL OR x1 IN x2 THEN x2 ELSE x2 + [x1] END)) >= 1) RETURN v0.id ORDER BY v0.id
PROBES
  printf '\n>> compiled summary\n'; run $c "CYPHER limit=100 sampleSize=5 $SUMMARY"
done
