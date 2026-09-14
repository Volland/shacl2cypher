//! Test-only lint over rendered queries. A comparison directly under `NOT` can be
//! NULL, and `NOT NULL` filters the row out, silently dropping a violation; every
//! comparison there must be wrapped (`coalesce`, `CASE`, a function) or sit next to
//! explicit NULL handling.

/// Operands of `NOT (…)` groups that are bare comparisons, e.g. `v1 >= 0` in `NOT (v1 >= 0)`.
// @lat: [[semantics#Null Safety]]
pub(crate) fn bare_comparisons_under_not(query: &str) -> Vec<String> {
    let chars: Vec<char> = query.chars().collect();
    let mut findings = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '\'' {
            index = skip_string(&chars, index);
            continue;
        }
        if starts_with(&chars, index, "NOT (") {
            let open = index + 4;
            if let Some(close) = matching_paren(&chars, open) {
                let group: String = chars[open + 1..close].iter().collect();
                if !group.contains("IS NULL") && !group.contains("IS NOT NULL") {
                    findings.extend(
                        top_level_operands(&group)
                            .into_iter()
                            .filter(|operand| is_bare_comparison(operand)),
                    );
                }
            }
            index = open + 1;
            continue;
        }
        index += 1;
    }
    findings
}

fn starts_with(chars: &[char], index: usize, needle: &str) -> bool {
    needle
        .chars()
        .enumerate()
        .all(|(offset, c)| chars.get(index + offset) == Some(&c))
}

/// Index just past a single-quoted string starting at `start`.
fn skip_string(chars: &[char], start: usize) -> usize {
    let mut index = start + 1;
    while index < chars.len() {
        match chars[index] {
            '\\' => index += 2,
            '\'' => return index + 1,
            _ => index += 1,
        }
    }
    index
}

fn matching_paren(chars: &[char], open: usize) -> Option<usize> {
    let mut depth = 0;
    let mut index = open;
    while index < chars.len() {
        match chars[index] {
            '\'' => {
                index = skip_string(chars, index);
                continue;
            }
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
        index += 1;
    }
    None
}

/// Splits on top-level ` AND ` / ` OR `, unwrapping fully parenthesized groups.
fn top_level_operands(group: &str) -> Vec<String> {
    let trimmed = group.trim();
    let chars: Vec<char> = trimmed.chars().collect();
    if chars.first() == Some(&'(') && matching_paren(&chars, 0) == Some(chars.len() - 1) {
        return top_level_operands(&trimmed[1..trimmed.len() - 1]);
    }
    let mut operands = Vec::new();
    let mut depth = 0;
    let mut current = String::new();
    let mut index = 0;
    while index < chars.len() {
        let c = chars[index];
        match c {
            '\'' => {
                let end = skip_string(&chars, index);
                current.extend(&chars[index..end]);
                index = end;
                continue;
            }
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth -= 1,
            _ => {}
        }
        if depth == 0 {
            if let Some(separator) = [" AND ", " OR "]
                .iter()
                .find(|separator| starts_with(&chars, index, separator))
            {
                operands.push(std::mem::take(&mut current));
                index += separator.len();
                continue;
            }
        }
        current.push(c);
        index += 1;
    }
    operands.push(current);
    operands
        .into_iter()
        .map(|operand| operand.trim().to_owned())
        .collect()
}

/// A comparison between plain identifiers or literals, without a wrapping call.
fn is_bare_comparison(operand: &str) -> bool {
    let unquoted = {
        let chars: Vec<char> = operand.chars().collect();
        let mut out = String::new();
        let mut index = 0;
        while index < chars.len() {
            if chars[index] == '\'' {
                index = skip_string(&chars, index);
                out.push_str("''");
            } else {
                out.push(chars[index]);
                index += 1;
            }
        }
        out
    };
    if unquoted.contains('(') || unquoted.contains("IS ") {
        return false;
    }
    [" < ", " <= ", " > ", " >= ", " = ", " <> "]
        .iter()
        .any(|operator| unquoted.contains(operator))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_bare_comparisons_under_not() {
        assert_eq!(
            bare_comparisons_under_not("WHERE NOT (v1 >= 0)"),
            vec!["v1 >= 0"]
        );
        assert_eq!(
            bare_comparisons_under_not("WHERE NOT ((v1 >= 1 AND v1 <= 5))"),
            vec!["v1 >= 1", "v1 <= 5"]
        );
        assert_eq!(
            bare_comparisons_under_not("WHERE x AND NOT (v0.`a` = 'x')"),
            vec!["v0.`a` = 'x'"]
        );
    }

    #[test]
    fn accepts_null_safe_forms() {
        for query in [
            "WHERE NOT (coalesce(v1 >= 0, false))",
            "WHERE NOT ((v0.`a` IS NULL OR v0.`b` IS NULL OR v0.`a` < v0.`b`))",
            "WHERE NOT (regexp_matches(v1, '(?i)a >= b'))",
            "WHERE NOT (size([x IN [a, b] WHERE x]) = 1)",
            "WHERE NOT (label(v1) IN ['A'])",
            "WHERE NOT EXISTS { MATCH (v0)-[:R]->(x) WHERE x.a > 1 }",
        ] {
            assert!(bare_comparisons_under_not(query).is_empty(), "{query}");
        }
    }
}
