#!/usr/bin/env bash
# Probe batch 15: LIMIT with large parameter values on the arm64 and amd64 builds of
# falkordb:v4.20.4. The runner sends i64::MAX for "no limit"; CI (amd64) got no detail rows.
set -u
PODMAN=${PODMAN:-/opt/podman/bin/podman}
for c in s2c-falkordb s2c-falkordb-amd64; do
  echo "######## $c ($("$PODMAN" exec $c uname -m))"
  for limit in 2 1000 2147483647 2147483648 4294967295 4294967296 9007199254740991 4611686018427387904 9223372036854775806 9223372036854775807; do
    param=$("$PODMAN" exec $c redis-cli GRAPH.RO_QUERY arch_probe "CYPHER limit=$limit MATCH (n:Person) RETURN count(*) AS c, 0 AS z LIMIT \$limit" 2>&1 | sed -n '3p')
    rows=$("$PODMAN" exec $c redis-cli GRAPH.RO_QUERY arch_probe "CYPHER limit=$limit MATCH (n:Person) RETURN n.id LIMIT \$limit" 2>&1 | grep -c '^p')
    literal=$("$PODMAN" exec $c redis-cli GRAPH.RO_QUERY arch_probe "MATCH (n:Person) RETURN n.id LIMIT $limit" 2>&1 | grep -c '^p')
    printf '  limit %-20s param rows=%s literal rows=%s\n' "$limit" "$rows" "$literal"
  done
done
