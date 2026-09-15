//! Cypher rendering of IR rules: pieces shared by the Neo4j, LadybugDB and FalkorDB backends.

use crate::ast::Direction;
use crate::ir::Constant;
use crate::mapping::LpgPath;

pub mod falkordb;
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
    FalkorDb,
}

impl Dialect {
    pub fn name(self) -> &'static str {
        match self {
            Dialect::Neo4j => "neo4j",
            Dialect::Ladybug => "ladybug",
            Dialect::FalkorDb => "falkordb",
        }
    }

    /// Largest variable-length upper bound the database accepts.
    // @lat: [[dialects#Dialect Backends#Renderer Probes]]
    pub fn max_path_depth(self) -> Option<u32> {
        match self {
            Dialect::Neo4j | Dialect::FalkorDb => None,
            Dialect::Ladybug => Some(30),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct RenderError(pub String);

/// Single-quoted string literal; only `\` and `'` need escaping in every dialect.
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

/// Backticked identifiers of a query, skipping string literals.
pub(crate) fn backticked(query: &str) -> Vec<String> {
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
        Constant::Date(lexical) => match dialect {
            Dialect::FalkorDb if !calendar_date(lexical) => {
                return Err(RenderError(format!(
                    "xsd:date constant {lexical} is not supported on FalkorDB, which only reads YYYY-MM-DD dates without a timezone and silently rolls over invalid days"
                )))
            }
            _ => format!("date({})", quote(lexical)),
        },
        Constant::DateTime(lexical) => match (dialect, has_timezone(lexical)) {
            (Dialect::Neo4j, true) => format!("datetime({})", quote(lexical)),
            (Dialect::Neo4j, false) => format!("localdatetime({})", quote(lexical)),
            (Dialect::Ladybug, true) => format!("CAST({} AS TIMESTAMP_TZ)", quote(lexical)),
            (Dialect::Ladybug, false) => format!("timestamp({})", quote(lexical)),
            (Dialect::FalkorDb, true) => return Err(zoned_on_falkordb("xsd:dateTime", lexical)),
            (Dialect::FalkorDb, false) => {
                let whole = lexical
                    .split_once('T')
                    .filter(|(date, _)| calendar_date(date))
                    .and_then(|(date, time)| clock_time(time).map(|time| format!("{date}T{time}")))
                    .ok_or_else(|| RenderError(format!(
                        "xsd:dateTime constant {lexical} is not supported on FalkorDB, which only keeps whole seconds of valid local date-times"
                    )))?;
                format!("localdatetime({})", quote(&whole))
            }
        },
        Constant::Time(lexical) => match dialect {
            Dialect::Neo4j if has_timezone(lexical) => format!("time({})", quote(lexical)),
            Dialect::Neo4j => format!("localtime({})", quote(lexical)),
            Dialect::Ladybug => {
                return Err(RenderError(format!(
                "xsd:time constant {lexical} is not supported on LadybugDB, which has no time type"
            )))
            }
            Dialect::FalkorDb if has_timezone(lexical) => {
                return Err(zoned_on_falkordb("xsd:time", lexical))
            }
            Dialect::FalkorDb => {
                let whole = clock_time(lexical).ok_or_else(|| RenderError(format!(
                    "xsd:time constant {lexical} is not supported on FalkorDB, which only keeps whole seconds of valid times"
                )))?;
                format!("localtime({})", quote(whole))
            }
        },
        Constant::Duration(lexical) => match dialect {
            Dialect::Neo4j => format!("duration({})", quote(lexical)),
            Dialect::Ladybug => format!(
                "interval({})",
                quote(&duration_words(lexical).map_err(RenderError)?)
            ),
            Dialect::FalkorDb => {
                return Err(RenderError(format!(
                    "xsd:duration constant {lexical} is not supported on FalkorDB, which compares durations as total seconds (P1Y equals P365D) and cannot hold negative or fractional ones"
                )))
            }
        },
    })
}

// @lat: [[dialects#Dialect Backends#FalkorDB]]
fn zoned_on_falkordb(datatype: &str, lexical: &str) -> RenderError {
    RenderError(format!(
        "{datatype} constant {lexical} with a timezone is not supported on FalkorDB, which silently drops timezone offsets"
    ))
}

