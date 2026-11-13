//! The interactive HTML page (`--format html`): its structure, the embedded
//! data, the escaping of what the graph holds, and the policy hashes.

use serde_json::Value;

use modfence::config::{self, Config};
use modfence::extract::python::PythonExtractor;
use modfence::extract::{ExtractCtx, Extractor, SourceFile};
use modfence::graph::{Builder, Graph};
use modfence::output::{Html, Level, Renderer, View};

/// Text that would break out of a script block, an attribute or a text node.
const EVIL: &str = "</script><img src=x onerror=alert(1)>&'\u{2028}\u{2029}";

/// A tag value naming a placeholder of the page template.
const PLACEHOLDER: &str = "@@script@@";

/// The base64 SHA-256 of the page's inline script: the four vendored libraries
/// (each followed by `;` and a newline) and `app.js`. When one of those files
/// changes, the policy test below fails and shows the new hash; compute it
/// independently (Python `hashlib.sha256` and `base64.b64encode` over the same
/// text) before pasting it here.
const SCRIPT_HASH: &str = "qGEAK1skMvcHQdPWXNEYNVC79aUCHs5D5NSu9shWiLQ=";

/// The base64 SHA-256 of `style.css`.
const STYLE_HASH: &str = "jpHdUM11btK/ML6dWsRcN6UHYG8BOSo4Lvsq0uTf0NI=";

fn config() -> String {
    format!(
        r#"
version = 1

[[node]]
match = "app/**"
tags = {{ layer = "{EVIL}", note = "{PLACEHOLDER}" }}

[[node]]
match = "lib/**"
tags = {{ layer = "lib" }}
"#
    )
}

fn graph() -> Graph {
    let config: Config = config::parse(&config()).unwrap();
    let mut builder = Builder::new(&config);
    let files = [
        ("app/__init__.py", ""),
        (
            "app/a.py",
            "import app.b\nimport requests\n\n\ndef run():\n    go()\n",
        ),
        ("app/b.py", "def go():\n    pass\n"),
        ("app/q\"uote<b>.py", ""),
        ("lib/c.py", "import app.a\nimport app.a\n"),
    ];
    for (path, text) in files {
        let file = SourceFile { path, text };
        builder.add(
            &file,
            PythonExtractor.extract(&file, &ExtractCtx::default()),
        );
    }
    builder.build()
}

fn page(level: Level, group_by: Option<&str>) -> String {
    let graph = graph();
    Html.render(&graph, &View::new(&graph, level, group_by))
}

/// The text between the first `open` and the next `close` after it.
fn between<'a>(text: &'a str, open: &str, close: &str) -> &'a str {
    let start = text.find(open).expect(open) + open.len();
    let end = start + text[start..].find(close).expect(close);
    &text[start..end]
}

fn data(page: &str) -> Value {
    let block = between(
        page,
        "<script type=\"application/json\" id=\"modfence-data\">",
        "</script>",
    );
    serde_json::from_str(block).unwrap()
}

#[test]
fn the_page_is_a_complete_accessible_document() {
    let page = page(Level::Module, None);
    assert!(page.starts_with("<!doctype html>\n<html lang=\"en\">\n"));
    assert!(page.ends_with("</body>\n</html>\n"));
    assert!(page.contains("<title>modfence dependency graph</title>"));
    assert!(page.contains("<h1>modfence dependency graph</h1>"));
    // The diagram is a labelled image, with the tables as its equivalent.
    assert!(page.contains(
        "<svg id=\"diagram\" role=\"img\" aria-labelledby=\"diagram-title diagram-desc\""
    ));
    assert!(page.contains("<title id=\"diagram-title\">Dependency diagram</title>"));
    assert!(page.contains("<th scope=\"col\">Node</th>"));
    assert!(page.contains("<th scope=\"col\">Relation</th>"));
    assert!(page.contains("aria-label=\"Nodes table, scrollable\" tabindex=\"0\""));
    assert!(page.contains("<button type=\"button\" id=\"zoom-in\">Zoom in</button>"));
    assert!(page.contains("<label for=\"tag-key\">Filter by tag</label>"));
    assert!(page.contains("role=\"status\" aria-live=\"polite\""));
    // Both colour schemes, a visible keyboard focus and reduced motion.
    assert!(page.contains("@media (prefers-color-scheme: dark)"));
    assert!(page.contains(":focus-visible {"));
    assert!(page.contains("(prefers-reduced-motion: reduce)"));
}

