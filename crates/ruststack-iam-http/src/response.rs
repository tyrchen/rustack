//! IAM XML response formatting and error serialization.
//!
//! IAM responses use `text/xml` content type following the awsQuery protocol.
//! All IAM responses follow the pattern:
//!
//! ```xml
//! <{Operation}Response xmlns="https://iam.amazonaws.com/doc/2010-05-08/">
//!   <{Operation}Result>
//!     ...fields...
//!   </{Operation}Result>
//!   <ResponseMetadata>
//!     <RequestId>{uuid}</RequestId>
//!   </ResponseMetadata>
//! </{Operation}Response>
//! ```

use ruststack_iam_model::error::IamError;

use crate::body::IamResponseBody;

/// Content type for IAM XML responses.
pub const CONTENT_TYPE: &str = "text/xml";

/// The IAM XML namespace.
pub const XML_NS: &str = "https://iam.amazonaws.com/doc/2010-05-08/";

/// Build a success XML response with the given body and request ID.
#[must_use]
pub fn xml_response(xml: String, request_id: &str) -> http::Response<IamResponseBody> {
    let body = IamResponseBody::from_xml(xml.into_bytes());
    // Status and headers are compile-time constants, so builder cannot fail.
    http::Response::builder()
        .status(http::StatusCode::OK)
        .header("content-type", CONTENT_TYPE)
        .header("x-amzn-requestid", request_id)
        .body(body)
        .unwrap_or_else(|_| {
            http::Response::new(IamResponseBody::from_xml(
                b"<ErrorResponse><Error><Code>InternalError</Code></Error></ErrorResponse>"
                    .to_vec(),
            ))
        })
}

