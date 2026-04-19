//! XML deserialization of request bodies.
//!
//! Parses incoming XML into a generic tree representation that callers then
//! map to domain types. This avoids a separate serde path and matches the
//! trim-whitespace semantics AWS applies to every element.

use quick_xml::{Reader, events::Event};

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
    reader.config_mut().trim_text(true);
    reader.config_mut().check_end_names = false;

    let mut stack: Vec<Node> = Vec::with_capacity(16);
    let mut root: Option<Node> = None;

    loop {
        match reader.read_event() {
            Ok(Event::Start(e)) => {
                let name = std::str::from_utf8(e.name().as_ref())
                    .map_err(|err| format!("invalid utf8 in name: {err}"))?
                    .to_owned();
                stack.push(Node {
                    name,
                    ..Node::default()
                });
            }
            Ok(Event::End(_)) => {
                if let Some(done) = stack.pop() {
                    if let Some(parent) = stack.last_mut() {
                        parent.children.push(done);
                    } else {
                        root = Some(done);
                    }
                }
            }
            Ok(Event::Empty(e)) => {
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
                    top.text.push_str(&unescape_xml(raw));
                }
            }
            Ok(Event::CData(c)) => {
                if let Some(top) = stack.last_mut() {
                    let s = std::str::from_utf8(&c).map_err(|e| format!("cdata utf8: {e}"))?;
                    top.text.push_str(s);
                }
            }
            Ok(Event::Eof) => break,
            Ok(_) => {}
            Err(e) => return Err(format!("xml parse error: {e}")),
        }
    }

    root.ok_or_else(|| "empty XML document".to_owned())
}

/// Minimal XML entity unescaping (covers the five core entities plus numeric).
fn unescape_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(idx) = rest.find('&') {
        out.push_str(&rest[..idx]);
        let remainder = &rest[idx..];
        if let Some(end) = remainder.find(';') {
            let entity = &remainder[1..end];
            let replaced = match entity {
                "amp" => "&".to_owned(),
                "lt" => "<".to_owned(),
                "gt" => ">".to_owned(),
                "quot" => "\"".to_owned(),
                "apos" => "'".to_owned(),
                hash if hash.starts_with('#') => {
                    let (radix, num) = if hash.starts_with("#x") || hash.starts_with("#X") {
                        (16, &hash[2..])
                    } else {
                        (10, &hash[1..])
                    };
                    if let Ok(n) = u32::from_str_radix(num, radix) {
                        char::from_u32(n)
                            .map(|c| c.to_string())
                            .unwrap_or_else(|| format!("&{entity};"))
                    } else {
                        format!("&{entity};")
                    }
                }
                _ => format!("&{entity};"),
            };
            out.push_str(&replaced);
            rest = &remainder[end + 1..];
        } else {
            out.push_str(remainder);
            rest = "";
        }
    }
    out.push_str(rest);
    out
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
}
