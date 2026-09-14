//! Lowering of shapes into IR rules: one rule per constraint of every targeted
//! shape, with referenced shapes inlined as `conforms` expressions.

use std::collections::{BTreeSet, HashSet};

use oxrdf::{NamedNode, NamedNodeRef, NamedOrBlankNode, Term};

use crate::ast::{
    Constraint, IriAsString, Located, NodeKind, Path, Shape, ShapeId, Shapes, Target,
};
use crate::cycles::recursive_references;
use crate::datatypes::DatatypeCheck;
use crate::hierarchy::{ClassHierarchy, LabelPolicy};
use crate::ir::{
    Cmp, Constant, Detail, Expr, FocusSet, PairRelation, Rule, RuleId, RuleStatus, ValueSource,
    ValueTest, Var, Violation,
};
use crate::load::{ShapesGraph, SourceLocation};
use crate::mapping::{local_name, LpgPath, Resolver};
use crate::schema_check::{SchemaChecker, SchemaStatus, StaticDiagnostic, SCHEMA_MISMATCH};
use crate::xsd_regex::XsdRegex;

const RDF_LANG_STRING: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString";

#[derive(Debug, Clone)]
pub struct LowerOptions {
    /// `--neo4j-labels`; LadybugDB always uses [`LabelPolicy::Explicit`].
    pub label_policy: LabelPolicy,
    /// `--node-key`, used to compare IRI constants with node values.
    pub node_key: Option<String>,
    /// `--max-path-depth`: cap for unbounded repeated paths.
    pub max_path_depth: u32,
    /// Largest path depth the dialect accepts (LadybugDB: 30).
    pub dialect_max_path_depth: Option<u32>,
    /// `--lenient`: rejected features become `unsupported` rules instead of errors.
    pub lenient: bool,
}

impl Default for LowerOptions {
    fn default() -> Self {
        LowerOptions {
            label_policy: LabelPolicy::Explicit,
            node_key: None,
            max_path_depth: 10,
            dialect_max_path_depth: None,
            lenient: false,
        }
    }
}

/// What lowering needs from the earlier compilation stages.
#[derive(Clone, Copy)]
pub struct Inputs<'a> {
    pub graph: &'a ShapesGraph,
    pub shapes: &'a Shapes,
    pub resolver: &'a Resolver<'a>,
    pub hierarchy: &'a ClassHierarchy,
    pub checker: Option<&'a SchemaChecker<'a>>,
}

#[derive(Debug, Default)]
pub struct Lowered {
    pub rules: Vec<Rule>,
    pub diagnostics: Vec<StaticDiagnostic>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{}", .0.join("\n"))]
pub struct LowerErrors(pub Vec<String>);

/// Lowers every targeted shape into rules, collecting every error.
// @lat: [[semantics#Violations and Conforms#Lowering Rules]]
pub fn lower(inputs: Inputs<'_>, options: &LowerOptions) -> Result<Lowered, LowerErrors> {
    let mut lowerer = Lowerer {
        inputs,
        options,
        out: Lowered::default(),
        errors: Vec::new(),
        recursive: HashSet::new(),
        next_var: 0,
        flags: RuleFlags::default(),
    };
    lowerer.run();
    if lowerer.errors.is_empty() {
        Ok(lowerer.out)
    } else {
        lowerer.errors.sort();
        lowerer.errors.dedup();
        Err(LowerErrors(lowerer.errors))
    }
}

/// Facts gathered while lowering one rule, including its inlined shapes.
#[derive(Default)]
struct RuleFlags {
    capped: bool,
    unsupported: Option<String>,
}

struct Focus {
    /// The targeted shape.
    shape: ShapeId,
    sets: Vec<FocusSet>,
    /// Labels of the focus nodes when every target is a class target.
    path_labels: Vec<String>,
    relationship: bool,
    unsupported: Option<String>,
}

struct Scope<'s> {
    /// Shape declaring the constraint.
    shape: &'s Shape,
    labels: &'s [String],
    /// Schema statuses and diagnostics only apply to top-level rules.
    top_level: bool,
    relationship: bool,
    /// Whether the bound focus value is a node.
    focus_nodes: bool,
}

struct Item {
    component: String,
    violation: Violation,
    details: Vec<Detail>,
    status: RuleStatus,
}

impl Item {
    fn per_value(component: &str, of: Var, source: &ValueSource, var: Var, test: Expr) -> Item {
        Item {
            component: component.to_owned(),
            violation: Violation::PerValue {
                of,
                source: source.clone(),
                var,
                test,
            },
            details: Vec::new(),
            status: RuleStatus::Compiled,
        }
    }

    fn per_focus(component: &str, of: Var, condition: Expr) -> Item {
        Item {
            component: component.to_owned(),
            violation: Violation::PerFocus { of, condition },
            details: Vec::new(),
            status: RuleStatus::Compiled,
        }
    }

    fn with_status(mut self, status: RuleStatus) -> Item {
        self.status = status;
        self
    }

    fn with_details(mut self, details: Vec<Detail>) -> Item {
        self.details = details;
        self
    }
}

enum Membership {
    Test(ValueTest),
    Never,
}

impl Membership {
    fn apply(self, var: Var) -> Expr {
        match self {
            Membership::Test(test) => Expr::test(var, test),
            Membership::Never => Expr::Bool(false),
        }
    }
}

struct Lowerer<'a> {
    inputs: Inputs<'a>,
    options: &'a LowerOptions,
    out: Lowered,
    errors: Vec<String>,
    recursive: HashSet<ShapeId>,
    next_var: u32,
    flags: RuleFlags,
}

impl<'a> Lowerer<'a> {
    fn run(&mut self) {
        if let Some(limit) = self.options.dialect_max_path_depth {
            if self.options.max_path_depth > limit {
                self.errors.push(format!(
                    "--max-path-depth {} exceeds the dialect's maximum path depth of {limit}",
                    self.options.max_path_depth
                ));
                return;
            }
        }
        let graph = self.inputs.graph;
        for cycle in recursive_references(self.inputs.shapes) {
            if self.options.lenient {
                self.recursive
                    .extend(cycle.steps.iter().map(|step| step.from.clone()));
            } else {
                self.errors.push(format!(
                    "recursive shape reference: {}",
                    cycle.describe(graph)
                ));
            }
        }
        if !self.errors.is_empty() {
            return;
        }
        let shapes = self.inputs.shapes;
        for shape in shapes.iter().filter(|shape| !shape.targets.is_empty()) {
            self.targeted_shape(shape);
        }
        self.out.diagnostics.sort_by(|a, b| {
            a.location
                .cmp(&b.location)
                .then_with(|| a.message.cmp(&b.message))
        });
        self.out.diagnostics.dedup();
    }

