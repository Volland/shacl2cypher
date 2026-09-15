'use strict';

// Keeps the npm packages on the workspace version.
//   node scripts/version.js check <version>   fails listing every mismatch
//   node scripts/version.js set <version>     writes the version everywhere

const fs = require('node:fs');
const path = require('node:path');

const ROOT = path.resolve(__dirname, '..');
const PLATFORMS = fs.readdirSync(path.join(ROOT, 'npm'));

function packageFiles() {
  return [
    path.join(ROOT, 'package.json'),
    ...PLATFORMS.map((platform) => path.join(ROOT, 'npm', platform, 'package.json')),
  ];
}

/** `[file, field, actual]` for every version field. */
function versionFields() {
  const fields = [];
  for (const file of packageFiles()) {
    const json = JSON.parse(fs.readFileSync(file, 'utf8'));
    fields.push([file, 'version', json.version]);
    for (const [name, version] of Object.entries(json.optionalDependencies || {})) {
      fields.push([file, `optionalDependencies.${name}`, version]);
    }
  }
  return fields;
}

function check(version) {
  const mismatches = versionFields().filter(([, , actual]) => actual !== version);
  for (const [file, field, actual] of mismatches) {
    console.error(`${path.relative(ROOT, file)}: ${field} is ${actual}, expected ${version}`);
  }
  if (mismatches.length > 0) process.exit(1);
  console.log(`npm packages are at ${version}`);
}

function set(version) {
  for (const file of packageFiles()) {
    const json = JSON.parse(fs.readFileSync(file, 'utf8'));
    json.version = version;
    for (const name of Object.keys(json.optionalDependencies || {})) {
      json.optionalDependencies[name] = version;
    }
    fs.writeFileSync(file, `${JSON.stringify(json, null, 2)}\n`);
  }
  console.log(`set npm packages to ${version}`);
}

const [command, version] = process.argv.slice(2);
if (!/^\d+\.\d+\.\d+/.test(version || '') || !['check', 'set'].includes(command)) {
  console.error('usage: node scripts/version.js check|set <version>');
  process.exit(2);
}
(command === 'check' ? check : set)(version);
