import { createRequire } from 'node:module';

const require = createRequire(import.meta.url);
const binding = require('./index.js');

export const {
  compile,
  compileSync,
  loadManifest,
  renderReport,
  exitCode,
  availableBackends,
  version,
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
} = binding;

export default binding;