    fn targeted_shape(&mut self, shape: &'a Shape) {
        let Some(focus) = self.focus(shape) else {
            return;
        };
        let name = self.shape_name(shape);
        let status = if shape.deactivated {
            Some(RuleStatus::Deactivated)
        } else {
            focus.unsupported.clone().map(RuleStatus::Unsupported)
        };

        match &shape.path {
            None => self.rules_for(
                shape,
                &focus,
                &ValueSource::Focus,
                vec![name.clone()],
                status.clone(),
                false,
            ),
            Some(path) => {
                if let Some((resolved, capped)) = self.top_level_path(shape, &focus) {
                    let prefix = vec![name.clone(), self.path_display(&path.value)];
                    self.rules_for(
                        shape,
                        &focus,
                        &ValueSource::Path(resolved),
                        prefix,
                        status.clone(),
                        capped,
                    );
                }
            }
        }

        let shapes = self.inputs.shapes;
        for property in &shape.properties {
            let Some(property_shape) = shapes.get(&property.value) else {
                continue;
            };
            let Some(path) = &property_shape.path else {
                self.error(
                    property.location,
                    "sh:property must reference a property shape with sh:path",
                );
                continue;
            };
            let Some((resolved, capped)) = self.top_level_path(property_shape, &focus) else {
                continue;
            };
            let prefix = vec![name.clone(), self.path_display(&path.value)];
            let status = if property_shape.deactivated {
                Some(RuleStatus::Deactivated)
            } else {
                status.clone()
            };
            self.rules_for(
                property_shape,
                &focus,
                &ValueSource::Path(resolved),
                prefix,
                status,
                capped,
            );
        }
    }

    fn focus(&mut self, shape: &'a Shape) -> Option<Focus> {
        let graph = self.inputs.graph;
        let resolver = self.inputs.resolver;
        let mut labels = BTreeSet::new();
        let mut sets = Vec::new();
        let (mut node_targets, mut relationship, mut ok) = (false, false, true);
        let mut unsupported = None;
        for target in &shape.targets {
            let at = target.location;
            match &target.value {
                Target::Class(class) | Target::ImplicitClass(class) => {
                    node_targets = true;
                    match self.class_labels(class.as_ref(), Some(shape), at) {
                        Some(names) => {
                            if let Some(checker) = self.inputs.checker {
                                let class_name = graph.compact(class.as_str());
                                if let Some(diagnostic) =
                                    checker.check_target(&class_name, &names, at)
                                {
                                    self.out.diagnostics.push(diagnostic);
                                }
                            }
                            labels.extend(names);
                        }
                        None => ok = false,
                    }
                }
                Target::SubjectsOf(predicate) | Target::ObjectsOf(predicate) => {
                    node_targets = true;
                    let objects = matches!(target.value, Target::ObjectsOf(_));
                    match resolver.predicate(predicate.as_ref(), &[], objects, at) {
                        Ok(path) if objects => sets.push(FocusSet::ObjectsOf(path)),
                        Ok(path) => sets.push(FocusSet::SubjectsOf(path)),
                        Err(e) => {
                            self.errors.push(e.0);
                            ok = false;
                        }
                    }
                }
                Target::Node(_) => {
                    node_targets = true;
                    let reason =
                        "sh:targetNode is not supported: labeled property graph nodes have no IRIs";
                    if self.options.lenient {
                        unsupported = Some(reason.to_owned());
                    } else {
                        self.error(at, reason);
                        ok = false;
                    }
                }
                Target::Relationship(rel_type) => {
                    relationship = true;
                    sets.push(FocusSet::Relationships(rel_type.clone()));
                }
            }
        }
        if relationship && node_targets {
            if let Some(target) = shape.targets.first() {
                self.error(
                    target.location,
                    "s2c:targetRelationship cannot be combined with node targets",
                );
            }
            ok = false;
        }
        if !ok {
            return None;
        }
        let only_class_targets = sets.is_empty();
        if !labels.is_empty() {
            sets.insert(0, FocusSet::Labels(labels.iter().cloned().collect()));
        }
        Some(Focus {
            shape: shape.id.clone(),
            path_labels: if only_class_targets {
                labels.into_iter().collect()
            } else {
                Vec::new()
            },
            sets,
            relationship,
            unsupported,
        })
    }

    /// Labels of a class and, under the label policy, its subclasses.
    fn class_labels(
        &mut self,
        class: NamedNodeRef<'_>,
        shape: Option<&Shape>,
        at: SourceLocation,
    ) -> Option<Vec<String>> {
        let mut names = Vec::new();
        for expanded in self
            .inputs
            .hierarchy
            .expand(class, self.options.label_policy)
        {
            let declaring = if expanded.as_ref() == class {
                shape
            } else {
                None
            };
            match self
                .inputs
                .resolver
                .class_label(expanded.as_ref(), declaring, at)
            {
                Ok(label) => names.push(label.name),
                Err(e) => {
                    self.errors.push(e.0);
                    return None;
                }
            }
        }
        names.sort();
        names.dedup();
        Some(names)
    }

    fn top_level_path(&mut self, shape: &Shape, focus: &Focus) -> Option<(LpgPath, bool)> {
        let location = shape.path.as_ref()?.location;
        let path = match self.inputs.resolver.path(shape, &focus.path_labels) {
            Ok(path) => path,
            Err(e) => {
                self.errors.push(e.0);
                return None;
            }
        };
        if focus.relationship && !matches!(path, LpgPath::Property(_)) {
            self.error(
                location,
                "paths from a relationship focus must be a single property (no traversal)",
            );
            return None;
        }
        if let Some(checker) = self.inputs.checker {
            self.out
                .diagnostics
                .extend(checker.check_path(&path, &focus.path_labels, location));
        }
        Some(self.cap(path))
    }

    fn rules_for(
        &mut self,
        shape: &'a Shape,
        focus: &Focus,
        source: &ValueSource,
        prefix: Vec<String>,
        status: Option<RuleStatus>,
        capped: bool,
    ) {
        for constraint in &shape.constraints {
            self.next_var = 0;
            self.flags = RuleFlags {
                capped,
                unsupported: None,
            };
            let scope = Scope {
                shape,
                labels: &focus.path_labels,
                top_level: true,
                relationship: focus.relationship,
                focus_nodes: !focus.relationship,
            };
            for item in self.constraint(constraint, Var::FOCUS, source, &scope, &prefix) {
                let mut rule_status = match (&status, &self.flags.unsupported) {
                    (Some(status), _) => status.clone(),
                    (None, Some(reason)) => RuleStatus::Unsupported(reason.clone()),
                    (None, None) => item.status,
                };
                if rule_status == RuleStatus::Compiled && item.violation.holds() == Expr::Bool(true)
                {
                    rule_status = RuleStatus::GuaranteedBySchema;
                }
                let mut id = prefix.clone();
                id.push(item.component);
                let rule = Rule {
                    id: RuleId(id),
                    shape: focus.shape.clone(),
                    declared_by: shape.id.clone(),
                    location: constraint.location,
                    severity: shape.severity.clone(),
                    messages: shape.messages.clone(),
                    focus: focus.sets.clone(),
                    violation: item.violation,
                    details: item.details,
                    status: rule_status,
                    path_depth_cap: self.flags.capped.then_some(self.options.max_path_depth),
                };
                if let Err(message) = rule.validate() {
                    self.errors.push(format!("internal error: {message}"));
                }
                self.out.rules.push(rule);
            }
        }
    }

