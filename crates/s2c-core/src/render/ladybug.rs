//! LadybugDB Cypher renderer. Column types from the schema snapshot decide value
//! types statically; constructs LadybugDB lacks are rewritten or rejected.

use std::collections::{BTreeSet, HashMap};

use super::{
    chain, constant, ident, quote, routes, Dialect, Hop, RenderError, Rendered, Route, RuleMeta,
};
use crate::ast::Direction;
use crate::datatypes::ColumnStatus;
use crate::ir::{
    Cmp, Constant, Detail, Expr, FocusSet, PairRelation, Rule, RuleStatus, ValueSource, ValueTest,
    Var, Violation,
};
use crate::mapping::LpgPath;
use crate::schema::{NodeType, SchemaSnapshot, ValueType};
use crate::xsd_regex::XsdRegex;

const LADYBUG: Dialect = Dialect::Ladybug;

/// What an IR variable is bound to.
#[derive(Debug, Clone)]
enum Binding {
    /// A node variable and the tables it may belong to (empty when unknown).
    Node {
        name: String,
        labels: Vec<String>,
    },
    Relationship {
        name: String,
        rel_type: String,
    },
    /// A non-null property value of a known column (element) type.
    Value {
        expr: String,
        value_type: ValueType,
    },
}

impl Binding {
    fn name(&self) -> &str {
        match self {
            Binding::Node { name, .. } | Binding::Relationship { name, .. } => name,
            Binding::Value { expr, .. } => expr,
        }
    }
}

type Env = HashMap<Var, Binding>;

/// One side of a pair constraint.
enum Side {
    Itself,
    /// A property no matched table declares.
    Empty,
    Scalar {
        expr: String,
        list: String,
        value_type: ValueType,
    },
    List {
        list: String,
        element: ValueType,
    },
    Nodes(Vec<Route>),
}

/// Renders a compiled rule as a detail query and a summary query.
// @lat: [[dialects#Dialect Backends#LadybugDB]]
pub fn render(
    rule: &Rule,
    meta: &RuleMeta<'_>,
    schema: &SchemaSnapshot,
) -> Result<Rendered, RenderError> {
    Renderer {
        meta,
        schema,
        names: 0,
    }
    .rule(rule)
}

struct Renderer<'a> {
    meta: &'a RuleMeta<'a>,
    schema: &'a SchemaSnapshot,
    names: u32,
}

