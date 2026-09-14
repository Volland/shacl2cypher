//! XSD / XPath regular expressions (`sh:pattern` with `sh:flags`), normalized to
//! the syntax shared by Java (Neo4j) and RE2 (LadybugDB) and validated.

use std::fmt::Write as _;

use regex_syntax::hir::{Class, HirKind};

/// Flags kept for rendering; `x` and `q` are applied during normalization.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RegexFlags {
    pub dot_all: bool,
    pub multi_line: bool,
    pub case_insensitive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XsdRegex {
    /// Pattern in Java/RE2-compatible syntax, without flags or anchoring wrappers.
    pub pattern: String,
    pub flags: RegexFlags,
    /// The pattern uses back-references, which RE2 cannot express.
    pub has_backreference: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct RegexError(pub String);

/// XML `NameStartChar` ranges, usable inside a character class.
const NAME_START: &str = r":A-Z_a-z\x{C0}-\x{D6}\x{D8}-\x{F6}\x{F8}-\x{2FF}\x{370}-\x{37D}\x{37F}-\x{1FFF}\x{200C}-\x{200D}\x{2070}-\x{218F}\x{2C00}-\x{2FEF}\x{3001}-\x{D7FF}\x{F900}-\x{FDCF}\x{FDF0}-\x{FFFD}\x{10000}-\x{EFFFF}";
/// Additional XML `NameChar` ranges.
const NAME_EXTRA: &str = r"\-.0-9\x{B7}\x{300}-\x{36F}\x{203F}-\x{2040}";

impl XsdRegex {
    /// Normalizes and validates an `sh:pattern` value with optional `sh:flags`.
    // @lat: [[semantics#Regex Translation]]
    pub fn parse(pattern: &str, flags: Option<&str>) -> Result<Self, RegexError> {
        let mut regex_flags = RegexFlags::default();
        let (mut extended, mut literal) = (false, false);
        for flag in flags.unwrap_or("").chars() {
            match flag {
                's' => regex_flags.dot_all = true,
                'm' => regex_flags.multi_line = true,
                'i' => regex_flags.case_insensitive = true,
                'x' => extended = true,
                'q' => literal = true,
                other => {
                    return Err(RegexError(format!(
                        "unsupported sh:flags character '{other}' (allowed: s, m, i, x, q)"
                    )))
                }
            }
        }
        if literal {
            return Ok(XsdRegex {
                pattern: regex_syntax::escape(pattern),
                flags: regex_flags,
                has_backreference: false,
            });
        }

        let translation = translate(pattern, extended)?;
        regex_syntax::ParserBuilder::new()
            .build()
            .parse(&translation.probe)
            .map_err(|e| invalid(pattern, &describe(&e)))?;
        Ok(XsdRegex {
            pattern: translation.pattern,
            flags: regex_flags,
            has_backreference: translation.has_backreference,
        })
    }
}

#[derive(Default)]
struct Translation {
    pattern: String,
    /// The pattern with back-references neutralized, for validation.
    probe: String,
    has_backreference: bool,
}

impl Translation {
    fn push(&mut self, text: &str) {
        self.pattern.push_str(text);
        self.probe.push_str(text);
    }
}

fn translate(source: &str, extended: bool) -> Result<Translation, RegexError> {
    let chars: Vec<char> = source.chars().collect();
    let mut out = Translation::default();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if extended && c.is_whitespace() {
            i += 1;
            continue;
        }
        match c {
            '\\' => {
                let Some(&next) = chars.get(i + 1) else {
                    return Err(invalid(source, "trailing backslash"));
                };
                match next {
                    '1'..='9' => {
                        let mut end = i + 1;
                        while end < chars.len() && chars[end].is_ascii_digit() {
                            end += 1;
                        }
                        let digits: String = chars[i + 1..end].iter().collect();
                        out.pattern.push('\\');
                        out.pattern.push_str(&digits);
                        out.probe.push_str("(?:)");
                        out.has_backreference = true;
                        i = end;
                        continue;
                    }
                    'i' => out.push(&format!("[{NAME_START}]")),
                    'I' => out.push(&format!("[^{NAME_START}]")),
                    'c' => out.push(&format!("[{NAME_START}{NAME_EXTRA}]")),
                    'C' => out.push(&format!("[^{NAME_START}{NAME_EXTRA}]")),
                    'p' | 'P' if is_block_escape(&chars, i) => {
                        return Err(invalid(
                            source,
                            "Unicode block escapes such as \\p{IsBasicLatin} are not supported",
                        ))
                    }
                    _ => {
                        out.push("\\");
                        out.push(&next.to_string());
                    }
                }
                i += 2;
            }
            '[' => {
                let end = class_end(&chars, i)
                    .ok_or_else(|| invalid(source, "unterminated character class"))?;
                let class = translate_class(&chars[i..=end], source)?;
                out.push(&class);
                i = end + 1;
            }
            _ => {
                out.push(&c.to_string());
                i += 1;
            }
        }
    }
    Ok(out)
}

