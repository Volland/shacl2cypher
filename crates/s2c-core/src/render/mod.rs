//! Cypher rendering of IR rules: pieces shared by the Neo4j and LadybugDB backends.

use crate::ast::Direction;
use crate::ir::Constant;
use crate::mapping::LpgPath;

pub mod ladybug;
#[cfg(test)]
mod lint;
pub mod neo4j;
mod rule;

pub use rule::{Rendered, RuleMeta};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dialect {
    Neo4j,
    Ladybug,
}

impl Dialect {
    pub fn name(self) -> &'static str {
        match self {
            Dialect::Neo4j => "neo4j",
            Dialect::Ladybug => "ladybug",
        }
    }

    /// Largest variable-length upper bound the database accepts.
    pub fn max_path_depth(self) -> Option<u32> {
        match self {
            Dialect::Neo4j => None,
            Dialect::Ladybug => Some(30),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct RenderError(pub String);

/// Single-quoted string literal; only `\` and `'` need escaping in either dialect.
pub fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('\'');
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\'' => out.push_str("\\'"),
            c => out.push(c),
        }
    }
    out.push('\'');
    out
}

/// Backtick-quoted identifier with embedded backticks doubled.
pub fn ident(name: &str) -> String {
    format!("`{}`", name.replace('`', "``"))
}

/// A typed constant as a Cypher expression.
// @lat: [[dialects#Literals and Identifiers]]
pub fn constant(dialect: Dialect, value: &Constant) -> Result<String, RenderError> {
    Ok(match value {
        Constant::String(text) => quote(text),
        Constant::Integer(number) => i64::try_from(*number)
            .map_err(|_| RenderError(format!("integer {number} does not fit a 64-bit integer")))?
            .to_string(),
        Constant::Decimal(lexical) => decimal_literal(lexical),
        Constant::Double(lexical) => {
            let number: f64 = lexical
                .parse()
                .map_err(|_| RenderError(format!("invalid double {lexical}")))?;
            format!("{number:?}")
        }
        Constant::Boolean(flag) => flag.to_string(),
        Constant::Date(lexical) => format!("date({})", quote(lexical)),
        Constant::DateTime(lexical) => match (dialect, has_timezone(lexical)) {
            (Dialect::Neo4j, true) => format!("datetime({})", quote(lexical)),
            (Dialect::Neo4j, false) => format!("localdatetime({})", quote(lexical)),
            (Dialect::Ladybug, true) => format!("CAST({} AS TIMESTAMP_TZ)", quote(lexical)),
            (Dialect::Ladybug, false) => format!("timestamp({})", quote(lexical)),
        },
        Constant::Time(lexical) => match dialect {
            Dialect::Neo4j if has_timezone(lexical) => format!("time({})", quote(lexical)),
            Dialect::Neo4j => format!("localtime({})", quote(lexical)),
            Dialect::Ladybug => {
                return Err(RenderError(format!(
                "xsd:time constant {lexical} is not supported on LadybugDB, which has no time type"
            )))
            }
        },
        Constant::Duration(lexical) => match dialect {
            Dialect::Neo4j => format!("duration({})", quote(lexical)),
            Dialect::Ladybug => format!(
                "interval({})",
                quote(&duration_words(lexical).map_err(RenderError)?)
            ),
        },
    })
}

/// XSD decimal lexical form as a numeric literal (`.5` -> `0.5`, `5.` -> `5.0`).
fn decimal_literal(lexical: &str) -> String {
    let (sign, unsigned) = match lexical.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", lexical.strip_prefix('+').unwrap_or(lexical)),
    };
    let (integer, fraction) = unsigned.split_once('.').unwrap_or((unsigned, "0"));
    let integer = if integer.is_empty() { "0" } else { integer };
    let fraction = if fraction.is_empty() { "0" } else { fraction };
    format!("{sign}{integer}.{fraction}")
}

/// Whether an XSD date-time or time lexical form ends with a timezone.
fn has_timezone(lexical: &str) -> bool {
    let bytes = lexical.as_bytes();
    let len = bytes.len();
    lexical.ends_with('Z')
        || (len >= 6 && matches!(bytes[len - 6], b'+' | b'-') && bytes[len - 3] == b':')
}

