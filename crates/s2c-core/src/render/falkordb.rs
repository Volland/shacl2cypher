//! FalkorDB 4.20 (C engine) Cypher renderer.
//!
//! FalkorDB evaluates a pattern correctly only when it starts at a row variable, and
//! `CASE` evaluates every branch. Every relationship traversal therefore becomes a
//! correlated `CALL { WITH … OPTIONAL MATCH … RETURN <aggregate> }` stage whose column
//! the rule condition reads, and every value test is total over all value types.

use std::collections::HashMap;
use std::fmt::Write as _;

use super::{
    backticked, constant, ident, quote, routes, Dialect, Hop, RenderError, Rendered, Route,
    RuleMeta,
};
use crate::ast::Direction;
use crate::datatypes::DatatypeCheck;
use crate::ir::{
    Cmp, Detail, Expr, FocusSet, PairRelation, Rule, RuleStatus, ValueSource, ValueTest, Var,
    Violation,
};
use crate::mapping::LpgPath;
use crate::schema::ValueType;
use crate::xsd_regex::XsdRegex;

const FALKORDB: Dialect = Dialect::FalkorDb;

/// How an IR variable is bound in the query.
#[derive(Debug, Clone)]
struct Binding {
    name: String,
    /// Bound by a clause, so subqueries can import it and patterns can start at it;
    /// false for list-quantifier variables.
    row: bool,
}

type Env = HashMap<Var, Binding>;

/// `CALL` stages computed before a condition is used.
#[derive(Debug)]
struct Scope {
    /// Row variables in scope, imported by every correlated subquery.
    rows: Vec<String>,
    stages: String,
    /// Columns returned by the stages, carried through the next `WITH`.
    columns: Vec<String>,
}

impl Scope {
    fn new(rows: Vec<String>) -> Self {
        Scope {
            rows,
            stages: String::new(),
            columns: Vec::new(),
        }
    }
}

/// Renders a compiled rule as a detail query and a summary query.
// @lat: [[dialects#Dialect Backends#FalkorDB]]
pub fn render(rule: &Rule, meta: &RuleMeta<'_>) -> Result<Rendered, RenderError> {
    Renderer { meta, names: 0 }.rule(rule)
}

