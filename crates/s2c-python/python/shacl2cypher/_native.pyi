import os
from typing import Any, List, Mapping, Optional, Sequence, Tuple, Union

from ._types import (
    FailOn,
    Manifest,
    ManifestDiagnostic,
    ReportData,
    ReportFormat,
    RuleResult,
    Summary,
)

_Source = Union[str, Tuple[str, str, Optional[str]]]

__version__: str


class Shacl2CypherError(Exception): ...


class CompileError(Shacl2CypherError):
    errors: List[str]


class ManifestError(Shacl2CypherError): ...


class DatabaseConnectionError(Shacl2CypherError): ...


class BackendUnavailableError(Shacl2CypherError): ...


class DatabaseClosedError(Shacl2CypherError): ...


def available_backends() -> List[str]: ...


def compile(
    shapes: Sequence[_Source], ontologies: Sequence[_Source], options: Mapping[str, Any]
) -> "Compilation": ...


def load_manifest(text: str) -> Manifest: ...


class Compilation:
    @property
    def manifest(self) -> Manifest: ...
    @property
    def manifest_json(self) -> str: ...
    @property
    def cypher(self) -> str: ...
    @property
    def static_diagnostics(self) -> List[ManifestDiagnostic]: ...
    def write(self, directory: Union[str, "os.PathLike[str]"]) -> None: ...


class Report:
    @staticmethod
    def from_json(text: str) -> "Report": ...
    @property
    def conforms(self) -> bool: ...
    @property
    def complete(self) -> bool: ...
    @property
    def summary(self) -> Summary: ...
    @property
    def rules(self) -> List[RuleResult]: ...
    def to_dict(self) -> ReportData: ...
    def to_json(self) -> str: ...
    def render(self, format: ReportFormat) -> str: ...
    def exit_code(self, fail_on: FailOn = "violation") -> int: ...


class Database:
    @staticmethod
    def neo4j(uri: str, user: str, password: str, database: Optional[str]) -> "Database": ...
    @staticmethod
    def ladybug(path: str) -> "Database": ...
    @property
    def dialect(self) -> str: ...
    @property
    def closed(self) -> bool: ...
    def validate_manifest(
        self, manifest_json: str, limit: Optional[int], sample_size: int, timeout: Optional[float]
    ) -> Report: ...
    def validate_shapes(
        self,
        shapes: Sequence[_Source],
        ontologies: Sequence[_Source],
        options: Mapping[str, Any],
        limit: Optional[int],
        sample_size: int,
        timeout: Optional[float],
    ) -> Report: ...
    def schema_json(self) -> str: ...
    def close(self) -> None: ...
