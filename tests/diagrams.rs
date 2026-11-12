//! The DOT, Mermaid and `GraphML` renderers: escaping of hostile text,
//! clusters, levels, and the data format ignoring the view. The goldens for
//! the three formats are in `tests/cli.rs`.

use modfence::config::{self, Config};
use modfence::extract::python::PythonExtractor;
use modfence::extract::{
    BindingFramework, ExtractCtx, Extractor, Lang, NodeId, NodeKind, SourceFile,
};
use modfence::graph::{Binding, Builder, Confidence, Edge, Graph, Relation};
use modfence::output::{Dot, GraphMl, Level, Mermaid, Renderer, View};

const CONFIG: &str = r#"
version = 1

[[node]]
match = "app/**"
tags = { layer = "app" }

[[node]]
match = "lib/**"
tags = { layer = "lib" }
"#;

/// Quotes, a backslash, a line break, markup that ends a script, a comment
/// and a CDATA section, two control characters and what Mermaid reads as
/// syntax (`#`, a backtick, `<`, `>`, `&`).
const HOSTILE: &str = "q\"b\\c\nline</script>-->]]>\u{1}\u{0}#`<>&'";

fn id(kind: NodeKind, path: &str, qualname: Option<&str>) -> NodeId {
    NodeId::new(kind, Lang::Py, path, qualname).unwrap()
}

fn graph() -> Graph {
    let config: Config = config::parse(CONFIG).unwrap();
    let mut builder = Builder::new(&config);
    let files = [
        ("app/__init__.py", ""),
        (
            "app/a.py",
            "import requests\nimport app.b\n\n\ndef run():\n    go()\n",
        ),
        ("app/b.py", "def go():\n    pass\n"),
        ("lib/c.py", "import app.a\nimport app.a\n"),
    ];
    for (path, text) in files {
        let file = SourceFile { path, text };
        builder.add(
            &file,
            PythonExtractor.extract(&file, &ExtractCtx::default()),
        );
    }
    let mut graph = builder.build();
    // A cross-language edge, which Python alone never produces, and a binding.
    let (run, go) = (
        id(NodeKind::Symbol, "app/a.py", Some("run")),
        id(NodeKind::Symbol, "app/b.py", Some("go")),
    );
    graph.edges.push(Edge {
        source: run.clone(),
        target: go,
        relation: Relation::Binds,
        confidence: Some(Confidence::Heuristic),
        specifier: None,
        sites: Vec::new(),
    });
    graph.nodes.get_mut(&run).unwrap().binding = Some(Binding {
        framework: BindingFramework::Pyo3,
        exported_name: "run".to_owned(),
    });
    graph
}

/// The graph with hostile text in the label, path and tags of `app/a.py`.
fn hostile() -> Graph {
    let mut graph = graph();
    let module = graph
        .nodes
        .get_mut(&id(NodeKind::Module, "app/a.py", None))
        .unwrap();
    HOSTILE.clone_into(&mut module.label);
    module.path = Some(HOSTILE.to_owned());
    module
        .tags
        .insert("note".to_owned(), [HOSTILE.to_owned()].into());
    graph
}

fn render(renderer: &dyn Renderer, graph: &Graph, level: Level, group_by: Option<&str>) -> String {
    renderer.render(graph, &View::new(graph, level, group_by))
}