/// FalkorDB has no escape for a backtick inside a backticked identifier.
// @lat: [[dialects#Literals and Identifiers]]
pub fn unrepresentable_identifier(name: &str) -> bool {
    name.contains('`')
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
    /// A list expression of the ids of nodes reached over relationships.
    Ids(String),
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
        let carry: Vec<String> = if relationship {
            vec!["s0".into(), "v0".into(), "e0".into()]
        } else {
            vec!["v0".into()]
        };
        let mut env = Env::new();
        env.insert(Var::FOCUS, row("v0"));

        let mut body = self.focus_match(&rule.focus)?;
        let (value, row_env, rows) = match &rule.violation {
            Violation::PerFocus { condition, .. } => {
                let mut scope = Scope::new(carry.clone());
                let condition = self.cond(condition, &env, &mut scope)?;
                body.push_str(&scope.stages);
                let _ = write!(
                    body,
                    "\nWITH {}\nWHERE NOT ({condition})",
                    with_list(&carry, &scope.columns)
                );
                (None, env, carry)
            }
            Violation::PerValue {
                source, var, test, ..
            } => {
                let name = var.to_string();
                let nodes = self.bind_values(&mut body, source, &name, &carry)?;
                let mut row_env = env.clone();
                row_env.insert(*var, row(&name));
                let mut rows = carry.clone();
                rows.push(name.clone());
                let mut scope = Scope::new(rows.clone());
                let test = self.cond(test, &row_env, &mut scope)?;
                body.push_str(&scope.stages);
                let guard = if nodes {
                    String::new()
                } else {
                    format!("{name} IS NOT NULL AND ")
                };
                let _ = write!(
                    body,
                    "\nWITH {}\nWHERE {guard}NOT ({test})",
                    with_list(&rows, &scope.columns)
                );
                (Some((name, nodes)), row_env, rows)
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
        let mut detail_scope = Scope::new(rows);
        let details = self.details(&rule.details, &row_env, &mut detail_scope)?;
        let path = meta.path.map_or_else(|| "null".to_owned(), quote);
        let detail = format!(
            "{body}{}\nRETURN {} AS ruleId, {} AS shape, {path} AS path, {} AS `constraint`, {} AS severity,\n       {focus} AS focus, {value_expr} AS value, {message} AS message, {details} AS details\nLIMIT $limit",
            detail_scope.stages,
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
                "identifier `{name}` contains a backtick, which FalkorDB cannot escape inside an identifier"
            )));
        }
        Ok(Rendered { detail, summary })
    }

    fn focus_match(&mut self, focus: &[FocusSet]) -> Result<String, RenderError> {
        match focus {
            [FocusSet::Relationships(rel_type)] => Ok(format!(
                "MATCH (s0)-[v0:{}]->(e0)\nWHERE id(v0) >= 0",
                ident(rel_type)
            )),
            [FocusSet::Labels(labels)] if labels.len() == 1 => {
                Ok(format!("MATCH (v0:{})", ident(&labels[0])))
            }
            sets if !sets.is_empty()
                && !sets
                    .iter()
                    .any(|set| matches!(set, FocusSet::Relationships(_))) =>
            {
                let rows = vec!["v0".to_owned()];
                let mut scope = Scope::new(rows.clone());
                let conditions = sets
                    .iter()
                    .map(|set| self.focus_condition(set, &mut scope))
                    .collect::<Result<Vec<_>, _>>()?;
                let conditions = conditions.join(" OR ");
                if scope.stages.is_empty() {
                    Ok(format!("MATCH (v0)\nWHERE {conditions}"))
                } else {
                    Ok(format!(
                        "MATCH (v0){}\nWITH {}\nWHERE {conditions}",
                        scope.stages,
                        with_list(&rows, &scope.columns)
                    ))
                }
            }
            _ => Err(RenderError(
                "a rule needs node focus sets or a single relationship focus".into(),
            )),
        }
    }

    fn focus_condition(
        &mut self,
        set: &FocusSet,
        scope: &mut Scope,
    ) -> Result<String, RenderError> {
        Ok(match set {
            FocusSet::Labels(labels) => label_test("v0", labels),
            FocusSet::SubjectsOf(path) => {
                let mut parts = Vec::new();
                for route in routes(path)? {
                    parts.push(match (&route.property, route.hops.is_empty()) {
                        (Some(key), true) => format!("v0.{} IS NOT NULL", ident(key)),
                        (None, _) => {
                            let x = self.fresh();
                            let (pattern, guards) = self.pattern("v0", &route.hops, &x);
                            self.stage(
                                scope,
                                &format!("OPTIONAL MATCH {pattern}{}", where_clause(&guards)),
                                &format!("count({x}) > 0"),
                            )
                        }
                        (Some(key), false) => {
                            let x = self.fresh();
                            let (pattern, guards) = self.pattern("v0", &route.hops, &x);
                            self.stage(
                                scope,
                                &format!("OPTIONAL MATCH {pattern}{}", where_clause(&guards)),
                                &format!("count({x}.{}) > 0", ident(key)),
                            )
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
                let mut parts = Vec::new();
                for route in &planned {
                    let x = self.fresh();
                    let (pattern, guards) = self.pattern(&x, &route.hops, "v0");
                    parts.push(self.stage(
                        scope,
                        &format!("OPTIONAL MATCH {pattern}{}", where_clause(&guards)),
                        &format!("count({x}) > 0"),
                    ));
                }
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
        carry: &[String],
    ) -> Result<bool, RenderError> {
        let carried = carry.join(", ");
        let path = match source {
            ValueSource::Focus => {
                let _ = write!(body, "\nWITH {carried}, v0 AS {name}");
                return Ok(true);
            }
            ValueSource::Path(path) => path,
        };
        let planned = routes(path)?;
        let nodes = planned.iter().all(|route| route.property.is_none());
        match planned.as_slice() {
            [route] => body.push_str(&self.route_clause(route, "v0", name, false)),
            _ if planned.iter().all(|route| route.hops.is_empty()) => {
                let lists: Vec<String> = planned
                    .iter()
                    .map(|route| values("v0", route.property.as_deref().unwrap_or_default()))
                    .collect();
                let _ = write!(body, "\nUNWIND {} AS {name}", lists.join(" + "));
            }
            _ => {
                let mut branches = Vec::new();
                for route in &planned {
                    let clause = self.route_clause(route, "v0", name, false);
                    branches.push(format!(
                        "WITH {carried}{} RETURN {name}",
                        clause.replace('\n', " ")
                    ));
                }
                let _ = write!(body, "\nCALL {{\n  {}\n}}", branches.join("\n  UNION\n  "));
            }
        }
        let _ = write!(body, "\nWITH DISTINCT {carried}, {name}");
        Ok(nodes)
    }

    /// Clauses binding `name` to the values of one route from the row variable `start`.
    fn route_clause(&mut self, route: &Route, start: &str, name: &str, optional: bool) -> String {
        let keyword = if optional { "OPTIONAL MATCH" } else { "MATCH" };
        match (&route.property, route.hops.is_empty()) {
            (Some(key), true) => format!("\nUNWIND {} AS {name}", values(start, key)),
            (None, _) => {
                let (pattern, guards) = self.pattern(start, &route.hops, name);
                format!("\n{keyword} {pattern}{}", where_clause(&guards))
            }
            (Some(key), false) => {
                let x = self.fresh();
                let (pattern, guards) = self.pattern(start, &route.hops, &x);
                format!(
                    "\n{keyword} {pattern}{}\nUNWIND {} AS {name}",
                    where_clause(&guards),
                    values(&x, key)
                )
            }
        }
    }

    /// `(start)-[x1:T]->()<-[:U*1..3]-(end)` and its guards. Single hops bind a
    /// relationship variable the guards reference, and variable-length hops bind a path
    /// variable, because FalkorDB otherwise collapses parallel relationships.
    fn pattern(&mut self, start: &str, hops: &[Hop], end: &str) -> (String, Vec<String>) {
        let mut guards = Vec::new();
        let mut text = format!("({start})");
        for (index, hop) in hops.iter().enumerate() {
            if index > 0 {
                text.push_str("()");
            }
            let types: Vec<String> = hop.types.iter().map(|t| ident(t)).collect();
            let types = types.join("|");
            let body = if hop.min == 1 && hop.max == 1 {
                let r = self.fresh();
                guards.push(format!("id({r}) >= 0"));
                format!("[{r}:{types}]")
            } else {
                format!("[:{types}*{}..{}]", hop.min, hop.max)
            };
            text.push_str(&match hop.direction {
                Direction::Out => format!("-{body}->"),
                Direction::In => format!("<-{body}-"),
            });
        }
        let _ = write!(text, "({end})");
        if hops.iter().any(|hop| hop.min != 1 || hop.max != 1) {
            let p = self.fresh();
            text = format!("{p} = {text}");
        }
        (text, guards)
    }

    /// Appends `CALL { WITH <rows> <body> RETURN <aggregate> AS cN }` to `scope`.
    fn stage(&mut self, scope: &mut Scope, body: &str, aggregate: &str) -> String {
        self.names += 1;
        let column = format!("c{}", self.names);
        let _ = write!(
            scope.stages,
            "\nCALL {{ WITH {} {} RETURN {aggregate} AS {column} }}",
            scope.rows.join(", "),
            body.trim().replace('\n', " ")
        );
        scope.columns.push(column.clone());
        column
    }

    fn cond(&mut self, expr: &Expr, env: &Env, scope: &mut Scope) -> Result<String, RenderError> {
        Ok(match expr {
            Expr::Bool(value) => value.to_string(),
            Expr::Not(inner) => format!("NOT ({})", self.cond(inner, env, scope)?),
            Expr::And(items) => self.join(items, " AND ", env, scope)?,
            Expr::Or(items) => self.join(items, " OR ", env, scope)?,
            Expr::Xone(items) => {
                let parts = items
                    .iter()
                    .map(|item| self.cond(item, env, scope))
                    .collect::<Result<Vec<_>, _>>()?;
                let x = self.fresh();
                format!("size([{x} IN [{}] WHERE {x}]) = 1", parts.join(", "))
            }
            Expr::All {
                of,
                source,
                var,
                test,
            } => self.all(*of, source, *var, test, env, scope)?,
            Expr::Count {
                of,
                source,
                var,
                filter,
                cmp,
                bound: limit,
            } => self.count(
                *of,
                source,
                *var,
                filter.as_deref(),
                *cmp,
                *limit,
                env,
                scope,
            )?,
            Expr::Test { var, test } => value_test(&bound(env, *var)?.name, test)?,
            Expr::Pair {
                of,
                left,
                right,
                relation,
            } => self.pair(*of, left, right, *relation, env, scope)?,
            Expr::Closed {
                of,
                allowed,
                allowed_relationships,
            } => {
                let node = row_bound(env, *of)?;
                let r = self.fresh();
                let types = self.stage(
                    scope,
                    &format!(
                        "OPTIONAL MATCH ({})-[{r}]->() WHERE id({r}) >= 0",
                        node.name
                    ),
                    &format!("collect(DISTINCT type({r}))"),
                );
                let x = self.fresh();
                let y = self.fresh();
                let allowed: Vec<String> = allowed.iter().map(|key| quote(key)).collect();
                let relationships: Vec<String> =
                    allowed_relationships.iter().map(|t| quote(t)).collect();
                format!(
                    "(all({x} IN keys({}) WHERE {x} IN [{}]) AND all({y} IN {types} WHERE {y} IN [{}]))",
                    node.name,
                    allowed.join(", "),
                    relationships.join(", ")
                )
            }
        })
    }

    fn join(
        &mut self,
        items: &[Expr],
        separator: &str,
        env: &Env,
        scope: &mut Scope,
    ) -> Result<String, RenderError> {
        let parts = items
            .iter()
            .map(|item| self.cond(item, env, scope))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(format!("({})", parts.join(separator)))
    }

    #[allow(clippy::too_many_arguments)]
    fn all(
        &mut self,
        of: Var,
        source: &ValueSource,
        var: Var,
        test: &Expr,
        env: &Env,
        scope: &mut Scope,
    ) -> Result<String, RenderError> {
        let node = bound(env, of)?;
        let planned = match source {
            ValueSource::Focus => {
                let mut inner = env.clone();
                inner.insert(var, node);
                return self.cond(test, &inner, scope);
            }
            ValueSource::Path(path) => routes(path)?,
        };
        let mut parts = Vec::new();
        for route in &planned {
            if route.hops.is_empty() {
                let key = route.property.as_deref().unwrap_or_default();
                let x = self.fresh();
                let mut inner = env.clone();
                inner.insert(
                    var,
                    Binding {
                        name: x.clone(),
                        row: false,
                    },
                );
                let test = self.cond(test, &inner, scope)?;
                parts.push(format!(
                    "all({x} IN {} WHERE {x} IS NULL OR ({test}))",
                    values(&node.name, key)
                ));
                continue;
            }
            require_row(&node)?;
            let y = self.fresh();
            let clause = self.route_clause(route, &node.name, &y, true);
            let mut inner_scope = Scope::new(scope.rows.clone());
            inner_scope.rows.push(y.clone());
            let mut inner = rows_only(env);
            inner.insert(var, row(&y));
            let test = self.cond(test, &inner, &mut inner_scope)?;
            let t = self.fresh();
            parts.push(self.stage(
                scope,
                &format!("{clause}{}", inner_scope.stages),
                &format!(
                    "all({t} IN collect(CASE WHEN {y} IS NULL THEN null ELSE ({test}) END) WHERE {t})"
                ),
            ));
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
        scope: &mut Scope,
    ) -> Result<String, RenderError> {
        let node = bound(env, of)?;
        let op = cmp_op(cmp);
        let planned = match source {
            ValueSource::Focus => {
                let condition = match filter {
                    Some(filter) => {
                        let mut inner = env.clone();
                        inner.insert(var, node);
                        self.cond(filter, &inner, scope)?
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
            let x = self.fresh();
            let mut inner = env.clone();
            inner.insert(
                var,
                Binding {
                    name: x.clone(),
                    row: false,
                },
            );
            let condition = filter.map(|f| self.cond(f, &inner, scope)).transpose()?;
            let lists: Vec<String> = planned
                .iter()
                .map(|route| values(&node.name, route.property.as_deref().unwrap_or_default()))
                .collect();
            let acc = self.fresh();
            let skip = condition.map_or_else(String::new, |c| format!(" OR NOT ({c})"));
            return Ok(format!(
                "size(reduce({acc} = [], {x} IN {} | CASE WHEN {x} IS NULL OR {x} IN {acc}{skip} THEN {acc} ELSE {acc} + [{x}] END)) {op} {limit}",
                lists.join(" + ")
            ));
        }

        require_row(&node)?;
        let x = self.fresh();
        let head = if let [route] = planned.as_slice() {
            self.route_clause(route, &node.name, &x, true)
        } else {
            let imported = scope.rows.join(", ");
            let mut branches = Vec::new();
            for route in &planned {
                let clause = self.route_clause(route, &node.name, &x, true);
                branches.push(format!(
                    "WITH {imported} {} RETURN {x}",
                    clause.trim().replace('\n', " ")
                ));
            }
            format!("\nCALL {{ {} }}", branches.join(" UNION "))
        };
        let mut inner_scope = Scope::new(scope.rows.clone());
        inner_scope.rows.push(x.clone());
        let mut inner = rows_only(env);
        inner.insert(var, row(&x));
        let condition = filter
            .map(|f| self.cond(f, &inner, &mut inner_scope))
            .transpose()?;
        let counted = match condition {
            Some(condition) => format!("CASE WHEN {x} IS NOT NULL AND ({condition}) THEN {x} END"),
            None => x.clone(),
        };
        Ok(self.stage(
            scope,
            &format!("{head}{}", inner_scope.stages),
            &format!("count(DISTINCT {counted}) {op} {limit}"),
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn pair(
        &mut self,
        of: Var,
        left: &ValueSource,
        right: &LpgPath,
        relation: PairRelation,
        env: &Env,
        scope: &mut Scope,
    ) -> Result<String, RenderError> {
        let node = bound(env, of)?;
        let left = match left {
            ValueSource::Focus => Side::Itself,
            ValueSource::Path(path) => self.side(&node, path, scope)?,
        };
        let right = self.side(&node, right, scope)?;
        let x = self.fresh();
        let y = self.fresh();
        let id = format!("id({})", node.name);
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
                        "all({x} IN {l} WHERE {x} IS NULL OR all({y} IN {r} WHERE {y} IS NULL OR {}))",
                        ordered(&x, op, &y)
                    )
                }
            },
            (Side::Ids(l), Side::Ids(r)) => match relation {
                PairRelation::Equals => format!(
                    "(all({x} IN {l} WHERE {x} IN {r}) AND all({y} IN {r} WHERE {y} IN {l}))"
                ),
                PairRelation::Disjoint => format!("none({x} IN {l} WHERE {x} IN {r})"),
                PairRelation::LessThan | PairRelation::LessThanOrEquals => {
                    format!("NOT (size({l}) > 0 AND size({r}) > 0)")
                }
            },
            (Side::Itself, Side::Ids(r)) => match relation {
                PairRelation::Equals => {
                    format!("(all({x} IN {r} WHERE {x} = {id}) AND {id} IN {r})")
                }
                PairRelation::Disjoint => format!("NOT ({id} IN {r})"),
                PairRelation::LessThan | PairRelation::LessThanOrEquals => {
                    format!("size({r}) = 0")
                }
            },
            (Side::Itself, Side::Values(r)) => match relation {
                PairRelation::Equals => "false".to_owned(),
                PairRelation::Disjoint => "true".to_owned(),
                PairRelation::LessThan | PairRelation::LessThanOrEquals => no_values(&r, &x),
            },
            (Side::Values(l), Side::Ids(r)) | (Side::Ids(r), Side::Values(l)) => match relation {
                PairRelation::Equals => format!("({} AND size({r}) = 0)", no_values(&l, &x)),
                PairRelation::Disjoint => "true".to_owned(),
                PairRelation::LessThan | PairRelation::LessThanOrEquals => {
                    format!("({} OR size({r}) = 0)", no_values(&l, &x))
                }
            },
            (_, Side::Itself) => {
                return Err(RenderError(
                    "internal error: the right side of a pair is always a path".into(),
                ))
            }
        })
    }

    fn side(
        &mut self,
        node: &Binding,
        path: &LpgPath,
        scope: &mut Scope,
    ) -> Result<Side, RenderError> {
        let planned = routes(path)?;
        if planned
            .iter()
            .all(|route| route.hops.is_empty() && route.property.is_some())
        {
            let lists: Vec<String> = planned
                .iter()
                .map(|route| values(&node.name, route.property.as_deref().unwrap_or_default()))
                .collect();
            return Ok(Side::Values(lists.join(" + ")));
        }
        if planned.iter().all(|route| route.property.is_none()) {
            require_row(node)?;
            let mut columns = Vec::new();
            for route in &planned {
                let y = self.fresh();
                let clause = self.route_clause(route, &node.name, &y, true);
                columns.push(self.stage(scope, &clause, &format!("collect(DISTINCT id({y}))")));
            }
            return Ok(Side::Ids(format!("({})", columns.join(" + "))));
        }
        Err(RenderError(
            "sh:equals, sh:disjoint, sh:lessThan and sh:lessThanOrEquals support direct properties and relationship paths only".into(),
        ))
    }

    fn focus_map(&self, focus: &[FocusSet], relationship: bool) -> String {
        if relationship {
            return format!(
                "{{type: type(v0), startKey: {}, endKey: {}, elementId: toString(id(v0))}}",
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
            "{{label: {label}, key: {key}, keyValue: {}, elementId: toString(id(v0))}}",
            key_value(self.meta.focus_key, "v0")
        )
    }

    fn node_map(&self, name: &str) -> String {
        format!(
            "{{label: head(labels({name})), keyValue: {}, elementId: toString(id({name}))}}",
            key_value(self.meta.value_key, name)
        )
    }

    fn message(&self, relationship: bool, value: Option<&(String, bool)>) -> String {
        let text = quote(self.meta.message);
        if !self.meta.message.contains("{$this}") && !self.meta.message.contains("{?value}") {
            return text;
        }
        let this = match (relationship, self.meta.focus_key) {
            (false, Some(key)) => format!(
                "coalesce(toStringOrNull(v0.{}), toString(id(v0)))",
                ident(key)
            ),
            _ => "toString(id(v0))".to_owned(),
        };
        let value = match (value, self.meta.value_key) {
            (None, _) => "''".to_owned(),
            (Some((name, false)), _) => format!("coalesce(toStringOrNull({name}), '')"),
            (Some((name, true)), Some(key)) => format!(
                "coalesce(toStringOrNull({name}.{}), toString(id({name})))",
                ident(key)
            ),
            (Some((name, true)), None) => format!("toString(id({name}))"),
        };
        format!("replace(replace({text}, '{{$this}}', {this}), '{{?value}}', {value})")
    }

    fn details(
        &mut self,
        details: &[Detail],
        env: &Env,
        scope: &mut Scope,
    ) -> Result<String, RenderError> {
        if details.is_empty() {
            return Ok("[]".to_owned());
        }
        let mut cases = Vec::new();
        for detail in details {
            cases.push(format!(
                "CASE WHEN NOT ({}) THEN {} END",
                self.cond(&detail.holds, env, scope)?,
                quote(&detail.rule.to_string())
            ));
        }
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

fn row(name: &str) -> Binding {
    Binding {
        name: name.to_owned(),
        row: true,
    }
}

/// The bindings a correlated subquery can see: list-quantifier variables stay outside.
fn rows_only(env: &Env) -> Env {
    env.iter()
        .filter(|(_, binding)| binding.row)
        .map(|(var, binding)| (*var, binding.clone()))
        .collect()
}

fn require_row(binding: &Binding) -> Result<(), RenderError> {
    if binding.row {
        Ok(())
    } else {
        Err(RenderError(format!(
            "FalkorDB cannot traverse relationships from the list element {}",
            binding.name
        )))
    }
}

fn with_list(rows: &[String], columns: &[String]) -> String {
    rows.iter()
        .chain(columns)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ")
}

fn where_clause(guards: &[String]) -> String {
    if guards.is_empty() {
        String::new()
    } else {
        format!(" WHERE {}", guards.join(" AND "))
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

/// A property as a list of its values: null becomes empty and scalars one-element lists.
fn values(base: &str, key: &str) -> String {
    let property = format!("{base}.{}", ident(key));
    format!("CASE typeOf({property}) WHEN 'Null' THEN [] WHEN 'List' THEN {property} ELSE [{property}] END")
}

/// A test that holds for the right kind and type of value and never raises: `CASE`
/// evaluates every branch, so string functions only see `toStringOrNull` results.
fn value_test(value: &str, test: &ValueTest) -> Result<String, RenderError> {
    Ok(match test {
        ValueTest::Datatype(check) => datatype_test(value, check)?,
        ValueTest::HasLabel(labels) => label_test(value, labels),
        ValueTest::Compare { cmp, bound } => match temporal_constant_key(bound)? {
            Some((type_name, key)) => format!(
                "coalesce({} {} {key}, false)",
                temporal_key(value, type_name),
                cmp_op(*cmp)
            ),
            None => format!(
                "coalesce({value} {} {}, false)",
                cmp_op(*cmp),
                constant(FALKORDB, bound)?
            ),
        },
        ValueTest::Length { cmp, bound } => format!(
            "(typeOf({value}) IN ['String', 'Integer'] AND coalesce(size(toStringOrNull({value})) {} {bound}, false))",
            cmp_op(*cmp)
        ),
        ValueTest::Matches(regex) => format!(
            "(typeOf({value}) IN ['String', 'Integer'] AND size(string.matchRegEx(toStringOrNull({value}), {})) > 0)",
            quote(&regex_pattern(regex)?)
        ),
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

/// An integer ordered like a temporal value of `type_name` (`yyyymmdd`,
/// `yyyymmddhhmmss` or `hhmmss`), null for other values. FalkorDB orders temporal
/// values wrongly when they are more than 2^31 seconds apart, so ranges compare keys.
// @lat: [[dialects#Dialect Backends#FalkorDB]]
fn temporal_key(value: &str, type_name: &str) -> String {
    let typed = format!("(CASE WHEN typeOf({value}) = '{type_name}' THEN {value} END)");
    let date = format!("{typed}.year * 10000 + {typed}.month * 100 + {typed}.day");
    let clock = format!("{typed}.hour * 10000 + {typed}.minute * 100 + {typed}.second");
    match type_name {
        "Date" => format!("({date})"),
        "Datetime" => format!("(({date}) * 1000000 + {clock})"),
        _ => format!("({clock})"),
    }
}

/// `left op right` for any two values: same-typed temporal values compare their keys,
/// everything else compares directly (null across types).
fn ordered(left: &str, op: &str, right: &str) -> String {
    let key = |value: &str| {
        format!(
            "CASE typeOf({value}) WHEN 'Date' THEN {} WHEN 'Datetime' THEN {} WHEN 'Time' THEN {} END",
            temporal_key(value, "Date"),
            temporal_key(value, "Datetime"),
            temporal_key(value, "Time")
        )
    };
    format!(
        "coalesce(CASE WHEN typeOf({left}) = typeOf({right}) AND typeOf({left}) IN ['Date', 'Datetime', 'Time'] THEN {} {op} {} ELSE {left} {op} {right} END, false)",
        key(left),
        key(right)
    )
}

/// The FalkorDB type name and ordering key of a date, date-time or time constant.
fn temporal_constant_key(
    value: &crate::ir::Constant,
) -> Result<Option<(&'static str, i64)>, RenderError> {
    use crate::ir::Constant;
    let type_name = match value {
        Constant::Date(_) => "Date",
        Constant::DateTime(_) => "Datetime",
        Constant::Time(_) => "Time",
        _ => return Ok(None),
    };
    // Rendering validates the constant and normalizes it to whole seconds.
    let rendered = constant(FALKORDB, value)?;
    let digits: String = rendered
        .split('\'')
        .nth(1)
        .unwrap_or_default()
        .chars()
        .filter(char::is_ascii_digit)
        .collect();
    let key = digits
        .parse()
        .map_err(|_| RenderError(format!("internal error: no ordering key for {rendered}")))?;
    Ok(Some((type_name, key)))
}

fn constant_list(constants: &[crate::ir::Constant]) -> Result<String, RenderError> {
    Ok(constants
        .iter()
        .map(|c| constant(FALKORDB, c))
        .collect::<Result<Vec<_>, _>>()?
        .join(", "))
}

fn datatype_test(value: &str, check: &DatatypeCheck) -> Result<String, RenderError> {
    let mut tests = Vec::new();
    for value_type in &check.allowed {
        if let Some(test) = type_test(value, value_type)? {
            tests.push(test);
        }
    }
    tests.sort();
    tests.dedup();
    let types = if tests.is_empty() {
        "false".to_owned()
    } else {
        tests.join(" OR ")
    };
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

/// A `typeOf` test for one value type; `None` for types FalkorDB cannot store.
fn type_test(value: &str, value_type: &ValueType) -> Result<Option<String>, RenderError> {
    let name = match value_type {
        ValueType::String => "String",
        ValueType::Int64
        | ValueType::Int32
        | ValueType::Int16
        | ValueType::Int8
        | ValueType::UInt64
        | ValueType::UInt32
        | ValueType::UInt16
        | ValueType::UInt8 => "Integer",
        ValueType::Double | ValueType::Float | ValueType::Decimal => "Float",
        ValueType::Boolean => "Boolean",
        ValueType::Date => "Date",
        ValueType::LocalDateTime => "Datetime",
        ValueType::LocalTime => "Time",
        ValueType::Duration => "Duration",
        ValueType::Point => "Point",
        ValueType::ZonedDateTime | ValueType::ZonedTime => return Ok(None),
        ValueType::Any => return Ok(Some(format!("typeOf({value}) <> 'Null'"))),
        ValueType::List(element) => {
            let item = format!("{value}e");
            let elements = format!("CASE WHEN typeOf({value}) = 'List' THEN {value} ELSE [] END");
            return Ok(Some(match type_test(&item, element)? {
                Some(test) => {
                    format!("(typeOf({value}) = 'List' AND all({item} IN {elements} WHERE {test}))")
                }
                None => format!("(typeOf({value}) = 'List' AND size({elements}) = 0)"),
            }));
        }
        ValueType::Blob => {
            return Err(RenderError(
                "BLOB values cannot be type-checked on FalkorDB".into(),
            ))
        }
    };
    Ok(Some(format!("typeOf({value}) = '{name}'")))
}

/// Oniguruma pattern with inline flags. `string.matchRegEx` already searches substrings,
/// so no wildcard wrapper is needed.
// @lat: [[semantics#Regex Translation]]
fn regex_pattern(regex: &XsdRegex) -> Result<String, RenderError> {
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
    Ok(format!("{prefix}{}", code_point_escapes(&regex.pattern)?))
}

/// Rewrites `\x{H}` escapes, which FalkorDB's regex engine never matches, to `\uHHHH`
/// within the Basic Multilingual Plane and to the literal character beyond it.
fn code_point_escapes(pattern: &str) -> Result<String, RenderError> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::with_capacity(pattern.len());
    let mut index = 0;
    while index < chars.len() {
        if chars[index] != '\\' || index + 1 == chars.len() {
            out.push(chars[index]);
            index += 1;
            continue;
        }
        if chars[index + 1] == 'x' && chars.get(index + 2) == Some(&'{') {
            if let Some(length) = chars[index + 3..].iter().position(|c| *c == '}') {
                let hex: String = chars[index + 3..index + 3 + length].iter().collect();
                let code = u32::from_str_radix(&hex, 16)
                    .ok()
                    .and_then(char::from_u32)
                    .ok_or_else(|| {
                        RenderError(format!("invalid code point escape \\x{{{hex}}}"))
                    })?;
                if u32::from(code) <= 0xFFFF {
                    let _ = write!(out, "\\u{:04X}", u32::from(code));
                } else {
                    out.push(code);
                }
                index += 4 + length;
                continue;
            }
        }
        out.push(chars[index]);
        out.push(chars[index + 1]);
        index += 2;
    }
    Ok(out)
}

/// `v:A` or `(v:A OR v:B)`: FalkorDB has no label expressions.
fn label_test(value: &str, labels: &[String]) -> String {
    let tests: Vec<String> = labels
        .iter()
        .map(|label| format!("{value}:{}", ident(label)))
        .collect();
    if tests.len() == 1 {
        tests[0].clone()
    } else {
        format!("({})", tests.join(" OR "))
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

fn bound(env: &Env, var: Var) -> Result<Binding, RenderError> {
    env.get(&var).cloned().ok_or_else(|| {
        RenderError(format!(
            "variable {var} is not available here: FalkorDB relationship subqueries cannot use list elements of an enclosing quantifier"
        ))
    })
}

fn row_bound(env: &Env, var: Var) -> Result<Binding, RenderError> {
    let binding = bound(env, var)?;
    require_row(&binding)?;
    Ok(binding)
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

    /// Exercises most IR constructs, as the Neo4j renderer tests do, plus nested traversals.
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
                [ sh:path ex:manager ; sh:node ex:ManagerShape ] ,
                [ sh:path [ sh:oneOrMorePath ex:knows ] ; sh:class ex:Person ] ,
                [ sh:path [ sh:alternativePath ( ex:email ex:phone ) ] ; sh:minCount 1 ] ;
    sh:or ( [ sh:path ex:email ; sh:minCount 1 ] [ sh:path ex:phone ; sh:minCount 1 ] ) .
ex:AddressShape sh:property [ sh:path ex:zip ; sh:minCount 1 ; sh:pattern \"^[0-9]{5}$\" ] .
ex:ManagerShape sh:property [ sh:path ex:worksFor ; sh:minCount 1 ; sh:node ex:CompanyShape ] .
ex:worksFor s2c:relationship \"WORKS_FOR\" .
ex:KnowsShape s2c:targetRelationship \"KNOWS\" ;
    sh:property [ sh:path ex:since ; sh:datatype xsd:date ; sh:maxCount 1 ] .
ex:CompanyShape sh:targetClass ex:Company ;
    sh:property [ sh:path [ sh:inversePath ex:worksFor ] ; sh:minCount 1 ] ,
                [ sh:path ex:legalName ; sh:minCount 1 ] .
ex:EmployeeShape sh:targetSubjectsOf ex:worksFor ;
    sh:property [ sh:path ex:badge ; sh:maxCount 1 ] .
ex:ClientShape sh:targetObjectsOf ex:servedBy ;
    sh:property [ sh:path ex:name ; sh:minCount 1 ] .
ex:servedBy s2c:relationship \"SERVED_BY\" .
ex:PairShape sh:targetClass ex:Team ;
    sh:property [ sh:path ex:lead ; sh:equals ex:owner ; sh:disjoint ex:member ] .
ex:lead s2c:relationship \"LEAD\" .
ex:owner s2c:relationship \"OWNER\" .
ex:member s2c:relationship \"MEMBER\" .
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
            .unwrap_or_else(|| {
                let ids: Vec<String> = rules.iter().map(|r| r.id.to_string()).collect();
                panic!("no rule {id}; rules: {ids:?}")
            });
        render_rule(rule).unwrap_or_else(|e| panic!("{id}: {e}"))
    }

    // @lat: [[tests#Compilation#FalkorDB Queries]]
    #[test]
    fn renders_every_compiled_rule_without_unsupported_constructs() {
        let rules = compile(SHAPES);
        assert!(rules.len() > 20, "{}", rules.len());
        let dump = std::env::var_os("S2C_RENDER_DUMP_FALKORDB").map(std::path::PathBuf::from);
        for (index, rule) in rules.iter().enumerate() {
            let queries = render_rule(rule).unwrap_or_else(|e| panic!("{}: {e}", rule.id));
            for query in [&queries.detail, &queries.summary] {
                for clause in ["CREATE ", "MERGE ", " SET ", "DELETE ", "REMOVE "] {
                    assert!(!query.contains(clause), "{}: {query}", rule.id);
                }
                let hazards = super::super::lint::falkordb_hazards(query);
                assert!(hazards.is_empty(), "{}: {hazards:?}\n{query}", rule.id);
            }
            assert!(queries.detail.ends_with("\nLIMIT $limit"), "{}", rule.id);
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
    fn traverses_relationships_in_correlated_subqueries() {
        let rules = compile(SHAPES);
        let class = rendered(&rules, "ex:PersonShape/ex:worksFor/sh:class");
        assert!(
            class.detail.contains("MATCH (v0)-[x1:`WORKS_FOR`]->(v1) WHERE id(x1) >= 0\nWITH DISTINCT v0, v1\nWITH v0, v1\nWHERE NOT (v1:`Company`)"),
            "{}",
            class.detail
        );
        assert!(class
            .detail
            .contains("{label: head(labels(v1)), keyValue: v1.`id`, elementId: toString(id(v1))}"));
        let max_count = rendered(&rules, "ex:PersonShape/ex:worksFor/sh:maxCount");
        assert!(
            max_count.detail.contains("CALL { WITH v0 OPTIONAL MATCH (v0)-[x2:`WORKS_FOR`]->(x1) WHERE id(x2) >= 0 RETURN count(DISTINCT x1) <= 1 AS c3 }\nWITH v0, c3\nWHERE NOT (c3)"),
            "{}",
            max_count.detail
        );
        let transitive = rendered(&rules, "ex:PersonShape/ex:knows+/sh:class");
        assert!(
            transitive
                .detail
                .contains("MATCH x1 = (v0)-[:`KNOWS`*1..10]->(v1)"),
            "{}",
            transitive.detail
        );
        let inverse = rendered(&rules, "ex:CompanyShape/^ex:worksFor/sh:minCount");
        assert!(
            inverse.detail.contains("(v0)<-[x2:`WORKS_FOR`]-(x1)"),
            "{}",
            inverse.detail
        );
    }

    #[test]
    fn nests_subqueries_for_nested_shapes() {
        let rules = compile(SHAPES);
        let manager = rendered(&rules, "ex:PersonShape/ex:manager/sh:node");
        let detail = &manager.detail;
        assert!(
            detail.contains("OPTIONAL MATCH (v1)-[") && detail.contains("CALL { WITH v0, v1, x"),
            "{detail}"
        );
        assert!(
            detail.contains("THEN 'ex:ManagerShape/ex:worksFor/sh:minCount' END"),
            "{detail}"
        );
        assert!(!detail.contains("[("), "{detail}");
    }

    #[test]
    fn keeps_value_tests_total() {
        let rules = compile(SHAPES);
        let pattern = rendered(&rules, "ex:PersonShape/ex:name/sh:pattern");
        assert!(pattern
            .detail
            .contains("size(string.matchRegEx(toStringOrNull(v1), '(?i)^[A-Z]')) > 0"));
        assert!(pattern
            .detail
            .contains("replace(replace('{$this} has a bad name {?value}', '{$this}', coalesce(toStringOrNull(v0.`id`), toString(id(v0)))), '{?value}', coalesce(toStringOrNull(v1), ''))"));
        let length = rendered(&rules, "ex:PersonShape/ex:name/sh:minLength");
        assert!(length
            .detail
            .contains("coalesce(size(toStringOrNull(v1)) >= 2, false)"));
        let datatype = rendered(&rules, "ex:PersonShape/ex:age/sh:datatype");
        assert!(datatype.detail.contains(
            "((typeOf(v1) = 'Integer') AND coalesce(v1 >= -32768 AND v1 <= 32767, false))"
        ));
        let min_count = rendered(&rules, "ex:PersonShape/ex:name/sh:minCount");
        assert!(min_count.detail.contains(
            "size(reduce(x2 = [], x1 IN CASE typeOf(v0.`name`) WHEN 'Null' THEN [] WHEN 'List' THEN v0.`name` ELSE [v0.`name`] END"
        ));
    }

    #[test]
    fn renders_pairs_closed_targets_and_relationship_focus() {
        let rules = compile(SHAPES);
        let equals = rendered(&rules, "ex:PairShape/ex:lead/sh:equals");
        assert!(
            equals.detail.contains("collect(DISTINCT id("),
            "{}",
            equals.detail
        );
        let disjoint = rendered(&rules, "ex:PairShape/ex:lead/sh:disjoint");
        assert!(disjoint.detail.contains("none("), "{}", disjoint.detail);
        let closed = rendered(&rules, "ex:PersonShape/sh:closed");
        assert!(closed.detail.contains("IN keys(v0) WHERE"));
        assert!(closed.detail.contains("RETURN collect(DISTINCT type("));
        let subjects = rendered(&rules, "ex:EmployeeShape/ex:badge/sh:maxCount");
        assert!(
            subjects
                .detail
                .starts_with("MATCH (v0)\nCALL { WITH v0 OPTIONAL MATCH (v0)-["),
            "{}",
            subjects.detail
        );
        let objects = rendered(&rules, "ex:ClientShape/ex:name/sh:minCount");
        assert!(
            objects.detail.contains("]->(v0) WHERE id("),
            "{}",
            objects.detail
        );
        let since = rendered(&rules, "ex:KnowsShape/ex:since/sh:datatype");
        assert!(since
            .detail
            .starts_with("MATCH (s0)-[v0:`KNOWS`]->(e0)\nWHERE id(v0) >= 0"));
        assert!(since.detail.contains(
            "{type: type(v0), startKey: s0.`id`, endKey: e0.`id`, elementId: toString(id(v0))}"
        ));
        assert!(since.summary.contains("WITH DISTINCT s0, v0, e0, v1"));
    }

    // @lat: [[tests#Compilation#Unrepresentable FalkorDB Identifiers]]
    #[test]
    fn rejects_identifiers_with_backticks() {
        assert!(unrepresentable_identifier("a`b"));
        assert!(!unrepresentable_identifier("a\\u0041 b.c"));
        let rules = compile(
            "ex:S sh:targetClass ex:Person ;
    sh:property [ sh:path ex:p ; s2c:property \"we`ird\" ; sh:minCount 1 ] .
",
        );
        let error = render_rule(&rules[0]).unwrap_err().to_string();
        assert!(error.contains("FalkorDB cannot escape"), "{error}");
    }

    #[test]
    fn compares_temporal_values_through_ordering_keys() {
        use crate::ir::Constant;
        let compare = |cmp, bound| value_test("v1", &ValueTest::Compare { cmp, bound });
        assert_eq!(
            compare(Cmp::Gt, Constant::Date("1900-01-01".into())).unwrap(),
            "coalesce(((CASE WHEN typeOf(v1) = 'Date' THEN v1 END).year * 10000 + (CASE WHEN typeOf(v1) = 'Date' THEN v1 END).month * 100 + (CASE WHEN typeOf(v1) = 'Date' THEN v1 END).day) > 19000101, false)"
        );
        let datetime = compare(
            Cmp::Le,
            Constant::DateTime("2020-01-01T10:00:00.000".into()),
        )
        .unwrap();
        assert!(datetime.contains("typeOf(v1) = 'Datetime'"), "{datetime}");
        assert!(
            datetime.ends_with("<= 20200101100000, false)"),
            "{datetime}"
        );
        let time = compare(Cmp::Ge, Constant::Time("23:59:59".into())).unwrap();
        assert!(time.ends_with(">= 235959, false)"), "{time}");
        assert_eq!(
            compare(Cmp::Ge, Constant::Integer(3)).unwrap(),
            "coalesce(v1 >= 3, false)"
        );
        let duration = compare(Cmp::Gt, Constant::Duration("P1D".into())).unwrap_err();
        assert!(duration.to_string().contains("FalkorDB"), "{duration}");
        let pair = ordered("x1", "<", "x2");
        assert!(
            pair.contains("typeOf(x1) = typeOf(x2) AND typeOf(x1) IN ['Date', 'Datetime', 'Time']"),
            "{pair}"
        );
        assert!(pair.ends_with("ELSE x1 < x2 END, false)"), "{pair}");
    }

    #[test]
    fn rewrites_code_point_escapes() {
        assert_eq!(
            code_point_escapes(r"[\x{41}-\x{5A}\x{1F600}]\\x{41}\.").unwrap(),
            "[\\u0041-\\u005A\u{1F600}]\\\\x{41}\\."
        );
        assert_eq!(
            code_point_escapes(r"[^\x{0}-\x{10FFFF}]").unwrap(),
            "[^\\u0000-\u{10FFFF}]"
        );
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
