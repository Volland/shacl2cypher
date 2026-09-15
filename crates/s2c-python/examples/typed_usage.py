"""Type-checked with `mypy --strict`: a consumer of the public API."""

from __future__ import annotations

import sys
from pathlib import Path
from typing import List, Optional

import shacl2cypher
from shacl2cypher import CompileError, ManifestRule, Report, RuleResult, Source


def compile_person_shapes(directory: Path) -> List[ManifestRule]:
    document = Source("person.ttl", "@prefix sh: <http://www.w3.org/ns/shacl#> .\n")
    try:
        compilation = shacl2cypher.compile(
            [directory / "shapes.ttl", document], dialect="neo4j", node_key="id"
        )
    except CompileError as error:
        for message in error.errors:
            print(message, file=sys.stderr)
        return []
    compilation.write(directory / "out")
    return compilation.manifest["rules"]


def first_violation_count(report: Report) -> Optional[int]:
    rules: List[RuleResult] = report.rules
    return rules[0]["violationCount"] if rules else None


def validate(database: Path, shapes: Path) -> int:
    with shacl2cypher.Ladybug(database) as db:
        report = db.validate([shapes], node_key="id", limit=10, timeout=2.5)
        print(report.render("sarif"))
        print(first_violation_count(report))
        snapshot = db.schema()
        print([node["name"] for node in snapshot["nodeTypes"]])
        return report.exit_code("warning")


if __name__ == "__main__":
    raise SystemExit(validate(Path(sys.argv[1]), Path(sys.argv[2])))
