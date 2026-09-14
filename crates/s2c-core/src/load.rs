//! Input assembly: parses shape files into one union graph with source locations.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use oxrdf::{BlankNode, Graph, NamedNodeRef, NamedOrBlankNode, Term, TermRef, Triple};
use oxttl::ntriples::LowLevelNTriplesParser;
use oxttl::trig::LowLevelTriGParser;
use oxttl::turtle::LowLevelTurtleParser;
use oxttl::{NTriplesParser, TriGParser, TurtleParser, TurtleSyntaxError};
use sha2::{Digest, Sha256};

const OWL_IMPORTS: NamedNodeRef<'static> =
    NamedNodeRef::new_unchecked("http://www.w3.org/2002/07/owl#imports");

/// Index of an input in [`ShapesGraph::inputs`].
pub type FileId = usize;

/// Input and 1-based line on which the parser completed a triple.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SourceLocation {
    pub file: FileId,
    pub line: u32,
}

/// Where an input's bytes came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputSource {
    File(PathBuf),
    Remote(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Input {
    pub source: InputSource,
    /// Lower-case hex SHA-256 of the bytes that were parsed.
    pub sha256: String,
}

impl Input {
    /// File path or remote IRI, for messages.
    pub fn name(&self) -> String {
        match &self.source {
            InputSource::File(path) => path.display().to_string(),
            InputSource::Remote(iri) => iri.clone(),
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct LoadOptions {
    /// Follow `owl:imports` of `http(s)` IRIs through a [`RemoteFetcher`].
    pub allow_remote_imports: bool,
}

/// Retrieves remote imports; supplied by the caller so the core does no network I/O.
pub trait RemoteFetcher {
    fn fetch(&self, iri: &str) -> Result<Vec<u8>, String>;
}

#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error("{}: {source}", path.display())]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{input}:{line}:{column}: {message}")]
    Syntax {
        input: String,
        line: u64,
        column: u64,
        message: String,
    },
    #[error("{input}: unsupported shapes file format (expected .ttl, .nt or .trig)")]
    UnsupportedFormat { input: String },
    #[error("{location}: cannot import <{iri}>: {reason}")]
    Import {
        location: String,
        iri: String,
        reason: String,
    },
}

/// Union graph of all inputs, with the location of every triple.
#[derive(Debug, Default)]
pub struct ShapesGraph {
    pub graph: Graph,
    /// Given files in canonical sorted order, then imports in discovery order.
    pub inputs: Vec<Input>,
    /// Prefix declarations; when inputs disagree, the first input in order wins.
    pub prefixes: BTreeMap<String, String>,
    locations: HashMap<Triple, Vec<SourceLocation>>,
}

impl ShapesGraph {
    /// Loads local shape files and their local imports.
    pub fn load(paths: &[impl AsRef<Path>]) -> Result<Self, LoadError> {
        Self::load_with(paths, LoadOptions::default(), None)
    }

    /// Loads shape files into one graph, then follows `owl:imports` until no new
    /// input appears. Paths are canonicalized, de-duplicated and sorted, and imports
    /// are resolved in IRI order, so the result never depends on argument order.
    // @lat: [[architecture#Input Assembly]]
    pub fn load_with(
        paths: &[impl AsRef<Path>],
        options: LoadOptions,
        fetcher: Option<&dyn RemoteFetcher>,
    ) -> Result<Self, LoadError> {
        let mut files = paths
            .iter()
            .map(|p| {
                let path = p.as_ref();
                std::fs::canonicalize(path).map_err(|source| LoadError::Io {
                    path: path.to_path_buf(),
                    source,
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        files.sort();
        files.dedup();

        let mut shapes = ShapesGraph::default();
        for path in files {
            shapes.add_file(path)?;
        }
        shapes.resolve_imports(options, fetcher)?;
        Ok(shapes)
    }

    /// Every location a triple was read from, in load order.
    pub fn locations(&self, triple: &Triple) -> &[SourceLocation] {
        self.locations.get(triple).map_or(&[], Vec::as_slice)
    }

    /// `input:line` for messages.
    pub fn display_location(&self, location: SourceLocation) -> String {
        format!("{}:{}", self.inputs[location.file].name(), location.line)
    }

    /// `prefix:local` using the longest matching declared prefix, else `<iri>`.
    pub fn compact(&self, iri: &str) -> String {
        self.prefixes
            .iter()
            .filter(|(_, namespace)| !namespace.is_empty() && iri.starts_with(namespace.as_str()))
            .max_by_key(|(_, namespace)| namespace.len())
            .map(|(prefix, namespace)| format!("{prefix}:{}", &iri[namespace.len()..]))
            .unwrap_or_else(|| format!("<{iri}>"))
    }

    fn resolve_imports(
        &mut self,
        options: LoadOptions,
        fetcher: Option<&dyn RemoteFetcher>,
    ) -> Result<(), LoadError> {
        loop {
            let mut imports: BTreeMap<String, SourceLocation> = BTreeMap::new();
            for triple in self.graph.iter().filter(|t| t.predicate == OWL_IMPORTS) {
                let TermRef::NamedNode(iri) = triple.object else {
                    continue;
                };
                let first_seen = self.locations[&triple.into_owned()]
                    .iter()
                    .min()
                    .copied()
                    .expect("every loaded triple has a location");
                imports
                    .entry(iri.as_str().to_owned())
                    .and_modify(|seen| *seen = (*seen).min(first_seen))
                    .or_insert(first_seen);
            }

            let mut loaded_any = false;
            for (iri, location) in imports {
                let at = self.display_location(location);
                let error = |reason: String| LoadError::Import {
                    location: at.clone(),
                    iri: iri.clone(),
                    reason,
                };
                if let Some(path) = file_iri_to_path(&iri) {
                    let path = std::fs::canonicalize(&path).map_err(|e| error(e.to_string()))?;
                    if self.is_loaded(&InputSource::File(path.clone())) {
                        continue;
                    }
                    self.add_file(path)?;
                } else if iri.starts_with("http://") || iri.starts_with("https://") {
                    let source = InputSource::Remote(iri.clone());
                    if self.is_loaded(&source) {
                        continue;
                    }
                    if !options.allow_remote_imports {
                        return Err(error(
                            "remote imports are disabled (use --allow-remote-imports)".into(),
                        ));
                    }
                    let fetcher =
                        fetcher.ok_or_else(|| error("no remote fetcher is available".into()))?;
                    let bytes = fetcher.fetch(&iri).map_err(error)?;
                    self.add_input(source, bytes)?;
                } else {
                    return Err(error("unsupported IRI scheme".into()));
                }
                loaded_any = true;
            }
            if !loaded_any {
                return Ok(());
            }
        }
    }

    fn is_loaded(&self, source: &InputSource) -> bool {
        self.inputs.iter().any(|input| &input.source == source)
    }

    fn add_file(&mut self, path: PathBuf) -> Result<FileId, LoadError> {
        let bytes = std::fs::read(&path).map_err(|source| LoadError::Io {
            path: path.clone(),
            source,
        })?;
        self.add_input(InputSource::File(path), bytes)
    }

    fn add_input(&mut self, source: InputSource, bytes: Vec<u8>) -> Result<FileId, LoadError> {
        let file = self.inputs.len();
        let input = Input {
            source,
            sha256: Sha256::digest(&bytes)
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect(),
        };
        let (format, base) = match &input.source {
            InputSource::File(path) => (Format::of_file(path), file_base_iri(path)),
            InputSource::Remote(iri) => (Some(Format::of_remote(iri)), iri.clone()),
        };
        let result = match format {
            Some(Format::Turtle) => {
                let parser = TurtleParser::new();
                let parser = parser.clone().with_base_iri(&base).unwrap_or(parser);
                self.parse(file, &bytes, parser.low_level())
            }
            Some(Format::TriG) => {
                let parser = TriGParser::new();
                let parser = parser.clone().with_base_iri(&base).unwrap_or(parser);
                self.parse(file, &bytes, parser.low_level())
            }
            Some(Format::NTriples) => self.parse(file, &bytes, NTriplesParser::new().low_level()),
            None => {
                return Err(LoadError::UnsupportedFormat {
                    input: input.name(),
                })
            }
        };
        result.map_err(|e| {
            let start = e.location().start;
            LoadError::Syntax {
                input: input.name(),
                line: start.line + 1,
                column: start.column + 1,
                message: e.message().to_string(),
            }
        })?;
        self.inputs.push(input);
        Ok(file)
    }

    /// Feeds the input line by line: oxttl reports no positions for parsed triples,
    /// so each triple is attributed to the line on which the parser completed it.
    fn parse(
        &mut self,
        file: FileId,
        text: &[u8],
        mut parser: impl LineParser,
    ) -> Result<(), TurtleSyntaxError> {
        let mut blank_nodes = HashMap::new();
        let mut line = 0u32;
        for chunk in text.split_inclusive(|b| *b == b'\n') {
            line += 1;
            parser.extend(chunk);
            self.drain(&mut parser, file, line, &mut blank_nodes)?;
        }
        parser.end();
        self.drain(&mut parser, file, line.max(1), &mut blank_nodes)?;
        for (prefix, iri) in parser.prefixes() {
            self.prefixes.entry(prefix).or_insert(iri);
        }
        Ok(())
    }

    fn drain(
        &mut self,
        parser: &mut impl LineParser,
        file: FileId,
        line: u32,
        blank_nodes: &mut HashMap<String, BlankNode>,
    ) -> Result<(), TurtleSyntaxError> {
        while let Some(triple) = parser.next_triple() {
            let mut triple = triple?;
            if let NamedOrBlankNode::BlankNode(node) = &triple.subject {
                triple.subject = scoped(node, file, blank_nodes).into();
            }
            if let Term::BlankNode(node) = &triple.object {
                triple.object = scoped(node, file, blank_nodes).into();
            }
            self.graph.insert(&triple);
            self.locations
                .entry(triple)
                .or_default()
                .push(SourceLocation { file, line });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Format {
    Turtle,
    TriG,
    NTriples,
}

impl Format {
    fn of_extension(extension: &str) -> Option<Self> {
        match extension {
            "ttl" => Some(Format::Turtle),
            "trig" => Some(Format::TriG),
            "nt" => Some(Format::NTriples),
            _ => None,
        }
    }

    fn of_file(path: &Path) -> Option<Self> {
        Self::of_extension(path.extension()?.to_str()?)
    }

    /// Remote documents without a known extension are assumed to be Turtle.
    fn of_remote(iri: &str) -> Self {
        let path = iri.split(['?', '#']).next().unwrap_or(iri);
        let last_segment = path.rsplit('/').next().unwrap_or(path);
        last_segment
            .rsplit_once('.')
            .and_then(|(_, extension)| Self::of_extension(extension))
            .unwrap_or(Format::Turtle)
    }
}

/// Renames a blank node to a deterministic id unique to its input.
fn scoped(node: &BlankNode, file: FileId, renamed: &mut HashMap<String, BlankNode>) -> BlankNode {
    let next = renamed.len();
    renamed
        .entry(node.as_str().to_owned())
        .or_insert_with(|| BlankNode::new_unchecked(format!("f{file}b{next}")))
        .clone()
}

/// `file://` URL of a path, percent-encoding everything outside the unreserved set.
fn file_base_iri(path: &Path) -> String {
    let mut iri = String::from("file://");
    for byte in path.to_string_lossy().bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~/".contains(&byte) {
            iri.push(byte as char);
        } else {
            iri.push_str(&format!("%{byte:02X}"));
        }
    }
    iri
}

/// Inverse of [`file_base_iri`]; `None` for non-`file:` IRIs.
fn file_iri_to_path(iri: &str) -> Option<PathBuf> {
    let encoded = iri.strip_prefix("file://")?;
    let encoded = encoded.split(['?', '#']).next().unwrap_or(encoded);
    let bytes = encoded.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(byte) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                decoded.push(byte);
                i += 3;
                continue;
            }
        }
        decoded.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(decoded).ok().map(PathBuf::from)
}

/// Common interface over oxttl's push-based parsers.
trait LineParser {
    fn extend(&mut self, data: &[u8]);
    fn end(&mut self);
    fn next_triple(&mut self) -> Option<Result<Triple, TurtleSyntaxError>>;
    fn prefixes(&self) -> Vec<(String, String)>;
}

impl LineParser for LowLevelTurtleParser {
    fn extend(&mut self, data: &[u8]) {
        self.extend_from_slice(data);
    }
    fn end(&mut self) {
        LowLevelTurtleParser::end(self);
    }
    fn next_triple(&mut self) -> Option<Result<Triple, TurtleSyntaxError>> {
        self.parse_next()
    }
    fn prefixes(&self) -> Vec<(String, String)> {
        LowLevelTurtleParser::prefixes(self)
            .map(|(p, iri)| (p.to_owned(), iri.to_owned()))
            .collect()
    }
}

impl LineParser for LowLevelTriGParser {
    fn extend(&mut self, data: &[u8]) {
        self.extend_from_slice(data);
    }
    fn end(&mut self) {
        LowLevelTriGParser::end(self);
    }
    fn next_triple(&mut self) -> Option<Result<Triple, TurtleSyntaxError>> {
        // Named graphs are unioned into the shapes graph.
        Some(
            self.parse_next()?
                .map(|q| Triple::new(q.subject, q.predicate, q.object)),
        )
    }
    fn prefixes(&self) -> Vec<(String, String)> {
        LowLevelTriGParser::prefixes(self)
            .map(|(p, iri)| (p.to_owned(), iri.to_owned()))
            .collect()
    }
}

impl LineParser for LowLevelNTriplesParser {
    fn extend(&mut self, data: &[u8]) {
        self.extend_from_slice(data);
    }
    fn end(&mut self) {
        LowLevelNTriplesParser::end(self);
    }
    fn next_triple(&mut self) -> Option<Result<Triple, TurtleSyntaxError>> {
        self.parse_next()
    }
    fn prefixes(&self) -> Vec<(String, String)> {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxrdf::{Literal, NamedNode, TripleRef};

    const PREFIXES: &str =
        "@prefix sh: <http://www.w3.org/ns/shacl#> .\n@prefix ex: <http://example.org/> .\n";
    const IMPORT_PREFIXES: &str =
        "@prefix owl: <http://www.w3.org/2002/07/owl#> .\n@prefix ex: <http://example.org/> .\n";

    fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, body).unwrap();
        path
    }

    fn ex(local: &str) -> NamedNode {
        NamedNode::new_unchecked(format!("http://example.org/{local}"))
    }

    fn sh(local: &str) -> NamedNode {
        NamedNode::new_unchecked(format!("http://www.w3.org/ns/shacl#{local}"))
    }

    fn sorted_ntriples(graph: &Graph) -> Vec<String> {
        let mut lines: Vec<String> = graph.iter().map(|t| t.to_string()).collect();
        lines.sort();
        lines
    }

    fn hex_sha256(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    #[test]
    fn shape_split_across_files_forms_one_graph() {
        let dir = tempfile::tempdir().unwrap();
        let core = write(
            dir.path(),
            "core.ttl",
            &format!("{PREFIXES}ex:PersonShape a sh:NodeShape ; sh:targetClass ex:Person .\n"),
        );
        let hr = write(
            dir.path(),
            "hr.ttl",
            &format!(
                "{PREFIXES}ex:PersonShape sh:property [ sh:path ex:salary ; sh:minCount 1 ] .\n"
            ),
        );
        let shapes = ShapesGraph::load(&[core, hr]).unwrap();

        let person_shape = ex("PersonShape");
        let target = shapes
            .graph
            .object_for_subject_predicate(&person_shape, &sh("targetClass"));
        assert_eq!(target, Some(ex("Person").as_ref().into()));
        let property = shapes
            .graph
            .object_for_subject_predicate(&person_shape, &sh("property"))
            .expect("property shape from hr.ttl");
        let TermRef::BlankNode(property) = property else {
            panic!("property shape should be a blank node");
        };
        assert_eq!(
            shapes
                .graph
                .object_for_subject_predicate(property, &sh("path")),
            Some(ex("salary").as_ref().into())
        );
    }

    #[test]
    fn input_order_does_not_matter() {
        let dir = tempfile::tempdir().unwrap();
        let a = write(
            dir.path(),
            "a.ttl",
            &format!("{PREFIXES}ex:S sh:property [ sh:path ex:x ] .\n"),
        );
        let b = write(
            dir.path(),
            "b.ttl",
            &format!("{PREFIXES}ex:S sh:property [ sh:path ex:y ] .\n"),
        );
        let forward = ShapesGraph::load(&[&a, &b]).unwrap();
        let backward = ShapesGraph::load(&[&b, &a, &b]).unwrap();
        assert_eq!(
            sorted_ntriples(&forward.graph),
            sorted_ntriples(&backward.graph)
        );
        assert_eq!(forward.inputs, backward.inputs);
    }

    #[test]
    fn blank_node_labels_are_scoped_per_file() {
        let dir = tempfile::tempdir().unwrap();
        let a = write(dir.path(), "a.ttl", &format!("{PREFIXES}_:p1 ex:v 1 .\n"));
        let b = write(dir.path(), "b.ttl", &format!("{PREFIXES}_:p1 ex:v 2 .\n"));
        let shapes = ShapesGraph::load(&[a, b]).unwrap();
        let subjects: std::collections::BTreeSet<String> =
            shapes.graph.iter().map(|t| t.subject.to_string()).collect();
        assert_eq!(subjects.len(), 2);
    }

    #[test]
    fn trig_named_graphs_are_unioned() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "shapes.trig",
            &format!("{PREFIXES}ex:g {{ ex:s ex:p ex:o }}\nex:s ex:q ex:o .\n"),
        );
        let shapes = ShapesGraph::load(&[path]).unwrap();
        assert!(shapes
            .graph
            .contains(TripleRef::new(&ex("s"), &ex("p"), &ex("o"))));
        assert!(shapes
            .graph
            .contains(TripleRef::new(&ex("s"), &ex("q"), &ex("o"))));
    }

    #[test]
    fn n_triples_are_supported() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "shapes.nt",
            "<http://example.org/s> <http://example.org/p> \"v\" .\n",
        );
        let shapes = ShapesGraph::load(&[path]).unwrap();
        assert_eq!(shapes.graph.len(), 1);
    }

    #[test]
    fn unsupported_format_names_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "shapes.json", "{}");
        let err = ShapesGraph::load(&[path]).unwrap_err();
        assert!(matches!(err, LoadError::UnsupportedFormat { .. }));
        assert!(err.to_string().contains("shapes.json"));
    }

    #[test]
    fn missing_file_is_an_io_error() {
        let err = ShapesGraph::load(&["/definitely/not/here.ttl"]).unwrap_err();
        assert!(matches!(err, LoadError::Io { .. }));
        assert!(err.to_string().contains("here.ttl"));
    }

    #[test]
    fn syntax_error_reports_file_and_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(
            dir.path(),
            "person.ttl",
            &format!("{PREFIXES}ex:s ex:p ex:o ex:extra .\n"),
        );
        let err = ShapesGraph::load(&[path]).unwrap_err();
        let LoadError::Syntax { line, .. } = &err else {
            panic!("expected syntax error, got {err}");
        };
        assert_eq!(*line, 3);
        assert!(err.to_string().contains("person.ttl:3:"));
    }

    #[test]
    fn triples_record_their_source_line() {
        let dir = tempfile::tempdir().unwrap();
        let body = format!(
            "{PREFIXES}\nex:PersonShape\n    sh:property [\n        sh:path ex:name ;\n        sh:minCount \"two\" ;\n    ] .\n"
        );
        let path = write(dir.path(), "person.ttl", &body);
        let shapes = ShapesGraph::load(&[path]).unwrap();
        let triple = shapes
            .graph
            .iter()
            .find(|t| t.predicate == sh("minCount").as_ref())
            .unwrap()
            .into_owned();
        assert_eq!(triple.object, Literal::new_simple_literal("two").into());
        let locations = shapes.locations(&triple);
        assert_eq!(locations, &[SourceLocation { file: 0, line: 7 }]);
        assert!(shapes
            .display_location(locations[0])
            .ends_with("person.ttl:7"));
    }

    #[test]
    fn collects_prefixes_first_file_wins() {
        let dir = tempfile::tempdir().unwrap();
        let a = write(
            dir.path(),
            "a.ttl",
            "@prefix ex: <http://example.org/> .\nex:s ex:p ex:o .\n",
        );
        let b = write(
            dir.path(),
            "b.ttl",
            "@prefix ex: <http://other.org/> .\nex:s ex:p ex:o .\n",
        );
        let shapes = ShapesGraph::load(&[b, a]).unwrap();
        assert_eq!(shapes.prefixes["ex"], "http://example.org/");
    }

    #[test]
    fn records_input_digests() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "empty.ttl", "");
        let shapes = ShapesGraph::load(&[path]).unwrap();
        assert_eq!(
            shapes.inputs[0].sha256,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn local_imports_are_followed_relative_to_the_importer() {
        let dir = tempfile::tempdir().unwrap();
        let main = write(
            dir.path(),
            "main.ttl",
            &format!("{IMPORT_PREFIXES}<> owl:imports <lib/shared.ttl> .\n"),
        );
        let shared = write(
            dir.path(),
            "lib/shared.ttl",
            &format!("{IMPORT_PREFIXES}ex:s ex:p ex:o .\n"),
        );
        let shapes = ShapesGraph::load(&[main]).unwrap();
        assert!(shapes
            .graph
            .contains(TripleRef::new(&ex("s"), &ex("p"), &ex("o"))));
        assert_eq!(shapes.inputs.len(), 2);
        assert_eq!(
            shapes.inputs[1].source,
            InputSource::File(std::fs::canonicalize(shared).unwrap())
        );
    }

    #[test]
    fn import_cycle_loads_each_file_once() {
        let dir = tempfile::tempdir().unwrap();
        let a = write(
            dir.path(),
            "a.ttl",
            &format!("{IMPORT_PREFIXES}<> owl:imports <b.ttl> .\n"),
        );
        write(
            dir.path(),
            "b.ttl",
            &format!("{IMPORT_PREFIXES}<> owl:imports <a.ttl> .\n"),
        );
        let shapes = ShapesGraph::load(&[a]).unwrap();
        assert_eq!(shapes.inputs.len(), 2);
    }

    #[test]
    fn missing_import_reports_the_importing_line() {
        let dir = tempfile::tempdir().unwrap();
        let main = write(
            dir.path(),
            "main.ttl",
            &format!("{IMPORT_PREFIXES}<> owl:imports <nope.ttl> .\n"),
        );
        let err = ShapesGraph::load(&[main]).unwrap_err();
        assert!(matches!(err, LoadError::Import { .. }));
        let message = err.to_string();
        assert!(message.contains("main.ttl:3"), "{message}");
        assert!(message.contains("nope.ttl"), "{message}");
    }

    struct FakeFetcher(&'static str);

    impl RemoteFetcher for FakeFetcher {
        fn fetch(&self, _iri: &str) -> Result<Vec<u8>, String> {
            Ok(self.0.as_bytes().to_vec())
        }
    }

    #[test]
    fn remote_import_without_opt_in_fails() {
        let dir = tempfile::tempdir().unwrap();
        let main = write(
            dir.path(),
            "main.ttl",
            &format!("{IMPORT_PREFIXES}<> owl:imports <https://example.org/shapes.ttl> .\n"),
        );
        let fetcher = FakeFetcher("");
        let err =
            ShapesGraph::load_with(&[main], LoadOptions::default(), Some(&fetcher)).unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains("https://example.org/shapes.ttl"),
            "{message}"
        );
        assert!(message.contains("--allow-remote-imports"), "{message}");
    }

    #[test]
    fn remote_import_with_opt_in_is_fetched_and_digested() {
        const REMOTE: &str =
            "<http://example.org/s> <http://example.org/p> <http://example.org/o> .\n";
        let dir = tempfile::tempdir().unwrap();
        let main = write(
            dir.path(),
            "main.ttl",
            &format!("{IMPORT_PREFIXES}<> owl:imports <https://example.org/shapes> .\n"),
        );
        let options = LoadOptions {
            allow_remote_imports: true,
        };
        let shapes = ShapesGraph::load_with(&[main], options, Some(&FakeFetcher(REMOTE))).unwrap();
        assert!(shapes
            .graph
            .contains(TripleRef::new(&ex("s"), &ex("p"), &ex("o"))));
        let remote = &shapes.inputs[1];
        assert_eq!(
            remote.source,
            InputSource::Remote("https://example.org/shapes".into())
        );
        assert_eq!(remote.sha256, hex_sha256(REMOTE.as_bytes()));
    }

    #[test]
    fn file_iris_round_trip_through_paths() {
        let path = Path::new("/tmp/my shapes/ä.ttl");
        assert_eq!(
            file_iri_to_path(&file_base_iri(path)).as_deref(),
            Some(path)
        );
        assert_eq!(file_iri_to_path("https://example.org/x.ttl"), None);
    }
}
