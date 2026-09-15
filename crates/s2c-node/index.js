'use strict';

// Public API of the `shacl2cypher` npm package, over the native addon in `src/lib.rs`.

const fs = require('node:fs');
const path = require('node:path');
const { fileURLToPath } = require('node:url');

const TARGETS = {
  'linux-x64': 'linux-x64-gnu',
  'linux-arm64': 'linux-arm64-gnu',
  'darwin-arm64': 'darwin-arm64',
};
const SUPPORTED = 'linux-x64 (glibc), linux-arm64 (glibc), darwin-arm64';

function isMusl() {
  if (process.platform !== 'linux') return false;
  const report = process.report && process.report.getReport();
  return Boolean(report && !report.header.glibcVersionRuntime);
}

/** The platform package suffix for a platform, or an error naming the platform. */
function targetFor(platform, arch, musl = false) {
  const target = TARGETS[`${platform}-${arch}`];
  if (!target || musl) {
    const name = `${platform}-${arch}${musl ? ' (musl)' : ''}`;
    throw new Error(`shacl2cypher has no native addon for ${name}; supported platforms: ${SUPPORTED}`);
  }
  return target;
}

function loadNative() {
  const target = targetFor(process.platform, process.arch, isMusl());
  const local = path.join(__dirname, `shacl2cypher.${target}.node`);
  if (fs.existsSync(local)) return require(local);
  return require(`shacl2cypher-${target}`);
}

const native = loadNative();

class Shacl2CypherError extends Error {
  constructor(message) {
    super(message);
    this.name = new.target.name;
  }
}
class CompileError extends Shacl2CypherError {
  constructor(message, errors) {
    super(message);
    this.errors = errors;
  }
}
class ManifestError extends Shacl2CypherError {}
class DatabaseConnectionError extends Shacl2CypherError {}
class BackendUnavailableError extends Shacl2CypherError {}
class DatabaseClosedError extends Shacl2CypherError {}

const ERRORS = {
  manifest: ManifestError,
  connection: DatabaseConnectionError,
  'backend-unavailable': BackendUnavailableError,
  closed: DatabaseClosedError,
};

function rethrow(error) {
  if (!(error instanceof Error) || !error.message.startsWith('{')) throw error;
  let payload;
  try {
    payload = JSON.parse(error.message);
  } catch {
    throw error;
  }
  if (!payload || typeof payload.s2c !== 'string') throw error;
  if (payload.s2c === 'type') throw new TypeError(payload.message);
  if (payload.s2c === 'range') throw new RangeError(payload.message);
  if (payload.s2c === 'compile') throw new CompileError(payload.message, payload.errors);
  throw new (ERRORS[payload.s2c] || Shacl2CypherError)(payload.message);
}

function call(fn) {
  try {
    return fn();
  } catch (error) {
    return rethrow(error);
  }
}

async function callAsync(fn) {
  try {
    return await fn();
  } catch (error) {
    return rethrow(error);
  }
}

const SETTINGS = [
  'schema', 'ontologies', 'nodeKey', 'neo4jLabels', 'strict', 'lenient', 'verbose',
  'maxPathDepth', 'allowRemoteImports', 'failOnSchemaMismatch', 'baseDir',
];
const COMPILE_KEYS = new Set(['shapes', 'dialect', ...SETTINGS]);
const VALIDATE_KEYS = new Set(['shapes', 'manifest', 'limit', 'sampleSize', 'timeoutMs', ...SETTINGS]);

function checkKeys(options, allowed, what) {
  if (options === null || typeof options !== 'object') {
    throw new TypeError(`${what} options must be an object`);
  }
  for (const key of Object.keys(options)) {
    if (!allowed.has(key)) throw new TypeError(`unknown ${what} option: ${key}`);
  }
}

function sources(list, name) {
  if (list === undefined) return [];
  if (!Array.isArray(list)) throw new TypeError(`${name} must be an array`);
  return list.map((item) => {
    if (typeof item === 'string') return item;
    if (item instanceof URL) return fileURLToPath(item);
    if (item && typeof item.name === 'string' && typeof item.text === 'string') {
      return { name: item.name, text: item.text, format: item.format };
    }
    throw new TypeError(`${name} entries must be paths or { name, text } documents`);
  });
}

function settings(options) {
  const { schema, ontologies } = options;
  if (schema !== undefined && schema !== null && typeof schema !== 'string' && typeof schema !== 'object') {
    throw new TypeError('schema must be snapshot JSON text or a parsed snapshot');
  }
  return { ...options, ontologies: sources(ontologies, 'ontologies') };
}

