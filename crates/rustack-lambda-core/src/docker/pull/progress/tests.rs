use rstest::rstest;

use super::{MAX_LINE, MAX_RECORDS, MAX_TOTAL, Progress};
use crate::docker::DockerError;

#[rstest]
#[case(b"{}\n".as_slice())]
#[case(b"{\"status\":\"Pull complete\"}\r\n".as_slice())]
#[case(b"{\"error\":null,\"errorDetail\":null,\"aux\":{\"Digest\":\"data\"}}\n".as_slice())]
fn test_should_consume_every_fragment_boundary(#[case] source: &[u8]) {
    for split in 0..=source.len() {
        let mut progress = Progress::default();
        progress.push(&source[..split]).unwrap();
        progress.push(&source[split..]).unwrap();
        progress.finish().unwrap();
    }
}
#[rstest]
#[case(b"\n".as_slice())]
#[case(b"[]\n".as_slice())]
#[case(b"[null,null]\n".as_slice())]
#[case(b"null\n".as_slice())]
#[case(b"\"text\"\n".as_slice())]
#[case(b"{bad}\n".as_slice())]
#[case(b"{\"x\":1,\"x\":2}\n".as_slice())]
fn test_should_reject_nonobject_or_malformed_records(#[case] source: &[u8]) {
    assert!(Progress::default().push(source).is_err());
}
#[rstest]
#[case(b"{\"error\":\"SECRET-PEER\"}\n".as_slice())]
#[case(b"{\"errorDetail\":{\"message\":\"SECRET-PEER\"}}\n".as_slice())]
#[case(b"{\"errorDetail\":{}}\n".as_slice())]
fn test_should_reject_in_band_errors_without_peer_text(#[case] source: &[u8]) {
    let error = Progress::default().push(source).unwrap_err();
    assert!(matches!(error, DockerError::PullFailed));
    assert!(!format!("{error:?}: {error}").contains("SECRET"));
}
#[test]
fn test_should_require_nonempty_complete_lines_at_eof() {
    assert!(Progress::default().finish().is_err());
    let mut progress = Progress::default();
    progress.push(b"{}").unwrap();
    assert!(progress.finish().is_err());
}
#[test]
fn test_should_bound_line_bytes_records_and_total_progress() {
    let mut line = b"{}".to_vec();
    line.resize(MAX_LINE, b' ');
    let mut progress = Progress::default();
    progress.push(&line).unwrap();
    progress.push(b"\r\n").unwrap();
    progress.finish().unwrap();
    line.push(b' ');
    assert!(matches!(
        Progress::default().push(&line),
        Err(DockerError::TooLarge)
    ));
    let source = b"{}\n".repeat(MAX_RECORDS);
    let mut progress = Progress::default();
    progress.push(&source).unwrap();
    assert!(matches!(progress.push(b"{}\n"), Err(DockerError::TooLarge)));
    assert!(matches!(
        Progress::default().push(&vec![b' '; MAX_TOTAL + 1]),
        Err(DockerError::TooLarge)
    ));
}
#[test]
fn test_should_reject_deep_and_oversized_progress_scalars() {
    let long = format!("{{\"status\":\"{}\"}}\n", "x".repeat(8193));
    assert!(Progress::default().push(long.as_bytes()).is_err());
    let deep = format!("{{\"aux\":{}0{}}}\n", "[".repeat(9), "]".repeat(9));
    assert!(Progress::default().push(deep.as_bytes()).is_err());
}
