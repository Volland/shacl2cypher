"""Neo4j through the Python binding; live tests need `S2C_NEO4J_URI`."""

from __future__ import annotations

import os

import pytest

import shacl2cypher

from conftest import SHAPES

requires_neo4j = pytest.mark.skipif(
    "neo4j" not in shacl2cypher.available_backends(),
    reason="built without the neo4j backend",
)
URI = os.environ.get("S2C_NEO4J_URI")
live = pytest.mark.skipif(URI is None, reason="set S2C_NEO4J_URI to run against Neo4j")


@requires_neo4j
def test_unreachable_neo4j_names_the_uri() -> None:
    uri = "bolt://127.0.0.1:1"
    with pytest.raises(shacl2cypher.DatabaseConnectionError, match="127.0.0.1:1"):
        shacl2cypher.Neo4j(uri, password="nope")


@requires_neo4j
@live
def test_validates_and_dumps_on_neo4j() -> None:
    assert URI is not None
    password = os.environ.get("S2C_NEO4J_PASSWORD", "")
    with shacl2cypher.Neo4j(URI, password=password) as db:
        assert db.dialect == "neo4j"
        report = db.validate([SHAPES], node_key="id")
        assert report.complete
        assert isinstance(db.schema()["nodeTypes"], list)