    fn constraint(
        &mut self,
        constraint: &Located<Constraint>,
        of: Var,
        source: &ValueSource,
        scope: &Scope<'_>,
        prefix: &[String],
    ) -> Vec<Item> {
        let at = constraint.location;
        let path = match source {
            ValueSource::Path(path) => Some(path),
            ValueSource::Focus => None,
        };
        let nodes = path.map_or(scope.focus_nodes, LpgPath::yields_nodes);
        let component = component_name(&constraint.value);
        if scope.relationship && (path.is_none() || !is_value_constraint(&constraint.value)) {
            self.error(
                at,
                format!(
                    "{component} is not allowed on a relationship focus; only value constraints on relationship properties apply"
                ),
            );
            return Vec::new();
        }

        match &constraint.value {
            Constraint::MinCount(bound) | Constraint::MaxCount(bound) => {
                let cmp = if matches!(constraint.value, Constraint::MinCount(_)) {
                    Cmp::Ge
                } else {
                    Cmp::Le
                };
                let var = self.fresh();
                let condition = Expr::count(of, source.clone(), var, None, cmp, *bound);
                vec![Item::per_focus(component, of, condition)]
            }
            Constraint::Datatype(datatype) => {
                let override_type = self.datatype_override(scope.shape);
                let check = match DatatypeCheck::for_datatype(
                    datatype.as_ref(),
                    override_type.as_deref(),
                ) {
                    Ok(check) => check,
                    Err(e) if datatype.as_str() == RDF_LANG_STRING => {
                        return self.rejected(at, component, of, &e.0)
                    }
                    Err(e) => {
                        self.error(at, e.0);
                        return Vec::new();
                    }
                };
                let schema = match (scope.top_level, self.inputs.checker, path) {
                    (true, Some(checker), Some(path)) => {
                        checker.datatype_status(path, scope.labels, &check)
                    }
                    _ => SchemaStatus::Check,
                };
                let status = self.status_from(schema, at);
                let var = self.fresh();
                let test = if nodes {
                    Expr::Bool(false)
                } else {
                    Expr::test(var, ValueTest::Datatype(check))
                };
                vec![Item::per_value(component, of, source, var, test).with_status(status)]
            }
            Constraint::NodeKind(kind) => {
                // Nodes behave as IRIs, property values as literals; blank nodes never occur.
                let holds = match kind {
                    NodeKind::Iri | NodeKind::BlankNodeOrIri => nodes,
                    NodeKind::BlankNode => false,
                    NodeKind::Literal | NodeKind::BlankNodeOrLiteral => !nodes,
                    NodeKind::IriOrLiteral => true,
                };
                let var = self.fresh();
                vec![Item::per_value(
                    component,
                    of,
                    source,
                    var,
                    Expr::Bool(holds),
                )]
            }
            Constraint::Class(class) => {
                let var = self.fresh();
                if !nodes {
                    return vec![Item::per_value(
                        component,
                        of,
                        source,
                        var,
                        Expr::Bool(false),
                    )];
                }
                let Some(labels) = self.class_labels(class.as_ref(), None, at) else {
                    return Vec::new();
                };
                let schema = match (scope.top_level, self.inputs.checker, path) {
                    (true, Some(checker), Some(path)) => {
                        checker.class_status(path, scope.labels, &labels)
                    }
                    _ => SchemaStatus::Check,
                };
                let status = self.status_from(schema, at);
                let test = Expr::test(var, ValueTest::HasLabel(labels));
                vec![Item::per_value(component, of, source, var, test).with_status(status)]
            }
            Constraint::MinInclusive(literal)
            | Constraint::MaxInclusive(literal)
            | Constraint::MinExclusive(literal)
            | Constraint::MaxExclusive(literal) => {
                let cmp = match constraint.value {
                    Constraint::MinInclusive(_) => Cmp::Ge,
                    Constraint::MaxInclusive(_) => Cmp::Le,
                    Constraint::MinExclusive(_) => Cmp::Gt,
                    _ => Cmp::Lt,
                };
                let bound = match Constant::from_literal(literal) {
                    Ok(bound) => bound,
                    Err(message) => {
                        self.error(at, message);
                        return Vec::new();
                    }
                };
                let var = self.fresh();
                let test = if nodes {
                    Expr::Bool(false)
                } else {
                    Expr::test(var, ValueTest::Compare { cmp, bound })
                };
                vec![Item::per_value(component, of, source, var, test)]
            }
            Constraint::MinLength(bound) | Constraint::MaxLength(bound) => {
                let cmp = if matches!(constraint.value, Constraint::MinLength(_)) {
                    Cmp::Ge
                } else {
                    Cmp::Le
                };
                let var = self.fresh();
                let test = if nodes {
                    Expr::Bool(false)
                } else {
                    Expr::test(var, ValueTest::Length { cmp, bound: *bound })
                };
                vec![Item::per_value(component, of, source, var, test)]
            }
            Constraint::Pattern { pattern, flags } => {
                let regex = match XsdRegex::parse(pattern, flags.as_deref()) {
                    Ok(regex) => regex,
                    Err(e) => {
                        self.error(at, e.0);
                        return Vec::new();
                    }
                };
                let var = self.fresh();
                let test = if nodes {
                    Expr::Bool(false)
                } else {
                    Expr::test(var, ValueTest::Matches(regex))
                };
                vec![Item::per_value(component, of, source, var, test)]
            }
            Constraint::In(terms) => {
                let var = self.fresh();
                let Some(membership) = self.membership(terms, nodes, scope.shape, at) else {
                    return Vec::new();
                };
                vec![Item::per_value(
                    component,
                    of,
                    source,
                    var,
                    membership.apply(var),
                )]
            }
            Constraint::HasValue(term) => {
                let var = self.fresh();
                let Some(membership) =
                    self.membership(std::slice::from_ref(term), nodes, scope.shape, at)
                else {
                    return Vec::new();
                };
                let condition = Expr::count(
                    of,
                    source.clone(),
                    var,
                    Some(membership.apply(var)),
                    Cmp::Ge,
                    1,
                );
                vec![Item::per_focus(component, of, condition)]
            }
            Constraint::Equals(predicate)
            | Constraint::Disjoint(predicate)
            | Constraint::LessThan(predicate)
            | Constraint::LessThanOrEquals(predicate) => {
                let relation = match constraint.value {
                    Constraint::Equals(_) => PairRelation::Equals,
                    Constraint::Disjoint(_) => PairRelation::Disjoint,
                    Constraint::LessThan(_) => PairRelation::LessThan,
                    _ => PairRelation::LessThanOrEquals,
                };
                let right = match self.inputs.resolver.predicate(
                    predicate.as_ref(),
                    scope.labels,
                    false,
                    at,
                ) {
                    Ok(path) => path,
                    Err(e) => {
                        self.errors.push(e.0);
                        return Vec::new();
                    }
                };
                let condition = Expr::Pair {
                    of,
                    left: source.clone(),
                    right,
                    relation,
                };
                vec![Item::per_focus(component, of, condition)]
            }
            Constraint::Closed { ignored_properties } => {
                if path.is_some() {
                    self.error(at, "sh:closed is only supported on node shapes");
                    return Vec::new();
                }
                let (allowed, allowed_relationships) =
                    self.closed_keys(scope.shape, scope.labels, ignored_properties, at);
                vec![Item::per_focus(
                    component,
                    of,
                    Expr::Closed {
                        of,
                        allowed,
                        allowed_relationships,
                    },
                )]
            }
            Constraint::Node(target) => {
                let var = self.fresh();
                let Some(holds) = self.conforms(target, var, nodes) else {
                    return Vec::new();
                };
                let details = self.details(target, var, &child(prefix, component), nodes);
                vec![Item::per_value(component, of, source, var, holds).with_details(details)]
            }
            Constraint::Not(target) => {
                let var = self.fresh();
                let Some(holds) = self.conforms(target, var, nodes) else {
                    return Vec::new();
                };
                vec![Item::per_value(
                    component,
                    of,
                    source,
                    var,
                    Expr::negate(holds),
                )]
            }
            Constraint::And(members) | Constraint::Or(members) | Constraint::Xone(members) => {
                let var = self.fresh();
                let mut operands = Vec::new();
                let mut details = Vec::new();
                for (index, member) in members.iter().enumerate() {
                    let Some(holds) = self.conforms(member, var, nodes) else {
                        return Vec::new();
                    };
                    operands.push(holds);
                    let branch = child(prefix, &format!("{component}[{index}]"));
                    details.extend(self.details(member, var, &branch, nodes));
                }
                let test = match constraint.value {
                    Constraint::And(_) => Expr::and(operands),
                    Constraint::Or(_) => Expr::or(operands),
                    _ => Expr::xone(operands),
                };
                vec![Item::per_value(component, of, source, var, test).with_details(details)]
            }
            Constraint::QualifiedValueShape {
                shape: target,
                min_count,
                max_count,
                disjoint,
            } => {
                let var = self.fresh();
                let Some(mut filter) = self.conforms(target, var, nodes) else {
                    return Vec::new();
                };
                if *disjoint {
                    for sibling in self.sibling_qualified_shapes(scope.shape, target) {
                        let Some(holds) = self.conforms(&sibling, var, nodes) else {
                            return Vec::new();
                        };
                        filter = Expr::and([filter, Expr::negate(holds)]);
                    }
                }
                let mut items = Vec::new();
                for (bound, cmp, name) in [
                    (min_count, Cmp::Ge, "sh:qualifiedMinCount"),
                    (max_count, Cmp::Le, "sh:qualifiedMaxCount"),
                ] {
                    if let Some(bound) = bound {
                        let condition =
                            Expr::count(of, source.clone(), var, Some(filter.clone()), cmp, *bound);
                        items.push(Item::per_focus(name, of, condition));
                    }
                }
                items
            }
            Constraint::LanguageIn(_) => self.rejected(
                at,
                component,
                of,
                "sh:languageIn is not supported: labeled property graphs have no language tags",
            ),
            Constraint::UniqueLang(_) => self.rejected(
                at,
                component,
                of,
                "sh:uniqueLang is not supported: labeled property graphs have no language tags",
            ),
            Constraint::Sparql(_) => self.rejected(
                at,
                component,
                of,
                "SHACL-SPARQL constraints (sh:sparql) are not supported",
            ),
        }
    }

