//! Answers `Expect: 100-continue` on HTTP/1.1 requests that carry no body.
//!
//! S3 answers `Expect: 100-continue` on an empty-body `PutObject` with `100 Continue` before the
//! final response:
//!
//! ```text
//! > PUT /key HTTP/1.1
//! > Expect: 100-continue
//! > Content-Length: 0
//! < HTTP/1.1 100 Continue
//! < HTTP/1.1 200 OK
//! < Server: AmazonS3
//! ```
//!
//! AWS SDK v3 for Node (`@smithy/node-http-handler` 2.x) depends on it: an empty-stream
//! `PutObject` waits for the 100 until its request timeout. hyper sends the 100 only when a
//! service polls a non-empty body, and gives services no way to send a 1xx themselves.
//!
//! [`ExpectContinueIo`] sits between the socket and hyper and covers clients that wait for each
//! response before sending the next request, as the AWS SDKs do. It follows request framing on
//! the read side, so body bytes are never taken for a head, and response framing on the write
//! side, so it knows when the previous response has ended. A head that arrives after that is the
//! only request in flight, so hyper's next bytes start its response and an owed 100 goes right
//! before them. Requests with a body are left to hyper, which answers them when the body is read.
//!
//! hyper reads ahead while it is still writing a response, so a pipelined head can arrive
//! mid-response; a written byte alone does not mean the previous response has ended. A head that
//! arrives before then, and anything else this cannot follow (HTTP/2, a head httparse rejects or
//! that outgrows [`MAX_BUFFERED_BYTES`], a close-delimited response, an upgrade), turns the
//! connection into a plain pass-through, which is how it behaved before.

use std::{
    io::{self, IoSlice},
    pin::Pin,
    task::{Context, Poll, ready},
};

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tracing::debug;

const CONTINUE: &[u8] = b"HTTP/1.1 100 Continue\r\n\r\n";

/// hyper's default request header limit; a head with more is one hyper rejects anyway.
const MAX_REQUEST_HEADERS: usize = 100;

/// Responses and trailers carry service-chosen headers, such as S3 object metadata.
const MAX_RESPONSE_HEADERS: usize = 256;

/// Longest partial head, chunk-size line or trailer section held before passing through.
///
/// Below hyper's 400 KiB read buffer limit, so a larger head still reaches hyper; the connection
/// just stops answering empty-body expectations.
const MAX_BUFFERED_BYTES: usize = 64 * 1024;

/// Why a connection stopped being followed.
type Unfollowable = &'static str;

const UNPARSEABLE: Unfollowable = "unparseable HTTP/1 framing";

/// Socket adapter that adds the `100 Continue` hyper skips for empty-body requests.
#[derive(Debug)]
pub(crate) struct ExpectContinueIo<T> {
    io: T,
    /// `None` once the connection is passed through untouched.
    watch: Option<Watch>,
    /// The unwritten part of an owed `100 Continue`, written before anything from hyper.
    owed: &'static [u8],
}

impl<T> ExpectContinueIo<T> {
    pub(crate) fn new(io: T) -> Self {
        Self {
            io,
            watch: Some(Watch::default()),
            owed: &[],
        }
    }

    fn pass_through(&mut self, reason: Unfollowable) {
        debug!(reason, "expect-continue adapter passing connection through");
        self.watch = None;
    }

    /// Follows the response bytes the socket accepted from hyper's buffers.
    fn observe_written<'a>(&mut self, bufs: impl IntoIterator<Item = &'a [u8]>, written: usize) {
        let Some(watch) = self.watch.as_mut() else {
            return;
        };
        let mut left = written;
        let followed = bufs.into_iter().try_for_each(|buf| {
            let taken = buf.len().min(left);
            left -= taken;
            watch.observe_responses(buf.get(..taken).unwrap_or_default())
        });
        if let Err(reason) = followed {
            self.pass_through(reason);
        }
    }
}

impl<T: AsyncWrite + Unpin> ExpectContinueIo<T> {
    fn poll_owed(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        while !self.owed.is_empty() {
            let written = ready!(Pin::new(&mut self.io).poll_write(cx, self.owed))?;
            if written == 0 {
                return Poll::Ready(Err(io::ErrorKind::WriteZero.into()));
            }
            self.owed = self.owed.get(written..).unwrap_or_default();
        }
        Poll::Ready(Ok(()))
    }
}

