"""The TypedDicts describe exactly the keys the Rust side serializes."""

from __future__ import annotations

import sys
from pathlib import Path
from typing import Any, Dict, List, Optional, Union, get_args, get_origin, get_type_hints

import pytest

import shacl2cypher
from shacl2cypher import _types

from conftest import SHAPES, requires_ladybug


def _check(value: Any, hint: Any, where: str) -> None:
    origin = get_origin(hint)
    if origin is Union:
        options = [a for a in get_args(hint) if a is not type(None)]
        if value is not None:
            _check(value, options[0], where)
        return
    if origin in (list, List):
        for index, item in enumerate(value):
            _check(item, get_args(hint)[0], f"{where}[{index}]")
        return
    if isinstance(hint, type) and issubclass(hint, dict) and hasattr(hint, "__required_keys__"):
        required = set(hint.__required_keys__)
        optional = set(hint.__optional_keys__)
        keys = set(value)
        assert required <= keys, f"{where}: missing {required - keys}"
        assert keys <= required | optional, f"{where}: undeclared {keys - required - optional}"
        hints = get_type_hints(hint)
        for key in keys:
            _check(value[key], hints[key], f"{where}.{key}")


# @lat: [[tests#Python Binding#Published Types]]
def test_manifest_keys_match_the_typed_dicts() -> None:
    manifest = shacl2cypher.compile([SHAPES], dialect="neo4j", node_key="id").manifest
    _check(manifest, _types.Manifest, "manifest")


@requires_ladybug
def test_report_and_schema_keys_match_the_typed_dicts(ladybug_db: Path) -> None:
    with shacl2cypher.Ladybug(ladybug_db) as db:
        report = db.validate([SHAPES], node_key="id")
        schema = db.schema()
    _check(report.to_dict(), _types.ReportData, "report")
    _check(schema, _types.SchemaSnapshot, "schema")


@pytest.mark.skipif(sys.version_info < (3, 9), reason="typing introspection")
def test_rules_with_optional_keys_are_declared() -> None:
    assert "statusReason" in _types.ManifestRule.__optional_keys__
    assert "pathDepthCap" in _types.ManifestRule.__optional_keys__
    assert "statusReason" in _types.RuleResult.__optional_keys__
