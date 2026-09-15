#!/usr/bin/env bash
# Probe batch 5: making value tests total without short-circuiting (toStringOrNull),
# which functions raise on mixed values, whether CASE evaluates every branch, regex
# code-point escapes replacing \x{...}, and duration/date leniency. Uses graph `probe3`
# from batch 3 (V nodes with v: 5, 'abcd', ['ab','c'], date, absent, 2.5, true).
# One query per line; `#` headers. Tokens: %%BS%% -> backslash.
set -u
PODMAN=${PODMAN:-/opt/podman/bin/podman}
q() {
  printf '\n>> %s\n' "$1"
  "$PODMAN" exec s2c-falkordb redis-cli GRAPH.QUERY probe3 "$1" 2>&1
}
while IFS= read -r line; do
  line=${line//%%BS%%/\\}
  case "$line" in
    '') ;;
    '#'*) printf '\n==== %s\n' "$line" ;;
    *) q "$line" ;;
  esac
done <<'EOF'
# setup: a node-valued and map-like stored values are impossible, so add an edge for entity tests
MATCH (a:V {id: 'i'}), (b:V {id: 's'}) CREATE (a)-[:R {w: [1, 2]}]->(b)
# toStringOrNull totality
RETURN toStringOrNull([1, 2]), toStringOrNull({a: 1}), toStringOrNull(null), toStringOrNull(point({latitude: 1, longitude: 2})), toStringOrNull(duration('P1D'))
MATCH (a:V {id: 'i'})-[r:R]->(b) RETURN toStringOrNull(a), toStringOrNull(r), toStringOrNull(r.w)
MATCH (n:V) RETURN n.id, size(toStringOrNull(n.v)) ORDER BY n.id
MATCH (n:V) RETURN n.id, CASE WHEN typeOf(n.v) = 'String' THEN size(toStringOrNull(n.v)) >= 3 ELSE false END ORDER BY n.id
MATCH (n:V) RETURN n.id, CASE typeOf(n.v) WHEN 'String' THEN size(toStringOrNull(n.v)) >= 3 WHEN 'Integer' THEN size(toStringOrNull(n.v)) >= 1 ELSE false END ORDER BY n.id
MATCH (n:V) RETURN n.id, CASE WHEN typeOf(n.v) = 'String' THEN size(string.matchRegEx(toStringOrNull(n.v), 'b')) > 0 ELSE false END ORDER BY n.id
MATCH (n:V) RETURN n.id, [x IN CASE typeOf(n.v) WHEN 'Null' THEN [] WHEN 'List' THEN n.v ELSE [n.v] END WHERE NOT (CASE WHEN typeOf(x) = 'String' THEN size(toStringOrNull(x)) >= 2 ELSE false END)] ORDER BY n.id
MATCH (n:V) RETURN n.id, all(x IN CASE typeOf(n.v) WHEN 'Null' THEN [] WHEN 'List' THEN n.v ELSE [n.v] END WHERE CASE WHEN typeOf(x) = 'String' THEN size(string.matchRegEx(toStringOrNull(x), '^a')) > 0 ELSE false END) ORDER BY n.id
MATCH (n:V) WITH n UNWIND CASE typeOf(n.v) WHEN 'Null' THEN [] WHEN 'List' THEN n.v ELSE [n.v] END AS v1 WITH n, v1 WHERE v1 IS NOT NULL AND NOT (CASE WHEN typeOf(v1) = 'String' THEN size(toStringOrNull(v1)) >= 2 ELSE false END) RETURN n.id, v1 ORDER BY n.id
# CASE branch evaluation
RETURN CASE WHEN false THEN 1 / 0 ELSE 1 END
MATCH (n:V) RETURN n.id, CASE WHEN typeOf(n.v) = 'List' THEN n.v[0] ELSE null END ORDER BY n.id
MATCH (n:V) RETURN n.id, CASE WHEN typeOf(n.v) = 'String' THEN toLower(n.v) ELSE null END ORDER BY n.id
MATCH (n:V) RETURN n.id, CASE WHEN typeOf(n.v) = 'String' THEN date(n.v) ELSE null END ORDER BY n.id
# WHERE filter short-circuiting shapes
MATCH (n:V) WHERE NOT (typeOf(n.v) = 'String' AND size(n.v) > 3) RETURN n.id ORDER BY n.id
MATCH (n:V) WHERE typeOf(n.v) <> 'String' OR size(n.v) <= 3 RETURN n.id ORDER BY n.id
MATCH (n:V) WHERE CASE WHEN typeOf(n.v) = 'String' THEN size(n.v) > 3 ELSE false END RETURN n.id ORDER BY n.id
# other functions over mixed values
MATCH (n:V) RETURN n.id, keys(n.v) ORDER BY n.id
MATCH (n:V) RETURN n.id, n.v.x ORDER BY n.id
MATCH (n:V) RETURN n.id, n.v[0] ORDER BY n.id
MATCH (n:V) RETURN n.id, toInteger(n.v) ORDER BY n.id
MATCH (n:V) RETURN n.id, toIntegerOrNull(n.v), toFloatOrNull(n.v), toBooleanOrNull(n.v) ORDER BY n.id
MATCH (n:V) RETURN n.id, replace(toStringOrNull(n.v), 'a', 'A') ORDER BY n.id
MATCH (n:V) RETURN n.id, n.v IN ['abcd', 5], n.v = date('2020-01-01'), n.v < date('2021-01-01') ORDER BY n.id
MATCH (n:V) RETURN n.id, labels(n.v) ORDER BY n.id
MATCH (n:V) RETURN n.id, n.v:Foo ORDER BY n.id
MATCH (n:V) RETURN n.id, id(n.v) ORDER BY n.id
MATCH (n:V) RETURN n.id, typeOf(n.v) IN ['String'] AND n.v STARTS WITH 'a' ORDER BY n.id
MATCH (n:V) RETURN n.id, coalesce(n.v STARTS WITH 'a', false) ORDER BY n.id
# regex code points replacing \x{...}
RETURN string.matchRegEx('A', '%%BS%%%%BS%%u0041'), string.matchRegEx('é', '%%BS%%%%BS%%u00e9'), string.matchRegEx('é', '%%BS%%%%BS%%u00E9')
RETURN string.matchRegEx('M', '[%%BS%%%%BS%%u0041-%%BS%%%%BS%%u005A]'), string.matchRegEx('m', '[%%BS%%%%BS%%u0041-%%BS%%%%BS%%u005A]')
RETURN string.matchRegEx('😀', '%%BS%%%%BS%%uD83D%%BS%%%%BS%%uDE00'), string.matchRegEx('😀', '[😀-😂]'), string.matchRegEx('😁', '[😀-😂]'), string.matchRegEx('😃', '[😀-😂]')
RETURN string.matchRegEx('A', '%%BS%%%%BS%%x{0041}'), string.matchRegEx('A', '%%BS%%%%BS%%101'), string.matchRegEx('A', '%%BS%%%%BS%%o{101}')
RETURN string.matchRegEx('&', '[%%BS%%%%BS%%&%%BS%%%%BS%%~]'), string.matchRegEx('~', '[%%BS%%%%BS%%&%%BS%%%%BS%%~]'), string.matchRegEx('-', '[a%%BS%%%%BS%%-z]'), string.matchRegEx('[', '[%%BS%%%%BS%%[]')
RETURN string.matchRegEx('a.b*', '%%BS%%%%BS%%Qa.b*%%BS%%%%BS%%E'), string.matchRegEx('axbb', '%%BS%%%%BS%%Qa.b*%%BS%%%%BS%%E')
RETURN string.matchRegEx(' ', '[%%BS%%%%BS%%u0009-%%BS%%%%BS%%u000D%%BS%%%%BS%%u0020]'), string.matchRegEx('x', '[^%%BS%%%%BS%%u0000-%%BS%%%%BS%%uFFFF]'), string.matchRegEx('😀', '[^%%BS%%%%BS%%u0000-%%BS%%%%BS%%uFFFF]')
RETURN string.matchRegEx('ab', '(?:a|b)+'), string.matchRegEx('aaa', 'a{2,}'), string.matchRegEx('a{', 'a%%BS%%%%BS%%{')
RETURN string.matchRegEx('ab', '[%%BS%%%%BS%%p{L}-[b]]')
# duration and date leniency
RETURN duration('P1Y') IS NULL, duration('P0D') IS NULL, duration('PT1H30M') IS NULL, duration('P1W') IS NULL, duration('PT0.5S') IS NULL, duration('P1.5D') IS NULL
RETURN duration('P1Y') = duration('P12M'), duration('P1Y') = duration('P365D'), duration('P1D') = duration('PT24H'), duration('P1M') = duration('P30D')
RETURN date('2020-13-01')
RETURN date('2020-1-1'), date('20200101'), date('2020-001')
RETURN localdatetime('2020-01-01T25:00:00')
RETURN localdatetime('2020-01-01 10:00:00'), localdatetime('2020-01-01T10:00')
RETURN localtime('10:00'), localtime('10:61:00')
EOF
