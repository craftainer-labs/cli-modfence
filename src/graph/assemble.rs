//! [`Builder`]: collects the files' facts and assembles the [`Graph`].

use std::collections::{BTreeMap, BTreeSet};

use super::locate::{Files, Resolved, dir_of};
use super::{
    Backend, Binding, Confidence, Edge, FileDiagnostic, Finding, Graph, Node, Relation, Site, Suppression,
};
use crate::config::{Config, Severity, Subject};
use crate::extract::{
    self, DiagnosticCode, FileFacts, Lang, NodeId, NodeKind, Parsed, SourceFile, Span, Tags,
    decode_component, merge_tags,
};

/// Placeholder for an external package whose name is empty (`import ""`).
const EMPTY_NAME: &str = "(empty)";

/// One analysed file, with the inline tags its annotations give.
struct FileEntry {
    path: String,
    lang: Lang,
    facts: FileFacts,
    module_tags: Tags,
    symbol_tags: BTreeMap<NodeId, Tags>,
}

/// Collects files and builds the [`Graph`].
pub struct Builder<'c> {
    config: &'c Config,
    python_roots: Vec<String>,
    files: Vec<FileEntry>,
    diagnostics: Vec<FileDiagnostic>,
    findings: Vec<Finding>,
    suppressions: Vec<Suppression>,
}

impl<'c> Builder<'c> {
    /// A builder for an analysis configured by `config`. Python absolute
    /// imports are resolved from the analysis root only; see
    /// [`with_python_roots`](Self::with_python_roots).
    #[must_use]
    pub fn new(config: &'c Config) -> Self {
        Self {
            config,
            python_roots: vec![String::new()],
            files: Vec::new(),
            diagnostics: Vec::new(),
            findings: Vec::new(),
            suppressions: Vec::new(),
        }
    }

    /// Sets the directories Python absolute imports start from, relative to
    /// the analysis root (`""` is the root itself, `"src"` a source layout),
    /// tried in order.
    #[must_use]
    pub fn with_python_roots<I, S>(mut self, roots: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.python_roots = roots.into_iter().map(Into::into).collect();
        self
    }

    /// Adds one file: its text (to tell own-line comments from trailing ones)
    /// and the facts its extractor returned. A file whose extension is not a
    /// supported language is ignored.
    pub fn add(&mut self, file: &SourceFile<'_>, facts: FileFacts) {
        let Some(lang) = Lang::from_path(file.path) else {
            return;
        };
        let mut module_tags = Tags::new();
        let mut symbol_tags: BTreeMap<NodeId, Tags> = BTreeMap::new();
        for annotation in &facts.annotations {
            let at = annotation.span;
            match &annotation.parsed {
                Err(error) => self.finding(file.path, at, Severity::Error, error.to_string()),
                Ok(Parsed::Tags(_)) if !own_line(file.text, at) => self.finding(
                    file.path,
                    at,
                    Severity::Warning,
                    "a trailing `modfence:` tag is not an annotation and is ignored; \
                     put it on its own line"
                        .to_owned(),
                ),
                Ok(Parsed::Tags(tags)) => match &annotation.target {
                    Some(id) => merge_tags(symbol_tags.entry(id.clone()).or_default(), tags),
                    None => merge_tags(&mut module_tags, tags),
                },
                Ok(Parsed::Ignore(ignore))
                    if self.config.require_reason && ignore.reason.is_none() =>
                {
                    self.finding(
                        file.path,
                        at,
                        Severity::Error,
                        "ignore[...] needs reason=\"...\" (`[annotations] require_reason`)"
                            .to_owned(),
                    );
                }
                Ok(Parsed::Ignore(ignore)) => self.suppressions.push(Suppression {
                    path: file.path.to_owned(),
                    span: at,
                    rules: ignore.rules.clone(),
                    reason: ignore.reason.clone(),
                }),
            }
        }
        self.diagnostics
            .extend(facts.diagnostics.iter().map(|diagnostic| FileDiagnostic {
                path: file.path.to_owned(),
                span: diagnostic.span,
                code: diagnostic.code,
                message: diagnostic.message.clone(),
            }));
        self.files.push(FileEntry {
            path: file.path.to_owned(),
            lang,
            facts,
            module_tags,
            symbol_tags,
        });
    }

