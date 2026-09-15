"""Shared fixtures: repository paths, the CLI and fixture binaries, LadybugDB files."""

from __future__ import annotations

import os
import subprocess
from pathlib import Path

import pytest

import shacl2cypher

ROOT = Path(__file__).resolve().parents[3]
CORE_FIXTURES = ROOT / "tests" / "conformance" / "core"
FIXTURE = CORE_FIXTURES / "in-has-value.yaml"
SHAPES = CORE_FIXTURES / "in-has-value.ttl"

requires_ladybug = pytest.mark.skipif(
    "ladybug" not in shacl2cypher.available_backends(),
    reason="built without the ladybug backend",
)


def _binary(variable: str, name: str) -> Path:
    path = Path(os.environ.get(variable, ROOT / "target" / "debug" / name))
    if not path.exists():
        pytest.skip(f"{name} not built; set {variable} or run cargo build")
    return path


@pytest.fixture(scope="session")
def cli() -> Path:
    """`shacl2cypher` built with the ladybug feature (`S2C_CLI`)."""
    return _binary("S2C_CLI", "shacl2cypher")


@pytest.fixture(scope="session")
def fixture_tool() -> Path:
    """`s2c-fixture` built with the ladybug feature (`S2C_FIXTURE`)."""
    return _binary("S2C_FIXTURE", "s2c-fixture")


@pytest.fixture
def ladybug_db(tmp_path: Path, fixture_tool: Path) -> Path:
    """A LadybugDB file holding the `in-has-value` fixture graph."""
    path = tmp_path / "graph.lbug"
    subprocess.run([fixture_tool, "ladybug-db", FIXTURE, path], check=True)
    return path


def run_cli(cli: Path, *args: object, cwd: Path) -> subprocess.CompletedProcess[str]:
    return subprocess.run(
        [str(cli), *map(str, args)], cwd=cwd, capture_output=True, text=True
    )
