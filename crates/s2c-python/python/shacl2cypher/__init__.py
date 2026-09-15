"""Compile SHACL shapes into Cypher diagnostic queries and validate Neo4j or LadybugDB graphs.

Manifest, report and schema dicts use the camelCase keys of their JSON files.
"""

from __future__ import annotations

import json
import os
from types import TracebackType
from typing import Any, Iterable, Mapping, NamedTuple, Optional, Type, Union

from . import _native
from ._native import (
    BackendUnavailableError,
    Compilation,
    CompileError,
    DatabaseClosedError,
    DatabaseConnectionError,
    ManifestError,
    Report,
    Shacl2CypherError,
    __version__,
    available_backends,
)
from ._types import (
    Dialect,
    Endpoint,
    FailOn,
    Manifest,
    ManifestDiagnostic,
    ManifestInput,
    ManifestOptions,
    ManifestRule,
    Neo4jLabels,
    NodeType,
    PropertyDef,
    Queries,
    RdfFormat,
    RecommendedIndex,
    RelType,
    ReportData,
    ReportFormat,
    RuleResult,
    RuleStatus,
    SchemaSnapshot,
    SourceRef,
    Summary,
)

__all__ = [
    "BackendUnavailableError",
    "Compilation",
    "CompileError",
    "Database",
    "DatabaseClosedError",
    "DatabaseConnectionError",
    "Dialect",
    "Endpoint",
    "FailOn",
    "Ladybug",
    "Manifest",
    "ManifestDiagnostic",
    "ManifestError",
    "ManifestInput",
    "ManifestOptions",
    "ManifestRule",
    "Neo4j",
    "Neo4jLabels",
    "NodeType",
    "PropertyDef",
    "Queries",
    "RdfFormat",
    "RecommendedIndex",
    "RelType",
    "Report",
    "ReportData",
    "ReportFormat",
    "RuleResult",
    "RuleStatus",
    "SchemaSnapshot",
    "Shacl2CypherError",
    "Source",
    "SourceRef",
    "Summary",
    "__version__",
    "available_backends",
    "compile",
    "load_manifest",
]


class Source(NamedTuple):
    """An in-memory shapes or ontology document.

    It is loaded as if it were the file ``name`` in the base directory, which
    decides its base IRI, relative imports and its path in the manifest.
    """

    name: str
    text: str
    format: Optional[RdfFormat] = None


PathLike = Union[str, "os.PathLike[str]"]
ShapesInput = Union[PathLike, Source]


def _sources(items: Union[ShapesInput, Iterable[ShapesInput]]) -> list[Union[str, Source]]:
    if isinstance(items, (str, os.PathLike, Source)):
        items = [items]
    sources: list[Union[str, Source]] = []
    for item in items:
        if isinstance(item, Source):
            sources.append(item)
        elif isinstance(item, (str, os.PathLike)):
            sources.append(os.fspath(item))
        else:
            raise TypeError(
                f"shapes and ontologies must be paths or Source documents, not {type(item).__name__}"
            )
    return sources


def _options(
    *,
    dialect: str,
    schema: Union[str, Mapping[str, Any], None],
    node_key: Optional[str],
    neo4j_labels: str,
    strict: bool,
    lenient: bool,
    verbose: bool,
    max_path_depth: int,
    allow_remote_imports: bool,
    fail_on_schema_mismatch: bool,
    base_dir: Optional[PathLike],
) -> dict[str, Any]:
    if schema is not None and not isinstance(schema, (str, Mapping)):
        raise TypeError("schema must be snapshot JSON text or a parsed snapshot mapping")
    return {
        "dialect": dialect,
        "schema": schema,
        "node_key": node_key,
        "neo4j_labels": neo4j_labels,
        "strict": strict,
        "lenient": lenient,
        "verbose": verbose,
        "max_path_depth": max_path_depth,
        "allow_remote_imports": allow_remote_imports,
        "fail_on_schema_mismatch": fail_on_schema_mismatch,
        "base_dir": None if base_dir is None else os.fspath(base_dir),
    }


def compile(
    shapes: Union[ShapesInput, Iterable[ShapesInput]],
    *,
    dialect: Dialect,
    schema: Union[str, Mapping[str, Any], None] = None,
    ontologies: Union[ShapesInput, Iterable[ShapesInput]] = (),
    node_key: Optional[str] = None,
    neo4j_labels: Neo4jLabels = "explicit",
    strict: bool = False,
    lenient: bool = False,
    verbose: bool = False,
    max_path_depth: int = 10,
    allow_remote_imports: bool = False,
    fail_on_schema_mismatch: bool = False,
    base_dir: Optional[PathLike] = None,
) -> Compilation:
    """Compile shapes into a manifest and a ``.cypher`` text, like ``shacl2cypher compile``.

    ``base_dir`` (default: the working directory) is where in-memory documents are
    placed and what manifest paths are relative to. Raises ``CompileError`` listing
    every problem.
    """
    options = _options(
        dialect=dialect,
        schema=schema,
        node_key=node_key,
        neo4j_labels=neo4j_labels,
        strict=strict,
        lenient=lenient,
        verbose=verbose,
        max_path_depth=max_path_depth,
        allow_remote_imports=allow_remote_imports,
        fail_on_schema_mismatch=fail_on_schema_mismatch,
        base_dir=base_dir,
    )
    return _native.compile(_sources(shapes), _sources(ontologies), options)