/// `YYYY-MM-DD` naming a real calendar day in years 0001 to 9999.
fn calendar_date(text: &str) -> bool {
    let mut parts = text.split('-');
    let (Some(year), Some(month), Some(day), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return false;
    };
    let (Some(year), Some(month), Some(day)) = (digits(year, 4), digits(month, 2), digits(day, 2))
    else {
        return false;
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    year >= 1 && (1..=days).contains(&day)
}

/// The whole-second `hh:mm:ss` form of a valid time whose fraction, if any, is zero.
fn clock_time(text: &str) -> Option<&str> {
    let (whole, fraction) = match text.split_once('.') {
        Some((whole, fraction)) if !fraction.is_empty() && fraction.bytes().all(|b| b == b'0') => {
            (whole, fraction)
        }
        Some(_) => return None,
        None => (text, ""),
    };
    let _ = fraction;
    let mut parts = whole.split(':');
    let (Some(hour), Some(minute), Some(second), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return None;
    };
    let valid = digits(hour, 2).is_some_and(|h| h <= 23)
        && digits(minute, 2).is_some_and(|m| m <= 59)
        && digits(second, 2).is_some_and(|s| s <= 59);
    valid.then_some(whole)
}

fn digits(text: &str, len: usize) -> Option<u32> {
    if text.len() == len && text.bytes().all(|b| b.is_ascii_digit()) {
        text.parse().ok()
    } else {
        None
    }
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
        Dialect::Neo4j | Dialect::FalkorDb => "|",
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
        assert_eq!(
            backticked("MATCH (n:`A``b`) WHERE n.x = 'it''s `no`' RETURN n.`c` AS `d`"),
            ["A`b", "c", "d"]
        );
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

    // @lat: [[tests#Compilation#FalkorDB Constants]]
    #[test]
    fn renders_only_exact_temporal_constants_on_falkordb() {
        let falkordb = |value: Constant| constant(Dialect::FalkorDb, &value);
        for (value, expected) in [
            (Constant::Date("2020-02-29".into()), "date('2020-02-29')"),
            (
                Constant::DateTime("2020-01-01T10:00:00".into()),
                "localdatetime('2020-01-01T10:00:00')",
            ),
            (
                Constant::DateTime("2020-01-01T10:00:00.000".into()),
                "localdatetime('2020-01-01T10:00:00')",
            ),
            (Constant::Time("23:59:59".into()), "localtime('23:59:59')"),
            (Constant::String("O'Brien".into()), "'O\\'Brien'"),
            (Constant::Integer(-5), "-5"),
        ] {
            assert_eq!(falkordb(value).unwrap(), expected);
        }
        for (value, expected) in [
            (
                Constant::Date("2021-02-29".into()),
                "rolls over invalid days",
            ),
            (Constant::Date("2020-01-01Z".into()), "YYYY-MM-DD"),
            (Constant::Date("-0001-01-01".into()), "YYYY-MM-DD"),
            (
                Constant::DateTime("2020-01-01T10:00:00Z".into()),
                "drops timezone offsets",
            ),
            (
                Constant::DateTime("2020-01-01T10:00:00+01:00".into()),
                "drops timezone offsets",
            ),
            (
                Constant::DateTime("2020-01-01T10:00:00.5".into()),
                "whole seconds",
            ),
            (
                Constant::DateTime("2020-01-01T24:00:00".into()),
                "whole seconds",
            ),
            (Constant::Time("10:00:00Z".into()), "drops timezone offsets"),
            (Constant::Time("10:00:00.25".into()), "whole seconds"),
            (Constant::Time("10:61:00".into()), "whole seconds"),
            (Constant::Duration("P1D".into()), "total seconds"),
        ] {
            let message = falkordb(value).unwrap_err().to_string();
            assert!(message.contains("FalkorDB"), "{message}");
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
        assert_eq!(
            chain(Dialect::FalkorDb, &hops),
            "-[:`A`|`B`]->()<-[:`C`*0..5]-"
        );
        assert_eq!(Dialect::FalkorDb.name(), "falkordb");
        assert_eq!(Dialect::FalkorDb.max_path_depth(), None);
    }
}
