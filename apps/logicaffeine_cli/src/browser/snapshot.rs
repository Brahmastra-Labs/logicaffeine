//! A page's observable state at one moment: the document as a tree, its title, its URL, and
//! which element has focus.

use serde::{Deserialize, Serialize};

/// One node of a snapshot document. An element carries its attributes sorted by name, and the
/// two control PROPERTIES a visitor moves — `value` and `checked` — only where they differ from
/// what the markup alone says (an untouched `<input value="x">` has no `value` entry; a typed-into
/// one does).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DomNode {
    Text {
        text: String,
    },
    Element {
        tag: String,
        attrs: Vec<(String, String)>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        checked: Option<bool>,
        children: Vec<DomNode>,
    },
}

/// The page at one moment, as the browser holds it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub url: String,
    pub title: String,
    /// `/html[1]/body[1]/main[1]/input[2]` — the focused element's path, `None` when focus is on
    /// the document itself.
    pub focus: Option<String>,
    pub document: DomNode,
}

impl DomNode {
    fn is_formatting_whitespace(&self) -> bool {
        matches!(self, DomNode::Text { text } if text.chars().all(char::is_whitespace))
    }

    /// The element's children, or nothing for a text node.
    pub fn children(&self) -> &[DomNode] {
        match self {
            DomNode::Element { children, .. } => children,
            DomNode::Text { .. } => &[],
        }
    }

    /// The element's tag, or `None` for a text node.
    pub fn tag(&self) -> Option<&str> {
        match self {
            DomNode::Element { tag, .. } => Some(tag),
            DomNode::Text { .. } => None,
        }
    }

    /// Everything a reader sees in this subtree, concatenated.
    pub fn text_content(&self) -> String {
        match self {
            DomNode::Text { text } => text.clone(),
            DomNode::Element { children, .. } => {
                children.iter().map(DomNode::text_content).collect()
            }
        }
    }

    /// This subtree in the reference renderer's serialization (`semantics::dom::render`):
    /// attributes in name order, then ` checked` and ` value="…"`, void elements unclosed,
    /// raw-text elements verbatim, everything else escaped by the same functions.
    pub fn markup(&self) -> String {
        let mut out = String::new();
        self.write_markup(&mut out);
        out
    }

    fn write_markup(&self, out: &mut String) {
        use logicaffeine_language::lgx::html::{escape_attr, escape_text, is_raw_text, is_void};
        match self {
            DomNode::Text { text } => out.push_str(&escape_text(text)),
            DomNode::Element {
                tag,
                attrs,
                value,
                checked,
                children,
            } => {
                out.push('<');
                out.push_str(tag);
                for (k, v) in attrs {
                    out.push_str(&format!(" {k}=\"{}\"", escape_attr(v)));
                }
                if *checked == Some(true) {
                    out.push_str(" checked");
                }
                if let Some(v) = value {
                    out.push_str(&format!(" value=\"{}\"", escape_attr(v)));
                }
                out.push('>');
                if is_void(tag) {
                    return;
                }
                for c in children {
                    match (is_raw_text(tag), c) {
                        (true, DomNode::Text { text }) => out.push_str(text),
                        _ => c.write_markup(out),
                    }
                }
                out.push_str(&format!("</{tag}>"));
            }
        }
    }
}

impl Snapshot {
    /// The `<body>` element.
    pub fn body(&self) -> Option<&DomNode> {
        self.document
            .children()
            .iter()
            .find(|c| c.tag() == Some("body"))
    }

    /// The `<head>` element.
    pub fn head(&self) -> Option<&DomNode> {
        self.document
            .children()
            .iter()
            .find(|c| c.tag() == Some("head"))
    }

    /// The body in the reference renderer's serialization, so it compares byte for byte with
    /// `semantics::dom::render()`.
    ///
    /// Whitespace-only text directly under `<body>` is the document's own formatting — the line
    /// breaks around the markup a shell or a pre-render writes — and never page content, so it is
    /// left out. Every other text node is kept exactly.
    pub fn body_markup(&self) -> String {
        let Some(DomNode::Element {
            attrs,
            value,
            checked,
            children,
            ..
        }) = self.body()
        else {
            return String::new();
        };
        let content: Vec<DomNode> = children
            .iter()
            .filter(|c| !c.is_formatting_whitespace())
            .cloned()
            .collect();
        DomNode::Element {
            tag: "body".into(),
            attrs: attrs.clone(),
            value: value.clone(),
            checked: *checked,
            children: content,
        }
        .markup()
    }

    /// The head's elements in the reference renderer's serialization, in document order — the
    /// whitespace text between them is the document's own formatting and is left out, as
    /// [`Snapshot::body_markup`] leaves it out under `<body>`. `None` when the head holds no element.
    pub fn head_markup(&self) -> Option<String> {
        self.head_markup_without_stylesheets(&[])
    }

    /// [`Snapshot::head_markup`] without the `<link rel="stylesheet">` elements for `hrefs` — the
    /// stylesheets a serving shell links for its project, which no page runtime on any tier
    /// produces, so they are not the page's head.
    pub fn head_markup_without_stylesheets(&self, hrefs: &[String]) -> Option<String> {
        let shell = |c: &DomNode| match c {
            DomNode::Element { tag, attrs, .. } if tag == "link" => {
                let attr = |name: &str| {
                    attrs
                        .iter()
                        .find(|(k, _)| k == name)
                        .map(|(_, v)| v.as_str())
                };
                attr("rel") == Some("stylesheet")
                    && attr("href").is_some_and(|h| hrefs.iter().any(|s| s == h))
            }
            _ => false,
        };
        let markup: String = self
            .head()?
            .children()
            .iter()
            .filter(|c| c.tag().is_some() && !shell(c))
            .map(DomNode::markup)
            .collect();
        (!markup.is_empty()).then_some(markup)
    }

    /// Every element in document order whose `id` attribute is `id`.
    pub fn by_id(&self, id: &str) -> Vec<&DomNode> {
        fn walk<'s>(n: &'s DomNode, id: &str, out: &mut Vec<&'s DomNode>) {
            if let DomNode::Element {
                attrs, children, ..
            } = n
            {
                if attrs.iter().any(|(k, v)| k == "id" && v == id) {
                    out.push(n);
                }
                for c in children {
                    walk(c, id, out);
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.document, id, &mut out);
        out
    }

    /// The snapshot as canonical JSON.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("a snapshot serializes")
    }
}