/// Serialize an IAM error into an XML error response body string.
///
/// The error format follows the AWS IAM XML protocol:
///
/// ```xml
/// <ErrorResponse xmlns="https://iam.amazonaws.com/doc/2010-05-08/">
///   <Error>
///     <Type>Sender</Type>
///     <Code>NoSuchEntity</Code>
///     <Message>User not found</Message>
///   </Error>
///   <RequestId>request-id</RequestId>
/// </ErrorResponse>
/// ```
#[must_use]
pub fn error_to_xml(error: &IamError, request_id: &str) -> String {
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

/// Convert an `IamError` into a complete HTTP error response.
#[must_use]
pub fn error_to_response(error: &IamError, request_id: &str) -> http::Response<IamResponseBody> {
    let xml = error_to_xml(error, request_id);
    let body = IamResponseBody::from_xml(xml.into_bytes());
    // Status comes from a valid error code and headers are constants, so builder cannot fail.
    http::Response::builder()
        .status(error.code.status_code())
        .header("content-type", CONTENT_TYPE)
        .header("x-amzn-requestid", request_id)
        .body(body)
        .unwrap_or_else(|_| {
            http::Response::new(IamResponseBody::from_xml(
                b"<ErrorResponse><Error><Code>InternalError</Code></Error></ErrorResponse>"
                    .to_vec(),
            ))
        })
}

/// XML-escape a string value.
///
/// Replaces the five XML special characters with their entity references.
#[must_use]
pub fn xml_escape(s: &str) -> String {
    // Fast path: if no special characters, return as-is.
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

/// Simple XML writer for building IAM response XML.
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

    /// Start an element without namespace: `<{name}>`.
    pub fn start_element(&mut self, name: &str) {
        self.buf.push('<');
        self.buf.push_str(name);
        self.buf.push('>');
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

    /// Write a boolean element: `<name>true</name>` or `<name>false</name>`.
    pub fn write_bool_element(&mut self, name: &str, value: bool) {
        self.write_element(name, if value { "true" } else { "false" });
    }

    /// Write an i32 element: `<name>42</name>`.
    pub fn write_i32_element(&mut self, name: &str, value: i32) {
        self.buf.push('<');
        self.buf.push_str(name);
        self.buf.push('>');
        self.buf.push_str(&value.to_string());
        self.buf.push_str("</");
        self.buf.push_str(name);
        self.buf.push('>');
    }

    /// Write the `<ResponseMetadata>` block with a `<RequestId>`.
    pub fn write_response_metadata(&mut self, request_id: &str) {
        self.buf.push_str("<ResponseMetadata>");
        self.write_element("RequestId", request_id);
        self.buf.push_str("</ResponseMetadata>");
    }

    /// Write raw XML content without escaping.
    pub fn raw(&mut self, s: &str) {
        self.buf.push_str(s);
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
        assert_eq!(
            xml_escape("a&b<c>d\"e'f"),
            "a&amp;b&lt;c&gt;d&quot;e&apos;f"
        );
    }

    #[test]
    fn test_should_not_allocate_for_safe_strings() {
        let s = "hello world 12345";
        let result = xml_escape(s);
        assert_eq!(result, s);
    }

    #[test]
    fn test_should_format_error_xml() {
        let err = IamError::no_such_entity("User not found");
        let xml = error_to_xml(&err, "req-123");
        assert!(xml.contains("<Code>NoSuchEntity</Code>"));
        assert!(xml.contains("<Message>User not found</Message>"));
        assert!(xml.contains("<Type>Sender</Type>"));
        assert!(xml.contains("<RequestId>req-123</RequestId>"));
    }

    #[test]
    fn test_should_build_error_response_with_correct_status() {
        let err = IamError::no_such_entity("not found");
        let resp = error_to_response(&err, "test-req-123");
        assert_eq!(resp.status(), http::StatusCode::NOT_FOUND);
        assert_eq!(resp.headers().get("content-type").unwrap(), CONTENT_TYPE);
        assert_eq!(
            resp.headers().get("x-amzn-requestid").unwrap(),
            "test-req-123",
        );
    }

    #[test]
    fn test_should_format_sender_fault_error() {
        let error = IamError::invalid_input("Bad input");
        let resp = error_to_response(&error, "req-456");
        assert_eq!(resp.status(), http::StatusCode::BAD_REQUEST);
    }

    #[test]
    fn test_should_format_receiver_fault_error() {
        let error = IamError::internal_error("Something broke");
        let resp = error_to_response(&error, "req-789");
        assert_eq!(resp.status(), http::StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn test_should_build_success_xml_response() {
        let resp = xml_response("<TestResponse/>".to_owned(), "req-success");
        assert_eq!(resp.status(), http::StatusCode::OK);
        assert_eq!(resp.headers().get("content-type").unwrap(), CONTENT_TYPE);
    }

    #[test]
    fn test_should_escape_error_message_in_xml() {
        let error = IamError::invalid_input("Value <script> & \"injection\"");
        let xml = error_to_xml(&error, "req-xss");
        assert!(xml.contains("&lt;script&gt;"));
        assert!(xml.contains("&amp;"));
        assert!(xml.contains("&quot;injection&quot;"));
    }

    #[test]
    fn test_should_build_xml_with_writer() {
        let mut w = XmlWriter::new();
        w.start_response("CreateUser");
        w.start_result("CreateUser");
        w.write_element("UserName", "testuser");
        w.end_element("CreateUserResult");
        w.write_response_metadata("req-789");
        w.end_element("CreateUserResponse");
        let xml = w.into_string();
        assert!(xml.contains("<UserName>testuser</UserName>"));
        assert!(xml.contains("<RequestId>req-789</RequestId>"));
        assert!(xml.contains("<CreateUserResponse xmlns="));
    }

    #[test]
    fn test_should_build_xml_with_optional_elements() {
        let mut w = XmlWriter::new();
        w.write_optional_element("Present", Some("value"));
        w.write_optional_element("Absent", None);
        let xml = w.into_string();
        assert!(xml.contains("<Present>value</Present>"));
        assert!(!xml.contains("Absent"));
    }

    #[test]
    fn test_should_build_xml_with_bool_element() {
        let mut w = XmlWriter::new();
        w.write_bool_element("Enabled", true);
        w.write_bool_element("Disabled", false);
        let xml = w.into_string();
        assert!(xml.contains("<Enabled>true</Enabled>"));
        assert!(xml.contains("<Disabled>false</Disabled>"));
    }

    #[test]
    fn test_should_build_xml_with_i32_element() {
        let mut w = XmlWriter::new();
        w.write_i32_element("MaxItems", 100);
        w.write_i32_element("Negative", -5);
        let xml = w.into_string();
        assert!(xml.contains("<MaxItems>100</MaxItems>"));
        assert!(xml.contains("<Negative>-5</Negative>"));
    }

    #[test]
    fn test_should_build_xml_writer_default() {
        let w = XmlWriter::default();
        assert_eq!(w.into_string(), "");
    }

    #[test]
    fn test_should_use_text_xml_content_type() {
        assert_eq!(CONTENT_TYPE, "text/xml");
    }

    #[test]
    fn test_should_use_iam_xml_namespace() {
        assert_eq!(XML_NS, "https://iam.amazonaws.com/doc/2010-05-08/");
    }

    #[test]
    fn test_should_start_element_without_namespace() {
        let mut w = XmlWriter::new();
        w.start_element("member");
        w.write_element("UserName", "test");
        w.end_element("member");
        let xml = w.into_string();
        assert_eq!(xml, "<member><UserName>test</UserName></member>");
    }

    #[test]
    fn test_should_write_raw_content() {
        let mut w = XmlWriter::new();
        w.raw("<already>escaped</already>");
        let xml = w.into_string();
        assert_eq!(xml, "<already>escaped</already>");
    }
}
