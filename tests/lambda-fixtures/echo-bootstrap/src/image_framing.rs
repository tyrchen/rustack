//! Independent, bounded raw Runtime HTTP profile probes from a consumed RIC.

use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    time::{Duration, Instant},
};

use crate::{Result, request_id};

const EXCHANGE: Duration = Duration::from_secs(5);
const SENTINEL: &str = "runtime-boundary-secret";

fn remaining(deadline: Instant) -> Result<Duration> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|value| !value.is_zero())
        .ok_or("fixture Runtime HTTP deadline")
}

fn status(raw: &[u8]) -> Result<u16> {
    if raw.len() > 2048 || !raw.windows(4).any(|part| part == b"\r\n\r\n") {
        return Err("fixture Runtime HTTP response cap or truncation");
    }
    if raw
        .windows(SENTINEL.len())
        .any(|part| part == SENTINEL.as_bytes())
    {
        return Err("fixture Runtime HTTP diagnostic leaked peer bytes");
    }
    let first = raw
        .split(|byte| *byte == b'\n')
        .next()
        .ok_or("missing HTTP status")?;
    let text = std::str::from_utf8(first).map_err(|_| "invalid HTTP status encoding")?;
    let mut fields = text.split_ascii_whitespace();
    if fields.next() != Some("HTTP/1.1") {
        return Err("invalid HTTP status protocol");
    }
    let number = fields.next().ok_or("missing HTTP status code")?;
    if number.len() != 3 || !number.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err("invalid HTTP status code");
    }
    number.parse().map_err(|_| "invalid HTTP status code")
}

fn exchange(address: SocketAddr, request: &[u8]) -> Result<u16> {
    let deadline = Instant::now() + EXCHANGE;
    if request.len() > 4096 || !address.ip().is_loopback() || address.port() == 0 {
        return Err("invalid fixture Runtime HTTP request");
    }
    let mut stream = TcpStream::connect_timeout(&address, remaining(deadline)?)
        .map_err(|_| "fixture Runtime HTTP connect failed")?;
    stream
        .set_write_timeout(Some(remaining(deadline)?))
        .map_err(|_| "fixture Runtime HTTP write budget failed")?;
    stream
        .write_all(request)
        .map_err(|_| "fixture Runtime HTTP request failed")?;
    read_response(&mut stream, deadline)
}

fn read_response(stream: &mut TcpStream, deadline: Instant) -> Result<u16> {
    let mut raw = Vec::new();
    let mut block = [0; 256];
    loop {
        stream
            .set_read_timeout(Some(remaining(deadline)?))
            .map_err(|_| "fixture Runtime HTTP read budget failed")?;
        let count = stream
            .read(&mut block)
            .map_err(|_| "fixture Runtime HTTP response failed")?;
        remaining(deadline)?;
        if count == 0 {
            return status(&raw);
        }
        if raw.len().saturating_add(count) > 2048 {
            return Err("fixture Runtime HTTP response exceeds limit");
        }
        raw.extend_from_slice(
            block
                .get(..count)
                .ok_or("invalid fixture HTTP read count")?,
        );
    }
}

fn requests(id: &str) -> Result<Vec<(String, u16)>> {
    request_id(id)?;
    let completion = format!("/2018-06-01/runtime/invocation/{id}/response");
    let next = "/2018-06-01/runtime/invocation/next";
    let headers = "HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n";
    let long = format!("{SENTINEL}{}", "x".repeat(257 - SENTINEL.len()));
    Ok(vec![
        (
            format!("GET {completion} {headers}Content-Length: 0\r\n\r\n"),
            405,
        ),
        (
            format!("POST {completion} {headers}X-Boundary: {long}\r\nContent-Length: 0\r\n\r\n"),
            431,
        ),
        (
            format!("GET {next} {headers}Content-Length: 1\r\n\r\ns"),
            413,
        ),
        (
            format!("POST {completion} {headers}Content-Length: 6291457\r\n\r\n"),
            413,
        ),
        (
            format!(
                "POST {completion} {headers}Transfer-Encoding: \
                 chunked\r\n\r\n2\r\n{{}}\r\n0\r\nX-Boundary: {SENTINEL}\r\n\r\n"
            ),
            400,
        ),
    ])
}

