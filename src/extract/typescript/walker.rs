//! The syntax-tree walk behind [`TsExtractor`](super::TsExtractor).
//!
//! One pass over the tree, in source order, collects nodes, imports, calls and
//! annotation comments. Annotation targets are resolved afterwards, because
//! the symbol an annotation sits above comes after the comment.

use std::collections::HashMap;

use tree_sitter::{Node as TsNode, Tree};

use super::qualname::{Sep, join};
use crate::extract::{
    Annotation, CallSite, Diagnostic, DiagnosticCode, FileFacts, ImportEdge, Lang, Node, NodeId,
    Parsed, SourceFile, Span, SymbolKind, column_from_byte, comment_body,
};

/// Syntax-tree depth beyond which a subtree is skipped, so a pathological file
/// cannot overflow the stack.
const MAX_DEPTH: usize = 500;

/// The name of a declaration whose name the parser could not read (an
/// error-recovered, unnamed declaration), so that it still gets a valid id.
const MISSING_NAME: &str = "(missing)";

/// Where the walk is: the enclosing qualified name and symbol, and whether
/// declarations found here are symbols (module and namespace level) or not
/// (inside bodies).
#[derive(Clone, Copy)]
struct Scope<'s> {
    qual: &'s str,
    caller: Option<&'s NodeId>,
    declares: bool,
    depth: usize,
}

impl Scope<'_> {
    const fn nested(self) -> Self {
        Self {
            declares: false,
            ..self
        }
    }

    const fn deeper(self) -> Self {
        Self {
            depth: self.depth + 1,
            ..self
        }
    }
}

/// A symbol just declared: its qualified name and, unless the declaration was
/// unusable, its id.
struct Defined {
    qual: String,
    id: Option<NodeId>,
}

impl Defined {
    /// The scope for the declaration's own contents.
    fn inside<'s>(&'s self, outer: Scope<'s>) -> Scope<'s> {
        Scope {
            qual: &self.qual,
            caller: self.id.as_ref().or(outer.caller),
            declares: false,
            depth: outer.depth,
        }
    }
}

/// An annotation whose target is not known yet.
struct Pending {
    annotation: Annotation,
    own_line: bool,
    end_row: usize,
}

pub(super) struct Walker<'a> {
    path: &'a str,
    src: &'a str,
    lines: Vec<&'a str>,
    facts: FileFacts,
    positions: HashMap<String, usize>,
    anchors: HashMap<usize, NodeId>,
    own_line_comments: HashMap<usize, usize>,
    pending: Vec<Pending>,
    truncated: bool,
}

/// Reads one file's facts from its parsed tree (`None` when parsing failed).
pub(super) fn extract(file: &SourceFile<'_>, tree: Option<&Tree>) -> FileFacts {
    let mut walker = Walker::new(file);
    walker.visit_roots(tree.map(Tree::root_node));
    walker.finish(tree)
}

fn text<'a>(src: &'a str, node: TsNode<'_>) -> &'a str {
    src.get(node.byte_range()).unwrap_or_default()
}

/// A string literal's content as written, without its quotes.
fn unquote(literal: &str) -> &str {
    literal
        .get(1..literal.len().saturating_sub(1))
        .unwrap_or_default()
}

/// A name as written. A string name loses its quotes, unless nothing would be
/// left; a name the parser could not read is `MISSING_NAME`.
fn name_text<'a>(src: &'a str, node: TsNode<'_>) -> &'a str {
    let raw = text(src, node);
    let quoted = node.kind() == "string";
    let unquoted = Some(unquote(raw)).filter(|inner| quoted && !inner.is_empty());
    Some(unquoted.unwrap_or(raw))
        .filter(|name| !name.is_empty())
        .unwrap_or(MISSING_NAME)
}

fn position(lines: &[&str], row: usize, byte_column: usize) -> (u32, u32) {
    let column = lines
        .get(row)
        .and_then(|line| column_from_byte(line, byte_column))
        .unwrap_or(1);
    (u32::try_from(row + 1).unwrap_or(u32::MAX), column)
}

