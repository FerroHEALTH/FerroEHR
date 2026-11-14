// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The element tree the BPPC and APPC documents are read into and written
//! from: element names without their namespace prefix, attributes by local
//! name, and text.
//!
//! A document type declaration is refused, since neither CDA R2 nor XACML
//! 2.0 needs one and a reader that resolved one would read entities the
//! document never stated. No specification governs this reader: our own
//! design.

use std::fmt::Write as _;

use quick_xml::Reader;
use quick_xml::events::{BytesStart, Event};

/// How deeply a document may nest.
const MAX_DEPTH: usize = 64;

/// One element.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Node {
    /// The local name.
    pub(crate) name: String,
    /// The attributes, by local name, in document order.
    pub(crate) attributes: Vec<(String, String)>,
    /// The child elements, in document order.
    pub(crate) children: Vec<Self>,
    /// The text directly inside, trimmed.
    pub(crate) text: String,
}

impl Node {
    /// An element named `name` with nothing in it.
    #[must_use]
    pub(crate) fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            ..Self::default()
        }
    }

    /// The element with `key` set to `value`.
    #[must_use]
    pub(crate) fn attr(mut self, key: &str, value: &str) -> Self {
        self.attributes.push((key.to_owned(), value.to_owned()));
        self
    }

    /// The element with `child` appended.
    #[must_use]
    pub(crate) fn child(mut self, child: Self) -> Self {
        self.children.push(child);
        self
    }

    /// The element with `text` inside.
    #[must_use]
    pub(crate) fn with_text(mut self, text: &str) -> Self {
        text.clone_into(&mut self.text);
        self
    }

    /// The value of the attribute `key`.
    #[must_use]
    pub(crate) fn get(&self, key: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
    }

    /// The child elements named `name`.
    pub(crate) fn all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Self> {
        self.children.iter().filter(move |c| c.name == name)
    }

    /// The first child element named `name`.
    #[must_use]
    pub(crate) fn first(&self, name: &str) -> Option<&Self> {
        self.children.iter().find(|c| c.name == name)
    }

    /// The document `self` is the root of, with an XML declaration.
    #[must_use]
    pub(crate) fn document(&self) -> String {
        let mut out = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        self.write(&mut out, 0);
        out
    }

    fn write(&self, out: &mut String, depth: usize) {
        let indent = "  ".repeat(depth);
        let _ = write!(out, "{indent}<{}", self.name);
        for (key, value) in &self.attributes {
            let _ = write!(
                out,
                " {key}=\"{}\"",
                quick_xml::escape::escape(value.as_str())
            );
        }
        if self.children.is_empty() && self.text.is_empty() {
            out.push_str("/>\n");
            return;
        }
        out.push('>');
        if self.children.is_empty() {
            out.push_str(&quick_xml::escape::escape(self.text.as_str()));
        } else {
            out.push('\n');
            if !self.text.is_empty() {
                let _ = writeln!(
                    out,
                    "{indent}  {}",
                    quick_xml::escape::escape(self.text.as_str())
                );
            }
            for child in &self.children {
                child.write(out, depth.saturating_add(1));
            }
            out.push_str(&indent);
        }
        let _ = writeln!(out, "</{}>", self.name);
    }
}

/// Why a document could not be read.
#[derive(Debug, thiserror::Error)]
pub(crate) enum XmlError {
    /// The parser refused the document.
    #[error("the document is no well-formed XML")]
    Parse(#[source] quick_xml::Error),
    /// An attribute could not be read.
    #[error("an attribute is no well-formed XML")]
    Attribute(#[source] quick_xml::events::attributes::AttrError),
    /// An escape could not be resolved.
    #[error("an escape is no well-formed XML")]
    Escape(#[source] quick_xml::escape::EscapeError),
    /// The document is not one element tree.
    #[error("{0}")]
    Shape(String),
}

/// Reads `text` as one element tree.
///
/// # Errors
/// [`XmlError`] for text that is no well-formed XML, a document type
/// declaration, a tree deeper than 64 levels, or anything but one root.
pub(crate) fn parse(text: &str) -> Result<Node, XmlError> {
    let mut reader = Reader::from_str(text);
    let mut stack: Vec<Node> = Vec::new();
    let mut root = None;
    loop {
        match reader.read_event().map_err(XmlError::Parse)? {
            Event::Start(start) => {
                if stack.len() >= MAX_DEPTH {
                    return Err(XmlError::Shape("the document nests too deeply".to_owned()));
                }
                stack.push(node(&start)?);
            }
            Event::Empty(start) => {
                let leaf = node(&start)?;
                attach(&mut stack, &mut root, leaf)?;
            }
            Event::End(_) => {
                let Some(mut done) = stack.pop() else {
                    return Err(XmlError::Shape("an end tag closes nothing".to_owned()));
                };
                let trimmed = done.text.trim().to_owned();
                done.text = trimmed;
                attach(&mut stack, &mut root, done)?;
            }
            Event::Text(text) => {
                let text = quick_xml::escape::unescape(text.as_ref()).map_err(XmlError::Escape)?;
                if let Some(open) = stack.last_mut() {
                    open.text.push_str(&text);
                }
            }
            Event::CData(data) => {
                if let Some(open) = stack.last_mut() {
                    open.text.push_str(data.as_ref());
                }
            }
            Event::GeneralRef(reference) => {
                let resolved =
                    if let Some(c) = reference.resolve_char_ref().map_err(XmlError::Parse)? {
                        c.to_string()
                    } else {
                        let name: &str = reference.as_ref();
                        quick_xml::escape::unescape(&format!("&{name};"))
                            .map_err(XmlError::Escape)?
                            .into_owned()
                    };
                if let Some(open) = stack.last_mut() {
                    open.text.push_str(&resolved);
                }
            }
            Event::DocType(_) => {
                return Err(XmlError::Shape(
                    "a document type declaration is not accepted".to_owned(),
                ));
            }
            Event::Eof => break,
            Event::Decl(_) | Event::Comment(_) | Event::PI(_) => {}
        }
    }
    if !stack.is_empty() {
        return Err(XmlError::Shape(
            "the document ends inside an element".to_owned(),
        ));
    }
    root.ok_or_else(|| XmlError::Shape("the document holds no element".to_owned()))
}

/// The element `start` opens.
fn node(start: &BytesStart<'_>) -> Result<Node, XmlError> {
    let mut out = Node::new(local(start.name().as_ref()));
    for attribute in start.attributes() {
        let attribute = attribute.map_err(XmlError::Attribute)?;
        let key = attribute.key.as_ref();
        if key == "xmlns" || key.starts_with("xmlns:") {
            continue;
        }
        let value =
            quick_xml::escape::unescape(attribute.value.as_ref()).map_err(XmlError::Escape)?;
        out.attributes
            .push((local(key).to_owned(), value.into_owned()));
    }
    Ok(out)
}

/// `name` without its namespace prefix.
fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

/// Appends `done` to the element it sits in, or makes it the root.
fn attach(stack: &mut [Node], root: &mut Option<Node>, done: Node) -> Result<(), XmlError> {
    match stack.last_mut() {
        Some(parent) => parent.children.push(done),
        None if root.is_none() => *root = Some(done),
        None => {
            return Err(XmlError::Shape(
                "the document holds more than one root".to_owned(),
            ));
        }
    }
    Ok(())
}