#[test]
fn dot_escapes_quotes_backslashes_and_line_breaks() {
    let out = render(&Dot, &hostile(), Level::Module, None);
    // `"` and `\` are escaped, the line break is `\n`, control characters
    // become spaces, and nothing else needs escaping inside a quoted string.
    assert!(
        out.contains(r#"label="q\"b\\c\nline</script>-->]]>  #`<>&'", shape=ellipse"#),
        "{out}"
    );
    assert!(!out.contains('\u{1}') && !out.contains('\u{0}'));
    // Every line is a statement of its own: the line break did not leak.
    assert!(
        out.lines()
            .all(|line| line.starts_with("  ") || line == "}" || line.starts_with("digraph"))
    );
    assert!(out.ends_with("}\n"));
}

#[test]
fn dot_tooltips_list_the_tags_escaped_and_only_for_tagged_nodes() {
    let out = render(&Dot, &hostile(), Level::Module, None);
    // One line per key in key order; the hostile value is escaped like a label.
    assert!(
        out.contains(
            r#"shape=ellipse, tooltip="layer: app\nnote: q\"b\\c\nline</script>-->]]>  #`<>&'"];"#
        ),
        "{out}"
    );
    // A node without tags has no tooltip, and a tooltip is one line.
    let external = out
        .lines()
        .find(|line| line.contains("id=\"ext:py:requests\""))
        .expect(&out);
    assert!(!external.contains("tooltip"), "{external}");
    assert!(
        out.lines()
            .all(|line| line.starts_with("  ") || line == "}" || line.starts_with("digraph"))
    );
}

#[test]
fn dot_draws_a_label_only_at_a_level_that_shows_it() {
    let mut graph = graph();
    let module = graph
        .nodes
        .get_mut(&id(NodeKind::Module, "app/b.py", None))
        .unwrap();
    "a\"b".clone_into(&mut module.label);
    let out = render(&Dot, &graph, Level::Package, None);
    // At package level the module is not drawn; the label is escaped at module
    // level.
    assert!(!out.contains("a\\\"b"));
    assert!(render(&Dot, &graph, Level::Module, None).contains("label=\"a\\\"b\""));
}

#[test]
fn mermaid_escapes_what_it_reads_as_syntax() {
    let out = render(&Mermaid, &hostile(), Level::Module, None);
    assert!(
        out.contains("([\"q#quot;b\\c line#lt;/script#gt;--#gt;]]#gt;  #35;#96;#lt;#gt;#amp;'\"])"),
        "{out}"
    );
    // No raw `"`, `<`, `>`, backtick or line break survives in any label.
    for line in out.lines().filter(|line| line.contains("([\"")) {
        let label = &line[line.find("([\"").unwrap() + 3..line.rfind("\"])").unwrap()];
        assert!(
            !label.contains(['"', '<', '>', '`', '\n', '\u{1}', '\u{0}']),
            "{label}"
        );
    }
    assert!(out.starts_with("flowchart LR\n"));
    assert!(out.ends_with('\n'));
}

#[test]
fn mermaid_node_ids_are_generated_so_no_label_is_syntax() {
    let out = render(&Mermaid, &hostile(), Level::Module, None);
    for line in out.lines().skip(1) {
        let first = line
            .trim_start()
            .split([' ', '[', '(', '{'])
            .next()
            .unwrap();
        assert!(
            ["subgraph", "end", "linkStyle"].contains(&first)
                || first
                    .strip_prefix('n')
                    .is_some_and(|rest| rest.parse::<usize>().is_ok()),
            "{line}"
        );
    }
}

#[test]
fn graphml_escapes_markup_and_replaces_characters_xml_forbids() {
    let out = render(&GraphMl, &hostile(), Level::Module, None);
    let expected =
        "q&quot;b\\c&#10;line&lt;/script&gt;--&gt;]]&gt;\u{FFFD}\u{FFFD}#`&lt;&gt;&amp;&apos;";
    assert!(
        out.contains(&format!("<data key=\"n_label\">{expected}</data>")),
        "{out}"
    );
    assert!(out.contains(&format!("<data key=\"n_path\">{expected}</data>")));
    // No forbidden character, raw `]]>` or `-->`, or stray markup survives.
    assert!(!out.contains('\u{0}') && !out.contains('\u{1}'));
    assert!(!out.contains("]]>") && !out.contains("-->"));
    // The tags are JSON text, escaped once more as XML.
    assert!(out.contains("<data key=\"n_tags\">{&quot;layer&quot;:[&quot;app&quot;],&quot;note&quot;:[&quot;q\\&quot;b\\\\c\\nline"));
    // Every `<` opens a tag, a declaration or a closing tag.
    for rest in out.split('<').skip(1) {
        assert!(
            rest.starts_with(['?', '/', 'g', 'k', 'n', 'e', 'd']),
            "{rest}"
        );
    }
}

#[test]
fn graphml_escapes_an_id_in_an_attribute_and_carriage_returns() {
    let mut graph = graph();
    let module = graph
        .nodes
        .get_mut(&id(NodeKind::Module, "app/b.py", None))
        .unwrap();
    "tab\there\r\nend\u{FFFE}".clone_into(&mut module.label);
    let out = render(&GraphMl, &graph, Level::Module, None);
    assert!(out.contains("<data key=\"n_label\">tab&#9;here&#13;&#10;end\u{FFFD}</data>"));
    let quoted = SourceFile {
        path: "app/q\"uote<b>.py",
        text: "",
    };
    let config: Config = config::parse(CONFIG).unwrap();
    let mut builder = Builder::new(&config);
    builder.add(
        &quoted,
        PythonExtractor.extract(&quoted, &ExtractCtx::default()),
    );
    let out = render(&GraphMl, &builder.build(), Level::Module, None);
    assert!(
        out.contains("<data key=\"node_id\">mod:py:app/q&quot;uote&lt;b&gt;.py</data>"),
        "{out}"
    );
}

/// The DOT node name (`n<number>`) declared for the node `id`.
fn dot_name(dot: &str, id: &str) -> String {
    let line = dot
        .lines()
        .find(|line| line.contains(&format!("[id=\"{id}\",")))
        .unwrap_or_else(|| panic!("no node {id} in {dot}"));
    line.trim_start().split(' ').next().unwrap().to_owned()
}

/// Paths that differ only in a character the formats cannot carry: a space,
/// a tab, two control characters and DEL.
const COLLIDING: [&str; 5] = [
    "app/a b.py",
    "app/a\tb.py",
    "app/a\u{1}b.py",
    "app/a\u{2}b.py",
    "app/a\u{7f}b.py",
];

fn colliding() -> Graph {
    let config: Config = config::parse(CONFIG).unwrap();
    let mut builder = Builder::new(&config);
    for path in COLLIDING {
        let file = SourceFile { path, text: "" };
        builder.add(
            &file,
            PythonExtractor.extract(&file, &ExtractCtx::default()),
        );
    }
    let mut graph = builder.build();
    let first = id(NodeKind::Module, COLLIDING[0], None);
    for path in &COLLIDING[1..] {
        graph.edges.push(Edge {
            source: first.clone(),
            target: id(NodeKind::Module, path, None),
            relation: Relation::Imports,
            confidence: Some(Confidence::Resolved),
            specifier: None,
            sites: Vec::new(),
        });
    }
    graph
}

#[test]
fn dot_keeps_ids_that_differ_only_in_an_unrepresentable_character() {
    let out = render(&Dot, &colliding(), Level::Module, None);
    let mut names: Vec<&str> = out
        .lines()
        .filter(|line| line.contains("[id=\"mod:py:"))
        .map(|line| line.trim_start().split(' ').next().unwrap())
        .collect();
    assert_eq!(names.len(), COLLIDING.len(), "{out}");
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), COLLIDING.len(), "{out}");
    // No two edges were rewired onto the same pair of nodes.
    let mut ends: Vec<(&str, &str)> = out
        .lines()
        .filter(|line| line.contains(" -> "))
        .map(|line| {
            let mut words = line.split_whitespace();
            (words.next().unwrap(), words.nth(1).unwrap())
        })
        .collect();
    assert!(ends.len() >= 2 * (COLLIDING.len() - 1), "{out}");
    let count = ends.len();
    ends.sort_unstable();
    ends.dedup();
    assert_eq!(ends.len(), count, "{out}");
    assert!(
        ends.iter()
            .all(|(a, b)| names.contains(a) || names.contains(b))
    );
}

