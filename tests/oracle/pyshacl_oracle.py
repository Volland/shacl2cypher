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


class Skipped(Exception):
    pass


def curie(graph, term):
    if isinstance(term, BNode):
        raise Unsupported(f"blank node {term} has no stable name")
    return graph.namespace_manager.normalizeUri(term)


def path_display(shapes, path):
    """SPARQL-like path syntax matching the compiler, e.g. `ex:a/^ex:b`, `ex:knows+`."""
    if isinstance(path, URIRef):
        return curie(shapes, path)
    if shapes.value(path, RDF.first) is not None:
        return "/".join(path_atom(shapes, step) for step in shapes.items(path))
    for predicate, template in (
        (SH.inversePath, "^{}"),
        (SH.zeroOrMorePath, "{}*"),
        (SH.oneOrMorePath, "{}+"),
        (SH.zeroOrOnePath, "{}?"),
    ):
        inner = shapes.value(path, predicate)
        if inner is not None:
            return template.format(path_atom(shapes, inner))
    alternatives = shapes.value(path, SH.alternativePath)
    if alternatives is not None:
        return "|".join(path_atom(shapes, option) for option in shapes.items(alternatives))
    raise Unsupported(f"unknown path form {path}")


def path_atom(shapes, path):
    text = path_display(shapes, path)
    compound = isinstance(path, BNode) and (
        shapes.value(path, RDF.first) is not None or shapes.value(path, SH.alternativePath) is not None
    )
    return f"({text})" if compound else text


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
    parents = sorted(shapes.subjects(SH.property, source), key=str)
    if not parents:
        raise Unsupported(f"property shape {source} has no parent node shape")
    return [f"{curie(shapes, p)}/{path_display(shapes, path)}/sh:{param}" for p in parents]


def run_fixture(bin_path, fixture_path):
    raw = subprocess.run(
        [bin_path, "oracle-input", str(fixture_path)], check=True, capture_output=True, text=True
    ).stdout
    data = json.loads(raw)
    if data["oracleSkip"]:
        raise Skipped(data["oracleSkip"])
    shapes = Graph().parse(data["shapes"], format="turtle")
    data_graph = Graph().parse(data=data["ntriples"], format="nt")
    _, results, _ = validate(
        data_graph, shacl_graph=shapes, inference="none", abort_on_first=False, allow_warnings=True
    )
    base = data["base"] + "node/"
    actual = set()
    # Only the report's own results; nested sh:detail results explain them.
    reports = list(results.subjects(RDF.type, SH.ValidationReport))
    for result in (r for report in reports for r in results.objects(report, SH.result)):
        focus = str(results.value(result, SH.focusNode))
        if not focus.startswith(base):
            raise Unsupported(f"focus {focus} is outside the fixture namespace")
        for rule in rule_ids(shapes, results, result):
            actual.add((rule, focus.removeprefix(base)))
    expected = {(v["rule"], v["focus"]) for v in data["expect"]}
    known = {(k["rule"], k["focus"]): k["reason"] for k in data["knownDifferences"]}
    missing = {v for v in expected - actual if v not in known}
    unexpected = {v for v in actual - expected if v not in known}
    return missing, unexpected


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--bin", default=str(ROOT / "target/debug/s2c-fixture"))
    parser.add_argument("fixtures", nargs="*", type=pathlib.Path)
    args = parser.parse_args()
    fixtures = args.fixtures or sorted((ROOT / "tests/conformance").rglob("*.yaml"))

    failures = 0
    skipped = 0
    for path in fixtures:
        name = path.relative_to(ROOT) if path.is_absolute() else path
        try:
            missing, unexpected = run_fixture(args.bin, path)
        except Skipped as e:
            print(f"skip {name}: {e}")
            skipped += 1
            continue
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
    checked = len(fixtures) - skipped
    print(f"{checked - failures}/{checked} fixtures agree with pySHACL ({skipped} skipped)")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