impl<T: AsyncRead + Unpin> AsyncRead for ExpectContinueIo<T> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let start = buf.filled().len();
        ready!(Pin::new(&mut this.io).poll_read(cx, buf))?;
        if let Some(watch) = this.watch.as_mut() {
            match watch.observe_requests(buf.filled().get(start..).unwrap_or_default()) {
                Ok(true) => this.owed = CONTINUE,
                Ok(false) => {}
                Err(reason) => this.pass_through(reason),
            }
        }
        Poll::Ready(Ok(()))
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for ExpectContinueIo<T> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        ready!(this.poll_owed(cx))?;
        let written = ready!(Pin::new(&mut this.io).poll_write(cx, buf))?;
        this.observe_written([buf], written);
        Poll::Ready(Ok(written))
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[IoSlice<'_>],
    ) -> Poll<io::Result<usize>> {
        let this = self.get_mut();
        ready!(this.poll_owed(cx))?;
        let written = ready!(Pin::new(&mut this.io).poll_write_vectored(cx, bufs))?;
        this.observe_written(bufs.iter().map(|buf| &**buf), written);
        Poll::Ready(Ok(written))
    }

    fn is_write_vectored(&self) -> bool {
        self.io.is_write_vectored()
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        ready!(this.poll_owed(cx))?;
        Pin::new(&mut this.io).poll_flush(cx)
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        ready!(this.poll_owed(cx))?;
        Pin::new(&mut this.io).poll_shutdown(cx)
    }
}

/// Both directions of one HTTP/1 connection, followed message by message.
#[derive(Debug, Default)]
struct Watch {
    requests: Framer,
    responses: Framer,
    /// The request read whose final response has not started yet.
    in_flight: Option<Exchange>,
}

#[derive(Debug, Clone, Copy)]
struct Exchange {
    /// A response to `HEAD` has no body, whatever its `Content-Length` says.
    is_head: bool,
}

impl Watch {
    /// Follows request bytes; `Ok(true)` when a request in them owes a `100 Continue`.
    fn observe_requests(&mut self, mut bytes: &[u8]) -> Result<bool, Unfollowable> {
        let mut owes_continue = false;
        while !bytes.is_empty() {
            let (used, head) = self.requests.advance(bytes, RequestHead::parse)?;
            if let Some(head) = head {
                if self.in_flight.is_some() || !self.responses.at_message_start() {
                    return Err("request pipelined before the previous response ended");
                }
                if head.upgrade {
                    return Err("protocol upgrade request");
                }
                self.requests.part = head.body.ok_or("request framing hyper rejects")?;
                self.in_flight = Some(Exchange {
                    is_head: head.is_head,
                });
                owes_continue = head.owes_continue;
            }
            bytes = bytes.get(used..).unwrap_or_default();
        }
        Ok(owes_continue)
    }

    fn observe_responses(&mut self, mut bytes: &[u8]) -> Result<(), Unfollowable> {
        while !bytes.is_empty() {
            let (used, head) = self.responses.advance(bytes, ResponseHead::parse)?;
            if let Some(head) = head {
                self.responses.part = self.body_after(&head)?;
            }
            bytes = bytes.get(used..).unwrap_or_default();
        }
        Ok(())
    }

    /// The part after a response head, retiring the request in flight when the head is final.
    fn body_after(&mut self, head: &ResponseHead) -> Result<Part, Unfollowable> {
        match head.status {
            101 => Err("protocol upgrade response"),
            100..=199 => Ok(Part::Head),
            status => {
                let exchange = self.in_flight.take().ok_or("response without a request")?;
                if exchange.is_head || status == 204 || status == 304 {
                    return Ok(Part::Head);
                }
                match head.framing {
                    Framing::Chunked => Ok(Part::ChunkSize),
                    Framing::Length(length) => Ok(Part::fixed(length)),
                    Framing::Unframed | Framing::Invalid => Err("close-delimited response"),
                }
            }
        }
    }
}

