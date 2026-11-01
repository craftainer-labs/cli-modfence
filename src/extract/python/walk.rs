//! The walk over a Python syntax tree.
//!
//! The walk is iterative, in pre-order (so facts come out in source order),
//! because a long chain of operators makes a very deep tree. Each node is
//! visited with the scope it sits in: the module, a class body or a function
//! body.

use std::collections::HashMap;
use std::rc::Rc;

use tree_sitter::{Node as TsNode, Tree};

use super::qualname::{LOCALS, encode_qualname};
use crate::extract::{
    Annotation, CallSite, Diagnostic, DiagnosticCode, FileFacts, ImportEdge, Lang, Node, NodeId,
    PREFIX, SourceFile, Span, SymbolKind, column_from_byte, comment_body,
};

#[derive(PartialEq)]
enum ScopeKind {
    Module,
    Class,
    Function,
}

/// Where a node sits: what the names defined here are called, and which symbol
/// a call here belongs to.
struct Scope {
    kind: ScopeKind,
    components: Vec<String>,
    caller: Option<NodeId>,
}

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
    pending: Vec<Pending>,
}

/// The callee name node and the receiver node of a call's `function`, for the
/// two forms that name a callee.
fn callee_parts(function: TsNode<'_>) -> Option<(TsNode<'_>, Option<TsNode<'_>>)> {
    match function.kind() {
        "identifier" => Some((function, None)),
        "attribute" => function
            .child_by_field_name("attribute")
            .map(|name| (name, function.child_by_field_name("object"))),
        _ => None,
    }
}

