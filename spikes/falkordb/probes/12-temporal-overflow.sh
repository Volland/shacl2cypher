#!/usr/bin/env bash
# Probe batch 12: temporal comparisons between values more than ~68 years apart
# (2^31 seconds) are wrong on FalkorDB 4.20.4. Checks the boundary, and whether ISO
# string comparison (`toString`) is an exact replacement for dates and date-times.
set -u
PODMAN=${PODMAN:-/opt/podman/bin/podman}
q() { printf '\n>> %s\n' "$1"; "$PODMAN" exec s2c-falkordb redis-cli GRAPH.RO_QUERY smoke "$1" 2>&1 | grep -v "execution time\|Cached execution"; }
"$PODMAN" exec s2c-falkordb redis-cli GRAPH.QUERY smoke "RETURN 1" >/dev/null
while IFS= read -r line; do
  case "$line" in ''|'#'*) ;; *) q "$line" ;; esac
done <<'PROBES'
# boundary: 2^31 seconds is 68.05 years
RETURN date('1950-01-01') < date('2020-01-01') AS y70neg, date('1971-01-01') < date('2045-01-01') AS y74pos, date('1971-01-01') < date('2030-01-01') AS y59pos, date('2000-01-01') < date('2068-01-01') AS y68, date('2000-01-01') < date('2068-02-01') AS y68m1
RETURN date('2020-01-01') > date('1950-01-01') AS gt, date('1950-01-01') <= date('2020-01-01') AS le, date('2020-01-01') >= date('1950-01-01') AS ge, date('1900-01-01') = date('2000-01-01') AS eq, date('1900-01-01') <> date('2000-01-01') AS ne
RETURN localdatetime('1950-01-01T00:00:00') < localdatetime('2020-01-01T00:00:00') AS ldt, localtime('00:00:01') < localtime('23:59:59') AS lt
# ISO strings of early and late years
RETURN toString(date('0005-03-04')) AS d5, toString(date('0999-12-31')) AS d999, toString(date('9999-12-31')) AS d9999, toString(localdatetime('0999-01-01T09:05:00')) AS ldt999, toString(localtime('09:05:00')) AS lt
# string comparison as a replacement
RETURN toString(date('1900-01-01')) < toString(date('2020-12-31')) AS a, toString(date('2000-01-01')) > '1900-01-01' AS b, toString(date('0999-12-31')) < toString(date('1000-01-01')) AS c, toString(localdatetime('1950-01-01T10:00:00')) < '2020-01-01T09:00:00' AS d
RETURN typeOf(date('2000-01-01')) = 'Date' AND coalesce(toStringOrNull(date('2000-01-01')) > '1900-01-01', false) AS guarded, typeOf('3000-01-01') = 'Date' AND coalesce(toStringOrNull('3000-01-01') > '1900-01-01', false) AS stringValue
# durations over many years (kept numeric)
RETURN duration('P100Y') > duration('P1D') AS big, duration('P10Y') > duration('P1D') AS small
PROBES
