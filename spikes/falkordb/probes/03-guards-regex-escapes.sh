#!/usr/bin/env bash
# Probe batch 3: re-checks batch 1 with property values instead of literals (plan-time
# constant checks), regex escapes as `render::quote` emits them (doubled backslashes),
# temporal edge cases, and identifier escaping. One Cypher query per line on graph
# `probe3`; `#` lines are headers. Tokens: %%BS%% -> backslash, %%NL%% -> newline.
set -u
PODMAN=${PODMAN:-/opt/podman/bin/podman}
q() {
  printf '\n>> %s\n' "$1"
  "$PODMAN" exec s2c-falkordb redis-cli GRAPH.QUERY probe3 "$1" 2>&1
}
"$PODMAN" exec s2c-falkordb redis-cli GRAPH.DELETE probe3 >/dev/null 2>&1
while IFS= read -r line; do
  line=${line//%%BS%%/\\}
  line=${line//%%NL%%/$'\n'}
  case "$line" in
    '') ;;
    '#'*) printf '\n==== %s\n' "$line" ;;
    *) q "$line" ;;
  esac
done <<'EOF'
# setup
CREATE (:V {id: 'i', v: 5}), (:V {id: 's', v: 'abcd'}), (:V {id: 'l', v: ['ab', 'c']}), (:V {id: 'd', v: date('2020-01-01')}), (:V {id: 'n'}), (:V {id: 'f', v: 2.5}), (:V {id: 'b', v: true})
# 1.4 guards over property values
MATCH (n:V) RETURN n.id, size(n.v) ORDER BY n.id
MATCH (n:V) RETURN n.id, typeOf(n.v) = 'String' AND size(n.v) > 3 ORDER BY n.id
MATCH (n:V) RETURN n.id, CASE WHEN typeOf(n.v) = 'String' THEN size(n.v) > 3 ELSE false END ORDER BY n.id
MATCH (n:V) RETURN n.id, CASE typeOf(n.v) WHEN 'String' THEN size(n.v) ELSE -1 END ORDER BY n.id
MATCH (n:V) WHERE typeOf(n.v) = 'String' AND size(n.v) > 3 RETURN n.id
MATCH (n:V) RETURN n.id, CASE WHEN typeOf(n.v) = 'String' THEN size(string.matchRegEx(n.v, 'b')) > 0 ELSE false END ORDER BY n.id
MATCH (n:V) RETURN n.id, [x IN CASE typeOf(n.v) WHEN 'Null' THEN [] WHEN 'List' THEN n.v ELSE [n.v] END WHERE NOT (CASE WHEN typeOf(x) = 'String' THEN size(x) >= 2 ELSE false END)] ORDER BY n.id
MATCH (n:V) RETURN n.id, NOT coalesce(n.v >= 3, false), n.v IN [5, 'abcd'] ORDER BY n.id
MATCH (n:V) RETURN n.id, toString(n.v) ORDER BY n.id
MATCH (n:V) WHERE n.id IN ['i', 's', 'f', 'b', 'd'] RETURN n.id, toString(n.v), toStringOrNull(n.v) ORDER BY n.id
MATCH (n:V) RETURN n.id, typeOf(n.v) IN ['Integer'] AND n.v >= -32768 AND n.v <= 32767 ORDER BY n.id
MATCH (n:V) RETURN n.id, CASE WHEN typeOf(n.v) = 'Integer' THEN n.v >= -32768 AND n.v <= 32767 ELSE false END ORDER BY n.id
# 1.2 regex escapes as rendered by quote() (doubled backslashes)
RETURN string.matchRegEx('A', '%%BS%%%%BS%%x{41}'), string.matchRegEx('A', '%%BS%%%%BS%%x41'), string.matchRegEx('é', '%%BS%%%%BS%%x{e9}'), string.matchRegEx('😀', '%%BS%%%%BS%%x{1F600}')
RETURN string.matchRegEx('é', '%%BS%%%%BS%%p{L}'), string.matchRegEx('٣', '%%BS%%%%BS%%d'), string.matchRegEx('٣', '%%BS%%%%BS%%p{Nd}'), string.matchRegEx('aa', '(a)%%BS%%%%BS%%1')
RETURN string.matchRegEx('a.b', 'a%%BS%%%%BS%%.b'), string.matchRegEx('axb', 'a%%BS%%%%BS%%.b'), string.matchRegEx('a\b', 'a%%BS%%%%BS%%%%BS%%%%BS%%b')
RETURN string.matchRegEx('😀', '^.$'), string.matchRegEx('😀x', '^..$'), size('😀')
RETURN string.matchRegEx('abc', '(?i:B)'), string.matchRegEx('A%%BS%%nb', '(?im)^B$'), string.matchRegEx('A%%BS%%nb', '(?s)(?s:.*)(?:b)(?s:.*)')
RETURN string.matchRegEx('M', '[%%BS%%%%BS%%x{41}-%%BS%%%%BS%%x{5A}]'), string.matchRegEx('m', '[%%BS%%%%BS%%x{41}-%%BS%%%%BS%%x{5A}]'), string.matchRegEx('&', '[%%BS%%%%BS%%&%%BS%%%%BS%%~]')
RETURN string.matchRegEx('b', '[a-z&&[^b]]'), string.matchRegEx('c', '[a-z&&[^b]]')
RETURN string.matchRegEx('x', '%%BS%%%%BS%%p{IsBasicLatin}')
RETURN string.matchRegEx('_', '[%%BS%%%%BS%%p{L}%%BS%%%%BS%%p{Nd}_]'), string.matchRegEx('a', '%%BS%%%%BS%%w'), string.matchRegEx('é', '%%BS%%%%BS%%w'), string.matchRegEx(' ', '%%BS%%%%BS%%s')
RETURN string.matchRegEx('ab', 'a(?#comment)b'), string.matchRegEx('aB', '(?i)a(?-i)B'), string.matchRegEx('ab', '%%BS%%%%BS%%Aab%%BS%%%%BS%%z')
# 1.3 temporal edge cases
RETURN typeOf(duration('-P1D')), duration('-P1D') IS NULL
RETURN typeOf(duration('P1Y2M3DT4H5M6.5S')), duration('P1Y2M3DT4H5M6.5S') IS NULL
RETURN duration('P1DT2H') = duration('PT26H'), duration('P1M') < duration('P31D'), duration('PT1.5S')
RETURN localdatetime('2020-01-01T10:00:00.5') = localdatetime('2020-01-01T10:00:00'), localtime('10:00:00.25') = localtime('10:00:00')
RETURN date('2020-02-30')
RETURN localdatetime('2020-01-01T10:00:00') = date('2020-01-01'), typeOf(localdatetime('2020-01-01'))
RETURN date('2020-01-01') = date('2020-01-01'), date('2020-01-01') IN [date('2020-01-01')], toString(date('2020-01-01')), toString(localdatetime('2020-01-01T10:00:00'))
RETURN localtime('24:00:00')
RETURN date('-0001-01-01'), date('+12020-01-01')
# 1.6 string escapes
RETURN 'x%%BS%%u00e9y', size('x%%BS%%u00e9y'), 'q%%BS%%u0027q'
RETURN 'multi%%NL%%line', size('multi%%NL%%line')
RETURN 'nul%%BS%%u0000x', size('nul%%BS%%u0000x')
RETURN '😀', size('😀'), '%%BS%%uD83D%%BS%%uDE00'
# 1.6 identifiers
CREATE (:`Sp ace` {`we ird`: 1, `dot.key`: 2})
MATCH (n:`Sp ace`) RETURN n.`we ird`, n.`dot.key`, keys(n), properties(n)
CREATE (:`Ünï` {id: 'u'})
MATCH (n:`Ünï`) RETURN labels(n)
CREATE (:`a%%BS%%u0041` {id: 'bu'})
MATCH (n) WHERE n.id = 'bu' RETURN labels(n)
CREATE (:`x%%BS%%`y` {id: 'bt'})
CREATE (:`x'y"z` {id: 'q'})
MATCH (n {id: 'q'}) RETURN labels(n)
CREATE (:`` {id: 'e'})
RETURN 1 AS `a b`
CREATE (:V {id: 'kk', `k``k`: 1})
CREATE (:`MATCH` {id: 'kw'})
MATCH (n:`MATCH`) RETURN n.id
MATCH (n) RETURN count(n)
EOF