    fn finding(&mut self, path: &str, span: Span, severity: Severity, message: String) {
        self.findings.push(Finding {
            path: path.to_owned(),
            span,
            severity,
            message,
        });
    }

    /// Resolves imports, calls and bindings and returns the graph.
    #[must_use]
    pub fn build(self) -> Graph {
        let Self {
            config,
            python_roots,
            mut files,
            mut diagnostics,
            mut findings,
            mut suppressions,
        } = self;
        files.sort_by(|a, b| a.path.cmp(&b.path));
        let paths: BTreeSet<String> = files.iter().map(|file| file.path.clone()).collect();
        let langs: BTreeMap<String, Lang> = files
            .iter()
            .map(|file| (file.path.clone(), file.lang))
            .collect();
        let resolver = Files {
            paths: &paths,
            python_roots: &python_roots,
        };
        let mut acc = Assembly::default();
        let mut imported: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for file in &files {
            acc.add_nodes(file);
            let module = module_id(file.lang, &file.path);
            for import in &file.facts.imports {
                let placed =
                    resolver.resolve(file.lang, &file.path, &import.specifier, &import.names);
                let (targets, confidence) = match placed {
                    Resolved::Files(found) => {
                        imported
                            .entry(file.path.clone())
                            .or_default()
                            .extend(found.iter().cloned());
                        let ids = found.iter().map(|path| module_id(langs[path], path));
                        (ids.collect::<Vec<_>>(), Confidence::Resolved)
                    }
                    Resolved::External(name) => {
                        (vec![acc.external(file.lang, &name)], Confidence::Resolved)
                    }
                    Resolved::Unplaced => {
                        diagnostics.push(FileDiagnostic {
                            path: file.path.clone(),
                            span: Some(import.span),
                            code: DiagnosticCode::UnresolvedImport,
                            message: format!("cannot place the import of `{}`", import.specifier),
                        });
                        let external = acc.external(file.lang, &import.specifier);
                        acc.unplaced.insert(external.clone());
                        (vec![external], Confidence::Heuristic)
                    }
                };
                for target in targets {
                    let site = (site_of(file, import.span), import.specifier.clone());
                    acc.link(
                        module.clone(),
                        target,
                        Relation::Imports,
                        Some(confidence),
                        Some(site),
                    );
                }
            }
        }
        let symbols = symbol_names(&files);
        let exports = collect_exports(&files, &acc);
        for file in &files {
            let module = module_id(file.lang, &file.path);
            let mut scope = BTreeSet::from([file.path.as_str()]);
            if let Some(others) = imported.get(&file.path) {
                scope.extend(others.iter().map(String::as_str));
            }
            for call in &file.facts.calls {
                let source = call.caller.clone().unwrap_or_else(|| module.clone());
                for path in &scope {
                    let found = symbols
                        .get(path)
                        .and_then(|names| names.get(call.callee.as_str()));
                    for target in found.into_iter().flatten() {
                        let site = (site_of(file, call.span), call.callee.clone());
                        acc.link_heuristic(source.clone(), target.clone(), Relation::Calls, site);
                    }
                }
            }
            acc.bind(file, &module, &exports);
        }
        acc.tag(config, &files);
        diagnostics.sort();
        findings.sort();
        suppressions.sort();
        Graph {
            backend: Backend::TreeSitter,
            edges: acc.edges(),
            nodes: acc.nodes,
            languages: files.iter().map(|file| file.lang).collect(),
            diagnostics,
            findings,
            suppressions,
        }
    }
}

/// A symbol exported across the language boundary, with where it lives.
struct Export<'a> {
    path: &'a str,
    lang: Lang,
    symbol: &'a NodeId,
    name: &'a str,
}

/// The exports of all files whose symbol is a node of the graph.
fn collect_exports<'a>(files: &'a [FileEntry], acc: &Assembly) -> Vec<Export<'a>> {
    let mut found = Vec::new();
    for file in files {
        for binding in &file.facts.bindings {
            if acc.nodes.contains_key(&binding.symbol) {
                found.push(Export {
                    path: &file.path,
                    lang: file.lang,
                    symbol: &binding.symbol,
                    name: &binding.exported_name,
                });
            }
        }
    }
    found
}