/// Finds message boundaries in one direction of an HTTP/1 byte stream.
#[derive(Debug, Default)]
struct Framer {
    part: Part,
    /// The start of a head, chunk-size line or trailer section split across reads or writes.
    buffered: Vec<u8>,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum Part {
    #[default]
    Head,
    Fixed(u64),
    ChunkSize,
    /// Chunk data plus the CRLF that ends it.
    ChunkData(u64),
    Trailers,
}

impl Part {
    fn fixed(length: u64) -> Self {
        if length == 0 {
            Self::Head
        } else {
            Self::Fixed(length)
        }
    }
}

/// A parsed prefix: its length and value, or `None` when the bytes end before it does.
type Parsed<T> = Result<Option<(usize, T)>, Unfollowable>;

impl Framer {
    fn at_message_start(&self) -> bool {
        self.part == Part::Head && self.buffered.is_empty()
    }

    /// Consumes a prefix of `bytes`; returns its length and the head it completed, if any.
    ///
    /// The caller sets the part that follows a head.
    fn advance<H>(
        &mut self,
        bytes: &[u8],
        parse_head: impl FnOnce(&[u8]) -> Parsed<H>,
    ) -> Result<(usize, Option<H>), Unfollowable> {
        let parse: fn(&[u8]) -> Parsed<Part> = match self.part {
            Part::Head => {
                return Ok(match self.parse_buffered(bytes, parse_head)? {
                    Some((used, head)) => (used, Some(head)),
                    None => (bytes.len(), None),
                });
            }
            Part::Fixed(left) | Part::ChunkData(left) => {
                let used = left.min(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
                let left = left - used;
                self.part = match self.part {
                    Part::Fixed(_) => Part::fixed(left),
                    _ if left == 0 => Part::ChunkSize,
                    _ => Part::ChunkData(left),
                };
                return Ok((usize::try_from(used).unwrap_or(bytes.len()), None));
            }
            Part::ChunkSize => chunk_size,
            Part::Trailers => trailers,
        };
        match self.parse_buffered(bytes, parse)? {
            Some((used, next)) => {
                self.part = next;
                Ok((used, None))
            }
            None => Ok((bytes.len(), None)),
        }
    }

    /// Parses a prefix that may have started in earlier bytes; the length covers only `bytes`.
    fn parse_buffered<T>(
        &mut self,
        bytes: &[u8],
        parse: impl FnOnce(&[u8]) -> Parsed<T>,
    ) -> Parsed<T> {
        let held = self.buffered.len();
        let parsed = if held == 0 {
            parse(bytes)?
        } else {
            self.buffered.extend_from_slice(bytes);
            parse(&self.buffered)?
        };
        let Some((end, value)) = parsed else {
            if held == 0 {
                self.buffered.extend_from_slice(bytes);
            }
            if self.buffered.len() > MAX_BUFFERED_BYTES {
                return Err("head, chunk size or trailers over the buffer cap");
            }
            return Ok(None);
        };
        self.buffered.clear();
        let used = end.checked_sub(held).filter(|used| *used > 0);
        Ok(Some((used.ok_or(UNPARSEABLE)?, value)))
    }
}

#[derive(Debug)]
struct RequestHead {
    is_head: bool,
    owes_continue: bool,
    /// `None` when hyper would reject the framing.
    body: Option<Part>,
    /// After `CONNECT` or `Upgrade` the bytes may stop being HTTP/1.
    upgrade: bool,
}

impl RequestHead {
    fn parse(bytes: &[u8]) -> Parsed<Self> {
        let mut headers = [httparse::EMPTY_HEADER; MAX_REQUEST_HEADERS];
        let mut request = httparse::Request::new(&mut headers);
        Ok(complete(request.parse(bytes))?.map(|end| (end, Self::new(&request))))
    }

