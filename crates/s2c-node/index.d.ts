// Types of the `shacl2cypher` npm package. Data shapes use the camelCase keys of
// `manifest.json`, `--format json` reports and `schema dump` snapshots.

export type Dialect = 'neo4j' | 'ladybug' | 'falkordb';
export type RdfFormat = 'turtle' | 'ntriples' | 'trig';
export type Neo4jLabels = 'explicit' | 'inherited';
export type ReportFormat = 'table' | 'json' | 'junit' | 'sarif';
export type FailOn = 'violation' | 'warning' | 'info';
export type RuleStatus = 'passed' | 'failed' | 'timeout' | 'error' | 'skipped' | 'guaranteed-by-schema';

export interface SourceRef {
  file: string;
  line: number;
}

export interface Queries {
  detail: string;
  summary: string;
}

export interface ManifestInput {
  path: string;
  role: string;
  sha256: string;
}

export interface ManifestOptions {
  nodeKey: string | null;
  neo4jLabels: string;
  strict: boolean;
  lenient: boolean;
  verbose: boolean;
  maxPathDepth: number;
  allowRemoteImports: boolean;
  failOnSchemaMismatch: boolean;
}

export interface ManifestRule {
  name: string;
  ruleId: string;
  fingerprint: string;
  shape: string;
  path: string | null;
  constraint: string;
  severity: string;
  status: string;
  statusReason?: string;
  costClass: string;
  pathDepthCap?: number;
  source: SourceRef;
  message: string;
  queries: Queries | null;
}

export interface ManifestDiagnostic {
  code: string;
  message: string;
  source: SourceRef;
}

export interface RecommendedIndex {
  label: string;
  property: string;
  reason: string;
}

export interface Manifest {
  schemaVersion: number;
  compilerVersion: string;
  dialect: string;
  inputs: ManifestInput[];
  schemaSnapshotHash: string | null;
  options: ManifestOptions;
  rules: ManifestRule[];
  staticDiagnostics: ManifestDiagnostic[];
  recommendedIndexes: RecommendedIndex[];
}

export interface Summary {
  rules: number;
  passed: number;
  failed: number;
  timeouts: number;
  errors: number;
  skipped: number;
  guaranteedBySchema: number;
  violations: number;
  durationMs: number;
}

export interface RuleResult {
  name: string;
  ruleId: string;
  shape: string;
  path: string | null;
  constraint: string;
  severity: string;
  status: RuleStatus;
  statusReason?: string;
  violationCount: number | null;
  durationMs: number;
  source: SourceRef;
  message: string;
  violations: Array<Record<string, unknown>>;
}

export interface Report {
  tool: string;
  toolVersion: string;
  dialect: string;
  conforms: boolean;
  complete: boolean;
  summary: Summary;
  rules: RuleResult[];
}

export interface PropertyDef {
  name: string;
  type: string;
}

export interface NodeType {
  name: string;
  properties: PropertyDef[];
}

export interface Endpoint {
  from: string;
  to: string;
}

export interface RelType {
  name: string;
  endpoints: Endpoint[];
  properties: PropertyDef[];
}

export interface SchemaSnapshot {
  nodeTypes: NodeType[];
  relTypes: RelType[];
}

/** An in-memory document, loaded as if it were the file `name` in `baseDir`. */
export interface SourceDocument {
  name: string;
  text: string;
  format?: RdfFormat;
}

/** A file path, a `file:` URL, or an in-memory document. */
export type ShapesInput = string | URL | SourceDocument;

export interface CompileSettings {
  /** Snapshot JSON text or a parsed snapshot. */
  schema?: string | SchemaSnapshot;
  ontologies?: ShapesInput[];
  nodeKey?: string;
  neo4jLabels?: Neo4jLabels;
  strict?: boolean;
  lenient?: boolean;
  verbose?: boolean;
  maxPathDepth?: number;
  allowRemoteImports?: boolean;
  failOnSchemaMismatch?: boolean;
  /** Where documents are placed and what manifest paths are relative to; default `process.cwd()`. */
  baseDir?: string;
}

export interface CompileOptions extends CompileSettings {
  shapes: ShapesInput[];
  dialect: Dialect;
}

export declare class Compilation {
  private constructor();
  readonly manifest: Manifest;
  /** Identical to the `manifest.json` the CLI writes. */
  readonly manifestJson: string;
  /** Identical to the `queries.cypher` the CLI writes. */
  readonly cypher: string;
  readonly staticDiagnostics: ManifestDiagnostic[];
  /** Writes `manifest.json` and `queries.cypher` into `directory`, creating it. */
  write(directory: string): void;
}

/** Compiles on a worker thread; rejects with `CompileError`. */
export declare function compile(options: CompileOptions): Promise<Compilation>;
/** Compiles on the calling thread; throws `CompileError`. */
export declare function compileSync(options: CompileOptions): Compilation;
/** Parses and checks a manifest; throws `ManifestError` for unsupported versions. */
export declare function loadManifest(manifest: string | Manifest): Manifest;

export interface RunOptions {
  /** Violations listed per rule (default 100); `0` or `null` lists every one. */
  limit?: number | null;
  /** Focus samples per summary row (default 5). */
  sampleSize?: number;
  /** Per-query timeout in milliseconds. */
  timeoutMs?: number;
}

export type ValidateOptions =
  | (RunOptions & CompileSettings & { shapes: ShapesInput[]; manifest?: undefined })
  | (RunOptions & { manifest: string | Manifest | Compilation; shapes?: undefined });

export declare class Database {
  private constructor();
  readonly dialect: Dialect;
  readonly closed: boolean;
  /** Validates shapes (compiled for this database) or a manifest. */
  validate(options: ValidateOptions): Promise<Report>;
  /** The schema snapshot, usable as the `schema` compile option. */
  schema(): Promise<SchemaSnapshot>;
  /** Waits for running calls, then closes the database. */
  close(): Promise<void>;
}

export interface Neo4jConnectOptions {
  uri: string;
  user?: string;
  password?: string;
  database?: string;
}

export declare class Neo4j {
  private constructor();
  /** Connects over Bolt and checks the connection. */
  static connect(options: Neo4jConnectOptions): Promise<Database>;
}

export declare class Ladybug {
  private constructor();
  /** Opens an existing LadybugDB database file read-only. */
  static open(path: string): Promise<Database>;
}

/** Renders a report exactly as `shacl2cypher validate --format` does. */
export declare function renderReport(report: Report, format: ReportFormat): string;
/** The CLI's exit code for a report: 0, 1 (failing rules) or 3 (incomplete). */
export declare function exitCode(report: Report, failOn?: FailOn): 0 | 1 | 3;
export declare function availableBackends(): Dialect[];
export declare const version: string;

export declare class Shacl2CypherError extends Error {}
export declare class CompileError extends Shacl2CypherError {
  readonly errors: string[];
}
export declare class ManifestError extends Shacl2CypherError {}
export declare class DatabaseConnectionError extends Shacl2CypherError {}
export declare class BackendUnavailableError extends Shacl2CypherError {}
export declare class DatabaseClosedError extends Shacl2CypherError {}