#[test]
fn graphml_keeps_ids_that_differ_only_in_an_unrepresentable_character() {
    let graph = colliding();
    let out = render(&GraphMl, &graph, Level::Module, None);
    let attribute = |line: &str, name: &str| -> String {
        let (_, rest) = line.split_once(&format!("{name}=\"")).unwrap();
        rest.split('"').next().unwrap().to_owned()
    };
    let mut ids: Vec<String> = out
        .lines()
        .filter(|line| line.trim_start().starts_with("<node "))
        .map(|line| attribute(line, "id"))
        .collect();
    assert_eq!(ids.len(), graph.nodes.len());
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), graph.nodes.len(), "{out}");
    // Each edge's endpoints are declared nodes, and no two edges coincide.
    let mut ends = Vec::new();
    for line in out
        .lines()
        .filter(|line| line.trim_start().starts_with("<edge "))
    {
        let (source, target) = (attribute(line, "source"), attribute(line, "target"));
        assert!(ids.contains(&source) && ids.contains(&target), "{line}");
        ends.push((source, target));
    }
    assert_eq!(ends.len(), graph.edges.len());
    ends.sort_unstable();
    ends.dedup();
    assert_eq!(ends.len(), graph.edges.len(), "{out}");
}

#[test]
fn mermaid_link_styles_follow_each_edge_in_order() {
    let mut graph = graph();
    let (run, go) = (
        id(NodeKind::Symbol, "app/a.py", Some("run")),
        id(NodeKind::Symbol, "app/b.py", Some("go")),
    );
    // A second binds edge between the same symbols, of another confidence.
    graph.edges.push(Edge {
        source: run,
        target: go,
        relation: Relation::Binds,
        confidence: Some(Confidence::Resolved),
        specifier: None,
        sites: Vec::new(),
    });
    let out = render(&Mermaid, &graph, Level::Symbol, None);
    let edges: Vec<&str> = out.lines().filter(|line| line.contains(" -->")).collect();
    let styles = [
        ("calls", "stroke-dasharray:6 3"),
        ("binds", "stroke-dasharray:2 3"),
        ("contains", "stroke:#bfbfbf"),
    ];
    for (relation, style) in styles {
        let label = format!("|\"{relation}\"|");
        let list: Vec<String> = edges
            .iter()
            .enumerate()
            .filter(|(_, edge)| edge.contains(&label))
            .map(|(number, _)| number.to_string())
            .collect();
        let line = format!("  linkStyle {} {style}\n", list.join(","));
        assert!(out.contains(&line), "{relation}: {out}");
    }
    let binds = edges.iter().filter(|edge| edge.contains("|\"binds\"|"));
    assert_eq!(binds.count(), 2, "{out}");
}

