"""Release smoke test for an installed `shacl2cypher` Python package.

Usage:
    python tests/bindings/smoke.py <s2c-fixture>   # wheel with both backends
    python tests/bindings/smoke.py --compile-only  # source build without backends

Run it outside `crates/s2c-python` so the installed package is imported.
"""

import subprocess
import sys
import tempfile
from pathlib import Path

import shacl2cypher

CORE = Path(__file__).resolve().parents[2] / "tests" / "conformance" / "core"


def main(args: list) -> None:
    compile_only = args == ["--compile-only"]
    expected = [] if compile_only else ["neo4j", "ladybug"]
    backends = shacl2cypher.available_backends()
    assert backends == expected, f"backends {backends}, expected {expected}"

    compilation = shacl2cypher.compile([CORE / "in-has-value.ttl"], dialect="neo4j", node_key="id")
    assert compilation.manifest["compilerVersion"] == shacl2cypher.__version__
    assert "// name: PersonShape.status.in" in compilation.cypher

    if not compile_only:
        (fixture_tool,) = args
        with tempfile.TemporaryDirectory() as directory:
            database = Path(directory) / "graph.lbug"
            subprocess.run(
                [fixture_tool, "ladybug-db", str(CORE / "in-has-value.yaml"), str(database)],
                check=True,
            )
            with shacl2cypher.Ladybug(database) as db:
                report = db.validate([CORE / "in-has-value.ttl"], node_key="id")
            assert report.summary["violations"] == 4, report.summary
    print(f"shacl2cypher {shacl2cypher.__version__} ok (backends: {backends})")


if __name__ == "__main__":
    main(sys.argv[1:])
