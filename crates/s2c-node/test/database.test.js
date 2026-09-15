'use strict';

// Database handles through the Node binding: validate, schema dump, reports, the event loop.

const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { describe, test } = require('node:test');

const s2c = require('..');
const { CORE_FIXTURES, SHAPES, cli, fixtureTool, tempDir, ladybugDb, runCli, untimed, manyRules } = require('./helpers');

const skip =
  (!s2c.availableBackends().includes('ladybug') && 'built without the ladybug backend') ||
  (!fixtureTool && 's2c-fixture not built');

describe('LadybugDB', { skip }, () => {
  // @lat: [[tests#Node Binding#Database Handles]]
  test('handles open, close and reject use after close', async () => {
    const missing = path.join(tempDir(), 'missing.lbug');
    await assert.rejects(s2c.Ladybug.open(missing), s2c.DatabaseConnectionError);
    assert.equal(fs.existsSync(missing), false);

    const db = await s2c.Ladybug.open(ladybugDb());
    assert.equal(db.dialect, 'ladybug');
    assert.equal(db.closed, false);
    await db.close();
    await db.close();
    assert.equal(db.closed, true);
    await assert.rejects(db.validate({ shapes: [SHAPES], nodeKey: 'id' }), s2c.DatabaseClosedError);
    await assert.rejects(db.schema(), s2c.DatabaseClosedError);
    assert.throws(() => new s2c.Database(), TypeError);
  });

  // @lat: [[tests#Node Binding#Validate]]
  test('validates shapes and manifests', async () => {
    const db = await s2c.Ladybug.open(ladybugDb());
    try {
      const report = await db.validate({ shapes: [SHAPES], nodeKey: 'id' });
      assert.equal(report.conforms, false);
      assert.equal(report.complete, true);
      const hasValue = report.rules.find((rule) => rule.name === 'PersonShape.tags.hasValue');
      assert.equal(hasValue.status, 'failed');
      assert.equal(hasValue.violationCount, 2);
      assert.equal(hasValue.violations.length, 2);

      const manifest = await s2c.compile({ shapes: [SHAPES], dialect: 'ladybug', nodeKey: 'id', schema: await db.schema() });
      const fromManifest = await db.validate({ manifest });
      assert.equal(fromManifest.summary.violations, report.summary.violations);

      const limited = await db.validate({ shapes: [SHAPES], nodeKey: 'id', limit: 1 });
      assert.equal(limited.rules.find((rule) => rule.name === 'PersonShape.tags.hasValue').violations.length, 1);

      const neo4jManifest = await s2c.compile({ shapes: [SHAPES], dialect: 'neo4j' });
      await assert.rejects(db.validate({ manifest: neo4jManifest }), (e) => e instanceof s2c.ManifestError && /neo4j.*ladybug/.test(e.message));
      await assert.rejects(db.validate({ shapes: [SHAPES], timeoutMs: -5 }), { name: 'RangeError', message: /timeoutMs must be a positive/ });
      await assert.rejects(db.validate({}), /either shapes or manifest/);
    } finally {
      await db.close();
    }
  });

  // @lat: [[tests#Node Binding#Reports And Exit Codes]]
  test('reports match the CLI and exit codes follow failOn', { skip: !cli && 'CLI not built' }, async () => {
    const file = ladybugDb();
    const db = await s2c.Ladybug.open(file);
    const report = await db.validate({ shapes: [SHAPES], nodeKey: 'id', baseDir: CORE_FIXTURES });
    await db.close();
    const out = path.join(tempDir(), 'report.json');
    const result = runCli(['validate', path.basename(SHAPES), '--ladybug', file, '--node-key', 'id', '--format', 'json', '-o', out], CORE_FIXTURES);
    assert.equal(result.status, 1, result.stderr);
    assert.deepEqual(untimed(report), untimed(JSON.parse(fs.readFileSync(out, 'utf8'))));

    for (const format of ['table', 'json', 'junit', 'sarif']) {
      assert.ok(s2c.renderReport(report, format).length > 0, format);
    }
    assert.deepEqual(JSON.parse(s2c.renderReport(report, 'json')), report);
    assert.throws(() => s2c.renderReport(report, 'html'), TypeError);
    assert.equal(s2c.exitCode(report), 1);
    const warnings = { ...report, rules: report.rules.map((rule) => ({ ...rule, severity: 'Warning' })) };
    assert.equal(s2c.exitCode(warnings, 'violation'), 0);
    assert.equal(s2c.exitCode(warnings, 'warning'), 1);
  });

  // @lat: [[tests#Node Binding#Schema Dump]]
  test('schema dumps round-trip into compile', { skip: !cli && 'CLI not built' }, async () => {
    const file = ladybugDb();
    const db = await s2c.Ladybug.open(file);
    const snapshot = await db.schema();
    await db.close();
    const out = path.join(tempDir(), 'schema.json');
    const result = runCli(['schema', 'dump', '--ladybug', file, '-o', out], tempDir());
    assert.equal(result.status, 0, result.stderr);
    const fromObject = await s2c.compile({ shapes: [SHAPES], dialect: 'ladybug', schema: snapshot });
    const fromFile = await s2c.compile({ shapes: [SHAPES], dialect: 'ladybug', schema: fs.readFileSync(out, 'utf8') });
    assert.equal(fromObject.manifestJson, fromFile.manifestJson);
  });

  // @lat: [[tests#Node Binding#Non-Blocking Execution]]
  test('validation does not block the event loop', async () => {
    const db = await s2c.Ladybug.open(ladybugDb());
    try {
      let timerFired = false;
      const started = Date.now();
      const validation = db.validate({ shapes: [manyRules(300)], nodeKey: 'id' }).then(() => timerFired);
      setTimeout(() => { timerFired = true; }, 10);
      const firedFirst = await validation;
      const elapsed = Date.now() - started;
      assert.ok(elapsed > 50, `validation too fast to observe (${elapsed} ms)`);
      assert.equal(firedFirst, true);
    } finally {
      await db.close();
    }
  });

  test('concurrent calls on one handle complete', async () => {
    const db = await s2c.Ladybug.open(ladybugDb());
    try {
      const reports = await Promise.all([
        db.validate({ shapes: [SHAPES], nodeKey: 'id' }),
        db.validate({ shapes: [SHAPES], nodeKey: 'id' }),
        db.schema(),
      ]);
      assert.ok(reports[0].complete && reports[1].complete);
    } finally {
      await db.close();
    }
  });
});

describe('Neo4j', { skip: !s2c.availableBackends().includes('neo4j') && 'built without the neo4j backend' }, () => {
  test('unreachable servers are named', async () => {
    await assert.rejects(s2c.Neo4j.connect({ uri: 'bolt://127.0.0.1:1', password: 'nope' }), (e) => e instanceof s2c.DatabaseConnectionError && /127\.0\.0\.1:1/.test(e.message));
  });

  test('validates and dumps on Neo4j', { skip: !process.env.S2C_NEO4J_URI && 'set S2C_NEO4J_URI' }, async () => {
    const db = await s2c.Neo4j.connect({ uri: process.env.S2C_NEO4J_URI, password: process.env.S2C_NEO4J_PASSWORD || '' });
    try {
      assert.equal(db.dialect, 'neo4j');
      assert.equal((await db.validate({ shapes: [SHAPES], nodeKey: 'id' })).complete, true);
      assert.ok(Array.isArray((await db.schema()).nodeTypes));
    } finally {
      await db.close();
    }
  });
});
