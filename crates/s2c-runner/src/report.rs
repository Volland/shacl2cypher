//! Report renderers: terminal table, JSON, JUnit XML and SARIF 2.1.0.

use serde_json::{json, Value};

use crate::executor::Row;
use crate::validate::{Report, RuleResult, Status};

/// Detail rows printed per failing rule in table and JUnit output.
const LISTED_VIOLATIONS: usize = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Table,
    Json,
    Junit,
    Sarif,
}

// @lat: [[architecture#Runner#Reports]]
pub fn render(report: &Report, format: Format) -> String {
    match format {
        Format::Table => table(report),
        Format::Json => {
            let mut json = serde_json::to_string_pretty(report).expect("reports serialize");
            json.push('\n');
            json
        }
        Format::Junit => junit(report),
        Format::Sarif => {
            let mut json = serde_json::to_string_pretty(&sarif(report)).expect("SARIF serializes");
            json.push('\n');
            json
        }
    }
}

fn status_name(status: Status) -> &'static str {
    match status {
        Status::Passed => "passed",
        Status::Failed => "failed",
        Status::Timeout => "timeout",
        Status::Error => "error",
        Status::Skipped => "skipped",
        Status::GuaranteedBySchema => "guaranteed",
    }
}

/// `Person id=p2`, `KNOWS p1->p2`, or the element id.
pub fn describe_focus(focus: &Value) -> String {
    let text = |key: &str| match focus.get(key) {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => Some(s.clone()),
        Some(other) => Some(other.to_string()),
    };
    if let Some(label) = text("label") {
        return match (text("key"), text("keyValue")) {
            (Some(key), Some(value)) => format!("{label} {key}={value}"),
            (None, Some(value)) => format!("{label} {value}"),
            _ => format!("{label} {}", text("elementId").unwrap_or_default()),
        };
    }
    if let Some(rel_type) = text("type") {
        return format!(
            "{rel_type} {}->{}",
            text("startKey").unwrap_or_default(),
            text("endKey").unwrap_or_default()
        );
    }
    focus.to_string()
}

fn describe_row(row: &Row) -> String {
    let focus = row.get("focus").map(describe_focus).unwrap_or_default();
    let message = row
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or_default();
    match row.get("value") {
        None | Some(Value::Null) => format!("{focus}: {message}"),
        Some(value) => format!("{focus}: {message} (value: {value})"),
    }
}

fn table(report: &Report) -> String {
    let mut out = format!(
        "{:<10}  {:<9}  {:>10}  {:>8}  RULE\n",
        "STATUS", "SEVERITY", "VIOLATIONS", "TIME(ms)"
    );
    for rule in &report.rules {
        let count = rule
            .violation_count
            .map_or_else(|| "-".to_owned(), |c| c.to_string());
        out.push_str(&format!(
            "{:<10}  {:<9}  {count:>10}  {:>8}  {}\n",
            status_name(rule.status),
            rule.severity,
            rule.duration_ms,
            rule.name
        ));
        for row in rule.violations.iter().take(LISTED_VIOLATIONS) {
            out.push_str(&format!("            - {}\n", describe_row(row)));
        }
        let listed = rule.violations.len().min(LISTED_VIOLATIONS) as u64;
        if let Some(count) = rule.violation_count.filter(|&c| c > listed && listed > 0) {
            out.push_str(&format!("            … {} more\n", count - listed));
        }
        if let Some(reason) = &rule.status_reason {
            out.push_str(&format!("            ({reason})\n"));
        }
    }
    let s = &report.summary;
    out.push_str(&format!(
        "\n{} rules: {} passed, {} failed, {} timed out, {} errors, {} skipped, {} guaranteed by schema; {} violations in {} ms\n",
        s.rules, s.passed, s.failed, s.timeouts, s.errors, s.skipped, s.guaranteed_by_schema, s.violations, s.duration_ms
    ));
    out
}

fn xml(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c if (c as u32) < 0x20 && !matches!(c, '\n' | '\r' | '\t') => {}
            c => out.push(c),
        }
    }
    out
}

fn seconds(ms: u64) -> String {
    format!("{:.3}", ms as f64 / 1000.0)
}

