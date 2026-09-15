"""The shared cases in `tests/bindings/cases.json` produce the CLI's exact output.

The Node suite compares the same cases with the same CLI, so Python and Node
outputs also equal each other.
"""

from __future__ import annotations

import json
import re
import subprocess
from pathlib import Path
from typing import Any, Dict, List

import pytest

import shacl2cypher

from conftest import ROOT, requires_ladybug, run_cli

CASES: Dict[str, List[Dict[str, Any]]] = json.loads(
    (ROOT / "tests" / "bindings" / "cases.json").read_text()
)
FORMATS = ("table", "json", "junit", "sarif")


def normalize(format: str, text: str) -> str:
    """Removes per-run timings: rule durations, totals and JUnit times."""
    if format in ("json", "sarif"):
        return re.sub(r'"durationMs": \d+', '"durationMs": 0', text)
    if format == "junit":
        return re.sub(r'time="[\d.]+"', 'time="0"', text)
    lines = []
    for line in text.splitlines():
        fields = line.split()
        if len(fields) == 5 and fields[3].isdigit() and fields[2].isdigit():
            fields[3] = "0"
            line = " ".join(fields)
        lines.append(re.sub(r" in \d+ ms$", " in 0 ms", line))
    return "\n".join(lines)


# @lat: [[tests#Python Binding#Shared Parity Cases]]
@pytest.mark.parametrize("case", CASES["compile"], ids=lambda case: case["name"])
def test_shared_cases_compile_like_the_cli(case: Dict[str, Any], tmp_path: Path, cli: Path) -> None:
    cwd = ROOT / case["cwd"]
    args: List[object] = ["compile", *case["shapes"], "--dialect", case["dialect"], "-o", tmp_path]
    if "nodeKey" in case:
        args += ["--node-key", case["nodeKey"]]
    if "schema" in case:
        args += ["--schema", case["schema"]]
    result = run_cli(cli, *args, cwd=cwd)
    assert result.returncode == 0, result.stderr

    compilation = shacl2cypher.compile(
        [cwd / shape for shape in case["shapes"]],
        dialect=case["dialect"],
        node_key=case.get("nodeKey"),
        schema=(cwd / case["schema"]).read_text() if "schema" in case else None,
        base_dir=cwd,
    )
    assert compilation.manifest_json == (tmp_path / "manifest.json").read_text()
    assert compilation.cypher == (tmp_path / "queries.cypher").read_text()


# @lat: [[tests#Python Binding#Report Parity]]
@requires_ladybug
@pytest.mark.parametrize("case", CASES["reports"], ids=lambda case: case["name"])
def test_reports_render_every_format_like_the_cli(
    case: Dict[str, Any], tmp_path: Path, cli: Path, fixture_tool: Path
) -> None:
    database = tmp_path / "graph.lbug"
    subprocess.run([fixture_tool, "ladybug-db", ROOT / case["fixture"], database], check=True)
    cwd = ROOT / case["cwd"]
    with shacl2cypher.Ladybug(database) as db:
        report = db.validate(
            [cwd / shape for shape in case["shapes"]], node_key=case["nodeKey"], base_dir=cwd
        )
    for format in FORMATS:
        out = tmp_path / f"report.{format}"
        result = run_cli(
            cli, "validate", *case["shapes"], "--ladybug", database, "--node-key", case["nodeKey"],
            "--format", format, "-o", out, cwd=cwd,
        )
        assert result.returncode in (0, 1), result.stderr
        assert normalize(format, report.render(format)) == normalize(format, out.read_text()), format