#[test]
fn clusters_are_subgraphs() {
    let graph = graph();
    let dot = render(&Dot, &graph, Level::Module, Some("layer"));
    let first = dot
        .find("  subgraph cluster_0 {\n    label=\"app\";\n")
        .expect(&dot);
    let second = dot
        .find("  subgraph cluster_1 {\n    label=\"lib\";\n")
        .expect(&dot);
    assert!(first < second);
    let app = &dot[first..second];
    assert!(app.contains("id=\"mod:py:app/a.py\"") && !app.contains("lib/c.py"));
    assert!(dot[second..].contains("id=\"mod:py:lib/c.py\""));
    let mermaid = render(&Mermaid, &graph, Level::Module, Some("layer"));
    let first = mermaid.find("  subgraph c0[\"app\"]\n").expect(&mermaid);
    let second = mermaid.find("  subgraph c1[\"lib\"]\n").expect(&mermaid);
    let end = mermaid[first..].find("  end\n").unwrap();
    assert!(first + end < second);
    // Packages by default, in both.
    assert!(render(&Dot, &graph, Level::Module, None).contains("label=\"app\";"));
    assert!(render(&Mermaid, &graph, Level::Module, None).contains("subgraph c0[\"app\"]"));
}

#[test]
fn the_level_decides_which_shapes_are_drawn() {
    let graph = graph();
    let dot = |level| render(&Dot, &graph, level, None);
    assert!(dot(Level::Package).contains("shape=box"));
    assert!(dot(Level::Package).contains("shape=diamond"));
    assert!(!dot(Level::Package).contains("shape=ellipse"));
    assert!(dot(Level::Module).contains("shape=ellipse"));
    assert!(!dot(Level::Module).contains("shape=triangle"));
    assert!(dot(Level::Symbol).contains("shape=triangle"));
    let mermaid = |level| render(&Mermaid, &graph, level, None);
    assert!(mermaid(Level::Package).contains("[\"app\"]"));
    assert!(mermaid(Level::Package).contains("{\"requests\"}"));
    assert!(!mermaid(Level::Package).contains("([\""));
    assert!(mermaid(Level::Module).contains("([\"app/a.py\"])"));
    assert!(!mermaid(Level::Module).contains("{{\""));
    assert!(mermaid(Level::Symbol).contains("{{\"run\"}}"));
}