/// `P1Y2M3DT4H5M6.5S` -> `1 years 2 months 3 days 4 hours 5 minutes 6.5 seconds`.
fn duration_words(lexical: &str) -> Result<String, String> {
    let invalid = || format!("invalid xsd:duration {lexical}");
    if lexical.starts_with('-') {
        return Err(format!(
            "negative duration {lexical} is not supported on LadybugDB"
        ));
    }
    let body = lexical.strip_prefix('P').ok_or_else(invalid)?;
    let (date_part, time_part) = match body.split_once('T') {
        Some((date, time)) => (date, Some(time)),
        None => (body, None),
    };
    let mut words = Vec::new();
    collect_units(
        date_part,
        &[('Y', "years"), ('M', "months"), ('D', "days")],
        &mut words,
    )
    .ok_or_else(invalid)?;
    if let Some(time) = time_part {
        if time.is_empty() {
            return Err(invalid());
        }
        collect_units(
            time,
            &[('H', "hours"), ('M', "minutes"), ('S', "seconds")],
            &mut words,
        )
        .ok_or_else(invalid)?;
    }
    if words.is_empty() {
        return Err(invalid());
    }
    Ok(words.join(" "))
}

/// Parses `<number><designator>` groups in designator order.
fn collect_units(text: &str, units: &[(char, &str)], words: &mut Vec<String>) -> Option<()> {
    let mut rest = text;
    let mut next_unit = 0;
    while !rest.is_empty() {
        let end = rest.find(|c: char| !(c.is_ascii_digit() || c == '.'))?;
        let (number, tail) = rest.split_at(end);
        let designator = tail.chars().next()?;
        let position = units[next_unit..]
            .iter()
            .position(|(unit, _)| *unit == designator)?;
        let (_, word) = units[next_unit + position];
        if number.is_empty() || (number.contains('.') && designator != 'S') {
            return None;
        }
        words.push(format!("{number} {word}"));
        next_unit += position + 1;
        rest = &tail[designator.len_utf8()..];
    }
    Some(())
}

/// One relationship step of a route; `min == max == 1` for a plain hop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hop {
    pub types: Vec<String>,
    pub direction: Direction,
    pub min: u32,
    pub max: u32,
}

/// A way to reach values: relationship hops, then optionally a property.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Route {
    pub hops: Vec<Hop>,
    pub property: Option<String>,
}

/// Flattens a resolved path into alternative routes.
pub fn routes(path: &LpgPath) -> Result<Vec<Route>, RenderError> {
    match path {
        LpgPath::Property(key) => Ok(vec![Route {
            hops: Vec::new(),
            property: Some(key.name.clone()),
        }]),
        LpgPath::Relationship {
            rel_type,
            direction,
        } => Ok(vec![Route {
            hops: vec![Hop {
                types: vec![rel_type.name.clone()],
                direction: *direction,
                min: 1,
                max: 1,
            }],
            property: None,
        }]),
        LpgPath::Sequence(steps) => {
            let mut acc = vec![Route {
                hops: Vec::new(),
                property: None,
            }];
            for step in steps {
                let step_routes = routes(step)?;
                let mut next = Vec::new();
                for prefix in &acc {
                    if prefix.property.is_some() {
                        return Err(RenderError(
                            "a property step can only end a sequence path".into(),
                        ));
                    }
                    for route in &step_routes {
                        let mut hops = prefix.hops.clone();
                        hops.extend(route.hops.iter().cloned());
                        next.push(Route {
                            hops,
                            property: route.property.clone(),
                        });
                    }
                }
                acc = next;
            }
            Ok(acc)
        }
        LpgPath::Alternative(options) => {
            let mut all = Vec::new();
            for option in options {
                all.extend(routes(option)?);
            }
            Ok(merge_single_hops(all))
        }
        LpgPath::Repeat { path, min, max } => {
            let Some(max) = max else {
                return Err(RenderError(
                    "internal error: unbounded repetition reached the renderer".into(),
                ));
            };
            match routes(path)?.as_slice() {
                [Route { hops, property: None }]
                    if hops.len() == 1 && hops[0].min == 1 && hops[0].max == 1 =>
                {
                    Ok(vec![Route {
                        hops: vec![Hop {
                            types: hops[0].types.clone(),
                            direction: hops[0].direction,
                            min: *min,
                            max: *max,
                        }],
                        property: None,
                    }])
                }
                _ => Err(RenderError(
                    "repetition (`*`, `+`, `?`) is only supported over a single relationship or an alternative of relationships in one direction".into(),
                )),
            }
        }
    }
}

