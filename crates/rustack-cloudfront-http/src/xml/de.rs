//! XML deserialization of request bodies.
//!
//! Parses incoming XML into a generic tree representation that callers then
//! map to domain types. This avoids a separate serde path and matches the
//! trim-whitespace semantics AWS applies to every element.

use quick_xml::{Reader, escape::resolve_xml_entity, events::Event};

/// A lightweight XML tree node.
#[derive(Debug, Default, Clone)]
pub struct Node {
    /// Element name.
    pub name: String,
    /// Direct text content (trimmed).
    pub text: String,
    /// Child nodes in document order.
    pub children: Vec<Node>,
}

impl Node {
    /// Find the first direct child element with the given name.
    #[must_use]
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }

    /// Collect children with the given name.
    pub fn children_named<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a Node> + 'a {
        self.children.iter().filter(move |c| c.name == name)
    }

    /// Text content of the named child, default empty.
    #[must_use]
    pub fn child_text<'a>(&'a self, name: &str) -> &'a str {
        self.child(name).map_or("", |c| c.text.as_str())
    }

    /// Parse text of named child as integer.
    #[must_use]
    pub fn child_i32(&self, name: &str) -> i32 {
        self.child_text(name).parse().unwrap_or(0)
    }

    /// Parse text of named child as i64.
    #[must_use]
    pub fn child_i64(&self, name: &str) -> i64 {
        self.child_text(name).parse().unwrap_or(0)
    }

    /// Parse text of named child as boolean (`true` / `false`).
    #[must_use]
    pub fn child_bool(&self, name: &str) -> bool {
        matches!(self.child_text(name).trim(), "true" | "True" | "TRUE" | "1")
    }

    /// Parse a `<Wrapper><Quantity/><Items>...</Items></Wrapper>` list shape.
    #[must_use]
    pub fn items_named<'a>(&'a self, wrapper: &'a str, item_name: &'a str) -> Vec<&'a Node> {
        self.child(wrapper)
            .and_then(|w| w.child("Items"))
            .map(|items| items.children_named(item_name).collect())
            .unwrap_or_default()
    }

    /// Parse a `<Wrapper><Items>...</Items></Wrapper>` list of string contents.
    #[must_use]
    pub fn string_items(&self, wrapper: &str, item_name: &str) -> Vec<String> {
        self.items_named(wrapper, item_name)
            .into_iter()
            .map(|n| n.text.clone())
            .collect()
    }

    /// Parse a `<Items>...<Item/>...</Items>` list where `Items` is a *direct* child.
    ///
    /// Use this when you have already descended into the wrapper node and the
    /// current node looks like `<Wrapper><Quantity/><Items><Item/></Items></Wrapper>`.
    #[must_use]
    pub fn direct_items<'a>(&'a self, item_name: &'a str) -> Vec<&'a Node> {
        self.child("Items")
            .map(|items| items.children_named(item_name).collect())
            .unwrap_or_default()
    }

    /// String variant of `direct_items`: returns the text of every matching item.
    #[must_use]
    pub fn direct_string_items(&self, item_name: &str) -> Vec<String> {
        self.direct_items(item_name)
            .into_iter()
            .map(|n| n.text.clone())
            .collect()
    }
}

