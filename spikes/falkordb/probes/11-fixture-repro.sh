#!/usr/bin/env bash
# Probe batch 11: reproduces the two conformance fixtures that disagreed on FalkorDB
# (core/cardinality.yaml, core/ranges.yaml) with the compiled queries, and isolates
# the parts involved: pre-1970 date comparisons and distinct counts via reduce.
set -u
ROOT=$(cd "$(dirname "$0")/../../.." && pwd)
PODMAN=${PODMAN:-/opt/podman/bin/podman}
BIN=${BIN:-$ROOT/target/debug/shacl2cypher}
OUT=$(mktemp -d)
cli() { "$PODMAN" exec s2c-falkordb redis-cli "$@" 2>&1; }
query_of() { python3 - "$1" "$2" <<'PY'
import sys, re
text = open(sys.argv[1]).read()
match = re.search(r"// name: " + re.escape(sys.argv[2]) + r"\n// ruleId: [^\n]*\n(.*?);\n", text, re.S)
print(match.group(1))
PY
}
run() { printf '\n>> %s\n' "$2"; cli GRAPH.RO_QUERY "$1" "CYPHER limit=100 sampleSize=5 $3" TIMEOUT 0 | grep -v "execution time\|Cached execution"; }

cli GRAPH.DELETE repro_card >/dev/null; cli GRAPH.DELETE repro_ranges >/dev/null
for s in "CREATE (:\`Person\` {\`id\`: 'p1', \`codes\`: ['x'], \`name\`: 'Ann', \`tags\`: ['a', 'b']})" \
         "CREATE (:\`Person\` {\`id\`: 'p2'})" \
         "CREATE (:\`Person\` {\`id\`: 'p3', \`tags\`: []})" \
         "CREATE (:\`Person\` {\`id\`: 'p4', \`name\`: 'Bo', \`tags\`: ['a', 'b', 'c']})" \
         "CREATE (:\`Person\` {\`id\`: 'p5', \`codes\`: ['x', 'x'], \`name\`: 'Cy', \`tags\`: ['a', 'a']})" \
         "CREATE (:\`Person\` {\`id\`: 'p6', \`codes\`: ['x', 'y'], \`name\`: 'Di', \`tags\`: ['a']})"; do
  cli GRAPH.QUERY repro_card "$s" >/dev/null
done
for s in "CREATE (:\`Person\` {\`id\`: 'p1', \`age\`: 0, \`born\`: date('2000-01-01'), \`score\`: 1.5})" \
         "CREATE (:\`Person\` {\`id\`: 'p2', \`age\`: -1, \`born\`: date('2021-01-01'), \`score\`: 0.0})" \
         "CREATE (:\`Person\` {\`id\`: 'p3', \`age\`: 150, \`born\`: date('1900-01-01'), \`score\`: 10.5})" \
         "CREATE (:\`Person\` {\`id\`: 'p4'})"; do
  cli GRAPH.QUERY repro_ranges "$s" >/dev/null
done

(cd "$ROOT/tests/conformance/core" && "$BIN" compile cardinality.ttl --dialect falkordb --node-key id -o "$OUT/card" 2>/dev/null && "$BIN" compile ranges.ttl --dialect falkordb --node-key id -o "$OUT/ranges" 2>/dev/null)

echo "==== cardinality: compiled tags.minCount detail"
q=$(query_of "$OUT/card/queries.cypher" PersonShape.tags.minCount); echo "$q"
run repro_card "tags.minCount detail" "$q"
echo "==== cardinality: reduce per row"
run repro_card "distinct tags per node" "MATCH (v0:Person) RETURN v0.id, v0.tags, typeOf(v0.tags), size(reduce(x2 = [], x1 IN CASE typeOf(v0.\`tags\`) WHEN 'Null' THEN [] WHEN 'List' THEN v0.\`tags\` ELSE [v0.\`tags\`] END | CASE WHEN x1 IS NULL OR x1 IN x2 THEN x2 ELSE x2 + [x1] END)) AS n ORDER BY v0.id"
run repro_card "distinct tags without ORDER BY" "MATCH (v0:Person) RETURN v0.id, size(reduce(x2 = [], x1 IN CASE typeOf(v0.\`tags\`) WHEN 'Null' THEN [] WHEN 'List' THEN v0.\`tags\` ELSE [v0.\`tags\`] END | CASE WHEN x1 IS NULL OR x1 IN x2 THEN x2 ELSE x2 + [x1] END)) AS n"
run repro_card "with WHERE" "MATCH (v0:Person) WITH v0 WHERE NOT (size(reduce(x2 = [], x1 IN CASE typeOf(v0.\`tags\`) WHEN 'Null' THEN [] WHEN 'List' THEN v0.\`tags\` ELSE [v0.\`tags\`] END | CASE WHEN x1 IS NULL OR x1 IN x2 THEN x2 ELSE x2 + [x1] END)) >= 1) RETURN v0.id"
run repro_card "reduce plain list" "MATCH (v0:Person) RETURN v0.id, reduce(acc = [], x IN coalesce(v0.tags, []) | acc + [x]) AS l, reduce(s = 0, x IN coalesce(v0.tags, []) | s + 1) AS n"
run repro_card "reduce on literal first" "UNWIND [['a','b'], [], ['a','b','c']] AS l RETURN size(reduce(acc = [], x IN l | CASE WHEN x IN acc THEN acc ELSE acc + [x] END)) AS n"
run repro_card "IN on empty accumulator" "UNWIND [['a','b']] AS l RETURN reduce(acc = [], x IN l | CASE WHEN x IS NULL OR x IN acc THEN acc ELSE acc + [x] END) AS r, 'a' IN [] AS e, null IS NULL OR 'a' IN [] AS o"

echo "==== ranges: compiled born rules"
for rule in PersonShape.born.minExclusive PersonShape.born.maxInclusive; do
  q=$(query_of "$OUT/ranges/queries.cypher" $rule); echo "$q" | grep WHERE
  run repro_ranges "$rule detail" "$q"
done
echo "==== ranges: date comparisons across 1970"
run repro_ranges "literal comparisons" "RETURN date('1900-01-01') < date('2020-12-31') AS a, date('2000-01-01') > date('1900-01-01') AS b, date('1969-12-31') < date('1970-01-02') AS c, date('1960-01-01') < date('1965-01-01') AS d, date('1965-01-01') > date('1960-01-01') AS e, date('1900-01-01') = date('1900-01-01') AS f"
run repro_ranges "stored comparisons" "MATCH (p:Person) WHERE p.born IS NOT NULL RETURN p.id, p.born, p.born > date('1900-01-01') AS gt1900, p.born <= date('2020-12-31') AS le2020, p.born < date('1970-01-01') AS lt1970 ORDER BY p.id"
run repro_ranges "localdatetime before 1970" "RETURN localdatetime('1900-01-01T00:00:00') < localdatetime('2000-01-01T00:00:00') AS a, localdatetime('1969-12-31T23:00:00') < localdatetime('1970-01-01T01:00:00') AS b"
run repro_ranges "negative epoch arithmetic" "RETURN toString(date('1900-01-01')) AS d, date('1900-01-01') < date('1900-01-02') AS a, date('1969-01-01') < date('1969-06-01') AS b"