impl Renderer<'_> {
    fn rule(&mut self, rule: &Rule) -> Result<Rendered, RenderError> {
        if rule.status != RuleStatus::Compiled {
            return Err(RenderError(format!(
                "rule {} is not rendered: status {:?}",
                rule.id, rule.status
            )));
        }
        let (mut body, focus, carry) = self.focus(&rule.focus)?;
        let mut env = Env::new();
        env.insert(Var::FOCUS, focus.clone());

        let (value, row_env) = match &rule.violation {
            Violation::PerFocus { condition, .. } => {
                match self.hoisted_count(condition, &env, carry)? {
                    Some(clauses) => body.push_str(&clauses),
                    None => {
                        let condition = self.expr(condition, &env)?;
                        body.push_str(&format!("\nWITH {carry}\nWHERE NOT ({condition})"));
                    }
                }
                (None, env)
            }
            Violation::PerValue {
                source, var, test, ..
            } => {
                let name = var.to_string();
                let (binding, guard) = self.bind_values(&mut body, source, &name, carry, &focus)?;
                // Unwound lists may contain NULL elements, so every property value is guarded.
                let guard = match &binding {
                    Binding::Value { .. } => format!("{name} IS NOT NULL AND "),
                    _ => guard,
                };
                let mut row_env = env.clone();
                row_env.insert(*var, binding.clone());
                let test = self.expr(test, &row_env)?;
                body.push_str(&format!("\nWHERE {guard}NOT ({test})"));
                (Some(binding), row_env)
            }
        };

        let meta = self.meta;
        let focus_struct = self.focus_struct(&focus)?;
        let focus_struct = if meta.verbose {
            self.with_properties(focus_struct, &focus)
        } else {
            focus_struct
        };
        let value_expr = match &value {
            None => "NULL".to_owned(),
            Some(Binding::Value { expr, .. }) => expr.clone(),
            Some(node) => self.node_struct(node)?,
        };
        let message = self.message(&focus, value.as_ref())?;
        let details = self.details(&rule.details, &row_env)?;
        let path = meta.path.map_or_else(|| "NULL".to_owned(), quote);
        let detail = format!(
            "{body}\nRETURN {} AS ruleId, {} AS shape, {path} AS path, {} AS `constraint`, {} AS severity,\n       {focus_struct} AS focus, {value_expr} AS value, {message} AS message, {details} AS details\nLIMIT $limit",
            quote(meta.name),
            quote(meta.shape),
            quote(meta.constraint),
            quote(meta.severity),
        );
        let summary = format!(
            "{body}\nWITH count(*) AS violationCount, collect({focus_struct}) AS samples\nRETURN {} AS ruleId, {} AS severity, violationCount, list_slice(coalesce(samples, []), 1, $sampleSize) AS sample",
            quote(meta.name),
            quote(meta.severity),
        );
        Ok(Rendered { detail, summary })
    }

    fn focus(
        &mut self,
        focus: &[FocusSet],
    ) -> Result<(String, Binding, &'static str), RenderError> {
        match focus {
            [FocusSet::Relationships(rel_type)] => Ok((
                format!("MATCH (s0)-[v0:{}]->(e0)", ident(rel_type)),
                Binding::Relationship {
                    name: "v0".into(),
                    rel_type: rel_type.clone(),
                },
                "s0, v0, e0",
            )),
            [FocusSet::Labels(labels)] => {
                let tables: Vec<String> = labels.iter().map(|label| ident(label)).collect();
                Ok((
                    format!("MATCH (v0:{})", tables.join(":")),
                    Binding::Node {
                        name: "v0".into(),
                        labels: labels.clone(),
                    },
                    "v0",
                ))
            }
            sets if !sets.is_empty()
                && !sets
                    .iter()
                    .any(|set| matches!(set, FocusSet::Relationships(_))) =>
            {
                let node = Binding::Node {
                    name: "v0".into(),
                    labels: Vec::new(),
                };
                let conditions = sets
                    .iter()
                    .map(|set| self.focus_condition(set, &node))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok((
                    format!("MATCH (v0)\nWHERE {}", conditions.join(" OR ")),
                    node,
                    "v0",
                ))
            }
            _ => Err(RenderError(
                "a rule needs node focus sets or a single relationship focus".into(),
            )),
        }
    }

    fn focus_condition(&mut self, set: &FocusSet, node: &Binding) -> Result<String, RenderError> {
        Ok(match set {
            FocusSet::Labels(labels) => format!("label(v0) IN [{}]", quoted_list(labels)),
            FocusSet::SubjectsOf(path) => {
                let mut parts = Vec::new();
                for route in routes(path)? {
                    let chain = chain(LADYBUG, &route.hops);
                    parts.push(match (&route.property, route.hops.is_empty()) {
                        (Some(key), true) => match self.column(node, key)? {
                            Some(_) => format!("v0.{} IS NOT NULL", ident(key)),
                            None => "false".to_owned(),
                        },
                        (None, _) => format!("EXISTS {{ MATCH (v0){chain}() }}"),
                        (Some(key), false) => {
                            let w = self.fresh();
                            let end = Binding::Node {
                                name: w.clone(),
                                labels: self.labels_after(&[], &route.hops),
                            };
                            match self.column(&end, key)? {
                                Some(_) => format!(
                                    "EXISTS {{ MATCH (v0){chain}({w}) WHERE {w}.{} IS NOT NULL }}",
                                    ident(key)
                                ),
                                None => "false".to_owned(),
                            }
                        }
                    });
                }
                format!("({})", parts.join(" OR "))
            }
            FocusSet::ObjectsOf(path) => {
                let planned = routes(path)?;
                if planned.iter().any(|route| route.property.is_some()) {
                    return Err(RenderError(
                        "sh:targetObjectsOf needs a relationship path".into(),
                    ));
                }
                let parts: Vec<String> = planned
                    .iter()
                    .map(|route| {
                        format!("EXISTS {{ MATCH (){}(v0) }}", chain(LADYBUG, &route.hops))
                    })
                    .collect();
                format!("({})", parts.join(" OR "))
            }
            FocusSet::Relationships(_) => {
                return Err(RenderError(
                    "relationship focus cannot be combined with node focus".into(),
                ))
            }
        })
    }

    /// Binds the rule's value variable; returns the binding and a null guard.
    fn bind_values(
        &mut self,
        body: &mut String,
        source: &ValueSource,
        name: &str,
        carry: &str,
        focus: &Binding,
    ) -> Result<(Binding, String), RenderError> {
        let path = match source {
            ValueSource::Focus => {
                body.push_str(&format!("\nWITH {carry}, v0 AS {name}"));
                let binding = Binding::Node {
                    name: name.to_owned(),
                    labels: node_labels(focus),
                };
                return Ok((binding, String::new()));
            }
            ValueSource::Path(path) => path,
        };
        let planned = routes(path)?;
        let focus_name = focus.name().to_owned();
        let absent = |body: &mut String| {
            body.push_str(&format!("\nWITH {carry}, CAST(NULL AS STRING) AS {name}"));
            Ok((
                Binding::Value {
                    expr: name.to_owned(),
                    value_type: ValueType::String,
                },
                format!("{name} IS NOT NULL AND "),
            ))
        };
        match planned.as_slice() {
            [Route {
                hops,
                property: Some(key),
            }] if hops.is_empty() => {
                let Some(column) = self.column(focus, key)? else {
                    return absent(body);
                };
                if let ValueType::List(element) = &column {
                    let list = unwind_list(&focus_name, key, &column)?;
                    body.push_str(&format!(
                        "\nUNWIND {list} AS {name}\nWITH DISTINCT {carry}, {name}"
                    ));
                    let binding = Binding::Value {
                        expr: name.to_owned(),
                        value_type: (**element).clone(),
                    };
                    return Ok((binding, String::new()));
                }
                body.push_str(&format!(
                    "\nWITH {carry}, {focus_name}.{} AS {name}",
                    ident(key)
                ));
                Ok((
                    Binding::Value {
                        expr: name.to_owned(),
                        value_type: column,
                    },
                    format!("{name} IS NOT NULL AND "),
                ))
            }
            [route] => {
                let labels = self.labels_after(&node_labels(focus), &route.hops);
                let chain = chain(LADYBUG, &route.hops);
                let Some(key) = &route.property else {
                    body.push_str(&format!(
                        "\nMATCH ({focus_name}){chain}({name})\nWITH DISTINCT {carry}, {name}"
                    ));
                    let binding = Binding::Node {
                        name: name.to_owned(),
                        labels,
                    };
                    return Ok((binding, String::new()));
                };
                let w = self.fresh();
                let end = Binding::Node {
                    name: w.clone(),
                    labels,
                };
                let Some(column) = self.column(&end, key)? else {
                    return absent(body);
                };
                let list = unwind_list(&w, key, &column)?;
                body.push_str(&format!(
                    "\nMATCH ({focus_name}){chain}({w})\nUNWIND {list} AS {name}\nWITH DISTINCT {carry}, {name}"
                ));
                Ok((
                    Binding::Value {
                        expr: name.to_owned(),
                        value_type: element_type(column),
                    },
                    String::new(),
                ))
            }
            _ if planned.iter().all(|route| route.hops.is_empty()) => {
                let Some((list, element)) = self.concatenated_lists(focus, &planned, true)? else {
                    return absent(body);
                };
                body.push_str(&format!(
                    "\nUNWIND {list} AS {name}\nWITH DISTINCT {carry}, {name}"
                ));
                Ok((
                    Binding::Value {
                        expr: name.to_owned(),
                        value_type: element,
                    },
                    String::new(),
                ))
            }
            _ => Err(RenderError(
                "alternative paths over different relationship routes are not supported on LadybugDB"
                    .into(),
            )),
        }
    }

    /// A rule-level count over one relationship route, hoisted so distinct nodes
    /// are counted exactly.
    fn hoisted_count(
        &mut self,
        condition: &Expr,
        env: &Env,
        carry: &str,
    ) -> Result<Option<String>, RenderError> {
        let Expr::Count {
            of: Var::FOCUS,
            source: ValueSource::Path(path),
            var,
            filter,
            cmp,
            bound: limit,
        } = condition
        else {
            return Ok(None);
        };
        let planned = routes(path)?;
        let [route] = planned.as_slice() else {
            return Ok(None);
        };
        if route.property.is_some() || route.hops.is_empty() {
            return Ok(None);
        }
        let focus = binding(env, Var::FOCUS)?;
        let name = var.to_string();
        let mut inner = env.clone();
        inner.insert(
            *var,
            Binding::Node {
                name: name.clone(),
                labels: self.labels_after(&node_labels(&focus), &route.hops),
            },
        );
        let where_clause = match filter {
            Some(filter) => format!(" WHERE {}", self.expr(filter, &inner)?),
            None => String::new(),
        };
        let count = self.fresh();
        Ok(Some(format!(
            "\nOPTIONAL MATCH ({}){}({name}){where_clause}\nWITH {carry}, count(DISTINCT {name}) AS {count}\nWHERE NOT (coalesce({count} {} {limit}, false))",
            focus.name(),
            chain(LADYBUG, &route.hops),
            cmp_op(*cmp),
        )))
    }

    fn expr(&mut self, expr: &Expr, env: &Env) -> Result<String, RenderError> {
        Ok(match expr {
            Expr::Bool(value) => value.to_string(),
            Expr::Not(inner) => format!("NOT ({})", self.expr(inner, env)?),
            Expr::And(items) => self.join(items, " AND ", env)?,
            Expr::Or(items) => self.join(items, " OR ", env)?,
            Expr::Xone(items) => {
                let terms = items
                    .iter()
                    .map(|item| {
                        Ok(format!(
                            "CASE WHEN {} THEN 1 ELSE 0 END",
                            self.expr(item, env)?
                        ))
                    })
                    .collect::<Result<Vec<_>, RenderError>>()?;
                format!("({}) = 1", terms.join(" + "))
            }
            Expr::All {
                of,
                source,
                var,
                test,
            } => self.all(*of, source, *var, test, env)?,
            Expr::Count {
                of,
                source,
                var,
                filter,
                cmp,
                bound: limit,
            } => self.count(*of, source, *var, filter.as_deref(), *cmp, *limit, env)?,
            Expr::Test { var, test } => {
                let value = binding(env, *var)?;
                self.test(&value, test)?
            }
            Expr::Pair {
                of,
                left,
                right,
                relation,
            } => self.pair(*of, left, right, *relation, env)?,
            Expr::Closed {
                of,
                allowed,
                allowed_relationships,
            } => self.closed(&binding(env, *of)?, allowed, allowed_relationships),
        })
    }

    fn join(&mut self, items: &[Expr], separator: &str, env: &Env) -> Result<String, RenderError> {
        let parts = items
            .iter()
            .map(|item| self.expr(item, env))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(format!("({})", parts.join(separator)))
    }

    fn all(
        &mut self,
        of: Var,
        source: &ValueSource,
        var: Var,
        test: &Expr,
        env: &Env,
    ) -> Result<String, RenderError> {
        let owner = binding(env, of)?;
        let planned = match source {
            ValueSource::Focus => {
                let mut inner = env.clone();
                inner.insert(var, owner);
                return self.expr(test, &inner);
            }
            ValueSource::Path(path) => routes(path)?,
        };
        let mut parts = Vec::new();
        for route in &planned {
            let chain = chain(LADYBUG, &route.hops);
            match (&route.property, route.hops.is_empty()) {
                (Some(key), true) => match self.column(&owner, key)? {
                    None => {}
                    Some(ValueType::List(element)) => {
                        let list =
                            self.value_list(owner.name(), key, &ValueType::List(element.clone()))?;
                        let y = self.fresh();
                        let mut inner = env.clone();
                        inner.insert(
                            var,
                            Binding::Value {
                                expr: y.clone(),
                                value_type: *element,
                            },
                        );
                        let test = self.expr(test, &inner)?;
                        parts.push(format!("all({y} IN {list} WHERE {test})"));
                    }
                    Some(scalar) => {
                        let property = format!("{}.{}", owner.name(), ident(key));
                        let mut inner = env.clone();
                        inner.insert(
                            var,
                            Binding::Value {
                                expr: property.clone(),
                                value_type: scalar,
                            },
                        );
                        let test = self.expr(test, &inner)?;
                        parts.push(format!("({property} IS NULL OR ({test}))"));
                    }
                },
                (None, _) => {
                    let y = self.fresh();
                    let mut inner = env.clone();
                    inner.insert(
                        var,
                        Binding::Node {
                            name: y.clone(),
                            labels: self.labels_after(&node_labels(&owner), &route.hops),
                        },
                    );
                    let test = self.expr(test, &inner)?;
                    parts.push(format!(
                        "NOT EXISTS {{ MATCH ({}){chain}({y}) WHERE NOT ({test}) }}",
                        owner.name()
                    ));
                }
                (Some(key), false) => {
                    let w = self.fresh();
                    let end = Binding::Node {
                        name: w.clone(),
                        labels: self.labels_after(&node_labels(&owner), &route.hops),
                    };
                    let Some(column) = self.column(&end, key)? else {
                        continue;
                    };
                    let list = self.value_list(&w, key, &column)?;
                    let y = self.fresh();
                    let mut inner = env.clone();
                    inner.insert(
                        var,
                        Binding::Value {
                            expr: y.clone(),
                            value_type: element_type(column),
                        },
                    );
                    let test = self.expr(test, &inner)?;
                    parts.push(format!(
                        "NOT EXISTS {{ MATCH ({}){chain}({w}) WHERE size(list_filter({list}, {y} -> NOT ({test}))) > 0 }}",
                        owner.name()
                    ));
                }
            }
        }
        Ok(if parts.is_empty() {
            "true".to_owned()
        } else {
            conjunction(parts)
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn count(
        &mut self,
        of: Var,
        source: &ValueSource,
        var: Var,
        filter: Option<&Expr>,
        cmp: Cmp,
        limit: u64,
        env: &Env,
    ) -> Result<String, RenderError> {
        let owner = binding(env, of)?;
        let op = cmp_op(cmp);
        let planned = match source {
            ValueSource::Focus => {
                let condition = match filter {
                    Some(filter) => {
                        let mut inner = env.clone();
                        inner.insert(var, owner);
                        self.expr(filter, &inner)?
                    }
                    None => "true".to_owned(),
                };
                return Ok(format!(
                    "(CASE WHEN {condition} THEN 1 ELSE 0 END) {op} {limit}"
                ));
            }
            ValueSource::Path(path) => routes(path)?,
        };

        if planned.iter().all(|route| route.hops.is_empty()) {
            if let [route] = planned.as_slice() {
                let key = route.property.as_deref().unwrap_or_default();
                match self.column(&owner, key)? {
                    None => return Ok(cmp.holds(0, limit).to_string()),
                    Some(ValueType::List(_)) => {}
                    Some(scalar) => {
                        let property = format!("{}.{}", owner.name(), ident(key));
                        let mut inner = env.clone();
                        inner.insert(
                            var,
                            Binding::Value {
                                expr: property.clone(),
                                value_type: scalar,
                            },
                        );
                        let condition = match filter {
                            Some(filter) => format!(" AND ({})", self.expr(filter, &inner)?),
                            None => String::new(),
                        };
                        return Ok(format!(
                            "(CASE WHEN {property} IS NOT NULL{condition} THEN 1 ELSE 0 END) {op} {limit}"
                        ));
                    }
                }
            }
            let Some((list, element)) = self.concatenated_lists(&owner, &planned, false)? else {
                return Ok(cmp.holds(0, limit).to_string());
            };
            let filtered = match filter {
                Some(filter) => {
                    let y = self.fresh();
                    let mut inner = env.clone();
                    inner.insert(
                        var,
                        Binding::Value {
                            expr: y.clone(),
                            value_type: element,
                        },
                    );
                    format!("list_filter({list}, {y} -> {})", self.expr(filter, &inner)?)
                }
                None => list,
            };
            return Ok(format!("size(list_distinct({filtered})) {op} {limit}"));
        }

        let existence = match (cmp, limit) {
            (Cmp::Ge, 1) | (Cmp::Gt, 0) => Some(true),
            (Cmp::Lt, 1) | (Cmp::Le, 0) | (Cmp::Eq, 0) => Some(false),
            _ => None,
        };
        let mut parts = Vec::new();
        for route in &planned {
            let chain = chain(LADYBUG, &route.hops);
            let labels = self.labels_after(&node_labels(&owner), &route.hops);
            let y = self.fresh();
            let (end, condition) = match &route.property {
                None => {
                    let mut inner = env.clone();
                    inner.insert(
                        var,
                        Binding::Node {
                            name: y.clone(),
                            labels,
                        },
                    );
                    let condition = filter.map(|f| self.expr(f, &inner)).transpose()?;
                    (y.clone(), condition)
                }
                Some(key) => {
                    let w = self.fresh();
                    let node = Binding::Node {
                        name: w.clone(),
                        labels,
                    };
                    let Some(column) = self.column(&node, key)? else {
                        continue;
                    };
                    let list = self.value_list(&w, key, &column)?;
                    let mut inner = env.clone();
                    inner.insert(
                        var,
                        Binding::Value {
                            expr: y.clone(),
                            value_type: element_type(column),
                        },
                    );
                    let keep = match filter {
                        Some(filter) => self.expr(filter, &inner)?,
                        None => "true".to_owned(),
                    };
                    (
                        w,
                        Some(format!("size(list_filter({list}, {y} -> {keep})) > 0")),
                    )
                }
            };
            let where_clause = condition.map_or_else(String::new, |c| format!(" WHERE {c}"));
            let subquery = format!("{{ MATCH ({}){chain}({end}){where_clause} }}", owner.name());
            match existence {
                Some(_) => parts.push(format!("EXISTS {subquery}")),
                None if planned.len() == 1
                    && route.property.is_none()
                    && route.hops.iter().all(|hop| hop.min == 1 && hop.max == 1) =>
                {
                    return Ok(format!("COUNT {subquery} {op} {limit}"));
                }
                None => {
                    return Err(RenderError(
                        "LadybugDB cannot count distinct values over this path; only at-least-one and none checks are supported"
                            .into(),
                    ))
                }
            }
        }
        Ok(match (existence, parts.is_empty()) {
            (Some(true), true) => "false".to_owned(),
            (Some(true), false) => format!("({})", parts.join(" OR ")),
            (_, true) => "true".to_owned(),
            (_, false) => format!("NOT ({})", parts.join(" OR ")),
        })
    }

    fn test(&mut self, value: &Binding, test: &ValueTest) -> Result<String, RenderError> {
        match (value, test) {
            (Binding::Node { name, .. }, ValueTest::HasLabel(labels)) => {
                Ok(format!("label({name}) IN [{}]", quoted_list(labels)))
            }
            (Binding::Node { name, .. }, ValueTest::NodeKeyIn { key, values }) => {
                if self.column(value, key)?.is_none() {
                    return Ok("false".to_owned());
                }
                Ok(format!(
                    "list_contains([{}], {name}.{})",
                    constant_list(&values.iter().collect::<Vec<_>>())?,
                    ident(key)
                ))
            }
            (Binding::Value { expr, value_type }, test) => value_test(expr, value_type, test),
            _ => Ok("false".to_owned()),
        }
    }

    fn pair(
        &mut self,
        of: Var,
        left: &ValueSource,
        right: &LpgPath,
        relation: PairRelation,
        env: &Env,
    ) -> Result<String, RenderError> {
        let owner = binding(env, of)?;
        let left = match left {
            ValueSource::Focus => Side::Itself,
            ValueSource::Path(path) => self.side(&owner, path)?,
        };
        let right = self.side(&owner, right)?;
        let node = owner.name().to_owned();
        Ok(match (left, right) {
            (_, Side::Itself) => {
                return Err(RenderError(
                    "internal error: the right side of a pair is always a path".into(),
                ))
            }
            (Side::Itself, Side::Empty) => match relation {
                PairRelation::Equals => "false".to_owned(),
                _ => "true".to_owned(),
            },
            (Side::Empty, other) | (other, Side::Empty) => match relation {
                PairRelation::Equals => side_is_empty(&node, &other),
                _ => "true".to_owned(),
            },
            (
                Side::Scalar {
                    expr: a,
                    value_type: left_type,
                    ..
                },
                Side::Scalar {
                    expr: b,
                    value_type: right_type,
                    ..
                },
            ) => {
                let comparable =
                    left_type == right_type || (is_numeric(&left_type) && is_numeric(&right_type));
                match (relation, comparable) {
                    (PairRelation::Equals, true) => {
                        format!("(({a} IS NULL AND {b} IS NULL) OR coalesce({a} = {b}, false))")
                    }
                    (PairRelation::Equals, false) => format!("({a} IS NULL AND {b} IS NULL)"),
                    (PairRelation::Disjoint, true) => {
                        format!("({a} IS NULL OR {b} IS NULL OR {a} <> {b})")
                    }
                    (PairRelation::Disjoint, false) => "true".to_owned(),
                    (PairRelation::LessThan, true) => {
                        format!("({a} IS NULL OR {b} IS NULL OR {a} < {b})")
                    }
                    (PairRelation::LessThanOrEquals, true) => {
                        format!("({a} IS NULL OR {b} IS NULL OR {a} <= {b})")
                    }
                    (_, false) => format!("({a} IS NULL OR {b} IS NULL)"),
                }
            }
            (
                left @ (Side::Scalar { .. } | Side::List { .. }),
                right @ (Side::Scalar { .. } | Side::List { .. }),
            ) => {
                let (l, left_type) = list_of(left);
                let (r, right_type) = list_of(right);
                let comparable =
                    left_type == right_type || (is_numeric(&left_type) && is_numeric(&right_type));
                match (relation, comparable) {
                    (PairRelation::Equals, true) => {
                        let x = self.fresh();
                        let y = self.fresh();
                        format!(
                            "(size(list_filter({l}, {x} -> NOT list_contains({r}, {x}))) = 0 AND size(list_filter({r}, {y} -> NOT list_contains({l}, {y}))) = 0)"
                        )
                    }
                    (PairRelation::Equals, false) => format!("(size({l}) = 0 AND size({r}) = 0)"),
                    (PairRelation::Disjoint, true) => {
                        let x = self.fresh();
                        format!("size(list_filter({l}, {x} -> list_contains({r}, {x}))) = 0")
                    }
                    (PairRelation::Disjoint, false) => "true".to_owned(),
                    (_, false) => format!("(size({l}) = 0 OR size({r}) = 0)"),
                    (_, true) => {
                        return Err(RenderError(
                            "sh:lessThan and sh:lessThanOrEquals over list properties are not supported on LadybugDB"
                                .into(),
                        ))
                    }
                }
            }
            (Side::Nodes(l), Side::Nodes(r)) => {
                let x = self.fresh();
                let y = self.fresh();
                match relation {
                    PairRelation::Equals => conjunction(vec![
                        only_within(&node, &l, &r, &x),
                        only_within(&node, &r, &l, &y),
                    ]),
                    PairRelation::Disjoint => conjunction(
                        l.iter()
                            .map(|route| {
                                format!(
                                    "NOT EXISTS {{ MATCH ({node}){}({x}) WHERE {} }}",
                                    chain(LADYBUG, &route.hops),
                                    reaches(&node, &r, &x)
                                )
                            })
                            .collect(),
                    ),
                    PairRelation::LessThan | PairRelation::LessThanOrEquals => format!(
                        "NOT ({} AND {})",
                        any_value(&node, &l),
                        any_value(&node, &r)
                    ),
                }
            }
            (Side::Itself, Side::Nodes(r)) => {
                let x = self.fresh();
                match relation {
                    PairRelation::Equals => format!(
                        "({} AND {})",
                        conjunction(
                            r.iter()
                                .map(|route| format!(
                                    "NOT EXISTS {{ MATCH ({node}){}({x}) WHERE id({x}) <> id({node}) }}",
                                    chain(LADYBUG, &route.hops)
                                ))
                                .collect()
                        ),
                        reaches(&node, &r, &node)
                    ),
                    PairRelation::Disjoint => format!("NOT {}", reaches(&node, &r, &node)),
                    PairRelation::LessThan | PairRelation::LessThanOrEquals => {
                        format!("NOT {}", any_value(&node, &r))
                    }
                }
            }
            (Side::Itself, values) => match relation {
                PairRelation::Equals => "false".to_owned(),
                PairRelation::Disjoint => "true".to_owned(),
                _ => side_is_empty(&node, &values),
            },
            (values, Side::Nodes(r)) | (Side::Nodes(r), values) => {
                let empty = side_is_empty(&node, &values);
                match relation {
                    PairRelation::Equals => format!("({empty} AND NOT {})", any_value(&node, &r)),
                    PairRelation::Disjoint => "true".to_owned(),
                    _ => format!("({empty} OR NOT {})", any_value(&node, &r)),
                }
            }
        })
    }

    fn side(&mut self, owner: &Binding, path: &LpgPath) -> Result<Side, RenderError> {
        let planned = routes(path)?;
        if let [Route {
            hops,
            property: Some(key),
        }] = planned.as_slice()
        {
            if hops.is_empty() {
                return Ok(match self.column(owner, key)? {
                    None => Side::Empty,
                    Some(column @ ValueType::List(_)) => Side::List {
                        list: self.value_list(owner.name(), key, &column)?,
                        element: element_type(column),
                    },
                    Some(scalar) => Side::Scalar {
                        expr: format!("{}.{}", owner.name(), ident(key)),
                        list: self.value_list(owner.name(), key, &scalar)?,
                        value_type: scalar,
                    },
                });
            }
        }
        if planned.iter().all(|route| route.property.is_none()) {
            return Ok(Side::Nodes(planned));
        }
        Err(RenderError(
            "on LadybugDB, sh:equals, sh:disjoint, sh:lessThan and sh:lessThanOrEquals support single properties and relationship paths only".into(),
        ))
    }

    /// `sh:closed` from declared tables: every column outside `allowed` is NULL and no
    /// relationship leaves the node through a rel table outside `allowed_relationships`.
    fn closed(
        &self,
        owner: &Binding,
        allowed: &[String],
        allowed_relationships: &[String],
    ) -> String {
        let Binding::Node { name, labels } = owner else {
            return "true".to_owned();
        };
        let extras: BTreeSet<&str> = self
            .node_types(labels)
            .into_iter()
            .flat_map(|node_type| node_type.properties.iter())
            .map(|property| property.name.as_str())
            .filter(|column| !allowed.iter().any(|key| key == column))
            .collect();
        let mut checks: Vec<String> = extras
            .iter()
            .map(|column| format!("{name}.{} IS NULL", ident(column)))
            .collect();
        let forbidden: BTreeSet<&str> = self
            .schema
            .rel_types
            .iter()
            .filter(|rel| !allowed_relationships.contains(&rel.name))
            .filter(|rel| {
                labels.is_empty() || rel.endpoints.iter().any(|e| labels.contains(&e.from))
            })
            .map(|rel| rel.name.as_str())
            .collect();
        if !forbidden.is_empty() {
            let types: Vec<String> = forbidden.iter().map(|t| format!(":{}", ident(t))).collect();
            checks.push(format!(
                "coalesce(COUNT {{ MATCH ({name})-[{}]->() }} = 0, false)",
                types.join("|")
            ));
        }
        if checks.is_empty() {
            return "true".to_owned();
        }
        format!("({})", checks.join(" AND "))
    }

    /// Adds the declared columns of the focus as a `properties` struct (`--verbose`).
    fn with_properties(&self, focus_struct: String, focus: &Binding) -> String {
        let columns: BTreeSet<&str> = match focus {
            Binding::Node { labels, .. } => self
                .node_types(labels)
                .into_iter()
                .flat_map(|node_type| node_type.properties.iter())
                .map(|property| property.name.as_str())
                .collect(),
            Binding::Relationship { rel_type, .. } => self
                .schema
                .rel_type(rel_type)
                .map(|rel| rel.properties.iter().map(|p| p.name.as_str()).collect())
                .unwrap_or_default(),
            Binding::Value { .. } => BTreeSet::new(),
        };
        let fields: Vec<String> = columns
            .into_iter()
            .filter(|column| matches!(self.column(focus, column), Ok(Some(_))))
            .map(|column| format!("{}: {}.{}", ident(column), focus.name(), ident(column)))
            .collect();
        if fields.is_empty() {
            return focus_struct;
        }
        format!(
            "{}, properties: {{{}}}}}",
            &focus_struct[..focus_struct.len() - 1],
            fields.join(", ")
        )
    }

    fn focus_struct(&self, focus: &Binding) -> Result<String, RenderError> {
        let name = focus.name();
        if let Binding::Relationship { .. } = focus {
            let start = self.key_expr(&unknown_node("s0"), self.meta.value_key)?;
            let end = self.key_expr(&unknown_node("e0"), self.meta.value_key)?;
            return Ok(format!(
                "{{type: label({name}), startKey: {start}, endKey: {end}, elementId: CAST(id({name}) AS STRING)}}"
            ));
        }
        let key = self.meta.focus_key.map_or_else(|| "NULL".to_owned(), quote);
        let key_value = self.key_expr(focus, self.meta.focus_key)?;
        Ok(format!(
            "{{label: label({name}), key: {key}, keyValue: {key_value}, elementId: CAST(id({name}) AS STRING)}}"
        ))
    }

    fn node_struct(&self, node: &Binding) -> Result<String, RenderError> {
        let name = node.name();
        let key_value = self.key_expr(node, self.meta.value_key)?;
        Ok(format!(
            "{{label: label({name}), keyValue: {key_value}, elementId: CAST(id({name}) AS STRING)}}"
        ))
    }

    /// A node's key property, or NULL when no key is configured or declared.
    fn key_expr(&self, node: &Binding, key: Option<&str>) -> Result<String, RenderError> {
        let Some(key) = key else {
            return Ok("NULL".to_owned());
        };
        Ok(match self.column(node, key)? {
            Some(_) => format!("{}.{}", node.name(), ident(key)),
            None => "NULL".to_owned(),
        })
    }

    fn message(&self, focus: &Binding, value: Option<&Binding>) -> Result<String, RenderError> {
        let text = quote(self.meta.message);
        if !self.meta.message.contains("{$this}") && !self.meta.message.contains("{?value}") {
            return Ok(text);
        }
        let this = match focus {
            Binding::Node { .. } => self.identity_text(focus, self.meta.focus_key)?,
            _ => format!("CAST(id({}) AS STRING)", focus.name()),
        };
        let value = match value {
            None => "''".to_owned(),
            Some(Binding::Value { expr, .. }) => format!("coalesce(CAST({expr} AS STRING), '')"),
            Some(node) => self.identity_text(node, self.meta.value_key)?,
        };
        Ok(format!(
            "replace(replace({text}, '{{$this}}', {this}), '{{?value}}', {value})"
        ))
    }

    fn identity_text(&self, node: &Binding, key: Option<&str>) -> Result<String, RenderError> {
        let id = format!("CAST(id({}) AS STRING)", node.name());
        Ok(match self.key_expr(node, key)?.as_str() {
            "NULL" => id,
            key_value => format!("coalesce(CAST({key_value} AS STRING), {id})"),
        })
    }

    fn details(&mut self, details: &[Detail], env: &Env) -> Result<String, RenderError> {
        if details.is_empty() {
            return Ok("CAST([] AS STRING[])".to_owned());
        }
        let cases = details
            .iter()
            .map(|detail| {
                Ok(format!(
                    "CASE WHEN NOT ({}) THEN {} END",
                    self.expr(&detail.holds, env)?,
                    quote(&detail.rule.to_string())
                ))
            })
            .collect::<Result<Vec<_>, RenderError>>()?;
        let d = self.fresh();
        Ok(format!(
            "list_filter([{}], {d} -> {d} IS NOT NULL)",
            cases.join(", ")
        ))
    }

    /// The declared type of a property on a node or relationship binding; `None`
    /// when no matched table declares it.
    fn column(&self, owner: &Binding, key: &str) -> Result<Option<ValueType>, RenderError> {
        let types: Vec<&ValueType> = match owner {
            Binding::Node { labels, .. } => self
                .node_types(labels)
                .into_iter()
                .filter_map(|node_type| node_type.property(key))
                .map(|property| &property.value_type)
                .collect(),
            Binding::Relationship { rel_type, .. } => self
                .schema
                .rel_type(rel_type)
                .and_then(|rel| rel.property(key))
                .map(|property| &property.value_type)
                .into_iter()
                .collect(),
            Binding::Value { .. } => {
                return Err(RenderError(
                    "internal error: property of a property value".into(),
                ))
            }
        };
        match types.split_first() {
            None => Ok(None),
            Some((first, rest)) if rest.iter().all(|other| other == first) => {
                Ok(Some((*first).clone()))
            }
            Some(_) => Err(RenderError(format!(
                "property `{key}` has different column types across the matched tables; LadybugDB needs one type"
            ))),
        }
    }

    fn node_types(&self, labels: &[String]) -> Vec<&NodeType> {
        if labels.is_empty() {
            self.schema.node_types.iter().collect()
        } else {
            labels
                .iter()
                .filter_map(|label| self.schema.node_type(label))
                .collect()
        }
    }

    /// Non-null values of a property as a typed list.
    fn value_list(
        &mut self,
        base: &str,
        key: &str,
        column: &ValueType,
    ) -> Result<String, RenderError> {
        let property = format!("{base}.{}", ident(key));
        match column {
            ValueType::List(element) => {
                let y = self.fresh();
                Ok(format!(
                    "list_filter(coalesce({property}, CAST([] AS {}[])), {y} -> {y} IS NOT NULL)",
                    ddl(element)?
                ))
            }
            scalar => Ok(format!(
                "CASE WHEN {property} IS NULL THEN CAST([] AS {}[]) ELSE [{property}] END",
                ddl(scalar)?
            )),
        }
    }

    /// Concatenated value lists of several properties of one owner.
    fn concatenated_lists(
        &mut self,
        owner: &Binding,
        planned: &[Route],
        unwind: bool,
    ) -> Result<Option<(String, ValueType)>, RenderError> {
        let mut combined: Option<(String, ValueType)> = None;
        for route in planned {
            let key = route.property.as_deref().unwrap_or_default();
            let Some(column) = self.column(owner, key)? else {
                continue;
            };
            let list = if unwind {
                concatenable_list(owner.name(), key, &column)?
            } else {
                self.value_list(owner.name(), key, &column)?
            };
            let element = element_type(column);
            combined = Some(match combined {
                None => (list, element),
                Some((previous, previous_type)) if previous_type == element => {
                    (format!("list_concat({previous}, {list})"), element)
                }
                Some(_) => {
                    return Err(RenderError(
                        "alternative properties have different column types; LadybugDB lists need one type"
                            .into(),
                    ))
                }
            });
        }
        Ok(combined)
    }

    /// Tables reachable from `from` over the hops, following declared endpoints
    /// (empty when unknown).
    fn labels_after(&self, from: &[String], hops: &[Hop]) -> Vec<String> {
        let mut current: BTreeSet<String> = from.iter().cloned().collect();
        for hop in hops {
            let step = |labels: &BTreeSet<String>| -> BTreeSet<String> {
                hop.types
                    .iter()
                    .filter_map(|rel_type| self.schema.rel_type(rel_type))
                    .flat_map(|rel| rel.endpoints.iter())
                    .filter_map(|endpoint| {
                        let (near, far) = match hop.direction {
                            Direction::Out => (&endpoint.from, &endpoint.to),
                            Direction::In => (&endpoint.to, &endpoint.from),
                        };
                        (labels.is_empty() || labels.contains(near)).then(|| far.clone())
                    })
                    .collect()
            };
            if hop.min == 1 && hop.max == 1 {
                current = step(&current);
            } else {
                let mut reached = if hop.min == 0 {
                    current.clone()
                } else {
                    BTreeSet::new()
                };
                let mut frontier = current;
                loop {
                    let next: BTreeSet<String> =
                        step(&frontier).difference(&reached).cloned().collect();
                    if next.is_empty() {
                        break;
                    }
                    reached.extend(next.iter().cloned());
                    frontier = next;
                }
                current = reached;
            }
            if current.is_empty() {
                return Vec::new();
            }
        }
        current.into_iter().collect()
    }

    fn fresh(&mut self) -> String {
        self.names += 1;
        format!("x{}", self.names)
    }
}

fn value_test(expr: &str, value_type: &ValueType, test: &ValueTest) -> Result<String, RenderError> {
    Ok(match test {
        ValueTest::Datatype(check) => match check.column_status(value_type) {
            ColumnStatus::Guaranteed => "true".to_owned(),
            ColumnStatus::Contradicted => "false".to_owned(),
            ColumnStatus::NeedsRangeCheck => {
                let (min, max) = check.range.unwrap_or((i128::MIN, i128::MAX));
                let mut bounds = Vec::new();
                if min > i128::from(i64::MIN) {
                    bounds.push(format!("{expr} >= {min}"));
                }
                if max < i128::from(i64::MAX) {
                    bounds.push(format!("{expr} <= {max}"));
                }
                if bounds.is_empty() {
                    "true".to_owned()
                } else {
                    format!("coalesce({}, false)", bounds.join(" AND "))
                }
            }
            ColumnStatus::NeedsCheck => {
                return Err(RenderError(
                    "values of ANY-typed columns cannot be datatype-checked on LadybugDB".into(),
                ))
            }
        },
        ValueTest::Compare { cmp, bound } if comparable(value_type, bound) => {
            format!(
                "coalesce({expr} {} {}, false)",
                cmp_op(*cmp),
                constant(LADYBUG, bound)?
            )
        }
        ValueTest::Compare { .. } => "false".to_owned(),
        ValueTest::Length { cmp, bound } => match value_type {
            ValueType::String => format!("coalesce(size({expr}) {} {bound}, false)", cmp_op(*cmp)),
            other if is_integer(other) => {
                format!(
                    "coalesce(size(CAST({expr} AS STRING)) {} {bound}, false)",
                    cmp_op(*cmp)
                )
            }
            _ => "false".to_owned(),
        },
        ValueTest::Matches(regex) => {
            if regex.has_backreference {
                return Err(RenderError(
                    "back-references in sh:pattern are not supported on LadybugDB (RE2)".into(),
                ));
            }
            let pattern = quote(&regex_pattern(regex));
            match value_type {
                ValueType::String => format!("regexp_matches({expr}, {pattern})"),
                other if is_integer(other) => {
                    format!("regexp_matches(CAST({expr} AS STRING), {pattern})")
                }
                _ => "false".to_owned(),
            }
        }
        ValueTest::In(constants) => {
            let kept: Vec<&Constant> = constants
                .iter()
                .filter(|c| same_kind(value_type, c))
                .collect();
            if kept.is_empty() {
                "false".to_owned()
            } else {
                format!("list_contains([{}], {expr})", constant_list(&kept)?)
            }
        }
        ValueTest::HasLabel(_) | ValueTest::NodeKeyIn { .. } => "false".to_owned(),
    })
}

/// RE2 regex with substring semantics: `regexp_matches` already searches.
// @lat: [[semantics#Regex Translation]]
fn regex_pattern(regex: &XsdRegex) -> String {
    let mut flags = String::new();
    if regex.flags.case_insensitive {
        flags.push('i');
    }
    if regex.flags.multi_line {
        flags.push('m');
    }
    if regex.flags.dot_all {
        flags.push('s');
    }
    if flags.is_empty() {
        regex.pattern.clone()
    } else {
        format!("(?{flags}){}", regex.pattern)
    }
}

fn ddl(value_type: &ValueType) -> Result<String, RenderError> {
    Ok(match value_type {
        ValueType::String => "STRING".into(),
        ValueType::Int64 => "INT64".into(),
        ValueType::Int32 => "INT32".into(),
        ValueType::Int16 => "INT16".into(),
        ValueType::Int8 => "INT8".into(),
        ValueType::UInt64 => "UINT64".into(),
        ValueType::UInt32 => "UINT32".into(),
        ValueType::UInt16 => "UINT16".into(),
        ValueType::UInt8 => "UINT8".into(),
        ValueType::Double => "DOUBLE".into(),
        ValueType::Float => "FLOAT".into(),
        ValueType::Boolean => "BOOL".into(),
        ValueType::Date => "DATE".into(),
        ValueType::LocalDateTime => "TIMESTAMP".into(),
        ValueType::ZonedDateTime => "TIMESTAMP_TZ".into(),
        ValueType::Duration => "INTERVAL".into(),
        ValueType::Blob => "BLOB".into(),
        ValueType::List(element) => format!("{}[]", ddl(element)?),
        other => {
            return Err(RenderError(format!(
                "{other} columns have no LadybugDB list type for value sets"
            )))
        }
    })
}

/// A property's values for `UNWIND`. LadybugDB leaks list elements across rows when
/// unwinding `list_filter(coalesce(…))`, so list columns are unwound raw (a NULL list
/// yields no rows) and NULL elements are dropped by the row guard.
fn unwind_list(base: &str, key: &str, column: &ValueType) -> Result<String, RenderError> {
    let property = format!("{base}.{}", ident(key));
    match column {
        ValueType::List(_) => Ok(property),
        scalar => Ok(format!(
            "CASE WHEN {property} IS NULL THEN CAST([] AS {}[]) ELSE [{property}] END",
            ddl(scalar)?
        )),
    }
}

/// Like [`unwind_list`], but never NULL, so lists can be concatenated before unwinding.
fn concatenable_list(base: &str, key: &str, column: &ValueType) -> Result<String, RenderError> {
    let property = format!("{base}.{}", ident(key));
    match column {
        ValueType::List(element) => Ok(format!(
            "coalesce({property}, CAST([] AS {}[]))",
            ddl(element)?
        )),
        scalar => unwind_list(base, key, scalar),
    }
}

fn element_type(column: ValueType) -> ValueType {
    match column {
        ValueType::List(element) => *element,
        scalar => scalar,
    }
}

fn node_labels(binding: &Binding) -> Vec<String> {
    match binding {
        Binding::Node { labels, .. } => labels.clone(),
        _ => Vec::new(),
    }
}

fn unknown_node(name: &str) -> Binding {
    Binding::Node {
        name: name.to_owned(),
        labels: Vec::new(),
    }
}

fn list_of(side: Side) -> (String, ValueType) {
    match side {
        Side::Scalar {
            list, value_type, ..
        } => (list, value_type),
        Side::List { list, element } => (list, element),
        _ => ("CAST([] AS STRING[])".to_owned(), ValueType::String),
    }
}

fn side_is_empty(node: &str, side: &Side) -> String {
    match side {
        Side::Itself => "false".to_owned(),
        Side::Empty => "true".to_owned(),
        Side::Scalar { expr, .. } => format!("{expr} IS NULL"),
        Side::List { list, .. } => format!("size({list}) = 0"),
        Side::Nodes(planned) => format!("NOT {}", any_value(node, planned)),
    }
}

fn only_within(node: &str, from: &[Route], other: &[Route], x: &str) -> String {
    conjunction(
        from.iter()
            .map(|route| {
                format!(
                    "NOT EXISTS {{ MATCH ({node}){}({x}) WHERE NOT {} }}",
                    chain(LADYBUG, &route.hops),
                    reaches(node, other, x)
                )
            })
            .collect(),
    )
}

fn reaches(node: &str, planned: &[Route], target: &str) -> String {
    let parts: Vec<String> = planned
        .iter()
        .map(|route| {
            format!(
                "EXISTS {{ MATCH ({node}){}({target}) }}",
                chain(LADYBUG, &route.hops)
            )
        })
        .collect();
    format!("({})", parts.join(" OR "))
}

fn any_value(node: &str, planned: &[Route]) -> String {
    let parts: Vec<String> = planned
        .iter()
        .map(|route| {
            format!(
                "EXISTS {{ MATCH ({node}){}() }}",
                chain(LADYBUG, &route.hops)
            )
        })
        .collect();
    format!("({})", parts.join(" OR "))
}

fn conjunction(mut parts: Vec<String>) -> String {
    if parts.len() == 1 {
        parts.remove(0)
    } else {
        format!("({})", parts.join(" AND "))
    }
}

fn quoted_list(items: &[String]) -> String {
    items
        .iter()
        .map(|item| quote(item))
        .collect::<Vec<_>>()
        .join(", ")
}

fn constant_list(constants: &[&Constant]) -> Result<String, RenderError> {
    Ok(constants
        .iter()
        .map(|c| constant(LADYBUG, c))
        .collect::<Result<Vec<_>, _>>()?
        .join(", "))
}

fn is_integer(value_type: &ValueType) -> bool {
    matches!(
        value_type,
        ValueType::Int64
            | ValueType::Int32
            | ValueType::Int16
            | ValueType::Int8
            | ValueType::UInt64
            | ValueType::UInt32
            | ValueType::UInt16
            | ValueType::UInt8
    )
}

fn is_numeric(value_type: &ValueType) -> bool {
    is_integer(value_type)
        || matches!(
            value_type,
            ValueType::Double | ValueType::Float | ValueType::Decimal
        )
}

/// Whether ordering comparisons between the column and the constant are defined.
fn comparable(value_type: &ValueType, constant: &Constant) -> bool {
    match constant {
        Constant::String(_) => *value_type == ValueType::String,
        Constant::Integer(_) | Constant::Decimal(_) | Constant::Double(_) => is_numeric(value_type),
        Constant::Boolean(_) => *value_type == ValueType::Boolean,
        Constant::Date(_) => *value_type == ValueType::Date,
        Constant::DateTime(lexical) => {
            let zoned = lexical.ends_with('Z') || {
                let bytes = lexical.as_bytes();
                bytes.len() >= 6
                    && matches!(bytes[bytes.len() - 6], b'+' | b'-')
                    && bytes[bytes.len() - 3] == b':'
            };
            *value_type
                == if zoned {
                    ValueType::ZonedDateTime
                } else {
                    ValueType::LocalDateTime
                }
        }
        Constant::Duration(_) => *value_type == ValueType::Duration,
        Constant::Time(_) => false,
    }
}

/// Whether the constant can be equal to a value of the column (for `sh:in`).
fn same_kind(value_type: &ValueType, constant: &Constant) -> bool {
    match constant {
        Constant::Integer(_) => is_integer(value_type),
        Constant::Decimal(_) | Constant::Double(_) => matches!(
            value_type,
            ValueType::Double | ValueType::Float | ValueType::Decimal
        ),
        other => comparable(value_type, other),
    }
}

fn cmp_op(cmp: Cmp) -> &'static str {
    match cmp {
        Cmp::Lt => "<",
        Cmp::Le => "<=",
        Cmp::Eq => "=",
        Cmp::Ge => ">=",
        Cmp::Gt => ">",
    }
}

fn binding(env: &Env, var: Var) -> Result<Binding, RenderError> {
    env.get(&var)
        .cloned()
        .ok_or_else(|| RenderError(format!("internal error: unbound variable {var}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::Shapes;
    use crate::hierarchy::ClassHierarchy;
    use crate::load::ShapesGraph;
    use crate::lower::{lower, Inputs, LowerOptions};
    use crate::mapping::{ResolveOptions, Resolver};

    const PREFIXES: &str = "@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix ex: <http://example.org/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix s2c: <https://w3id.org/shacl2cypher#> .
";

    /// Same shapes as the Neo4j renderer tests, rendered against a typed schema.
    const SHAPES: &str = "ex:PersonShape sh:targetClass ex:Person ;
    sh:closed true ;
    sh:ignoredProperties ( ex:id ex:email ex:phone ) ;
    sh:property [ sh:path ex:name ; sh:minCount 1 ; sh:maxCount 1 ; sh:datatype xsd:string ;
                  sh:minLength 2 ; sh:pattern \"^[A-Z]\" ; sh:flags \"i\" ;
                  sh:message \"{$this} has a bad name {?value}\" ] ,
                [ sh:path ex:age ; sh:datatype xsd:short ; sh:minInclusive 0 ; sh:maxInclusive 150 ] ,
                [ sh:path ex:tags ; sh:in ( \"a\" \"b\" ) ] ,
                [ sh:path ex:start ; sh:lessThan ex:end ] ,
                [ sh:path ex:end ] ,
                [ sh:path ex:worksFor ; sh:class ex:Company ; sh:maxCount 1 ] ,
                [ sh:path ex:address ; sh:node ex:AddressShape ] ,
                [ sh:path [ sh:oneOrMorePath ex:knows ] ; sh:class ex:Person ] ,
                [ sh:path [ sh:alternativePath ( ex:email ex:phone ) ] ; sh:minCount 1 ] ;
    sh:or ( [ sh:path ex:email ; sh:minCount 1 ] [ sh:path ex:phone ; sh:minCount 1 ] ) .
ex:AddressShape sh:property [ sh:path ex:zip ; sh:minCount 1 ; sh:pattern \"^[0-9]{5}$\" ] .
ex:KnowsShape s2c:targetRelationship \"KNOWS\" ;
    sh:property [ sh:path ex:since ; sh:datatype xsd:date ; sh:maxCount 1 ] .
ex:CompanyShape sh:targetClass ex:Company ;
    sh:property [ sh:path [ sh:inversePath ex:worksFor ] ; sh:minCount 1 ] .
";

    const SCHEMA: &str = r#"{
        "nodeTypes": [
            {"name": "Person", "properties": [
                {"name": "id", "type": "STRING"}, {"name": "name", "type": "STRING"},
                {"name": "age", "type": "INT64"}, {"name": "tags", "type": "LIST<STRING>"},
                {"name": "start", "type": "INT64"}, {"name": "end", "type": "INT64"},
                {"name": "email", "type": "STRING"}, {"name": "phone", "type": "STRING"},
                {"name": "extra", "type": "BOOLEAN"}
            ]},
            {"name": "Company", "properties": [{"name": "id", "type": "STRING"}]},
            {"name": "Address", "properties": [
                {"name": "id", "type": "STRING"}, {"name": "zip", "type": "STRING"}
            ]}
        ],
        "relTypes": [
            {"name": "WORKS_FOR", "endpoints": [
                {"from": "Person", "to": "Company"}, {"from": "Person", "to": "Person"}
            ]},
            {"name": "ADDRESS", "endpoints": [{"from": "Person", "to": "Address"}]},
            {"name": "KNOWS", "endpoints": [{"from": "Person", "to": "Person"}],
             "properties": [{"name": "since", "type": "DATE"}]}
        ]
    }"#;

    fn compile(body: &str) -> Vec<Rule> {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shapes.ttl");
        std::fs::write(&path, format!("{PREFIXES}{body}")).unwrap();
        let graph = ShapesGraph::load(&[path]).unwrap();
        let shapes = Shapes::from_graph(&graph).unwrap();
        let hierarchy = ClassHierarchy::build(&[&graph]).unwrap();
        let resolver = Resolver::new(&graph, &shapes, None, ResolveOptions::default());
        let inputs = Inputs {
            graph: &graph,
            shapes: &shapes,
            resolver: &resolver,
            hierarchy: &hierarchy,
            checker: None,
        };
        let options = LowerOptions {
            node_key: Some("id".into()),
            dialect_max_path_depth: LADYBUG.max_path_depth(),
            ..LowerOptions::default()
        };
        lower(inputs, &options).unwrap().rules
    }

    fn render_rule(rule: &Rule) -> Result<Rendered, RenderError> {
        let schema = SchemaSnapshot::from_json(SCHEMA).unwrap();
        let segments = &rule.id.0;
        let name = rule.id.to_string();
        let constraint = segments.last().unwrap();
        let path = (segments.len() > 2).then(|| segments[1..segments.len() - 1].join("/"));
        let default_message = format!("{constraint} is violated");
        let message = rule
            .messages
            .first()
            .map_or(default_message.as_str(), |m| m.value());
        let meta = RuleMeta {
            name: &name,
            shape: &segments[0],
            path: path.as_deref(),
            constraint,
            severity: "Violation",
            message,
            focus_key: Some("id"),
            value_key: Some("id"),
            verbose: false,
        };
        render(rule, &meta, &schema)
    }

    fn rendered(rules: &[Rule], id: &str) -> Rendered {
        let rule = rules
            .iter()
            .find(|rule| rule.id.to_string() == id)
            .unwrap_or_else(|| panic!("no rule {id}"));
        render_rule(rule).unwrap()
    }

    #[test]
    fn renders_every_compiled_rule_without_write_clauses() {
        let rules = compile(SHAPES);
        let dump = std::env::var_os("S2C_RENDER_DUMP_LADYBUG").map(std::path::PathBuf::from);
        for (index, rule) in rules.iter().enumerate() {
            let queries = render_rule(rule).unwrap_or_else(|e| panic!("{}: {e}", rule.id));
            for query in [&queries.detail, &queries.summary] {
                for clause in ["CREATE ", "MERGE ", " SET ", "DELETE ", "REMOVE "] {
                    assert!(!query.contains(clause), "{}: {query}", rule.id);
                }
            }
            assert!(queries.detail.ends_with("LIMIT $limit"));
            assert!(queries
                .summary
                .ends_with("list_slice(coalesce(samples, []), 1, $sampleSize) AS sample"));
            if let Some(dir) = &dump {
                std::fs::create_dir_all(dir).unwrap();
                let stem = format!("{index:02}");
                std::fs::write(dir.join(format!("{stem}.id")), rule.id.to_string()).unwrap();
                std::fs::write(dir.join(format!("{stem}.detail.cypher")), &queries.detail).unwrap();
                std::fs::write(dir.join(format!("{stem}.summary.cypher")), &queries.summary)
                    .unwrap();
            }
        }
    }

    #[test]
    fn decides_value_types_from_columns() {
        let rules = compile(SHAPES);
        let name = rendered(&rules, "ex:PersonShape/ex:name/sh:datatype");
        assert!(name
            .detail
            .contains("WITH v0, v0.`name` AS v1\nWHERE v1 IS NOT NULL AND NOT (true)"));
        let age = rendered(&rules, "ex:PersonShape/ex:age/sh:datatype");
        assert!(age
            .detail
            .contains("NOT (coalesce(v1 >= -32768 AND v1 <= 32767, false))"));
        let pattern = rendered(&rules, "ex:PersonShape/ex:name/sh:pattern");
        assert!(pattern.detail.contains("regexp_matches(v1, '(?i)^[A-Z]')"));
        let tags = rendered(&rules, "ex:PersonShape/ex:tags/sh:in");
        assert!(tags.detail.contains(
            "UNWIND v0.`tags` AS v1
WITH DISTINCT v0, v1
WHERE v1 IS NOT NULL AND NOT"
        ));
        assert!(tags.detail.contains("list_contains(['a', 'b'], v1)"));
    }

    #[test]
    fn hoists_relationship_counts_for_exact_distinct_counts() {
        let rules = compile(SHAPES);
        let inverse = rendered(&rules, "ex:CompanyShape/^ex:worksFor/sh:minCount");
        assert!(inverse.detail.contains(
            "OPTIONAL MATCH (v0)<-[:`WORKS_FOR`]-(v1)\nWITH v0, count(DISTINCT v1) AS x1\nWHERE NOT (coalesce(x1 >= 1, false))"
        ));
        let name = rendered(&rules, "ex:PersonShape/ex:name/sh:minCount");
        assert!(name
            .detail
            .contains("(CASE WHEN v0.`name` IS NOT NULL THEN 1 ELSE 0 END) >= 1"));
    }

    #[test]
    fn renders_closed_pairs_labels_and_nested_shapes() {
        let rules = compile(SHAPES);
        let closed = rendered(&rules, "ex:PersonShape/sh:closed");
        assert!(closed.detail.contains(
            "WHERE NOT ((v0.`extra` IS NULL AND coalesce(COUNT { MATCH (v0)-[:`KNOWS`]->() } = 0, false)))"
        ));
        let pair = rendered(&rules, "ex:PersonShape/ex:start/sh:lessThan");
        assert!(pair
            .detail
            .contains("(v0.`start` IS NULL OR v0.`end` IS NULL OR v0.`start` < v0.`end`)"));
        let class = rendered(&rules, "ex:PersonShape/ex:worksFor/sh:class");
        assert!(class.detail.contains("NOT (label(v1) IN ['Company'])"));
        let node = rendered(&rules, "ex:PersonShape/ex:address/sh:node");
        assert!(node
            .detail
            .contains("regexp_matches(v1.`zip`, '^[0-9]{5}$')"));
        let since = rendered(&rules, "ex:KnowsShape/ex:since/sh:datatype");
        assert!(since.detail.starts_with("MATCH (s0)-[v0:`KNOWS`]->(e0)"));
    }

    #[test]
    fn never_emits_bare_comparisons_under_not() {
        for rule in compile(SHAPES) {
            let queries = render_rule(&rule).unwrap();
            for query in [&queries.detail, &queries.summary] {
                let findings = super::super::lint::bare_comparisons_under_not(query);
                assert!(findings.is_empty(), "{}: {findings:?}\n{query}", rule.id);
            }
        }
    }

    #[test]
    fn rejects_constructs_ladybugdb_cannot_express() {
        let back_reference =
            compile("ex:S sh:targetClass ex:Person ;\n    sh:property [ sh:path ex:name ; sh:pattern \"(a)\\\\1\" ] .\n");
        let message = render_rule(&back_reference[0]).unwrap_err().to_string();
        assert!(message.contains("back-references"), "{message}");

        let list_order =
            compile("ex:S sh:targetClass ex:Person ;\n    sh:property [ sh:path ex:tags ; sh:lessThan ex:name ] .\n");
        let message = render_rule(&list_order[0]).unwrap_err().to_string();
        assert!(message.contains("over list properties"), "{message}");
    }
}
