//! The file walker over the analysis root.

use std::fs;
use std::io;
use std::path::Path;

use crate::config::Config;

/// Directories that are never analysed: version control, and modfence's own
/// cache and indexes (ADR 0004, invariant 22).
const SKIPPED: [&str; 2] = [".git", ".modfence"];

/// The root-relative, `/`-separated paths of the files below `root` for which
/// `wanted` is true, sorted. Symbolic links are not followed, and a path that
/// matches an `exclude` pattern of `config` is left out together with
/// everything below it.
pub(super) fn walk(
    root: &Path,
    config: &Config,
    wanted: impl Fn(&str) -> bool,
) -> io::Result<Vec<String>> {
    let mut found = Vec::new();
    let mut pending = vec![(root.to_path_buf(), String::new())];
    while let Some((directory, prefix)) = pending.pop() {
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            let file_type = entry.file_type()?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let relative = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            if file_type.is_symlink() || config.exclude.iter().any(|glob| glob.is_match(&relative))
            {
                continue;
            }
            if file_type.is_dir() {
                if !SKIPPED.contains(&name.as_str()) {
                    pending.push((entry.path(), relative));
                }
            } else if wanted(&relative) {
                found.push(relative);
            }
        }
    }
    found.sort();
    Ok(found)
}
