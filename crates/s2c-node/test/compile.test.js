'use strict';

// Compile through the Node binding: sources, options, errors, parity with the CLI.

const assert = require('node:assert/strict');
const fs = require('node:fs');
const http = require('node:http');
const path = require('node:path');
const { test } = require('node:test');

const s2c = require('..');
const { CORE_FIXTURES, SHAPES, cli, tempDir, runCli } = require('./helpers');

const PREFIXES =
  '@prefix sh: <http://www.w3.org/ns/shacl#> .\n' +
  '@prefix ex: <http://example.org/> .\n' +
  '@prefix s2c: <https://w3id.org/shacl2cypher#> .\n';

// @lat: [[tests#Node Binding#Compile From Files And Documents]]
test('compiles files and in-memory documents', async () => {
  const fromFile = await s2c.compile({ shapes: [SHAPES], dialect: 'neo4j', nodeKey: 'id' });
  assert.equal(typeof fromFile.manifestJson, 'string');
  assert.match(fromFile.cypher, /\/\/ name: PersonShape\.status\.in/);
  assert.ok(fromFile.manifest.rules.some((rule) => rule.ruleId === 'ex:PersonShape/ex:status/sh:in'));

  const document = { name: 'person.ttl', text: fs.readFileSync(SHAPES, 'utf8') };
  const fromDocument = await s2c.compile({ shapes: [document], dialect: 'neo4j', nodeKey: 'id', baseDir: tempDir() });
  assert.equal(fromDocument.manifest.inputs[0].path, 'person.ttl');
  assert.equal(fromDocument.cypher, fromFile.cypher);

  const sync = s2c.compileSync({ shapes: [SHAPES], dialect: 'neo4j', nodeKey: 'id' });
  assert.equal(sync.manifestJson, fromFile.manifestJson);

  await assert.rejects(s2c.compile({ shapes: [], dialect: 'neo4j' }), s2c.CompileError);
  await assert.rejects(s2c.compile({ shapes: [], dialect: 'neo4j' }), /no shapes/);
});

// @lat: [[tests#Node Binding#Output Parity With The CLI]]
test('output matches the CLI byte for byte', { skip: !cli && 'CLI not built' }, async () => {
  const out = tempDir();
  const result = runCli(['compile', path.basename(SHAPES), '--dialect', 'neo4j', '-o', out], CORE_FIXTURES);
  assert.equal(result.status, 0, result.stderr);
  const compilation = await s2c.compile({ shapes: [SHAPES], dialect: 'neo4j', baseDir: CORE_FIXTURES });
  assert.equal(compilation.manifestJson, fs.readFileSync(path.join(out, 'manifest.json'), 'utf8'));
  assert.equal(compilation.cypher, fs.readFileSync(path.join(out, 'queries.cypher'), 'utf8'));
});

test('option values and names are checked', async () => {
  await assert.rejects(s2c.compile({ shapes: [SHAPES], dialect: 'postgres' }), {
    name: 'TypeError',
    message: /dialect must be one of neo4j, ladybug/,
  });
  await assert.rejects(s2c.compile({ shapes: [SHAPES], dialect: 'neo4j', neo4jLabels: 'all' }), TypeError);
  await assert.rejects(s2c.compile({ shapes: [SHAPES], dialect: 'neo4j', nodekey: 'id' }), /unknown compile option: nodekey/);
  assert.throws(() => s2c.compileSync({ shapes: [{ name: 'a.ttl', text: '', format: 'json' }], dialect: 'neo4j' }), /format must be one of/);
});

test('writes outputs into a new directory', async () => {
  const compilation = await s2c.compile({ shapes: [SHAPES], dialect: 'neo4j' });
  const out = path.join(tempDir(), 'nested', 'out');
  compilation.write(out);
  assert.equal(fs.readFileSync(path.join(out, 'manifest.json'), 'utf8'), compilation.manifestJson);
  assert.equal(fs.readFileSync(path.join(out, 'queries.cypher'), 'utf8'), compilation.cypher);
});

