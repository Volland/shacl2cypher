//! End-to-end compilation: shapes files to a manifest of named diagnostic queries
//! and the matching `.cypher` file.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::ast::{Severity, Shapes};
use crate::hierarchy::{ClassHierarchy, LabelPolicy};
use crate::ir::{Expr, FocusSet, Rule, RuleStatus, ValueSource, Violation};
use crate::load::{InputSource, LoadOptions, RemoteFetcher, ShapesGraph, SourceLocation};
use crate::lower::{lower, Inputs, LowerOptions};
use crate::mapping::{LpgPath, ResolveOptions, Resolver};
use crate::render::{self, Dialect, RuleMeta};
use crate::schema::SchemaSnapshot;
use crate::schema_check::{fail_on_mismatch, SchemaChecker};

pub const MANIFEST_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone)]
pub struct CompileOptions {
    pub dialect: Dialect,
    pub node_key: Option<String>,
    /// `--neo4j-labels`; ignored for LadybugDB.
    pub label_policy: LabelPolicy,
    pub strict: bool,
    pub lenient: bool,
    pub verbose: bool,
    pub max_path_depth: u32,
    pub allow_remote_imports: bool,
    pub fail_on_schema_mismatch: bool,
    /// Directory that input paths are reported relative to.
    pub base_dir: Option<PathBuf>,
}

impl Default for CompileOptions {
    fn default() -> Self {
        CompileOptions {
            dialect: Dialect::Neo4j,
            node_key: None,
            label_policy: LabelPolicy::Explicit,
            strict: false,
            lenient: false,
            verbose: false,
            max_path_depth: 10,
            allow_remote_imports: false,
            fail_on_schema_mismatch: false,
            base_dir: None,
        }
    }
}

pub struct CompileRequest<'a> {
    pub shapes: &'a [PathBuf],
    pub ontologies: &'a [PathBuf],
    /// Schema snapshot JSON text.
    pub schema: Option<&'a str>,
    pub fetcher: Option<&'a dyn RemoteFetcher>,
}

/// Every problem that prevented compilation.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{}", .0.join("\n"))]
pub struct CompileError(pub Vec<String>);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compilation {
    pub manifest: Manifest,
    /// Contents of the `.cypher` file.
    pub cypher: String,
}

impl Manifest {
    /// Parses a `manifest.json` written by `compile`, rejecting other schema versions.
    pub fn from_json(text: &str) -> Result<Self, String> {
        let manifest: Manifest =
            serde_json::from_str(text).map_err(|e| format!("invalid manifest: {e}"))?;
        if manifest.schema_version != MANIFEST_SCHEMA_VERSION {
            return Err(format!(
                "unsupported manifest schemaVersion {} (expected {MANIFEST_SCHEMA_VERSION})",
                manifest.schema_version
            ));
        }
        Ok(manifest)
    }
}