def _manifest_json(manifest: Union[Compilation, str, Mapping[str, Any]]) -> str:
    if isinstance(manifest, Compilation):
        return manifest.manifest_json
    if isinstance(manifest, str):
        return manifest
    return json.dumps(manifest)


def load_manifest(manifest: Union[str, Mapping[str, Any]]) -> Manifest:
    """Parse and check a manifest; raises ``ManifestError`` for unsupported versions."""
    return _native.load_manifest(_manifest_json(manifest))


class Database:
    """An open Neo4j or LadybugDB database; create one with ``Neo4j`` or ``Ladybug``.

    Calls on one handle run one at a time on the handle's own thread and release the
    GIL while they run. Close the handle (or use it as a context manager) when done.
    """

    def __init__(self, native: _native.Database) -> None:
        self._native = native

    @property
    def dialect(self) -> Dialect:
        return "ladybug" if self._native.dialect == "ladybug" else "neo4j"

    @property
    def closed(self) -> bool:
        return self._native.closed

    def validate(
        self,
        shapes: Union[ShapesInput, Iterable[ShapesInput], None] = None,
        *,
        manifest: Union[Compilation, str, Mapping[str, Any], None] = None,
        limit: Optional[int] = 100,
        sample_size: int = 5,
        timeout: Optional[float] = None,
        schema: Union[str, Mapping[str, Any], None] = None,
        ontologies: Union[ShapesInput, Iterable[ShapesInput]] = (),
        node_key: Optional[str] = None,
        neo4j_labels: Neo4jLabels = "explicit",
        strict: bool = False,
        lenient: bool = False,
        verbose: bool = False,
        max_path_depth: int = 10,
        allow_remote_imports: bool = False,
        fail_on_schema_mismatch: bool = False,
        base_dir: Optional[PathLike] = None,
    ) -> Report:
        """Validate shapes (compiled for this database) or a manifest, like ``shacl2cypher validate``.

        ``limit`` of ``0`` or ``None`` lists every violation; ``timeout`` is per query,
        in seconds. Compile options apply only when validating shapes.
        """
        if (shapes is None) == (manifest is None):
            raise ValueError("pass either shapes or manifest")
        if limit is not None and limit < 0:
            raise ValueError("limit must not be negative")
        if sample_size < 0:
            raise ValueError("sample_size must not be negative")
        if manifest is not None:
            return self._native.validate_manifest(
                _manifest_json(manifest), limit, sample_size, timeout
            )
        assert shapes is not None
        options = _options(
            dialect=self.dialect,
            schema=schema,
            node_key=node_key,
            neo4j_labels=neo4j_labels,
            strict=strict,
            lenient=lenient,
            verbose=verbose,
            max_path_depth=max_path_depth,
            allow_remote_imports=allow_remote_imports,
            fail_on_schema_mismatch=fail_on_schema_mismatch,
            base_dir=base_dir,
        )
        return self._native.validate_shapes(
            _sources(shapes), _sources(ontologies), options, limit, sample_size, timeout
        )

    def schema(self) -> SchemaSnapshot:
        """The database's schema snapshot, usable as ``compile(..., schema=...)``."""
        snapshot: SchemaSnapshot = json.loads(self.schema_json())
        return snapshot

    def schema_json(self) -> str:
        """The schema snapshot as the text ``shacl2cypher schema dump`` writes."""
        return self._native.schema_json()

    def close(self) -> None:
        """Wait for running calls, then close the database. Closing twice is a no-op."""
        self._native.close()

    def __enter__(self) -> Database:
        return self

    def __exit__(
        self,
        exc_type: Optional[Type[BaseException]],
        exc: Optional[BaseException],
        traceback: Optional[TracebackType],
    ) -> None:
        self.close()


def Neo4j(
    uri: str, user: str = "neo4j", password: str = "", database: Optional[str] = None
) -> Database:
    """Connect to Neo4j over Bolt; the connection is checked before returning."""
    return Database(_native.Database.neo4j(uri, user, password, database))


def Ladybug(path: PathLike) -> Database:
    """Open an existing LadybugDB database file read-only."""
    return Database(_native.Database.ladybug(os.fspath(path)))