class Compilation {
  constructor(result) {
    this.manifestJson = result.manifestJson;
    this.cypher = result.cypher;
    this.manifest = JSON.parse(result.manifestJson);
  }

  get staticDiagnostics() {
    return this.manifest.staticDiagnostics;
  }

  /** Writes `manifest.json` and `queries.cypher` into `directory`, creating it. */
  write(directory) {
    fs.mkdirSync(directory, { recursive: true });
    fs.writeFileSync(path.join(directory, 'manifest.json'), this.manifestJson);
    fs.writeFileSync(path.join(directory, 'queries.cypher'), this.cypher);
  }
}

function compileInput(options) {
  checkKeys(options, COMPILE_KEYS, 'compile');
  return JSON.stringify({ ...settings(options), shapes: sources(options.shapes, 'shapes') });
}

async function compile(options) {
  const input = compileInput(options);
  return new Compilation(JSON.parse(await callAsync(() => native.compile(input))));
}

function compileSync(options) {
  const input = compileInput(options);
  return new Compilation(JSON.parse(call(() => native.compileSync(input))));
}

function manifestText(manifest) {
  if (manifest instanceof Compilation) return manifest.manifestJson;
  return typeof manifest === 'string' ? manifest : JSON.stringify(manifest);
}

function loadManifest(manifest) {
  return JSON.parse(call(() => native.loadManifest(manifestText(manifest))));
}

function renderReport(report, format) {
  return call(() => native.renderReport(JSON.stringify(report), format));
}

function exitCode(report, failOn = 'violation') {
  return call(() => native.exitCode(JSON.stringify(report), failOn));
}

const CREATE = Symbol('create');

class Database {
  #native;

  constructor(token, handle) {
    if (token !== CREATE) throw new TypeError('use Neo4j.connect() or Ladybug.open()');
    this.#native = handle;
  }

  get dialect() {
    return this.#native.dialect;
  }

  get closed() {
    return this.#native.closed;
  }

  /** Validates shapes (compiled for this database) or a manifest. */
  async validate(options) {
    checkKeys(options, VALIDATE_KEYS, 'validate');
    const { manifest, shapes, limit, sampleSize, timeoutMs, ...rest } = options;
    if ((manifest === undefined) === (shapes === undefined)) {
      throw new TypeError('pass either shapes or manifest');
    }
    if (limit !== undefined && limit !== null && !(Number.isInteger(limit) && limit >= 0)) {
      throw new TypeError('limit must be a non-negative integer or null');
    }
    if (sampleSize !== undefined && !(Number.isInteger(sampleSize) && sampleSize >= 0)) {
      throw new TypeError('sampleSize must be a non-negative integer');
    }
    if (timeoutMs !== undefined && typeof timeoutMs !== 'number') {
      throw new TypeError('timeoutMs must be a number');
    }
    const input = { limit: limit === null ? 0 : limit, sampleSize, timeoutMs };
    if (manifest !== undefined) {
      input.manifest = manifestText(manifest);
    } else {
      Object.assign(input, settings(rest), { shapes: sources(shapes, 'shapes') });
    }
    const text = JSON.stringify(input);
    return JSON.parse(await callAsync(() => this.#native.validate(text)));
  }

  /** The schema snapshot, usable as the `schema` compile option. */
  async schema() {
    return JSON.parse(await callAsync(() => this.#native.schema()));
  }

  /** Waits for running calls, then closes the database. */
  async close() {
    await callAsync(() => this.#native.close());
  }
}

class Neo4j {
  constructor() {
    throw new TypeError('use Neo4j.connect()');
  }

  /** Connects over Bolt and checks the connection. */
  static async connect(options) {
    if (!options || typeof options.uri !== 'string') throw new TypeError('uri must be a string');
    const { uri, user = 'neo4j', password = '', database } = options;
    const input = JSON.stringify({ uri, user, password, database });
    return new Database(CREATE, await callAsync(() => native.openNeo4j(input)));
  }
}

class Ladybug {
  constructor() {
    throw new TypeError('use Ladybug.open()');
  }

  /** Opens an existing LadybugDB database file read-only. */
  static async open(file) {
    if (typeof file !== 'string') throw new TypeError('path must be a string');
    return new Database(CREATE, await callAsync(() => native.openLadybug(file)));
  }
}

module.exports = {
  compile,
  compileSync,
  loadManifest,
  renderReport,
  exitCode,
  availableBackends: () => native.availableBackends(),
  version: native.version(),
  Compilation,
  Database,
  Neo4j,
  Ladybug,
  Shacl2CypherError,
  CompileError,
  ManifestError,
  DatabaseConnectionError,
  BackendUnavailableError,
  DatabaseClosedError,
  _targetFor: targetFor,
};
