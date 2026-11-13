//! The walk over a Rust syntax tree.
//!
//! The walk is iterative, in pre-order (so facts come out in source order),
//! because a long chain of operators makes a very deep tree. Each node is
//! visited with the scope it sits in: a module (the file or an inline `mod`),
//! an `impl`, a `trait` or a function body.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use tree_sitter::{Node as TsNode, Tree};

use super::qualname::{LOCALS, encode_qualname};
use super::uses::{self, compact};
use crate::extract::{
    Annotation, BindingExport, BindingFramework, CallSite, Diagnostic, DiagnosticCode, FileFacts,
    ImportEdge, Lang, Node, NodeId, PREFIX, SourceFile, Span, SymbolKind, column_from_byte,
    comment_body,
};

/// The message when the parser returns no tree at all.
const NO_TREE: &str = "the parser produced no syntax tree";

#[derive(PartialEq)]
enum ScopeKind {
    Module,
    Impl,
    Trait,
    Function,
}

/// Where a node sits: what the names defined here are called, which inline
/// modules enclose it and which symbol a call here belongs to.
struct Scope {
    kind: ScopeKind,
    components: Vec<String>,
    modules: Vec<String>,
    caller: Option<NodeId>,
}

/// The body node of a definition and the scope of that body.
type Inner = (usize, Rc<Scope>);

/// A comment that may be an annotation, resolved once all symbols are known.
struct Pending {
    span: Span,
    row: usize,
    own_line: bool,
    text: String,
}

pub(super) struct Walker<'a> {
    path: &'a str,
    text: &'a str,
    lines: Vec<&'a str>,
    facts: FileFacts,
    counts: HashMap<String, u32>,
    symbol_rows: HashMap<usize, NodeId>,
    comment_rows: HashSet<usize>,
    pending: Vec<Pending>,
}

/// The callee name node and the receiver node of a call's `function`, for the
/// forms that name a callee.
fn callee_parts(function: TsNode<'_>) -> Option<(TsNode<'_>, Option<TsNode<'_>>)> {
    match function.kind() {
        "identifier" => Some((function, None)),
        "scoped_identifier" => function
            .child_by_field_name("name")
            .map(|name| (name, function.child_by_field_name("path"))),
        "field_expression" => function
            .child_by_field_name("field")
            .filter(|field| field.kind() == "field_identifier")
            .map(|name| (name, function.child_by_field_name("value"))),
        "generic_function" => function
            .child_by_field_name("function")
            .and_then(callee_parts),
        _ => None,
    }
}

/// The text with each run of white space replaced by one space.
fn squash(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Rewrites a leading `self` or `super` of a module path to be relative to the
/// file's module. `modules` are the inline modules the statement sits in:
/// `super::x` in `mod a { mod b { .. } }` is `self::a::x`, and a `super` that
/// leaves the file's module stays `super`.
fn anchored(specifier: &str, modules: &[String]) -> String {
    let parts: Vec<&str> = specifier.split("::").collect();
    let (skip, supers) = match parts[0] {
        "self" => (1, 0),
        "super" => {
            let count = parts.iter().take_while(|part| **part == "super").count();
            (count, count)
        }
        _ => return specifier.to_owned(),
    };
    let kept = modules.len().saturating_sub(supers);
    let beyond = supers - (modules.len() - kept);
    let mut out: Vec<&str> = Vec::new();
    if beyond > 0 {
        out.extend(std::iter::repeat_n("super", beyond));
    } else {
        out.push("self");
        out.extend(modules[..kept].iter().map(String::as_str));
    }
    out.extend(&parts[skip..]);
    out.join("::")
}

fn function_kind(scope: &Scope) -> SymbolKind {
    if matches!(scope.kind, ScopeKind::Impl | ScopeKind::Trait) {
        SymbolKind::Method
    } else {
        SymbolKind::Function
    }
}

/// The row an item starts at, counting the attributes above it, and those
/// attributes in source order. Comments between attributes are looked through.
fn leading(node: TsNode<'_>) -> (usize, Vec<TsNode<'_>>) {
    let mut row = node.start_position().row;
    let mut attributes = Vec::new();
    let mut previous = node.prev_sibling();
    while let Some(sibling) = previous {
        match sibling.kind() {
            "attribute_item" => {
                row = sibling.start_position().row;
                attributes.push(sibling);
            }
            "line_comment" | "block_comment" => {}
            _ => break,
        }
        previous = sibling.prev_sibling();
    }
    attributes.reverse();
    (row, attributes)
}

fn framework_of(attribute: &str) -> Option<BindingFramework> {
    match attribute {
        "pyfunction" | "pymodule" => Some(BindingFramework::Pyo3),
        "napi" => Some(BindingFramework::Napi),
        "wasm_bindgen" => Some(BindingFramework::WasmBindgen),
        _ => None,
    }
}

/// The key that renames the export in the framework's attribute.
fn rename_key(framework: BindingFramework) -> &'static str {
    match framework {
        BindingFramework::Pyo3 => "name",
        BindingFramework::Napi | BindingFramework::WasmBindgen => "js_name",
    }
}

