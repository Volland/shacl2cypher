//! Dialect-neutral intermediate representation of compiled constraints.
//!
//! Every test is total: value tests only reference variables bound by a quantifier
//! over a value set, and value sets never contain nulls, so renderers translate them
//! without three-valued logic.

use std::collections::BTreeSet;
use std::fmt;

use oxrdf::Literal;

use crate::ast::{Severity, ShapeId};
use crate::datatypes::DatatypeCheck;
use crate::load::SourceLocation;
use crate::mapping::LpgPath;
use crate::xsd_regex::XsdRegex;

const XSD: &str = "http://www.w3.org/2001/XMLSchema#";

/// A variable bound to a node, relationship or property value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Var(pub u32);

impl Var {
    /// The focus node (or relationship) of a rule.
    pub const FOCUS: Var = Var(0);
}

impl fmt::Display for Var {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "v{}", self.0)
    }
}

/// A typed constant taken from the shapes; lexical forms are kept verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Constant {
    String(String),
    Integer(i128),
    Decimal(String),
    Double(String),
    Boolean(bool),
    Date(String),
    DateTime(String),
    Time(String),
    Duration(String),
}

impl Constant {
    pub fn from_literal(literal: &Literal) -> Result<Self, String> {
        if literal.language().is_some() {
            return Err(format!(
                "language-tagged literal {literal} is not supported: labeled property graphs have no language tags"
            ));
        }
        let value = literal.value();
        let invalid = || format!("invalid literal {literal}");
        let Some(local) = literal.datatype().as_str().strip_prefix(XSD) else {
            return Err(format!("literal {literal} has an unsupported datatype"));
        };
        Ok(match local {
            "string" | "normalizedString" | "token" | "anyURI" => {
                Constant::String(value.to_owned())
            }
            "boolean" => match value {
                "true" | "1" => Constant::Boolean(true),
                "false" | "0" => Constant::Boolean(false),
                _ => return Err(invalid()),
            },
            "integer" | "long" | "int" | "short" | "byte" | "nonNegativeInteger"
            | "positiveInteger" | "nonPositiveInteger" | "negativeInteger" | "unsignedLong"
            | "unsignedInt" | "unsignedShort" | "unsignedByte" => Constant::Integer(
                value
                    .strip_prefix('+')
                    .unwrap_or(value)
                    .parse()
                    .map_err(|_| invalid())?,
            ),
            "decimal" if is_decimal(value) => Constant::Decimal(value.to_owned()),
            "decimal" => return Err(invalid()),
            "double" | "float" => match value.parse::<f64>() {
                Ok(number) if number.is_finite() => Constant::Double(value.to_owned()),
                _ => return Err(format!("{} (only finite numbers are supported)", invalid())),
            },
            "date" => Constant::Date(value.to_owned()),
            "dateTime" | "dateTimeStamp" => Constant::DateTime(value.to_owned()),
            "time" => Constant::Time(value.to_owned()),
            "duration" | "dayTimeDuration" | "yearMonthDuration" => {
                Constant::Duration(value.to_owned())
            }
            _ => return Err(format!("literal {literal} has an unsupported datatype")),
        })
    }
}