    /// Inlines a referenced shape as a condition on the value bound to `var`.
    fn conforms(&mut self, id: &ShapeId, var: Var, nodes: bool) -> Option<Expr> {
        let shapes = self.inputs.shapes;
        let shape = shapes.get(id)?;
        if shape.deactivated {
            return Some(Expr::Bool(true));
        }
        if self.recursive.contains(id) {
            self.flags.unsupported = Some("recursive shape reference".into());
            return Some(Expr::Bool(true));
        }
        let source = self.nested_source(shape, nodes)?;
        let scope = Scope {
            shape,
            labels: &[],
            top_level: false,
            relationship: false,
            focus_nodes: nodes,
        };
        let mut parts = Vec::new();
        for constraint in &shape.constraints {
            for item in self.constraint(constraint, var, &source, &scope, &[]) {
                if let RuleStatus::Unsupported(reason) = &item.status {
                    self.flags.unsupported = Some(reason.clone());
                }
                parts.push(item.violation.holds());
            }
        }
        for property in &shape.properties {
            parts.push(self.conforms(&property.value, var, nodes)?);
        }
        Some(Expr::and(parts))
    }

    fn nested_source(&mut self, shape: &Shape, nodes: bool) -> Option<ValueSource> {
        let Some(path) = &shape.path else {
            return Some(ValueSource::Focus);
        };
        if !nodes {
            self.error(
                path.location,
                "a property shape cannot be evaluated on property values, which have no properties",
            );
            return None;
        }
        match self.inputs.resolver.path(shape, &[]) {
            Ok(resolved) => {
                let (resolved, capped) = self.cap(resolved);
                self.flags.capped |= capped;
                Some(ValueSource::Path(resolved))
            }
            Err(e) => {
                self.errors.push(e.0);
                None
            }
        }
    }

    /// Inner rules reported in `details`: a named shape keeps its own rule ids, a
    /// blank shape extends `prefix`.
    fn details(&mut self, id: &ShapeId, var: Var, prefix: &[String], nodes: bool) -> Vec<Detail> {
        let shapes = self.inputs.shapes;
        let Some(shape) = shapes.get(id) else {
            return Vec::new();
        };
        if shape.deactivated || self.recursive.contains(id) {
            return Vec::new();
        }
        let base = match id {
            NamedOrBlankNode::NamedNode(iri) => vec![self.inputs.graph.compact(iri.as_str())],
            NamedOrBlankNode::BlankNode(_) => prefix.to_vec(),
        };
        let mut out = Vec::new();
        self.detail_constraints(shape, var, &base, nodes, &mut out);
        for property in &shape.properties {
            if let Some(property_shape) = shapes.get(&property.value) {
                if !property_shape.deactivated {
                    self.detail_constraints(property_shape, var, &base, nodes, &mut out);
                }
            }
        }
        out
    }

    fn detail_constraints(
        &mut self,
        shape: &Shape,
        var: Var,
        base: &[String],
        nodes: bool,
        out: &mut Vec<Detail>,
    ) {
        let mut prefix = base.to_vec();
        if let Some(path) = &shape.path {
            prefix.push(self.path_display(&path.value));
        }
        let Some(source) = self.nested_source(shape, nodes) else {
            return;
        };
        let scope = Scope {
            shape,
            labels: &[],
            top_level: false,
            relationship: false,
            focus_nodes: nodes,
        };
        for constraint in &shape.constraints {
            for item in self.constraint(constraint, var, &source, &scope, &prefix) {
                let mut id = prefix.clone();
                id.push(item.component);
                out.push(Detail {
                    rule: RuleId(id),
                    holds: item.violation.holds(),
                });
            }
        }
    }

    /// Qualified value shapes of the other property shapes of the same parents.
    fn sibling_qualified_shapes(&self, property_shape: &Shape, target: &ShapeId) -> Vec<ShapeId> {
        let shapes = self.inputs.shapes;
        let mut siblings = Vec::new();
        let parents = shapes.iter().filter(|shape| {
            shape
                .properties
                .iter()
                .any(|p| p.value == property_shape.id)
        });
        for parent in parents {
            for property in &parent.properties {
                if property.value == property_shape.id {
                    continue;
                }
                let Some(sibling) = shapes.get(&property.value) else {
                    continue;
                };
                for constraint in &sibling.constraints {
                    if let Constraint::QualifiedValueShape { shape, .. } = &constraint.value {
                        if shape != target && !siblings.contains(shape) {
                            siblings.push(shape.clone());
                        }
                    }
                }
            }
        }
        siblings
    }

