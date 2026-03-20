//! STS XML response formatting and error serialization.
//!
//! STS responses use `text/xml` content type following the awsQuery protocol.
//! All STS responses follow the pattern:
//!
//! ```xml
//! <{Operation}Response xmlns="https://sts.amazonaws.com/doc/2011-06-15/">
//!   <{Operation}Result>
//!     ...fields...
//!   </{Operation}Result>
//!   <ResponseMetadata>
//!     <RequestId>{uuid}</RequestId>
//!   </ResponseMetadata>
//! </{Operation}Response>
//! ```

use ruststack_sts_model::error::StsError;

use crate::body::StsResponseBody;

/// Content type for STS XML responses.
pub const CONTENT_TYPE: &str = "text/xml";

/// The STS XML namespace.
const XML_NS: &str = "https://sts.amazonaws.com/doc/2011-06-15/";

/// Build a success XML response with the given body and request ID.
#[must_use]
pub fn xml_response(xml: String, request_id: &str) -> http::Response<StsResponseBody> {
    let body = StsResponseBody::from_xml(xml.into_bytes());
    http::Response::builder()
        .status(http::StatusCode::OK)
        .header("content-type", CONTENT_TYPE)
        .header("x-amzn-requestid", request_id)
        .body(body)
        .expect("valid XML response")
}

/// Serialize an STS error into an XML error response body string.
#[must_use]
pub fn error_to_xml(error: &StsError, request_id: &str) -> String {
    format!(
        "<ErrorResponse xmlns=\"{XML_NS}\">\
         <Error>\
         <Type>{}</Type>\
         <Code>{}</Code>\
         <Message>{}</Message>\
         </Error>\
         <RequestId>{}</RequestId>\
         </ErrorResponse>",
        error.code.fault(),
        error.code.code(),
        xml_escape(&error.message),
        xml_escape(request_id),
    )
}

/// Convert an `StsError` into a complete HTTP error response.
#[must_use]
pub fn error_to_response(error: &StsError, request_id: &str) -> http::Response<StsResponseBody> {
    let xml = error_to_xml(error, request_id);
    let body = StsResponseBody::from_xml(xml.into_bytes());
    http::Response::builder()
        .status(error.status_code)
        .header("content-type", CONTENT_TYPE)
        .header("x-amzn-requestid", request_id)
        .body(body)
        .expect("valid error response")
}

/// XML-escape a string value.
///
/// Replaces the five XML special characters with their entity references.
#[must_use]
pub fn xml_escape(s: &str) -> String {
    if !s.contains(['&', '<', '>', '"', '\'']) {
        return s.to_owned();
    }

    let mut result = String::with_capacity(s.len() + 16);
    for ch in s.chars() {
        match ch {
            '&' => result.push_str("&amp;"),
            '<' => result.push_str("&lt;"),
            '>' => result.push_str("&gt;"),
            '"' => result.push_str("&quot;"),
            '\'' => result.push_str("&apos;"),
            _ => result.push(ch),
        }
    }
    result
}

/// Simple XML writer for building STS response XML.
#[derive(Debug)]
pub struct XmlWriter {
    buf: String,
}

impl Default for XmlWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl XmlWriter {
    /// Create a new `XmlWriter`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            buf: String::with_capacity(512),
        }
    }

    /// Start the response envelope: `<{op}Response xmlns="...">`.
    pub fn start_response(&mut self, operation: &str) {
        self.buf.push('<');
        self.buf.push_str(operation);
        self.buf.push_str("Response xmlns=\"");
        self.buf.push_str(XML_NS);
        self.buf.push_str("\">");
    }

    /// Start the result element: `<{op}Result>`.
    pub fn start_result(&mut self, operation: &str) {
        self.buf.push('<');
        self.buf.push_str(operation);
        self.buf.push_str("Result>");
    }

    /// End an element: `</{name}>`.
    pub fn end_element(&mut self, name: &str) {
        self.buf.push_str("</");
        self.buf.push_str(name);
        self.buf.push('>');
    }

    /// Write a simple text element: `<Name>Value</Name>`.
    pub fn write_element(&mut self, name: &str, value: &str) {
        self.buf.push('<');
        self.buf.push_str(name);
        self.buf.push('>');
        self.buf.push_str(&xml_escape(value));
        self.buf.push_str("</");
        self.buf.push_str(name);
        self.buf.push('>');
    }

    /// Write an optional element (skip if `None`).
    pub fn write_optional_element(&mut self, name: &str, value: Option<&str>) {
        if let Some(v) = value {
            self.write_element(name, v);
        }
    }

    /// Write an integer element.
    pub fn write_int_element(&mut self, name: &str, value: i32) {
        self.write_element(name, &value.to_string());
    }

    /// Write raw XML content without escaping.
    pub fn raw(&mut self, s: &str) {
        self.buf.push_str(s);
    }

    /// Write the `<ResponseMetadata>` block with a `<RequestId>`.
    pub fn write_response_metadata(&mut self, request_id: &str) {
        self.buf.push_str("<ResponseMetadata>");
        self.write_element("RequestId", request_id);
        self.buf.push_str("</ResponseMetadata>");
    }

    /// Consume the writer and return the final XML string.
    #[must_use]
    pub fn into_string(self) -> String {
        self.buf
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_escape_xml_special_chars() {
        assert_eq!(xml_escape("hello"), "hello");
        assert_eq!(xml_escape("a & b"), "a &amp; b");
        assert_eq!(xml_escape("<tag>"), "&lt;tag&gt;");
    }

    #[test]
    fn test_should_format_error_xml() {
        let err = StsError::invalid_parameter_value("Bad value");
        let xml = error_to_xml(&err, "req-123");
        assert!(xml.contains("<Code>InvalidParameterValue</Code>"));
        assert!(xml.contains("<Message>Bad value</Message>"));
        assert!(xml.contains("<Type>Sender</Type>"));
        assert!(xml.contains("<RequestId>req-123</RequestId>"));
        assert!(xml.contains("sts.amazonaws.com"));
    }

    #[test]
    fn test_should_build_error_response_with_correct_status() {
        let err = StsError::invalid_parameter_value("bad");
        let resp = error_to_response(&err, "test-req-123");
        assert_eq!(resp.status(), http::StatusCode::BAD_REQUEST);
        assert_eq!(resp.headers().get("content-type").unwrap(), CONTENT_TYPE);
    }

    #[test]
    fn test_should_build_xml_with_writer() {
        let mut w = XmlWriter::new();
        w.start_response("GetCallerIdentity");
        w.start_result("GetCallerIdentity");
        w.write_element("Account", "000000000000");
        w.write_element("Arn", "arn:aws:iam::000000000000:root");
        w.write_element("UserId", "000000000000");
        w.end_element("GetCallerIdentityResult");
        w.write_response_metadata("req-789");
        w.end_element("GetCallerIdentityResponse");
        let xml = w.into_string();
        assert!(xml.contains("<Account>000000000000</Account>"));
        assert!(xml.contains("<RequestId>req-789</RequestId>"));
        assert!(xml.contains("GetCallerIdentityResponse xmlns="));
    }

    #[test]
    fn test_should_build_success_xml_response() {
        let resp = xml_response("<TestResponse/>".to_owned(), "req-success");
        assert_eq!(resp.status(), http::StatusCode::OK);
        assert_eq!(resp.headers().get("content-type").unwrap(), CONTENT_TYPE);
    }
}
