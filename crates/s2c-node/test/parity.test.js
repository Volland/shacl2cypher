'use strict';

// The shared cases in `tests/bindings/cases.json` produce the CLI's exact output.
// The Python suite compares the same cases with the same CLI, so Node and Python
// outputs also equal each other.

const assert = require('node:assert/strict');
const { execFileSync } = require('node:child_process');
const fs = require('node:fs');
const path = require('node:path');
const { describe, test } = require('node:test');

const s2c = require('..');
const { ROOT, cli, fixtureTool, tempDir, runCli } = require('./helpers');

const CASES = JSON.parse(fs.readFileSync(path.join(ROOT, 'tests', 'bindings', 'cases.json'), 'utf8'));
const FORMATS = ['table', 'json', 'junit', 'sarif'];

/** Removes per-run timings: rule durations, totals and JUnit times. */
function normalize(format, text) {
  if (format === 'json' || format === 'sarif') return text.replace(/"durationMs": \d+/g, '"durationMs": 0');
  if (format === 'junit') return text.replace(/time="[\d.]+"/g, 'time="0"');
  return text
    .split('\n')
    .map((line) => {
      const fields = line.trim().split(/\s+/);
      if (fields.length === 5 && /^\d+$/.test(fields[2]) && /^\d+$/.test(fields[3])) {
        fields[3] = '0';
        line = fields.join(' ');
      }
      return line.replace(/ in \d+ ms$/, ' in 0 ms');
    })
    .filter((line, index, lines) => !(index === lines.length - 1 && line === ''))
    .join('\n');
}

// @lat: [[tests#Node Binding#Shared Parity Cases]]
describe('shared cases compile like the CLI', { skip: !cli && 'CLI not built' }, () => {
  for (const testCase of CASES.compile) {
    test(testCase.name, async () => {
      const cwd = path.join(ROOT, testCase.cwd);
      const out = tempDir();
      const args = ['compile', ...testCase.shapes, '--dialect', testCase.dialect, '-o', out];
      if (testCase.nodeKey) args.push('--node-key', testCase.nodeKey);
      if (testCase.schema) args.push('--schema', testCase.schema);
      const result = runCli(args, cwd);
      assert.equal(result.status, 0, result.stderr);

      const compilation = await s2c.compile({
        shapes: testCase.shapes.map((shape) => path.join(cwd, shape)),
        dialect: testCase.dialect,
        nodeKey: testCase.nodeKey,
        schema: testCase.schema ? fs.readFileSync(path.join(cwd, testCase.schema), 'utf8') : undefined,
        baseDir: cwd,
      });
      assert.equal(compilation.manifestJson, fs.readFileSync(path.join(out, 'manifest.json'), 'utf8'));
      assert.equal(compilation.cypher, fs.readFileSync(path.join(out, 'queries.cypher'), 'utf8'));
    });
  }
});

const reportSkip =
  (!cli && 'CLI not built') ||
  (!fixtureTool && 's2c-fixture not built') ||
  (!s2c.availableBackends().includes('ladybug') && 'built without the ladybug backend');

// @lat: [[tests#Node Binding#Report Parity]]
describe('reports render every format like the CLI', { skip: reportSkip }, () => {
  for (const testCase of CASES.reports) {
    test(testCase.name, async () => {
      const database = path.join(tempDir(), 'graph.lbug');
      execFileSync(fixtureTool, ['ladybug-db', path.join(ROOT, testCase.fixture), database]);
      const cwd = path.join(ROOT, testCase.cwd);
      const db = await s2c.Ladybug.open(database);
      let report;
      try {
        report = await db.validate({
          shapes: testCase.shapes.map((shape) => path.join(cwd, shape)),
          nodeKey: testCase.nodeKey,
          baseDir: cwd,
        });
      } finally {
        await db.close();
      }
      for (const format of FORMATS) {
        const out = path.join(tempDir(), `report.${format}`);
        const result = runCli(
          ['validate', ...testCase.shapes, '--ladybug', database, '--node-key', testCase.nodeKey, '--format', format, '-o', out],
          cwd,
        );
        assert.ok([0, 1].includes(result.status), result.stderr);
        assert.equal(normalize(format, s2c.renderReport(report, format)), normalize(format, fs.readFileSync(out, 'utf8')), format);
      }
    });
  }
});
