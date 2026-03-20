//! CloudWatch XML response formatting and error serialization.
//!
//! CloudWatch Metrics responses use `text/xml` content type following the awsQuery protocol.
//! All responses follow the pattern:
//!
//! ```xml
//! <{Operation}Response xmlns="http://monitoring.amazonaws.com/doc/2010-08-01/">
//!   <{Operation}Result>
//!     ...fields...
//!   </{Operation}Result>
//!   <ResponseMetadata>
//!     <RequestId>{uuid}</RequestId>
//!   </ResponseMetadata>
//! </{Operation}Response>
//! ```

use ruststack_cloudwatch_model::error::CloudWatchError;

use crate::body::CloudWatchResponseBody;

/// Content type for CloudWatch XML responses.
pub const CONTENT_TYPE: &str = "text/xml";

/// The CloudWatch Metrics XML namespace.
const XML_NS: &str = "http://monitoring.amazonaws.com/doc/2010-08-01/";

/// Build a success XML response with the given body and request ID.
#[must_use]
pub fn xml_response(xml: String, request_id: &str) -> http::Response<CloudWatchResponseBody> {
    let body = CloudWatchResponseBody::from_xml(xml.into_bytes());
    http::Response::builder()
        .status(http::StatusCode::OK)
        .header("content-type", CONTENT_TYPE)
        .header("x-amzn-requestid", request_id)
        .body(body)
        .expect("valid XML response")
}

/// Serialize a CloudWatch error into an XML error response body string.
#[must_use]
pub fn error_to_xml(error: &CloudWatchError, request_id: &str) -> String {
    let fault = if error.status_code.is_server_error() {
        "Receiver"
    } else {
        "Sender"
    };
    format!(
        "<ErrorResponse xmlns=\"{XML_NS}\">\
         <Error>\
         <Type>{fault}</Type>\
         <Code>{}</Code>\
         <Message>{}</Message>\
         </Error>\
         <RequestId>{}</RequestId>\
         </ErrorResponse>",
        xml_escape(&error.code.to_string()),
        xml_escape(&error.message),
        xml_escape(request_id),
    )
}

/// Convert a `CloudWatchError` into a complete HTTP error response.
#[must_use]
pub fn error_to_response(
    error: &CloudWatchError,
    request_id: &str,
) -> http::Response<CloudWatchResponseBody> {
    let xml = error_to_xml(error, request_id);
    let body = CloudWatchResponseBody::from_xml(xml.into_bytes());
    http::Response::builder()
        .status(error.status_code)
        .header("content-type", CONTENT_TYPE)
        .header("x-amzn-requestid", request_id)
        .body(body)
        .expect("valid error response")
}

/// XML-escape a string value.
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

/// Simple XML writer for building CloudWatch response XML.
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

    /// Start an element: `<{name}>`.
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

    /// Write a boolean element.
    pub fn write_bool_element(&mut self, name: &str, value: bool) {
        self.write_element(name, if value { "true" } else { "false" });
    }

    /// Write an f64 element.
    pub fn write_f64_element(&mut self, name: &str, value: f64) {
        self.write_element(name, &value.to_string());
    }

    /// Write an optional f64 element.
    pub fn write_optional_f64_element(&mut self, name: &str, value: Option<f64>) {
        if let Some(v) = value {
            self.write_f64_element(name, v);
        }
    }

    /// Write an i32 element.
    pub fn write_i32_element(&mut self, name: &str, value: i32) {
        self.write_element(name, &value.to_string());
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
    }

    #[test]
    fn test_should_format_error_xml() {
        let err = CloudWatchError::with_message(
            ruststack_cloudwatch_model::error::CloudWatchErrorCode::ResourceNotFound,
            "Alarm not found",
        );
        let xml = error_to_xml(&err, "req-123");
        assert!(xml.contains("<Code>ResourceNotFound</Code>"));
        assert!(xml.contains("<Message>Alarm not found</Message>"));
        assert!(xml.contains("<Type>Sender</Type>"));
        assert!(xml.contains("<RequestId>req-123</RequestId>"));
    }

    #[test]
    fn test_should_format_server_error_as_receiver() {
        let err = CloudWatchError::internal_error("something broke");
        let xml = error_to_xml(&err, "req-456");
        assert!(xml.contains("<Type>Receiver</Type>"));
    }

    #[test]
    fn test_should_build_xml_with_writer() {
        let mut w = XmlWriter::new();
        w.start_response("PutMetricData");
        w.start_result("PutMetricData");
        w.end_element("PutMetricDataResult");
        w.write_response_metadata("req-789");
        w.end_element("PutMetricDataResponse");
        let xml = w.into_string();
        assert!(xml.contains("<PutMetricDataResponse xmlns="));
        assert!(xml.contains("<RequestId>req-789</RequestId>"));
    }
}
