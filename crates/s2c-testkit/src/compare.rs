use std::collections::BTreeSet;
use std::fmt;

use crate::fixture::{Expected, KnownDifference};

/// Difference between the hand-written `expect` block and an engine's violations.
#[derive(Debug, Default, PartialEq)]
pub struct Diff {
    pub missing: Vec<Expected>,
    pub unexpected: Vec<Expected>,
}

impl Diff {
    pub fn is_empty(&self) -> bool {
        self.missing.is_empty() && self.unexpected.is_empty()
    }
}

impl fmt::Display for Diff {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for v in &self.missing {
            writeln!(f, "  missing:    {} @ {}", v.rule, v.focus)?;
        }
        for v in &self.unexpected {
            writeln!(f, "  unexpected: {} @ {}", v.rule, v.focus)?;
        }
        Ok(())
    }
}

fn key(v: &Expected) -> Expected {
    Expected {
        rule: v.rule.clone(),
        focus: v.focus.clone(),
        details: None,
    }
}

/// Compares violations as sets of `(rule, focus)`; duplicates, order and details are ignored.
pub fn compare(expected: &[Expected], actual: &[Expected]) -> Diff {
    let expected: BTreeSet<Expected> = expected.iter().map(key).collect();
    let actual: BTreeSet<Expected> = actual.iter().map(key).collect();
    Diff {
        missing: expected.difference(&actual).cloned().collect(),
        unexpected: actual.difference(&expected).cloned().collect(),
    }
}

/// Drops differences the fixture documents for `engine`.
pub fn without_known(diff: Diff, known: &[KnownDifference], engine: &str) -> Diff {
    let documented = |v: &Expected| {
        known
            .iter()
            .any(|k| k.engine == engine && k.rule == v.rule && k.focus == v.focus)
    };
    Diff {
        missing: diff
            .missing
            .into_iter()
            .filter(|v| !documented(v))
            .collect(),
        unexpected: diff
            .unexpected
            .into_iter()
            .filter(|v| !documented(v))
            .collect(),
    }
}

/// Expectations with `details` whose actual violations report other details.
pub fn detail_mismatches(expected: &[Expected], actual: &[Expected]) -> Vec<String> {
    let mut mismatches = Vec::new();
    for want in expected {
        let Some(details) = &want.details else {
            continue;
        };
        let want_set: BTreeSet<&String> = details.iter().collect();
        for got in actual
            .iter()
            .filter(|v| v.rule == want.rule && v.focus == want.focus)
        {
            let got_set: BTreeSet<&String> = got.details.iter().flatten().collect();
            if got_set != want_set {
                mismatches.push(format!(
                    "{} @ {}: expected details {want_set:?}, got {got_set:?}",
                    want.rule, want.focus
                ));
            }
        }
    }
    mismatches
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(rule: &str, focus: &str) -> Expected {
        Expected {
            rule: rule.into(),
            focus: focus.into(),
            details: None,
        }
    }

    #[test]
    fn reports_missing_and_unexpected() {
        let diff = compare(
            &[v("r1", "a"), v("r2", "b")],
            &[v("r2", "b"), v("r3", "c"), v("r2", "b")],
        );
        assert_eq!(diff.missing, vec![v("r1", "a")]);
        assert_eq!(diff.unexpected, vec![v("r3", "c")]);
        assert!(compare(&[v("r1", "a")], &[v("r1", "a")]).is_empty());
    }

    #[test]
    fn filters_known_differences_and_checks_details() {
        let known = [KnownDifference {
            engine: "pyshacl".into(),
            rule: "r1".into(),
            focus: "a".into(),
            reason: "test".into(),
        }];
        let diff = compare(&[v("r1", "a")], &[]);
        assert!(without_known(diff, &known, "pyshacl").is_empty());
        assert!(!without_known(compare(&[v("r1", "a")], &[]), &known, "neo4j").is_empty());

        let with = |details: &[&str]| Expected {
            details: Some(details.iter().map(|d| d.to_string()).collect()),
            ..v("r1", "a")
        };
        assert!(detail_mismatches(&[with(&["x", "y"])], &[with(&["y", "x"])]).is_empty());
        assert_eq!(detail_mismatches(&[with(&["x"])], &[with(&[])]).len(), 1);
        assert!(detail_mismatches(&[v("r1", "a")], &[with(&["z"])]).is_empty());
    }
}
