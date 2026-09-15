#!/usr/bin/env bash
# Probe batch 1: typeOf strings, temporal storage, regex dialect, short-circuiting,
# mixed-type comparisons and literal escaping. One Cypher query per line; lines
# starting with `#` are section headers. Queries run with GRAPH.QUERY on graph `probe`.
set -u
PODMAN=${PODMAN:-/opt/podman/bin/podman}
q() {
  printf '\n>> %s\n' "$1"
  "$PODMAN" exec s2c-falkordb redis-cli GRAPH.QUERY probe "$1" 2>&1
}
"$PODMAN" exec s2c-falkordb redis-cli GRAPH.DELETE probe >/dev/null 2>&1
while IFS= read -r line; do
  case "$line" in
    '') ;;
    '#'*) printf '\n==== %s\n' "$line" ;;
    *) q "$line" ;;
  esac
done <<'EOF'
# setup
CREATE (:P {id: 'a', s: 'abc', i: 5, f: 1.5, b: true, l: [1, 2], ls: ['ab', 'c'], d: date('2020-01-01'), ldt: localdatetime('2020-01-01T10:00:00'), lt: localtime('10:00:00'), du: duration('P1DT2H')})
# 1.3 typeOf over stored properties (also 1.6 dynamic n[k])
MATCH (n:P) UNWIND keys(n) AS k RETURN k, typeOf(n[k]) ORDER BY k
RETURN typeOf(null), typeOf({a: 1}), typeOf(point({latitude: 1, longitude: 2})), typeOf(vecf32([1.0]))
RETURN typeOf([]), typeOf(['a', 1]), typeOf(1.0), typeOf(-0)
# 1.3 zoned temporals
RETURN datetime('2020-01-01T10:00:00Z')
RETURN time('10:00:00+01:00')
RETURN localdatetime('2020-01-01T10:00:00+01:00')
RETURN localdatetime('2020-01-01T10:00:00Z')
RETURN typeOf(localdatetime('2020-01-01T10:00:00')), typeOf(localtime('10:00:00')), typeOf(date('2020-01-01')), typeOf(duration('P1D'))
RETURN localdatetime('2020-01-01T10:00:00.5'), localtime('10:00:00.25'), duration('-P1D'), duration('P1Y2M3DT4H5M6.5S')
RETURN date('2020-01-01') < date('2020-01-02'), localdatetime('2020-01-01T10:00:00') < localdatetime('2020-01-01T11:00:00'), duration('P1D') < duration('P2D')
CREATE (:T {x: [date('2020-01-01')]})
# 1.2 regex operator and function
RETURN 'a' =~ 'a'
RETURN string.matchRegEx('xabcx', 'abc')
RETURN string.matchRegEx('abc', '^b'), string.matchRegEx('abc', 'b$'), string.matchRegEx('abc', '^abc$')
RETURN string.matchRegEx('ABC', '(?i)abc')
RETURN size('a\nb'), string.matchRegEx('a\nb', 'a.b'), string.matchRegEx('a\nb', '(?s)a.b')
RETURN string.matchRegEx('a\nb', '^b$'), string.matchRegEx('a\nb', '(?m)^b$')
RETURN string.matchRegEx('A', '\x{41}'), string.matchRegEx('A', '\x41'), string.matchRegEx('A', 'A')
RETURN string.matchRegEx('é', '\p{L}'), string.matchRegEx('É', '\p{Lu}'), string.matchRegEx('é', '^.$')
RETURN string.matchRegEx('aa', '(a)\1')
RETURN string.matchRegEx('ab', '(?=a)a')
RETURN string.matchRegEx('٣', '\d'), string.matchRegEx('a', '[[:alpha:]]'), string.matchRegEx('aaa', 'a++')
RETURN string.matchRegEx('abc', '(')
RETURN string.matchRegEx('a b', '(?x)a b'), string.matchRegEx('ab', '(?s:.*)b')
RETURN string.matchRegEx(5, 'a')
RETURN string.matchRegEx(null, 'a')
# 1.4 short-circuiting and guards
RETURN size(5)
RETURN false AND size(5) > 1
RETURN typeOf(5) = 'String' AND size(5) > 1
RETURN true OR size(5) > 1
RETURN CASE WHEN typeOf(5) = 'String' THEN size(5) > 1 ELSE false END
RETURN NOT (typeOf(5) = 'String' AND size(5) > 1)
# 1.4 mixed-type comparisons
RETURN 1 < 'a', 1 = 'a', [1] = 'a', date('2020-01-01') < 5, coalesce(1 < 'a', false), 1 = 1.0, 1 < 1.5
RETURN null = null, [1, null] = [1, null], 1 IN [1, 2], null IN [1], 'a' IN ['a', 1]
# 1.6 string escapes and identifiers
RETURN 'O\'Brien', 'a\\b', 'xéy', size('xéy'), "dq\"x", 'tab\tx'
RETURN 'multi
line'
CREATE (:`We``ird` {id: 'w'})
MATCH (n:`We``ird`) RETURN labels(n), n.id
RETURN `x`
MATCH (n) WHERE n.`i` = 5 RETURN n.id
RETURN 9223372036854775807, -9223372036854775808, 1.0e308, 0.1
EOF