/// One test case per rule; violations are failures, timeouts and query errors are errors.
fn junit(report: &Report) -> String {
    let s = &report.summary;
    let errors = s.timeouts + s.errors;
    let attrs = format!(
        "tests=\"{}\" failures=\"{}\" errors=\"{errors}\" skipped=\"{}\" time=\"{}\"",
        s.rules,
        s.failed,
        s.skipped,
        seconds(s.duration_ms)
    );
    let mut out = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<testsuites name=\"shacl2cypher\" {attrs}>\n  <testsuite name=\"shacl2cypher ({})\" {attrs}>\n",
        xml(&report.dialect)
    );
    for rule in &report.rules {
        out.push_str(&format!(
            "    <testcase name=\"{}\" classname=\"{}\" time=\"{}\" file=\"{}\" line=\"{}\">\n",
            xml(&rule.name),
            xml(&rule.shape),
            seconds(rule.duration_ms),
            xml(&rule.source.file),
            rule.source.line
        ));
        match rule.status {
            Status::Failed => {
                let mut body: Vec<String> = rule
                    .violations
                    .iter()
                    .take(LISTED_VIOLATIONS)
                    .map(describe_row)
                    .collect();
                if let Some(reason) = &rule.status_reason {
                    body.push(reason.clone());
                }
                out.push_str(&format!(
                    "      <failure type=\"{}\" message=\"{} violations of {}\">{}</failure>\n",
                    xml(&rule.severity),
                    rule.violation_count.unwrap_or(0),
                    xml(&rule.constraint),
                    xml(&body.join("\n"))
                ));
            }
            Status::Timeout => {
                out.push_str("      <error type=\"timeout\" message=\"query timed out\"/>\n")
            }
            Status::Error => out.push_str(&format!(
                "      <error type=\"error\" message=\"{}\"/>\n",
                xml(rule.status_reason.as_deref().unwrap_or("query failed"))
            )),
            Status::Skipped => out.push_str(&format!(
                "      <skipped message=\"{}\"/>\n",
                xml(rule.status_reason.as_deref().unwrap_or("skipped"))
            )),
            Status::Passed | Status::GuaranteedBySchema => {}
        }
        out.push_str("    </testcase>\n");
    }
    out.push_str("  </testsuite>\n</testsuites>\n");
    out
}

fn level(severity: &str) -> &'static str {
    match severity {
        "Warning" => "warning",
        "Info" => "note",
        _ => "error",
    }
}