/// Alternatives of plain single hops in one direction become one multi-type hop.
fn merge_single_hops(routes: Vec<Route>) -> Vec<Route> {
    let mergeable = routes.len() > 1
        && routes.iter().all(|route| {
            route.property.is_none()
                && route.hops.len() == 1
                && route.hops[0].min == 1
                && route.hops[0].max == 1
                && route.hops[0].direction == routes[0].hops[0].direction
        });
    if !mergeable {
        return routes;
    }
    let mut types: Vec<String> = routes
        .iter()
        .flat_map(|route| route.hops[0].types.iter().cloned())
        .collect();
    types.sort();
    types.dedup();
    vec![Route {
        hops: vec![Hop {
            types,
            direction: routes[0].hops[0].direction,
            min: 1,
            max: 1,
        }],
        property: None,
    }]
}

/// A relationship pattern such as `-[:A|B*1..5]->`, to be placed between node patterns.
pub fn hop_pattern(dialect: Dialect, hop: &Hop) -> String {
    let separator = match dialect {
        Dialect::Neo4j => "|",
        Dialect::Ladybug => "|:",
    };
    let types: Vec<String> = hop.types.iter().map(|t| ident(t)).collect();
    let length = if hop.min == 1 && hop.max == 1 {
        String::new()
    } else {
        format!("*{}..{}", hop.min, hop.max)
    };
    let body = format!("[:{}{length}]", types.join(separator));
    match hop.direction {
        Direction::Out => format!("-{body}->"),
        Direction::In => format!("<-{body}-"),
    }
}