    /// Reads the head the way hyper's server does in `proto::h1::role`.
    fn new(request: &httparse::Request<'_, '_>) -> Self {
        let is_http_11 = request.version == Some(1);
        let method = request.method.unwrap_or_default();
        let mut expects_continue = false;
        let mut upgrade = method == "CONNECT";
        for header in &*request.headers {
            if header.name.eq_ignore_ascii_case("expect") {
                // hyper keeps the last Expect header.
                expects_continue = header.value.eq_ignore_ascii_case(b"100-continue");
            } else if header.name.eq_ignore_ascii_case("upgrade") {
                upgrade |= is_http_11;
            }
        }
        let body = match Framing::of(request.headers) {
            Framing::Chunked if is_http_11 => Some(Part::ChunkSize),
            Framing::Length(length) => Some(Part::fixed(length)),
            Framing::Unframed => Some(Part::Head),
            Framing::Chunked | Framing::Invalid => None,
        };
        Self {
            is_head: method == "HEAD",
            owes_continue: is_http_11 && expects_continue && body == Some(Part::Head),
            body,
            upgrade,
        }
    }
}

#[derive(Debug)]
struct ResponseHead {
    status: u16,
    framing: Framing,
}

impl ResponseHead {
    fn parse(bytes: &[u8]) -> Parsed<Self> {
        let mut headers = [httparse::EMPTY_HEADER; MAX_RESPONSE_HEADERS];
        let mut response = httparse::Response::new(&mut headers);
        let parsed = complete(response.parse(bytes))?;
        let head = Self {
            status: response.code.unwrap_or_default(),
            framing: Framing::of(response.headers),
        };
        Ok(parsed.map(|end| (end, head)))
    }
}

/// Body framing from `Transfer-Encoding` and `Content-Length`, where the former wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Framing {
    Chunked,
    Length(u64),
    Unframed,
    Invalid,
}

impl Framing {
    fn of(headers: &[httparse::Header<'_>]) -> Self {
        let mut chunked = None;
        let mut length = None;
        for header in headers {
            if header.name.eq_ignore_ascii_case("transfer-encoding") {
                // Chunked must be the last coding; hyper keeps the last header.
                chunked = Some(
                    header
                        .value
                        .rsplit(|byte| *byte == b',')
                        .next()
                        .is_some_and(|coding| coding.trim_ascii().eq_ignore_ascii_case(b"chunked")),
                );
            } else if header.name.eq_ignore_ascii_case("content-length") {
                match (content_length(header.value), length) {
                    (Some(value), None) => length = Some(value),
                    (Some(value), Some(seen)) if value == seen => {}
                    _ => return Self::Invalid,
                }
            }
        }
        match (chunked, length) {
            (Some(true), _) => Self::Chunked,
            (Some(false), _) => Self::Invalid,
            (None, Some(length)) => Self::Length(length),
            (None, None) => Self::Unframed,
        }
    }
}

/// Digits only, as hyper requires: no sign, no whitespace, no list.
fn content_length(value: &[u8]) -> Option<u64> {
    if value.is_empty() || !value.iter().all(u8::is_ascii_digit) {
        return None;
    }
    std::str::from_utf8(value).ok()?.parse().ok()
}

fn complete<S, E>(status: Result<httparse::Status<S>, E>) -> Result<Option<S>, Unfollowable> {
    match status {
        Ok(httparse::Status::Complete(value)) => Ok(Some(value)),
        Ok(httparse::Status::Partial) => Ok(None),
        Err(_) => Err(UNPARSEABLE),
    }
}

fn chunk_size(bytes: &[u8]) -> Parsed<Part> {
    let Some((end, size)) = complete(httparse::parse_chunk_size(bytes))? else {
        return Ok(None);
    };
    let next = match size {
        0 => Part::Trailers,
        size => Part::ChunkData(size.checked_add(2).ok_or("chunk size overflow")?),
    };
    Ok(Some((end, next)))
}

/// The trailer fields after the last chunk, up to and including the empty line.
fn trailers(bytes: &[u8]) -> Parsed<Part> {
    let mut headers = [httparse::EMPTY_HEADER; MAX_RESPONSE_HEADERS];
    Ok(complete(httparse::parse_headers(bytes, &mut headers))?.map(|(end, _)| (end, Part::Head)))
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, convert::Infallible, time::Duration};

    use bytes::Bytes;
    use http_body_util::{BodyExt, Full, combinators::BoxBody};
    use hyper::{
        Request, Response,
        body::{Body, Frame, Incoming},
        service::service_fn,
    };
    use hyper_util::{
        rt::{TokioExecutor, TokioIo},
        server::conn::auto::Builder,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream};

    use super::*;

    const BIG: usize = 200_000;
    const CONTINUE_THEN_OK: &str = "HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 200 OK\r\n";

    /// A body with no known length, so hyper frames it as chunked.
    struct Chunks(VecDeque<Bytes>);

