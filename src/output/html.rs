//! The interactive HTML page (`--format html`, ADR 0004, "Traceability
//! outputs"): one self-contained file that draws the [`View`] with a force
//! layout, and lists the same nodes and edges in tables.
//!
//! The page makes no network request. The layout is the vendored d3-force
//! (`html/vendor/d3-force/`, with its licence and checksums) inlined next to
//! the page's own script. A `Content-Security-Policy` meta allows exactly that
//! one inline script and the one inline style, each by the SHA-256 of its
//! text, and nothing else. The view is embedded as a JSON data block, escaped
//! so no text in it can end the block; the tables are static HTML, so the
//! graph is readable without the script.
//!
//! The text is a pure function of the view: all the randomness of the layout
//! happens in the browser.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::{Level, Renderer, View};
use crate::extract::{NodeId, Tags};
use crate::graph::{Confidence, Graph};

const PAGE: &str = include_str!("html/page.html");
const STYLE: &str = include_str!("html/style.css");
const APP: &str = include_str!("html/app.js");
/// The vendored UMD builds, dependencies first.
const D3: [&str; 4] = [
    include_str!("html/vendor/d3-force/d3-dispatch.min.js"),
    include_str!("html/vendor/d3-force/d3-quadtree.min.js"),
    include_str!("html/vendor/d3-force/d3-timer.min.js"),
    include_str!("html/vendor/d3-force/d3-force.min.js"),
];

/// The interactive HTML page of a view.
#[derive(Debug, Clone, Copy, Default)]
pub struct Html;

impl Renderer for Html {
    fn render(&self, _graph: &Graph, view: &View<'_>) -> String {
        let script = script();
        let csp = format!(
            "default-src 'none'; script-src 'sha256-{}'; style-src 'sha256-{}'; \
             base-uri 'none'; form-action 'none'",
            base64(&sha256(script.as_bytes())),
            base64(&sha256(lf(STYLE).as_bytes())),
        );
        let summary = summary(view);
        let description = format!("{summary} The Nodes and Edges tables list the same graph.");
        let values = [
            ("csp", csp),
            ("style", lf(STYLE)),
            ("summary", escape(&summary)),
            ("description", escape(&description)),
            ("nodes", node_rows(view)),
            ("edges", edge_rows(view)),
            ("data", script_safe(&data(view).to_string())),
            ("script", script),
        ];
        fill(PAGE, &values)
    }
}

/// The inline script: the vendored libraries, then the page's own code.
fn script() -> String {
    let mut text = String::new();
    for library in D3 {
        text.push_str(library);
        text.push_str(";\n");
    }
    text.push_str(APP);
    lf(&text)
}

/// `text` with CR LF and lone CR as LF, which is how a browser reads inline
/// script and style text before it hashes it for the policy, so a checkout
/// with CRLF line endings still matches.
fn lf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

/// What the view shows, in one sentence.
fn summary(view: &View<'_>) -> String {
    let level = match view.level {
        Level::Package => "packages and external packages",
        Level::Module => "packages, modules and external packages",
        Level::Symbol => "packages, modules, symbols and external packages",
    };
    let group = view
        .group_by
        .as_deref()
        .map_or_else(|| "package".to_owned(), |key| format!("tag {key}"));
    format!(
        "Showing {level}: {} nodes, {} edges, {} clusters (grouped by {group}).",
        view.nodes.len(),
        view.edges.len(),
        view.clusters.len(),
    )
}

/// The label of the cluster of each clustered node.
fn cluster_of<'v, 'a>(view: &'v View<'a>) -> BTreeMap<&'a NodeId, &'v str> {
    view.clusters
        .iter()
        .flat_map(|cluster| {
            cluster
                .members
                .iter()
                .map(move |member| (*member, cluster.label.as_str()))
        })
        .collect()
}

