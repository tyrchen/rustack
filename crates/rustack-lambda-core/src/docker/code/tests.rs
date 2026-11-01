use std::{
    io::{Cursor, Read, Write},
    path::Path,
    sync::{Arc, mpsc as blocking},
    time::Duration,
};

use bytes::Bytes;
use rstest::rstest;
use tar::Archive;
use tempfile::TempDir;
use tokio::{
    fs,
    sync::{Semaphore, oneshot},
    time::{Instant, timeout},
};
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

use super::{
    BoundedWriter, CheckedReader, DockerCodeArchive, convert, validate_elf, with_reservation,
};
use crate::{
    artifact::{Architecture, ValidatedArtifact},
    docker::DockerError,
    error::LambdaServiceError,
    storage::FunctionStore,
};

pub(in crate::docker) fn elf(architecture: Architecture) -> Vec<u8> {
    let mut bytes = vec![0; 64];
    bytes
        .get_mut(..7)
        .unwrap()
        .copy_from_slice(b"\x7fELF\x02\x01\x01");
    bytes
        .get_mut(16..18)
        .unwrap()
        .copy_from_slice(&2u16.to_le_bytes());
    let machine: u16 = match architecture {
        Architecture::Arm64 => 183,
        Architecture::X86_64 => 62,
    };
    bytes
        .get_mut(18..20)
        .unwrap()
        .copy_from_slice(&machine.to_le_bytes());
    bytes
        .get_mut(20..24)
        .unwrap()
        .copy_from_slice(&1u32.to_le_bytes());
    bytes
        .get_mut(52..54)
        .unwrap()
        .copy_from_slice(&64u16.to_le_bytes());
    bytes
}

pub(in crate::docker) fn zip(entries: &[(&str, &[u8], u32)]) -> Vec<u8> {
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes, mode) in entries {
        writer
            .start_file(
                *name,
                SimpleFileOptions::default()
                    .unix_permissions(*mode)
                    .compression_method(CompressionMethod::Stored),
            )
            .unwrap();
        writer.write_all(bytes).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

// Deliberately bypass staging only for converter defense-in-depth controls.
pub(in crate::docker) async fn source(bytes: Vec<u8>) -> (TempDir, ValidatedArtifact) {
    let root = tempfile::tempdir().unwrap();
    let owned = tempfile::tempdir_in(root.path()).unwrap();
    let extracted = owned.path().join("extracted");
    fs::create_dir(&extracted).await.unwrap();
    let artifact = ValidatedArtifact::from_staged(
        owned,
        root.path().to_owned(),
        extracted,
        Bytes::from(bytes),
    );
    (root, artifact)
}
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(10)
}

pub(in crate::docker) async fn prepare(source: ValidatedArtifact) -> DockerCodeArchive {
    prepare_with(source, Arc::new(Semaphore::new(2))).await
}

pub(in crate::docker) async fn prepare_with(
    source: ValidatedArtifact,
    admission: Arc<Semaphore>,
) -> DockerCodeArchive {
    let deadline = deadline();
    with_reservation(admission, deadline, move |permit| {
        convert(source, Architecture::Arm64, permit, deadline)
    })
    .await
    .unwrap()
}

#[rstest]
#[case(Architecture::Arm64)]
#[case(Architecture::X86_64)]
#[tokio::test]
async fn test_should_transfer_original_bytes_with_deterministic_headers_and_long_paths(
    #[case] architecture: Architecture,
) {
    let bootstrap = elf(architecture);
    let path = format!("{}{}", "long/".repeat(203), "v".repeat(9));
    assert_eq!(path.len(), 1024);
    let (_root, artifact) = source(zip(&[
        ("bootstrap", &bootstrap, 0o755),
        (&path, b"original", 0o644),
    ]))
    .await;
    fs::write(artifact.code_root().join("bootstrap"), b"mutated host tree")
        .await
        .unwrap();
    let hash = artifact.sha256().to_owned();
    let first = {
        let artifact = artifact.clone();
        with_reservation(Arc::new(Semaphore::new(1)), deadline(), move |permit| {
            convert(artifact, architecture, permit, deadline())
        })
        .await
        .unwrap()
    };
    let second = with_reservation(Arc::new(Semaphore::new(1)), deadline(), move |permit| {
        convert(artifact, architecture, permit, deadline())
    })
    .await
    .unwrap();
    assert_eq!(first.bytes(), second.bytes());
    assert_eq!(first.code_sha256(), hash);
    assert_eq!(first.architecture(), architecture);
    assert!(!format!("{first:?}").contains("original"));
    let mut archive = Archive::new(Cursor::new(first.bytes()));
    let mut count = 0;
    let mut directories = 0;
    for entry in archive.entries().unwrap() {
        let mut entry = entry.unwrap();
        let name = entry.path().unwrap().to_string_lossy().into_owned();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes).unwrap();
        assert_eq!(entry.header().uid().unwrap(), 0);
        assert_eq!(entry.header().gid().unwrap(), 0);
        assert_eq!(entry.header().mtime().unwrap(), 0);
        if entry.header().entry_type().is_dir() {
            assert!(Path::new(&path).starts_with(Path::new(&name)));
            assert_eq!(entry.header().mode().unwrap(), 0o555);
            assert_eq!(bytes, [] as [u8; 0]);
            directories += 1;
            continue;
        }
        match name.as_str() {
            "bootstrap" => {
                assert_eq!(bytes, bootstrap);
                assert_eq!(entry.header().mode().unwrap(), 0o555);
            }
            name if name == path => {
                assert_eq!(bytes, b"original");
                assert_eq!(entry.header().mode().unwrap(), 0o444);
            }
            _ => panic!("unexpected transfer member"),
        }
        count += 1;
    }
    assert_eq!(count, 2);
    assert_eq!(directories, 203);
}