/// Whether `after` (the bytes past a quote) starts with `hashes` `#`s.
fn closes(after: &[u8], hashes: usize) -> bool {
    after.iter().take_while(|byte| **byte == b'#').count() >= hashes
}

/// The index just past the string literal whose opening quote is at `open`:
/// an ordinary one ends at the first quote no backslash escapes, a raw one
/// (`r"`, `r#"`, `br##"`, ..) at a quote followed by as many `#` as opened it.
fn string_end(bytes: &[u8], open: usize) -> usize {
    let hashes = bytes[..open]
        .iter()
        .rev()
        .take_while(|byte| **byte == b'#')
        .count();
    let raw = (open - hashes).checked_sub(1).map(|at| bytes[at]) == Some(b'r');
    let hashes = if raw { hashes } else { 0 };
    let mut at = open + 1;
    while at < bytes.len() {
        match bytes[at] {
            b'\\' if !raw => at += 2,
            b'"' if closes(&bytes[at + 1..], hashes) => {
                return at + 1 + hashes;
            }
            _ => at += 1,
        }
    }
    bytes.len()
}

/// The index just past the character literal whose quote is at `quote`
/// (`'x'`, `'\''`, `'\u{2c}'`), or just past the quote when it starts a
/// lifetime (`'a`).
fn char_end(text: &str, quote: usize) -> usize {
    let rest = text.get(quote + 1..).unwrap_or_default();
    let mut chars = rest.chars();
    match chars.next() {
        Some('\\') => rest
            .get(2..)
            .and_then(|after| after.find('\''))
            .map_or(quote + 1, |end| quote + 4 + end),
        Some(ch) if chars.next() == Some('\'') => quote + 2 + ch.len_utf8(),
        _ => quote + 1,
    }
}

/// The comma-separated pieces of `text` that are not inside brackets, a
/// string (ordinary, raw or byte) or a character literal.
fn top_level_pieces(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut pieces = Vec::new();
    let (mut depth, mut start, mut at) = (0_usize, 0_usize, 0_usize);
    while at < bytes.len() {
        match bytes[at] {
            b'"' => at = string_end(bytes, at),
            b'\'' => at = char_end(text, at),
            b'(' | b'[' | b'{' => {
                depth += 1;
                at += 1;
            }
            b')' | b']' | b'}' => {
                depth = depth.saturating_sub(1);
                at += 1;
            }
            b',' if depth == 0 => {
                pieces.push(text.get(start..at).unwrap_or_default());
                at += 1;
                start = at;
            }
            _ => at += 1,
        }
    }
    pieces.push(text.get(start..).unwrap_or_default());
    pieces
}