fn span_of(lines: &[&str], node: TsNode<'_>) -> Span {
    let start = node.start_position();
    let end = node.end_position();
    let (start_line, start_column) = position(lines, start.row, start.column);
    let (end_line, end_column) = position(lines, end.row, end.column);
    Span {
        start_line,
        start_column,
        end_line,
        end_column,
    }
}

/// The smallest span holding both.
fn union(a: Span, b: Span) -> Span {
    let start = (a.start_line, a.start_column).min((b.start_line, b.start_column));
    let end = (a.end_line, a.end_column).max((b.end_line, b.end_column));
    Span {
        start_line: start.0,
        start_column: start.1,
        end_line: end.0,
        end_column: end.1,
    }
}

/// Whether a declaration is wrapped by `node` for the purpose of where its
/// statement starts.
fn wraps_statement(node: &TsNode<'_>) -> bool {
    matches!(
        node.kind(),
        "export_statement" | "ambient_declaration" | "lexical_declaration" | "variable_declaration"
    )
}

fn is_decorator(node: &TsNode<'_>) -> bool {
    node.kind() == "decorator"
}

/// The row an annotation comment must sit directly above to tag `node`: the
/// first row of its statement, `export` and decorators included.
fn anchor_row(node: TsNode<'_>) -> usize {
    let mut top = node;
    while let Some(parent) = top.parent().filter(wraps_statement) {
        top = parent;
    }
    while let Some(previous) = top.prev_sibling().filter(is_decorator) {
        top = previous;
    }
    top.start_position().row
}

/// The symbol kind of the declarations that are a single named symbol.
fn declaration_kind(kind: &str) -> Option<SymbolKind> {
    match kind {
        "function_declaration" | "generator_function_declaration" | "function_signature" => {
            Some(SymbolKind::Function)
        }
        "interface_declaration" => Some(SymbolKind::Interface),
        "type_alias_declaration" => Some(SymbolKind::Type),
        "enum_declaration" => Some(SymbolKind::Enum),
        _ => None,
    }
}

fn is_function_value(kind: &str) -> bool {
    matches!(
        kind,
        "arrow_function" | "function_expression" | "generator_function"
    )
}

fn is_static(member: TsNode<'_>) -> bool {
    let mut cursor = member.walk();
    member
        .children(&mut cursor)
        .any(|child| matches!(child.kind(), "static" | "static get"))
}

fn member_kind(member: TsNode<'_>) -> Option<SymbolKind> {
    match member.kind() {
        "method_definition" | "method_signature" | "abstract_method_signature" => {
            Some(SymbolKind::Method)
        }
        "public_field_definition" => {
            let function = member
                .child_by_field_name("value")
                .is_some_and(|value| is_function_value(value.kind()));
            Some(if function {
                SymbolKind::Method
            } else {
                SymbolKind::Variable
            })
        }
        _ => None,
    }
}

fn first_named(node: TsNode<'_>) -> Option<TsNode<'_>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).next()
}

/// The callee name node and the receiver node of a call's function part;
/// `None` for anything but `name(...)` and `receiver.name(...)`.
fn callee_parts(function: TsNode<'_>) -> Option<(TsNode<'_>, Option<TsNode<'_>>)> {
    match function.kind() {
        "identifier" => Some((function, None)),
        "member_expression" => function
            .child_by_field_name("property")
            .map(|property| (property, function.child_by_field_name("object"))),
        _ => None,
    }
}

/// The names of the specifiers in an `import { a as b }` or `export { a as b }`
/// clause: what is imported, not the local alias.
fn specifier_names(src: &str, clause: TsNode<'_>) -> Vec<String> {
    let mut cursor = clause.walk();
    clause
        .named_children(&mut cursor)
        .filter_map(|specifier| specifier.child_by_field_name("name"))
        .map(|name| name_text(src, name).to_owned())
        .collect()
}