#[rstest]
#[case::magic(0, 0)]
#[case::class(4, 1)]
#[case::endian(5, 2)]
#[case::ident_version(6, 2)]
#[case::object_type(16, 1)]
#[case::object_type_high(17, 1)]
#[case::machine(18, 62)]
#[case::machine_high(19, 1)]
#[case::version(20, 2)]
#[case::version_high(23, 1)]
#[case::header_size(52, 0)]
#[tokio::test]
async fn test_should_reject_invalid_elf_before_returning_a_transfer(
    #[case] offset: usize,
    #[case] value: u8,
) {
    let mut bytes = elf(Architecture::Arm64);
    *bytes.get_mut(offset).unwrap() = value;
    let (_root, artifact) = source(zip(&[("bootstrap", &bytes, 0o755)])).await;
    assert!(matches!(
        DockerCodeArchive::prepare(artifact, Architecture::Arm64, deadline()).await,
        Err(DockerError::InvalidCode)
    ));
}

#[rstest]
#[case::script(b"#!/bin/sh\nexit 0", 0o755)]
#[case::truncated(b"\x7fELF", 0o755)]
#[case::mach_o(b"\xcf\xfa\xed\xfe", 0o755)]
#[case::not_executable(b"unused", 0o644)]
#[tokio::test]
async fn test_should_reject_non_executable_bootstrap_forms(
    #[case] bytes: &[u8],
    #[case] mode: u32,
) {
    let (_root, artifact) = source(zip(&[("bootstrap", bytes, mode)])).await;
    assert!(matches!(
        DockerCodeArchive::prepare(artifact, Architecture::Arm64, deadline()).await,
        Err(DockerError::InvalidCode)
    ));
}

#[tokio::test]
async fn test_should_accept_pie_but_require_the_complete_current_header() {
    let mut bytes = elf(Architecture::Arm64);
    *bytes.get_mut(16).unwrap() = 3;
    validate_elf(&bytes, Architecture::Arm64).unwrap();
    assert!(validate_elf(bytes.get(..63).unwrap(), Architecture::Arm64).is_err());
    let (_root, artifact) = source(zip(&[("data", b"no bootstrap", 0o644)])).await;
    assert!(matches!(
        DockerCodeArchive::prepare(artifact, Architecture::Arm64, deadline()).await,
        Err(DockerError::InvalidCode)
    ));
}

#[rstest]
#[case::traversal("../outside")]
#[case::absolute("/outside")]
#[case::backslash("folder\\outside")]
#[tokio::test]
async fn test_should_revalidate_hostile_zip_paths(#[case] name: &str) {
    let bootstrap = elf(Architecture::Arm64);
    let (_root, artifact) = source(zip(&[
        ("bootstrap", &bootstrap, 0o755),
        (name, b"hostile", 0o644),
    ]))
    .await;
    assert!(matches!(
        DockerCodeArchive::prepare(artifact, Architecture::Arm64, deadline()).await,
        Err(DockerError::InvalidCode)
    ));
}

