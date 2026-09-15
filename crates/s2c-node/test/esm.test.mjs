// ES module and CommonJS entry points expose the same API.

import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { test } from 'node:test';

import binding, { compile, CompileError, Ladybug, version } from 'shacl2cypher';

const require = createRequire(import.meta.url);

test('import and require load the same module', async () => {
  const cjs = require('shacl2cypher');
  assert.equal(binding, cjs);
  assert.equal(compile, cjs.compile);
  assert.equal(CompileError, cjs.CompileError);
  assert.equal(Ladybug, cjs.Ladybug);
  assert.equal(version, cjs.version);
  await assert.rejects(compile({ shapes: [], dialect: 'neo4j' }), CompileError);
});