#[test]
fn the_diagram_is_hidden_until_the_script_runs() {
    // With scripting off the page is the summary and the tables.
    let page = page(Level::Module, None);
    for opening in [
        "<div class=\"controls\" id=\"controls\" role=\"group\" aria-label=\"Diagram controls\" hidden>",
        "<p class=\"muted\" id=\"hint\" hidden>",
        "<div id=\"stage\" hidden>",
        "<ul class=\"legend\" id=\"legend\" aria-label=\"Legend\" hidden>",
    ] {
        assert!(page.contains(opening), "{opening}");
    }
    // The hidden attribute must win over the display set on those blocks.
    assert!(page.contains("[hidden] {\n  display: none !important;"));
    assert!(page.contains("Hold Ctrl and scroll to zoom."));
    for button in ["pan-up", "pan-down", "pan-left", "pan-right"] {
        assert!(page.contains(&format!("<button type=\"button\" id=\"{button}\">Pan ")));
    }
}

#[test]
fn the_page_makes_no_network_request() {
    let graph = Graph::default();
    let page = Html.render(&graph, &View::new(&graph, Level::Module, None));
    for needle in [
        "<link", "<img", "<iframe", "src=", "href=", "@import", "url(http",
    ] {
        assert!(!page.contains(needle), "{needle}");
    }
    // The vendored libraries' banner comments and the SVG namespace are the
    // only URLs in the page, and none of them is fetched.
    for line in page.lines().filter(|line| line.contains("http")) {
        assert!(
            line.starts_with("// https://d3js.org/")
                || line.starts_with("<script>// https://d3js.org/")
                || line.starts_with("<meta http-equiv=")
                || line.trim() == "var NS = 'http://www.w3.org/2000/svg';",
            "{line}"
        );
    }
    let csp = between(
        &page,
        "http-equiv=\"Content-Security-Policy\" content=\"",
        "\"",
    );
    assert!(csp.starts_with("default-src 'none'; script-src 'sha256-"));
    assert!(csp.ends_with("base-uri 'none'; form-action 'none'"));
    assert!(!csp.contains("unsafe"));
}

#[test]
fn the_policy_allows_exactly_the_inline_script_and_style() {
    let page = page(Level::Module, None);
    let csp = between(
        &page,
        "http-equiv=\"Content-Security-Policy\" content=\"",
        "\"",
    );
    assert!(
        csp.contains(&format!("script-src 'sha256-{SCRIPT_HASH}'")),
        "{csp}"
    );
    assert!(
        csp.contains(&format!("style-src 'sha256-{STYLE_HASH}'")),
        "{csp}"
    );
    // The hashes are of the text between the tags, which is the vendored
    // libraries followed by the page's code.
    let script = between(&page, "<script>", "</script>");
    let expected = [
        include_str!("../src/output/html/vendor/d3-force/d3-dispatch.min.js"),
        include_str!("../src/output/html/vendor/d3-force/d3-quadtree.min.js"),
        include_str!("../src/output/html/vendor/d3-force/d3-timer.min.js"),
        include_str!("../src/output/html/vendor/d3-force/d3-force.min.js"),
    ]
    .iter()
    .fold(String::new(), |all, library| format!("{all}{library};\n"))
        + include_str!("../src/output/html/app.js");
    assert_eq!(script, expected);
    assert_eq!(
        between(&page, "<style>", "</style>"),
        include_str!("../src/output/html/style.css")
    );
    assert_eq!(page.matches("<script").count(), 2, "one data, one code");
    assert_eq!(page.matches("</script").count(), 2);
    assert_eq!(page.matches("<style").count(), 1);
}

#[test]
fn the_page_is_deterministic() {
    assert_eq!(page(Level::Module, None), page(Level::Module, None));
}