/// One SARIF result per listed violation, located at the rule's shapes file and line.
fn sarif(report: &Report) -> Value {
    let descriptors: Vec<Value> = report
        .rules
        .iter()
        .map(|rule| {
            json!({
                "id": rule.name,
                "shortDescription": {"text": rule.message},
                "defaultConfiguration": {"level": level(&rule.severity)},
                "properties": {
                    "ruleId": rule.rule_id,
                    "shape": rule.shape,
                    "path": rule.path,
                    "constraint": rule.constraint,
                },
            })
        })
        .collect();
    let mut results = Vec::new();
    let mut notifications = Vec::new();
    for (index, rule) in report.rules.iter().enumerate() {
        let location = |rule: &RuleResult| {
            json!([{
                "physicalLocation": {
                    "artifactLocation": {"uri": rule.source.file.replace('\\', "/")},
                    "region": {"startLine": rule.source.line},
                }
            }])
        };
        match rule.status {
            Status::Failed if rule.violations.is_empty() => results.push(json!({
                "ruleId": rule.name,
                "ruleIndex": index,
                "level": level(&rule.severity),
                "message": {"text": format!("{} violations of {}", rule.violation_count.unwrap_or(0), rule.name)},
                "locations": location(rule),
                "properties": {"violationCount": rule.violation_count},
            })),
            Status::Failed => {
                for row in &rule.violations {
                    let message = row
                        .get("message")
                        .and_then(Value::as_str)
                        .unwrap_or(&rule.message);
                    results.push(json!({
                        "ruleId": rule.name,
                        "ruleIndex": index,
                        "level": level(&rule.severity),
                        "message": {"text": message},
                        "locations": location(rule),
                        "properties": {
                            "focus": row.get("focus"),
                            "value": row.get("value"),
                            "details": row.get("details"),
                            "violationCount": rule.violation_count,
                        },
                    }));
                }
            }
            Status::Timeout | Status::Error => notifications.push(json!({
                "level": "error",
                "message": {"text": format!(
                    "{}: {}",
                    rule.name,
                    rule.status_reason.as_deref().unwrap_or("query timed out")
                )},
                "associatedRule": {"id": rule.name, "index": index},
            })),
            _ => {}
        }
    }
    json!({
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": {"driver": {
                "name": report.tool,
                "version": report.tool_version,
                "informationUri": "https://w3id.org/shacl2cypher",
                "rules": descriptors,
            }},
            "invocations": [{
                "executionSuccessful": report.complete,
                "toolExecutionNotifications": notifications,
            }],
            "results": results,
        }]
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validate::tests::{executor, manifest, rule};
    use crate::validate::{validate, ValidateOptions};

    fn sample_report() -> Report {
        let mut rules: Vec<_> = (0..10)
            .map(|i| rule(&format!("r{i}"), "Violation", "compiled"))
            .collect();
        rules.push(rule("skip", "Warning", "deactivated"));
        rules.push(rule("slow", "Info", "compiled"));
        let counts: Vec<(String, Option<u64>)> = (0..10)
            .map(|i| (format!("r{i}"), Some(if i < 3 { 2 } else { 0 })))
            .chain([("slow".to_owned(), None)])
            .collect();
        let counts: Vec<(&str, Option<u64>)> =
            counts.iter().map(|(n, c)| (n.as_str(), *c)).collect();
        validate(
            &manifest(rules),
            &mut executor(&counts),
            &ValidateOptions::default(),
        )
        .unwrap()
    }

    #[test]
    fn junit_has_one_test_case_per_rule() {
        let xml = render(&sample_report(), Format::Junit);
        assert_eq!(xml.matches("<testcase ").count(), 12);
        assert_eq!(xml.matches("<failure ").count(), 3);
        assert_eq!(xml.matches("<error ").count(), 1);
        assert_eq!(xml.matches("<skipped ").count(), 1);
        assert!(xml.contains("tests=\"12\" failures=\"3\" errors=\"1\" skipped=\"1\""));
        assert!(xml.contains("<failure type=\"Violation\" message=\"2 violations of sh:minCount\">Person id=p0: r0 failed on p0\nPerson id=p1: r0 failed on p1</failure>"));
    }

    #[test]
    fn sarif_results_point_at_the_shapes_source() {
        let sarif: Value = serde_json::from_str(&render(&sample_report(), Format::Sarif)).unwrap();
        let run = &sarif["runs"][0];
        assert_eq!(run["tool"]["driver"]["rules"].as_array().unwrap().len(), 12);
        let results = run["results"].as_array().unwrap();
        assert_eq!(results.len(), 6);
        for result in results {
            let location = &result["locations"][0]["physicalLocation"];
            assert_eq!(location["artifactLocation"]["uri"], "shapes/person.ttl");
            assert_eq!(location["region"]["startLine"], 7);
            assert_eq!(result["level"], "error");
        }
        assert_eq!(results[0]["properties"]["focus"]["keyValue"], "p0");
        assert_eq!(run["invocations"][0]["executionSuccessful"], false);
        assert_eq!(
            run["invocations"][0]["toolExecutionNotifications"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn table_lists_rules_violations_and_totals() {
        let table = render(&sample_report(), Format::Table);
        assert!(table.starts_with("STATUS      SEVERITY   VIOLATIONS  TIME(ms)  RULE\n"));
        assert!(table.contains("- Person id=p1: r0 failed on p1\n"));
        assert!(table.contains("timeout     Info"));
        assert!(table.contains(
            "12 rules: 7 passed, 3 failed, 1 timed out, 0 errors, 1 skipped, 0 guaranteed by schema; 6 violations"
        ));
    }

    #[test]
    fn json_reports_durations_per_rule() {
        let json: Value = serde_json::from_str(&render(&sample_report(), Format::Json)).unwrap();
        for rule in json["rules"].as_array().unwrap() {
            assert!(rule["durationMs"].is_u64());
        }
        assert_eq!(json["summary"]["failed"], 3);
    }

    #[test]
    fn describes_focus_objects() {
        assert_eq!(
            describe_focus(
                &json!({"type": "KNOWS", "startKey": "p1", "endKey": "p2", "elementId": "5:x"})
            ),
            "KNOWS p1->p2"
        );
        assert_eq!(
            describe_focus(
                &json!({"label": "Person", "key": null, "keyValue": "4:x:1", "elementId": "4:x:1"})
            ),
            "Person 4:x:1"
        );
        assert_eq!(
            xml("a<\"b\"&'c'>\u{1}"),
            "a&lt;&quot;b&quot;&amp;&apos;c&apos;&gt;"
        );
    }
}
