//! LPG projections of a fixture graph: Cypher load scripts for Neo4j and LadybugDB.
//!
//! Every node gets its fixture id in the `id` property. LadybugDB needs declared
//! tables, so its script also infers node and relationship table DDL from the data;
//! a node must then carry exactly one label and a property must have one type.

use std::collections::{BTreeMap, BTreeSet};

use crate::fixture::{Fixture, Value, ID_PROPERTY};

#[derive(Debug, Clone, PartialEq)]
pub enum ColumnType {
    String,
    Int64,
    Double,
    Bool,
    Date,
    List(Box<ColumnType>),
}

impl ColumnType {
    pub fn ddl(&self) -> String {
        match self {
            ColumnType::String => "STRING".into(),
            ColumnType::Int64 => "INT64".into(),
            ColumnType::Double => "DOUBLE".into(),
            ColumnType::Bool => "BOOL".into(),
            ColumnType::Date => "DATE".into(),
            ColumnType::List(elem) => format!("{}[]", elem.ddl()),
        }
    }
}

/// Neo4j load script: one statement per node, then one per edge.
pub fn neo4j_script(fixture: &Fixture) -> Vec<String> {
    let mut statements = Vec::new();
    for node in &fixture.graph.nodes {
        let labels: String = node
            .labels
            .iter()
            .map(|l| format!(":{}", ident(l)))
            .collect();
        let props = props_map(&node.id, &node.props, &BTreeMap::new());
        statements.push(format!("CREATE ({labels} {props})"));
    }
    for edge in &fixture.graph.edges {
        statements.push(format!(
            "MATCH (a {{{id}: {from}}}), (b {{{id}: {to}}}) CREATE (a)-[:{ty}{props}]->(b)",
            id = ident(ID_PROPERTY),
            from = quote(&edge.from),
            to = quote(&edge.to),
            ty = ident(&edge.rel_type()),
            props = edge_props(&edge.props, &BTreeMap::new()),
        ));
    }
    statements
}

/// LadybugDB load script: node table DDL, rel table DDL, node inserts, edge inserts.
pub fn ladybug_script(fixture: &Fixture) -> Result<Vec<String>, String> {
    /// Column name -> type inferred so far (`None` until a typed value is seen).
    type InferredColumns<'a> = BTreeMap<&'a str, Option<ColumnType>>;
    /// FROM/TO label pairs and inferred columns of one relationship table.
    type RelTable<'a> = (BTreeSet<(&'a str, &'a str)>, InferredColumns<'a>);

    let mut node_tables: BTreeMap<&str, InferredColumns> = BTreeMap::new();
    let mut node_label: BTreeMap<&str, &str> = BTreeMap::new();
    for node in &fixture.graph.nodes {
        let [label] = node.labels.as_slice() else {
            return Err(format!(
                "LadybugDB node `{}` needs exactly one label",
                node.id
            ));
        };
        node_label.insert(&node.id, label);
        let columns = node_tables.entry(label).or_default();
        for (key, value) in &node.props {
            let slot = columns.entry(key).or_insert(None);
            *slot = merge(slot.take(), infer(value)?).map_err(|e| format!("{label}.{key}: {e}"))?;
        }
    }

    let mut rel_tables: BTreeMap<String, RelTable> = BTreeMap::new();
    for edge in &fixture.graph.edges {
        let ty = edge.rel_type();
        let (pairs, columns) = rel_tables.entry(ty.clone()).or_default();
        pairs.insert((node_label[edge.from.as_str()], node_label[edge.to.as_str()]));
        for (key, value) in &edge.props {
            let slot = columns.entry(key).or_insert(None);
            *slot = merge(slot.take(), infer(value)?).map_err(|e| format!("{ty}.{key}: {e}"))?;
        }
    }

    let mut statements = Vec::new();
    let mut node_types: BTreeMap<&str, BTreeMap<&str, ColumnType>> = BTreeMap::new();
    for (label, columns) in &node_tables {
        let columns = resolve_columns(label, columns)?;
        let mut defs = vec![format!("{} STRING", ident(ID_PROPERTY))];
        defs.extend(
            columns
                .iter()
                .map(|(k, t)| format!("{} {}", ident(k), t.ddl())),
        );
        defs.push(format!("PRIMARY KEY({})", ident(ID_PROPERTY)));
        statements.push(format!(
            "CREATE NODE TABLE {}({})",
            ident(label),
            defs.join(", ")
        ));
        node_types.insert(label, columns);
    }
    let mut rel_types: BTreeMap<&str, BTreeMap<&str, ColumnType>> = BTreeMap::new();
    for (ty, (pairs, columns)) in &rel_tables {
        let columns = resolve_columns(ty, columns)?;
        let mut defs: Vec<String> = pairs
            .iter()
            .map(|(from, to)| format!("FROM {} TO {}", ident(from), ident(to)))
            .collect();
        defs.extend(
            columns
                .iter()
                .map(|(k, t)| format!("{} {}", ident(k), t.ddl())),
        );
        statements.push(format!(
            "CREATE REL TABLE {}({})",
            ident(ty),
            defs.join(", ")
        ));
        rel_types.insert(ty, columns);
    }

    for node in &fixture.graph.nodes {
        let label = node_label[node.id.as_str()];
        let props = props_map(&node.id, &node.props, &node_types[label]);
        statements.push(format!("CREATE (:{} {props})", ident(label)));
    }
    for edge in &fixture.graph.edges {
        let ty = edge.rel_type();
        statements.push(format!(
            "MATCH (a:{fl} {{{id}: {from}}}), (b:{tl} {{{id}: {to}}}) CREATE (a)-[:{t}{props}]->(b)",
            fl = ident(node_label[edge.from.as_str()]),
            tl = ident(node_label[edge.to.as_str()]),
            id = ident(ID_PROPERTY),
            from = quote(&edge.from),
            to = quote(&edge.to),
            t = ident(&ty),
            props = edge_props(&edge.props, &rel_types[ty.as_str()]),
        ));
    }
    Ok(statements)
}