/// The symbols of each file by short name (the last component of the
/// qualified name, without a repeat ordinal), for call resolution.
fn symbol_names(files: &[FileEntry]) -> BTreeMap<&str, BTreeMap<String, Vec<NodeId>>> {
    let mut by_file = BTreeMap::new();
    for file in files {
        let names: &mut BTreeMap<String, Vec<NodeId>> =
            by_file.entry(file.path.as_str()).or_default();
        for node in &file.facts.nodes {
            if node.kind == NodeKind::Symbol {
                names
                    .entry(short_name(node))
                    .or_default()
                    .push(node.id.clone());
            }
        }
    }
    by_file
}

fn short_name(node: &extract::Node) -> String {
    short_name_of(&node.id)
}

/// The short name of a symbol id: the last component of its qualified name,
/// without a repeat ordinal. Empty for an id without a qualified name.
pub(super) fn short_name_of(id: &NodeId) -> String {
    let encoded = id.qualname().unwrap_or_default();
    let qualified = decode_component(encoded).unwrap_or_else(|| encoded.to_owned());
    let last = qualified.rsplit(['.', '~']).next().unwrap_or_default();
    last.split('@').next().unwrap_or_default().to_owned()
}

fn module_id(lang: Lang, path: &str) -> NodeId {
    NodeId::new(NodeKind::Module, lang, path, None).expect("a file path is not empty")
}

fn site_of(file: &FileEntry, span: Span) -> Site {
    Site {
        path: file.path.clone(),
        span,
    }
}

/// Whether the comment at `span` is the first thing on its line. Columns are
/// code points, as in [`Span`]; a line the text does not have is not.
fn own_line(text: &str, span: Span) -> bool {
    let row = usize::try_from(span.start_line)
        .unwrap_or(0)
        .saturating_sub(1);
    let column = usize::try_from(span.start_column)
        .unwrap_or(0)
        .saturating_sub(1);
    text.lines()
        .nth(row)
        .is_some_and(|line| line.chars().take(column).all(char::is_whitespace))
}

/// Whether an import names the export: the exported name is one of the
/// imported names, or the last segment of the specifier (`from pkg import
/// _core`, `import pkg._core`, `import { add } from "./native"`).
fn names_export(import: &extract::ImportEdge, name: &str) -> bool {
    import.names.iter().any(|imported| imported == name)
        || import.specifier.rsplit(['.', '/', ':']).next() == Some(name)
}

type EdgeKey = (NodeId, NodeId, Relation, Option<Confidence>);

/// The nodes and edges collected so far.
#[derive(Default)]
struct Assembly {
    nodes: BTreeMap<NodeId, Node>,
    edges: BTreeMap<EdgeKey, Vec<(Site, String)>>,
    /// External nodes made for an import that could not be placed: not
    /// packages, so a `package` selector never matches them.
    unplaced: BTreeSet<NodeId>,
}

impl Assembly {
    fn link(
        &mut self,
        source: NodeId,
        target: NodeId,
        relation: Relation,
        confidence: Option<Confidence>,
        site: Option<(Site, String)>,
    ) {
        self.edges
            .entry((source, target, relation, confidence))
            .or_default()
            .extend(site);
    }

    /// A heuristic edge with one site.
    fn link_heuristic(
        &mut self,
        source: NodeId,
        target: NodeId,
        relation: Relation,
        site: (Site, String),
    ) {
        self.link(
            source,
            target,
            relation,
            Some(Confidence::Heuristic),
            Some(site),
        );
    }

    /// The file's nodes, its package and the `contains` edges.
    fn add_nodes(&mut self, file: &FileEntry) {
        let module = module_id(file.lang, &file.path);
        let module_node = extract::Node::module(file.lang, &file.path).expect("a path");
        self.nodes
            .insert(module.clone(), Node::from_extracted(&module_node));
        for node in &file.facts.nodes {
            self.nodes
                .insert(node.id.clone(), Node::from_extracted(node));
            if node.kind == NodeKind::Symbol {
                self.link(
                    module.clone(),
                    node.id.clone(),
                    Relation::Contains,
                    None,
                    None,
                );
            }
        }
        let dir = match dir_of(&file.path) {
            "" => ".",
            dir => dir,
        };
        let package = self.group(NodeKind::Package, file.lang, dir);
        self.link(package, module, Relation::Contains, None, None);
        for binding in &file.facts.bindings {
            if let Some(node) = self.nodes.get_mut(&binding.symbol) {
                node.binding = Some(Binding {
                    framework: binding.framework,
                    exported_name: binding.exported_name.clone(),
                });
            }
        }
    }

