//! Output of the dependency graph (ADR 0004, "Traceability outputs"): the
//! [`Renderer`] trait that every document format implements, the [`Json`]
//! graph document, the other formats ([`Dot`], [`Mermaid`], [`GraphMl`],
//! [`Html`]) and the [`View`] that the diagram formats draw.
//!
//! Everything here is a pure function from a [`Graph`] to text; the command
//! line decides where the text goes.

mod dot;
mod graphml;
mod html;
mod json;
mod mermaid;
mod view;

pub use self::dot::Dot;
pub use self::graphml::GraphMl;
pub use self::html::Html;
pub use self::json::{Json, SCHEMA_VERSION};
pub use self::mermaid::Mermaid;
pub use self::view::{Cluster, Level, View, ViewEdge, incident};

use crate::graph::Graph;

/// A document format.
///
/// A renderer receives the whole graph and the view of it that the user asked
/// for. The data formats (JSON, `GraphML`) write the graph and ignore the view,
/// so they always carry everything; the diagram formats (DOT, Mermaid, HTML)
/// draw the view.
pub trait Renderer {
    /// The document, ending in a newline.
    fn render(&self, graph: &Graph, view: &View<'_>) -> String;
}
