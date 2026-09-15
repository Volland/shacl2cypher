// Type-checked with `tsc --strict --noEmit`: a consumer of the public API.

import {
  compile,
  CompileError,
  exitCode,
  Ladybug,
  renderReport,
  type ManifestRule,
  type Report,
  type SchemaSnapshot,
} from 'shacl2cypher';

export async function compilePersonShapes(directory: string): Promise<ManifestRule[]> {
  try {
    const compilation = await compile({
      shapes: [`${directory}/shapes.ttl`, { name: 'person.ttl', text: '@prefix sh: <http://www.w3.org/ns/shacl#> .\n' }],
      dialect: 'neo4j',
      nodeKey: 'id',
    });
    compilation.write(`${directory}/out`);
    return compilation.manifest.rules;
  } catch (error) {
    if (error instanceof CompileError) {
      for (const message of error.errors) console.error(message);
      return [];
    }
    throw error;
  }
}

export function firstViolationCount(report: Report): number | null {
  const [first] = report.rules;
  return first ? first.violationCount : null;
}

export async function validate(database: string, shapes: string): Promise<0 | 1 | 3> {
  const db = await Ladybug.open(database);
  try {
    const report = await db.validate({ shapes: [shapes], nodeKey: 'id', limit: 10, timeoutMs: 2500 });
    console.log(renderReport(report, 'sarif'), firstViolationCount(report));
    const snapshot: SchemaSnapshot = await db.schema();
    console.log(snapshot.nodeTypes.map((node) => node.name));
    const compilation = await compile({ shapes: [shapes], dialect: db.dialect, schema: snapshot });
    const fromManifest = await db.validate({ manifest: compilation, limit: null });
    return exitCode(fromManifest, 'warning');
  } finally {
    await db.close();
  }
}
