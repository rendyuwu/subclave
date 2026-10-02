//! The XML scan the two listing parsers share.
//!
//! Both bodies are read with `quick-xml`, whose event stream is walked once
//! into a small tree, so a lookup by element name can be made from anywhere in
//! the document. The plumbing is protocol-agnostic: the two providers differ in
//! the element names they look up, not in how the document is walked, so it
//! lives here rather than once per backend.
//! `src-tauri/src/modules/sync/providers/http.rs` is the same move for the HTTP
//! shell.
//!
//! IT MATCHES ON THE LOCAL NAME, so the server's choice of namespace prefix
//! does not reach a caller. The ceiling that buys, named: an element from some
//! other namespace sharing a local name would be read as well, and closing
//! that means resolving namespaces, which a listing from either service has no
//! use for.

use quick_xml::events::Event;
use quick_xml::name::QName;
use quick_xml::Reader;

use crate::modules::sync::provider::ProviderError;

/// The local name of an element: everything after the last `:` of its
/// qualified name.
///
/// Cannot fail: the reader borrows the document from a `&str`, so every name it
/// reports is UTF-8.
fn local_name<'a>(name: &QName<'a>) -> &'a str {
    std::str::from_utf8(name.local_name().into_inner())
        .expect("the reader reported a non-UTF-8 name")
}

/// A parse failure of any kind, in the taxonomy's own terms: the body came from
/// the remote and a listing this client cannot read is not an empty listing.
fn malformed<E: std::fmt::Display>(e: E) -> ProviderError {
    ProviderError::Malformed(format!("the listing did not parse: {e}"))
}

/// One element of a body, with the text inside it and its children.
///
/// A self-closing element yields the empty string rather than being skipped, so
/// a property a server declined to supply is visible as empty rather than as
/// absent, which is the distinction the DAV status gate is built on.
pub(super) struct Element {
    /// The LOCAL name, so the server's choice of prefix reaches no caller.
    pub(super) name: String,
    /// The character data between this element's tags, with the predefined
    /// entities and any character references resolved. Nested elements are in
    /// `children` and their text is NOT folded in here.
    pub(super) text: String,
    children: Vec<Element>,
}

impl Element {
    fn new(name: &str) -> Self {
        Element {
            name: name.to_string(),
            text: String::new(),
            children: Vec::new(),
        }
    }

    /// The first element with this local name, at or below this one.
    pub(super) fn find(&self, name: &str) -> Option<&Element> {
        if self.name == name {
            return Some(self);
        }
        self.children.iter().find_map(|child| child.find(name))
    }

    /// Every element with this local name below this one, in document order.
    pub(super) fn all(&self, name: &str) -> Vec<&Element> {
        let mut out = Vec::new();
        self.collect(name, &mut out);
        out
    }

    fn collect<'a>(&'a self, name: &str, out: &mut Vec<&'a Element>) {
        for child in &self.children {
            if child.name == name {
                out.push(child);
            }
            child.collect(name, out);
        }
    }
}

/// Walk `body`'s event stream into one element per node.
///
/// The text of an element is the text of the events between its tags, so a
/// self-closing element and `<x></x>` both come out empty, and text after a
/// nested element belongs to the nested one.
pub(super) fn tree(body: &str) -> Result<Vec<Element>, ProviderError> {
    let mut reader = Reader::from_str(body);
    // THE PARITY FIX: the hand scanners this replaced scanned text and could
    // not fail on well-formedness, so a body with a dangling `&`, a mismatched
    // end tag or a stray end tag still yielded rows. `quick-xml`'s defaults
    // refuse all three, which turns one bad byte in a listing into a failed
    // pull for the whole prefix, so those three checks are loosened and every
    // other setting keeps its default.
    let config = reader.config_mut();
    config.allow_dangling_amp = true;
    config.allow_unmatched_ends = true;
    config.check_end_names = false;

    let mut roots = Vec::new();
    let mut open: Vec<Element> = Vec::new();
    loop {
        match reader.read_event().map_err(malformed)? {
            Event::Start(e) => open.push(Element::new(local_name(&e.name()))),
            Event::Empty(e) => {
                let element = Element::new(local_name(&e.name()));
                attach(&mut open, &mut roots, element);
            }
            Event::End(_) => {
                if let Some(element) = open.pop() {
                    attach(&mut open, &mut roots, element);
                }
            }
            Event::Text(t) => {
                if let Some(element) = open.last_mut() {
                    element.text.push_str(&t.decode().map_err(malformed)?);
                }
            }
            Event::GeneralRef(r) => {
                if let Some(element) = open.last_mut() {
                    // A character reference resolves on its own; the five names
                    // XML defines without a DTD are spelled out; an entity no
                    // DTD defines is left as it was written rather than
                    // refused.
                    match r.resolve_char_ref().map_err(malformed)? {
                        Some(c) => element.text.push(c),
                        None => {
                            let name = r.decode().map_err(malformed)?;
                            match name.as_ref() {
                                "amp" => element.text.push('&'),
                                "lt" => element.text.push('<'),
                                "gt" => element.text.push('>'),
                                "quot" => element.text.push('"'),
                                "apos" => element.text.push('\''),
                                other => {
                                    element.text.push('&');
                                    element.text.push_str(other);
                                    element.text.push(';');
                                }
                            }
                        }
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(roots)
}

/// A finished element lands in its parent, or among the roots when there is no
/// open parent left.
fn attach(open: &mut [Element], roots: &mut Vec<Element>, element: Element) {
    match open.last_mut() {
        Some(parent) => parent.children.push(element),
        None => roots.push(element),
    }
}