    fn membership(
        &mut self,
        terms: &[Term],
        nodes: bool,
        shape: &Shape,
        at: SourceLocation,
    ) -> Option<Membership> {
        let full_iris = shape
            .annotations
            .iri_as_string
            .as_ref()
            .is_some_and(|a| a.value == IriAsString::Full);
        let mut constants = Vec::new();
        for term in terms {
            match term {
                Term::NamedNode(iri) => constants.push(Constant::String(if full_iris {
                    iri.as_str().to_owned()
                } else {
                    local_name(iri.as_str()).to_owned()
                })),
                // Literals never equal node values.
                Term::Literal(_) if nodes => {}
                Term::Literal(literal) => match Constant::from_literal(literal) {
                    Ok(constant) => constants.push(constant),
                    Err(message) => {
                        self.error(at, message);
                        return None;
                    }
                },
                other => {
                    self.error(at, format!("{other} cannot be used as a constant"));
                    return None;
                }
            }
        }
        if !nodes {
            return Some(Membership::Test(ValueTest::In(constants)));
        }
        if constants.is_empty() {
            return Some(Membership::Never);
        }
        let Some(key) = self.options.node_key.clone() else {
            self.error(
                at,
                "IRI constants compared with node values need a node key (--node-key)",
            );
            return None;
        };
        Some(Membership::Test(ValueTest::NodeKeyIn {
            key,
            values: constants,
        }))
    }

    fn closed_keys(
        &mut self,
        shape: &Shape,
        labels: &[String],
        ignored: &[NamedNode],
        at: SourceLocation,
    ) -> (Vec<String>, Vec<String>) {
        let shapes = self.inputs.shapes;
        let resolver = self.inputs.resolver;
        let mut keys = BTreeSet::new();
        let mut relationships = BTreeSet::new();
        for property in &shape.properties {
            if let Some(property_shape) = shapes.get(&property.value) {
                if let Ok(path) = resolver.path(property_shape, labels) {
                    allow_closed(path, &mut keys, &mut relationships);
                }
            }
        }
        for iri in ignored {
            match resolver.predicate(iri.as_ref(), labels, false, at) {
                Ok(path) => allow_closed(path, &mut keys, &mut relationships),
                Err(e) => self.errors.push(e.0),
            }
            // An ignored predicate may be stored either way, so its relationship form is allowed too.
            if let Ok(path) = resolver.predicate(iri.as_ref(), labels, true, at) {
                allow_closed(path, &mut keys, &mut relationships);
            }
        }
        // The identifying key comes from the LPG mapping, not from the shape's data.
        if let Some(key) = shape
            .annotations
            .key
            .as_ref()
            .map(|key| key.value.clone())
            .or_else(|| self.options.node_key.clone())
        {
            keys.insert(key);
        }
        (
            keys.into_iter().collect(),
            relationships.into_iter().collect(),
        )
    }

    fn datatype_override(&self, shape: &Shape) -> Option<String> {
        if let Some(datatype) = &shape.annotations.datatype {
            return Some(datatype.value.clone());
        }
        let Some(Located {
            value: Path::Predicate(predicate),
            ..
        }) = &shape.path
        else {
            return None;
        };
        self.inputs
            .shapes
            .iri_annotations(predicate.as_ref())
            .and_then(|annotations| annotations.datatype.as_ref())
            .map(|datatype| datatype.value.clone())
    }

    fn rejected(
        &mut self,
        at: SourceLocation,
        component: &str,
        of: Var,
        reason: &str,
    ) -> Vec<Item> {
        if self.options.lenient {
            vec![Item::per_focus(component, of, Expr::Bool(true))
                .with_status(RuleStatus::Unsupported(reason.to_owned()))]
        } else {
            self.error(at, reason);
            Vec::new()
        }
    }

    fn status_from(&mut self, schema: SchemaStatus, at: SourceLocation) -> RuleStatus {
        match schema {
            SchemaStatus::Check => RuleStatus::Compiled,
            SchemaStatus::Guaranteed => RuleStatus::GuaranteedBySchema,
            SchemaStatus::Contradicted(message) => {
                self.out.diagnostics.push(StaticDiagnostic {
                    code: SCHEMA_MISMATCH,
                    location: at,
                    message,
                });
                RuleStatus::SchemaMismatch
            }
        }
    }

    fn cap(&self, path: LpgPath) -> (LpgPath, bool) {
        let mut capped = false;
        let path = cap_repeats(path, self.options.max_path_depth, &mut capped);
        (path, capped)
    }

    fn shape_name(&self, shape: &Shape) -> String {
        match &shape.id {
            NamedOrBlankNode::NamedNode(iri) => self.inputs.graph.compact(iri.as_str()),
            NamedOrBlankNode::BlankNode(_) => match shape.location {
                Some(location) => {
                    let input = self.inputs.graph.inputs[location.file].name();
                    let file = std::path::Path::new(&input)
                        .file_name()
                        .map_or_else(|| input.clone(), |f| f.to_string_lossy().into_owned());
                    format!("[]@{file}:{}", location.line)
                }
                None => "[]".into(),
            },
        }
    }

    /// SPARQL-like path syntax with compacted IRIs, e.g. `ex:a/^ex:b`, `ex:knows+`.
    fn path_display(&self, path: &Path) -> String {
        let join = |items: &[Path], separator: &str| {
            items
                .iter()
                .map(|item| self.path_atom(item))
                .collect::<Vec<_>>()
                .join(separator)
        };
        match path {
            Path::Predicate(predicate) => self.inputs.graph.compact(predicate.as_str()),
            Path::Inverse(inner) => format!("^{}", self.path_atom(inner)),
            Path::Sequence(items) => join(items, "/"),
            Path::Alternative(items) => join(items, "|"),
            Path::ZeroOrMore(inner) => format!("{}*", self.path_atom(inner)),
            Path::OneOrMore(inner) => format!("{}+", self.path_atom(inner)),
            Path::ZeroOrOne(inner) => format!("{}?", self.path_atom(inner)),
        }
    }

    fn path_atom(&self, path: &Path) -> String {
        match path {
            Path::Sequence(_) | Path::Alternative(_) => format!("({})", self.path_display(path)),
            _ => self.path_display(path),
        }
    }

    fn fresh(&mut self) -> Var {
        self.next_var += 1;
        Var(self.next_var)
    }

    fn error(&mut self, at: SourceLocation, message: impl Into<String>) {
        self.errors.push(format!(
            "{}: {}",
            self.inputs.graph.display_location(at),
            message.into()
        ));
    }
}

fn cap_repeats(path: LpgPath, depth: u32, capped: &mut bool) -> LpgPath {
    match path {
        LpgPath::Repeat { path, min, max } => {
            let max = max.or_else(|| {
                *capped = true;
                Some(depth)
            });
            LpgPath::Repeat {
                path: Box::new(cap_repeats(*path, depth, capped)),
                min,
                max,
            }
        }
        LpgPath::Sequence(steps) => LpgPath::Sequence(
            steps
                .into_iter()
                .map(|step| cap_repeats(step, depth, capped))
                .collect(),
        ),
        LpgPath::Alternative(options) => LpgPath::Alternative(
            options
                .into_iter()
                .map(|option| cap_repeats(option, depth, capped))
                .collect(),
        ),
        other => other,
    }
}

/// Simple predicate paths allowed by `sh:closed`: property keys and outgoing relationship types.
// @lat: [[semantics#Violations and Conforms#Lowering Rules]]
fn allow_closed(path: LpgPath, keys: &mut BTreeSet<String>, relationships: &mut BTreeSet<String>) {
    match path {
        LpgPath::Property(key) => {
            keys.insert(key.name);
        }
        LpgPath::Relationship {
            rel_type,
            direction: crate::ast::Direction::Out,
        } => {
            relationships.insert(rel_type.name);
        }
        _ => {}
    }
}

