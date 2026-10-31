//! Bounded raw archives of persistent installation identity; never extraction.

use std::{
    collections::BTreeSet,
    io::{Cursor, Read},
};

use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use tar::Archive;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Identity {
    pub owner: Uuid,
    pub engine: Uuid,
}

pub(super) fn identity(owner: &[u8], daemon: &[u8], engine: Uuid) -> Result<Identity> {
    let owner = record(owner, "owner.v1", *b"RSDKOWN2", 56)?;
    let daemon = record(daemon, "daemon.v1", *b"RSDKDMD1", 72)?;
    let id = Uuid::from_slice(owner.get(8..24).context("owner identity bytes")?)?;
    let bound_owner = Uuid::from_slice(daemon.get(8..24).context("bound owner bytes")?)?;
    let bound_engine = Uuid::from_slice(daemon.get(24..40).context("bound engine bytes")?)?;
    ensure!(
        !id.is_nil() && id == bound_owner && engine == bound_engine && !engine.is_nil(),
        "installation identity affiliation differs"
    );
    Ok(Identity { owner: id, engine })
}

fn record(raw: &[u8], name: &str, magic: [u8; 8], length: usize) -> Result<Vec<u8>> {
    ensure!(
        (2048..=32768).contains(&raw.len()) && raw.len().is_multiple_of(512),
        "identity archive byte bound"
    );
    let mut archive = Archive::new(Cursor::new(raw));
    let mut payload = None;
    let mut timing_seen = false;
    for (index, entry) in archive.entries()?.raw(true).enumerate() {
        ensure!(index < 2, "extra identity archive member");
        let mut entry = entry?;
        let path = entry.path_bytes();
        ensure!(
            !path.is_empty() && path.len() <= 256,
            "identity archive path bound"
        );
        if entry.header().entry_type().is_pax_local_extensions() {
            ensure!(
                index == 0 && !timing_seen && entry.size() <= 1024,
                "identity timing metadata bound"
            );
            ensure!(
                path.is_ascii()
                    && path.split(|byte| *byte == b'/').all(|part| !part.is_empty()
                        && part != b"."
                        && part != b".."
                        && part
                            .iter()
                            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(b))),
                "identity timing metadata path"
            );
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes)?;
            timing(&bytes)?;
            timing_seen = true;
            continue;
        }
        ensure!(
            payload.is_none()
                && path.as_ref() == name.as_bytes()
                && entry.header().entry_type().is_file()
                && entry.size() == u64::try_from(length)?
                && entry.header().uid()? == 65532
                && entry.header().gid()? == 65532
                && entry.header().mode()? == 0o600
                && entry.link_name_bytes().is_none(),
            "identity archive member differs"
        );
        let mut bytes = Vec::with_capacity(length);
        entry.read_to_end(&mut bytes)?;
        ensure!(bytes.len() == length, "truncated installation identity");
        payload = Some(bytes);
    }
    let position = usize::try_from(archive.into_inner().position())?;
    let tail = raw.get(position..).context("identity archive cursor")?;
    ensure!(
        tail.len() >= 512 && tail.iter().all(|byte| *byte == 0),
        "identity archive terminator or trailing content"
    );
    let bytes = payload.context("missing installation identity")?;
    let end = length.checked_sub(32).context("identity checksum length")?;
    let data = bytes.get(..end).context("identity checksum payload")?;
    ensure!(
        bytes.get(..8) == Some(magic.as_slice())
            && bytes.get(end..) == Some(Sha256::digest(data).as_slice()),
        "installation identity checksum or generation differs"
    );
    Ok(bytes)
}

fn timing(mut bytes: &[u8]) -> Result<()> {
    let mut seen = BTreeSet::new();
    while !bytes.is_empty() {
        ensure!(seen.len() < 4, "identity timing field count");
        let space = bytes
            .iter()
            .position(|byte| *byte == b' ')
            .context("PAX timing length")?;
        let digits = bytes.get(..space).context("PAX timing length digits")?;
        ensure!(
            (1..=4).contains(&digits.len()) && digits.iter().all(u8::is_ascii_digit),
            "PAX timing length grammar"
        );
        let length = std::str::from_utf8(digits)?.parse::<usize>()?;
        let line = bytes.get(..length).context("PAX timing record bound")?;
        ensure!(line.last() == Some(&b'\n'), "PAX timing newline");
        let content = line
            .get(space.saturating_add(1)..length.saturating_sub(1))
            .context("PAX timing content")?;
        let content = std::str::from_utf8(content)?;
        let (key, value) = content.split_once('=').context("PAX timing pair")?;
        ensure!(
            matches!(key, "mtime" | "atime" | "ctime")
                && seen.insert(key)
                && (1..=64).contains(&value.len()),
            "unexpected identity timing field"
        );
        let unsigned = value.strip_prefix('-').unwrap_or(value);
        let mut parts = unsigned.split('.');
        let integral = parts.next().context("PAX timing integer")?;
        ensure!(
            !integral.is_empty() && integral.bytes().all(|byte| byte.is_ascii_digit()),
            "PAX timing integer grammar"
        );
        if let Some(fraction) = parts.next() {
            ensure!(
                (1..=9).contains(&fraction.len())
                    && fraction.bytes().all(|byte| byte.is_ascii_digit()),
                "PAX timing fraction grammar"
            );
        }
        ensure!(parts.next().is_none(), "PAX timing extra component");
        bytes = bytes.get(length..).context("PAX timing next record")?;
    }
    ensure!(!seen.is_empty(), "empty identity timing metadata");
    Ok(())
}

#[cfg(test)]
mod tests {
    use tar::{Builder, EntryType, Header};

    use super::*;