fn is_decimal(value: &str) -> bool {
    let unsigned = value.strip_prefix(['+', '-']).unwrap_or(value);
    let (integer, fraction) = unsigned.split_once('.').unwrap_or((unsigned, ""));
    !(integer.is_empty() && fraction.is_empty())
        && integer.chars().all(|c| c.is_ascii_digit())
        && fraction.chars().all(|c| c.is_ascii_digit())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cmp {
    Lt,
    Le,
    Eq,
    Ge,
    Gt,
}

impl Cmp {
    pub fn holds(self, left: u64, right: u64) -> bool {
        match self {
            Cmp::Lt => left < right,
            Cmp::Le => left <= right,
            Cmp::Eq => left == right,
            Cmp::Ge => left >= right,
            Cmp::Gt => left > right,
        }
    }
}

/// Where the values of a quantifier come from, relative to a bound variable.
#[derive(Debug, Clone, PartialEq, Eq)]
// @lat: [[mapping#Value Sets]]
pub enum ValueSource {
    /// The bound node itself (constraints declared on node shapes).
    Focus,
    /// Distinct non-null values reached over a resolved path.
    Path(LpgPath),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PairRelation {
    Equals,
    Disjoint,
    LessThan,
    LessThanOrEquals,
}

/// A test on one bound value; false for values of the wrong kind or type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValueTest {
    Datatype(DatatypeCheck),
    /// A node carrying any of the labels.
    HasLabel(Vec<String>),
    Compare {
        cmp: Cmp,
        bound: Constant,
    },
    /// String length compared with a bound.
    Length {
        cmp: Cmp,
        bound: u64,
    },
    Matches(XsdRegex),
    In(Vec<Constant>),
    /// A node whose key property equals one of the values (IRI constants on relationship values).
    NodeKeyIn {
        key: String,
        values: Vec<Constant>,
    },
}

/// A total boolean expression.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Expr {
    Bool(bool),
    Not(Box<Expr>),
    And(Vec<Expr>),
    Or(Vec<Expr>),
    /// Exactly one operand holds.
    Xone(Vec<Expr>),
    /// Every value of `source` from `of`, bound to `var`, satisfies `test`.
    All {
        of: Var,
        source: ValueSource,
        var: Var,
        test: Box<Expr>,
    },
    /// The number of distinct values of `source` from `of` (satisfying `filter`) compared with `bound`.
    Count {
        of: Var,
        source: ValueSource,
        var: Var,
        filter: Option<Box<Expr>>,
        cmp: Cmp,
        bound: u64,
    },
    Test {
        var: Var,
        test: ValueTest,
    },
    /// Relation between the value sets of `left` and `right` from `of`.
    Pair {
        of: Var,
        left: ValueSource,
        right: LpgPath,
        relation: PairRelation,
    },
    /// `of` has no properties besides `allowed` and no outgoing relationships
    /// besides `allowed_relationships`.
    Closed {
        of: Var,
        allowed: Vec<String>,
        allowed_relationships: Vec<String>,
    },
}

impl Expr {
    // @lat: [[dialects#IR]]
    pub fn negate(expr: Expr) -> Expr {
        match expr {
            Expr::Bool(value) => Expr::Bool(!value),
            Expr::Not(inner) => *inner,
            other => Expr::Not(Box::new(other)),
        }
    }

    pub fn and(items: impl IntoIterator<Item = Expr>) -> Expr {
        let mut operands = Vec::new();
        for item in items {
            match item {
                Expr::Bool(true) => {}
                Expr::Bool(false) => return Expr::Bool(false),
                Expr::And(inner) => operands.extend(inner),
                other => operands.push(other),
            }
        }
        match operands.len() {
            0 => Expr::Bool(true),
            1 => operands.remove(0),
            _ => Expr::And(operands),
        }
    }

    pub fn or(items: impl IntoIterator<Item = Expr>) -> Expr {
        let mut operands = Vec::new();
        for item in items {
            match item {
                Expr::Bool(false) => {}
                Expr::Bool(true) => return Expr::Bool(true),
                Expr::Or(inner) => operands.extend(inner),
                other => operands.push(other),
            }
        }
        match operands.len() {
            0 => Expr::Bool(false),
            1 => operands.remove(0),
            _ => Expr::Or(operands),
        }
    }

    pub fn xone(items: impl IntoIterator<Item = Expr>) -> Expr {
        let mut trues = 0;
        let mut operands = Vec::new();
        for item in items {
            match item {
                Expr::Bool(true) => trues += 1,
                Expr::Bool(false) => {}
                other => operands.push(other),
            }
        }
        match (trues, operands.len()) {
            (0, 0) => Expr::Bool(false),
            (0, 1) => operands.remove(0),
            (0, _) => Expr::Xone(operands),
            (1, _) => Expr::negate(Expr::or(operands)),
            _ => Expr::Bool(false),
        }
    }

