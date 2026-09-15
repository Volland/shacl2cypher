#!/usr/bin/env bash
# Probe batch 13: (1) does a cached plan corrupt `reduce` distinct counts on repeated
# execution (core/cardinality.yaml fails through the runner but not in one-shot runs)?
# (2) reduce-free distinct counts, (3) temporal component accessors for an overflow-free
# comparison key, (4) whether double precision is lost in the database or the protocol.
# Uses graph `repro_card` from probes/11.
set -u
PODMAN=${PODMAN:-/opt/podman/bin/podman}
q() { printf '\n>> [%s] %s\n' "$1" "$2"; "$PODMAN" exec s2c-falkordb redis-cli GRAPH.RO_QUERY "$1" "$2" TIMEOUT 0 2>&1 | grep -v "execution time"; }
REDUCE="size(reduce(x2 = [], x1 IN CASE typeOf(v0.\`tags\`) WHEN 'Null' THEN [] WHEN 'List' THEN v0.\`tags\` ELSE [v0.\`tags\`] END | CASE WHEN x1 IS NULL OR x1 IN x2 THEN x2 ELSE x2 + [x1] END))"
DETAIL="CYPHER limit=9223372036854775807 sampleSize=5 MATCH (v0:\`Person\`) WITH v0 WHERE NOT ($REDUCE >= 1) RETURN v0.\`id\` AS id LIMIT \$limit"
SUMMARY="CYPHER limit=9223372036854775807 sampleSize=5 MATCH (v0:\`Person\`) WITH v0 WHERE NOT ($REDUCE >= 1) WITH count(*) AS n, collect(v0.\`id\`)[0..\$sampleSize] AS sample RETURN n, sample"
echo "==== (1) repeated executions of the same reduce query"
for i in 1 2 3; do q repro_card "$DETAIL"; done
q repro_card "$SUMMARY"; q repro_card "$SUMMARY"
q repro_card "$DETAIL"
q repro_card "CYPHER limit=9223372036854775807 MATCH (v0:\`Person\`) RETURN v0.\`id\` AS id, $REDUCE AS n ORDER BY id"
q repro_card "CYPHER limit=9223372036854775807 MATCH (v0:\`Person\`) RETURN v0.\`id\` AS id, $REDUCE AS n ORDER BY id"
echo "==== (2) reduce-free distinct counts"
LIST="CASE typeOf(v0.\`tags\`) WHEN 'Null' THEN [] WHEN 'List' THEN v0.\`tags\` ELSE [v0.\`tags\`] END"
for i in 1 2; do
q repro_card "MATCH (v0:\`Person\`) CALL { WITH v0 UNWIND $LIST AS x1 RETURN count(DISTINCT x1) >= 1 AS c2 } WITH v0, c2 WHERE NOT (c2) RETURN v0.\`id\` AS id ORDER BY id"
q repro_card "MATCH (v0:\`Person\`) WITH v0, $LIST AS l RETURN v0.\`id\` AS id, size([i IN range(0, size(l) - 1) WHERE NOT l[i] IN l[0..i]]) AS n ORDER BY id"
done
q repro_card "MATCH (v0:\`Person\`) CALL { WITH v0 UNWIND $LIST AS x1 RETURN count(DISTINCT CASE WHEN x1 <> 'b' THEN x1 END) AS c2 } RETURN v0.\`id\` AS id, c2 ORDER BY id"
q repro_card "UNWIND [1, 1.0, '1', null] AS x RETURN count(DISTINCT x) AS n"
echo "==== (3) temporal component accessors"
q repro_card "RETURN date('0005-03-04').year AS y, date('0005-03-04').month AS m, date('0005-03-04').day AS d, localdatetime('0999-01-02T09:05:07').hour AS h, localdatetime('0999-01-02T09:05:07').minute AS mi, localdatetime('0999-01-02T09:05:07').second AS s, localtime('09:05:07').hour AS th"
q repro_card "RETURN (CASE WHEN typeOf('abc') = 'Date' THEN 'abc' END).year AS s, (CASE WHEN typeOf(5) = 'Date' THEN 5 END).year AS i, (CASE WHEN typeOf(date('1900-01-01')) = 'Date' THEN date('1900-01-01') END).year AS d"
q repro_card "WITH date('1900-01-01') AS a, date('2020-12-31') AS b RETURN a.year * 10000 + a.month * 100 + a.day < b.year * 10000 + b.month * 100 + b.day AS lt"
q repro_card "WITH [date('0999-12-31'), date('1000-01-01')] AS ds RETURN [d IN ds | d.year * 10000 + d.month * 100 + d.day] AS keys"
echo "==== (4) double precision in the database vs the protocol"
q repro_card "RETURN 0.1 + 0.2 AS v, 0.1 + 0.2 = 0.30000000000000004 AS exact, 2.2250738585072014e-308 = 2.225073858507201e-308 AS distinct_min, 1.1449084417284436e190 = 1.14490844172844e190 AS distinct_big, toString(1.1449084417284436e190) AS s"
q repro_card "RETURN 1.1449084417284436e190 - 1.14490844172844e190 AS diff"
