'use strict';

// Release smoke test for a built Node addon.
// Usage: node tests/bindings/smoke.js <package directory> <s2c-fixture>

const assert = require('node:assert/strict');
const { execFileSync } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const CORE = path.resolve(__dirname, '..', 'conformance', 'core');

async function main([packageDir, fixtureTool]) {
  const s2c = require(path.resolve(packageDir));
  assert.deepEqual(s2c.availableBackends(), ['neo4j', 'ladybug']);

  const compilation = await s2c.compile({ shapes: [path.join(CORE, 'in-has-value.ttl')], dialect: 'neo4j', nodeKey: 'id' });
  assert.equal(compilation.manifest.compilerVersion, s2c.version);
  assert.match(compilation.cypher, /\/\/ name: PersonShape\.status\.in/);

  const database = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 's2c-smoke-')), 'graph.lbug');
  execFileSync(fixtureTool, ['ladybug-db', path.join(CORE, 'in-has-value.yaml'), database]);
  const db = await s2c.Ladybug.open(database);
  try {
    const report = await db.validate({ shapes: [path.join(CORE, 'in-has-value.ttl')], nodeKey: 'id' });
    assert.equal(report.summary.violations, 4);
  } finally {
    await db.close();
  }
  console.log(`shacl2cypher ${s2c.version} ok (backends: ${s2c.availableBackends().join(', ')})`);
}

main(process.argv.slice(2)).catch((error) => {
  console.error(error);
  process.exit(1);
});