#[test]
fn the_embedded_data_is_the_view() {
    let graph = graph();
    let view = View::new(&graph, Level::Module, None);
    let data = data(&Html.render(&graph, &view));
    let nodes = data["nodes"].as_array().unwrap();
    let ids: Vec<&str> = nodes.iter().map(|n| n["id"].as_str().unwrap()).collect();
    let expected: Vec<&str> = view.nodes.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(ids, expected);
    let a = nodes.iter().find(|n| n["id"] == "mod:py:app/a.py").unwrap();
    assert_eq!(a["kind"], "module");
    assert_eq!(a["lang"], "py");
    assert_eq!(a["path"], "app/a.py");
    assert_eq!(a["cluster"], "app");
    assert_eq!(a["tags"]["layer"][0], EVIL, "round-trips unchanged");
    assert_eq!(a["tags"]["note"][0], PLACEHOLDER);
    let external = nodes.iter().find(|n| n["id"] == "ext:py:requests").unwrap();
    assert_eq!(external["kind"], "external");
    assert_eq!(external["path"], Value::Null);
    assert_eq!(external["cluster"], Value::Null);
    let edges = data["edges"].as_array().unwrap();
    assert_eq!(edges.len(), view.edges.len());
    let import = edges
        .iter()
        .find(|e| e["source"] == "mod:py:lib/c.py" && e["target"] == "mod:py:app/a.py")
        .unwrap();
    assert_eq!(import["relation"], "imports");
    assert_eq!(import["confidence"], "resolved");
    assert_eq!(import["count"], 2);
    let contains = edges.iter().find(|e| e["relation"] == "contains").unwrap();
    assert_eq!(contains["confidence"], Value::Null);
    assert_eq!(contains["count"], 0);
    let clusters = data["clusters"].as_array().unwrap();
    let labels: Vec<&str> = clusters
        .iter()
        .map(|c| c["label"].as_str().unwrap())
        .collect();
    assert_eq!(labels, ["app", "lib"]);
    assert_eq!(data["group_by"], Value::Null);
}

#[test]
fn nothing_in_the_graph_can_break_out_of_the_data_block() {
    let page = page(Level::Module, Some("layer"));
    // Only the two real blocks end with `</script>`; the hostile text is
    // escaped inside the JSON.
    assert_eq!(page.matches("</script").count(), 2);
    let block = between(
        &page,
        "<script type=\"application/json\" id=\"modfence-data\">",
        "</script>",
    );
    for raw in ['<', '>', '&', '\u{2028}', '\u{2029}'] {
        assert!(!block.contains(raw), "{raw:?}");
    }
    let escaped = "\\u003c/script\\u003e\\u003cimg src=x onerror=alert(1)\\u003e\\u0026'";
    assert!(block.contains(escaped));
    assert!(block.contains("\\u2028\\u2029"));
    let data = data(&page);
    assert_eq!(data["group_by"], "layer");
    let labels: Vec<&str> = data["clusters"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| c["label"].as_str().unwrap())
        .collect();
    assert_eq!(labels, [EVIL, "lib"]);
    // In the tables and the summary it is HTML-escaped text instead.
    assert!(page.contains("&lt;/script&gt;&lt;img src=x onerror=alert(1)&gt;&amp;&#39;"));
    assert!(!page.contains("<img src=x"));
    assert!(page.contains("grouped by tag layer)."));
    // A node whose path holds a quote sits in an attribute escaped.
    assert!(page.contains("data-id=\"mod:py:app/q&quot;uote&lt;b&gt;.py\""));
}

#[test]
fn a_value_that_looks_like_a_placeholder_is_left_alone() {
    let page = page(Level::Module, None);
    assert!(page.contains("note: @@script@@"));
    let script = between(&page, "<script>", "</script>");
    assert!(script.starts_with("// https://d3js.org/d3-dispatch/"));
}

#[test]
fn the_tables_list_every_node_and_edge() {
    let graph = graph();
    let view = View::new(&graph, Level::Module, None);
    let page = Html.render(&graph, &view);
    assert_eq!(page.matches("class=\"node-btn\"").count(), view.nodes.len());
    let edges = between(&page, "<h2 id=\"edges-heading\">", "</tbody>");
    assert_eq!(edges.matches("<tr><td").count(), view.edges.len());
    assert!(page.contains(
        "<tr><td class=\"path\">mod:py:lib/c.py</td><td>imports</td>\
         <td class=\"path\">mod:py:app/a.py</td><td>resolved</td><td>2</td></tr>"
    ));
    // A `contains` edge has no confidence and no sites.
    assert!(page.contains(
        "<td class=\"path\">pkg:py:lib</td><td>contains</td>\
         <td class=\"path\">mod:py:lib/c.py</td><td>none</td><td>none</td></tr>"
    ));
    // An external node has no path and no cluster; an untagged one has no tags.
    assert!(page.contains(
        "data-id=\"ext:py:requests\">requests</button></th><td>external</td><td>py</td>\
         <td>none</td><td class=\"path\">none</td><td class=\"tags\">none</td></tr>"
    ));
    assert!(page.contains("<td class=\"tags\">layer: lib</td>"));
    assert!(
        page.contains("; note: @@script@@</td>"),
        "tags are joined with `; `"
    );
}

