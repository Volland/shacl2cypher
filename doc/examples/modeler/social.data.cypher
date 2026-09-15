CREATE (:Person {id: 'p1', email: 'ann@example.org', born: date('1990-04-01'), createdAt: timestamp('2024-01-01T10:00:00+00:00')});
CREATE (:Person {id: 'p2', born: date('1985-02-11'), createdAt: timestamp('2024-02-01T10:00:00+00:00')});
CREATE (:Person {id: 'p3', email: 'cy@example.org'});
CREATE (:Company {id: 'c1', vat: 'DE123'});
CREATE (:Car {vin: 'VIN-1', seats: 5});
MATCH (a:Person {id:'p1'}), (b:Person {id:'p2'}) CREATE (a)-[:KNOWS {since: date('2020-01-01')}]->(b);
MATCH (a:Person {id:'p1'}), (c:Car {vin:'VIN-1'}) CREATE (a)-[:OWNS {since: date('2021-06-01')}]->(c);
MATCH (a:Person {id:'p3'}), (c:Company {id:'c1'}) CREATE (a)-[:LIKES]->(c);