/// The imported name inside an `x as y` clause; any other node as it is.
fn imported(node: TsNode<'_>) -> TsNode<'_> {
    if node.kind() == "aliased_import" {
        node.child_by_field_name("name").unwrap_or(node)
    } else {
        node
    }
}

/// The text without any white space (`. . x` is `..x`).
fn compact(text: &str) -> String {
    text.split_whitespace().collect()
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
            pending: Vec::new(),
        }
    }

    /// Walks the tree (when there is one) and returns all facts.
    pub(super) fn run(mut self, tree: Option<&Tree>) -> FileFacts {
        if let Some(tree) = tree {
            self.walk(tree.root_node());
        }
        self.finish()
    }

    fn walk(&mut self, root: TsNode<'_>) {
        if root.has_error() {
            let span = self.span(first_error(root));
            self.facts.diagnostics.push(Diagnostic {
                code: DiagnosticCode::ParseError,
                message: "the file has a syntax error".to_owned(),
                span: Some(span),
            });
        }
        let module = Rc::new(Scope {
            kind: ScopeKind::Module,
            components: Vec::new(),
            caller: None,
        });
        let mut stack = vec![(root, module)];
        while let Some((node, scope)) = stack.pop() {
            let inner = self.visit(node, &scope);
            let mut cursor = node.walk();
            let children: Vec<TsNode<'_>> = node.children(&mut cursor).collect();
            for child in children.into_iter().rev() {
                let child_scope = match &inner {
                    Some((body, inside)) if *body == child.id() => Rc::clone(inside),
                    _ => Rc::clone(&scope),
                };
                stack.push((child, child_scope));
            }
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
    fn visit(&mut self, node: TsNode<'_>, scope: &Scope) -> Option<(usize, Rc<Scope>)> {
        match node.kind() {
            "comment" => self.comment(node),
            "import_statement" | "import_from_statement" => self.import(node),
            "call" => self.call(node, scope),
            "function_definition" | "class_definition" => return self.definition(node, scope),
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

    fn definition(&mut self, node: TsNode<'_>, scope: &Scope) -> Option<(usize, Rc<Scope>)> {
        let name = node.child_by_field_name("name")?;
        let body = node.child_by_field_name("body")?;
        let is_class = node.kind() == "class_definition";
        let mut components = scope.components.clone();
        if scope.kind == ScopeKind::Function {
            components.push(LOCALS.to_owned());
        }
        components.push(self.text_of(name).to_owned());
        let base = encode_qualname(&components, 1);
        let seen = self.counts.entry(base).or_insert(0);
        *seen += 1;
        let qualname = encode_qualname(&components, *seen);
        let symbol_kind = match (is_class, &scope.kind) {
            (true, _) => SymbolKind::Class,
            (false, ScopeKind::Class) => SymbolKind::Method,
            (false, _) => SymbolKind::Function,
        };
        let symbol = Node::symbol(
            Lang::Py,
            self.path,
            &qualname,
            &components.join("."),
            symbol_kind,
            self.span(node),
        )
        .ok()?;
        let id = symbol.id.clone();
        self.facts.nodes.push(symbol);
        let outer = node
            .parent()
            .filter(|parent| parent.kind() == "decorated_definition")
            .unwrap_or(node);
        for row in outer.start_position().row..=node.start_position().row {
            self.symbol_rows.insert(row, id.clone());
        }
        let inner = Scope {
            kind: if is_class {
                ScopeKind::Class
            } else {
                ScopeKind::Function
            },
            components,
            caller: Some(id),
        };
        Some((body.id(), Rc::new(inner)))
    }

    fn import(&mut self, node: TsNode<'_>) {
        let span = self.span(node);
        let mut cursor = node.walk();
        let names: Vec<TsNode<'_>> = node
            .children_by_field_name("name", &mut cursor)
            .map(imported)
            .collect();
        if node.kind() == "import_statement" {
            for name in names {
                let specifier = compact(self.text_of(name));
                self.facts.imports.push(ImportEdge {
                    specifier,
                    names: Vec::new(),
                    span,
                });
            }
        } else {
            let module = node.child_by_field_name("module_name");
            let edge = module.map(|module| self.import_from_edge(node, module, names, span));
            self.facts.imports.extend(edge);
        }
    }

    /// The edge of `from <module> import <names>`; `*` is a name of its own.
    fn import_from_edge(
        &self,
        node: TsNode<'_>,
        module: TsNode<'_>,
        names: Vec<TsNode<'_>>,
        span: Span,
    ) -> ImportEdge {
        let mut listed: Vec<String> = names
            .into_iter()
            .map(|name| compact(self.text_of(name)))
            .collect();
        let mut cursor = node.walk();
        if node
            .named_children(&mut cursor)
            .any(|child| child.kind() == "wildcard_import")
        {
            listed.push("*".to_owned());
        }
        ImportEdge {
            specifier: compact(self.text_of(module)),
            names: listed,
            span,
        }
    }

    fn call(&mut self, node: TsNode<'_>, scope: &Scope) {
        let function = node.child_by_field_name("function");
        if let Some((name, receiver)) = function.and_then(callee_parts) {
            let site = CallSite {
                callee: self.text_of(name).to_owned(),
                receiver: receiver.map(|node| self.text_of(node).to_owned()),
                caller: scope.caller.clone(),
                span: self.span(name),
            };
            self.facts.calls.push(site);
        }
    }

    fn comment(&mut self, node: TsNode<'_>) {
        let text = self.text_of(node);
        if text.contains(PREFIX) {
            let start = node.start_position();
            let own_line = self
                .lines
                .get(start.row)
                .and_then(|line| line.get(..start.column))
                .is_some_and(|before| before.trim().is_empty());
            let span = self.span(node);
            self.pending.push(Pending {
                span,
                row: start.row,
                own_line,
                text: text.to_owned(),
            });
        }
    }

    /// The symbol that starts on the first line after `row` that is neither a
    /// comment nor blank; `None` when a blank line (or the end of the file)
    /// comes first, or the line starts no symbol.
    fn target_below(&self, row: usize) -> Option<NodeId> {
        self.lines
            .iter()
            .enumerate()
            .skip(row + 1)
            .find(|(_, line)| !line.trim_start().starts_with('#'))
            .filter(|(_, line)| !line.trim().is_empty())
            .and_then(|(next, _)| self.symbol_rows.get(&next).cloned())
    }
}
