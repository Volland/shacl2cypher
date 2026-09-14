/// Names, keys and message a rendered rule reports in its rows.
#[derive(Debug, Clone, Copy)]
pub struct RuleMeta<'a> {
    pub name: &'a str,
    pub shape: &'a str,
    pub path: Option<&'a str>,
    pub constraint: &'a str,
    pub severity: &'a str,
    /// `sh:message` text (with `{$this}` / `{?value}` placeholders) or a generated description.
    pub message: &'a str,
    /// Key property identifying focus nodes (`s2c:key`, else `--node-key`).
    pub focus_key: Option<&'a str>,
    /// Key property identifying node values and relationship endpoints (`--node-key`).
    pub value_key: Option<&'a str>,
    /// Add all focus properties to rows (`--verbose`).
    pub verbose: bool,
}

/// The detail and summary query of one rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rendered {
    pub detail: String,
    pub summary: String,
}
