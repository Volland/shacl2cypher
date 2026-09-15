"""Compile through the Python binding: sources, options, errors, parity with the CLI."""

from __future__ import annotations

import http.server
import threading
from pathlib import Path
from typing import Iterator

import pytest

import shacl2cypher
from shacl2cypher import CompileError, ManifestError, Source

from conftest import CORE_FIXTURES, SHAPES, run_cli

PREFIXES = (
    "@prefix sh: <http://www.w3.org/ns/shacl#> .\n"
    "@prefix ex: <http://example.org/> .\n"
    "@prefix s2c: <https://w3id.org/shacl2cypher#> .\n"
)


# @lat: [[tests#Python Binding#Compile From Files And Documents]]
def test_compiles_files_and_in_memory_documents(tmp_path: Path) -> None:
    from_file = shacl2cypher.compile([SHAPES], dialect="neo4j", node_key="id")
    assert isinstance(from_file.manifest, dict)
    assert isinstance(from_file.manifest_json, str)
    assert "// name: PersonShape.status.in" in from_file.cypher
    assert any(r["ruleId"] == "ex:PersonShape/ex:status/sh:in" for r in from_file.manifest["rules"])

    document = Source("person.ttl", SHAPES.read_text())
    from_document = shacl2cypher.compile(
        [document], dialect="neo4j", node_key="id", base_dir=tmp_path
    )
    assert from_document.manifest["inputs"][0]["path"] == "person.ttl"
    assert from_document.cypher == from_file.cypher

    with pytest.raises(CompileError, match="no shapes"):
        shacl2cypher.compile([], dialect="neo4j")


# @lat: [[tests#Python Binding#Output Parity With The CLI]]
def test_output_matches_the_cli_byte_for_byte(tmp_path: Path, cli: Path) -> None:
    result = run_cli(cli, "compile", SHAPES.name, "--dialect", "neo4j", "-o", tmp_path, cwd=CORE_FIXTURES)
    assert result.returncode == 0, result.stderr
    compilation = shacl2cypher.compile([SHAPES], dialect="neo4j", base_dir=CORE_FIXTURES)
    assert compilation.manifest_json == (tmp_path / "manifest.json").read_text()
    assert compilation.cypher == (tmp_path / "queries.cypher").read_text()


def test_option_values_are_checked() -> None:
    with pytest.raises(ValueError, match="dialect must be one of neo4j, ladybug"):
        shacl2cypher.compile([SHAPES], dialect="postgres")  # type: ignore[arg-type]
    with pytest.raises(ValueError, match="neo4j_labels"):
        shacl2cypher.compile([SHAPES], dialect="neo4j", neo4j_labels="all")  # type: ignore[arg-type]
    with pytest.raises(ValueError, match="format must be one of"):
        shacl2cypher.compile([Source("a.ttl", "", "json")], dialect="neo4j")  # type: ignore[arg-type]


def test_writes_outputs_into_a_new_directory(tmp_path: Path) -> None:
    compilation = shacl2cypher.compile([SHAPES], dialect="neo4j")
    out = tmp_path / "nested" / "out"
    compilation.write(out)
    assert (out / "manifest.json").read_text() == compilation.manifest_json
    assert (out / "queries.cypher").read_text() == compilation.cypher


# @lat: [[tests#Python Binding#Typed Errors]]
def test_compile_errors_list_every_problem(tmp_path: Path) -> None:
    text = (
        PREFIXES
        + 'ex:A sh:targetClass ex:P ; sh:property [ sh:path ex:n ; sh:minCount "two" ] .\n'
        + "\n\n"
        + 'ex:B sh:targetClass ex:P ; sh:property [ sh:path ex:m ; sh:maxCount "three" ] .\n'
    )
    with pytest.raises(CompileError) as caught:
        shacl2cypher.compile([Source("broken.ttl", text)], dialect="neo4j", base_dir=tmp_path)
    errors = caught.value.errors
    assert len(errors) == 2, errors
    assert any("broken.ttl:4" in e for e in errors), errors
    assert any("broken.ttl:7" in e for e in errors), errors
    assert not list(tmp_path.iterdir())

    for error in (CompileError, ManifestError, shacl2cypher.DatabaseConnectionError,
                  shacl2cypher.BackendUnavailableError, shacl2cypher.DatabaseClosedError):
        assert issubclass(error, shacl2cypher.Shacl2CypherError)