/// The pattern between the start and end nodes of a route, e.g. `-[:A]->()-[:B]->`.
pub fn chain(dialect: Dialect, hops: &[Hop]) -> String {
    hops.iter()
        .map(|hop| hop_pattern(dialect, hop))
        .collect::<Vec<_>>()
        .join("()")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapping::{Evidence, Resolved};

    fn resolved(name: &str) -> Resolved {
        Resolved {
            name: name.into(),
            evidence: Evidence::Convention,
        }
    }

    fn property(name: &str) -> LpgPath {
        LpgPath::Property(resolved(name))
    }

    fn relationship(name: &str, direction: Direction) -> LpgPath {
        LpgPath::Relationship {
            rel_type: resolved(name),
            direction,
        }
    }

    fn hop(types: &[&str], direction: Direction, min: u32, max: u32) -> Hop {
        Hop {
            types: types.iter().map(|t| (*t).to_owned()).collect(),
            direction,
            min,
            max,
        }
    }

    #[test]
    fn quotes_strings_and_identifiers() {
        assert_eq!(quote("O'Brien \\ x\ny"), "'O\\'Brien \\\\ x\ny'");
        assert_eq!(ident("We`ird Label"), "`We``ird Label`");
    }

    #[test]
    fn renders_constants_per_dialect() {
        let both = |value: Constant| {
            (
                constant(Dialect::Neo4j, &value).unwrap(),
                constant(Dialect::Ladybug, &value).unwrap(),
            )
        };
        assert_eq!(both(Constant::Integer(-5)), ("-5".into(), "-5".into()));
        assert_eq!(
            both(Constant::Decimal("+.5".into())),
            ("0.5".into(), "0.5".into())
        );
        assert_eq!(
            both(Constant::Double("1.5e3".into())),
            ("1500.0".into(), "1500.0".into())
        );
        assert_eq!(
            both(Constant::Date("2020-01-01".into())),
            ("date('2020-01-01')".into(), "date('2020-01-01')".into())
        );
        assert_eq!(
            both(Constant::DateTime("2020-01-01T10:00:00Z".into())),
            (
                "datetime('2020-01-01T10:00:00Z')".into(),
                "CAST('2020-01-01T10:00:00Z' AS TIMESTAMP_TZ)".into()
            )
        );
        assert_eq!(
            both(Constant::DateTime("2020-01-01T10:00:00".into())),
            (
                "localdatetime('2020-01-01T10:00:00')".into(),
                "timestamp('2020-01-01T10:00:00')".into()
            )
        );
        assert_eq!(
            both(Constant::Duration("P1DT2H30M".into())),
            (
                "duration('P1DT2H30M')".into(),
                "interval('1 days 2 hours 30 minutes')".into()
            )
        );
        assert_eq!(
            constant(Dialect::Neo4j, &Constant::Time("10:00:00+01:00".into())).unwrap(),
            "time('10:00:00+01:00')"
        );
        for (dialect, value, expected) in [
            (
                Dialect::Ladybug,
                Constant::Time("10:00:00".into()),
                "no time type",
            ),
            (
                Dialect::Ladybug,
                Constant::Duration("-P1D".into()),
                "negative duration",
            ),
            (
                Dialect::Ladybug,
                Constant::Duration("P1H".into()),
                "invalid xsd:duration",
            ),
            (Dialect::Neo4j, Constant::Integer(i128::MAX), "64-bit"),
        ] {
            let message = constant(dialect, &value).unwrap_err().to_string();
            assert!(message.contains(expected), "{message}");
        }
    }

    #[test]
    fn plans_routes_for_resolved_paths() {
        assert_eq!(
            routes(&property("name")).unwrap(),
            vec![Route {
                hops: vec![],
                property: Some("name".into())
            }]
        );
        let sequence = LpgPath::Sequence(vec![
            relationship("WORKS_FOR", Direction::Out),
            LpgPath::Alternative(vec![property("name"), property("legalName")]),
        ]);
        assert_eq!(
            routes(&sequence).unwrap(),
            vec![
                Route {
                    hops: vec![hop(&["WORKS_FOR"], Direction::Out, 1, 1)],
                    property: Some("name".into())
                },
                Route {
                    hops: vec![hop(&["WORKS_FOR"], Direction::Out, 1, 1)],
                    property: Some("legalName".into())
                },
            ]
        );
        let repeat = LpgPath::Repeat {
            path: Box::new(LpgPath::Alternative(vec![
                relationship("KNOWS", Direction::Out),
                relationship("LIKES", Direction::Out),
            ])),
            min: 1,
            max: Some(5),
        };
        assert_eq!(
            routes(&repeat).unwrap(),
            vec![Route {
                hops: vec![hop(&["KNOWS", "LIKES"], Direction::Out, 1, 5)],
                property: None
            }]
        );
        let mixed = LpgPath::Alternative(vec![
            relationship("KNOWS", Direction::Out),
            relationship("KNOWS", Direction::In),
        ]);
        assert_eq!(routes(&mixed).unwrap().len(), 2);
        let complex_repeat = LpgPath::Repeat {
            path: Box::new(LpgPath::Sequence(vec![
                relationship("A", Direction::Out),
                relationship("B", Direction::Out),
            ])),
            min: 0,
            max: Some(3),
        };
        assert!(routes(&complex_repeat)
            .unwrap_err()
            .to_string()
            .contains("only supported over a single relationship"));
    }

    #[test]
    fn renders_relationship_chains_per_dialect() {
        let hops = [
            hop(&["A", "B"], Direction::Out, 1, 1),
            hop(&["C"], Direction::In, 0, 5),
        ];
        assert_eq!(
            chain(Dialect::Neo4j, &hops),
            "-[:`A`|`B`]->()<-[:`C`*0..5]-"
        );
        assert_eq!(
            chain(Dialect::Ladybug, &hops),
            "-[:`A`|:`B`]->()<-[:`C`*0..5]-"
        );
    }
}
