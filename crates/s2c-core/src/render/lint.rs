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

/// Constructs FalkorDB evaluates wrongly or that can raise at runtime: pattern
/// predicates and comprehensions outside `MATCH`, `EXISTS`/`COUNT` subqueries, and
/// string functions applied to anything but `toStringOrNull(…)`.
// @lat: [[dialects#Dialect Backends#FalkorDB]]
pub(crate) fn falkordb_hazards(query: &str) -> Vec<String> {
    let chars: Vec<char> = query.chars().collect();
    let mut findings = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '\'' {
            index = skip_string(&chars, index);
            continue;
        }
        for needle in ["EXISTS {", "COUNT {", " =~ ", " IS :: ", "elementId(", "[("] {
            if starts_with(&chars, index, needle) {
                findings.push(format!("`{needle}` at {index}"));
            }
        }
        if (starts_with(&chars, index, ")-[") || starts_with(&chars, index, ")<-["))
            && !in_match_clause(&chars, index)
        {
            findings.push(format!("pattern outside MATCH at {index}"));
        }
        for (call, allowed) in [
            (
                "size(",
                &[
                    "toStringOrNull(",
                    "reduce(",
                    "[",
                    "(",
                    "string.matchRegEx(",
                    "c",
                ][..],
            ),
            ("string.matchRegEx(", &["toStringOrNull("][..]),
            ("toString(", &["id("][..]),
        ] {
            let preceded_by_word =
                index > 0 && (chars[index - 1].is_alphanumeric() || chars[index - 1] == '.');
            if !preceded_by_word && starts_with(&chars, index, call) {
                let argument = index + call.chars().count();
                if !allowed
                    .iter()
                    .any(|prefix| starts_with(&chars, argument, prefix))
                {
                    findings.push(format!("`{call}` over a raw value at {index}"));
                }
            }
        }
        index += 1;
    }
    findings
}

/// Whether the nearest clause keyword before `index` is `MATCH`.
fn in_match_clause(chars: &[char], index: usize) -> bool {
    let before: String = chars[..index].iter().collect();
    let keyword = ["MATCH ", "WHERE ", "WITH ", "RETURN ", "UNWIND ", "CALL {"]
        .iter()
        .filter_map(|keyword| before.rfind(keyword).map(|at| (at, *keyword)))
        .max_by_key(|(at, _)| *at);
    matches!(keyword, Some((_, "MATCH ")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_falkordb_hazards() {
        for query in [
            "MATCH (v0) WHERE (v0)-[:R]->() RETURN v0",
            "MATCH (v0) RETURN [(v0)-[:R]->(x) | x]",
            "MATCH (v0) RETURN size(v0.`name`)",
            "MATCH (v0) RETURN string.matchRegEx(v1, 'a')",
            "MATCH (v0) RETURN toString(v1)",
            "MATCH (v0) WHERE EXISTS { MATCH (v0)-[:R]->() } RETURN v0",
        ] {
            assert!(!falkordb_hazards(query).is_empty(), "{query}");
        }
        for query in [
            "MATCH (v0)-[x1:R]->(v1) WHERE id(x1) >= 0 RETURN v1",
            "CALL { WITH v0 OPTIONAL MATCH x2 = (v0)-[:R*1..3]->(x1) RETURN count(DISTINCT x1) > 0 AS c3 }",
            "RETURN size(toStringOrNull(v1)), size(reduce(a = [], x IN [] | a)), size(string.matchRegEx(toStringOrNull(v1), '[(a)-[b')), size((c1 + c2)), toString(id(v0))",
        ] {
            assert!(falkordb_hazards(query).is_empty(), "{query}: {:?}", falkordb_hazards(query));
        }
    }

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
