//! Flattening of `use` trees into (module path, name) pairs.

use tree_sitter::Node as TsNode;

/// One thing a `use` brings in: `specifier` is the module path as written and
/// `name` the item taken from it (`*` for a glob), or `None` when the path
/// names the module itself.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Leaf {
    pub(super) specifier: String,
    pub(super) name: Option<String>,
}

/// The text without any white space (`a :: b` is `a::b`).
pub(super) fn compact(text: &str) -> String {
    text.split_whitespace().collect()
}

fn join(prefix: &str, path: &str) -> String {
    if prefix.is_empty() {
        path.to_owned()
    } else if path.is_empty() {
        prefix.to_owned()
    } else {
        format!("{prefix}::{path}")
    }
}

/// The compact text of an optional node.
fn path_text(text: &str, node: Option<TsNode<'_>>) -> String {
    node.map_or_else(String::new, |node| {
        compact(text.get(node.byte_range()).unwrap_or_default())
    })
}

/// A path split into the module and the last name: `a::b::C` takes `C` from
/// `a::b`, `a::self` is the module `a` itself, and a single name is a module.
fn leaf(full: &str) -> Leaf {
    match full.rsplit_once("::") {
        Some((head, "self")) => Leaf {
            specifier: head.to_owned(),
            name: None,
        },
        Some((head, name)) if !head.is_empty() => Leaf {
            specifier: head.to_owned(),
            name: Some(name.to_owned()),
        },
        _ => Leaf {
            specifier: full.to_owned(),
            name: None,
        },
    }
}

/// Pushes the children of a list so that they pop in source order. Comments
/// and error nodes are left out.
fn push_children<'t>(list: TsNode<'t>, prefix: &str, stack: &mut Vec<(TsNode<'t>, String)>) {
    let first = stack.len();
    let mut cursor = list.walk();
    for child in list.named_children(&mut cursor) {
        if !child.is_extra() && !child.is_error() {
            stack.push((child, prefix.to_owned()));
        }
    }
    stack[first..].reverse();
}

/// The leaves of the argument of a `use` declaration, in source order. The
/// walk is iterative: a long chain of nested lists must not overflow the stack.
pub(super) fn leaves(text: &str, argument: TsNode<'_>) -> Vec<Leaf> {
    let mut found = Vec::new();
    let mut stack = vec![(argument, String::new())];
    while let Some((node, prefix)) = stack.pop() {
        match node.kind() {
            "scoped_use_list" => {
                let path = path_text(text, node.child_by_field_name("path"));
                if let Some(list) = node.child_by_field_name("list") {
                    push_children(list, &join(&prefix, &path), &mut stack);
                }
            }
            "use_list" => push_children(node, &prefix, &mut stack),
            "use_wildcard" => {
                let mut cursor = node.walk();
                let path = path_text(text, node.named_children(&mut cursor).next());
                found.push(Leaf {
                    specifier: join(&prefix, &path),
                    name: Some("*".to_owned()),
                });
            }
            "use_as_clause" => {
                let path = path_text(text, node.child_by_field_name("path"));
                found.push(leaf(&join(&prefix, &path)));
            }
            _ => found.push(leaf(&join(&prefix, &path_text(text, Some(node))))),
        }
    }
    found
}