    fn payload(magic: [u8; 8], owner: Uuid, engine: Option<Uuid>) -> Vec<u8> {
        let mut bytes = magic.to_vec();
        bytes.extend_from_slice(owner.as_bytes());
        if let Some(engine) = engine {
            bytes.extend_from_slice(engine.as_bytes());
        }
        bytes.extend_from_slice(&Sha256::digest(&bytes));
        bytes
    }
    fn append(
        builder: &mut Builder<Vec<u8>>,
        name: &str,
        bytes: &[u8],
        kind: EntryType,
        uid: u64,
        mode: u32,
    ) -> Result<()> {
        let mut header = Header::new_ustar();
        header.set_entry_type(kind);
        header.set_uid(uid);
        header.set_gid(65532);
        header.set_mode(mode);
        header.set_size(u64::try_from(bytes.len())?);
        header.set_cksum();
        builder.append_data(&mut header, name, bytes)?;
        Ok(())
    }
    fn archive(name: &str, bytes: &[u8]) -> Result<Vec<u8>> {
        let mut builder = Builder::new(Vec::new());
        append(&mut builder, name, bytes, EntryType::Regular, 65532, 0o600)?;
        Ok(builder.into_inner()?)
    }
    #[test]
    fn test_should_bind_the_actual_persisted_owner_and_engine_records() -> Result<()> {
        let owner = Uuid::new_v4();
        let engine = Uuid::new_v4();
        let a = archive("owner.v1", &payload(*b"RSDKOWN2", owner, None))?;
        let b = archive("daemon.v1", &payload(*b"RSDKDMD1", owner, Some(engine)))?;
        assert_eq!(identity(&a, &b, engine)?, Identity { owner, engine });
        assert!(identity(&a, &b, Uuid::new_v4()).is_err());
        let foreign = archive(
            "daemon.v1",
            &payload(*b"RSDKDMD1", Uuid::new_v4(), Some(engine)),
        )?;
        assert!(identity(&a, &foreign, engine).is_err());
        let nil = archive("owner.v1", &payload(*b"RSDKOWN2", Uuid::nil(), None))?;
        assert!(identity(&nil, &b, engine).is_err());
        Ok(())
    }
    #[test]
    fn test_should_reject_record_replacement_paths_types_permissions_and_checksums() -> Result<()> {
        let bytes = payload(*b"RSDKOWN2", Uuid::new_v4(), None);
        for (name, kind, uid, mode) in [
            ("unrelated", EntryType::Regular, 65532, 0o600),
            ("sub/owner.v1", EntryType::Regular, 65532, 0o600),
            ("owner.v1", EntryType::Symlink, 65532, 0o600),
            ("owner.v1", EntryType::Regular, 0, 0o600),
            ("owner.v1", EntryType::Regular, 65532, 0o644),
        ] {
            let mut builder = Builder::new(Vec::new());
            append(&mut builder, name, &bytes, kind, uid, mode)?;
            assert!(record(&builder.into_inner()?, "owner.v1", *b"RSDKOWN2", 56).is_err());
        }
        let mut bad = bytes.clone();
        *bad.last_mut().context("test checksum byte")? ^= 1;
        assert!(record(&archive("owner.v1", &bad)?, "owner.v1", *b"RSDKOWN2", 56).is_err());
        assert!(record(&archive("owner.v1", &bytes)?, "owner.v1", *b"RSDKOWN3", 56).is_err());
        Ok(())
    }
    #[test]
    fn test_should_reject_duplicate_extra_truncated_and_concatenated_archives() -> Result<()> {
        let bytes = payload(*b"RSDKOWN2", Uuid::new_v4(), None);
        let mut builder = Builder::new(Vec::new());
        append(
            &mut builder,
            "owner.v1",
            &bytes,
            EntryType::Regular,
            65532,
            0o600,
        )?;
        append(
            &mut builder,
            "owner.v1",
            &bytes,
            EntryType::Regular,
            65532,
            0o600,
        )?;
        assert!(record(&builder.into_inner()?, "owner.v1", *b"RSDKOWN2", 56).is_err());
        let raw = archive("owner.v1", &bytes)?;
        assert!(
            record(
                raw.get(..1024).context("test short archive")?,
                "owner.v1",
                *b"RSDKOWN2",
                56
            )
            .is_err()
        );
        let mut joined = raw.clone();
        joined.extend_from_slice(&raw);
        assert!(record(&joined, "owner.v1", *b"RSDKOWN2", 56).is_err());
        assert!(record(&vec![0; 33280], "owner.v1", *b"RSDKOWN2", 56).is_err());
        Ok(())
    }
    #[test]
    fn test_should_allow_only_bounded_unique_timing_pax_metadata() -> Result<()> {
        timing(b"13 mtime=1.2\n")?;
        for bytes in [
            b"12 path=abc\n".as_slice(),
            b"13 mtime=1.2\n13 mtime=1.2\n",
            b"13 mtime=1e2\n",
            b"9999 mtime=1\n",
            b"",
            b"0 \n",
        ] {
            assert!(timing(bytes).is_err());
        }
        let bytes = payload(*b"RSDKOWN2", Uuid::new_v4(), None);
        let mut builder = Builder::new(Vec::new());
        append(
            &mut builder,
            "PaxHeaders.0/owner.v1",
            b"13 mtime=1.2\n",
            EntryType::XHeader,
            65532,
            0o600,
        )?;
        append(
            &mut builder,
            "owner.v1",
            &bytes,
            EntryType::Regular,
            65532,
            0o600,
        )?;
        assert_eq!(
            record(&builder.into_inner()?, "owner.v1", *b"RSDKOWN2", 56)?,
            bytes
        );
        Ok(())
    }
}