    /// The external node for a package outside the analysis.
    fn external(&mut self, lang: Lang, name: &str) -> NodeId {
        let name = if name.is_empty() { EMPTY_NAME } else { name };
        self.group(NodeKind::External, lang, name)
    }

    /// A package or external node, created on first use.
    fn group(&mut self, kind: NodeKind, lang: Lang, name: &str) -> NodeId {
        let id = NodeId::new(kind, lang, name, None).expect("a name is not empty");
        let path = (kind == NodeKind::Package).then(|| name.to_owned());
        self.nodes.entry(id.clone()).or_insert_with(|| Node {
            id: id.clone(),
            kind,
            lang,
            label: name.to_owned(),
            path,
            symbol_kind: None,
            span: None,
            module: None,
            binding: None,
            tags: Tags::new(),
        });
        id
    }

    /// Links imports and calls of `file` to the cross-language exports they
    /// name (ADR 0003, "Cross-language edges"): an import names an export when
    /// the exported name is an imported name or the last segment of the
    /// specifier; a call binds when its callee is the exported name of a file
    /// the module imports an export from.
    fn bind(&mut self, file: &FileEntry, module: &NodeId, exports: &[Export<'_>]) {
        let foreign: Vec<&Export<'_>> = exports
            .iter()
            .filter(|export| export.lang != file.lang)
            .collect();
        let mut bound: BTreeSet<&str> = BTreeSet::new();
        for import in &file.facts.imports {
            for export in &foreign {
                if names_export(import, export.name) {
                    bound.insert(export.path);
                    let site = (site_of(file, import.span), import.specifier.clone());
                    let (source, target) = (module.clone(), export.symbol.clone());
                    self.link_heuristic(source, target, Relation::Binds, site);
                }
            }
        }
        for call in &file.facts.calls {
            let source = call.caller.clone().unwrap_or_else(|| module.clone());
            for export in &foreign {
                if bound.contains(export.path) && export.name == call.callee {
                    let site = (site_of(file, call.span), call.callee.clone());
                    let (source, target) = (source.clone(), export.symbol.clone());
                    self.link_heuristic(source, target, Relation::Binds, site);
                }
            }
        }
    }

    /// Computes every node's effective tags (ADR 0004).
    fn tag(&mut self, config: &Config, files: &[FileEntry]) {
        let module_tags: BTreeMap<&str, &Tags> = files
            .iter()
            .map(|file| (file.path.as_str(), &file.module_tags))
            .collect();
        let symbol_tags: BTreeMap<&NodeId, &Tags> = files
            .iter()
            .flat_map(|file| file.symbol_tags.iter())
            .collect();
        let none = Tags::new();
        for node in self.nodes.values_mut() {
            let is_package = node.kind == NodeKind::External && !self.unplaced.contains(&node.id);
            let inline: Vec<&Tags> = match node.kind {
                NodeKind::Module | NodeKind::Symbol => node
                    .path
                    .as_deref()
                    .and_then(|path| module_tags.get(path))
                    .copied()
                    .into_iter()
                    .chain(symbol_tags.get(&node.id).copied())
                    .collect(),
                NodeKind::Package | NodeKind::External => Vec::new(),
            };
            let subject = Subject {
                kind: node.kind,
                lang: node.lang,
                path: node.path.as_deref(),
                package: is_package.then_some(node.label.as_str()),
                tags: &none,
            };
            let tags = config.effective_tags(&subject, &inline);
            node.tags = tags;
        }
    }

    /// The folded edges, in key order, with their sites sorted.
    fn edges(&mut self) -> Vec<Edge> {
        std::mem::take(&mut self.edges)
            .into_iter()
            .map(|((source, target, relation, confidence), mut sites)| {
                sites.sort();
                Edge {
                    source,
                    target,
                    relation,
                    confidence,
                    specifier: sites.first().map(|(_, specifier)| specifier.clone()),
                    sites: sites.into_iter().map(|(site, _)| site).collect(),
                }
            })
            .collect()
    }
}
