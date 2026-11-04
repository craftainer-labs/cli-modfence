//! The file walker over the analysis root.

use std::fs;
use std::path::Path;

use super::Failure;
use super::commands::failed;
use crate::config::Config;

/// Directories that are never analysed: version control, and modfence's own
/// cache and indexes (ADR 0004, invariant 22).
const SKIPPED: [&str; 2] = [".git", ".modfence"];

/// Whether `exclude` matches the root-relative `path`, a directory or a file.
fn excluded(config: &Config, path: &str, directory: bool) -> bool {
    config.exclude.iter().any(|glob| {
        if directory {
            glob.is_dir_match(path)
        } else {
            glob.is_match(path)
        }
    })
}

/// The root-relative, `/`-separated paths of the files below `root` for which
/// `wanted` is true, sorted. Symbolic links are not followed. A directory that
/// `exclude` matches (or that is skipped) is not read, and a file that it
/// matches is left out. A directory or entry that cannot be read fails with
/// its own path.
pub(super) fn walk(
    root: &Path,
    config: &Config,
    wanted: impl Fn(&str) -> bool,
) -> Result<Vec<String>, Failure> {
    let mut found = Vec::new();
    let mut pending = vec![(root.to_path_buf(), String::new())];
    while let Some((directory, prefix)) = pending.pop() {
        for entry in fs::read_dir(&directory).map_err(failed("walk", &directory))? {
            let entry = entry.map_err(failed("walk", &directory))?;
            let file_type = entry.file_type().map_err(failed("walk", &entry.path()))?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let relative = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}/{name}")
            };
            if file_type.is_symlink() || excluded(config, &relative, file_type.is_dir()) {
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