/// Reject fixed out-of-profile requests without consuming the active attempt.
///
/// # Errors
/// Rejects invalid fixture inputs, bounded IO failure or an unexpected status.
pub(super) fn check(root: &str, id: &str) -> Result<Vec<u16>> {
    let (authority, path) = root
        .strip_prefix("http://")
        .and_then(|root| root.split_once('/'))
        .ok_or("invalid fixture Runtime HTTP root")?;
    if path != "2018-06-01/runtime" {
        return Err("invalid fixture Runtime HTTP root path");
    }
    let address: SocketAddr = authority
        .parse()
        .map_err(|_| "invalid fixture Runtime HTTP address")?;
    requests(id)?
        .into_iter()
        .map(|(request, expected)| {
            let actual = exchange(address, request.as_bytes())?;
            if actual != expected {
                return Err("fixture Runtime HTTP request was not rejected correctly");
            }
            Ok(actual)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::{io::ErrorKind, net::TcpListener, sync::mpsc, thread};

    use super::*;

    #[test]
    fn test_should_reject_oversized_truncated_invalid_and_leaking_http_replies() {
        for raw in [
            b"HTTP/1.1 413 Empty\r\n".as_slice(),
            b"HTTP/1.0 413 Empty\r\n\r\n",
            b"HTTP/1.1 nope Empty\r\n\r\n",
            b"HTTP/1.1 400 Empty\r\nX: runtime-boundary-secret\r\n\r\n",
            &[b'x'; 2049],
        ] {
            assert!(status(raw).is_err());
        }
        assert_eq!(
            status(b"HTTP/1.1 413 Empty\r\nContent-Length: 0\r\n\r\n"),
            Ok(413)
        );
        assert!(remaining(Instant::now()).is_err());
    }

    #[test]
    fn test_should_expire_a_live_peer_before_it_can_publish_a_late_response() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let (entered, seen) = mpsc::channel();
        let (release, released) = mpsc::channel();
        let worker = thread::spawn(move || {
            let deadline = Instant::now() + EXCHANGE;
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(pair) => break pair,
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline);
                        thread::sleep(Duration::from_millis(1));
                    }
                    Err(error) => panic!("fixture accept failed: {error}"),
                }
            };
            stream.set_read_timeout(Some(EXCHANGE)).unwrap();
            stream.set_write_timeout(Some(EXCHANGE)).unwrap();
            let mut request = [0; 1];
            stream.read_exact(&mut request).unwrap();
            entered.send(()).unwrap();
            released.recv_timeout(EXCHANGE).unwrap();
            let _ = stream.write_all(b"HTTP/1.1 400 Empty\r\nContent-Length: 0\r\n\r\n");
        });
        let mut observed = false;
        let result = (|| {
            let mut stream = TcpStream::connect_timeout(&address, EXCHANGE)
                .map_err(|_| "fixture setup connection failed")?;
            stream
                .set_write_timeout(Some(EXCHANGE))
                .map_err(|_| "fixture setup write budget")?;
            stream
                .write_all(b"x")
                .map_err(|_| "fixture setup write failed")?;
            seen.recv_timeout(EXCHANGE)
                .map_err(|_| "fixture setup observation failed")?;
            observed = true;
            // Only the already observed response wait is deliberately short.
            read_response(&mut stream, Instant::now() + Duration::from_millis(100))
        })();
        let signaled = release.send(());
        let joined = worker.join();
        assert!(observed && signaled.is_ok() && joined.is_ok() && result.is_err());
    }

    #[test]
    fn test_should_send_independent_capped_requests_over_a_joined_loopback_socket() {
        let requests = requests("11111111-1111-4111-8111-111111111111").unwrap();
        assert_eq!(
            requests
                .iter()
                .map(|(_, status)| *status)
                .collect::<Vec<_>>(),
            [405, 431, 413, 413, 400]
        );
        for (request, expected) in requests {
            assert!(request.len() <= 4096);
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let address = listener.local_addr().unwrap();
            let bytes = request.clone();
            let worker = thread::spawn(move || {
                let deadline = Instant::now() + EXCHANGE;
                let (mut stream, _) = loop {
                    match listener.accept() {
                        Ok(pair) => break pair,
                        Err(error) if error.kind() == ErrorKind::WouldBlock => {
                            assert!(Instant::now() < deadline, "fixture accept deadline");
                            thread::sleep(Duration::from_millis(1));
                        }
                        Err(error) => panic!("fixture accept failed: {error}"),
                    }
                };
                stream.set_read_timeout(Some(EXCHANGE)).unwrap();
                stream.set_write_timeout(Some(EXCHANGE)).unwrap();
                let mut received = vec![0; bytes.len()];
                stream.read_exact(&mut received).unwrap();
                assert_eq!(received, bytes.as_bytes());
                stream
                    .write_all(
                        format!("HTTP/1.1 {expected} Empty\r\nContent-Length: 0\r\n\r\n")
                            .as_bytes(),
                    )
                    .unwrap();
            });
            let actual = exchange(address, request.as_bytes());
            worker.join().unwrap();
            assert_eq!(actual, Ok(expected));
        }
    }
}
