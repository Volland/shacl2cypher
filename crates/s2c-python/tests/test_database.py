"""Database handles through the Python binding: validate, schema dump, reports, threads."""

from __future__ import annotations

import json
import threading
import time
from pathlib import Path

import pytest

import shacl2cypher
from shacl2cypher import Source

from conftest import CORE_FIXTURES, SHAPES, requires_ladybug, run_cli

pytestmark = requires_ladybug


# @lat: [[tests#Python Binding#Database Handles]]
def test_handles_open_close_and_reject_use_after_close(tmp_path: Path, ladybug_db: Path) -> None:
    missing = tmp_path / "missing.lbug"
    with pytest.raises(shacl2cypher.DatabaseConnectionError):
        shacl2cypher.Ladybug(missing)
    assert not missing.exists()

    with shacl2cypher.Ladybug(ladybug_db) as db:
        assert db.dialect == "ladybug"
        assert not db.closed
    assert db.closed
    db.close()
    with pytest.raises(shacl2cypher.DatabaseClosedError):
        db.validate([SHAPES], node_key="id")
    with pytest.raises(shacl2cypher.DatabaseClosedError):
        db.schema()


# @lat: [[tests#Python Binding#Validate]]
def test_validates_shapes_and_manifests(ladybug_db: Path) -> None:
    with shacl2cypher.Ladybug(ladybug_db) as db:
        report = db.validate([SHAPES], node_key="id")
        assert not report.conforms
        assert report.complete
        rules = {rule["name"]: rule for rule in report.rules}
        has_value = rules["PersonShape.tags.hasValue"]
        assert has_value["status"] == "failed"
        assert has_value["violationCount"] == 2
        assert len(has_value["violations"]) == 2

        manifest = shacl2cypher.compile([SHAPES], dialect="ladybug", node_key="id", schema=db.schema())
        from_manifest = db.validate(manifest=manifest)
        assert from_manifest.summary["violations"] == report.summary["violations"]

        limited = db.validate([SHAPES], node_key="id", limit=1)
        limited_rules = {rule["name"]: rule for rule in limited.rules}
        assert len(limited_rules["PersonShape.tags.hasValue"]["violations"]) == 1

        neo4j_manifest = shacl2cypher.compile([SHAPES], dialect="neo4j")
        with pytest.raises(shacl2cypher.ManifestError, match="neo4j.*ladybug"):
            db.validate(manifest=neo4j_manifest)
        with pytest.raises(ValueError, match="timeout must be a positive"):
            db.validate([SHAPES], timeout=0)
        with pytest.raises(ValueError, match="either shapes or manifest"):
            db.validate()


# @lat: [[tests#Python Binding#Reports And Exit Codes]]
def test_reports_render_like_the_cli(tmp_path: Path, ladybug_db: Path, cli: Path) -> None:
    with shacl2cypher.Ladybug(ladybug_db) as db:
        report = db.validate([SHAPES], node_key="id", base_dir=CORE_FIXTURES)
    result = run_cli(
        cli, "validate", SHAPES.name, "--ladybug", ladybug_db, "--node-key", "id",
        "--format", "json", "-o", tmp_path / "report.json", cwd=CORE_FIXTURES,
    )
    assert result.returncode == 1, result.stderr
    cli_report = shacl2cypher.Report.from_json((tmp_path / "report.json").read_text())

    def untimed(data: object) -> object:
        if isinstance(data, dict):
            return {k: 0 if k == "durationMs" else untimed(v) for k, v in data.items()}
        if isinstance(data, list):
            return [untimed(item) for item in data]
        return data

    assert untimed(report.to_dict()) == untimed(cli_report.to_dict())
    for format in ("table", "json", "junit", "sarif"):
        assert report.render(format)
    assert json.loads(report.to_json()) == report.to_dict()
    assert report.exit_code() == 1
    with pytest.raises(ValueError, match="format must be one of"):
        report.render("html")  # type: ignore[arg-type]


def test_exit_codes_follow_fail_on(ladybug_db: Path) -> None:
    with shacl2cypher.Ladybug(ladybug_db) as db:
        data = db.validate([SHAPES], node_key="id").to_dict()
    for rule in data["rules"]:
        rule["severity"] = "Warning"
    warnings_only = shacl2cypher.Report.from_json(json.dumps(data))
    assert warnings_only.exit_code("violation") == 0
    assert warnings_only.exit_code("warning") == 1
    assert warnings_only.exit_code("info") == 1


# @lat: [[tests#Python Binding#Schema Dump]]
def test_schema_dump_round_trips_into_compile(tmp_path: Path, ladybug_db: Path, cli: Path) -> None:
    with shacl2cypher.Ladybug(ladybug_db) as db:
        snapshot = db.schema()
        text = db.schema_json()
    result = run_cli(cli, "schema", "dump", "--ladybug", ladybug_db, "-o", tmp_path / "schema.json", cwd=tmp_path)
    assert result.returncode == 0, result.stderr
    assert text == (tmp_path / "schema.json").read_text()
    from_object = shacl2cypher.compile([SHAPES], dialect="ladybug", schema=snapshot)
    from_file = shacl2cypher.compile([SHAPES], dialect="ladybug", schema=text)
    assert from_object.manifest_json == from_file.manifest_json


def _many_rules(count: int) -> Source:
    properties = " ,\n".join(
        f'  [ sh:path ex:status ; sh:minLength {n} ; s2c:name "rule{n}" ]' for n in range(count)
    )
    text = (
        "@prefix sh: <http://www.w3.org/ns/shacl#> .\n"
        "@prefix ex: <http://example.org/> .\n"
        "@prefix s2c: <https://w3id.org/shacl2cypher#> .\n"
        f"ex:PersonShape sh:targetClass ex:Person ;\n  sh:property\n{properties} .\n"
    )
    return Source("many.ttl", text)


# @lat: [[tests#Python Binding#Non-Blocking Execution]]
def test_validation_releases_the_gil(ladybug_db: Path) -> None:
    counter = 0
    running = True

    def spin() -> None:
        nonlocal counter
        while running:
            counter += 1

    with shacl2cypher.Ladybug(ladybug_db) as db:
        shapes = [_many_rules(300)]
        thread = threading.Thread(target=spin, daemon=True)
        thread.start()
        try:
            started = time.monotonic()
            before = counter
            db.validate(shapes, node_key="id")
            during = counter - before
            elapsed = time.monotonic() - started
        finally:
            running = False
            thread.join()
    assert elapsed > 0.05, f"validation too fast to observe ({elapsed:.3f}s)"
    # Holding the GIL for the whole call would let the spinner run at most one
    # switch interval (5 ms); it keeps counting while Rust runs instead.
    assert during > 100_000, (during, elapsed)


def test_concurrent_calls_on_one_handle_complete(ladybug_db: Path) -> None:
    results: list[shacl2cypher.Report] = []
    with shacl2cypher.Ladybug(ladybug_db) as db:
        threads = [
            threading.Thread(target=lambda: results.append(db.validate([SHAPES], node_key="id")))
            for _ in range(2)
        ]
        for thread in threads:
            thread.start()
        for thread in threads:
            thread.join()
    assert len(results) == 2
    assert all(report.complete for report in results)
