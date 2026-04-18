//! Minimal XML writer — linear, allocation-aware, no external schema.

/// Sequential XML writer used to emit CloudFront response bodies.
#[derive(Debug, Default)]
pub struct XmlWriter {
    buf: String,
}

impl XmlWriter {
    /// Create an empty writer with a default capacity.
    #[must_use]
    pub fn new() -> Self {
        Self::with_capacity(512)
    }

    /// Create a writer pre-reserving `cap` bytes.
    #[must_use]
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            buf: String::with_capacity(cap),
        }
    }

    /// Emit the XML declaration.
    pub fn declaration(&mut self) {
        self.buf
            .push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>");
    }

    /// Open element with an optional `xmlns` attribute.
    pub fn open_root(&mut self, name: &str, namespace: Option<&str>) {
        self.buf.push('<');
        self.buf.push_str(name);
        if let Some(ns) = namespace {
            self.buf.push_str(" xmlns=\"");
            self.buf.push_str(ns);
            self.buf.push('\"');
        }
        self.buf.push('>');
    }

    /// Open element.
    pub fn open(&mut self, name: &str) {
        self.buf.push('<');
        self.buf.push_str(name);
        self.buf.push('>');
    }

    /// Close element.
    pub fn close(&mut self, name: &str) {
        self.buf.push_str("</");
        self.buf.push_str(name);
        self.buf.push('>');
    }

    /// Emit `<name/>`.
    pub fn empty(&mut self, name: &str) {
        self.buf.push('<');
        self.buf.push_str(name);
        self.buf.push_str("/>");
    }

    /// Emit `<name>text</name>` with escaping.
    pub fn element(&mut self, name: &str, text: &str) {
        self.open(name);
        self.buf.push_str(&escape(text));
        self.close(name);
    }

    /// Emit `<name>value</name>` using `Display`.
    pub fn element_display<D: std::fmt::Display>(&mut self, name: &str, v: D) {
        let s = v.to_string();
        self.element(name, &s);
    }

    /// Emit `<name>true</name>` / `<name>false</name>`.
    pub fn bool(&mut self, name: &str, v: bool) {
        self.element(name, if v { "true" } else { "false" });
    }

    /// Emit an optional string element only when `v` is non-empty.
    pub fn optional_str(&mut self, name: &str, v: &str) {
        if !v.is_empty() {
            self.element(name, v);
        }
    }

    /// Emit a `<Quantity>n</Quantity><Items>…</Items>` wrapped list.
    pub fn items<T, F: FnMut(&mut Self, &T)>(
        &mut self,
        items: &[T],
        wrapper: &str,
        item_name: &str,
        mut emit: F,
    ) {
        self.open(wrapper);
        self.element_display("Quantity", items.len());
        if items.is_empty() {
            // CloudFront omits Items entirely when the list is empty.
        } else {
            self.open("Items");
            for it in items {
                self.open(item_name);
                emit(self, it);
                self.close(item_name);
            }
            self.close("Items");
        }
        self.close(wrapper);
    }

    /// Consume the writer and return the XML body.
    #[must_use]
    pub fn finish(self) -> String {
        self.buf
    }

    /// Append raw, pre-escaped XML. Caller is responsible for correctness.
    pub fn raw(&mut self, s: &str) {
        self.buf.push_str(s);
    }
}

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}
