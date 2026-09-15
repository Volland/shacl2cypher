'use strict';

// Shared paths and fixtures for the binding tests.

const { execFileSync, spawnSync } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const ROOT = path.resolve(__dirname, '..', '..', '..');
const CORE_FIXTURES = path.join(ROOT, 'tests', 'conformance', 'core');
const FIXTURE = path.join(CORE_FIXTURES, 'in-has-value.yaml');
const SHAPES = path.join(CORE_FIXTURES, 'in-has-value.ttl');

function binary(variable, name) {
  const file = process.env[variable] || path.join(ROOT, 'target', 'debug', name);
  return fs.existsSync(file) ? file : null;
}

/** `shacl2cypher` built with the ladybug feature, or null. */
const cli = binary('S2C_CLI', 'shacl2cypher');
/** `s2c-fixture` built with the ladybug feature, or null. */
const fixtureTool = binary('S2C_FIXTURE', 's2c-fixture');

function tempDir() {
  return fs.mkdtempSync(path.join(os.tmpdir(), 's2c-node-'));
}

/** A LadybugDB file holding the `in-has-value` fixture graph. */
function ladybugDb() {
  const file = path.join(tempDir(), 'graph.lbug');
  execFileSync(fixtureTool, ['ladybug-db', FIXTURE, file]);
  return file;
}

function runCli(args, cwd) {
  return spawnSync(cli, args.map(String), { cwd, encoding: 'utf8' });
}

/** Replaces every `durationMs` with 0, recursively. */
function untimed(value) {
  if (Array.isArray(value)) return value.map(untimed);
  if (value && typeof value === 'object') {
    return Object.fromEntries(
      Object.entries(value).map(([key, item]) => [key, key === 'durationMs' ? 0 : untimed(item)]),
    );
  }
  return value;
}

/** A shapes document with `count` string-length rules on `ex:status`. */
function manyRules(count) {
  const properties = Array.from(
    { length: count },
    (_, n) => `  [ sh:path ex:status ; sh:minLength ${n} ; s2c:name "rule${n}" ]`,
  ).join(' ,\n');
  return {
    name: 'many.ttl',
    text:
      '@prefix sh: <http://www.w3.org/ns/shacl#> .\n' +
      '@prefix ex: <http://example.org/> .\n' +
      '@prefix s2c: <https://w3id.org/shacl2cypher#> .\n' +
      `ex:PersonShape sh:targetClass ex:Person ;\n  sh:property\n${properties} .\n`,
  };
}

module.exports = {
  ROOT, CORE_FIXTURES, FIXTURE, SHAPES, cli, fixtureTool, tempDir, ladybugDb, runCli, untimed, manyRules,
};