#[test]
fn the_level_and_the_grouping_show_in_the_summary() {
    let package = page(Level::Package, None);
    assert!(package.contains("Showing packages and external packages: "));
    assert!(package.contains("grouped by package)."));
    assert_eq!(data(&package)["clusters"], serde_json::json!([]));
    let module = page(Level::Module, None);
    assert!(module.contains("Showing packages, modules and external packages: "));
    let symbol = page(Level::Symbol, Some("layer"));
    assert!(symbol.contains("Showing packages, modules, symbols and external packages: "));
    let tail = "grouped by tag layer). The Nodes and Edges tables list the same graph.";
    assert!(symbol.contains(tail));
}

#[test]
fn an_empty_graph_is_a_valid_page() {
    let graph = Graph::default();
    let page = Html.render(&graph, &View::new(&graph, Level::Module, None));
    assert!(page.contains("0 nodes, 0 edges, 0 clusters"));
    let data = data(&page);
    assert_eq!(data["nodes"], serde_json::json!([]));
    assert_eq!(data["edges"], serde_json::json!([]));
}

// The reference check (NFR-0002, "HTML check"): a small tokenizer over the
// generated page, not a substring search. It walks the tags (skipping comments
// and treating `<script>` and `<style>` bodies as raw text), and fails on any
// URL-valued attribute, CSS `url(...)` or `@import` that is not a fragment
// (`#...`) or a `data:` URI, on tags that fetch (`img`, `link`, `iframe`, ...)
// and on script code that names a network API.

/// What the check found in one page.
#[derive(Default)]
struct References {
    tags: usize,
    checked: usize,
    problems: Vec<String>,
}

/// Attributes whose value is a URL (or a list of them).
const URL_ATTRIBUTES: [&str; 20] = [
    "src",
    "href",
    "xlink:href",
    "srcset",
    "action",
    "formaction",
    "poster",
    "data",
    "cite",
    "background",
    "ping",
    "manifest",
    "codebase",
    "longdesc",
    "imagesrcset",
    "usemap",
    "archive",
    "classid",
    "profile",
    "icon",
];

/// Tags that load something or change how URLs resolve.
const FETCHING_TAGS: [&str; 13] = [
    "link", "img", "iframe", "frame", "object", "embed", "base", "audio", "video", "source",
    "track", "picture", "form",
];

/// Names a script must not contain.
const NETWORK_APIS: [&str; 11] = [
    "fetch(",
    "serviceWorker",
    "window.open(",
    "XMLHttpRequest",
    "WebSocket",
    "EventSource",
    "sendBeacon",
    "importScripts",
    "newImage",
    "import(",
    "Worker(",
];

fn is_local(value: &str) -> bool {
    let value = value.trim();
    value.starts_with('#')
        || value
            .get(..5)
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("data:"))
}

/// Whether every URL in a list-valued attribute (`srcset`, `ping`) is local:
/// each whitespace-separated token must be local or an image descriptor
/// (`2x`, `100w`), so a second URL after a comma is found.
fn list_is_local(value: &str) -> bool {
    value.split_whitespace().all(|token| {
        let token = token.trim_end_matches(',');
        let descriptor = token.strip_suffix(['x', 'w']).is_some_and(|number| {
            !number.is_empty() && number.chars().all(|c| c.is_ascii_digit() || c == '.')
        });
        token.is_empty() || descriptor || is_local(token)
    })
}

fn check_css(css: &str, context: &str, found: &mut References) {
    let lower = css.to_ascii_lowercase();
    for banned in ["@import", "image-set("] {
        if lower.contains(banned) {
            found.problems.push(format!("{context}: {banned}"));
        }
    }
    let mut from = 0;
    while let Some(at) = lower[from..].find("url(") {
        let start = from + at + 4;
        let tail = lower[start..].trim_start();
        let argument = match tail.chars().next() {
            Some(quote @ ('"' | '\'')) => tail[1..].split(quote).next().unwrap_or(""),
            _ => tail.split(')').next().unwrap_or(""),
        };
        found.checked += 1;
        if !is_local(argument) {
            found.problems.push(format!("{context}: url({argument})"));
        }
        from = start;
    }
}

