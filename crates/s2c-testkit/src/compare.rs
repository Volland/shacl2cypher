use std::collections::BTreeSet;
use std::fmt;

use crate::fixture::Expected;

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

/// Compares violations as sets of `(rule, focus)`; duplicates and order are ignored.
pub fn compare(expected: &[Expected], actual: &[Expected]) -> Diff {
    let expected: BTreeSet<&Expected> = expected.iter().collect();
    let actual: BTreeSet<&Expected> = actual.iter().collect();
    Diff {
        missing: expected.difference(&actual).map(|v| (*v).clone()).collect(),
        unexpected: actual.difference(&expected).map(|v| (*v).clone()).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(rule: &str, focus: &str) -> Expected {
        Expected {
            rule: rule.into(),
            focus: focus.into(),
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
}