    impl Body for Chunks {
        type Data = Bytes;
        type Error = Infallible;

        fn poll_frame(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Option<Result<Frame<Bytes>, Infallible>>> {
            Poll::Ready(
                self.get_mut()
                    .0
                    .pop_front()
                    .map(|chunk| Ok(Frame::data(chunk))),
            )
        }
    }

    /// Reads the whole body, so hyper sends its own 100 for bodies, then answers with a tag.
    async fn tagged(
        request: Request<Incoming>,
    ) -> Result<Response<BoxBody<Bytes, Infallible>>, hyper::Error> {
        let method = request.method().clone();
        let path = request.uri().path().to_owned();
        let body = request.into_body();
        if path == "/background" {
            // Answers before the body arrives; hyper keeps reading it for the spawned task.
            tokio::spawn(body.collect());
            return Ok(Response::new(
                Full::new(Bytes::from(format!("[{method} {path}]"))).boxed(),
            ));
        }
        let length = body.collect().await?.to_bytes().len();
        let tag = Bytes::from(format!("[{method} {path} {length}]"));
        let body = match path.as_str() {
            "/big" => Full::new(Bytes::from([vec![b'x'; BIG], tag.to_vec()].concat())).boxed(),
            "/chunked" => Chunks(VecDeque::from([Bytes::from_static(b"streamed "), tag])).boxed(),
            _ => Full::new(tag).boxed(),
        };
        Ok(Response::new(body))
    }

    /// Serves one connection the way `serve()` does, through the adapter.
    fn connect(buffer: usize) -> DuplexStream {
        let (client, server) = tokio::io::duplex(buffer);
        tokio::spawn(async move {
            let builder = Builder::new(TokioExecutor::new());
            let connection = builder.serve_connection(
                TokioIo::new(ExpectContinueIo::new(server)),
                service_fn(tagged),
            );
            let _ = connection.await;
        });
        client
    }

    /// Reads until the server closes; the last request of each test asks it to.
    async fn read_to_close(client: &mut DuplexStream) -> String {
        let mut transcript = Vec::new();
        let read =
            tokio::time::timeout(Duration::from_secs(5), client.read_to_end(&mut transcript));
        assert!(
            read.await.is_ok(),
            "server did not finish: {}",
            String::from_utf8_lossy(&transcript)
        );
        String::from_utf8_lossy(&transcript).into_owned()
    }

    /// Reads until `needle` arrives.
    async fn read_until(client: &mut DuplexStream, needle: &str) -> String {
        let mut transcript = Vec::new();
        let read = async {
            let mut chunk = [0_u8; 1024];
            while !String::from_utf8_lossy(&transcript).contains(needle) {
                let read = client.read(&mut chunk).await.unwrap_or_default();
                assert!(read > 0, "connection closed before {needle:?}");
                transcript.extend_from_slice(chunk.get(..read).unwrap_or_default());
            }
        };
        assert!(
            tokio::time::timeout(Duration::from_secs(5), read)
                .await
                .is_ok(),
            "no {needle:?} in {}",
            String::from_utf8_lossy(&transcript)
        );
        String::from_utf8_lossy(&transcript).into_owned()
    }

    /// The status line of every response, interim ones included, in wire order.
    fn statuses(transcript: &str) -> Vec<&str> {
        transcript
            .match_indices("HTTP/1.1 ")
            .filter_map(|(start, _)| {
                let line = transcript.get(start + "HTTP/1.1 ".len()..)?;
                line.split("\r\n").next()
            })
            .collect()
    }

    async fn send(client: &mut DuplexStream, bytes: &str) {
        assert!(client.write_all(bytes.as_bytes()).await.is_ok());
    }

    #[tokio::test]
    async fn test_should_send_continue_before_response_to_empty_expect_request() {
        let mut client = connect(64 * 1024);
        send(
            &mut client,
            "PUT /empty HTTP/1.1\r\nHost: s3\r\nExpect: 100-continue\r\nContent-Length: \
             0\r\nConnection: close\r\n\r\n",
        )
        .await;

        let transcript = read_to_close(&mut client).await;

        assert!(transcript.starts_with(CONTINUE_THEN_OK), "{transcript}");
        assert_eq!(statuses(&transcript), ["100 Continue", "200 OK"]);
        assert!(transcript.ends_with("[PUT /empty 0]"), "{transcript}");
    }

