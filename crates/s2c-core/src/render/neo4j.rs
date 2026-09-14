//! Neo4j 5 Cypher renderer (no APOC).

use std::collections::HashMap;

use super::{
    chain, constant, ident, quote, routes, Dialect, RenderError, Rendered, Route, RuleMeta,
};
use crate::datatypes::DatatypeCheck;
use crate::ir::{
    Cmp, Detail, Expr, FocusSet, PairRelation, Rule, RuleStatus, ValueSource, ValueTest, Var,
    Violation,
};
use crate::mapping::LpgPath;
use crate::schema::ValueType;
use crate::xsd_regex::XsdRegex;

const NEO4J: Dialect = Dialect::Neo4j;

/// Cypher expression bound to each IR variable.
type Env = HashMap<Var, String>;

/// Renders a compiled rule as a detail query and a summary query.
// @lat: [[dialects#Dialect Backends#Neo4j]]
pub fn render(rule: &Rule, meta: &RuleMeta<'_>) -> Result<Rendered, RenderError> {
    Renderer { meta, names: 0 }.rule(rule)
}

/// Whether Neo4j would decode part of `name` as a `\uXXXX` escape inside backticks;
/// such names cannot be written as identifiers.
// @lat: [[dialects#Literals and Identifiers]]
pub fn unrepresentable_identifier(name: &str) -> bool {
    name.as_bytes().windows(6).any(|w| {
        w[0] == b'\\' && matches!(w[1], b'u' | b'U') && w[2..].iter().all(u8::is_ascii_hexdigit)
    })
}

/// Backticked identifiers of a query, skipping string literals.
fn backticked(query: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut chars = query.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' | '"' => {
                while let Some(inner) = chars.next() {
                    if inner == '\\' {
                        chars.next();
                    } else if inner == c {
                        break;
                    }
                }
            }
            '`' => {
                let mut name = String::new();
                while let Some(inner) = chars.next() {
                    if inner == '`' {
                        if chars.peek() == Some(&'`') {
                            chars.next();
                            name.push('`');
                        } else {
                            break;
                        }
                    } else {
                        name.push(inner);
                    }
                }
                names.push(name);
            }
            _ => {}
        }
    }
    names
}

struct Renderer<'m> {
    meta: &'m RuleMeta<'m>,
    names: u32,
}

/// One side of a pair constraint.
enum Side {
    /// The focus node itself.
    Itself,
    /// A list expression of property values (may contain nulls).
    Values(String),
    /// Node-valued routes.
    Nodes(Vec<Route>),
}