def test_static_diagnostics_are_returned_not_raised() -> None:
    schema = {
        "nodeTypes": [{"name": "Person", "properties": [{"name": "status", "type": "STRING"}]}],
        "relTypes": [],
    }
    compilation = shacl2cypher.compile([SHAPES], dialect="neo4j", schema=schema)
    diagnostics = compilation.static_diagnostics
    assert diagnostics, compilation.manifest_json
    assert {"code", "message", "source"} <= diagnostics[0].keys()
    assert diagnostics[0]["source"]["file"].endswith("in-has-value.ttl")


def test_schema_text_and_object_compile_alike() -> None:
    schema = {
        "nodeTypes": [
            {
                "name": "Person",
                "properties": [
                    {"name": "id", "type": "STRING"},
                    {"name": "level", "type": "INT64"},
                    {"name": "status", "type": "STRING"},
                    {"name": "tags", "type": "LIST<STRING>"},
                ],
            }
        ],
        "relTypes": [],
    }
    from_object = shacl2cypher.compile([SHAPES], dialect="ladybug", schema=schema)
    import json

    text = json.dumps(schema, indent=2) + "\n"
    from_text = shacl2cypher.compile([SHAPES], dialect="ladybug", schema=text)
    assert from_object.cypher == from_text.cypher
    assert from_object.manifest["rules"] == from_text.manifest["rules"]


def test_manifests_load_and_reject_other_versions() -> None:
    manifest = shacl2cypher.compile([SHAPES], dialect="neo4j").manifest
    assert shacl2cypher.load_manifest(manifest) == manifest
    with pytest.raises(ManifestError, match="99"):
        shacl2cypher.load_manifest({**manifest, "schemaVersion": 99})


# @lat: [[tests#Python Binding#Backend Availability]]
@pytest.mark.skipif(
    bool(shacl2cypher.available_backends()), reason="only for compile-only builds"
)
def test_compile_only_builds_open_no_database(tmp_path: Path) -> None:
    assert shacl2cypher.available_backends() == []
    with pytest.raises(shacl2cypher.BackendUnavailableError, match="no database backend"):
        shacl2cypher.Ladybug(tmp_path / "graph.lbug")


def test_version_matches_the_manifest() -> None:
    manifest = shacl2cypher.compile([SHAPES], dialect="neo4j").manifest
    assert manifest["compilerVersion"] == shacl2cypher.__version__


class _BigHandler(http.server.BaseHTTPRequestHandler):
    def do_GET(self) -> None:  # noqa: N802 - http.server API
        body = b"#" * (16 * 1024 * 1024 + 1)
        self.send_response(200)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args: object) -> None:
        pass


@pytest.fixture
def big_server() -> Iterator[str]:
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), _BigHandler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    yield f"http://127.0.0.1:{server.server_address[1]}/big.ttl"
    server.shutdown()


# @lat: [[tests#Python Binding#Remote Imports]]
def test_remote_imports_use_the_cli_limits(tmp_path: Path, big_server: str) -> None:
    text = (
        "@prefix owl: <http://www.w3.org/2002/07/owl#> .\n"
        f"<> owl:imports <{big_server}> .\n"
    )
    document = [Source("main.ttl", text)]
    with pytest.raises(CompileError, match="remote imports are disabled"):
        shacl2cypher.compile(document, dialect="neo4j", base_dir=tmp_path)
    if "ladybug" not in shacl2cypher.available_backends():
        pytest.skip("compile-only builds have no remote fetcher")
    with pytest.raises(CompileError) as caught:
        shacl2cypher.compile(
            document, dialect="neo4j", base_dir=tmp_path, allow_remote_imports=True
        )
    message = str(caught.value)
    assert big_server in message
    assert "larger than 16777216 bytes" in message