/// The tag starting at `rest` (which begins with `<` and a letter): its
/// lowercase name, its attributes (lowercase names) and its length in bytes.
fn parse_tag(rest: &str) -> (String, Vec<(String, String)>, usize) {
    let b = rest.as_bytes();
    let mut p = 1;
    while p < b.len() && !b[p].is_ascii_whitespace() && !matches!(b[p], b'/' | b'>') {
        p += 1;
    }
    let tag = rest[1..p].to_ascii_lowercase();
    let mut attributes = Vec::new();
    loop {
        while p < b.len() && (b[p].is_ascii_whitespace() || b[p] == b'/') {
            p += 1;
        }
        if p >= b.len() {
            break;
        }
        if b[p] == b'>' {
            p += 1;
            break;
        }
        let start = p;
        while p < b.len() && !b[p].is_ascii_whitespace() && !matches!(b[p], b'=' | b'>' | b'/') {
            p += 1;
        }
        let name = rest[start..p].to_ascii_lowercase();
        while p < b.len() && b[p].is_ascii_whitespace() {
            p += 1;
        }
        let mut value = String::new();
        if p < b.len() && b[p] == b'=' {
            p += 1;
            while p < b.len() && b[p].is_ascii_whitespace() {
                p += 1;
            }
            if p < b.len() && matches!(b[p], b'"' | b'\'') {
                let quote = b[p];
                p += 1;
                let from = p;
                while p < b.len() && b[p] != quote {
                    p += 1;
                }
                rest[from..p].clone_into(&mut value);
                p = (p + 1).min(b.len());
            } else {
                let from = p;
                while p < b.len() && !b[p].is_ascii_whitespace() && b[p] != b'>' {
                    p += 1;
                }
                rest[from..p].clone_into(&mut value);
            }
        }
        attributes.push((name, value));
    }
    (tag, attributes, p)
}

fn references(page: &str) -> References {
    let mut found = References::default();
    let mut at = 0;
    while let Some(offset) = page[at..].find('<') {
        at += offset;
        let rest = &page[at..];
        if let Some(tail) = rest.strip_prefix("<!--") {
            // `<!-->` and `<!--->` are complete comments in HTML.
            let skip = if rest.starts_with("<!-->") {
                5
            } else if rest.starts_with("<!--->") {
                6
            } else {
                // `--!>` also ends a comment.
                match (tail.find("-->"), tail.find("--!>")) {
                    (Some(a), Some(b)) => (a + 7).min(b + 8),
                    (Some(a), None) => a + 7,
                    (None, Some(b)) => b + 8,
                    (None, None) => rest.len(),
                }
            };
            at += skip;
            continue;
        }
        if !rest[1..].starts_with(|c: char| c.is_ascii_alphabetic()) {
            at += 1;
            continue;
        }
        let (tag, attributes, length) = parse_tag(rest);
        found.tags += 1;
        at += length;
        if FETCHING_TAGS.contains(&tag.as_str()) {
            found.problems.push(format!("<{tag}> is not allowed"));
        }
        let mut kind = String::new();
        let mut refresh = false;
        for (name, value) in &attributes {
            if URL_ATTRIBUTES.contains(&name.as_str()) {
                found.checked += 1;
                let local = if matches!(name.as_str(), "srcset" | "imagesrcset" | "ping") {
                    list_is_local(value)
                } else {
                    is_local(value)
                };
                if !local {
                    found.problems.push(format!("<{tag} {name}=\"{value}\">"));
                }
            } else if name == "style" {
                check_css(value, &format!("<{tag}> style"), &mut found);
            } else if name == "srcdoc" {
                found.problems.push(format!("<{tag} srcdoc>"));
            } else if name == "type" {
                kind = value.trim().to_ascii_lowercase();
            } else if name == "http-equiv" && value.eq_ignore_ascii_case("refresh") {
                refresh = true;
            }
        }
        if tag == "meta" && refresh {
            found.problems.push("<meta http-equiv=refresh>".to_owned());
        }
        if tag == "script" || tag == "style" {
            let close = format!("</{tag}");
            let end = page[at..]
                .to_ascii_lowercase()
                .find(&close)
                .unwrap_or(page.len() - at);
            let body = &page[at..at + end];
            at += end;
            if tag == "style" {
                check_css(body, "<style>", &mut found);
            } else if kind != "application/json" {
                let compact: String = body.split_whitespace().collect();
                for api in NETWORK_APIS {
                    if compact.contains(api) {
                        found.problems.push(format!("<script> names {api}"));
                    }
                }
            }
        }
    }
    found
}