/// Parse an XML document into a tree rooted at the first element.
///
/// # Errors
/// Returns a string error on malformed XML.
pub fn parse(xml: &[u8]) -> Result<Node, String> {
    if xml.is_empty() {
        return Ok(Node::default());
    }
    let mut reader = Reader::from_reader(xml);

    let mut stack: Vec<Node> = Vec::with_capacity(16);
    let mut root: Option<Node> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                if stack.is_empty() && root.is_some() {
                    return Err("multiple XML root elements".into());
                }
                let name = std::str::from_utf8(e.name().as_ref())
                    .map_err(|err| format!("invalid utf8 in name: {err}"))?
                    .to_owned();
                stack.push(Node {
                    name,
                    ..Node::default()
                });
            }
            Ok(Event::End(_)) => {
                if let Some(mut done) = stack.pop() {
                    // References and CDATA split text into multiple events;
                    // trim only after reassembling the complete element value.
                    let trimmed = done.text.trim_matches([' ', '\t', '\r', '\n']);
                    if trimmed.len() != done.text.len() {
                        done.text = trimmed.to_owned();
                    }
                    if let Some(parent) = stack.last_mut() {
                        parent.children.push(done);
                    } else {
                        root = Some(done);
                    }
                }
            }
            Ok(Event::Empty(e)) => {
                if stack.is_empty() && root.is_some() {
                    return Err("multiple XML root elements".into());
                }
                let name = std::str::from_utf8(e.name().as_ref())
                    .map_err(|err| format!("invalid utf8 in name: {err}"))?
                    .to_owned();
                let node = Node {
                    name,
                    ..Node::default()
                };
                if let Some(parent) = stack.last_mut() {
                    parent.children.push(node);
                } else {
                    root = Some(node);
                }
            }
            Ok(Event::Text(t)) => {
                if let Some(top) = stack.last_mut() {
                    let raw =
                        std::str::from_utf8(t.as_ref()).map_err(|e| format!("text utf8: {e}"))?;
                    top.text.push_str(raw);
                }
            }
            Ok(Event::GeneralRef(reference)) => {
                let character = reference
                    .resolve_char_ref()
                    .map_err(|err| format!("invalid character reference: {err}"))?;
                if let Some(character) = character {
                    if !is_xml_character(character) {
                        return Err("character reference is not a valid XML character".into());
                    }
                    if let Some(top) = stack.last_mut() {
                        top.text.push(character);
                    }
                } else {
                    let name = reference
                        .decode()
                        .map_err(|err| format!("entity utf8: {err}"))?;
                    let value = resolve_xml_entity(&name)
                        .ok_or_else(|| "unsupported XML entity".to_owned())?;
                    if let Some(top) = stack.last_mut() {
                        top.text.push_str(value);
                    }
                }
            }
            Ok(Event::DocType(_)) => return Err("XML document types are not supported".into()),
            Ok(Event::CData(c)) => {
                if let Some(top) = stack.last_mut() {
                    let s = std::str::from_utf8(&c).map_err(|e| format!("cdata utf8: {e}"))?;
                    top.text.push_str(s);
                }
            }
            Ok(Event::Eof) => {
                if !stack.is_empty() {
                    return Err("unclosed XML element".into());
                }
                break;
            }
            Ok(_) => {}
            Err(e) => return Err(format!("xml parse error: {e}")),
        }
    }

    root.ok_or_else(|| "empty XML document".to_owned())
}

/// The XML 1.0 `Char` production excludes controls, surrogates and U+FFFE/U+FFFF.
fn is_xml_character(character: char) -> bool {
    matches!(
        character,
        '\u{9}' | '\u{A}' | '\u{D}'
            | '\u{20}'..='\u{D7FF}'
            | '\u{E000}'..='\u{FFFD}'
            | '\u{10000}'..='\u{10FFFF}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parses_basic() {
        let xml = b"<Foo><Bar>hello</Bar></Foo>";
        let n = parse(xml).unwrap();
        assert_eq!(n.name, "Foo");
        assert_eq!(n.child_text("Bar"), "hello");
    }

    #[test]
    fn test_parses_items() {
        let xml = b"<Foo><Items><Item>a</Item><Item>b</Item></Items></Foo>";
        let n = parse(xml).unwrap();
        let names: Vec<_> = n
            .children_named("Items")
            .next()
            .unwrap()
            .children_named("Item")
            .map(|x| x.text.as_str())
            .collect();
        assert_eq!(names, vec!["a", "b"]);
    }

    #[test]
    fn test_should_decode_predefined_and_numeric_entities_once() {
        let node = parse(b"<Name>&lt;&gt;&quot;&apos;&amp;&#38;&#x26;&amp;amp;</Name>").unwrap();
        assert_eq!(node.text, "<>\"'&&&&amp;");
    }

    #[test]
    fn test_should_preserve_text_order_and_spaces_across_entities_and_cdata() {
        let node = parse(b"<Name>  a &amp; <![CDATA[b&c]]> &#38; d  </Name>").unwrap();
        assert_eq!(node.text, "a & b&c & d");
    }

    #[test]
    fn test_should_preserve_non_xml_unicode_whitespace_in_names() {
        let node = parse("<Name>\u{a0}my-config\u{a0}</Name>".as_bytes()).unwrap();
        assert_eq!(node.text, "\u{a0}my-config\u{a0}");
    }

    #[test]
    fn test_should_reject_unknown_entities_document_types_and_invalid_character_references() {
        for xml in [
            "<Name>a&unknown;b</Name>",
            "<!DOCTYPE Name [<!ENTITY name 'value'>]><Name>&name;</Name>",
            "<!DOCTYPE Name SYSTEM 'file:///etc/passwd'><Name>value</Name>",
            "<Name>&#0;</Name>",
            "<Name>&#1;</Name>",
            "<Name>&#xD800;</Name>",
            "<Name>&#xFFFE;</Name>",
            "<Name>&#x110000;</Name>",
            "<Name>&#not-a-number;</Name>",
        ] {
            assert!(parse(xml.as_bytes()).is_err(), "{xml}");
        }
    }

    #[test]
    fn test_should_reject_mismatched_unclosed_and_multiple_root_elements() {
        for xml in [
            "<Request><Name>config</Other></Request>",
            "<Request><Name>config</Name>",
            "<Request><Name>config",
            "<Request></Request><Other/>",
            "<Request/><Other></Other>",
        ] {
            assert!(parse(xml.as_bytes()).is_err(), "{xml}");
        }
    }
}