fn child(prefix: &[String], segment: &str) -> Vec<String> {
    let mut id = prefix.to_vec();
    id.push(segment.to_owned());
    id
}

fn component_name(constraint: &Constraint) -> &'static str {
    match constraint {
        Constraint::MinCount(_) => "sh:minCount",
        Constraint::MaxCount(_) => "sh:maxCount",
        Constraint::Datatype(_) => "sh:datatype",
        Constraint::NodeKind(_) => "sh:nodeKind",
        Constraint::Class(_) => "sh:class",
        Constraint::MinInclusive(_) => "sh:minInclusive",
        Constraint::MaxInclusive(_) => "sh:maxInclusive",
        Constraint::MinExclusive(_) => "sh:minExclusive",
        Constraint::MaxExclusive(_) => "sh:maxExclusive",
        Constraint::MinLength(_) => "sh:minLength",
        Constraint::MaxLength(_) => "sh:maxLength",
        Constraint::Pattern { .. } => "sh:pattern",
        Constraint::In(_) => "sh:in",
        Constraint::HasValue(_) => "sh:hasValue",
        Constraint::Equals(_) => "sh:equals",
        Constraint::Disjoint(_) => "sh:disjoint",
        Constraint::LessThan(_) => "sh:lessThan",
        Constraint::LessThanOrEquals(_) => "sh:lessThanOrEquals",
        Constraint::Closed { .. } => "sh:closed",
        Constraint::Node(_) => "sh:node",
        Constraint::Not(_) => "sh:not",
        Constraint::And(_) => "sh:and",
        Constraint::Or(_) => "sh:or",
        Constraint::Xone(_) => "sh:xone",
        Constraint::QualifiedValueShape { .. } => "sh:qualifiedValueShape",
        Constraint::LanguageIn(_) => "sh:languageIn",
        Constraint::UniqueLang(_) => "sh:uniqueLang",
        Constraint::Sparql(_) => "sh:sparql",
    }
}