/// The value of `key = ".."` (or `key = name`) among an attribute's own
/// top-level arguments, so a default inside `signature = (..)` is not read.
fn renamed(arguments: &str, key: &str) -> Option<String> {
    let inner = arguments.trim().strip_prefix('(')?.strip_suffix(')')?;
    top_level_pieces(inner)
        .into_iter()
        .filter_map(|piece| {
            let (name, value) = piece.split_once('=')?;
            (name.trim() == key).then(|| value.trim().trim_matches('"'))
        })
        .find(|value| !value.is_empty())
        .map(str::to_owned)
}

fn parse_error(message: &str, span: Option<Span>) -> Diagnostic {
    Diagnostic {
        code: DiagnosticCode::ParseError,
        message: message.to_owned(),
        span,
    }
}

fn line_number(row: usize) -> u32 {
    u32::try_from(row + 1).unwrap_or(u32::MAX)
}

/// The first error or missing node below `root`.
fn first_error(root: TsNode<'_>) -> TsNode<'_> {
    let mut node = root;
    loop {
        let mut cursor = node.walk();
        let next = node.children(&mut cursor).find(TsNode::has_error);
        let done = node.is_error() || node.is_missing() || next.is_none();
        node = next.filter(|_| !done).unwrap_or(node);
        if done {
            break;
        }
    }
    node
}

impl<'a> Walker<'a> {
    pub(super) fn new(file: &SourceFile<'a>, facts: FileFacts) -> Self {
        Self {
            path: file.path,
            text: file.text,
            lines: file.text.split('\n').collect(),
            facts,
            counts: HashMap::new(),
            symbol_rows: HashMap::new(),
            comment_rows: HashSet::new(),
            pending: Vec::new(),
        }
    }

    /// Walks the tree (when there is one) and returns all facts.
    pub(super) fn run(mut self, tree: Option<&Tree>) -> FileFacts {
        let root = tree.map(Tree::root_node);
        let missing = root.is_none().then(|| parse_error(NO_TREE, None));
        self.facts.diagnostics.extend(missing);
        if let Some(root) = root {
            self.walk(root);
        }
        self.finish()
    }

    fn walk(&mut self, root: TsNode<'_>) {
        if root.has_error() {
            let span = self.span(first_error(root));
            let error = parse_error("the file has a syntax error", Some(span));
            self.facts.diagnostics.push(error);
        }
        let module = Rc::new(Scope {
            kind: ScopeKind::Module,
            components: Vec::new(),
            modules: Vec::new(),
            caller: None,
        });
        let mut stack = vec![(root, module)];
        while let Some((node, scope)) = stack.pop() {
            let inner = self.visit(node, &scope);
            let mut cursor = node.walk();
            let first = stack.len();
            for child in node.children(&mut cursor) {
                let child_scope = match &inner {
                    Some((body, inside)) if *body == child.id() => Rc::clone(inside),
                    _ => Rc::clone(&scope),
                };
                stack.push((child, child_scope));
            }
            stack[first..].reverse();
        }
    }

    fn finish(mut self) -> FileFacts {
        self.facts.calls.sort_by_key(|call| call.span);
        for pending in std::mem::take(&mut self.pending) {
            let target = if pending.own_line {
                self.target_below(pending.row)
            } else {
                None
            };
            let found = comment_body(&pending.text)
                .and_then(|body| Annotation::from_comment(body, pending.span, target));
            self.facts.annotations.extend(found);
        }
        self.facts
    }

    /// Handles one node. A definition returns the id of its body node and the
    /// scope of that body.
    fn visit(&mut self, node: TsNode<'_>, scope: &Scope) -> Option<Inner> {
        match node.kind() {
            "line_comment" | "block_comment" => self.comment(node),
            "use_declaration" => self.use_declaration(node, scope),
            "extern_crate_declaration" => self.extern_crate(node),
            "call_expression" => self.call(node, scope),
            "mod_item" => return self.module(node, scope),
            "impl_item" => return self.impl_block(node, scope),
            "function_item" | "function_signature_item" => {
                return self.item(node, scope, function_kind(scope));
            }
            "struct_item" | "union_item" => return self.item(node, scope, SymbolKind::Struct),
            "enum_item" => return self.item(node, scope, SymbolKind::Enum),
            "trait_item" => return self.item(node, scope, SymbolKind::Trait),
            "type_item" => return self.item(node, scope, SymbolKind::Type),
            "const_item" => return self.item(node, scope, SymbolKind::Constant),
            "static_item" => return self.item(node, scope, SymbolKind::Variable),
            _ => {}
        }
        None
    }

    fn text_of(&self, node: TsNode<'_>) -> &'a str {
        self.text.get(node.byte_range()).unwrap_or_default()
    }

    fn column(&self, row: usize, byte: usize) -> u32 {
        self.lines
            .get(row)
            .and_then(|line| column_from_byte(line, byte))
            .unwrap_or(1)
    }

    fn span(&self, node: TsNode<'_>) -> Span {
        let (start, end) = (node.start_position(), node.end_position());
        Span {
            start_line: line_number(start.row),
            start_column: self.column(start.row, start.column),
            end_line: line_number(end.row),
            end_column: self.column(end.row, end.column),
        }
    }

    /// Declares a symbol named `name` (one component) in `scope`. Returns its
    /// id and the components of its qualified name.
    fn declare(
        &mut self,
        node: TsNode<'_>,
        scope: &Scope,
        name: String,
        kind: SymbolKind,
    ) -> Option<(NodeId, Vec<String>)> {
        let mut components = scope.components.clone();
        if scope.kind == ScopeKind::Function {
            components.push(LOCALS.to_owned());
        }
        components.push(name);
        let base = encode_qualname(&components, 1);
        let ordinal = self.counts.get(&base).map_or(1, |seen| seen + 1);
        let qualname = encode_qualname(&components, ordinal);
        let symbol = Node::symbol(
            Lang::Rs,
            self.path,
            &qualname,
            &components.join("::"),
            kind,
            self.span(node),
        )
        .ok()?;
        self.counts.insert(base, ordinal);
        let id = symbol.id.clone();
        self.facts.nodes.push(symbol);
        let (first, _) = leading(node);
        for row in first..=node.start_position().row {
            self.symbol_rows.entry(row).or_insert_with(|| id.clone());
        }
        Some((id, components))
    }

    /// A definition that is not an `impl` or a `mod`. The body of a function,
    /// a trait or a constant is the scope of what is inside.
    fn item(&mut self, node: TsNode<'_>, scope: &Scope, kind: SymbolKind) -> Option<Inner> {
        let name = self.text_of(node.child_by_field_name("name")?);
        let (id, components) = self.declare(node, scope, name.to_owned(), kind)?;
        let (_, attributes) = leading(node);
        self.bindings(&attributes, &id, name);
        let (field, inner_kind) = match node.kind() {
            "function_item" => ("body", ScopeKind::Function),
            "trait_item" => ("body", ScopeKind::Trait),
            "const_item" | "static_item" => ("value", ScopeKind::Function),
            _ => return None,
        };
        let body = node.child_by_field_name(field)?;
        let inner = Scope {
            kind: inner_kind,
            components,
            modules: scope.modules.clone(),
            caller: Some(id),
        };
        Some((body.id(), Rc::new(inner)))
    }

    /// An `impl` block. It is named `<Type>` or `<Type as Trait>`, with the
    /// type and the trait as written (white space squashed), without the
    /// block's own generics.
    fn impl_block(&mut self, node: TsNode<'_>, scope: &Scope) -> Option<Inner> {
        let mut name = squash(self.text_of(node.child_by_field_name("type")?));
        if let Some(implemented) = node.child_by_field_name("trait") {
            name = format!("{name} as {}", squash(self.text_of(implemented)));
        }
        let declared = self.declare(node, scope, format!("<{name}>"), SymbolKind::Impl);
        let (id, components) = declared?;
        let body = node.child_by_field_name("body")?;
        let inner = Scope {
            kind: ScopeKind::Impl,
            components,
            modules: scope.modules.clone(),
            caller: Some(id),
        };
        Some((body.id(), Rc::new(inner)))
    }

    /// A `mod`. An inline one adds a component to the names inside it; one
    /// without a body (`mod name;`) is an import of the file module.
    fn module(&mut self, node: TsNode<'_>, scope: &Scope) -> Option<Inner> {
        let name = self.text_of(node.child_by_field_name("name")?);
        // A module declared in a function body cannot be named from outside
        // the body (`crate::f::m` is not a path), so its path says so, as the
        // qualified names of what it holds do.
        let component = if scope.kind == ScopeKind::Function {
            format!("{LOCALS}::{name}")
        } else {
            name.to_owned()
        };
        // A `mod` is not a symbol, so a binding export on it belongs to the
        // file's module (ADR 0003).
        let (_, attributes) = leading(node);
        let file_module = self.facts.nodes.first().map(|module| module.id.clone());
        if let Some(id) = file_module {
            self.bindings(&attributes, &id, name);
        }
        let Some(body) = node.child_by_field_name("body") else {
            let specifier = anchored(&format!("self::{component}"), &scope.modules);
            let span = self.span(node);
            self.facts.imports.push(ImportEdge {
                specifier,
                names: Vec::new(),
                span,
            });
            return None;
        };
        let mut components = scope.components.clone();
        if scope.kind == ScopeKind::Function {
            components.push(LOCALS.to_owned());
        }
        components.push(name.to_owned());
        let mut modules = scope.modules.clone();
        modules.push(component);
        let inner = Scope {
            kind: ScopeKind::Module,
            components,
            modules,
            caller: scope.caller.clone(),
        };
        Some((body.id(), Rc::new(inner)))
    }

    fn use_declaration(&mut self, node: TsNode<'_>, scope: &Scope) {
        let span = self.span(node);
        let argument = node.child_by_field_name("argument");
        let leaves = argument.map(|argument| uses::leaves(self.text, argument));
        let mut edges: Vec<ImportEdge> = Vec::new();
        for leaf in leaves.unwrap_or_default() {
            let specifier = anchored(&leaf.specifier, &scope.modules);
            let slot = edges.iter().position(|edge| {
                edge.specifier == specifier && edge.names.is_empty() == leaf.name.is_none()
            });
            if let Some(index) = slot {
                edges[index].names.extend(leaf.name);
            } else {
                let names = leaf.name.into_iter().collect();
                edges.push(ImportEdge {
                    specifier,
                    names,
                    span,
                });
            }
        }
        self.facts.imports.extend(edges);
    }

    fn extern_crate(&mut self, node: TsNode<'_>) {
        let name = node.child_by_field_name("name");
        let edge = name.map(|name| ImportEdge {
            specifier: self.text_of(name).to_owned(),
            names: Vec::new(),
            span: self.span(node),
        });
        self.facts.imports.extend(edge);
    }

    fn call(&mut self, node: TsNode<'_>, scope: &Scope) {
        let function = node.child_by_field_name("function");
        if let Some((name, receiver)) = function.and_then(callee_parts) {
            let site = CallSite {
                callee: self.text_of(name).to_owned(),
                receiver: receiver.map(|node| compact(self.text_of(node))),
                caller: scope.caller.clone(),
                span: self.span(name),
            };
            self.facts.calls.push(site);
        }
    }

    /// The last path segment and the arguments text of an `attribute_item`.
    fn attribute_parts(&self, item: TsNode<'_>) -> Option<(&'a str, &'a str)> {
        let mut cursor = item.walk();
        let attribute = item.named_children(&mut cursor).next()?;
        let mut inner = attribute.walk();
        let path = self.text_of(attribute.named_children(&mut inner).next()?);
        let arguments = attribute
            .child_by_field_name("arguments")
            .map_or("", |arguments| self.text_of(arguments));
        Some((path.rsplit("::").next().unwrap_or(path).trim(), arguments))
    }

    /// Reports a binding export for each framework attribute on an item.
    fn bindings(&mut self, attributes: &[TsNode<'_>], id: &NodeId, name: &str) {
        let parsed: Vec<(&str, &str, Span)> = attributes
            .iter()
            .filter_map(|attribute| {
                let (path, arguments) = self.attribute_parts(*attribute)?;
                Some((path, arguments, self.span(*attribute)))
            })
            .collect();
        for (path, _, span) in &parsed {
            if let Some(framework) = framework_of(path) {
                let key = rename_key(framework);
                let exported = parsed
                    .iter()
                    .filter(|(other, _, _)| {
                        other == path || (*other == "pyo3" && framework == BindingFramework::Pyo3)
                    })
                    .find_map(|(_, arguments, _)| renamed(arguments, key))
                    .unwrap_or_else(|| name.to_owned());
                self.facts.bindings.push(BindingExport {
                    symbol: id.clone(),
                    framework,
                    exported_name: exported,
                    span: *span,
                });
            }
        }
    }

    fn comment(&mut self, node: TsNode<'_>) {
        let (start, end) = (node.start_position(), node.end_position());
        let own_line = self
            .lines
            .get(start.row)
            .and_then(|line| line.get(..start.column))
            .is_some_and(|before| before.trim().is_empty());
        if own_line {
            self.comment_rows.extend(start.row..=end.row);
        }
        let text = self.text_of(node).trim_end();
        if text.contains(PREFIX) {
            let span = self.span(node);
            self.pending.push(Pending {
                span,
                row: start.row,
                own_line,
                text: text.to_owned(),
            });
        }
    }

    /// The symbol that starts on the first line after `row` that is not part
    /// of a comment; `None` when that line is blank (or there is none), or
    /// starts no symbol. Attributes above a symbol count as its lines.
    fn target_below(&self, row: usize) -> Option<NodeId> {
        let mut next = row + 1;
        while self.comment_rows.contains(&next) {
            next += 1;
        }
        let line = self.lines.get(next)?;
        if line.trim().is_empty() {
            None
        } else {
            self.symbol_rows.get(&next).cloned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rename_is_read_only_from_top_level_arguments() {
        let signature = r#"(signature = (a, name="other"))"#;
        assert_eq!(renamed(signature, "name"), None);
        assert_eq!(
            renamed(r#"(name = "a, b")"#, "name").as_deref(),
            Some("a, b")
        );
        assert_eq!(
            renamed(r#"(name = "a\"b, c")"#, "name").as_deref(),
            Some(r#"a\"b, c"#)
        );
        let both = r#"(signature = (x, name="no"), name = "yes")"#;
        assert_eq!(renamed(both, "name").as_deref(), Some("yes"));
    }

    #[test]
    fn unfinished_literals_end_the_scan() {
        assert_eq!(top_level_pieces(r#"a, "b, c"#), ["a", r#" "b, c"#]);
        assert_eq!(top_level_pieces(r#"a, r#"b, c"#), ["a", r#" r#"b, c"#]);
        assert_eq!(top_level_pieces(r"a, '\"), ["a", r" '\"]);
        assert_eq!(top_level_pieces(r#"a, "b\"#), ["a", r#" "b\"#]);
        assert_eq!(top_level_pieces("a, '"), ["a", " '"]);
    }

    #[test]
    fn a_missing_tree_is_a_parse_error() {
        let file = SourceFile {
            path: "m.rs",
            text: "fn f() {}\n",
        };
        let facts = Walker::new(&file, FileFacts::default()).run(None);
        assert_eq!(facts.diagnostics.len(), 1);
        assert_eq!(facts.diagnostics[0].code, DiagnosticCode::ParseError);
        assert_eq!(facts.diagnostics[0].message, NO_TREE);
        assert_eq!(facts.diagnostics[0].span, None);
    }
}