/// Index of the `]` closing the class opened at `start`, including subtractions.
fn class_end(chars: &[char], start: usize) -> Option<usize> {
    let mut depth = 0;
    let mut i = start;
    while i < chars.len() {
        match chars[i] {
            '\\' => {
                i += 2;
                continue;
            }
            '[' => depth += 1,
            ']' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// Translates one character class (brackets included). Classes using subtraction
/// or negated name escapes are expanded to explicit ranges.
fn translate_class(class: &[char], source: &str) -> Result<String, RegexError> {
    let mut body = String::new();
    let mut needs_expansion = false;
    let mut i = 0;
    while i < class.len() {
        let c = class[i];
        match c {
            '\\' => {
                let Some(&next) = class.get(i + 1) else {
                    return Err(invalid(source, "trailing backslash"));
                };
                match next {
                    'i' => body.push_str(NAME_START),
                    'c' => {
                        body.push_str(NAME_START);
                        body.push_str(NAME_EXTRA);
                    }
                    'I' => {
                        let _ = write!(body, "[^{NAME_START}]");
                        needs_expansion = true;
                    }
                    'C' => {
                        let _ = write!(body, "[^{NAME_START}{NAME_EXTRA}]");
                        needs_expansion = true;
                    }
                    'p' | 'P' if is_block_escape(class, i) => {
                        return Err(invalid(
                            source,
                            "Unicode block escapes such as \\p{IsBasicLatin} are not supported",
                        ))
                    }
                    _ => {
                        body.push('\\');
                        body.push(next);
                    }
                }
                i += 2;
            }
            '-' if i > 1 && class.get(i + 1) == Some(&'[') => {
                body.push_str("--");
                needs_expansion = true;
                i += 1;
            }
            '&' | '~' => {
                body.push('\\');
                body.push(c);
                i += 1;
            }
            _ => {
                body.push(c);
                i += 1;
            }
        }
    }
    if needs_expansion {
        expand_class(&body).map_err(|reason| invalid(source, &reason))
    } else {
        Ok(body)
    }
}

/// Rewrites a class as explicit `\x{…}` ranges, which Java and RE2 both accept.
fn expand_class(class: &str) -> Result<String, String> {
    let hir = regex_syntax::ParserBuilder::new()
        .build()
        .parse(class)
        .map_err(|e| describe(&e))?;
    let ranges: Vec<(u32, u32)> = match hir.kind() {
        HirKind::Class(Class::Unicode(unicode)) => unicode
            .ranges()
            .iter()
            .map(|r| (u32::from(r.start()), u32::from(r.end())))
            .collect(),
        HirKind::Class(Class::Bytes(bytes)) if bytes.ranges().is_empty() => Vec::new(),
        HirKind::Literal(literal) => {
            let text = std::str::from_utf8(&literal.0).map_err(|e| e.to_string())?;
            text.chars().map(|c| (u32::from(c), u32::from(c))).collect()
        }
        _ => return Err(format!("cannot expand character class {class}")),
    };
    if ranges.is_empty() {
        return Ok(r"[^\x{0}-\x{10FFFF}]".into());
    }
    let mut out = String::from("[");
    for (start, end) in ranges {
        if start == end {
            let _ = write!(out, "\\x{{{start:X}}}");
        } else {
            let _ = write!(out, "\\x{{{start:X}}}-\\x{{{end:X}}}");
        }
    }
    out.push(']');
    Ok(out)
}

fn is_block_escape(chars: &[char], backslash: usize) -> bool {
    chars.get(backslash + 2) == Some(&'{')
        && chars.get(backslash + 3) == Some(&'I')
        && chars.get(backslash + 4) == Some(&'s')
}

fn describe(error: &regex_syntax::Error) -> String {
    match error {
        regex_syntax::Error::Parse(e) => e.kind().to_string(),
        regex_syntax::Error::Translate(e) => e.kind().to_string(),
        other => other.to_string(),
    }
}

fn invalid(source: &str, reason: &str) -> RegexError {
    RegexError(format!("invalid sh:pattern \"{source}\": {reason}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(pattern: &str) -> XsdRegex {
        XsdRegex::parse(pattern, None).unwrap()
    }

    fn matcher(regex: &XsdRegex) -> regex::Regex {
        regex::Regex::new(&regex.pattern).unwrap()
    }

    #[test]
    fn passes_common_syntax_through() {
        let regex = parse(r"^\d{5}(-\d{4})?$");
        assert_eq!(regex.pattern, r"^\d{5}(-\d{4})?$");
        assert_eq!(regex.flags, RegexFlags::default());
        assert!(!regex.has_backreference);
    }

    #[test]
    fn parses_flags_and_applies_x_and_q() {
        let flags = XsdRegex::parse("a", Some("ims")).unwrap().flags;
        assert!(flags.case_insensitive && flags.multi_line && flags.dot_all);
        assert_eq!(XsdRegex::parse("a.b", Some("q")).unwrap().pattern, r"a\.b");
        assert_eq!(
            XsdRegex::parse("a b [ c]", Some("x")).unwrap().pattern,
            "ab[ c]"
        );
        let message = XsdRegex::parse("a", Some("g")).unwrap_err().to_string();
        assert!(
            message.contains("unsupported sh:flags character 'g'"),
            "{message}"
        );
    }

    // @lat: [[tests#Compilation#Regex Normalization]]
    #[test]
    fn expands_class_subtraction_to_explicit_ranges() {
        let regex = parse("[a-z-[aeiou]]+");
        assert_eq!(
            regex.pattern,
            r"[\x{62}-\x{64}\x{66}-\x{68}\x{6A}-\x{6E}\x{70}-\x{74}\x{76}-\x{7A}]+"
        );
        let matcher = matcher(&regex);
        assert!(matcher.is_match("bcd"));
        assert!(!matcher.is_match("aeiou"));
    }

    #[test]
    fn expands_xml_name_escapes() {
        let name = matcher(&parse(r"^\i\c*$"));
        assert!(name.is_match("_ns:elem-1.x"));
        assert!(!name.is_match("1abc"));
        let not_name_start = matcher(&parse(r"^[\I]$"));
        assert!(not_name_start.is_match("1"));
        assert!(!not_name_start.is_match("a"));
    }

    #[test]
    fn flags_back_references() {
        let regex = parse(r"(a)\1");
        assert_eq!(regex.pattern, r"(a)\1");
        assert!(regex.has_backreference);
    }

    #[test]
    fn escapes_set_operators_inside_classes() {
        let regex = parse("[a&&b~]");
        assert_eq!(regex.pattern, r"[a\&\&b\~]");
        assert!(matcher(&regex).is_match("&"));
    }

    #[test]
    fn rejects_invalid_patterns_and_block_escapes() {
        for (pattern, expected) in [
            ("(", "invalid sh:pattern \"(\""),
            ("[abc", "unterminated character class"),
            (r"\p{IsBasicLatin}", "Unicode block escapes"),
            ("a\\", "trailing backslash"),
        ] {
            let message = XsdRegex::parse(pattern, None).unwrap_err().to_string();
            assert!(message.contains(expected), "{pattern}: {message}");
        }
        assert!(XsdRegex::parse(r"\p{L}+", None).is_ok());
    }
}