impl Compilation {
    pub fn manifest_json(&self) -> String {
        let mut json =
            serde_json::to_string_pretty(&self.manifest).expect("manifests always serialize");
        json.push('\n');
        json
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Manifest {
    pub schema_version: u32,
    pub compiler_version: String,
    pub dialect: String,
    pub inputs: Vec<ManifestInput>,
    pub schema_snapshot_hash: Option<String>,
    pub options: ManifestOptions,
    pub rules: Vec<ManifestRule>,
    pub static_diagnostics: Vec<ManifestDiagnostic>,
    pub recommended_indexes: Vec<RecommendedIndex>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestInput {
    pub path: String,
    pub role: String,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestOptions {
    pub node_key: Option<String>,
    pub neo4j_labels: String,
    pub strict: bool,
    pub lenient: bool,
    pub verbose: bool,
    pub max_path_depth: u32,
    pub allow_remote_imports: bool,
    pub fail_on_schema_mismatch: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestRule {
    pub name: String,
    pub rule_id: String,
    pub fingerprint: String,
    pub shape: String,
    pub path: Option<String>,
    pub constraint: String,
    pub severity: String,
    pub status: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_reason: Option<String>,
    pub cost_class: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_depth_cap: Option<u32>,
    pub source: SourceRef,
    pub message: String,
    pub queries: Option<Queries>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRef {
    pub file: String,
    pub line: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Queries {
    pub detail: String,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestDiagnostic {
    pub code: String,
    pub message: String,
    pub source: SourceRef,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct RecommendedIndex {
    pub label: String,
    pub property: String,
    pub reason: String,
}

/// Compiles shapes files into a manifest and `.cypher` file.
// @lat: [[output#Manifest]]
// @lat: [[overview#Pipeline]]
pub fn compile(
    request: &CompileRequest<'_>,
    options: &CompileOptions,
) -> Result<Compilation, CompileError> {
    let dialect = options.dialect;
    if request.shapes.is_empty() {
        return Err(one("no shapes files were given"));
    }
    let schema = request
        .schema
        .map(SchemaSnapshot::from_json)
        .transpose()
        .map_err(one)?;
    if dialect == Dialect::Ladybug && schema.is_none() {
        return Err(one(
            "the LadybugDB dialect requires a schema snapshot (--schema)",
        ));
    }

    let load_options = LoadOptions {
        allow_remote_imports: options.allow_remote_imports,
    };
    let graph =
        ShapesGraph::load_with(request.shapes, load_options, request.fetcher).map_err(one)?;
    let ontologies = request
        .ontologies
        .iter()
        .map(|path| {
            ShapesGraph::load_with(std::slice::from_ref(path), load_options, request.fetcher)
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(one)?;
    let shapes = Shapes::from_graph(&graph).map_err(|e| CompileError(e.messages))?;
    let mut graphs = vec![&graph];
    graphs.extend(&ontologies);
    let hierarchy = ClassHierarchy::build(&graphs).map_err(one)?;
    let resolver = Resolver::new(
        &graph,
        &shapes,
        schema.as_ref(),
        ResolveOptions {
            strict: options.strict,
        },
    );
    let checker = schema
        .as_ref()
        .map(|schema| SchemaChecker::new(schema, dialect == Dialect::Ladybug));
    let lower_options = LowerOptions {
        label_policy: match dialect {
            Dialect::Neo4j => options.label_policy,
            Dialect::Ladybug => LabelPolicy::Explicit,
        },
        node_key: options.node_key.clone(),
        max_path_depth: options.max_path_depth,
        dialect_max_path_depth: dialect.max_path_depth(),
        lenient: options.lenient,
    };
    let inputs = Inputs {
        graph: &graph,
        shapes: &shapes,
        resolver: &resolver,
        hierarchy: &hierarchy,
        checker: checker.as_ref(),
    };
    let lowered = lower(inputs, &lower_options).map_err(|e| CompileError(e.0))?;
    if options.fail_on_schema_mismatch {
        fail_on_mismatch(&lowered.diagnostics, &graph).map_err(one)?;
    }

    let base = options
        .base_dir
        .as_deref()
        .and_then(|dir| std::fs::canonicalize(dir).ok());
    let base = base.as_deref();
    let namings = Namer {
        graph: &graph,
        shapes: &shapes,
    }
    .assign(&lowered.rules)?;

    let mut errors = Vec::new();
    let mut rules = Vec::new();
    let mut indexes = BTreeSet::new();
    for (rule, naming) in lowered.rules.iter().zip(namings) {
        let segments = &rule.id.0;
        let shape = segments[0].clone();
        let constraint = segments.last().cloned().unwrap_or_default();
        let path = (segments.len() > 2).then(|| segments[1..segments.len() - 1].join("/"));
        let severity = severity_name(&rule.severity, &graph);
        let message = rule.messages.first().map_or_else(
            || default_message(&shape, path.as_deref(), &constraint),
            |message| message.value().to_owned(),
        );
        let focus_key = shapes
            .get(&rule.shape)
            .and_then(|shape| shape.annotations.key.as_ref())
            .map(|key| key.value.clone())
            .or_else(|| options.node_key.clone());

        let mut status = rule.status.clone();
        let queries = if status == RuleStatus::Compiled {
            let meta = RuleMeta {
                name: &naming.name,
                shape: &shape,
                path: path.as_deref(),
                constraint: &constraint,
                severity: &severity,
                message: &message,
                focus_key: focus_key.as_deref(),
                value_key: options.node_key.as_deref(),
                verbose: options.verbose,
            };
            let rendered = match (dialect, schema.as_ref()) {
                (Dialect::Neo4j, _) => render::neo4j::render(rule, &meta),
                (Dialect::Ladybug, Some(schema)) => render::ladybug::render(rule, &meta, schema),
                (Dialect::Ladybug, None) => unreachable!("checked before lowering"),
            };
            match rendered {
                Ok(rendered) => Some(Queries {
                    detail: rendered.detail,
                    summary: rendered.summary,
                }),
                Err(error) if options.lenient => {
                    status = RuleStatus::Unsupported(error.0);
                    None
                }
                Err(error) => {
                    errors.push(format!(
                        "{}: rule {}: {error}",
                        graph.display_location(rule.location),
                        naming.rule_id
                    ));
                    None
                }
            }
        } else {
            None
        };

        if queries.is_some() {
            if let Some(key) = &focus_key {
                for set in &rule.focus {
                    if let FocusSet::Labels(labels) = set {
                        indexes.extend(labels.iter().map(|label| RecommendedIndex {
                            label: label.clone(),
                            property: key.clone(),
                            reason: "focus key".into(),
                        }));
                    }
                }
            }
        }

        let (status_name, status_reason) = status_parts(status);
        rules.push(ManifestRule {
            name: naming.name,
            rule_id: naming.rule_id,
            fingerprint: fingerprint(dialect, rule),
            shape,
            path,
            constraint,
            severity,
            status: status_name.into(),
            status_reason,
            cost_class: cost_class(rule).into(),
            path_depth_cap: rule.path_depth_cap,
            source: source_ref(&graph, rule.location, base),
            message,
            queries,
        });
    }
    if !errors.is_empty() {
        return Err(CompileError(errors));
    }
    rules.sort_by(|a, b| a.rule_id.cmp(&b.rule_id));

    let given = request
        .shapes
        .iter()
        .filter_map(|path| std::fs::canonicalize(path).ok())
        .collect::<BTreeSet<_>>()
        .len();
    let mut manifest_inputs: Vec<ManifestInput> = graph
        .inputs
        .iter()
        .enumerate()
        .map(|(index, input)| ManifestInput {
            path: display_input(&input.source, base),
            role: if index < given { "shapes" } else { "import" }.into(),
            sha256: input.sha256.clone(),
        })
        .collect();
    for ontology in &ontologies {
        manifest_inputs.extend(ontology.inputs.iter().enumerate().map(|(index, input)| {
            ManifestInput {
                path: display_input(&input.source, base),
                role: if index == 0 { "ontology" } else { "import" }.into(),
                sha256: input.sha256.clone(),
            }
        }));
    }

    let manifest = Manifest {
        schema_version: MANIFEST_SCHEMA_VERSION,
        compiler_version: env!("CARGO_PKG_VERSION").to_owned(),
        dialect: dialect.name().to_owned(),
        inputs: manifest_inputs,
        schema_snapshot_hash: request.schema.map(|text| sha256_hex(text.as_bytes())),
        options: ManifestOptions {
            node_key: options.node_key.clone(),
            neo4j_labels: match options.label_policy {
                LabelPolicy::Explicit => "explicit",
                LabelPolicy::Inherited => "inherited",
            }
            .into(),
            strict: options.strict,
            lenient: options.lenient,
            verbose: options.verbose,
            max_path_depth: options.max_path_depth,
            allow_remote_imports: options.allow_remote_imports,
            fail_on_schema_mismatch: options.fail_on_schema_mismatch,
        },
        rules,
        static_diagnostics: lowered
            .diagnostics
            .iter()
            .map(|diagnostic| ManifestDiagnostic {
                code: diagnostic.code.to_owned(),
                message: diagnostic.message.clone(),
                source: source_ref(&graph, diagnostic.location, base),
            })
            .collect(),
        recommended_indexes: indexes.into_iter().collect(),
    };
    let cypher = cypher_file(&manifest);
    Ok(Compilation { manifest, cypher })
}

struct Naming {
    rule_id: String,
    name: String,
}

struct Namer<'a> {
    graph: &'a ShapesGraph,
    shapes: &'a Shapes,
}

impl Namer<'_> {
    /// Structural ids and names, with a content hash on collisions.
    // @lat: [[output#Rule Naming]]
    fn assign(&self, rules: &[Rule]) -> Result<Vec<Naming>, CompileError> {
        let base_ids: Vec<String> = rules.iter().map(|rule| rule.id.to_string()).collect();
        let mut id_counts: HashMap<&str, usize> = HashMap::new();
        for id in &base_ids {
            *id_counts.entry(id).or_default() += 1;
        }
        let namings: Vec<Naming> = rules
            .iter()
            .zip(&base_ids)
            .map(|(rule, base_id)| {
                let name = self.name(rule);
                if id_counts[base_id.as_str()] > 1 {
                    let hash = content_hash(rule);
                    Naming {
                        rule_id: format!("{base_id}~{hash}"),
                        name: format!("{name}~{hash}"),
                    }
                } else {
                    Naming {
                        rule_id: base_id.clone(),
                        name,
                    }
                }
            })
            .collect();

        let mut errors = Vec::new();
        for (what, key) in [
            (
                "rule id",
                (|n: &Naming| n.rule_id.as_str()) as fn(&Naming) -> &str,
            ),
            ("rule name", |n: &Naming| n.name.as_str()),
        ] {
            let mut seen: HashMap<&str, Vec<usize>> = HashMap::new();
            for (index, naming) in namings.iter().enumerate() {
                seen.entry(key(naming)).or_default().push(index);
            }
            let mut duplicates: Vec<(&str, Vec<usize>)> = seen
                .into_iter()
                .filter(|(_, indexes)| indexes.len() > 1)
                .collect();
            duplicates.sort();
            for (value, indexes) in duplicates {
                let locations: Vec<String> = indexes
                    .iter()
                    .map(|&index| self.graph.display_location(rules[index].location))
                    .collect();
                errors.push(format!(
                    "duplicate {what} `{value}` (at {})",
                    locations.join(", ")
                ));
            }
        }
        if errors.is_empty() {
            Ok(namings)
        } else {
            Err(CompileError(errors))
        }
    }

    fn name(&self, rule: &Rule) -> String {
        let segments = &rule.id.0;
        let component = local_name(segments.last().map_or("", String::as_str));
        let path =
            (segments.len() > 2).then(|| flatten_path(&segments[1..segments.len() - 1].join("/")));
        if rule.declared_by != rule.shape {
            if let Some(name) = self
                .shapes
                .get(&rule.declared_by)
                .and_then(|shape| shape.annotations.name.as_ref())
            {
                return format!("{}.{component}", name.value);
            }
        }
        let shape = self
            .shapes
            .get(&rule.shape)
            .and_then(|shape| shape.annotations.name.as_ref())
            .map_or_else(|| shape_name(&segments[0]), |name| name.value.clone());
        match path {
            Some(path) => format!("{shape}.{path}.{component}"),
            None => format!("{shape}.{component}"),
        }
    }
}

/// `ex:PersonShape` -> `PersonShape`; blank shapes `[]@file.ttl:12` -> `file_ttl_12`.
fn shape_name(segment: &str) -> String {
    match segment.strip_prefix("[]@") {
        Some(location) => sanitize(location),
        None => local_name(segment),
    }
}

/// The part after the last `:` (or `/`, `#` inside `<…>`), sanitized.
fn local_name(token: &str) -> String {
    let token = token.trim();
    let local = match token.strip_prefix('<').and_then(|t| t.strip_suffix('>')) {
        Some(iri) => iri.rsplit(['/', '#']).next().unwrap_or(iri),
        None => token.rsplit(':').next().unwrap_or(token),
    };
    sanitize(local)
}

/// Path syntax as identifier characters: `^ex:a/ex:b+` -> `inv_a.b_plus`.
fn flatten_path(path: &str) -> String {
    let mut out = String::new();
    let mut token = String::new();
    let flush = |token: &mut String, out: &mut String| {
        if !token.is_empty() {
            out.push_str(&local_name(token));
            token.clear();
        }
    };
    let mut in_iri = false;
    for c in path.chars() {
        match c {
            '<' => {
                in_iri = true;
                token.push(c);
            }
            '>' => {
                in_iri = false;
                token.push(c);
            }
            _ if in_iri => token.push(c),
            '/' => {
                flush(&mut token, &mut out);
                out.push('.');
            }
            '|' => {
                flush(&mut token, &mut out);
                out.push_str("_or_");
            }
            '^' => {
                flush(&mut token, &mut out);
                out.push_str("inv_");
            }
            '*' => {
                flush(&mut token, &mut out);
                out.push_str("_star");
            }
            '+' => {
                flush(&mut token, &mut out);
                out.push_str("_plus");
            }
            '?' => {
                flush(&mut token, &mut out);
                out.push_str("_opt");
            }
            '(' | ')' => flush(&mut token, &mut out),
            _ => token.push(c),
        }
    }
    flush(&mut token, &mut out);
    out
}

fn sanitize(text: &str) -> String {
    text.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

static LOCATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"SourceLocation \{ file: \d+, line: \d+ \}").unwrap());
static BLANK_SHAPE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\[\]@[^/"]*"#).unwrap());

/// The rule's meaning without source positions.
fn canonical(rule: &Rule) -> String {
    let text = format!(
        "{:?}|{:?}|{:?}|{:?}|{:?}",
        rule.focus, rule.violation, rule.details, rule.severity, rule.messages
    );
    let text = LOCATION.replace_all(&text, "_");
    BLANK_SHAPE.replace_all(&text, "[]").into_owned()
}

fn content_hash(rule: &Rule) -> String {
    sha256_hex(canonical(rule).as_bytes())[..6].to_owned()
}

// @lat: [[output#Rule Naming]]
fn fingerprint(dialect: Dialect, rule: &Rule) -> String {
    sha256_hex(format!("{}|{}", dialect.name(), canonical(rule)).as_bytes())[..16].to_owned()
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Static cost estimate; see `lat.md/output.md#Cost Classes`.
// @lat: [[output#Cost Classes]]
fn cost_class(rule: &Rule) -> &'static str {
    #[derive(Default)]
    struct Cost {
        expand: bool,
        quadratic: bool,
    }
    fn path(p: &LpgPath, cost: &mut Cost) {
        match p {
            LpgPath::Property(_) => {}
            LpgPath::Relationship { .. } => cost.expand = true,
            LpgPath::Sequence(items) | LpgPath::Alternative(items) => {
                items.iter().for_each(|item| path(item, cost))
            }
            LpgPath::Repeat { path: inner, .. } => {
                cost.expand = true;
                path(inner, cost);
            }
        }
    }
    fn source(s: &ValueSource, cost: &mut Cost) {
        if let ValueSource::Path(p) = s {
            path(p, cost);
        }
    }
    fn expr(e: &Expr, cost: &mut Cost) {
        match e {
            Expr::Bool(_) | Expr::Test { .. } | Expr::Closed { .. } => {}
            Expr::Not(inner) => expr(inner, cost),
            Expr::And(items) | Expr::Or(items) | Expr::Xone(items) => {
                items.iter().for_each(|item| expr(item, cost))
            }
            Expr::All {
                source: s, test, ..
            } => {
                source(s, cost);
                expr(test, cost);
            }
            Expr::Count {
                source: s, filter, ..
            } => {
                source(s, cost);
                if let Some(filter) = filter {
                    expr(filter, cost);
                }
            }
            Expr::Pair { left, right, .. } => {
                let left_nodes = matches!(left, ValueSource::Path(p) if p.yields_nodes());
                if left_nodes || right.yields_nodes() {
                    cost.quadratic = true;
                }
                source(left, cost);
                path(right, cost);
            }
        }
    }

    let mut cost = Cost::default();
    for set in &rule.focus {
        match set {
            FocusSet::SubjectsOf(p) | FocusSet::ObjectsOf(p) => path(p, &mut cost),
            FocusSet::Labels(_) | FocusSet::Relationships(_) => {}
        }
    }
    match &rule.violation {
        Violation::PerValue {
            source: s, test, ..
        } => {
            source(s, &mut cost);
            expr(test, &mut cost);
        }
        Violation::PerFocus { condition, .. } => expr(condition, &mut cost),
    }
    for detail in &rule.details {
        expr(&detail.holds, &mut cost);
    }
    if rule.path_depth_cap.is_some() {
        "unbounded-path"
    } else if cost.quadratic {
        "quadratic"
    } else if cost.expand {
        "scan+expand"
    } else {
        "scan"
    }
}

fn status_parts(status: RuleStatus) -> (&'static str, Option<String>) {
    match status {
        RuleStatus::Compiled => ("compiled", None),
        RuleStatus::GuaranteedBySchema => ("guaranteed-by-schema", None),
        RuleStatus::SchemaMismatch => ("schema-mismatch", None),
        RuleStatus::Deactivated => ("deactivated", None),
        RuleStatus::Unsupported(reason) => ("unsupported", Some(reason)),
    }
}

fn severity_name(severity: &Severity, graph: &ShapesGraph) -> String {
    match severity {
        Severity::Violation => "Violation".into(),
        Severity::Warning => "Warning".into(),
        Severity::Info => "Info".into(),
        Severity::Other(iri) => graph.compact(iri.as_str()),
    }
}

fn default_message(shape: &str, path: Option<&str>, constraint: &str) -> String {
    match path {
        Some(path) => format!("{shape} violates {constraint} on {path}"),
        None => format!("{shape} violates {constraint}"),
    }
}

fn display_input(source: &InputSource, base: Option<&Path>) -> String {
    match source {
        InputSource::File(path) => base
            .and_then(|base| path.strip_prefix(base).ok())
            .unwrap_or(path)
            .display()
            .to_string(),
        InputSource::Remote(iri) => iri.clone(),
    }
}

fn source_ref(graph: &ShapesGraph, location: SourceLocation, base: Option<&Path>) -> SourceRef {
    SourceRef {
        file: display_input(&graph.inputs[location.file].source, base),
        line: location.line,
    }
}

fn cypher_file(manifest: &Manifest) -> String {
    let mut out = format!(
        "// Generated by shacl2cypher {} for {}. Read-only diagnostic queries.\n",
        manifest.compiler_version, manifest.dialect
    );
    for rule in &manifest.rules {
        let Some(queries) = &rule.queries else {
            continue;
        };
        out.push_str(&format!(
            "\n// name: {name}\n// ruleId: {id}\n{detail};\n\n// name: {name}#summary\n// ruleId: {id}\n{summary};\n",
            name = rule.name,
            id = rule.rule_id,
            detail = queries.detail,
            summary = queries.summary,
        ));
    }
    out
}

fn one(error: impl std::fmt::Display) -> CompileError {
    CompileError(vec![error.to_string()])
}

#[cfg(test)]
mod tests {
    use super::*;

    const PREFIXES: &str = "@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix ex: <http://example.org/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix s2c: <https://w3id.org/shacl2cypher#> .
";

    struct Project {
        dir: tempfile::TempDir,
    }

    impl Project {
        fn new(files: &[(&str, &str)]) -> Self {
            let dir = tempfile::tempdir().unwrap();
            for (name, body) in files {
                std::fs::write(dir.path().join(name), format!("{PREFIXES}{body}")).unwrap();
            }
            Project { dir }
        }

        fn compile_files(
            &self,
            names: &[&str],
            schema: Option<&str>,
            options: CompileOptions,
        ) -> Result<Compilation, CompileError> {
            let shapes: Vec<PathBuf> = names.iter().map(|n| self.dir.path().join(n)).collect();
            let request = CompileRequest {
                shapes: &shapes,
                ontologies: &[],
                schema,
                fetcher: None,
            };
            let options = CompileOptions {
                base_dir: Some(self.dir.path().to_path_buf()),
                ..options
            };
            compile(&request, &options)
        }

        fn compile(&self, options: CompileOptions) -> Result<Compilation, CompileError> {
            self.compile_files(&["shapes.ttl"], None, options)
        }
    }

    fn neo4j() -> CompileOptions {
        CompileOptions {
            node_key: Some("id".into()),
            ..CompileOptions::default()
        }
    }

    fn rule<'m>(manifest: &'m Manifest, name: &str) -> &'m ManifestRule {
        manifest
            .rules
            .iter()
            .find(|rule| rule.name == name)
            .unwrap_or_else(|| {
                let names: Vec<&str> = manifest.rules.iter().map(|r| r.name.as_str()).collect();
                panic!("no rule named {name}; rules: {names:?}")
            })
    }

    const PERSON: &str = "ex:PersonShape sh:targetClass ex:Person ;
    sh:property [ sh:path ex:name ; sh:minCount 1 ] ,
                [ sh:path ex:worksFor ; sh:class ex:Company ] ,
                [ sh:path [ sh:oneOrMorePath ex:knows ] ; sh:class ex:Person ] .
";

    #[test]
    fn compiles_a_manifest_and_cypher_file() {
        let project = Project::new(&[("shapes.ttl", PERSON)]);
        let compilation = project.compile(neo4j()).unwrap();
        let manifest = &compilation.manifest;
        assert_eq!(manifest.schema_version, MANIFEST_SCHEMA_VERSION);
        assert_eq!(manifest.dialect, "neo4j");
        assert_eq!(manifest.inputs.len(), 1);
        assert_eq!(manifest.inputs[0].path, "shapes.ttl");
        assert_eq!(manifest.inputs[0].role, "shapes");
        assert_eq!(manifest.inputs[0].sha256.len(), 64);

        let min_count = rule(manifest, "PersonShape.name.minCount");
        assert_eq!(min_count.rule_id, "ex:PersonShape/ex:name/sh:minCount");
        assert_eq!(min_count.status, "compiled");
        assert_eq!(min_count.cost_class, "scan");
        assert_eq!(min_count.source.file, "shapes.ttl");
        assert_eq!(min_count.source.line, 6);
        assert_eq!(
            min_count.message,
            "ex:PersonShape violates sh:minCount on ex:name"
        );
        assert!(min_count.queries.is_some());
        assert_eq!(
            rule(manifest, "PersonShape.worksFor.class").cost_class,
            "scan+expand"
        );
        let transitive = rule(manifest, "PersonShape.knows_plus.class");
        assert_eq!(transitive.cost_class, "unbounded-path");
        assert_eq!(transitive.path_depth_cap, Some(10));
        assert_eq!(
            manifest.recommended_indexes,
            vec![RecommendedIndex {
                label: "Person".into(),
                property: "id".into(),
                reason: "focus key".into()
            }]
        );

        assert!(compilation
            .cypher
            .starts_with("// Generated by shacl2cypher"));
        assert!(compilation
            .cypher
            .contains("\n// name: PersonShape.name.minCount\n// ruleId: ex:PersonShape/ex:name/sh:minCount\nMATCH"));
        assert!(compilation
            .cypher
            .contains("\n// name: PersonShape.name.minCount#summary\n"));
        assert_eq!(
            &Manifest::from_json(&compilation.manifest_json()).unwrap(),
            manifest
        );
        let json: serde_json::Value = serde_json::from_str(&compilation.manifest_json()).unwrap();
        assert_eq!(
            json["rules"][0]["ruleId"],
            "ex:PersonShape/ex:knows+/sh:class"
        );
        assert!(json["rules"][0].get("statusReason").is_none());
    }

    #[test]
    fn output_is_deterministic_across_runs_and_input_order() {
        let project = Project::new(&[
            ("a.ttl", "ex:PersonShape sh:targetClass ex:Person .\n"),
            ("b.ttl", "ex:PersonShape sh:property [ sh:path ex:name ; sh:minCount 1 ] , [ sh:path ex:age ; sh:maxCount 1 ] .\n"),
        ]);
        let first = project
            .compile_files(&["a.ttl", "b.ttl"], None, neo4j())
            .unwrap();
        let second = project
            .compile_files(&["b.ttl", "a.ttl"], None, neo4j())
            .unwrap();
        assert_eq!(first.manifest_json(), second.manifest_json());
        assert_eq!(first.cypher, second.cypher);
    }

    // @lat: [[tests#Manifest#Collision Hashes]]
    #[test]
    fn colliding_ids_get_stable_content_hashes() {
        let body = "ex:PersonShape sh:targetClass ex:Person ;
    sh:property [ sh:path ex:employer ; sh:class ex:Company , ex:Agency ] .
";
        let project = Project::new(&[("shapes.ttl", body)]);
        let manifest = project.compile(neo4j()).unwrap().manifest;
        let ids: Vec<&str> = manifest.rules.iter().map(|r| r.rule_id.as_str()).collect();
        assert_eq!(ids.len(), 2);
        assert_ne!(ids[0], ids[1]);
        for rule in &manifest.rules {
            assert!(
                rule.rule_id
                    .starts_with("ex:PersonShape/ex:employer/sh:class~"),
                "{}",
                rule.rule_id
            );
            assert_eq!(
                rule.rule_id.len(),
                "ex:PersonShape/ex:employer/sh:class~".len() + 6
            );
            assert!(rule.name.starts_with("PersonShape.employer.class~"));
        }

        let moved = Project::new(&[("shapes.ttl", &format!("\n\n{body}"))]);
        let moved_ids: Vec<String> = moved
            .compile(neo4j())
            .unwrap()
            .manifest
            .rules
            .into_iter()
            .map(|r| r.rule_id)
            .collect();
        assert_eq!(moved_ids, ids);
    }

    // @lat: [[tests#Manifest#Explicit Names]]
    #[test]
    fn explicit_names_override_and_duplicates_fail() {
        let project = Project::new(&[(
            "shapes.ttl",
            "ex:PersonShape sh:targetClass ex:Person ;
    s2c:name \"person\" ;
    sh:property [ sh:path ex:name ; sh:minCount 1 ; s2c:name \"person-name\" ] ,
                [ sh:path ex:age ; sh:maxCount 1 ] .
",
        )]);
        let manifest = project.compile(neo4j()).unwrap().manifest;
        rule(&manifest, "person-name.minCount");
        rule(&manifest, "person.age.maxCount");

        let duplicate = Project::new(&[(
            "shapes.ttl",
            "ex:A sh:targetClass ex:Person ;
    sh:property [ sh:path ex:name ; sh:minCount 1 ; s2c:name \"person-check\" ] .
ex:B sh:targetClass ex:Company ;
    sh:property [ sh:path ex:title ; sh:minCount 1 ; s2c:name \"person-check\" ] .
",
        )]);
        let message = duplicate.compile(neo4j()).unwrap_err().to_string();
        assert!(
            message.contains("duplicate rule name `person-check.minCount`"),
            "{message}"
        );
        assert!(message.contains("shapes.ttl:6"), "{message}");
        assert!(message.contains("shapes.ttl:8"), "{message}");
    }

    // @lat: [[tests#Manifest#Fingerprints]]
    #[test]
    fn fingerprints_track_meaning_and_dialect_not_ids() {
        let schema = r#"{"nodeTypes": [{"name": "Person", "properties": [{"name": "id", "type": "STRING"}, {"name": "name", "type": "STRING"}]}]}"#;
        let fingerprint_of = |bound: u32, dialect: Dialect| {
            let project = Project::new(&[(
                "shapes.ttl",
                &format!(
                    "ex:PersonShape sh:targetClass ex:Person ;\n    sh:property [ sh:path ex:name ; sh:maxCount {bound} ] .\n"
                ),
            )]);
            let options = CompileOptions { dialect, ..neo4j() };
            let manifest = project
                .compile_files(&["shapes.ttl"], Some(schema), options)
                .unwrap()
                .manifest;
            (
                manifest.rules[0].rule_id.clone(),
                manifest.rules[0].fingerprint.clone(),
            )
        };
        let (id_one, one) = fingerprint_of(1, Dialect::Neo4j);
        let (id_two, two) = fingerprint_of(2, Dialect::Neo4j);
        let (_, ladybug) = fingerprint_of(1, Dialect::Ladybug);
        assert_eq!(id_one, id_two);
        assert_ne!(one, two);
        assert_ne!(one, ladybug);
        assert_eq!(one, fingerprint_of(1, Dialect::Neo4j).1);
    }

    #[test]
    fn pair_constraints_over_relationships_are_quadratic() {
        let project = Project::new(&[(
            "shapes.ttl",
            "ex:PersonShape sh:targetClass ex:Person ;
    sh:property [ sh:path ex:friend ; sh:class ex:Person ; sh:disjoint ex:enemy ] .
ex:enemy s2c:relationship \"ENEMY\" .
",
        )]);
        let manifest = project.compile(neo4j()).unwrap().manifest;
        assert_eq!(
            rule(&manifest, "PersonShape.friend.disjoint").cost_class,
            "quadratic"
        );
    }

    #[test]
    fn ladybug_needs_a_schema_and_reports_schema_statuses() {
        let project = Project::new(&[(
            "shapes.ttl",
            "ex:PersonShape sh:targetClass ex:Person ;
    sh:property [ sh:path ex:name ; sh:datatype xsd:string ] ,
                [ sh:path ex:name ; sh:datatype xsd:integer ] .
",
        )]);
        let ladybug = CompileOptions {
            dialect: Dialect::Ladybug,
            ..neo4j()
        };
        let message = project.compile(ladybug.clone()).unwrap_err().to_string();
        assert!(message.contains("requires a schema snapshot"), "{message}");

        let schema = r#"{"nodeTypes": [{"name": "Person", "properties": [{"name": "id", "type": "STRING"}, {"name": "name", "type": "STRING"}]}]}"#;
        let compilation = project
            .compile_files(&["shapes.ttl"], Some(schema), ladybug.clone())
            .unwrap();
        let manifest = &compilation.manifest;
        let statuses: BTreeSet<&str> = manifest.rules.iter().map(|r| r.status.as_str()).collect();
        assert_eq!(
            statuses,
            BTreeSet::from(["guaranteed-by-schema", "schema-mismatch"])
        );
        assert!(manifest.rules.iter().all(|r| r.queries.is_none()));
        assert_eq!(manifest.static_diagnostics.len(), 1);
        assert_eq!(manifest.static_diagnostics[0].code, "s2c:SchemaMismatch");
        assert_eq!(
            manifest.schema_snapshot_hash.as_deref(),
            Some(sha256_hex(schema.as_bytes()).as_str())
        );

        let failing = CompileOptions {
            fail_on_schema_mismatch: true,
            ..ladybug
        };
        let message = project
            .compile_files(&["shapes.ttl"], Some(schema), failing)
            .unwrap_err()
            .to_string();
        assert!(message.contains("--fail-on-schema-mismatch"), "{message}");
    }

    #[test]
    fn render_errors_fail_or_become_unsupported_when_lenient() {
        let project = Project::new(&[(
            "shapes.ttl",
            "ex:PersonShape sh:targetClass ex:Person ;\n    sh:property [ sh:path ex:name ; sh:pattern \"(a)\\\\1\" ] .\n",
        )]);
        let schema = r#"{"nodeTypes": [{"name": "Person", "properties": [{"name": "name", "type": "STRING"}]}]}"#;
        let ladybug = CompileOptions {
            dialect: Dialect::Ladybug,
            ..neo4j()
        };
        let message = project
            .compile_files(&["shapes.ttl"], Some(schema), ladybug.clone())
            .unwrap_err()
            .to_string();
        assert!(message.contains("back-references"), "{message}");

        let lenient = CompileOptions {
            lenient: true,
            ..ladybug
        };
        let manifest = project
            .compile_files(&["shapes.ttl"], Some(schema), lenient)
            .unwrap()
            .manifest;
        assert_eq!(manifest.rules[0].status, "unsupported");
        assert!(manifest.rules[0]
            .status_reason
            .as_deref()
            .unwrap()
            .contains("back-references"));
        assert!(manifest.rules[0].queries.is_none());
    }

    #[test]
    fn verbose_rows_include_focus_properties() {
        let project = Project::new(&[("shapes.ttl", PERSON)]);
        let verbose = CompileOptions {
            verbose: true,
            ..neo4j()
        };
        let manifest = project.compile(verbose).unwrap().manifest;
        let queries = rule(&manifest, "PersonShape.name.minCount")
            .queries
            .as_ref()
            .unwrap();
        assert!(queries.detail.contains("properties: properties(v0)}"));
    }

    #[test]
    fn flattens_paths_into_names() {
        assert_eq!(flatten_path("^ex:a/ex:b+"), "inv_a.b_plus");
        assert_eq!(flatten_path("ex:email|ex:phone"), "email_or_phone");
        assert_eq!(flatten_path("(ex:a/ex:b)*"), "a.b_star");
        assert_eq!(flatten_path("<http://x.org/ns#val>"), "val");
        assert_eq!(shape_name("[]@shapes.ttl:12"), "shapes_ttl_12");
    }
}
