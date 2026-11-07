//! Paths in a SCIP index: validation and the project root (ADR 0003,
//! invariant 12: modfence only reads below the analysis root).

/// Whether `path` is a safe relative path in a SCIP document: non-empty, `/`
/// separators only, and no empty, `.` or `..` segment (so it is neither
/// absolute nor able to leave the analysis root).
pub(crate) fn is_safe_relative(path: &str) -> bool {
    !path.is_empty()
        && !path.contains(['\\', '\0'])
        && path
            .split('/')
            .all(|segment| !segment.is_empty() && segment != "." && segment != "..")
}

/// Decodes `%XX` escapes; `None` for a malformed escape or a result that is
/// not UTF-8.
fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'%' {
            let hex = text.get(at + 1..at + 3)?;
            if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            out.push(u8::from_str_radix(hex, 16).ok()?);
            at += 3;
        } else {
            out.push(bytes[at]);
            at += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// What to put before a document's path to make it relative to the analysis
/// root: `""` when the index root is the analysis root (or states none), `"dir/"`
/// when it is a directory below it, and `None` when it is anything else (a
/// parent, a sibling, a non-`file` URI, or an unsafe path).
///
/// `analysis_root` is absolute, with `/` separators.
pub(crate) fn root_prefix(project_root: &str, analysis_root: &str) -> Option<String> {
    if project_root.is_empty() {
        return Some(String::new());
    }
    let uri = project_root.strip_prefix("file://")?;
    // A query or fragment is not part of a directory path.
    if uri.contains(['?', '#']) {
        return None;
    }
    let path = percent_decode(uri)?;
    let path = path.trim_end_matches('/');
    let root = analysis_root.trim_end_matches('/');
    if path == root {
        return Some(String::new());
    }
    let relative = path.strip_prefix(root)?.strip_prefix('/')?;
    is_safe_relative(relative).then(|| format!("{relative}/"))
}