fn resolve_columns<'a>(
    table: &str,
    columns: &BTreeMap<&'a str, Option<ColumnType>>,
) -> Result<BTreeMap<&'a str, ColumnType>, String> {
    columns
        .iter()
        .map(|(key, ty)| match ty {
            Some(ty) => Ok((*key, ty.clone())),
            None => Err(format!(
                "{table}.{key}: cannot infer a column type (only nulls or empty lists)"
            )),
        })
        .collect()
}

/// Column type of a value; `None` when the value carries no type (null, empty list).
fn infer(value: &Value) -> Result<Option<ColumnType>, String> {
    Ok(match value {
        Value::Null => None,
        Value::Bool(_) => Some(ColumnType::Bool),
        Value::Int(_) => Some(ColumnType::Int64),
        Value::Float(_) => Some(ColumnType::Double),
        Value::Str(_) => Some(ColumnType::String),
        Value::Date { .. } => Some(ColumnType::Date),
        Value::List(items) => {
            let mut elem = None;
            for item in items {
                elem = merge(elem, infer(item)?)?;
            }
            elem.map(|t| ColumnType::List(Box::new(t)))
        }
    })
}

fn merge(a: Option<ColumnType>, b: Option<ColumnType>) -> Result<Option<ColumnType>, String> {
    match (a, b) {
        (None, other) | (other, None) => Ok(other),
        (Some(x), Some(y)) if x == y => Ok(Some(x)),
        (Some(x), Some(y)) => Err(format!("conflicting types {} and {}", x.ddl(), y.ddl())),
    }
}

fn props_map(
    id: &str,
    props: &BTreeMap<String, Value>,
    types: &BTreeMap<&str, ColumnType>,
) -> String {
    let mut entries = vec![format!("{}: {}", ident(ID_PROPERTY), quote(id))];
    entries.extend(props_entries(props, types));
    format!("{{{}}}", entries.join(", "))
}

fn edge_props(props: &BTreeMap<String, Value>, types: &BTreeMap<&str, ColumnType>) -> String {
    let entries = props_entries(props, types);
    if entries.is_empty() {
        String::new()
    } else {
        format!(" {{{}}}", entries.join(", "))
    }
}

fn props_entries(
    props: &BTreeMap<String, Value>,
    types: &BTreeMap<&str, ColumnType>,
) -> Vec<String> {
    props
        .iter()
        .filter(|(_, v)| **v != Value::Null)
        .map(|(k, v)| format!("{}: {}", ident(k), literal(v, types.get(k.as_str()))))
        .collect()
}

