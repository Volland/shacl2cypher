'use strict';

// `index.d.ts` declares exactly the keys the Rust side serializes.

const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const { test } = require('node:test');

const s2c = require('..');
const { SHAPES, fixtureTool, ladybugDb } = require('./helpers');

let ts;
try {
  ts = require('typescript');
} catch {
  ts = null;
}

/** Interface name -> { required, optional, types: key -> referenced interface } */
function declaredInterfaces() {
  const file = path.join(__dirname, '..', 'index.d.ts');
  const source = ts.createSourceFile(file, fs.readFileSync(file, 'utf8'), ts.ScriptTarget.Latest, true);
  const interfaces = {};
  const referenced = (type) => {
    if (ts.isArrayTypeNode(type)) return referenced(type.elementType);
    if (ts.isUnionTypeNode(type)) return type.types.map(referenced).find(Boolean) || null;
    if (ts.isTypeReferenceNode(type)) return type.typeName.getText(source);
    return null;
  };
  source.forEachChild((node) => {
    if (!ts.isInterfaceDeclaration(node)) return;
    const shape = { required: new Set(), optional: new Set(), types: {} };
    for (const member of node.members) {
      const name = member.name.getText(source);
      (member.questionToken ? shape.optional : shape.required).add(name);
      shape.types[name] = referenced(member.type);
    }
    interfaces[node.name.text] = shape;
  });
  return interfaces;
}

function check(value, name, interfaces, where) {
  const shape = interfaces[name];
  if (!shape || value === null || typeof value !== 'object') return;
  if (Array.isArray(value)) {
    value.forEach((item, index) => check(item, name, interfaces, `${where}[${index}]`));
    return;
  }
  const keys = new Set(Object.keys(value));
  for (const key of shape.required) assert.ok(keys.has(key), `${where}: missing ${key}`);
  for (const key of keys) {
    assert.ok(shape.required.has(key) || shape.optional.has(key), `${where}: undeclared ${key}`);
    check(value[key], shape.types[key], interfaces, `${where}.${key}`);
  }
}

// @lat: [[tests#Node Binding#Published Types]]
test('manifest, report and schema keys match index.d.ts', { skip: !ts && 'typescript not installed' }, async (t) => {
  const interfaces = declaredInterfaces();
  const { manifest } = await s2c.compile({ shapes: [SHAPES], dialect: 'neo4j', nodeKey: 'id' });
  check(manifest, 'Manifest', interfaces, 'manifest');

  if (!s2c.availableBackends().includes('ladybug') || !fixtureTool) {
    t.diagnostic('report and schema checks need the ladybug backend and s2c-fixture');
    return;
  }
  const db = await s2c.Ladybug.open(ladybugDb());
  try {
    check(await db.validate({ shapes: [SHAPES], nodeKey: 'id' }), 'Report', interfaces, 'report');
    check(await db.schema(), 'SchemaSnapshot', interfaces, 'schema');
  } finally {
    await db.close();
  }
  assert.ok(interfaces.ManifestRule.optional.has('statusReason'));
  assert.ok(interfaces.RuleResult.optional.has('statusReason'));
});