/// The first syntax error or missing token, in source order.
fn first_error(node: TsNode<'_>) -> Option<TsNode<'_>> {
    if node.is_error() || node.is_missing() {
        return Some(node);
    }
    let mut cursor = node.walk();
    node.children(&mut cursor)
        .filter(TsNode::has_error)
        .find_map(first_error)
}

impl<'a> Walker<'a> {
    fn new(file: &SourceFile<'a>) -> Self {
        Self {
            path: file.path,
            src: file.text,
            lines: file.text.split('\n').collect(),
            facts: FileFacts {
                nodes: Node::module(Lang::Ts, file.path).into_iter().collect(),
                ..FileFacts::default()
            },
            positions: HashMap::new(),
            anchors: HashMap::new(),
            own_line_comments: HashMap::new(),
            pending: Vec::new(),
            truncated: false,
        }
    }

    fn finish(mut self, tree: Option<&Tree>) -> FileFacts {
        for Pending {
            mut annotation,
            own_line,
            end_row,
        } in std::mem::take(&mut self.pending)
        {
            let is_tags = matches!(annotation.parsed, Ok(Parsed::Tags(_)));
            if is_tags && !own_line {
                continue;
            }
            if is_tags {
                annotation.target = self.target_below(end_row);
            }
            self.facts.annotations.push(annotation);
        }
        if tree.is_none_or(|tree| tree.root_node().has_error()) {
            let span = tree
                .and_then(|tree| first_error(tree.root_node()))
                .map(|node| span_of(&self.lines, node));
            self.facts.diagnostics.push(Diagnostic {
                code: DiagnosticCode::ParseError,
                message: "syntax error".to_owned(),
                span,
            });
        }
        self.facts
    }

    /// The symbol whose statement starts on the row after a comment ending at
    /// `end_row`, skipping the own-line comments (documentation, other
    /// annotations) in between.
    fn target_below(&self, end_row: usize) -> Option<NodeId> {
        let mut row = end_row + 1;
        while let Some(&end) = self.own_line_comments.get(&row) {
            row = end + 1;
        }
        self.anchors.get(&row).cloned()
    }

    fn visit_roots<'t>(&mut self, roots: impl IntoIterator<Item = TsNode<'t>>) {
        let scope = Scope {
            qual: "",
            caller: None,
            declares: true,
            depth: 0,
        };
        for root in roots {
            self.visit(root, scope);
        }
    }

    fn visit(&mut self, node: TsNode<'_>, scope: Scope<'_>) {
        if scope.depth > MAX_DEPTH {
            if !self.truncated {
                self.truncated = true;
                self.facts.diagnostics.push(Diagnostic {
                    code: DiagnosticCode::ParseError,
                    message: format!("nesting deeper than {MAX_DEPTH} levels, rest skipped"),
                    span: Some(span_of(&self.lines, node)),
                });
            }
            return;
        }
        match node.kind() {
            "comment" => self.comment(node),
            "import_statement" => self.import_statement(node),
            "call_expression" => {
                self.call(node, scope);
                self.children(node, scope.nested());
            }
            "program" | "ambient_declaration" | "expression_statement" => {
                self.children(node, scope);
            }
            "export_statement" => {
                self.export_source(node);
                self.children(node, scope);
            }
            "internal_module" | "module" if scope.declares => self.namespace(node, scope),
            "class_declaration" | "abstract_class_declaration" if scope.declares => {
                let name = self.declared_name(node).unwrap_or(MISSING_NAME);
                self.class(node, scope, name);
            }
            "lexical_declaration" | "variable_declaration" if scope.declares => {
                self.variables(node, scope);
            }
            "arrow_function" | "function_expression" | "generator_function" | "class"
                if scope.declares =>
            {
                self.default_export(node, scope);
            }
            kind => match declaration_kind(kind).filter(|_| scope.declares) {
                Some(symbol) => {
                    let name = self.declared_name(node).unwrap_or(MISSING_NAME);
                    self.named(node, scope, name, symbol);
                }
                None => self.children(node, scope.nested()),
            },
        }
    }

    fn children(&mut self, node: TsNode<'_>, scope: Scope<'_>) {
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            self.visit(child, scope.deeper());
        }
    }

    fn declared_name(&self, node: TsNode<'_>) -> Option<&'a str> {
        node.child_by_field_name("name")
            .map(|name| name_text(self.src, name))
    }

    /// Records a symbol declared at `scope` and returns its qualified name and
    /// id.
    fn define(
        &mut self,
        node: TsNode<'_>,
        scope: Scope<'_>,
        sep: Sep,
        name: &str,
        kind: SymbolKind,
    ) -> Defined {
        let qual = join(scope.qual, sep, name);
        let id = self.insert(node, &qual, name, kind);
        Defined { qual, id }
    }

    /// Adds the symbol node, or, when the file already declares the same
    /// qualified name (overloads, declaration merging), widens the existing
    /// node's span to cover both.
    fn insert(
        &mut self,
        node: TsNode<'_>,
        qual: &str,
        label: &str,
        kind: SymbolKind,
    ) -> Option<NodeId> {
        let span = span_of(&self.lines, node);
        let made = Node::symbol(Lang::Ts, self.path, qual, label, kind, span);
        made.ok().map(|symbol| self.store(symbol, anchor_row(node)))
    }

    fn store(&mut self, symbol: Node, row: usize) -> NodeId {
        let id = symbol.id.clone();
        self.anchors.entry(row).or_insert_with(|| id.clone());
        if let Some(&index) = self.positions.get(id.as_str()) {
            let existing = &mut self.facts.nodes[index];
            existing.span = existing.span.zip(symbol.span).map(|(a, b)| union(a, b));
        } else {
            self.positions
                .insert(id.as_str().to_owned(), self.facts.nodes.len());
            self.facts.nodes.push(symbol);
        }
        id
    }

    /// A function, interface, type alias or enum.
    fn named(&mut self, node: TsNode<'_>, scope: Scope<'_>, name: &str, kind: SymbolKind) {
        let defined = self.define(node, scope, Sep::Dot, name, kind);
        self.children(node, defined.inside(scope));
    }

    fn class(&mut self, node: TsNode<'_>, scope: Scope<'_>, name: &str) {
        let defined = self.define(node, scope, Sep::Dot, name, SymbolKind::Class);
        let inner = defined.inside(scope);
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "class_body" {
                self.members(child, inner);
            } else {
                self.visit(child, inner.deeper());
            }
        }
    }

    fn members(&mut self, body: TsNode<'_>, class: Scope<'_>) {
        let mut cursor = body.walk();
        for member in body.children(&mut cursor) {
            let named = member_kind(member).zip(member.child_by_field_name("name"));
            if let Some((kind, name)) = named {
                let sep = if is_static(member) {
                    Sep::Dot
                } else {
                    Sep::Tilde
                };
                let name = name_text(self.src, name);
                let defined = self.define(member, class, sep, name, kind);
                self.children(member, defined.inside(class));
            } else {
                self.visit(member, class.deeper());
            }
        }
    }

    /// `const`, `let` and `var` declarations: one symbol per declarator with a
    /// plain identifier name.
    fn variables(&mut self, node: TsNode<'_>, scope: Scope<'_>) {
        let constant = node
            .child_by_field_name("kind")
            .is_some_and(|kind| kind.kind() == "const");
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            let name = child
                .child_by_field_name("name")
                .filter(|name| name.kind() == "identifier");
            let Some(name) = name.filter(|_| child.kind() == "variable_declarator") else {
                self.visit(child, scope.deeper());
                continue;
            };
            let kind = match child.child_by_field_name("value").map(|v| v.kind()) {
                Some("class") => SymbolKind::Class,
                Some(value) if is_function_value(value) => SymbolKind::Function,
                _ if constant => SymbolKind::Constant,
                _ => SymbolKind::Variable,
            };
            let name = name_text(self.src, name);
            let defined = self.define(child, scope, Sep::Dot, name, kind);
            self.children(child, defined.inside(scope));
        }
    }

    /// The value of `export default`: a function, arrow function or class,
    /// named `default` when it has no name of its own.
    fn default_export(&mut self, node: TsNode<'_>, scope: Scope<'_>) {
        let name = self.declared_name(node).unwrap_or("default");
        if node.kind() == "class" {
            self.class(node, scope, name);
        } else {
            self.named(node, scope, name, SymbolKind::Function);
        }
    }

    /// `namespace A.B { ... }` and `module "m" { ... }`: no node of their own,
    /// but their name prefixes the qualified names of what they declare.
    fn namespace(&mut self, node: TsNode<'_>, scope: Scope<'_>) {
        let name = node.child_by_field_name("name");
        let string = name.is_some_and(|name| name.kind() == "string");
        let raw = name.map_or("", |name| name_text(self.src, name));
        let parts = if string {
            vec![raw]
        } else {
            raw.split('.').collect()
        };
        let mut qual = scope.qual.to_owned();
        let parts = parts.into_iter().map(str::trim);
        for part in parts.filter(|part| !part.is_empty()) {
            qual = join(&qual, Sep::Dot, part);
        }
        let inner = Scope {
            qual: &qual,
            caller: scope.caller,
            declares: true,
            depth: scope.depth,
        };
        let mut cursor = node.walk();
        for child in node.children(&mut cursor) {
            if child.kind() == "statement_block" {
                self.children(child, inner.deeper());
            } else {
                self.visit(child, inner.nested().deeper());
            }
        }
    }

    fn import_statement(&mut self, node: TsNode<'_>) {
        let mut names = Vec::new();
        let mut source = node.child_by_field_name("source");
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            match child.kind() {
                "import_clause" => names.extend(self.clause_names(child)),
                "import_require_clause" => source = child.child_by_field_name("source"),
                _ => {}
            }
        }
        self.push_import(source, names, node);
    }

    fn clause_names(&self, clause: TsNode<'_>) -> Vec<String> {
        let mut names = Vec::new();
        let mut cursor = clause.walk();
        for part in clause.named_children(&mut cursor) {
            match part.kind() {
                "identifier" => names.push("default".to_owned()),
                "named_imports" => names.extend(specifier_names(self.src, part)),
                _ => {}
            }
        }
        names
    }

    /// `export ... from "m"`; an export without a source is not an import.
    fn export_source(&mut self, node: TsNode<'_>) {
        let mut names = Vec::new();
        let mut cursor = node.walk();
        for clause in node.named_children(&mut cursor) {
            if clause.kind() == "export_clause" {
                names.extend(specifier_names(self.src, clause));
            }
        }
        let source = node.child_by_field_name("source");
        self.push_import(source, names, node);
    }

    fn push_import(&mut self, source: Option<TsNode<'_>>, names: Vec<String>, node: TsNode<'_>) {
        if let Some(source) = source {
            self.facts.imports.push(ImportEdge {
                specifier: unquote(text(self.src, source)).to_owned(),
                names,
                span: span_of(&self.lines, node),
            });
        }
    }

    /// A call is an import when it is `import("m")` or `require("m")` with a
    /// string literal; otherwise it is a call site if its callee is a name or
    /// a member.
    fn call(&mut self, node: TsNode<'_>, scope: Scope<'_>) {
        let function = node.child_by_field_name("function");
        let literal = node
            .child_by_field_name("arguments")
            .filter(|arguments| arguments.kind() == "arguments")
            .and_then(first_named)
            .filter(|first| first.kind() == "string");
        let loads = function.is_some_and(|function| {
            function.kind() == "import"
                || (function.kind() == "identifier" && text(self.src, function) == "require")
        });
        if loads && literal.is_some() {
            self.push_import(literal, Vec::new(), node);
        } else if let Some((callee, receiver)) = function.and_then(callee_parts) {
            self.facts.calls.push(CallSite {
                callee: text(self.src, callee).to_owned(),
                receiver: receiver.map(|receiver| text(self.src, receiver).to_owned()),
                caller: scope.caller.cloned(),
                span: span_of(&self.lines, callee),
            });
        }
    }

    fn comment(&mut self, node: TsNode<'_>) {
        let start = node.start_position();
        let end = node.end_position();
        let own_line = self
            .lines
            .get(start.row)
            .and_then(|line| line.get(..start.column))
            .is_some_and(|before| before.trim().is_empty());
        if own_line {
            self.own_line_comments.insert(start.row, end.row);
        }
        let span = span_of(&self.lines, node);
        let annotation = comment_body(text(self.src, node))
            .and_then(|body| Annotation::from_comment(body, span, None));
        if let Some(annotation) = annotation {
            self.pending.push(Pending {
                annotation,
                own_line,
                end_row: end.row,
            });
        }
    }
}
