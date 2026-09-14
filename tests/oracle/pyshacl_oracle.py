#!/usr/bin/env python3
"""pySHACL reference oracle for conformance fixtures.

For each fixture under tests/conformance, the Rust testkit projects the graph to
N-Triples; pySHACL validates it; results are mapped to (ruleId, focus) pairs and
compared with the fixture's hand-written `expect` block.

Rule ids follow the compiler's structural scheme `{nodeShape}/{path}/{param}`
(or `{nodeShape}/{param}` for node-level constraints). Results whose shape or path
cannot be named that way (blank-node node shapes, complex paths) are reported as
unsupported and make the fixture fail loudly rather than pass silently.
"""

import argparse
import json
import pathlib
import subprocess
import sys

from pyshacl import validate
from rdflib import RDF, BNode, Graph, Namespace, URIRef

SH = Namespace("http://www.w3.org/ns/shacl#")
ROOT = pathlib.Path(__file__).resolve().parents[2]

# Constraint component -> the parameter name used in rule ids.
COMPONENT_PARAMS = {
    "MinCount": "minCount", "MaxCount": "maxCount", "Datatype": "datatype",
    "NodeKind": "nodeKind", "Class": "class", "MinInclusive": "minInclusive",
    "MaxInclusive": "maxInclusive", "MinExclusive": "minExclusive",
    "MaxExclusive": "maxExclusive", "MinLength": "minLength", "MaxLength": "maxLength",
    "Pattern": "pattern", "In": "in", "HasValue": "hasValue", "Equals": "equals",
    "Disjoint": "disjoint", "LessThan": "lessThan", "LessThanOrEquals": "lessThanOrEquals",
    "Closed": "closed", "Node": "node", "Not": "not", "And": "and", "Or": "or",
    "Xone": "xone", "QualifiedMinCount": "qualifiedMinCount",
    "QualifiedMaxCount": "qualifiedMaxCount",
}


class Unsupported(Exception):
    pass


def curie(graph, term):
    if isinstance(term, BNode):
        raise Unsupported(f"blank node {term} has no stable name")
    return graph.namespace_manager.normalizeUri(term)


def rule_ids(shapes, result_graph, result):
    component = result_graph.value(result, SH.sourceConstraintComponent)
    source = result_graph.value(result, SH.sourceShape)
    local = str(component).removeprefix(str(SH)).removesuffix("ConstraintComponent")
    param = COMPONENT_PARAMS.get(local)
    if param is None:
        raise Unsupported(f"component {component}")
    path = shapes.value(source, SH.path)
    if path is None:
        return [f"{curie(shapes, source)}/sh:{param}"]
    if not isinstance(path, URIRef):
        raise Unsupported(f"complex path on {source}")
    parents = sorted(shapes.subjects(SH.property, source), key=str)
    if not parents:
        raise Unsupported(f"property shape {source} has no parent node shape")
    return [f"{curie(shapes, p)}/{curie(shapes, path)}/sh:{param}" for p in parents]


def run_fixture(bin_path, fixture_path):
    raw = subprocess.run(
        [bin_path, "oracle-input", str(fixture_path)], check=True, capture_output=True, text=True
    ).stdout
    data = json.loads(raw)
    shapes = Graph().parse(data["shapes"], format="turtle")
    data_graph = Graph().parse(data=data["ntriples"], format="nt")
    _, results, _ = validate(
        data_graph, shacl_graph=shapes, inference="none", abort_on_first=False, allow_warnings=True
    )
    base = data["base"] + "node/"
    actual = set()
    for result in results.subjects(RDF.type, SH.ValidationResult):
        focus = str(results.value(result, SH.focusNode))
        if not focus.startswith(base):
            raise Unsupported(f"focus {focus} is outside the fixture namespace")
        for rule in rule_ids(shapes, results, result):
            actual.add((rule, focus.removeprefix(base)))
    expected = {(v["rule"], v["focus"]) for v in data["expect"]}
    return expected - actual, actual - expected


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--bin", default=str(ROOT / "target/debug/s2c-fixture"))
    parser.add_argument("fixtures", nargs="*", type=pathlib.Path)
    args = parser.parse_args()
    fixtures = args.fixtures or sorted((ROOT / "tests/conformance").rglob("*.yaml"))

    failures = 0
    for path in fixtures:
        name = path.relative_to(ROOT) if path.is_absolute() else path
        try:
            missing, unexpected = run_fixture(args.bin, path)
        except Unsupported as e:
            print(f"UNSUPPORTED {name}: {e}")
            failures += 1
            continue
        if missing or unexpected:
            failures += 1
            print(f"FAIL {name}")
            for rule, focus in sorted(missing):
                print(f"  missing:    {rule} @ {focus}")
            for rule, focus in sorted(unexpected):
                print(f"  unexpected: {rule} @ {focus}")
        else:
            print(f"ok   {name}")
    print(f"{len(fixtures) - failures}/{len(fixtures)} fixtures agree with pySHACL")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