    #[tokio::test]
    async fn test_should_send_continue_to_each_empty_expect_request_on_keep_alive_connection() {
        let mut client = connect(64 * 1024);
        send(
            &mut client,
            "PUT /first HTTP/1.1\r\nHost: s3\r\nExpect: 100-continue\r\nContent-Length: 0\r\n\r\n",
        )
        .await;
        let first = read_until(&mut client, "[PUT /first 0]").await;
        assert!(first.starts_with(CONTINUE_THEN_OK), "{first}");
        // No Content-Length and no Transfer-Encoding also means no body.
        send(
            &mut client,
            "PUT /second HTTP/1.1\r\nHost: s3\r\nEXPECT: 100-Continue\r\nConnection: close\r\n\r\n",
        )
        .await;

        let transcript = read_to_close(&mut client).await;

        assert!(transcript.starts_with(CONTINUE_THEN_OK), "{transcript}");
        assert_eq!(statuses(&transcript), ["100 Continue", "200 OK"]);
        assert!(transcript.ends_with("[PUT /second 0]"), "{transcript}");
    }

    #[tokio::test]
    async fn test_should_send_continue_after_large_head_and_chunked_responses() {
        // A small pipe makes hyper write the big and chunked responses in many short writes.
        let mut client = connect(4096);
        send(&mut client, "GET /big HTTP/1.1\r\nHost: s3\r\n\r\n").await;
        read_until(&mut client, "[GET /big 0]").await;
        send(&mut client, "HEAD /big HTTP/1.1\r\nHost: s3\r\n\r\n").await;
        read_until(&mut client, "\r\n\r\n").await;
        send(&mut client, "GET /chunked HTTP/1.1\r\nHost: s3\r\n\r\n").await;
        read_until(&mut client, "[GET /chunked 0]\r\n0\r\n\r\n").await;
        send(
            &mut client,
            "PUT /last HTTP/1.1\r\nHost: s3\r\nExpect: 100-continue\r\nConnection: close\r\n\r\n",
        )
        .await;

        let transcript = read_to_close(&mut client).await;

        assert!(transcript.starts_with(CONTINUE_THEN_OK), "{transcript}");
        assert!(transcript.ends_with("[PUT /last 0]"), "{transcript}");
    }

    #[tokio::test]
    async fn test_should_leave_continue_for_request_with_body_to_hyper() {
        let mut client = connect(64 * 1024);
        send(
            &mut client,
            "PUT /body HTTP/1.1\r\nHost: s3\r\nExpect: 100-continue\r\nContent-Length: \
             5\r\nConnection: close\r\n\r\n",
        )
        .await;
        let interim = read_until(&mut client, "\r\n\r\n").await;
        assert_eq!(interim, "HTTP/1.1 100 Continue\r\n\r\n");
        send(&mut client, "hello").await;

        let transcript = read_to_close(&mut client).await;

        assert_eq!(statuses(&transcript), ["200 OK"]);
        assert!(transcript.ends_with("[PUT /body 5]"), "{transcript}");
    }

    #[tokio::test]
    async fn test_should_send_continue_to_empty_expect_request_after_chunked_body() {
        let mut client = connect(64 * 1024);
        send(
            &mut client,
            "PUT /chunks HTTP/1.1\r\nHost: s3\r\nTransfer-Encoding: \
             chunked\r\n\r\n5;ext=1\r\nhello\r\n3\r\nabc\r\n0\r\nx-checksum: 1\r\n\r\n",
        )
        .await;
        read_until(&mut client, "[PUT /chunks 8]").await;
        send(
            &mut client,
            "PUT /empty HTTP/1.1\r\nHost: s3\r\nExpect: 100-continue\r\nContent-Length: \
             0\r\nConnection: close\r\n\r\n",
        )
        .await;

        let transcript = read_to_close(&mut client).await;

        assert!(transcript.starts_with(CONTINUE_THEN_OK), "{transcript}");
        assert_eq!(statuses(&transcript), ["100 Continue", "200 OK"]);
        assert!(transcript.ends_with("[PUT /empty 0]"), "{transcript}");
    }