#[rstest]
#[case::symlink(0o120_777)]
#[case::fifo(0o010_644)]
#[case::character_device(0o020_644)]
#[case::block_device(0o060_644)]
#[case::socket(0o140_644)]
#[case::setuid(0o104_755)]
#[tokio::test]
async fn test_should_revalidate_zip_special_file_declarations(#[case] mode: u32) {
    let bootstrap = elf(Architecture::Arm64);
    let mut bytes = zip(&[("bootstrap", &bootstrap, 0o755)]);
    let central = bytes
        .windows(4)
        .position(|part| part == b"PK\x01\x02")
        .unwrap();
    bytes
        .get_mut(central + 4..central + 6)
        .unwrap()
        .copy_from_slice(&((3u16 << 8) | 0x14).to_le_bytes());
    bytes
        .get_mut(central + 38..central + 42)
        .unwrap()
        .copy_from_slice(&(mode << 16).to_le_bytes());
    let original = bytes.clone();
    let (_root, artifact) = source(bytes).await;
    assert!(matches!(
        DockerCodeArchive::prepare(artifact, Architecture::Arm64, deadline()).await,
        Err(DockerError::InvalidCode)
    ));
    let root = tempfile::tempdir().unwrap();
    let store = FunctionStore::new(root.path());
    assert!(matches!(
        store
            .store_zip_code("special-file", "$LATEST", &Bytes::from(original))
            .await,
        Err(LambdaServiceError::InvalidZipFile { .. })
    ));
}

#[test]
fn test_should_bound_retained_buffer_capacity_and_reject_without_partial_append() {
    let mut writer = BoundedWriter {
        bytes: Vec::new(),
        deadline: deadline(),
        limit: 16,
    };
    writer.write_all(b"123456789").unwrap();
    assert!(writer.bytes.capacity() <= 16);
    writer.write_all(b"abcdefg").unwrap();
    assert!(writer.bytes.capacity() <= 16);
    assert_eq!(
        writer.write(b"x").unwrap_err().kind(),
        std::io::ErrorKind::FileTooLarge
    );
    assert_eq!(writer.bytes, b"123456789abcdefg");
    writer.deadline = Instant::now();
    assert_eq!(
        writer.write(b"").unwrap_err().kind(),
        std::io::ErrorKind::TimedOut
    );
}

#[tokio::test]
async fn test_should_bind_exactly_two_global_transfer_reservations() {
    let bootstrap = elf(Architecture::Arm64);
    let (_root, source) = source(zip(&[("bootstrap", &bootstrap, 0o755)])).await;
    let first = DockerCodeArchive::prepare(source.clone(), Architecture::Arm64, deadline())
        .await
        .unwrap();
    let second = DockerCodeArchive::prepare(source.clone(), Architecture::Arm64, deadline())
        .await
        .unwrap();
    let error = DockerCodeArchive::prepare(
        source,
        Architecture::Arm64,
        Instant::now() + Duration::from_millis(50),
    )
    .await
    .unwrap_err();
    assert!(matches!(error, DockerError::Deadline));
    drop(first);
    drop(second);
}

#[tokio::test]
async fn test_should_hold_buffer_admission_until_the_last_archive_clone_drops() {
    let bootstrap = elf(Architecture::Arm64);
    let (_root, artifact) = source(zip(&[("bootstrap", &bootstrap, 0o755)])).await;
    let admission = Arc::new(Semaphore::new(1));
    let first = with_reservation(admission.clone(), deadline(), move |permit| {
        convert(artifact, Architecture::Arm64, permit, deadline())
    })
    .await
    .unwrap();
    let clone = first.clone();
    drop(first);
    assert_eq!(admission.available_permits(), 0);
    let error = with_reservation(
        admission.clone(),
        Instant::now() + Duration::from_millis(10),
        Ok,
    )
    .await
    .unwrap_err();
    assert!(matches!(error, DockerError::Deadline));
    drop(clone);
    assert_eq!(admission.available_permits(), 1);
}

