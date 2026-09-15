# shacl2cypher for Node.js

Compile [SHACL](https://www.w3.org/TR/shacl/) shapes into named, read-only Cypher queries, and validate Neo4j 5 or LadybugDB graphs from Node.js 18 or newer.

```ts
import { compile, Ladybug, renderReport, exitCode } from 'shacl2cypher';

const compilation = await compile({ shapes: ['shapes/person.ttl'], dialect: 'neo4j', nodeKey: 'id' });
compilation.write('out');

const db = await Ladybug.open('graph.lbug');
try {
  const report = await db.validate({ shapes: ['shapes/person.ttl'], nodeKey: 'id' });
  console.log(renderReport(report, 'table'));
  process.exitCode = exitCode(report, 'violation');
} finally {
  await db.close();
}
```

Prebuilt addons with both database backends are installed for Linux x64 and arm64 (glibc) and macOS arm64. Manifest and report objects use the same camelCase keys as `manifest.json` and `--format json`. See the [project README](https://github.com/Volland/shacl2cypher#readme) for the mapping rules and the `s2c:` annotation vocabulary.