    #[tokio::test]
    async fn test_should_not_send_continue_without_expect() {
        let mut client = connect(64 * 1024);
        send(
            &mut client,
            "PUT /plain HTTP/1.1\r\nHost: s3\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )
        .await;

        let transcript = read_to_close(&mut client).await;

        assert_eq!(statuses(&transcript), ["200 OK"]);
        assert!(transcript.ends_with("[PUT /plain 0]"), "{transcript}");
    }

    #[tokio::test]
    async fn test_should_detect_expect_request_head_split_across_reads() {
        let mut client = connect(64 * 1024);
        for piece in [
            "PUT /split HTTP/1.1\r\nHost: s3\r\nExp",
            "ect: 100-con",
            "tinue\r\nConnection: close\r\n\r",
            "\n",
        ] {
            send(&mut client, piece).await;
            tokio::time::sleep(Duration::from_millis(20)).await;
        }

        let transcript = read_to_close(&mut client).await;

        assert!(transcript.starts_with(CONTINUE_THEN_OK), "{transcript}");
        assert!(transcript.ends_with("[PUT /split 0]"), "{transcript}");
    }

    #[tokio::test]
    async fn test_should_not_inject_continue_into_pipelined_requests() {
        let mut client = connect(4096);
        send(
            &mut client,
            "GET /big HTTP/1.1\r\nHost: s3\r\n\r\nHEAD /big HTTP/1.1\r\nHost: s3\r\n\r\nGET \
             /chunked HTTP/1.1\r\nHost: s3\r\n\r\nPUT /a HTTP/1.1\r\nHost: s3\r\nExpect: \
             100-continue\r\nContent-Length: 0\r\n\r\nGET /plain HTTP/1.1\r\nHost: s3\r\n\r\nPUT \
             /b HTTP/1.1\r\nHost: s3\r\nExpect: 100-continue\r\nConnection: close\r\n\r\n",
        )
        .await;

        let transcript = read_to_close(&mut client).await;

        assert_eq!(statuses(&transcript), ["200 OK"; 6], "{transcript}");
        assert!(transcript.contains(&format!("{}[GET /big 0]", "x".repeat(BIG))));
        let tags = [
            "[GET /big 0]",
            "streamed ",
            "[GET /chunked 0]",
            "[PUT /a 0]",
            "[GET /plain 0]",
            "[PUT /b 0]",
        ];
        let positions: Vec<usize> = tags.iter().filter_map(|tag| transcript.find(tag)).collect();
        assert_eq!(positions.len(), tags.len(), "{transcript}");
        assert!(positions.is_sorted(), "{positions:?}");
        assert!(transcript.ends_with("[PUT /b 0]"));
    }

    #[tokio::test]
    async fn test_should_not_inject_continue_into_response_stalled_on_backpressure() {
        // The pipe fills long before the big body is written, so hyper stalls mid-body.
        let mut client = connect(4096);
        send(&mut client, "GET /big HTTP/1.1\r\nHost: s3\r\n\r\n").await;
        let start = read_until(&mut client, "HTTP/1.1 200 OK").await;
        send(
            &mut client,
            "PUT /empty HTTP/1.1\r\nHost: s3\r\nExpect: 100-continue\r\nContent-Length: \
             0\r\nConnection: close\r\n\r\n",
        )
        .await;

        let transcript = start + &read_to_close(&mut client).await;

        assert!(transcript.contains(&format!("{}[GET /big 0]", "x".repeat(BIG))));
        assert_eq!(statuses(&transcript), ["200 OK", "200 OK"]);
        assert!(transcript.ends_with("[PUT /empty 0]"), "{transcript}");
    }

    #[tokio::test]
    async fn test_should_not_inject_continue_for_empty_request_behind_body_hyper_continued() {
        let mut client = connect(64 * 1024);
        send(
            &mut client,
            "PUT /body HTTP/1.1\r\nHost: s3\r\nExpect: 100-continue\r\nContent-Length: 5\r\n\r\n",
        )
        .await;
        let interim = read_until(&mut client, "\r\n\r\n").await;
        assert_eq!(interim, "HTTP/1.1 100 Continue\r\n\r\n");
        send(
            &mut client,
            "helloPUT /empty HTTP/1.1\r\nHost: s3\r\nExpect: 100-continue\r\nContent-Length: \
             0\r\nConnection: close\r\n\r\n",
        )
        .await;

        let transcript = read_to_close(&mut client).await;

        assert_eq!(statuses(&transcript), ["200 OK", "200 OK"], "{transcript}");
        let body = transcript.find("[PUT /body 5]");
        let empty = transcript.find("[PUT /empty 0]");
        assert!(body.is_some() && body < empty, "{transcript}");
    }

    #[tokio::test]
    async fn test_should_not_take_request_body_for_expect_request_head() {
        let mut client = connect(64 * 1024);
        let lookalike =
            "PUT /fake HTTP/1.1\r\nHost: s3\r\nExpect: 100-continue\r\nContent-Length: 0\r\n\r\n";
        send(
            &mut client,
            &format!(
                "PUT /background HTTP/1.1\r\nHost: s3\r\nContent-Length: {}\r\n\r\n",
                lookalike.len()
            ),
        )
        .await;
        // The response ends before the body is sent, so a misread head would owe a 100.
        read_until(&mut client, "[PUT /background]").await;
        send(&mut client, lookalike).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        send(
            &mut client,
            "GET /plain HTTP/1.1\r\nHost: s3\r\nConnection: close\r\n\r\n",
        )
        .await;

        let transcript = read_to_close(&mut client).await;

        assert_eq!(statuses(&transcript), ["200 OK"], "{transcript}");
        assert!(transcript.ends_with("[GET /plain 0]"), "{transcript}");
    }

    #[tokio::test]
    async fn test_should_pass_http2_through() {
        let client = connect(64 * 1024);
        let handshake =
            hyper::client::conn::http2::handshake(TokioExecutor::new(), TokioIo::new(client));
        let Ok((mut sender, connection)) = handshake.await else {
            panic!("HTTP/2 handshake failed");
        };
        tokio::spawn(connection);
        let request = Request::put("http://s3/h2")
            .header("expect", "100-continue")
            .body(Full::new(Bytes::new()))
            .unwrap_or_default();

        let response = sender.send_request(request).await;

        let Ok(response) = response else {
            panic!("HTTP/2 request failed: {response:?}");
        };
        assert_eq!(response.status(), 200);
        let body = response
            .into_body()
            .collect()
            .await
            .map(http_body_util::Collected::to_bytes);
        assert_eq!(body.ok().as_deref(), Some(&b"[PUT /h2 0]"[..]));
    }

    #[test]
    fn test_should_stop_watching_http2_preface() {
        let mut watch = Watch::default();

        let followed = watch.observe_requests(b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n");

        assert!(followed.is_err());
    }

    #[test]
    fn test_should_follow_exchanges_fed_one_byte_at_a_time() {
        let mut watch = Watch::default();
        let exchanges: [(&[u8], &[u8]); 3] = [
            (
                b"PUT /a HTTP/1.1\r\nTransfer-Encoding: gzip, chunked\r\n\r\n\
                  4;name=value\r\nwxyz\r\n0\r\nx-trailer: 1\r\n\r\n",
                b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n3\r\nabc\r\n0\r\nx-t: \
                  1\r\n\r\n",
            ),
            (
                b"HEAD /b HTTP/1.1\r\n\r\n",
                b"HTTP/1.1 200 OK\r\ncontent-length: 50\r\n\r\n",
            ),
            (
                b"PUT /c HTTP/1.1\r\nExpect: 100-continue\r\nContent-Length: 3\r\n\r\nabc",
                b"HTTP/1.1 100 Continue\r\n\r\nHTTP/1.1 204 No Content\r\n\r\n",
            ),
        ];

        for (request, response) in exchanges {
            for byte in request {
                assert_eq!(watch.observe_requests(&[*byte]), Ok(false));
            }
            for byte in response {
                assert_eq!(watch.observe_responses(&[*byte]), Ok(()));
            }
        }
        let owed: Vec<_> = b"PUT /d HTTP/1.1\r\nExpect: 100-continue\r\n\r\n"
            .iter()
            .map(|byte| watch.observe_requests(&[*byte]))
            .collect();

        assert_eq!(owed.iter().filter(|owes| **owes == Ok(true)).count(), 1);
        assert_eq!(owed.last(), Some(&Ok(true)));
    }
}
