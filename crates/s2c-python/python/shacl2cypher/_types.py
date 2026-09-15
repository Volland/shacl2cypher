"""Shapes of the manifest, report and schema snapshot data.

Keys match `manifest.json`, `--format json` reports and `schema dump` output, so
they are camelCase.
"""

from typing import Any, Dict, List, Literal, Optional, TypedDict

Dialect = Literal["neo4j", "ladybug", "falkordb"]
RdfFormat = Literal["turtle", "ntriples", "trig"]
Neo4jLabels = Literal["explicit", "inherited"]
ReportFormat = Literal["table", "json", "junit", "sarif"]
FailOn = Literal["violation", "warning", "info"]
RuleStatus = Literal[
    "passed", "failed", "timeout", "error", "skipped", "guaranteed-by-schema"
]


class SourceRef(TypedDict):
    file: str
    line: int


class Queries(TypedDict):
    detail: str
    summary: str


class ManifestInput(TypedDict):
    path: str
    role: str
    sha256: str


class ManifestOptions(TypedDict):
    nodeKey: Optional[str]
    neo4jLabels: str
    strict: bool
    lenient: bool
    verbose: bool
    maxPathDepth: int
    allowRemoteImports: bool
    failOnSchemaMismatch: bool


class _ManifestRuleOptional(TypedDict, total=False):
    statusReason: str
    pathDepthCap: int


class ManifestRule(_ManifestRuleOptional):
    name: str
    ruleId: str
    fingerprint: str
    shape: str
    path: Optional[str]
    constraint: str
    severity: str
    status: str
    costClass: str
    source: SourceRef
    message: str
    queries: Optional[Queries]


class ManifestDiagnostic(TypedDict):
    code: str
    message: str
    source: SourceRef


class RecommendedIndex(TypedDict):
    label: str
    property: str
    reason: str


class Manifest(TypedDict):
    schemaVersion: int
    compilerVersion: str
    dialect: str
    inputs: List[ManifestInput]
    schemaSnapshotHash: Optional[str]
    options: ManifestOptions
    rules: List[ManifestRule]
    staticDiagnostics: List[ManifestDiagnostic]
    recommendedIndexes: List[RecommendedIndex]


class Summary(TypedDict):
    rules: int
    passed: int
    failed: int
    timeouts: int
    errors: int
    skipped: int
    guaranteedBySchema: int
    violations: int
    durationMs: int


class _RuleResultOptional(TypedDict, total=False):
    statusReason: str


class RuleResult(_RuleResultOptional):
    name: str
    ruleId: str
    shape: str
    path: Optional[str]
    constraint: str
    severity: str
    status: RuleStatus
    violationCount: Optional[int]
    durationMs: int
    source: SourceRef
    message: str
    violations: List[Dict[str, Any]]


class ReportData(TypedDict):
    tool: str
    toolVersion: str
    dialect: str
    conforms: bool
    complete: bool
    summary: Summary
    rules: List[RuleResult]


class PropertyDef(TypedDict):
    name: str
    type: str


class NodeType(TypedDict):
    name: str
    properties: List[PropertyDef]


# `from` is a keyword, so this one uses the functional syntax.
Endpoint = TypedDict("Endpoint", {"from": str, "to": str})


class RelType(TypedDict):
    name: str
    endpoints: List[Endpoint]
    properties: List[PropertyDef]


class SchemaSnapshot(TypedDict):
    nodeTypes: List[NodeType]
    relTypes: List[RelType]