/// The view as the page's script reads it.
fn data(view: &View<'_>) -> Value {
    let cluster_of = cluster_of(view);
    let nodes: Vec<Value> = view
        .nodes
        .iter()
        .map(|node| {
            json!({
                "id": node.id.as_str(),
                "label": node.label,
                "kind": node.kind.as_str(),
                "lang": node.lang.as_str(),
                "path": node.path,
                "tags": node.tags,
                "cluster": cluster_of.get(&node.id).copied(),
            })
        })
        .collect();
    let edges: Vec<Value> = view
        .edges
        .iter()
        .map(|edge| {
            json!({
                "source": edge.source.as_str(),
                "target": edge.target.as_str(),
                "relation": edge.relation.as_str(),
                "confidence": edge.confidence.map(Confidence::as_str),
                "count": edge.count,
            })
        })
        .collect();
    let clusters: Vec<Value> = view
        .clusters
        .iter()
        .map(|cluster| {
            json!({
                "label": cluster.label,
                "members": cluster.members.iter().map(|id| id.as_str()).collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({
        "group_by": view.group_by,
        "nodes": nodes,
        "edges": edges,
        "clusters": clusters,
    })
}

/// The rows of the node table.
fn node_rows(view: &View<'_>) -> String {
    let cluster_of = cluster_of(view);
    view.nodes
        .iter()
        .map(|node| {
            format!(
                "<tr><th scope=\"row\"><button type=\"button\" class=\"node-btn\" \
                 aria-pressed=\"false\" data-id=\"{}\">{}</button></th><td>{}</td><td>{}</td>\
                 <td>{}</td><td class=\"path\">{}</td><td class=\"tags\">{}</td></tr>\n",
                escape(node.id.as_str()),
                escape(&node.label),
                node.kind.as_str(),
                node.lang.as_str(),
                escape(cluster_of.get(&node.id).copied().unwrap_or("none")),
                escape(node.path.as_deref().unwrap_or("none")),
                escape(&tags_text(&node.tags)),
            )
        })
        .collect::<Vec<String>>()
        .concat()
}

/// `key: value, value; key: value`, or `none`.
fn tags_text(tags: &Tags) -> String {
    if tags.is_empty() {
        return "none".to_owned();
    }
    tags.iter()
        .map(|(key, values)| {
            let values: Vec<&str> = values.iter().map(String::as_str).collect();
            format!("{key}: {}", values.join(", "))
        })
        .collect::<Vec<_>>()
        .join("; ")
}

/// The rows of the edge table.
fn edge_rows(view: &View<'_>) -> String {
    view.edges
        .iter()
        .map(|edge| {
            let sites = if edge.count == 0 {
                "none".to_owned()
            } else {
                edge.count.to_string()
            };
            format!(
                "<tr><td class=\"path\">{}</td><td>{}</td><td class=\"path\">{}</td>\
                 <td>{}</td><td>{sites}</td></tr>\n",
                escape(edge.source.as_str()),
                edge.relation.as_str(),
                escape(edge.target.as_str()),
                edge.confidence.map_or("none", Confidence::as_str),
            )
        })
        .collect::<Vec<String>>()
        .concat()
}

/// `text` for an HTML text node or a double-quoted attribute.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// JSON text for a `<script type="application/json">` block. Outside strings
/// JSON has none of these characters, so the replacement (a JSON escape that
/// reads back as the same character) only changes string contents, and
/// `</script>`, `<!--` and the character references cannot form.
fn script_safe(json: &str) -> String {
    let mut out = String::with_capacity(json.len());
    for c in json.chars() {
        match c {
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            _ => out.push(c),
        }
    }
    out
}

/// `template` with each `@@name@@` replaced by its value, in one pass, so a
/// value that contains `@@name@@` is left alone.
fn fill(template: &str, values: &[(&str, String)]) -> String {
    let mut out = String::with_capacity(template.len() + 8192);
    let mut rest = template;
    while let Some((before, after)) = rest.split_once("@@") {
        out.push_str(before);
        let (name, tail) = after.split_once("@@").unwrap_or_default();
        out.push_str(
            values
                .iter()
                .find(|(key, _)| *key == name)
                .map_or("", |(_, value)| value.as_str()),
        );
        rest = tail;
    }
    out.push_str(rest);
    out
}

/// The round constants of SHA-256.
const K: [u32; 64] = [
    0x428a_2f98,
    0x7137_4491,
    0xb5c0_fbcf,
    0xe9b5_dba5,
    0x3956_c25b,
    0x59f1_11f1,
    0x923f_82a4,
    0xab1c_5ed5,
    0xd807_aa98,
    0x1283_5b01,
    0x2431_85be,
    0x550c_7dc3,
    0x72be_5d74,
    0x80de_b1fe,
    0x9bdc_06a7,
    0xc19b_f174,
    0xe49b_69c1,
    0xefbe_4786,
    0x0fc1_9dc6,
    0x240c_a1cc,
    0x2de9_2c6f,
    0x4a74_84aa,
    0x5cb0_a9dc,
    0x76f9_88da,
    0x983e_5152,
    0xa831_c66d,
    0xb003_27c8,
    0xbf59_7fc7,
    0xc6e0_0bf3,
    0xd5a7_9147,
    0x06ca_6351,
    0x1429_2967,
    0x27b7_0a85,
    0x2e1b_2138,
    0x4d2c_6dfc,
    0x5338_0d13,
    0x650a_7354,
    0x766a_0abb,
    0x81c2_c92e,
    0x9272_2c85,
    0xa2bf_e8a1,
    0xa81a_664b,
    0xc24b_8b70,
    0xc76c_51a3,
    0xd192_e819,
    0xd699_0624,
    0xf40e_3585,
    0x106a_a070,
    0x19a4_c116,
    0x1e37_6c08,
    0x2748_774c,
    0x34b0_bcb5,
    0x391c_0cb3,
    0x4ed8_aa4a,
    0x5b9c_ca4f,
    0x682e_6ff3,
    0x748f_82ee,
    0x78a5_636f,
    0x84c8_7814,
    0x8cc7_0208,
    0x90be_fffa,
    0xa450_6ceb,
    0xbef9_a3f7,
    0xc671_78f2,
];

/// The SHA-256 digest (FIPS 180-4), for the policy's script and style hashes.
fn sha256(data: &[u8]) -> [u8; 32] {
    let mut state: [u32; 8] = [
        0x6a09_e667,
        0xbb67_ae85,
        0x3c6e_f372,
        0xa54f_f53a,
        0x510e_527f,
        0x9b05_688c,
        0x1f83_d9ab,
        0x5be0_cd19,
    ];
    let mut message = data.to_vec();
    message.push(0x80);
    // Zeros up to 56 bytes into a block, then the bit length.
    message.resize(message.len() + (119 - data.len() % 64) % 64, 0);
    let bits = u64::try_from(data.len()).unwrap_or_default() * 8;
    message.extend_from_slice(&bits.to_be_bytes());
    for block in message.as_chunks::<64>().0 {
        compress(&mut state, block);
    }
    let mut digest = [0_u8; 32];
    for (bytes, word) in digest.as_chunks_mut::<4>().0.iter_mut().zip(state) {
        *bytes = word.to_be_bytes();
    }
    digest
}

/// One block of SHA-256 into `state`.
fn compress(state: &mut [u32; 8], block: &[u8; 64]) {
    let mut words = [0_u32; 64];
    for (word, bytes) in words.iter_mut().zip(block.as_chunks::<4>().0) {
        *word = u32::from_be_bytes(*bytes);
    }
    for at in 16..64 {
        let near = words[at - 15];
        let far = words[at - 2];
        let low = near.rotate_right(7) ^ near.rotate_right(18) ^ (near >> 3);
        let high = far.rotate_right(17) ^ far.rotate_right(19) ^ (far >> 10);
        words[at] = words[at - 16]
            .wrapping_add(low)
            .wrapping_add(words[at - 7])
            .wrapping_add(high);
    }
    // The working variables a to h of the specification, in order.
    let mut v = *state;
    for (constant, word) in K.iter().zip(words) {
        let e_mix = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
        let choose = (v[4] & v[5]) ^ (!v[4] & v[6]);
        let first = v[7]
            .wrapping_add(e_mix)
            .wrapping_add(choose)
            .wrapping_add(*constant)
            .wrapping_add(word);
        let a_mixture = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
        let majority = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
        let second = a_mixture.wrapping_add(majority);
        v = [
            first.wrapping_add(second),
            v[0],
            v[1],
            v[2],
            v[3].wrapping_add(first),
            v[4],
            v[5],
            v[6],
        ];
    }
    for (slot, value) in state.iter_mut().zip(v) {
        *slot = slot.wrapping_add(value);
    }
}

/// Standard base64 with padding, as the policy's hash sources use.
fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let mut group = 0_u32;
        for (at, byte) in chunk.iter().enumerate() {
            group |= u32::from(*byte) << (16 - 8 * at);
        }
        for at in 0..4 {
            if at <= chunk.len() {
                let index = usize::try_from((group >> (18 - 6 * at)) & 63).unwrap_or_default();
                out.push(char::from(ALPHABET[index]));
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::{base64, sha256};

    /// Lower-case hex of a digest.
    fn hex(bytes: &[u8]) -> String {
        bytes.iter().fold(String::new(), |mut out, byte| {
            let _ = write!(out, "{byte:02x}");
            out
        })
    }

    #[test]
    fn sha256_matches_the_nist_short_message_vectors() {
        let cases: [(&str, &str); 3] = [
            (
                "",
                "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
            ),
            (
                "abc",
                "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            ),
            (
                "abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq",
                "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1",
            ),
        ];
        for (message, digest) in cases {
            assert_eq!(hex(&sha256(message.as_bytes())), digest, "{message:?}");
        }
    }

    #[test]
    fn base64_matches_the_rfc_4648_section_10_vectors() {
        let cases = [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ];
        for (message, encoded) in cases {
            assert_eq!(base64(message.as_bytes()), encoded, "{message:?}");
        }
    }
}
