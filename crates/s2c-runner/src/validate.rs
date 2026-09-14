//! The `validate` flow: summary queries for every rule, detail queries for rules
//! with violations, timings and the exit code.

use std::time::{Duration, Instant};

use s2c_core::compile::{Manifest, ManifestRule, SourceRef};
use serde::Serialize;

use crate::executor::{ExecError, Executor, Params, Row};

/// Severity threshold for a failing exit code, ordered from least to most severe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FailOn {
    Info,
    Warning,
    Violation,
}

impl FailOn {
    /// Rank of a manifest severity; custom severities count as violations.
    pub fn of_severity(severity: &str) -> FailOn {
        match severity {
            "Info" => FailOn::Info,
            "Warning" => FailOn::Warning,
            _ => FailOn::Violation,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidateOptions {
    /// Maximum detail rows per rule; `None` lists every violation.
    pub limit: Option<u64>,
    pub sample_size: u32,
    /// Per-query timeout.
    pub timeout: Option<Duration>,
}

impl Default for ValidateOptions {
    fn default() -> Self {
        ValidateOptions {
            limit: Some(100),
            sample_size: 5,
            timeout: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    Passed,
    Failed,
    Timeout,
    Error,
    Skipped,
    GuaranteedBySchema,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuleResult {
    pub name: String,
    pub rule_id: String,
    pub shape: String,
    pub path: Option<String>,
    pub constraint: String,
    pub severity: String,
    pub status: Status,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<String>,
    /// Null when the summary query did not complete or the rule has no queries.
    pub violation_count: Option<u64>,
    pub duration_ms: u64,
    pub source: SourceRef,
    pub message: String,
    /// Detail rows, at most `limit`.
    pub violations: Vec<Row>,
}

impl RuleResult {
    pub fn has_violations(&self) -> bool {
        self.violation_count.unwrap_or(0) > 0
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub rules: usize,
    pub passed: usize,
    pub failed: usize,
    pub timeouts: usize,
    pub errors: usize,
    pub skipped: usize,
    pub guaranteed_by_schema: usize,
    pub violations: u64,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub tool: String,
    pub tool_version: String,
    pub dialect: String,
    /// No rule found a violation.
    pub conforms: bool,
    /// Every query completed, so `conforms` is trustworthy.
    pub complete: bool,
    pub summary: Summary,
    pub rules: Vec<RuleResult>,
}

/// Runs a manifest against a database.
// @lat: [[architecture#Runner#Validate Flow]]
pub fn validate(
    manifest: &Manifest,
    executor: &mut dyn Executor,
    options: &ValidateOptions,
) -> Result<Report, String> {
    let backend = executor.dialect().name();
    if manifest.dialect != backend {
        return Err(format!(
            "the manifest was compiled for {} but the database is {backend}",
            manifest.dialect
        ));
    }
    let params = Params {
        limit: options
            .limit
            .map_or(i64::MAX, |limit| i64::try_from(limit).unwrap_or(i64::MAX)),
        sample_size: i64::from(options.sample_size),
    };
    let rules: Vec<RuleResult> = manifest
        .rules
        .iter()
        .map(|rule| run_rule(rule, executor, params, options.timeout))
        .collect();

    let mut summary = Summary {
        rules: rules.len(),
        ..Summary::default()
    };
    for rule in &rules {
        match rule.status {
            Status::Passed => summary.passed += 1,
            Status::Failed => summary.failed += 1,
            Status::Timeout => summary.timeouts += 1,
            Status::Error => summary.errors += 1,
            Status::Skipped => summary.skipped += 1,
            Status::GuaranteedBySchema => summary.guaranteed_by_schema += 1,
        }
        summary.violations += rule.violation_count.unwrap_or(0);
        summary.duration_ms += rule.duration_ms;
    }
    Ok(Report {
        tool: "shacl2cypher".into(),
        tool_version: env!("CARGO_PKG_VERSION").into(),
        dialect: manifest.dialect.clone(),
        conforms: summary.failed == 0,
        complete: summary.timeouts == 0 && summary.errors == 0,
        summary,
        rules,
    })
}

fn run_rule(
    rule: &ManifestRule,
    executor: &mut dyn Executor,
    params: Params,
    timeout: Option<Duration>,
) -> RuleResult {
    let mut result = RuleResult {
        name: rule.name.clone(),
        rule_id: rule.rule_id.clone(),
        shape: rule.shape.clone(),
        path: rule.path.clone(),
        constraint: rule.constraint.clone(),
        severity: rule.severity.clone(),
        status: Status::Passed,
        status_reason: None,
        violation_count: None,
        duration_ms: 0,
        source: rule.source.clone(),
        message: rule.message.clone(),
        violations: Vec::new(),
    };
    let Some(queries) = &rule.queries else {
        (result.status, result.status_reason) = match rule.status.as_str() {
            "guaranteed-by-schema" => (Status::GuaranteedBySchema, None),
            "schema-mismatch" => (
                Status::Skipped,
                Some("schema mismatch; see the manifest's static diagnostics".into()),
            ),
            other => (
                Status::Skipped,
                Some(rule.status_reason.clone().unwrap_or_else(|| other.into())),
            ),
        };
        return result;
    };

    let started = Instant::now();
    let summary = executor.run(&queries.summary, params, timeout);
    let count = summary.and_then(|rows| violation_count(&rows));
    match count {
        Err(error) => fail(&mut result, error),
        Ok(0) => result.violation_count = Some(0),
        Ok(count) => {
            result.violation_count = Some(count);
            result.status = Status::Failed;
            match executor.run(&queries.detail, params, timeout) {
                Ok(rows) => result.violations = rows,
                Err(ExecError::Timeout) => {
                    result.status_reason =
                        Some("detail query timed out; violations are not listed".into())
                }
                Err(error) => {
                    result.status_reason = Some(format!(
                        "detail query failed; violations are not listed: {error}"
                    ))
                }
            }
        }
    }
    result.duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    result
}

fn fail(result: &mut RuleResult, error: ExecError) {
    match error {
        ExecError::Timeout => result.status = Status::Timeout,
        other => {
            result.status = Status::Error;
            result.status_reason = Some(other.to_string());
        }
    }
}

fn violation_count(rows: &[Row]) -> Result<u64, ExecError> {
    rows.first()
        .and_then(|row| row.get("violationCount"))
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| ExecError::Query("summary query returned no violationCount".into()))
}

/// 1 when a rule at or above `fail_on` has violations, else 3 when a query timed
/// out or failed, else 0.
// @lat: [[architecture#Runner#Exit Codes]]
pub fn exit_code(report: &Report, fail_on: FailOn) -> u8 {
    let failing = report
        .rules
        .iter()
        .any(|rule| rule.has_violations() && FailOn::of_severity(&rule.severity) >= fail_on);
    if failing {
        1
    } else if !report.complete {
        3
    } else {
        0
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::HashMap;

    use s2c_core::compile::Queries;
    use s2c_core::render::Dialect;
    use s2c_core::schema::SchemaSnapshot;
    use serde_json::json;

    use super::*;

    /// Answers queries from a table keyed by query text.
    pub(crate) struct FakeExecutor {
        pub answers: HashMap<String, Result<Vec<Row>, ExecError>>,
        pub calls: Vec<(String, Params)>,
    }

    impl Executor for FakeExecutor {
        fn dialect(&self) -> Dialect {
            Dialect::Neo4j
        }

        fn run(
            &mut self,
            query: &str,
            params: Params,
            _timeout: Option<Duration>,
        ) -> Result<Vec<Row>, ExecError> {
            self.calls.push((query.to_owned(), params));
            self.answers[query].clone()
        }

        fn schema(&mut self) -> Result<SchemaSnapshot, ExecError> {
            Ok(SchemaSnapshot::default())
        }
    }

    fn row(value: serde_json::Value) -> Row {
        value.as_object().unwrap().clone()
    }

    pub(crate) fn rule(name: &str, severity: &str, status: &str) -> ManifestRule {
        ManifestRule {
            name: name.into(),
            rule_id: format!("ex:{name}"),
            fingerprint: "0".into(),
            shape: "ex:PersonShape".into(),
            path: Some("ex:name".into()),
            constraint: "sh:minCount".into(),
            severity: severity.into(),
            status: status.into(),
            status_reason: None,
            cost_class: "scan".into(),
            path_depth_cap: None,
            source: SourceRef {
                file: "shapes/person.ttl".into(),
                line: 7,
            },
            message: format!("{name} failed"),
            queries: (status == "compiled").then(|| Queries {
                detail: format!("{name} detail"),
                summary: format!("{name} summary"),
            }),
        }
    }

    pub(crate) fn manifest(rules: Vec<ManifestRule>) -> Manifest {
        let json = json!({
            "schemaVersion": 1, "compilerVersion": "0.1.0", "dialect": "neo4j", "inputs": [],
            "schemaSnapshotHash": null,
            "options": {"nodeKey": "id", "neo4jLabels": "explicit", "strict": false, "lenient": false,
                        "verbose": false, "maxPathDepth": 10, "allowRemoteImports": false,
                        "failOnSchemaMismatch": false},
            "rules": [], "staticDiagnostics": [], "recommendedIndexes": []
        });
        let mut manifest: Manifest = serde_json::from_value(json).unwrap();
        manifest.rules = rules;
        manifest
    }

    /// `violations` per rule name; `None` makes the summary time out.
    pub(crate) fn executor(counts: &[(&str, Option<u64>)]) -> FakeExecutor {
        let mut answers = HashMap::new();
        for (name, count) in counts {
            let summary = match count {
                Some(count) => Ok(vec![row(json!({"ruleId": name, "violationCount": count}))]),
                None => Err(ExecError::Timeout),
            };
            answers.insert(format!("{name} summary"), summary);
            let details = (0..count.unwrap_or(0))
                .map(|i| {
                    row(json!({
                        "ruleId": name,
                        "focus": {"label": "Person", "key": "id", "keyValue": format!("p{i}"), "elementId": format!("4:x:{i}")},
                        "value": null,
                        "message": format!("{name} failed on p{i}"),
                        "details": []
                    }))
                })
                .collect();
            answers.insert(format!("{name} detail"), Ok(details));
        }
        FakeExecutor {
            answers,
            calls: Vec::new(),
        }
    }

    #[test]
    fn runs_summaries_and_drills_into_failures() {
        let manifest = manifest(vec![
            rule("a", "Violation", "compiled"),
            rule("b", "Violation", "compiled"),
            rule("c", "Violation", "guaranteed-by-schema"),
            rule("d", "Violation", "unsupported"),
        ]);
        let mut executor = executor(&[("a", Some(2)), ("b", Some(0))]);
        let options = ValidateOptions {
            limit: None,
            ..ValidateOptions::default()
        };
        let report = validate(&manifest, &mut executor, &options).unwrap();

        let queries: Vec<&str> = executor.calls.iter().map(|(q, _)| q.as_str()).collect();
        assert_eq!(queries, ["a summary", "a detail", "b summary"]);
        assert_eq!(executor.calls[0].1.limit, i64::MAX);
        assert_eq!(executor.calls[0].1.sample_size, 5);

        let statuses: Vec<Status> = report.rules.iter().map(|r| r.status).collect();
        assert_eq!(
            statuses,
            [
                Status::Failed,
                Status::Passed,
                Status::GuaranteedBySchema,
                Status::Skipped
            ]
        );
        assert_eq!(report.rules[0].violations.len(), 2);
        assert_eq!(report.summary.violations, 2);
        assert!(!report.conforms);
        assert!(report.complete);
        assert_eq!(
            report.rules[3].status_reason.as_deref(),
            Some("unsupported")
        );
    }

    #[test]
    fn warnings_fail_only_at_the_warning_threshold() {
        let manifest = manifest(vec![
            rule("warn", "Warning", "compiled"),
            rule("ok", "Violation", "compiled"),
        ]);
        let mut executor = executor(&[("warn", Some(1)), ("ok", Some(0))]);
        let report = validate(&manifest, &mut executor, &ValidateOptions::default()).unwrap();
        assert_eq!(exit_code(&report, FailOn::Violation), 0);
        assert_eq!(exit_code(&report, FailOn::Warning), 1);
        assert_eq!(exit_code(&report, FailOn::Info), 1);
    }

    #[test]
    fn timeouts_are_reported_without_aborting() {
        let manifest = manifest(vec![
            rule("slow", "Violation", "compiled"),
            rule("fast", "Violation", "compiled"),
        ]);
        let mut executor = executor(&[("slow", None), ("fast", Some(0))]);
        let report = validate(&manifest, &mut executor, &ValidateOptions::default()).unwrap();
        assert_eq!(report.rules[0].status, Status::Timeout);
        assert_eq!(report.rules[0].violation_count, None);
        assert_eq!(report.rules[1].status, Status::Passed);
        assert!(!report.complete);
        assert_eq!(exit_code(&report, FailOn::Violation), 3);

        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(json["rules"][0]["status"], "timeout");
        assert!(json["rules"][1]["durationMs"].is_u64());
    }

    #[test]
    fn rejects_a_manifest_for_another_dialect() {
        let mut manifest = manifest(vec![]);
        manifest.dialect = "ladybug".into();
        let error =
            validate(&manifest, &mut executor(&[]), &ValidateOptions::default()).unwrap_err();
        assert!(error.contains("compiled for ladybug"), "{error}");
    }
}