#[tokio::test]
async fn test_should_retain_canceled_blocking_work_and_classify_panics() {
    let admission = Arc::new(Semaphore::new(1));
    let (entered, observed) = oneshot::channel();
    let (release, held) = blocking::channel();
    let task = tokio::spawn(with_reservation(
        admission.clone(),
        deadline(),
        move |permit| {
            entered.send(()).unwrap();
            held.recv().unwrap();
            Ok(permit)
        },
    ));
    observed.await.unwrap();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert_eq!(admission.available_permits(), 0);
    release.send(()).unwrap();
    timeout(Duration::from_secs(3), async {
        while admission.available_permits() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let result = with_reservation::<()>(admission.clone(), deadline(), |_| {
        panic!("controlled code worker panic")
    })
    .await;
    assert!(matches!(result, Err(DockerError::Worker)));
    assert_eq!(admission.available_permits(), 1);
}

#[test]
fn test_should_bound_actual_reader_progress_and_expiry() {
    let mut reader = CheckedReader {
        inner: Cursor::new(vec![0; 8193]),
        actual: 0,
        remaining: 8193,
        deadline: deadline(),
    };
    let mut first = vec![0; 16_384];
    assert_eq!(reader.read(&mut first).unwrap(), 8192);
    assert_eq!(reader.read(&mut first).unwrap(), 1);
    assert!(reader.read(&mut first).is_err());
    reader.deadline = Instant::now();
    assert_eq!(
        reader.read(&mut first).unwrap_err().kind(),
        std::io::ErrorKind::TimedOut
    );
}

#[tokio::test]
async fn test_should_read_empty_directory_streams_before_accepting_their_metadata() {
    let bootstrap = elf(Architecture::Arm64);
    let mut bytes = zip(&[
        ("bootstrap", &bootstrap, 0o755),
        ("directory/", b"\x07", 0o755),
    ]);
    let local = bytes
        .windows(4)
        .rposition(|part| part == b"PK\x03\x04")
        .unwrap();
    let central = bytes
        .windows(4)
        .rposition(|part| part == b"PK\x01\x02")
        .unwrap();
    bytes
        .get_mut(local + 8..local + 10)
        .unwrap()
        .copy_from_slice(&8u16.to_le_bytes());
    bytes.get_mut(local + 14..local + 18).unwrap().fill(0);
    bytes.get_mut(local + 22..local + 26).unwrap().fill(0);
    bytes
        .get_mut(central + 10..central + 12)
        .unwrap()
        .copy_from_slice(&8u16.to_le_bytes());
    bytes.get_mut(central + 16..central + 20).unwrap().fill(0);
    bytes.get_mut(central + 24..central + 28).unwrap().fill(0);
    bytes
        .get_mut(central + 38..central + 42)
        .unwrap()
        .copy_from_slice(&(0o040_755u32 << 16).to_le_bytes());
    let original = bytes.clone();
    let (_root, artifact) = source(bytes).await;
    assert!(matches!(
        DockerCodeArchive::prepare(artifact, Architecture::Arm64, deadline()).await,
        Err(DockerError::InvalidCode)
    ));
    let root = tempfile::tempdir().unwrap();
    let store = FunctionStore::new(root.path());
    assert!(matches!(
        store
            .store_zip_code("directory", "$LATEST", &Bytes::from(original))
            .await,
        Err(LambdaServiceError::InvalidZipFile { .. })
    ));
}

#[rstest]
#[case::crc(false)]
#[case::declared_expansion(true)]
#[tokio::test]
async fn test_should_reject_regular_crc_or_declared_expansion_before_exposing_code(
    #[case] oversized: bool,
) {
    let bootstrap = elf(Architecture::Arm64);
    let mut bytes = zip(&[("bootstrap", &bootstrap, 0o755)]);
    let central = bytes
        .windows(4)
        .position(|part| part == b"PK\x01\x02")
        .unwrap();
    if oversized {
        let size = 250u32 * 1024 * 1024 + 1;
        bytes
            .get_mut(22..26)
            .unwrap()
            .copy_from_slice(&size.to_le_bytes());
        bytes
            .get_mut(central + 24..central + 28)
            .unwrap()
            .copy_from_slice(&size.to_le_bytes());
    } else {
        // Leave the complete checked ELF identity intact and corrupt only CRC.
        bytes.get_mut(14..18).unwrap().fill(0);
        bytes.get_mut(central + 16..central + 20).unwrap().fill(0);
    }
    let (_root, artifact) = source(bytes).await;
    let error = DockerCodeArchive::prepare(artifact, Architecture::Arm64, deadline())
        .await
        .unwrap_err();
    assert!(matches!(
        (oversized, error),
        (true, DockerError::TooLarge) | (false, DockerError::InvalidCode)
    ));
}

#[tokio::test]
async fn test_should_consume_valid_compressed_empty_directory_and_emit_each_parent_once() {
    let bootstrap = elf(Architecture::Arm64);
    let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
    for (name, data) in [
        ("bootstrap", bootstrap.as_slice()),
        ("parent/file", b"data".as_slice()),
        ("parent/", b"".as_slice()),
    ] {
        writer
            .start_file(
                name,
                SimpleFileOptions::default()
                    .unix_permissions(0o755)
                    .compression_method(CompressionMethod::Deflated),
            )
            .unwrap();
        writer.write_all(data).unwrap();
    }
    let mut bytes = writer.finish().unwrap().into_inner();
    let central = bytes
        .windows(4)
        .rposition(|part| part == b"PK\x01\x02")
        .unwrap();
    bytes
        .get_mut(central + 38..central + 42)
        .unwrap()
        .copy_from_slice(&(0o040_755u32 << 16).to_le_bytes());
    let (_root, source) = source(bytes).await;
    let code = prepare(source).await;
    let mut archive = Archive::new(Cursor::new(code.bytes()));
    let names: Vec<_> = archive
        .entries()
        .unwrap()
        .map(|entry| entry.unwrap().path().unwrap().into_owned())
        .collect();
    assert_eq!(
        names,
        vec![
            Path::new("bootstrap"),
            Path::new("parent"),
            Path::new("parent/file")
        ]
    );
}