fn literal(value: &Value, ty: Option<&ColumnType>) -> String {
    match value {
        Value::Null => "NULL".into(),
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Float(x) => {
            let s = format!("{x:?}");
            if s.contains(['.', 'e', 'E']) {
                s
            } else {
                format!("{s}.0")
            }
        }
        Value::Str(s) => quote(s),
        Value::Date { date } => format!("date({})", quote(date)),
        Value::List(items) if items.is_empty() => match ty {
            Some(list @ ColumnType::List(_)) => format!("CAST([] AS {})", list.ddl()),
            _ => "[]".into(),
        },
        Value::List(items) => {
            let elem = match ty {
                Some(ColumnType::List(elem)) => Some(elem.as_ref()),
                _ => None,
            };
            let items: Vec<String> = items.iter().map(|i| literal(i, elem)).collect();
            format!("[{}]", items.join(", "))
        }
    }
}

/// Single-quoted Cypher string literal; valid for both Neo4j and LadybugDB.
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}

/// Backtick-escaped identifier.
pub fn ident(s: &str) -> String {
    format!("`{}`", s.replace('`', "``"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(yaml: &str) -> Fixture {
        serde_yaml_ng::from_str(yaml).unwrap()
    }

    const GRAPH: &str = r#"
shapes: s.ttl
graph:
  nodes:
    - {id: p1, labels: [Person], props: {name: "O'Brien", tags: [a, b]}}
    - {id: p2, labels: [Person], props: {tags: []}}
    - {id: c1, labels: [Company]}
  edges:
    - {from: p1, to: c1, pred: worksFor, props: {since: {date: "2020-01-01"}}}
"#;

    #[test]
    fn neo4j_script_creates_nodes_and_edges() {
        let script = neo4j_script(&fixture(GRAPH));
        assert_eq!(
            script,
            vec![
                r"CREATE (:`Person` {`id`: 'p1', `name`: 'O\'Brien', `tags`: ['a', 'b']})",
                "CREATE (:`Person` {`id`: 'p2', `tags`: []})",
                "CREATE (:`Company` {`id`: 'c1'})",
                "MATCH (a {`id`: 'p1'}), (b {`id`: 'c1'}) CREATE (a)-[:`WORKS_FOR` {`since`: date('2020-01-01')}]->(b)",
            ]
        );
    }

    #[test]
    fn ladybug_script_infers_typed_tables() {
        let script = ladybug_script(&fixture(GRAPH)).unwrap();
        assert_eq!(
            script,
            vec![
                "CREATE NODE TABLE `Company`(`id` STRING, PRIMARY KEY(`id`))",
                "CREATE NODE TABLE `Person`(`id` STRING, `name` STRING, `tags` STRING[], PRIMARY KEY(`id`))",
                "CREATE REL TABLE `WORKS_FOR`(FROM `Person` TO `Company`, `since` DATE)",
                r"CREATE (:`Person` {`id`: 'p1', `name`: 'O\'Brien', `tags`: ['a', 'b']})",
                "CREATE (:`Person` {`id`: 'p2', `tags`: CAST([] AS STRING[])})",
                "CREATE (:`Company` {`id`: 'c1'})",
                "MATCH (a:`Person` {`id`: 'p1'}), (b:`Company` {`id`: 'c1'}) CREATE (a)-[:`WORKS_FOR` {`since`: date('2020-01-01')}]->(b)",
            ]
        );
    }

    #[test]
    fn ladybug_rejects_conflicting_types_and_multi_labels() {
        let conflict = fixture(
            "shapes: s.ttl\ngraph:\n  nodes:\n    - {id: a, labels: [T], props: {x: 1}}\n    - {id: b, labels: [T], props: {x: s}}\n",
        );
        assert!(ladybug_script(&conflict).unwrap_err().contains("T.x"));
        let multi = fixture("shapes: s.ttl\ngraph:\n  nodes:\n    - {id: a, labels: [A, B]}\n");
        assert!(ladybug_script(&multi)
            .unwrap_err()
            .contains("exactly one label"));
    }
}
