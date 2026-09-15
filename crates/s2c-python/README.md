# shacl2cypher for Python

Compile [SHACL](https://www.w3.org/TR/shacl/) shapes into named, read-only Cypher queries, and validate Neo4j 5 or LadybugDB graphs from Python.

```python
import shacl2cypher

compilation = shacl2cypher.compile(["shapes/person.ttl"], dialect="neo4j", node_key="id")
compilation.write("out")

with shacl2cypher.Ladybug("graph.lbug") as db:
    report = db.validate(["shapes/person.ttl"], node_key="id")
    print(report.render("table"))
    raise SystemExit(report.exit_code("violation"))
```

Manifest and report dicts use the same camelCase keys as `manifest.json` and `--format json`. See the [project README](https://github.com/Volland/shacl2cypher#readme) for the mapping rules and the `s2c:` annotation vocabulary.
