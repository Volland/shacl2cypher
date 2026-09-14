// Spike for task 1.4: Neo4j 5 type predicates, null semantics, regex anchoring
// and the subquery constructs the renderer relies on. Run with --fail-at-end.

RETURN '1.4 int' AS probe, 1 IS :: INTEGER AS r;
RETURN '1.4 int-not-float' AS probe, 1 IS :: FLOAT AS r;
RETURN '1.4 float' AS probe, 1.5 IS :: FLOAT AS r;
RETURN '1.4 string' AS probe, '1' IS :: STRING AS r;
RETURN '1.4 bool' AS probe, true IS :: BOOLEAN AS r;
RETURN '1.4 null IS :: INTEGER' AS probe, null IS :: INTEGER AS r;
RETURN '1.4 null IS :: INTEGER NOT NULL' AS probe, null IS :: INTEGER NOT NULL AS r;
RETURN '1.4 list int' AS probe, [1, 2] IS :: LIST<INTEGER> AS r;
RETURN '1.4 mixed list ANY' AS probe, [1, 'a'] IS :: LIST<ANY> AS r;
RETURN '1.4 mixed list as INTEGER' AS probe, [1, 'a'] IS :: LIST<INTEGER> AS r;
RETURN '1.4 list with null' AS probe, [1, null] IS :: LIST<INTEGER> AS r;
RETURN '1.4 scalar IS LIST' AS probe, 1 IS :: LIST<ANY> AS r;
RETURN '1.4 date' AS probe, date('2020-01-01') IS :: DATE AS r;
RETURN '1.4 zoned datetime' AS probe, datetime() IS :: ZONED DATETIME AS r;
RETURN '1.4 local datetime' AS probe, localdatetime() IS :: LOCAL DATETIME AS r;
RETURN '1.4 local datetime as zoned' AS probe, localdatetime() IS :: ZONED DATETIME AS r;
RETURN '1.4 local time' AS probe, localtime() IS :: LOCAL TIME AS r;
RETURN '1.4 duration' AS probe, duration('P1D') IS :: DURATION AS r;
RETURN '1.4 date string not DATE' AS probe, '2020-01-01' IS :: DATE AS r;

RETURN 'null cmp' AS probe, ('abc' >= 18) AS r;
RETURN 'coalesce cmp' AS probe, coalesce('abc' >= 18, false) AS r;
RETURN 'NOT null' AS probe, (NOT null) IS NULL AS r;
RETURN 'int range short' AS probe, 40000 >= -32768 AND 40000 <= 32767 AS r;

RETURN 'regex anchored' AS probe, 'abc123' =~ '\\d+' AS r;
RETURN 'regex substring wrap' AS probe, 'abc123' =~ '(?s).*(?:\\d+).*' AS r;
RETURN 'regex flag i' AS probe, 'ABC' =~ '(?i)(?s).*(?:^abc$).*' AS r;
RETURN 'regex multiline newline' AS probe, 'x\nabc' =~ '(?s).*(?:^abc).*' AS r;
RETURN 'regex backreference' AS probe, 'aa' =~ '(a)\\1' AS r;
RETURN 'regex unicode' AS probe, 'Łódź' =~ '\\p{L}+' AS r;
RETURN 'regex on null' AS probe, (null =~ 'a') IS NULL AS r;
RETURN 'NOT NULL type check on null' AS probe, null IS :: INTEGER NOT NULL AS r;
RETURN 'union type datetime' AS probe, localdatetime() IS :: ZONED DATETIME | LOCAL DATETIME AS r;
RETURN 'guarded size on int' AS probe, CASE WHEN 5 IS :: STRING THEN size(5) >= 1 ELSE false END AS r;
RETURN 'guarded regex on int' AS probe, CASE WHEN 5 IS :: STRING THEN 5 =~ 'a' ELSE false END AS r;

RETURN 'string escape' AS probe, 'O\'Brien \\ x' AS r;
RETURN 'backtick ident' AS probe, 1 AS `we``ird`;
RETURN 'unicode escape' AS probe, 'Ł' AS r;

CREATE (:S2cProbe:Person {id: 'p1', name: 'Ann', age: 30, tags: ['a', 'b']}),
       (:S2cProbe:Person {id: 'p2', age: '10', tags: []}),
       (:S2cProbe:Employee {id: 'e1'});
MATCH (a:S2cProbe {id: 'p1'}), (b:S2cProbe {id: 'p2'}) CREATE (a)-[:KNOWS {since: date('2020-01-01')}]->(b);

MATCH (n:S2cProbe) WHERE COUNT { (n)-[:KNOWS]->() } >= 1 RETURN 'COUNT{} in WHERE' AS probe, collect(n.id) AS r;
MATCH (n:S2cProbe) RETURN 'COUNT{} in bool expr' AS probe, collect([n.id, COUNT { (n)-[:KNOWS]->() } = 1]) AS r;
MATCH (n:S2cProbe) WHERE NOT EXISTS { MATCH (n)-[:KNOWS]->(m) WHERE NOT EXISTS { (m)-[:KNOWS]->() } } RETURN 'nested EXISTS' AS probe, collect(n.id) AS r;
MATCH (n:S2cProbe) RETURN 'list normalize' AS probe, collect([n.id, CASE WHEN n.tags IS NULL THEN [] WHEN n.tags IS :: LIST<ANY> THEN n.tags ELSE [n.tags] END]) AS r;
MATCH (n:S2cProbe) RETURN 'null-safe range' AS probe, collect([n.id, all(v IN CASE WHEN n.age IS NULL THEN [] WHEN n.age IS :: LIST<ANY> THEN n.age ELSE [n.age] END WHERE coalesce(v >= 18, false))]) AS r;
MATCH (n:S2cProbe) WHERE n:Person|Employee RETURN 'label expr' AS probe, count(n) AS r;
MATCH (n:S2cProbe) RETURN 'keys closed' AS probe, collect([n.id, [k IN keys(n) WHERE NOT k IN ['id', 'name']]]) AS r;
MATCH (n:S2cProbe:Person) WHERE n.id = 'nope' WITH count(*) AS c, collect(n)[0..3] AS s RETURN 'zero-row summary' AS probe, [c, size(s)] AS r;
MATCH (n:S2cProbe:Person) RETURN 'map row' AS probe, {ruleId: 'r1', focus: {label: 'Person', keyValue: n.id, elementId: elementId(n)}} AS r LIMIT 1;
MATCH (n:S2cProbe:Person) RETURN 'LIMIT expr' AS probe, n.id AS r LIMIT coalesce(null, 9223372036854775807);
MATCH ()-[r:KNOWS]->() WHERE r.since IS NOT NULL RETURN 'rel focus' AS probe, count(r) AS r;
MATCH (n:S2cProbe {id: 'p1'})-[:KNOWS*1..10]->(m) RETURN 'var-length' AS probe, collect(m.id) AS r;
RETURN 'message replace' AS probe, replace(replace('Person {$this} age {?value}', '{$this}', 'p7'), '{?value}', toString(-3)) AS r;

MATCH (n:S2cProbe) DETACH DELETE n;