#[test]
fn the_reference_check_accepts_fragments_and_data_uris() {
    let good = "<p>1 < 2</p><a href=\"#top\">t</a><use xlink:href='#sym'/>\
        <div style=\"fill:url(#grad)\"></div>\
        <style>.a{background:url(\"data:image/png;base64,AA\")}</style>\
        <!-- <a href=\"http://commented\"> -->\
        <script type=\"application/json\">{\"href\":\"http://x\",\"a\":\"fetch(\"}</script>";
    let found = references(good);
    assert_eq!(found.problems, Vec::<String>::new());
    assert_eq!(found.checked, 4);
    assert_eq!(found.tags, 6);
}

#[test]
fn the_reference_check_catches_the_awkward_spellings() {
    for (bad, expected) in [
        ("<A HREF=http://x>a</A>", "<a href=\"http://x\">"),
        ("<a href=&#x68;ttp://x>a</a>", "href"),
        ("<a href='javascript:alert(1)'>a</a>", "javascript:"),
        ("<svg><use href=\"http://x/s.svg#a\"/></svg>", "<use href"),
        ("<svg><image href=//x/y.png /></svg>", "<image href"),
        (
            "<svg><image srcset=\"#a 1x, https://x/b.png 2x\"/></svg>",
            "srcset",
        ),
        (
            "<svg><image srcset=\"data:a,b 1x,//x/y 2x\"/></svg>",
            "srcset",
        ),
        ("<!--> <a href=http://x>a</a>", "<a href"),
        ("<!-- x --!> <a href=http://x>a</a>", "<a href"),
        ("<script>fe\ntch (\"x\")</script>", "names fetch("),
        (
            "<script type=\"Text/JavaScript\">new  Image</script>",
            "newImage",
        ),
        ("<p srcdoc=\"<b>\"></p>", "srcdoc"),
    ] {
        let problems = references(bad).problems.join("\n");
        assert!(
            problems.contains(expected),
            "{expected} in {bad}\n{problems}"
        );
    }
}

#[test]
fn the_reference_check_catches_every_kind_of_reference() {
    let bad = "<a href=\"https://x\">a</a><img src=//x/y.png><link rel=stylesheet href=x.css>\
        <style>@import \"x.css\"; a{background:URL( 'http://x/f' )}</style>\
        <div style=\"background:url(/rel)\"></div><script>fetch(\"x\")</script>\
        <meta http-equiv=\"refresh\" content=\"0;url=http://x\">";
    let problems = references(bad).problems.join("\n");
    for expected in [
        "<a href=\"https://x\">",
        "<img> is not allowed",
        "<img src=\"//x/y.png\">",
        "<link> is not allowed",
        "<link href=\"x.css\">",
        "<style>: @import",
        "<style>: url(http://x/f)",
        "<div> style: url(/rel)",
        "<script> names fetch(",
        "<meta http-equiv=refresh>",
    ] {
        assert!(problems.contains(expected), "{expected} in\n{problems}");
    }
}

/// The page of `graph --format html` for a fixture directory, at `level`.
fn fixture_page(fixture: &str, level: &str) -> String {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(fixture);
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_modfence"))
        .args(["graph", "--format", "html", "--level", level])
        .current_dir(directory)
        .output()
        .unwrap();
    assert!(output.status.success(), "{fixture}");
    String::from_utf8(output.stdout).unwrap()
}

#[test]
fn the_pages_of_the_real_fixtures_reference_nothing_outside_the_file() {
    for fixture in ["cli/app", "python", "rust", "typescript"] {
        for level in ["module", "symbol"] {
            let page = fixture_page(fixture, level);
            let nodes = data(&page)["nodes"].as_array().unwrap().len();
            assert!(nodes > 3, "{fixture} {level}: {nodes} nodes");
            let found = references(&page);
            assert!(found.tags > 40, "{fixture} {level}: {} tags", found.tags);
            assert_eq!(found.problems, Vec::<String>::new(), "{fixture} {level}");
        }
    }
    let graph = graph();
    let page = Html.render(&graph, &View::new(&graph, Level::Symbol, Some("layer")));
    assert_eq!(references(&page).problems, Vec::<String>::new());
}
