//! Import resolution against the files of the analysis (ADR 0003, "Resolution
//! is the core's job"). Pure functions over a set of repository-relative paths:
//! nothing here reads the file system.

use std::collections::BTreeSet;

use crate::extract::Lang;

/// The extensions a TypeScript or JavaScript import may omit, in the order
/// they are tried.
const TS_EXTENSIONS: [&str; 8] = ["ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs"];

/// Where an import goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Resolved {
    /// Files of the analysis, sorted and without duplicates.
    Files(Vec<String>),
    /// A package outside the analysis.
    External(String),
    /// An import that looks internal but could not be placed.
    Unplaced,
}

/// What imports are resolved against.
pub(crate) struct Files<'a> {
    /// Every file of the analysis, relative to the root, with `/` separators.
    pub paths: &'a BTreeSet<String>,
    /// Directories that Python absolute imports start from; `""` is the root.
    pub python_roots: &'a [String],
}

impl Files<'_> {
    /// Resolves the import `specifier` (with the imported `names`) written in
    /// the file `from` of language `lang`.
    pub(crate) fn resolve(
        &self,
        lang: Lang,
        from: &str,
        specifier: &str,
        names: &[String],
    ) -> Resolved {
        match lang {
            Lang::Py => self.python(from, specifier, names),
            Lang::Ts => self.typescript(from, specifier),
            Lang::Rs => rust(specifier),
        }
    }

    fn python(&self, from: &str, specifier: &str, names: &[String]) -> Resolved {
        let dots = specifier.chars().take_while(|&c| c == '.').count();
        let module = specifier[dots..].replace('.', "/");
        let bases: Vec<String> = if dots == 0 {
            self.python_roots
                .iter()
                .map(|root| join(root, &module))
                .collect()
        } else {
            match up(dir_of(from), dots - 1) {
                Some(dir) => vec![join(dir, &module)],
                None => return Resolved::Unplaced,
            }
        };
        for base in &bases {
            let submodules: Vec<Option<String>> = names
                .iter()
                .map(|name| self.python_module(&join(base, name), false))
                .collect();
            let all_submodules = !names.is_empty() && submodules.iter().all(Option::is_some);
            let mut found: BTreeSet<String> = submodules.into_iter().flatten().collect();
            if !all_submodules {
                found.extend(self.python_module(base, module.is_empty() && dots > 0));
            }
            if !found.is_empty() {
                return Resolved::Files(found.into_iter().collect());
            }
        }
        if dots > 0 {
            Resolved::Unplaced
        } else {
            Resolved::External(module.split('/').next().unwrap_or_default().to_owned())
        }
    }

    /// The file of the Python module at `base` (`a/b` is `a/b.py` or
    /// `a/b/__init__.py`). With `package_only`, only `__init__.py` counts: a
    /// bare `from . import x` names the package, not a sibling `a.py`.
    fn python_module(&self, base: &str, package_only: bool) -> Option<String> {
        [format!("{base}.py"), join(base, "__init__.py")]
            .into_iter()
            .skip(usize::from(package_only))
            .filter(|path| !base.is_empty() || path.as_str() == "__init__.py")
            .find(|path| self.paths.contains(path))
    }

    fn typescript(&self, from: &str, specifier: &str) -> Resolved {
        let relative = matches!(specifier, "." | "..")
            || specifier.starts_with("./")
            || specifier.starts_with("../");
        if !relative {
            return Resolved::External(package_name(specifier));
        }
        normalize(dir_of(from), specifier)
            .and_then(|base| self.typescript_file(&base))
            .map_or(Resolved::Unplaced, |path| Resolved::Files(vec![path]))
    }

    /// The file an import of `base` means: the file itself, `base` plus an
    /// extension, a TypeScript file for a `.js` import, or `base/index.*`.
    fn typescript_file(&self, base: &str) -> Option<String> {
        let swapped: &[&str] = match base.rsplit_once('.') {
            Some((_, "js")) => &["ts", "tsx"],
            Some((_, "jsx")) => &["tsx"],
            Some((_, "mjs")) => &["mts"],
            Some((_, "cjs")) => &["cts"],
            _ => &[],
        };
        let stem = base.rsplit_once('.').map_or(base, |(stem, _)| stem);
        let candidates = std::iter::once(base.to_owned())
            .chain(TS_EXTENSIONS.iter().map(|ext| format!("{base}.{ext}")))
            .chain(swapped.iter().map(|ext| format!("{stem}.{ext}")))
            .chain(
                TS_EXTENSIONS
                    .iter()
                    .map(|ext| join(base, &format!("index.{ext}"))),
            );
        candidates
            .filter(|path| Lang::from_path(path) == Some(Lang::Ts))
            .find(|path| self.paths.contains(path))
    }
}

/// A Rust path: its first segment is a crate, unless it is relative.
fn rust(specifier: &str) -> Resolved {
    let first = specifier.split("::").next().unwrap_or_default();
    if matches!(first, "crate" | "self" | "super") {
        Resolved::Unplaced
    } else {
        Resolved::External(first.to_owned())
    }
}

/// The package a bare TypeScript specifier names: `@scope/name` or the first
/// segment.
fn package_name(specifier: &str) -> String {
    let mut parts = specifier.split('/');
    let first = parts.next().unwrap_or_default();
    match parts.next() {
        Some(second) if first.starts_with('@') => format!("{first}/{second}"),
        _ => first.to_owned(),
    }
}

/// `dir` and `rel` (which may hold `.` and `..`) as one path, or `None` when
/// `..` leaves the root.
fn normalize(dir: &str, rel: &str) -> Option<String> {
    let mut parts: Vec<&str> = dir.split('/').filter(|part| !part.is_empty()).collect();
    for part in rel.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    Some(parts.join("/"))
}

/// The directory part of a `/`-separated path (`""` for the root).
pub(crate) fn dir_of(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(dir, _)| dir)
}

/// `dir` with its last `levels` components removed, or `None` when it has
/// fewer.
fn up(dir: &str, levels: usize) -> Option<&str> {
    let mut dir = dir;
    for _ in 0..levels {
        if dir.is_empty() {
            return None;
        }
        dir = dir_of(dir);
    }
    Some(dir)
}

fn join(base: &str, rest: &str) -> String {
    match (base.is_empty(), rest.is_empty()) {
        (true, _) => rest.to_owned(),
        (_, true) => base.to_owned(),
        _ => format!("{base}/{rest}"),
    }
}
