// Spike for task group 6: Neo4j 5 constructs the renderer will emit.
:param sampleSize => 2
:param limit => null

CREATE (:R:Person {id: 'p1', name: 'Ann', tags: ['a', 'a', 'b'], age: 30}),
       (:R:Person {id: 'p2', name: 'Bob', tags: 'solo', age: 'x'}),
       (:R:Employee {id: 'e1'}),
       (:R:Company {id: 'c1', name: 'Acme'});
MATCH (a:R {id: 'p1'}), (c:R {id: 'c1'}) CREATE (a)-[:WORKS_FOR]->(c), (a)-[:WORKS_FOR]->(c);
MATCH (a:R {id: 'p1'}), (b:R {id: 'p2'}) CREATE (a)-[:KNOWS {since: date('2020-01-01')}]->(b);

RETURN 'raw newline in literal' AS probe, 'a
b' AS r;
RETURN 'unicode escape' AS probe, 'aA' AS r;

MATCH (n:R:Person) RETURN 'distinct list count via reduce' AS probe, collect([n.id, size(reduce(acc = [], x IN CASE WHEN n.tags IS NULL THEN [] WHEN n.tags IS :: LIST<ANY> THEN n.tags ELSE [n.tags] END | CASE WHEN x IS NULL OR x IN acc THEN acc ELSE acc + [x] END))]) AS r;
MATCH (n:R:Person) RETURN 'COUNT UNWIND DISTINCT' AS probe, collect([n.id, COUNT { UNWIND CASE WHEN n.tags IS :: LIST<ANY> THEN n.tags ELSE [n.tags] END AS x WITH DISTINCT x WHERE x IS NOT NULL RETURN x }]) AS r;
MATCH (n:R:Person) RETURN 'COUNT distinct rel ends' AS probe, collect([n.id, COUNT { MATCH (n)-[:WORKS_FOR]->(m) RETURN DISTINCT m }]) AS r;
MATCH (n:R:Person) WHERE COUNT { MATCH (n)-[:WORKS_FOR]->(m) RETURN DISTINCT m } >= 1 RETURN 'COUNT DISTINCT in WHERE' AS probe, collect(n.id) AS r;

MATCH (n:R:Person) UNWIND CASE WHEN n.tags IS NULL THEN [] WHEN n.tags IS :: LIST<ANY> THEN n.tags ELSE [n.tags] END AS v WITH DISTINCT n, v WHERE v IS NOT NULL AND NOT (CASE WHEN v IS :: STRING NOT NULL THEN size(v) >= 2 ELSE false END) RETURN 'per-value unwind rows' AS probe, collect([n.id, v]) AS r;
MATCH (n:R:Person)-[:WORKS_FOR]->(v) WITH DISTINCT n, v RETURN 'per-value rel rows distinct' AS probe, count(*) AS r;

MATCH (n:R) WHERE n:Person|Employee RETURN 'label disjunction' AS probe, count(n) AS r;
MATCH (n:R:Person) RETURN 'dynamic label test' AS probe, collect([n.id, n:Company OR n:Person]) AS r;

RETURN 'temporal constructors' AS probe, [datetime('2020-01-01T10:00:00Z'), localdatetime('2020-01-01T10:00:00'), time('10:00:00+01:00'), localtime('10:00:00'), duration('P1DT2H')] AS r;
RETURN 'decimal and exponent literals' AS probe, [1.50, 1.5e3, -0.0] AS r;

RETURN 'regex scoped flags' AS probe, ['X\nabc' =~ '(?i)(?s:.*)(?:ABC)(?s:.*)', 'X\nabc' =~ '(?s:.*)(?:^abc)(?s:.*)', 'X\nabc' =~ '(?m)(?s:.*)(?:^abc)(?s:.*)', 'a\nb' =~ '(?s:.*)(?:a.b)(?s:.*)', 'a\nb' =~ '(?s)(?s:.*)(?:a.b)(?s:.*)'] AS r;
RETURN 'regex hex escapes' AS probe, ['b' =~ '[\\x{62}-\\x{64}]', 'Ł' =~ '[\\x{141}]', 'a&b' =~ '.*[\\&].*'] AS r;

MATCH (n:R:Person) RETURN 'pair lists equals' AS probe, collect([n.id, all(x IN [1, 2] WHERE x IN [2, 1]) AND all(x IN [2, 1] WHERE x IN [1, 2])]) AS r;
MATCH (n:R:Person) RETURN 'pair nodes via NOT EXISTS' AS probe, collect([n.id, NOT EXISTS { MATCH (n)-[:WORKS_FOR]->(m) WHERE NOT EXISTS { MATCH (n)-[:KNOWS]->(m) } }]) AS r;
MATCH (n:R:Person) RETURN 'closed keys' AS probe, collect([n.id, [k IN keys(n) WHERE NOT k IN ['id', 'name']]]) AS r;

MATCH (n:R:Person) WHERE n.id = 'nope' WITH count(n) AS c, collect({id: n.id})[0..$sampleSize] AS s RETURN 'zero-row summary with param' AS probe, [c, size(s)] AS r;
MATCH (n:R:Person) RETURN 'LIMIT with null param' AS probe, n.id AS r LIMIT coalesce($limit, 9223372036854775807);
RETURN 'toString of values' AS probe, [toString(1.5), toString(date('2020-01-01')), toString(true), toString(30)] AS r;
MATCH ()-[r:KNOWS]->() RETURN 'relationship identity' AS probe, collect([elementId(r), r.since]) AS r;
MATCH (n:R:Person) RETURN 'map with null' AS probe, {keyValue: n.id, missing: null} AS r LIMIT 1;

MATCH (n:R) DETACH DELETE n;