// @lat: [[tests#Node Binding#Typed Errors]]
test('compile errors list every problem', async () => {
  const text =
    PREFIXES +
    'ex:A sh:targetClass ex:P ; sh:property [ sh:path ex:n ; sh:minCount "two" ] .\n' +
    '\n\n' +
    'ex:B sh:targetClass ex:P ; sh:property [ sh:path ex:m ; sh:maxCount "three" ] .\n';
  const dir = tempDir();
  const error = await s2c.compile({ shapes: [{ name: 'broken.ttl', text }], dialect: 'neo4j', baseDir: dir }).then(
    () => assert.fail('expected a CompileError'),
    (e) => e,
  );
  assert.ok(error instanceof s2c.CompileError);
  assert.equal(error.errors.length, 2, error.errors.join('\n'));
  assert.ok(error.errors.some((e) => e.includes('broken.ttl:4')));
  assert.ok(error.errors.some((e) => e.includes('broken.ttl:7')));
  assert.deepEqual(fs.readdirSync(dir), []);

  for (const Kind of [s2c.CompileError, s2c.ManifestError, s2c.DatabaseConnectionError, s2c.BackendUnavailableError, s2c.DatabaseClosedError]) {
    assert.ok(Kind.prototype instanceof s2c.Shacl2CypherError, Kind.name);
    assert.ok(Kind.prototype instanceof Error);
  }
});

test('static diagnostics are returned, not thrown', async () => {
  const schema = { nodeTypes: [{ name: 'Person', properties: [{ name: 'status', type: 'STRING' }] }], relTypes: [] };
  const compilation = await s2c.compile({ shapes: [SHAPES], dialect: 'neo4j', schema });
  assert.ok(compilation.staticDiagnostics.length > 0);
  assert.deepEqual(Object.keys(compilation.staticDiagnostics[0]).sort(), ['code', 'message', 'source']);
});

test('schema text and objects compile alike', async () => {
  const schema = {
    nodeTypes: [{
      name: 'Person',
      properties: [
        { name: 'id', type: 'STRING' }, { name: 'level', type: 'INT64' },
        { name: 'status', type: 'STRING' }, { name: 'tags', type: 'LIST<STRING>' },
      ],
    }],
    relTypes: [],
  };
  const fromObject = await s2c.compile({ shapes: [SHAPES], dialect: 'ladybug', schema });
  const fromText = await s2c.compile({ shapes: [SHAPES], dialect: 'ladybug', schema: `${JSON.stringify(schema, null, 2)}\n` });
  assert.equal(fromObject.cypher, fromText.cypher);
  assert.deepEqual(fromObject.manifest.rules, fromText.manifest.rules);
});

test('manifests load and other versions are rejected', async () => {
  const { manifest } = await s2c.compile({ shapes: [SHAPES], dialect: 'neo4j' });
  assert.deepEqual(s2c.loadManifest(manifest), manifest);
  assert.throws(() => s2c.loadManifest({ ...manifest, schemaVersion: 99 }), (e) => e instanceof s2c.ManifestError && /99/.test(e.message));
});

test('the version matches the manifest', async () => {
  const { manifest } = await s2c.compile({ shapes: [SHAPES], dialect: 'neo4j' });
  assert.equal(manifest.compilerVersion, s2c.version);
});

// @lat: [[tests#Node Binding#Backend Availability]]
test('compile-only builds open no database', { skip: s2c.availableBackends().length > 0 && 'backends built in' }, async () => {
  await assert.rejects(s2c.Ladybug.open(path.join(tempDir(), 'graph.lbug')), s2c.BackendUnavailableError);
});

test('unsupported platforms are named', () => {
  assert.throws(() => s2c._targetFor('win32', 'x64'), /no native addon for win32-x64; supported platforms: linux-x64 \(glibc\)/);
  assert.throws(() => s2c._targetFor('linux', 'x64', true), /linux-x64 \(musl\)/);
  assert.equal(s2c._targetFor('darwin', 'arm64'), 'darwin-arm64');
});

// @lat: [[tests#Node Binding#Remote Imports]]
test('remote imports use the CLI limits', async (t) => {
  const server = http.createServer((request, response) => {
    const body = Buffer.alloc(16 * 1024 * 1024 + 1, 35);
    response.writeHead(200, { 'Content-Length': body.length });
    response.end(body);
  });
  await new Promise((resolve) => server.listen(0, '127.0.0.1', resolve));
  t.after(() => server.close());
  const iri = `http://127.0.0.1:${server.address().port}/big.ttl`;
  const shapes = [{ name: 'main.ttl', text: `@prefix owl: <http://www.w3.org/2002/07/owl#> .\n<> owl:imports <${iri}> .\n` }];
  const baseDir = tempDir();
  await assert.rejects(s2c.compile({ shapes, dialect: 'neo4j', baseDir }), /remote imports are disabled/);
  if (!s2c.availableBackends().includes('ladybug')) return;
  const error = await s2c.compile({ shapes, dialect: 'neo4j', baseDir, allowRemoteImports: true }).catch((e) => e);
  assert.ok(error instanceof s2c.CompileError);
  assert.ok(error.message.includes(iri), error.message);
  assert.ok(error.message.includes('larger than 16777216 bytes'), error.message);
});