    pub fn all(of: Var, source: ValueSource, var: Var, test: Expr) -> Expr {
        if test == Expr::Bool(true) {
            return Expr::Bool(true);
        }
        Expr::All {
            of,
            source,
            var,
            test: Box::new(test),
        }
    }

    pub fn count(
        of: Var,
        source: ValueSource,
        var: Var,
        filter: Option<Expr>,
        cmp: Cmp,
        bound: u64,
    ) -> Expr {
        let filter = match filter {
            None | Some(Expr::Bool(true)) => None,
            Some(Expr::Bool(false)) => return Expr::Bool(cmp.holds(0, bound)),
            Some(other) => Some(Box::new(other)),
        };
        if cmp == Cmp::Ge && bound == 0 {
            return Expr::Bool(true);
        }
        Expr::Count {
            of,
            source,
            var,
            filter,
            cmp,
            bound,
        }
    }

    pub fn test(var: Var, test: ValueTest) -> Expr {
        Expr::Test { var, test }
    }

    /// Variables used but not bound within the expression.
    pub fn free_vars(&self) -> BTreeSet<Var> {
        let mut vars = BTreeSet::new();
        self.collect_free(&mut vars);
        vars
    }

    fn collect_free(&self, out: &mut BTreeSet<Var>) {
        match self {
            Expr::Bool(_) => {}
            Expr::Not(inner) => inner.collect_free(out),
            Expr::And(items) | Expr::Or(items) | Expr::Xone(items) => {
                items.iter().for_each(|item| item.collect_free(out));
            }
            Expr::All { of, var, test, .. } => {
                out.insert(*of);
                let mut inner = test.free_vars();
                inner.remove(var);
                out.extend(inner);
            }
            Expr::Count {
                of, var, filter, ..
            } => {
                out.insert(*of);
                if let Some(filter) = filter {
                    let mut inner = filter.free_vars();
                    inner.remove(var);
                    out.extend(inner);
                }
            }
            Expr::Test { var, .. } => {
                out.insert(*var);
            }
            Expr::Pair { of, .. } | Expr::Closed { of, .. } => {
                out.insert(*of);
            }
        }
    }
}

/// Focus nodes of a rule; a rule's focus is the union of its sets.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FocusSet {
    Labels(Vec<String>),
    SubjectsOf(LpgPath),
    ObjectsOf(LpgPath),
    Relationships(String),
}

/// How violation rows are produced, relative to the variable `of`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Violation {
    /// One row per value of `source`, bound to `var`, that fails `test`.
    PerValue {
        of: Var,
        source: ValueSource,
        var: Var,
        test: Expr,
    },
    /// One row per `of` that fails `condition`.
    PerFocus { of: Var, condition: Expr },
}

impl Violation {
    /// The condition under which `of` satisfies the constraint.
    pub fn holds(&self) -> Expr {
        match self {
            Violation::PerValue {
                of,
                source,
                var,
                test,
            } => Expr::all(*of, source.clone(), *var, test.clone()),
            Violation::PerFocus { condition, .. } => condition.clone(),
        }
    }
}

/// Structural rule id segments, e.g. `ex:PersonShape`, `ex:name`, `sh:minCount`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RuleId(pub Vec<String>);

impl fmt::Display for RuleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.join("/"))
    }
}