#[test]
fn edges_are_styled_by_relation_and_counted() {
    let graph = graph();
    let dot = render(&Dot, &graph, Level::Symbol, None);
    // The target carries the arrowhead.
    let (run, go) = (
        dot_name(&dot, "sym:py:app/a.py#run"),
        dot_name(&dot, "sym:py:app/b.py#go"),
    );
    assert!(dot.contains(&format!("{run} -> {go} [style=dashed, arrowhead=normal];")));
    assert!(dot.contains(&format!("{run} -> {go} [style=dotted, arrowhead=normal];")));
    assert!(dot.contains("[style=solid, color=\"gray75\", arrowhead=normal];"));
    let (lib, app) = (
        dot_name(&dot, "mod:py:lib/c.py"),
        dot_name(&dot, "mod:py:app/a.py"),
    );
    assert!(dot.contains(&format!(
        "{lib} -> {app} [style=solid, arrowhead=normal, label=\"2\"];"
    )));
    let mermaid = render(&Mermaid, &graph, Level::Symbol, None);
    assert!(mermaid.contains("|\"calls\"|"));
    assert!(mermaid.contains("|\"binds\"|"));
    assert!(mermaid.contains("|\"contains\"|"));
    assert!(mermaid.contains("|\"2\"|"));
    for line in [
        "stroke-dasharray:6 3",
        "stroke-dasharray:2 3",
        "stroke:#bfbfbf",
    ] {
        assert!(mermaid.contains(&format!(" {line}\n")), "{line}");
    }
    assert_eq!(mermaid.matches("linkStyle").count(), 3);
}

#[test]
fn graphml_carries_everything_whatever_the_view() {
    let graph = graph();
    let all = render(&GraphMl, &graph, Level::Module, None);
    for level in [Level::Package, Level::Module, Level::Symbol] {
        for group_by in [None, Some("layer")] {
            assert_eq!(render(&GraphMl, &graph, level, group_by), all);
        }
    }
    assert!(all.contains("<data key=\"node_id\">sym:py:app/a.py#run</data>"));
    assert!(all.contains("<graph id=\"modfence\" edgedefault=\"directed\">"));
    assert!(all.contains("<data key=\"n_binding\">{&quot;exported_name&quot;:&quot;run&quot;,&quot;framework&quot;:&quot;pyo3&quot;}</data>"));
    assert!(all.contains("<data key=\"e_count\">2</data>"));
    assert!(all.contains("<data key=\"e_relation\">binds</data>"));
    assert_eq!(all.matches("<edge ").count(), graph.edges.len());
    assert_eq!(all.matches("<node ").count(), graph.nodes.len());
}

#[test]
fn an_empty_graph_is_an_empty_document() {
    let graph = Graph::default();
    assert_eq!(
        render(&Dot, &graph, Level::Module, None),
        "digraph \"modfence\" {\n  rankdir=LR;\n}\n"
    );
    assert_eq!(
        render(&Mermaid, &graph, Level::Module, None),
        "flowchart LR\n"
    );
    let xml = render(&GraphMl, &graph, Level::Module, None);
    assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<graphml xmlns=\"http://graphml.graphdrawing.org/xmlns\">\n"));
    assert!(
        xml.ends_with(
            "  <graph id=\"modfence\" edgedefault=\"directed\">\n  </graph>\n</graphml>\n"
        )
    );
}
