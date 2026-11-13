//! The module tree of Rust crates, and `use` resolution against it (ADR 0003,
//! "Rust qualified names"). Pure functions over what the extractor reported:
//! nothing here reads the file system or `Cargo.toml`.
//!
//! A crate is found from the layout Cargo defaults to: `src/lib.rs`,
//! `src/main.rs`, `src/bin/<name>.rs`, `src/bin/<name>/main.rs`, and the
//! single-file targets in `tests/`, `examples/` and `benches/` (not below a
//! `src/`). Every `mod name;` of a file then pulls in `name.rs` or
//! `name/mod.rs` below that file's module directory, recursively, so a file
//! that no crate root reaches has no module path. `#[path]` is not read.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use super::locate::{Resolved, dir_of, join};

/// What the tree needs from one Rust file.
pub(crate) struct RustFile {
    /// The `mod name;` declarations: the inline modules they sit in, then the
    /// declared name.
    pub declared: Vec<(Vec<String>, String)>,
    /// The inline modules that hold items at the top of the file.
    pub inline: BTreeSet<String>,
}

/// The crates of the analysis, as a tree of modules that each have a file.
#[derive(Default)]
pub(crate) struct RustTree {
    /// For each file: the crate root file and the path from that root.
    modules: BTreeMap<String, (String, Vec<String>)>,
    /// The file of the module at a path of a crate (root file, path).
    files: BTreeMap<(String, Vec<String>), String>,
    /// The inline modules of each file, which a bare path may start with.
    inline: BTreeMap<String, BTreeSet<String>>,
}

/// Whether Cargo's default layout makes `path` the root of a crate.
pub(crate) fn is_root(path: &str) -> bool {
    let parts: Vec<&str> = path.split('/').collect();
    match parts.as_slice() {
        [.., "src", "lib.rs" | "main.rs"]
        | [.., "src", "bin", _, "main.rs"]
        | [.., "src", "bin", _] => true,
        [before @ .., "tests" | "examples" | "benches", _] => !before.contains(&"src"),
        _ => false,
    }
}

/// The directory below which the modules declared in `file` are looked up.
fn module_dir(file: &str, root: bool) -> String {
    let named_mod = file.rsplit('/').next() == Some("mod.rs");
    if root || named_mod {
        dir_of(file).to_owned()
    } else {
        file.strip_suffix(".rs").unwrap_or(file).to_owned()
    }
}

/// The first segment of an external path, with its leading `::` removed.
fn extern_crate(specifier: &str) -> Resolved {
    let name = specifier.trim_start_matches(':').split("::").next();
    Resolved::External(name.unwrap_or_default().to_owned())
}

impl RustTree {
    /// Builds the tree from the Rust files of the analysis, by path.
    pub(crate) fn new(info: BTreeMap<String, RustFile>) -> Self {
        let mut tree = Self::default();
        let mut queue = VecDeque::new();
        let roots: BTreeSet<&String> = info.keys().filter(|path| is_root(path)).collect();
        for root in &roots {
            tree.place(root, root, Vec::new());
            queue.push_back((*root).clone());
        }
        while let Some(file) = queue.pop_front() {
            let (root, module) = tree.modules[&file].clone();
            let dir = module_dir(&file, roots.contains(&file));
            for (inline, name) in &info[&file].declared {
                let base = inline.iter().fold(dir.clone(), |base, part| join(&base, part));
                let plain = join(&base, name);
                let candidates = [format!("{plain}.rs"), join(&plain, "mod.rs")];
                let found = candidates
                    .into_iter()
                    .find(|path| info.contains_key(path) && !tree.modules.contains_key(path));
                if let Some(child) = found {
                    let mut path = module.clone();
                    path.extend(inline.iter().cloned());
                    path.push(name.clone());
                    tree.place(&root, &child, path);
                    queue.push_back(child);
                }
            }
        }
        tree.inline = info
            .into_iter()
            .map(|(path, file)| (path, file.inline))
            .collect();
        tree
    }

    fn place(&mut self, root: &str, file: &str, path: Vec<String>) {
        self.files
            .entry((root.to_owned(), path.clone()))
            .or_insert_with(|| file.to_owned());
        self.modules
            .insert(file.to_owned(), (root.to_owned(), path));
    }

    /// Resolves the import `specifier` (with the imported `names`) written in
    /// the Rust file `from`. `crate::`, `self::` and `super::` paths go through
    /// the module tree of the file's crate, as does a bare path that starts
    /// with a module of the file's module; any other bare path is a crate.
    /// What an import reaches is the file of its longest prefix that is a
    /// module (so an item, or an inline module, is its module's file). An
    /// import of the file itself yields no file.
    pub(crate) fn resolve(&self, from: &str, specifier: &str, names: &[String]) -> Resolved {
        let parts: Vec<&str> = specifier.split("::").collect();
        let head = parts[0];
        let Some((root, module)) = self.modules.get(from) else {
            return match head {
                "crate" | "self" | "super" => Resolved::Unplaced,
                _ => extern_crate(specifier),
            };
        };
        let supers = parts.iter().take_while(|part| **part == "super").count();
        let (mut path, rest) = match head {
            "crate" => (Vec::new(), &parts[1..]),
            "self" => (module.clone(), &parts[1..]),
            "super" => match module.len().checked_sub(supers) {
                Some(kept) => (module[..kept].to_vec(), &parts[supers..]),
                None => return Resolved::Unplaced,
            },
            _ if self.is_local(from, root, module, head) => (module.clone(), &parts[..]),
            _ => return extern_crate(specifier),
        };
        path.extend(rest.iter().map(|part| (*part).to_owned()));
        Resolved::Files(self.targets(from, root, &path, names))
    }

    /// Whether `name` is a module declared in the module at `module`: a file of
    /// the crate, or an inline module of the file.
    fn is_local(&self, from: &str, root: &str, module: &[String], name: &str) -> bool {
        let mut child = module.to_vec();
        child.push(name.to_owned());
        self.files.contains_key(&(root.to_owned(), child))
            || self
                .inline
                .get(from)
                .is_some_and(|inline| inline.contains(name))
    }

    /// The files an import of `names` from the module at `path` reaches.
    fn targets(&self, from: &str, root: &str, path: &[String], names: &[String]) -> Vec<String> {
        let at = |segments: &[String]| self.files.get(&(root.to_owned(), segments.to_vec()));
        let submodules: Vec<Option<&String>> = names
            .iter()
            .map(|name| {
                let mut deeper = path.to_vec();
                deeper.push(name.clone());
                at(&deeper)
            })
            .collect();
        let all_modules = !names.is_empty() && submodules.iter().all(Option::is_some);
        let mut found: BTreeSet<&String> = submodules.into_iter().flatten().collect();
        if !all_modules {
            found.extend((0..=path.len()).rev().find_map(|keep| at(&path[..keep])));
        }
        found
            .into_iter()
            .filter(|file| file.as_str() != from)
            .cloned()
            .collect()
    }
}