impl Renderer<'_> {
    fn rule(&mut self, rule: &Rule) -> Result<Rendered, RenderError> {
        if rule.status != RuleStatus::Compiled {
            return Err(RenderError(format!(
                "rule {} is not rendered: status {:?}",
                rule.id, rule.status
            )));
        }
        let relationship = matches!(rule.focus.as_slice(), [FocusSet::Relationships(_)]);
        let carry = if relationship { "s0, v0, e0" } else { "v0" };
        let mut env = Env::new();
        env.insert(Var::FOCUS, "v0".to_owned());

        let mut body = self.focus_match(&rule.focus)?;
        let (value, row_env) = match &rule.violation {
            Violation::PerFocus { condition, .. } => {
                let condition = self.expr(condition, &env)?;
                body.push_str(&format!("\nWITH {carry}\nWHERE NOT ({condition})"));
                (None, env)
            }
            Violation::PerValue {
                source, var, test, ..
            } => {
                let name = var.to_string();
                let nodes = self.bind_values(&mut body, source, &name, carry)?;
                let mut row_env = env.clone();
                row_env.insert(*var, name.clone());
                let test = self.expr(test, &row_env)?;
                let guard = if nodes {
                    String::new()
                } else {
                    format!("{name} IS NOT NULL AND ")
                };
                body.push_str(&format!("\nWHERE {guard}NOT ({test})"));
                (Some((name, nodes)), row_env)
            }
        };

        let meta = self.meta;
        let focus = self.focus_map(&rule.focus, relationship);
        let focus = if meta.verbose {
            format!(
                "{}, properties: properties(v0)}}",
                &focus[..focus.len() - 1]
            )
        } else {
            focus
        };
        let value_expr = match &value {
            None => "null".to_owned(),
            Some((name, false)) => name.clone(),
            Some((name, true)) => self.node_map(name),
        };
        let message = self.message(relationship, value.as_ref());
        let details = self.details(&rule.details, &row_env)?;
        let path = meta.path.map_or_else(|| "null".to_owned(), quote);
        let detail = format!(
            "{body}\nRETURN {} AS ruleId, {} AS shape, {path} AS path, {} AS `constraint`, {} AS severity,\n       {focus} AS focus, {value_expr} AS value, {message} AS message, {details} AS details\nLIMIT coalesce($limit, 9223372036854775807)",
            quote(meta.name),
            quote(meta.shape),
            quote(meta.constraint),
            quote(meta.severity),
        );
        let summary = format!(
            "{body}\nWITH count(*) AS violationCount, collect({focus})[0..$sampleSize] AS sample\nRETURN {} AS ruleId, {} AS severity, violationCount, sample",
            quote(meta.name),
            quote(meta.severity),
        );
        if let Some(name) = backticked(&detail)
            .into_iter()
            .find(|name| unrepresentable_identifier(name))
        {
            return Err(RenderError(format!(
                "identifier `{name}` contains a `\\uXXXX` sequence, which Neo4j decodes inside backticks"
            )));
        }
        Ok(Rendered { detail, summary })
    }

    fn focus_match(&mut self, focus: &[FocusSet]) -> Result<String, RenderError> {
        match focus {
            [FocusSet::Relationships(rel_type)] => {
                Ok(format!("MATCH (s0)-[v0:{}]->(e0)", ident(rel_type)))
            }
            [FocusSet::Labels(labels)] => Ok(format!("MATCH (v0:{})", label_expr(labels))),
            sets if !sets.is_empty()
                && !sets
                    .iter()
                    .any(|set| matches!(set, FocusSet::Relationships(_))) =>
            {
                let conditions = sets
                    .iter()
                    .map(|set| self.focus_condition(set))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(format!("MATCH (v0)\nWHERE {}", conditions.join(" OR ")))
            }
            _ => Err(RenderError(
                "a rule needs node focus sets or a single relationship focus".into(),
            )),
        }
    }

    fn focus_condition(&mut self, set: &FocusSet) -> Result<String, RenderError> {
        Ok(match set {
            FocusSet::Labels(labels) => format!("v0:{}", label_expr(labels)),
            FocusSet::SubjectsOf(path) => {
                let parts: Vec<String> = routes(path)?
                    .iter()
                    .map(|route| {
                        let chain = chain(NEO4J, &route.hops);
                        match (&route.property, route.hops.is_empty()) {
                            (Some(key), true) => format!("v0.{} IS NOT NULL", ident(key)),
                            (None, _) => format!("EXISTS {{ MATCH (v0){chain}() }}"),
                            (Some(key), false) => {
                                let x = self.fresh();
                                format!(
                                    "EXISTS {{ MATCH (v0){chain}({x}) WHERE {x}.{} IS NOT NULL }}",
                                    ident(key)
                                )
                            }
                        }
                    })
                    .collect();
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
                    .map(|route| format!("EXISTS {{ MATCH (){}(v0) }}", chain(NEO4J, &route.hops)))
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

    /// Binds the rule's value variable; returns whether values are nodes.
    fn bind_values(
        &mut self,
        body: &mut String,
        source: &ValueSource,
        name: &str,
        carry: &str,
    ) -> Result<bool, RenderError> {
        let path = match source {
            ValueSource::Focus => {
                body.push_str(&format!("\nWITH {carry}, v0 AS {name}"));
                return Ok(true);
            }
            ValueSource::Path(path) => path,
        };
        let planned = routes(path)?;
        let nodes = planned.iter().all(|route| route.property.is_none());
        match planned.as_slice() {
            [route] => body.push_str(&self.route_clause(route, "v0", name)),
            _ if planned.iter().all(|route| route.hops.is_empty()) => {
                let lists: Vec<String> = planned
                    .iter()
                    .map(|route| values("v0", route.property.as_deref().unwrap_or_default()))
                    .collect();
                body.push_str(&format!("\nUNWIND {} AS {name}", lists.join(" + ")));
            }
            _ => {
                let branches: Vec<String> = planned
                    .iter()
                    .map(|route| {
                        format!(
                            "WITH v0{} RETURN {name}",
                            self.route_clause(route, "v0", name).replace('\n', " ")
                        )
                    })
                    .collect();
                body.push_str(&format!(
                    "\nCALL {{\n  {}\n}}",
                    branches.join("\n  UNION\n  ")
                ));
            }
        }
        body.push_str(&format!("\nWITH DISTINCT {carry}, {name}"));
        Ok(nodes)
    }

    /// Clauses binding `name` to the values of one route from `start`.
    fn route_clause(&mut self, route: &Route, start: &str, name: &str) -> String {
        let chain = chain(NEO4J, &route.hops);
        match (&route.property, route.hops.is_empty()) {
            (Some(key), true) => format!("\nUNWIND {} AS {name}", values(start, key)),
            (None, _) => format!("\nMATCH ({start}){chain}({name})"),
            (Some(key), false) => {
                let x = self.fresh();
                format!(
                    "\nMATCH ({start}){chain}({x})\nUNWIND {} AS {name}",
                    values(&x, key)
                )
            }
        }
    }

    fn expr(&mut self, expr: &Expr, env: &Env) -> Result<String, RenderError> {
        Ok(match expr {
            Expr::Bool(value) => value.to_string(),
            Expr::Not(inner) => format!("NOT ({})", self.expr(inner, env)?),
            Expr::And(items) => self.join(items, " AND ", env)?,
            Expr::Or(items) => self.join(items, " OR ", env)?,
            Expr::Xone(items) => {
                let parts = items
                    .iter()
                    .map(|item| self.expr(item, env))
                    .collect::<Result<Vec<_>, _>>()?;
                let x = self.fresh();
                format!("size([{x} IN [{}] WHERE {x}]) = 1", parts.join(", "))
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
            Expr::Test { var, test } => value_test(&bound(env, *var)?, test)?,
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
            } => {
                let node = bound(env, *of)?;
                let x = self.fresh();
                let y = self.fresh();
                let r = self.fresh();
                let allowed: Vec<String> = allowed.iter().map(|key| quote(key)).collect();
                let relationships: Vec<String> =
                    allowed_relationships.iter().map(|t| quote(t)).collect();
                format!(
                    "(all({x} IN keys({node}) WHERE {x} IN [{}]) AND all({y} IN [({node})-[{r}]->() | type({r})] WHERE {y} IN [{}]))",
                    allowed.join(", "),
                    relationships.join(", ")
                )
            }
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
        let node = bound(env, of)?;
        let planned = match source {
            ValueSource::Focus => {
                let mut inner = env.clone();
                inner.insert(var, node);
                return self.expr(test, &inner);
            }
            ValueSource::Path(path) => routes(path)?,
        };
        let mut parts = Vec::new();
        for route in &planned {
            let x = self.fresh();
            let mut inner = env.clone();
            inner.insert(var, x.clone());
            let test = self.expr(test, &inner)?;
            let chain = chain(NEO4J, &route.hops);
            parts.push(match (&route.property, route.hops.is_empty()) {
                (Some(key), true) => format!(
                    "all({x} IN {} WHERE {x} IS NULL OR ({test}))",
                    values(&node, key)
                ),
                (None, _) => format!("NOT EXISTS {{ MATCH ({node}){chain}({x}) WHERE NOT ({test}) }}"),
                (Some(key), false) => {
                    let y = self.fresh();
                    format!(
                        "NOT EXISTS {{ MATCH ({node}){chain}({y}) WHERE any({x} IN {} WHERE {x} IS NOT NULL AND NOT ({test})) }}",
                        values(&y, key)
                    )
                }
            });
        }
        Ok(conjunction(parts))
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
        let node = bound(env, of)?;
        let op = cmp_op(cmp);
        let planned = match source {
            ValueSource::Focus => {
                let condition = match filter {
                    Some(filter) => {
                        let mut inner = env.clone();
                        inner.insert(var, node);
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

        let x = self.fresh();
        let mut inner = env.clone();
        inner.insert(var, x.clone());
        let condition = filter.map(|f| self.expr(f, &inner)).transpose()?;

        if planned.iter().all(|route| route.hops.is_empty()) {
            let lists: Vec<String> = planned
                .iter()
                .map(|route| values(&node, route.property.as_deref().unwrap_or_default()))
                .collect();
            let acc = self.fresh();
            let skip = condition.map_or_else(String::new, |c| format!(" OR NOT ({c})"));
            return Ok(format!(
                "size(reduce({acc} = [], {x} IN {} | CASE WHEN {x} IS NULL OR {x} IN {acc}{skip} THEN {acc} ELSE {acc} + [{x}] END)) {op} {limit}",
                lists.join(" + ")
            ));
        }

        let returning = if planned.len() == 1 {
            "RETURN DISTINCT"
        } else {
            "RETURN"
        };
        let mut branches = Vec::new();
        for route in &planned {
            let chain = chain(NEO4J, &route.hops);
            let mut guards = Vec::new();
            let head = match (&route.property, route.hops.is_empty()) {
                (None, _) => format!("MATCH ({node}){chain}({x})"),
                (Some(key), true) => {
                    guards.push(format!("{x} IS NOT NULL"));
                    format!("UNWIND {} AS {x} WITH *", values(&node, key))
                }
                (Some(key), false) => {
                    let y = self.fresh();
                    guards.push(format!("{x} IS NOT NULL"));
                    format!(
                        "MATCH ({node}){chain}({y}) UNWIND {} AS {x} WITH *",
                        values(&y, key)
                    )
                }
            };
            if let Some(condition) = &condition {
                guards.push(format!("({condition})"));
            }
            let filter_clause = if guards.is_empty() {
                String::new()
            } else {
                format!(" WHERE {}", guards.join(" AND "))
            };
            branches.push(format!("{head}{filter_clause} {returning} {x}"));
        }
        Ok(format!(
            "COUNT {{ {} }} {op} {limit}",
            branches.join(" UNION ")
        ))
    }

    fn pair(
        &mut self,
        of: Var,
        left: &ValueSource,
        right: &LpgPath,
        relation: PairRelation,
        env: &Env,
    ) -> Result<String, RenderError> {
        let node = bound(env, of)?;
        let left = match left {
            ValueSource::Focus => Side::Itself,
            ValueSource::Path(path) => side(&node, path)?,
        };
        let right = side(&node, right)?;
        let x = self.fresh();
        let y = self.fresh();
        Ok(match (left, right) {
            (Side::Values(l), Side::Values(r)) => match relation {
                PairRelation::Equals => format!(
                    "(all({x} IN {l} WHERE {x} IS NULL OR coalesce({x} IN {r}, false)) AND all({y} IN {r} WHERE {y} IS NULL OR coalesce({y} IN {l}, false)))"
                ),
                PairRelation::Disjoint => {
                    format!("none({x} IN {l} WHERE {x} IS NOT NULL AND coalesce({x} IN {r}, false))")
                }
                PairRelation::LessThan | PairRelation::LessThanOrEquals => {
                    let op = if relation == PairRelation::LessThan { "<" } else { "<=" };
                    format!(
                        "all({x} IN {l} WHERE {x} IS NULL OR all({y} IN {r} WHERE {y} IS NULL OR coalesce({x} {op} {y}, false)))"
                    )
                }
            },
            (Side::Nodes(l), Side::Nodes(r)) => match relation {
                PairRelation::Equals => conjunction(vec![
                    only_within(&node, &l, &r, &x),
                    only_within(&node, &r, &l, &y),
                ]),
                PairRelation::Disjoint => conjunction(
                    l.iter()
                        .map(|route| {
                            format!(
                                "NOT EXISTS {{ MATCH ({node}){}({x}) WHERE {} }}",
                                chain(NEO4J, &route.hops),
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
            },
            (Side::Itself, Side::Nodes(r)) => match relation {
                PairRelation::Equals => format!(
                    "({} AND {})",
                    conjunction(
                        r.iter()
                            .map(|route| format!(
                                "NOT EXISTS {{ MATCH ({node}){}({x}) WHERE {x} <> {node} }}",
                                chain(NEO4J, &route.hops)
                            ))
                            .collect()
                    ),
                    reaches(&node, &r, &node)
                ),
                PairRelation::Disjoint => format!("NOT {}", reaches(&node, &r, &node)),
                PairRelation::LessThan | PairRelation::LessThanOrEquals => {
                    format!("NOT {}", any_value(&node, &r))
                }
            },
            (Side::Itself, Side::Values(r)) => match relation {
                PairRelation::Equals => "false".to_owned(),
                PairRelation::Disjoint => "true".to_owned(),
                PairRelation::LessThan | PairRelation::LessThanOrEquals => no_values(&r, &x),
            },
            (Side::Values(l), Side::Nodes(r)) | (Side::Nodes(r), Side::Values(l)) => match relation
            {
                PairRelation::Equals => {
                    format!("({} AND NOT {})", no_values(&l, &x), any_value(&node, &r))
                }
                PairRelation::Disjoint => "true".to_owned(),
                PairRelation::LessThan | PairRelation::LessThanOrEquals => {
                    format!("({} OR NOT {})", no_values(&l, &x), any_value(&node, &r))
                }
            },
            (_, Side::Itself) => {
                return Err(RenderError(
                    "internal error: the right side of a pair is always a path".into(),
                ))
            }
        })
    }

    fn focus_map(&self, focus: &[FocusSet], relationship: bool) -> String {
        if relationship {
            return format!(
                "{{type: type(v0), startKey: {}, endKey: {}, elementId: elementId(v0)}}",
                key_value(self.meta.value_key, "s0"),
                key_value(self.meta.value_key, "e0")
            );
        }
        let labels: Vec<String> = focus
            .iter()
            .filter_map(|set| match set {
                FocusSet::Labels(labels) => Some(labels.iter().map(|l| quote(l))),
                _ => None,
            })
            .flatten()
            .collect();
        let label = if labels.is_empty() {
            "head(labels(v0))".to_owned()
        } else {
            format!(
                "head([l IN labels(v0) WHERE l IN [{}]] + labels(v0))",
                labels.join(", ")
            )
        };
        let key = key_literal(self.meta.focus_key);
        format!(
            "{{label: {label}, key: {key}, keyValue: {}, elementId: elementId(v0)}}",
            key_value(self.meta.focus_key, "v0")
        )
    }

    fn node_map(&self, name: &str) -> String {
        format!(
            "{{label: head(labels({name})), keyValue: {}, elementId: elementId({name})}}",
            key_value(self.meta.value_key, name)
        )
    }

    fn message(&self, relationship: bool, value: Option<&(String, bool)>) -> String {
        let text = quote(self.meta.message);
        if !self.meta.message.contains("{$this}") && !self.meta.message.contains("{?value}") {
            return text;
        }
        let this = match (relationship, self.meta.focus_key) {
            (false, Some(key)) => format!("coalesce(toString(v0.{}), elementId(v0))", ident(key)),
            _ => "elementId(v0)".to_owned(),
        };
        let value = match (value, self.meta.value_key) {
            (None, _) => "''".to_owned(),
            (Some((name, false)), _) => format!("coalesce(toString({name}), '')"),
            (Some((name, true)), Some(key)) => {
                format!(
                    "coalesce(toString({name}.{}), elementId({name}))",
                    ident(key)
                )
            }
            (Some((name, true)), None) => format!("elementId({name})"),
        };
        format!("replace(replace({text}, '{{$this}}', {this}), '{{?value}}', {value})")
    }

    fn details(&mut self, details: &[Detail], env: &Env) -> Result<String, RenderError> {
        if details.is_empty() {
            return Ok("[]".to_owned());
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
            "[{d} IN [{}] WHERE {d} IS NOT NULL]",
            cases.join(", ")
        ))
    }

    fn fresh(&mut self) -> String {
        self.names += 1;
        format!("x{}", self.names)
    }
}

/// The key property name as a literal, or `null`.
fn key_literal(key: Option<&str>) -> String {
    key.map_or_else(|| "null".to_owned(), quote)
}

/// The key property of `var`, or `null` when no key is configured.
fn key_value(key: Option<&str>, var: &str) -> String {
    key.map_or_else(|| "null".to_owned(), |key| format!("{var}.{}", ident(key)))
}

fn side(node: &str, path: &LpgPath) -> Result<Side, RenderError> {
    let planned = routes(path)?;
    if planned
        .iter()
        .all(|route| route.hops.is_empty() && route.property.is_some())
    {
        let lists: Vec<String> = planned
            .iter()
            .map(|route| values(node, route.property.as_deref().unwrap_or_default()))
            .collect();
        return Ok(Side::Values(lists.join(" + ")));
    }
    if planned.iter().all(|route| route.property.is_none()) {
        return Ok(Side::Nodes(planned));
    }
    Err(RenderError(
        "sh:equals, sh:disjoint, sh:lessThan and sh:lessThanOrEquals support direct properties and relationship paths only".into(),
    ))
}

/// Every node reached over `from` is also reached over `other`.
fn only_within(node: &str, from: &[Route], other: &[Route], x: &str) -> String {
    conjunction(
        from.iter()
            .map(|route| {
                format!(
                    "NOT EXISTS {{ MATCH ({node}){}({x}) WHERE NOT {} }}",
                    chain(NEO4J, &route.hops),
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
                chain(NEO4J, &route.hops)
            )
        })
        .collect();
    format!("({})", parts.join(" OR "))
}

fn any_value(node: &str, planned: &[Route]) -> String {
    let parts: Vec<String> = planned
        .iter()
        .map(|route| format!("EXISTS {{ MATCH ({node}){}() }}", chain(NEO4J, &route.hops)))
        .collect();
    format!("({})", parts.join(" OR "))
}

fn no_values(list: &str, x: &str) -> String {
    format!("none({x} IN {list} WHERE {x} IS NOT NULL)")
}

fn conjunction(mut parts: Vec<String>) -> String {
    if parts.len() == 1 {
        parts.remove(0)
    } else {
        format!("({})", parts.join(" AND "))
    }
}

/// A property as a list of its values: scalars become one-element lists.
fn values(base: &str, key: &str) -> String {
    let property = format!("{base}.{}", ident(key));
    format!(
        "CASE WHEN {property} IS NULL THEN [] WHEN {property} IS :: LIST<ANY> THEN {property} ELSE [{property}] END"
    )
}

fn value_test(value: &str, test: &ValueTest) -> Result<String, RenderError> {
    Ok(match test {
        ValueTest::Datatype(check) => datatype_test(value, check)?,
        ValueTest::HasLabel(labels) => format!("{value}:{}", label_expr(labels)),
        ValueTest::Compare { cmp, bound } => format!(
            "coalesce({value} {} {}, false)",
            cmp_op(*cmp),
            constant(NEO4J, bound)?
        ),
        ValueTest::Length { cmp, bound } => {
            let op = cmp_op(*cmp);
            format!(
                "CASE WHEN {value} IS :: STRING NOT NULL THEN size({value}) {op} {bound} WHEN {value} IS :: INTEGER NOT NULL THEN size(toString({value})) {op} {bound} ELSE false END"
            )
        }
        ValueTest::Matches(regex) => {
            let pattern = quote(&regex_pattern(regex));
            format!(
                "CASE WHEN {value} IS :: STRING NOT NULL THEN {value} =~ {pattern} WHEN {value} IS :: INTEGER NOT NULL THEN toString({value}) =~ {pattern} ELSE false END"
            )
        }
        ValueTest::In(constants) if constants.is_empty() => "false".to_owned(),
        ValueTest::In(constants) => format!(
            "coalesce({value} IN [{}], false)",
            constant_list(constants)?
        ),
        ValueTest::NodeKeyIn { key, values } => format!(
            "coalesce({value}.{} IN [{}], false)",
            ident(key),
            constant_list(values)?
        ),
    })
}

fn constant_list(constants: &[crate::ir::Constant]) -> Result<String, RenderError> {
    Ok(constants
        .iter()
        .map(|c| constant(NEO4J, c))
        .collect::<Result<Vec<_>, _>>()?
        .join(", "))
}

fn datatype_test(value: &str, check: &DatatypeCheck) -> Result<String, RenderError> {
    let mut names = check
        .allowed
        .iter()
        .map(neo4j_type)
        .collect::<Result<Vec<_>, _>>()?;
    names.sort();
    names.dedup();
    let types: Vec<String> = names
        .iter()
        .map(|name| format!("{value} IS :: {name} NOT NULL"))
        .collect();
    let types = types.join(" OR ");
    let Some((min, max)) = check.range else {
        return Ok(format!("({types})"));
    };
    let mut bounds = Vec::new();
    if min > i128::from(i64::MIN) {
        bounds.push(format!("{value} >= {min}"));
    }
    if max < i128::from(i64::MAX) {
        bounds.push(format!("{value} <= {max}"));
    }
    if bounds.is_empty() {
        Ok(format!("({types})"))
    } else {
        Ok(format!(
            "(({types}) AND coalesce({}, false))",
            bounds.join(" AND ")
        ))
    }
}

fn neo4j_type(value_type: &ValueType) -> Result<String, RenderError> {
    Ok(match value_type {
        ValueType::String => "STRING".into(),
        ValueType::Int64
        | ValueType::Int32
        | ValueType::Int16
        | ValueType::Int8
        | ValueType::UInt64
        | ValueType::UInt32
        | ValueType::UInt16
        | ValueType::UInt8 => "INTEGER".into(),
        ValueType::Double | ValueType::Float | ValueType::Decimal => "FLOAT".into(),
        ValueType::Boolean => "BOOLEAN".into(),
        ValueType::Date => "DATE".into(),
        ValueType::LocalDateTime => "LOCAL DATETIME".into(),
        ValueType::ZonedDateTime => "ZONED DATETIME".into(),
        ValueType::LocalTime => "LOCAL TIME".into(),
        ValueType::ZonedTime => "ZONED TIME".into(),
        ValueType::Duration => "DURATION".into(),
        ValueType::Point => "POINT".into(),
        ValueType::Any => "ANY".into(),
        ValueType::List(element) => format!("LIST<{}>", neo4j_type(element)?),
        ValueType::Blob => {
            return Err(RenderError(
                "BLOB values cannot be type-checked on Neo4j".into(),
            ))
        }
    })
}

/// Java regex with XSD substring semantics: flags, then the pattern between
/// dot-all wildcards scoped so they do not change the pattern's own flags.
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
    let prefix = if flags.is_empty() {
        String::new()
    } else {
        format!("(?{flags})")
    };
    format!("{prefix}(?s:.*)(?:{})(?s:.*)", regex.pattern)
}

fn label_expr(labels: &[String]) -> String {
    labels
        .iter()
        .map(|label| ident(label))
        .collect::<Vec<_>>()
        .join("|")
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

fn bound(env: &Env, var: Var) -> Result<String, RenderError> {
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

    /// Exercises most IR constructs; kept in sync with the live Neo4j smoke run.
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
            ..LowerOptions::default()
        };
        lower(inputs, &options).unwrap().rules
    }

    fn render_rule(rule: &Rule) -> Result<Rendered, RenderError> {
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
        render(rule, &meta)
    }

    fn rendered(rules: &[Rule], id: &str) -> Rendered {
        let rule = rules
            .iter()
            .find(|rule| rule.id.to_string() == id)
            .unwrap_or_else(|| panic!("no rule {id}"));
        render_rule(rule).unwrap()
    }

    #[test]
    fn rejects_identifiers_neo4j_would_decode() {
        assert!(unrepresentable_identifier("a\\u0041"));
        assert!(!unrepresentable_identifier("a\\u00G1"));
        assert!(!unrepresentable_identifier("a\\\\x"));
        assert_eq!(
            backticked("MATCH (n:`A``b`) WHERE n.x = 'it''s `no`' RETURN n.`c` AS `d`"),
            ["A`b", "c", "d"]
        );
        let rules = compile(
            "ex:S sh:targetClass ex:Person ;
    sh:property [ sh:path ex:p ; s2c:property \"x\\\\u0041\" ; sh:minCount 1 ] .
",
        );
        let error = render_rule(&rules[0]).unwrap_err().to_string();
        assert!(error.contains("decodes inside backticks"), "{error}");
    }

    #[test]
    fn renders_every_compiled_rule_without_write_clauses() {
        let rules = compile(SHAPES);
        assert!(rules.len() > 15, "{}", rules.len());
        let dump = std::env::var_os("S2C_RENDER_DUMP").map(std::path::PathBuf::from);
        for (index, rule) in rules.iter().enumerate() {
            let queries = render_rule(rule).unwrap_or_else(|e| panic!("{}: {e}", rule.id));
            for query in [&queries.detail, &queries.summary] {
                for clause in ["CREATE ", "MERGE ", " SET ", "DELETE ", "REMOVE "] {
                    assert!(!query.contains(clause), "{}: {query}", rule.id);
                }
            }
            assert!(queries
                .detail
                .ends_with("LIMIT coalesce($limit, 9223372036854775807)"));
            assert!(queries.summary.contains("[0..$sampleSize] AS sample"));
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
    fn counts_distinct_property_values() {
        let rules = compile(SHAPES);
        let min_count = rendered(&rules, "ex:PersonShape/ex:name/sh:minCount");
        assert!(min_count
            .detail
            .starts_with("MATCH (v0:`Person`)\nWITH v0\nWHERE NOT (size(reduce("));
        let alternative = rendered(&rules, "ex:PersonShape/ex:email|ex:phone/sh:minCount");
        assert!(alternative.detail.contains("`email`"));
        assert!(alternative.detail.contains(" + CASE WHEN v0.`phone`"));
    }

    #[test]
    fn traverses_relationships_for_node_values() {
        let rules = compile(SHAPES);
        let class = rendered(&rules, "ex:PersonShape/ex:worksFor/sh:class");
        assert!(class.detail.contains(
            "MATCH (v0)-[:`WORKS_FOR`]->(v1)\nWITH DISTINCT v0, v1\nWHERE NOT (v1:`Company`)"
        ));
        assert!(class
            .detail
            .contains("{label: head(labels(v1)), keyValue: v1.`id`"));
        let max_count = rendered(&rules, "ex:PersonShape/ex:worksFor/sh:maxCount");
        assert!(max_count
            .detail
            .contains("COUNT { MATCH (v0)-[:`WORKS_FOR`]->(x1) RETURN DISTINCT x1 } <= 1"));
        let transitive = rendered(&rules, "ex:PersonShape/ex:knows+/sh:class");
        assert!(transitive.detail.contains("-[:`KNOWS`*1..10]->(v1)"));
        let inverse = rendered(&rules, "ex:CompanyShape/^ex:worksFor/sh:minCount");
        assert!(inverse.detail.contains("(v0)<-[:`WORKS_FOR`]-(x1)"));
    }

    #[test]
    fn guards_value_tests_and_wraps_regexes() {
        let rules = compile(SHAPES);
        let pattern = rendered(&rules, "ex:PersonShape/ex:name/sh:pattern");
        assert!(pattern
            .detail
            .contains("v1 =~ '(?i)(?s:.*)(?:^[A-Z])(?s:.*)'"));
        assert!(pattern
            .detail
            .contains("replace(replace('{$this} has a bad name {?value}'"));
        let datatype = rendered(&rules, "ex:PersonShape/ex:age/sh:datatype");
        assert!(datatype.detail.contains(
            "((v1 IS :: INTEGER NOT NULL) AND coalesce(v1 >= -32768 AND v1 <= 32767, false))"
        ));
        let range = rendered(&rules, "ex:PersonShape/ex:age/sh:minInclusive");
        assert!(range.detail.contains("coalesce(v1 >= 0, false)"));
    }

    #[test]
    fn renders_nested_details_pairs_closed_and_relationship_focus() {
        let rules = compile(SHAPES);
        let node = rendered(&rules, "ex:PersonShape/ex:address/sh:node");
        assert!(node
            .detail
            .contains("THEN 'ex:AddressShape/ex:zip/sh:minCount' END"));
        let pair = rendered(&rules, "ex:PersonShape/ex:start/sh:lessThan");
        assert!(pair.detail.contains("coalesce(x1 < x2, false)"));
        let closed = rendered(&rules, "ex:PersonShape/sh:closed");
        assert!(closed.detail.contains("IN keys(v0) WHERE"));
        assert!(closed.detail.contains("IN [(v0)-["));
        assert!(closed
            .detail
            .contains("'ADDRESS', 'EMAIL', 'ID', 'PHONE', 'WORKS_FOR']"));
        let since = rendered(&rules, "ex:KnowsShape/ex:since/sh:datatype");
        assert!(since.detail.starts_with("MATCH (s0)-[v0:`KNOWS`]->(e0)"));
        assert!(since.detail.contains("{type: type(v0), startKey: s0.`id`"));
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
    fn refuses_rules_that_are_not_compiled() {
        let mut rules = compile(SHAPES);
        rules[0].status = RuleStatus::Deactivated;
        assert!(render_rule(&rules[0])
            .unwrap_err()
            .to_string()
            .contains("not rendered"));
    }
}