/// An inner rule reported in `details` when it fails for the violating value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Detail {
    pub rule: RuleId,
    pub holds: Expr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleStatus {
    Compiled,
    GuaranteedBySchema,
    Deactivated,
    Unsupported(String),
    /// Contradicted by the enforced schema; reported as a static diagnostic.
    SchemaMismatch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rule {
    pub id: RuleId,
    /// The targeted shape the rule was compiled for.
    pub shape: ShapeId,
    /// The shape declaring the constraint: the targeted shape or one of its property shapes.
    pub declared_by: ShapeId,
    pub location: SourceLocation,
    pub severity: Severity,
    pub messages: Vec<Literal>,
    pub focus: Vec<FocusSet>,
    pub violation: Violation,
    pub details: Vec<Detail>,
    pub status: RuleStatus,
    /// Depth at which unbounded repeated paths were capped, if any.
    pub path_depth_cap: Option<u32>,
}

impl Rule {
    /// Checks the IR invariant: expressions only reference bound variables.
    pub fn validate(&self) -> Result<(), String> {
        let (of, bound) = match &self.violation {
            Violation::PerValue { of, var, test, .. } => {
                let bound = vec![Var::FOCUS, *var];
                self.check_bound(test, &bound, "test")?;
                (*of, bound)
            }
            Violation::PerFocus { of, condition } => {
                let bound = vec![Var::FOCUS];
                self.check_bound(condition, &bound, "condition")?;
                (*of, bound)
            }
        };
        if of != Var::FOCUS {
            return Err(format!(
                "rule {}: violations must be relative to the focus",
                self.id
            ));
        }
        for detail in &self.details {
            self.check_bound(&detail.holds, &bound, &format!("detail {}", detail.rule))?;
        }
        Ok(())
    }

    fn check_bound(&self, expr: &Expr, bound: &[Var], what: &str) -> Result<(), String> {
        let unbound: Vec<String> = expr
            .free_vars()
            .into_iter()
            .filter(|var| !bound.contains(var))
            .map(|var| var.to_string())
            .collect();
        if unbound.is_empty() {
            Ok(())
        } else {
            Err(format!(
                "rule {}: {what} references unbound variables {}",
                self.id,
                unbound.join(", ")
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapping::{Evidence, Resolved};
    use oxrdf::NamedNode;

    fn typed(value: &str, local: &str) -> Literal {
        Literal::new_typed_literal(value, NamedNode::new_unchecked(format!("{XSD}{local}")))
    }

    fn name_path() -> ValueSource {
        ValueSource::Path(LpgPath::Property(Resolved {
            name: "name".into(),
            evidence: Evidence::Convention,
        }))
    }

    fn length_test(var: Var) -> Expr {
        Expr::test(
            var,
            ValueTest::Length {
                cmp: Cmp::Ge,
                bound: 1,
            },
        )
    }

    #[test]
    fn converts_literals_to_typed_constants() {
        assert_eq!(
            Constant::from_literal(&Literal::new_simple_literal("a")),
            Ok(Constant::String("a".into()))
        );
        assert_eq!(
            Constant::from_literal(&typed("+5", "integer")),
            Ok(Constant::Integer(5))
        );
        assert_eq!(
            Constant::from_literal(&typed("1", "boolean")),
            Ok(Constant::Boolean(true))
        );
        assert_eq!(
            Constant::from_literal(&typed("-1.50", "decimal")),
            Ok(Constant::Decimal("-1.50".into()))
        );
        assert_eq!(
            Constant::from_literal(&typed("1.5e3", "double")),
            Ok(Constant::Double("1.5e3".into()))
        );
        assert_eq!(
            Constant::from_literal(&typed("2020-01-01", "date")),
            Ok(Constant::Date("2020-01-01".into()))
        );
        for (literal, expected) in [
            (
                Literal::new_language_tagged_literal_unchecked("a", "en"),
                "no language tags",
            ),
            (typed("2020", "gYear"), "unsupported datatype"),
            (typed("x", "integer"), "invalid literal"),
            (typed(".", "decimal"), "invalid literal"),
            (typed("INF", "double"), "only finite numbers"),
        ] {
            let message = Constant::from_literal(&literal).unwrap_err();
            assert!(message.contains(expected), "{message}");
        }
    }

    #[test]
    fn constructors_fold_constants() {
        let e = length_test(Var(1));
        assert_eq!(Expr::negate(Expr::negate(e.clone())), e);
        assert_eq!(Expr::negate(Expr::Bool(true)), Expr::Bool(false));
        assert_eq!(Expr::and([Expr::Bool(true), e.clone()]), e);
        assert_eq!(Expr::and([Expr::Bool(false), e.clone()]), Expr::Bool(false));
        assert_eq!(Expr::and([]), Expr::Bool(true));
        assert_eq!(Expr::or([Expr::Bool(true), e.clone()]), Expr::Bool(true));
        assert_eq!(Expr::or([Expr::Bool(false)]), Expr::Bool(false));
        assert_eq!(
            Expr::and([Expr::and([e.clone(), e.clone()]), e.clone()]),
            Expr::And(vec![e.clone(), e.clone(), e.clone()])
        );
        assert_eq!(
            Expr::xone([Expr::Bool(false), e.clone(), e.clone()]),
            Expr::Xone(vec![e.clone(), e.clone()])
        );
        assert_eq!(
            Expr::xone([Expr::Bool(true), e.clone()]),
            Expr::negate(e.clone())
        );
        assert_eq!(
            Expr::xone([Expr::Bool(true), Expr::Bool(true), e.clone()]),
            Expr::Bool(false)
        );
        let count =
            |filter, cmp, bound| Expr::count(Var::FOCUS, name_path(), Var(1), filter, cmp, bound);
        assert_eq!(count(Some(Expr::Bool(false)), Cmp::Le, 3), Expr::Bool(true));
        assert_eq!(
            count(Some(Expr::Bool(false)), Cmp::Ge, 1),
            Expr::Bool(false)
        );
        assert_eq!(count(None, Cmp::Ge, 0), Expr::Bool(true));
        assert!(matches!(
            count(Some(Expr::Bool(true)), Cmp::Ge, 1),
            Expr::Count { filter: None, .. }
        ));
        assert_eq!(
            Expr::all(Var::FOCUS, name_path(), Var(1), Expr::Bool(true)),
            Expr::Bool(true)
        );
    }

    #[test]
    fn tracks_free_variables_through_quantifiers() {
        let all = Expr::all(Var::FOCUS, name_path(), Var(1), length_test(Var(1)));
        assert_eq!(all.free_vars(), BTreeSet::from([Var::FOCUS]));
        let leaking = Expr::all(Var::FOCUS, name_path(), Var(1), length_test(Var(2)));
        assert_eq!(leaking.free_vars(), BTreeSet::from([Var::FOCUS, Var(2)]));
    }

    #[test]
    fn validation_rejects_unbound_variables() {
        let rule = |test: Expr, details: Vec<Detail>| Rule {
            id: RuleId(vec!["ex:S".into(), "ex:name".into(), "sh:minLength".into()]),
            shape: NamedNode::new_unchecked("http://example.org/S").into(),
            declared_by: NamedNode::new_unchecked("http://example.org/S").into(),
            location: SourceLocation { file: 0, line: 1 },
            severity: Severity::Violation,
            messages: Vec::new(),
            focus: vec![FocusSet::Labels(vec!["Person".into()])],
            violation: Violation::PerValue {
                of: Var::FOCUS,
                source: name_path(),
                var: Var(1),
                test,
            },
            details,
            status: RuleStatus::Compiled,
            path_depth_cap: None,
        };
        assert_eq!(rule(length_test(Var(1)), Vec::new()).validate(), Ok(()));
        let message = rule(length_test(Var(7)), Vec::new())
            .validate()
            .unwrap_err();
        assert!(message.contains("unbound variables v7"), "{message}");
        let detail = Detail {
            rule: RuleId(vec!["ex:Inner".into()]),
            holds: length_test(Var(9)),
        };
        assert!(rule(length_test(Var(1)), vec![detail])
            .validate()
            .unwrap_err()
            .contains("detail ex:Inner"));
        assert_eq!(
            RuleId(vec!["ex:S".into(), "ex:name".into(), "sh:minLength".into()]).to_string(),
            "ex:S/ex:name/sh:minLength"
        );
    }
}