/// Constraints that only test the values of a path, allowed on relationship properties.
fn is_value_constraint(constraint: &Constraint) -> bool {
    matches!(
        constraint,
        Constraint::MinCount(_)
            | Constraint::MaxCount(_)
            | Constraint::Datatype(_)
            | Constraint::NodeKind(_)
            | Constraint::MinInclusive(_)
            | Constraint::MaxInclusive(_)
            | Constraint::MinExclusive(_)
            | Constraint::MaxExclusive(_)
            | Constraint::MinLength(_)
            | Constraint::MaxLength(_)
            | Constraint::Pattern { .. }
            | Constraint::In(_)
            | Constraint::HasValue(_)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mapping::{Evidence, ResolveOptions, Resolved};
    use crate::schema::SchemaSnapshot;

    /// Five prefix lines, so the first body line is line 6 of `shapes.ttl`.
    const PREFIXES: &str = "@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix ex: <http://example.org/> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
@prefix s2c: <https://w3id.org/shacl2cypher#> .
";

    struct Setup {
        graph: ShapesGraph,
        shapes: Shapes,
        hierarchy: ClassHierarchy,
        schema: Option<SchemaSnapshot>,
    }

    fn setup(body: &str, schema: Option<&str>) -> Setup {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shapes.ttl");
        std::fs::write(&path, format!("{PREFIXES}{body}")).unwrap();
        let graph = ShapesGraph::load(&[path]).unwrap();
        let shapes = Shapes::from_graph(&graph).unwrap();
        let hierarchy = ClassHierarchy::build(&[&graph]).unwrap();
        let schema = schema.map(|json| SchemaSnapshot::from_json(json).unwrap());
        Setup {
            graph,
            shapes,
            hierarchy,
            schema,
        }
    }

    fn run(setup: &Setup, options: &LowerOptions, enforced: bool) -> Result<Lowered, LowerErrors> {
        let resolver = Resolver::new(
            &setup.graph,
            &setup.shapes,
            setup.schema.as_ref(),
            ResolveOptions::default(),
        );
        let checker = setup
            .schema
            .as_ref()
            .map(|schema| SchemaChecker::new(schema, enforced));
        let inputs = Inputs {
            graph: &setup.graph,
            shapes: &setup.shapes,
            resolver: &resolver,
            hierarchy: &setup.hierarchy,
            checker: checker.as_ref(),
        };
        let lowered = lower(inputs, options)?;
        for rule in &lowered.rules {
            rule.validate().unwrap();
        }
        Ok(lowered)
    }

    fn lower_body(body: &str) -> Lowered {
        run(&setup(body, None), &LowerOptions::default(), false).unwrap()
    }

    fn lower_error(body: &str, options: &LowerOptions) -> String {
        run(&setup(body, None), options, false)
            .unwrap_err()
            .to_string()
    }

    fn lenient() -> LowerOptions {
        LowerOptions {
            lenient: true,
            ..LowerOptions::default()
        }
    }

    fn rule<'l>(lowered: &'l Lowered, id: &str) -> &'l Rule {
        lowered
            .rules
            .iter()
            .find(|rule| rule.id.to_string() == id)
            .unwrap_or_else(|| {
                let ids: Vec<String> = lowered.rules.iter().map(|r| r.id.to_string()).collect();
                panic!("no rule {id}; rules: {ids:?}")
            })
    }

    fn resolved(name: &str) -> Resolved {
        Resolved {
            name: name.into(),
            evidence: Evidence::Convention,
        }
    }

    fn property(name: &str) -> ValueSource {
        ValueSource::Path(LpgPath::Property(resolved(name)))
    }

    fn value_test(rule: &Rule) -> &Expr {
        match &rule.violation {
            Violation::PerValue { test, .. } => test,
            other => panic!("expected a per-value violation, got {other:?}"),
        }
    }

    fn condition(rule: &Rule) -> &Expr {
        match &rule.violation {
            Violation::PerFocus { condition, .. } => condition,
            other => panic!("expected a per-focus violation, got {other:?}"),
        }
    }

    #[test]
    fn lowers_cardinality_to_per_focus_counts() {
        let lowered = lower_body(
            "ex:PersonShape sh:targetClass ex:Person ;
    sh:property [ sh:path ex:name ; sh:minCount 1 ] .
",
        );
        assert_eq!(lowered.rules.len(), 1);
        let min_count = rule(&lowered, "ex:PersonShape/ex:name/sh:minCount");
        assert_eq!(
            min_count.focus,
            vec![FocusSet::Labels(vec!["Person".into()])]
        );
        assert_eq!(
            min_count.violation,
            Violation::PerFocus {
                of: Var::FOCUS,
                condition: Expr::Count {
                    of: Var::FOCUS,
                    source: property("name"),
                    var: Var(1),
                    filter: None,
                    cmp: Cmp::Ge,
                    bound: 1,
                },
            }
        );
        assert_eq!(min_count.status, RuleStatus::Compiled);
        assert_eq!(min_count.location.line, 7);
    }

    #[test]
    fn lowers_value_constraints_to_per_value_tests() {
        let lowered = lower_body(
            "ex:PersonShape sh:targetClass ex:Person ;
    sh:property [ sh:path ex:name ; sh:datatype xsd:string ; sh:maxLength 10 ;
                  sh:pattern \"^A\" ; sh:in ( \"Ann\" \"Bob\" ) ] ,
                [ sh:path ex:age ; sh:minInclusive 0 ] .
",
        );
        assert_eq!(
            rule(&lowered, "ex:PersonShape/ex:age/sh:minInclusive").violation,
            Violation::PerValue {
                of: Var::FOCUS,
                source: property("age"),
                var: Var(1),
                test: Expr::test(
                    Var(1),
                    ValueTest::Compare {
                        cmp: Cmp::Ge,
                        bound: Constant::Integer(0)
                    }
                ),
            }
        );
        assert_eq!(
            value_test(rule(&lowered, "ex:PersonShape/ex:name/sh:in")),
            &Expr::test(
                Var(1),
                ValueTest::In(vec![
                    Constant::String("Ann".into()),
                    Constant::String("Bob".into())
                ])
            )
        );
        assert!(matches!(
            value_test(rule(&lowered, "ex:PersonShape/ex:name/sh:datatype")),
            Expr::Test {
                test: ValueTest::Datatype(_),
                ..
            }
        ));
        assert!(matches!(
            value_test(rule(&lowered, "ex:PersonShape/ex:name/sh:maxLength")),
            Expr::Test {
                test: ValueTest::Length {
                    cmp: Cmp::Le,
                    bound: 10
                },
                ..
            }
        ));
        assert!(matches!(
            value_test(rule(&lowered, "ex:PersonShape/ex:name/sh:pattern")),
            Expr::Test {
                test: ValueTest::Matches(_),
                ..
            }
        ));
    }

    #[test]
    fn class_targets_expand_by_label_policy() {
        let body = "ex:Employee rdfs:subClassOf ex:Person .
ex:PersonShape sh:targetClass ex:Person ;
    sh:property [ sh:path ex:name ; sh:minCount 1 ] .
";
        let setup = setup(body, None);
        let explicit = run(&setup, &LowerOptions::default(), false).unwrap();
        assert_eq!(
            explicit.rules[0].focus,
            vec![FocusSet::Labels(vec!["Employee".into(), "Person".into()])]
        );
        let inherited = LowerOptions {
            label_policy: LabelPolicy::Inherited,
            ..LowerOptions::default()
        };
        assert_eq!(
            run(&setup, &inherited, false).unwrap().rules[0].focus,
            vec![FocusSet::Labels(vec!["Person".into()])]
        );
    }

    #[test]
    fn decides_value_kinds_statically() {
        let lowered = lower_body(
            "ex:PersonShape sh:targetClass ex:Person ;
    sh:property [ sh:path ex:worksFor ; sh:class ex:Company ; sh:datatype xsd:string ;
                  sh:nodeKind sh:IRI ] .
",
        );
        let datatype = rule(&lowered, "ex:PersonShape/ex:worksFor/sh:datatype");
        assert_eq!(value_test(datatype), &Expr::Bool(false));
        assert_eq!(datatype.status, RuleStatus::Compiled);
        assert_eq!(
            value_test(rule(&lowered, "ex:PersonShape/ex:worksFor/sh:class")),
            &Expr::test(Var(1), ValueTest::HasLabel(vec!["Company".into()]))
        );
        assert_eq!(
            rule(&lowered, "ex:PersonShape/ex:worksFor/sh:nodeKind").status,
            RuleStatus::GuaranteedBySchema
        );
    }

    #[test]
    fn lowers_closed_shapes_pairs_and_has_value() {
        let lowered = lower_body(
            "ex:EventShape sh:targetClass ex:Event ;
    sh:closed true ;
    sh:ignoredProperties ( ex:id ) ;
    sh:property [ sh:path ex:start ; sh:lessThan ex:end ] ,
                [ sh:path ex:end ; sh:minCount 1 ] ,
                [ sh:path ex:status ; sh:hasValue \"open\" ] .
",
        );
        assert_eq!(
            condition(rule(&lowered, "ex:EventShape/sh:closed")),
            &Expr::Closed {
                of: Var::FOCUS,
                allowed: vec!["end".into(), "id".into(), "start".into(), "status".into()],
                allowed_relationships: vec!["ID".into()],
            }
        );
        assert_eq!(
            condition(rule(&lowered, "ex:EventShape/ex:start/sh:lessThan")),
            &Expr::Pair {
                of: Var::FOCUS,
                left: property("start"),
                right: LpgPath::Property(resolved("end")),
                relation: PairRelation::LessThan,
            }
        );
        assert_eq!(
            condition(rule(&lowered, "ex:EventShape/ex:status/sh:hasValue")),
            &Expr::count(
                Var::FOCUS,
                property("status"),
                Var(1),
                Some(Expr::test(
                    Var(1),
                    ValueTest::In(vec![Constant::String("open".into())])
                )),
                Cmp::Ge,
                1
            )
        );
    }

    #[test]
    fn iri_constants_use_local_names_and_node_keys() {
        let body = "ex:TaskShape sh:targetClass ex:Task ;
    sh:property [ sh:path ex:state ; sh:class ex:State ; sh:in ( ex:Open ex:Closed ) ] ,
                [ sh:path ex:status ; sh:in ( ex:Active ) ] .
";
        let message = lower_error(body, &LowerOptions::default());
        assert!(message.contains("--node-key"), "{message}");

        let options = LowerOptions {
            node_key: Some("id".into()),
            ..LowerOptions::default()
        };
        let lowered = run(&setup(body, None), &options, false).unwrap();
        assert_eq!(
            value_test(rule(&lowered, "ex:TaskShape/ex:state/sh:in")),
            &Expr::test(
                Var(1),
                ValueTest::NodeKeyIn {
                    key: "id".into(),
                    values: vec![
                        Constant::String("Open".into()),
                        Constant::String("Closed".into())
                    ],
                }
            )
        );
        assert_eq!(
            value_test(rule(&lowered, "ex:TaskShape/ex:status/sh:in")),
            &Expr::test(
                Var(1),
                ValueTest::In(vec![Constant::String("Active".into())])
            )
        );
    }

    #[test]
    fn inlines_node_shapes_with_named_details() {
        let lowered = lower_body(
            "ex:PersonShape sh:targetClass ex:Person ;
    sh:property [ sh:path ex:address ; sh:node ex:AddressShape ] .
ex:AddressShape sh:property [ sh:path ex:zip ; sh:minCount 1 ] .
",
        );
        assert_eq!(lowered.rules.len(), 1);
        let node = rule(&lowered, "ex:PersonShape/ex:address/sh:node");
        assert_eq!(
            node.violation,
            Violation::PerValue {
                of: Var::FOCUS,
                source: ValueSource::Path(LpgPath::Relationship {
                    rel_type: resolved("ADDRESS"),
                    direction: crate::ast::Direction::Out,
                }),
                var: Var(1),
                test: Expr::count(Var(1), property("zip"), Var(2), None, Cmp::Ge, 1),
            }
        );
        let details: Vec<String> = node.details.iter().map(|d| d.rule.to_string()).collect();
        assert_eq!(details, vec!["ex:AddressShape/ex:zip/sh:minCount"]);
    }

    #[test]
    fn logical_branches_extend_detail_ids() {
        let lowered = lower_body(
            "ex:ContactShape sh:targetClass ex:Person ;
    sh:or ( [ sh:path ex:email ; sh:minCount 1 ] [ sh:path ex:phone ; sh:minCount 1 ] ) .
",
        );
        let or = rule(&lowered, "ex:ContactShape/sh:or");
        assert!(matches!(
            &or.violation,
            Violation::PerValue {
                source: ValueSource::Focus,
                test: Expr::Or(operands),
                ..
            } if operands.len() == 2
        ));
        let details: Vec<String> = or.details.iter().map(|d| d.rule.to_string()).collect();
        assert_eq!(
            details,
            vec![
                "ex:ContactShape/sh:or[0]/ex:email/sh:minCount",
                "ex:ContactShape/sh:or[1]/ex:phone/sh:minCount",
            ]
        );
    }

    #[test]
    fn qualified_counts_exclude_disjoint_siblings() {
        let lowered = lower_body(
            "ex:PersonShape sh:targetClass ex:Person ;
    sh:property [ sh:path ex:parent ; s2c:relationship \"PARENT\" ;
                  sh:qualifiedValueShape ex:MotherShape ; sh:qualifiedMinCount 1 ;
                  sh:qualifiedValueShapesDisjoint true ] ,
                [ sh:path ex:parent ; s2c:relationship \"PARENT\" ;
                  sh:qualifiedValueShape ex:FatherShape ; sh:qualifiedMaxCount 1 ;
                  sh:qualifiedValueShapesDisjoint true ] .
ex:MotherShape sh:property [ sh:path ex:gender ; sh:hasValue \"female\" ] .
ex:FatherShape sh:property [ sh:path ex:gender ; sh:hasValue \"male\" ] .
",
        );
        let Expr::Count {
            filter: Some(filter),
            cmp: Cmp::Ge,
            bound: 1,
            ..
        } = condition(rule(
            &lowered,
            "ex:PersonShape/ex:parent/sh:qualifiedMinCount",
        ))
        else {
            panic!("expected a filtered count");
        };
        assert!(
            matches!(filter.as_ref(), Expr::And(parts) if parts.len() == 2 && matches!(parts[1], Expr::Not(_)))
        );
        assert!(matches!(
            condition(rule(
                &lowered,
                "ex:PersonShape/ex:parent/sh:qualifiedMaxCount"
            )),
            Expr::Count {
                cmp: Cmp::Le,
                bound: 1,
                ..
            }
        ));
    }

    #[test]
    fn caps_unbounded_paths_and_checks_the_dialect_limit() {
        let body = "ex:PersonShape sh:targetClass ex:Person ;
    sh:property [ sh:path [ sh:oneOrMorePath ex:knows ] ; sh:class ex:Person ] .
";
        let options = LowerOptions {
            max_path_depth: 5,
            ..LowerOptions::default()
        };
        let lowered = run(&setup(body, None), &options, false).unwrap();
        let class = rule(&lowered, "ex:PersonShape/ex:knows+/sh:class");
        assert_eq!(class.path_depth_cap, Some(5));
        assert!(matches!(
            &class.violation,
            Violation::PerValue {
                source: ValueSource::Path(LpgPath::Repeat { max: Some(5), .. }),
                ..
            }
        ));

        let too_deep = LowerOptions {
            max_path_depth: 40,
            dialect_max_path_depth: Some(30),
            ..LowerOptions::default()
        };
        assert!(lower_error(body, &too_deep).contains("exceeds the dialect's maximum"));
    }

    #[test]
    fn relationship_focus_allows_only_property_value_constraints() {
        let lowered = lower_body(
            "ex:KnowsShape s2c:targetRelationship \"KNOWS\" ;
    sh:property [ sh:path ex:since ; sh:minCount 1 ; sh:datatype xsd:date ] .
",
        );
        assert_eq!(lowered.rules.len(), 2);
        assert!(lowered
            .rules
            .iter()
            .all(|r| r.focus == vec![FocusSet::Relationships("KNOWS".into())]));

        let message = lower_error(
            "ex:KnowsShape s2c:targetRelationship \"KNOWS\" ;
    sh:property [ sh:path ex:since ; sh:class ex:Date ] .
",
            &LowerOptions::default(),
        );
        assert!(message.contains("relationship focus"), "{message}");
    }

    #[test]
    fn rejected_features_are_errors_or_unsupported_rules() {
        let cases = [
            (
                "ex:S sh:targetClass ex:P ;\n    sh:sparql [ sh:select \"SELECT $this WHERE {}\" ] .\n",
                "SHACL-SPARQL",
                "ex:S/sh:sparql",
            ),
            (
                "ex:S sh:targetNode ex:alice ;\n    sh:property [ sh:path ex:name ; sh:minCount 1 ] .\n",
                "sh:targetNode",
                "ex:S/ex:name/sh:minCount",
            ),
            (
                "ex:S sh:targetClass ex:P ;\n    sh:property [ sh:path ex:label ; sh:languageIn ( \"en\" ) ] .\n",
                "sh:languageIn",
                "ex:S/ex:label/sh:languageIn",
            ),
            (
                "ex:S sh:targetClass ex:P ;\n    sh:property [ sh:path ex:knows ; sh:node ex:S ] .\n",
                "recursive shape reference",
                "ex:S/ex:knows/sh:node",
            ),
        ];
        for (body, error, id) in cases {
            let message = lower_error(body, &LowerOptions::default());
            assert!(message.contains(error), "{message}");
            let lowered = run(&setup(body, None), &lenient(), false).unwrap();
            assert!(
                matches!(rule(&lowered, id).status, RuleStatus::Unsupported(_)),
                "{id}"
            );
        }
    }

    #[test]
    fn deactivated_shapes_produce_deactivated_rules() {
        let lowered = lower_body(
            "ex:S sh:targetClass ex:P ;
    sh:deactivated true ;
    sh:property [ sh:path ex:name ; sh:minCount 1 ] .
",
        );
        assert_eq!(
            rule(&lowered, "ex:S/ex:name/sh:minCount").status,
            RuleStatus::Deactivated
        );
    }

    #[test]
    fn enforced_schema_decides_constraints_and_reports_mismatches() {
        let schema = r#"{"nodeTypes": [{"name": "Person", "properties": [{"name": "name", "type": "STRING"}]}]}"#;
        let setup = setup(
            "ex:PersonShape sh:targetClass ex:Person ;
    sh:property [ sh:path ex:name ; sh:datatype xsd:string ] ,
                [ sh:path ex:name ; sh:datatype xsd:integer ] .
ex:GhostShape sh:targetClass ex:Ghost ;
    sh:property [ sh:path ex:name ; sh:minCount 1 ] .
",
            Some(schema),
        );
        let lowered = run(&setup, &LowerOptions::default(), true).unwrap();
        let statuses: Vec<&RuleStatus> = lowered
            .rules
            .iter()
            .filter(|r| r.id.to_string() == "ex:PersonShape/ex:name/sh:datatype")
            .map(|r| &r.status)
            .collect();
        assert!(statuses.contains(&&RuleStatus::GuaranteedBySchema));
        assert!(statuses.contains(&&RuleStatus::SchemaMismatch));
        let messages: Vec<&str> = lowered
            .diagnostics
            .iter()
            .map(|d| d.message.as_str())
            .collect();
        assert!(
            messages
                .iter()
                .any(|m| m.contains("ex:Ghost maps to label `Ghost`")),
            "{messages:?}"
        );
        assert!(
            messages.iter().any(|m| m.contains("contradicts")),
            "{messages:?}"
        );
    }
}
