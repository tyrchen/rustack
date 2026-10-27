//! Borrowed structural validation before typed snapshot CBOR allocation.
//!
//! The v2 profile intentionally excludes tags, indefinite strings and compound
//! map keys. All generated snapshot DTOs use scalar keys and definite strings.

#![forbid(unsafe_code)]

use std::{
    collections::BTreeSet,
    error::Error as StdError,
    fmt,
    io::{self, Cursor, Write},
    str,
};

use serde::{Serialize, de::DeserializeOwned};

const TYPED_RECURSION_LIMIT: usize = 256;

const LIMITS: Limits = Limits {
    bytes: 2 * 1024 * 1024 * 1024,
    depth: 128,
    nodes: 8_000_000,
    collection: 1_000_000,
    scalar: 128 * 1024 * 1024,
    key: 128 * 1024 * 1024,
};

#[derive(Debug, Clone, Copy)]
struct Limits {
    bytes: usize,
    depth: usize,
    nodes: usize,
    collection: usize,
    scalar: usize,
    key: usize,
}

/// Fixed classes: parser/serde diagnostics must never echo snapshot data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Failure {
    Bytes,
    Depth,
    Nodes,
    Collection,
    Scalar,
    Key,
    Duplicate,
    Syntax,
    Profile,
    Trailing,
    Schema,
    Encoding,
    Allocation,
}
impl fmt::Display for Failure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Bytes => "snapshot CBOR byte limit exceeded",
            Self::Depth => "snapshot CBOR depth limit exceeded",
            Self::Nodes => "snapshot CBOR value limit exceeded",
            Self::Collection => "snapshot CBOR collection limit exceeded",
            Self::Scalar => "snapshot CBOR scalar limit exceeded",
            Self::Key => "snapshot CBOR key limit exceeded",
            Self::Duplicate => "duplicate snapshot CBOR key",
            Self::Syntax => "invalid snapshot CBOR syntax",
            Self::Profile => "unsupported snapshot CBOR profile",
            Self::Trailing => "trailing snapshot CBOR data",
            Self::Schema => "invalid snapshot CBOR schema",
            Self::Encoding => "failed to encode snapshot CBOR",
            Self::Allocation => "snapshot CBOR allocation failed",
        })
    }
}
impl StdError for Failure {}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Key<'a> {
    Unsigned(u64),
    Negative(u64),
    Text(&'a str),
}

struct Scan<'a> {
    input: &'a [u8],
    offset: usize,
    remaining: usize,
    limits: Limits,
}
impl<'a> Scan<'a> {
    fn validate(input: &'a [u8], limits: Limits) -> Result<(), Failure> {
        if input.len() > limits.bytes {
            return Err(Failure::Bytes);
        }
        let mut scan = Self {
            input,
            offset: 0,
            remaining: limits.nodes,
            limits,
        };
        scan.item(0)?;
        if scan.offset != input.len() {
            return Err(Failure::Trailing);
        }
        Ok(())
    }
    fn read(&mut self, len: usize) -> Result<&'a [u8], Failure> {
        let end = self.offset.checked_add(len).ok_or(Failure::Syntax)?;
        let bytes = self.input.get(self.offset..end).ok_or(Failure::Syntax)?;
        self.offset = end;
        Ok(bytes)
    }
    fn byte(&mut self) -> Result<u8, Failure> {
        self.read(1)?.first().copied().ok_or(Failure::Syntax)
    }
    fn header(&mut self) -> Result<(u8, Option<u64>), Failure> {
        self.remaining = self.remaining.checked_sub(1).ok_or(Failure::Nodes)?;
        let initial = self.byte()?;
        let major = initial >> 5;
        let value = match initial & 31 {
            value @ 0..=23 => Some(u64::from(value)),
            24 => {
                let value = self.byte()?;
                if major == 7 && value < 32 {
                    return Err(Failure::Syntax);
                }
                Some(u64::from(value))
            }
            25 => Some(u64::from(u16::from_be_bytes(
                self.read(2)?.try_into().map_err(|_| Failure::Syntax)?,
            ))),
            26 => Some(u64::from(u32::from_be_bytes(
                self.read(4)?.try_into().map_err(|_| Failure::Syntax)?,
            ))),
            27 => Some(u64::from_be_bytes(
                self.read(8)?.try_into().map_err(|_| Failure::Syntax)?,
            )),
            31 => None,
            _ => return Err(Failure::Syntax),
        };
        Ok((major, value))
    }
    fn scalar(&mut self, len: Option<u64>, key: bool) -> Result<&'a [u8], Failure> {
        let len = len.ok_or(Failure::Profile)?;
        let len = usize::try_from(len).map_err(|_| Failure::Scalar)?;
        if key && len > self.limits.key {
            return Err(Failure::Key);
        }
        if len > self.limits.scalar {
            return Err(Failure::Scalar);
        }
        self.read(len)
    }
    fn key(&mut self) -> Result<Key<'a>, Failure> {
        let (major, value) = self.header()?;
        match major {
            0 => Ok(Key::Unsigned(value.ok_or(Failure::Syntax)?)),
            1 => Ok(Key::Negative(value.ok_or(Failure::Syntax)?)),
            3 => Ok(Key::Text(
                str::from_utf8(self.scalar(value, true)?).map_err(|_| Failure::Syntax)?,
            )),
            _ => Err(Failure::Profile),
        }
    }
    fn item(&mut self, depth: usize) -> Result<(), Failure> {
        let (major, value) = self.header()?;
        match major {
            0 | 1 | 7 => {
                value.ok_or(Failure::Syntax)?;
            }
            2 => {
                self.scalar(value, false)?;
            }
            3 => {
                str::from_utf8(self.scalar(value, false)?).map_err(|_| Failure::Syntax)?;
            }
            4 | 5 => {
                let depth = depth.checked_add(1).ok_or(Failure::Depth)?;
                if depth > self.limits.depth {
                    return Err(Failure::Depth);
                }
                self.collection(major == 5, value, depth)?;
            }
            _ => return Err(Failure::Profile),
        }
        Ok(())
    }
    fn collection(&mut self, map: bool, count: Option<u64>, depth: usize) -> Result<(), Failure> {
        let count = count
            .map(|count| usize::try_from(count).map_err(|_| Failure::Collection))
            .transpose()?;
        if let Some(count) = count {
            if count > self.limits.collection {
                return Err(Failure::Collection);
            }
            let minimum = count
                .checked_mul(if map { 2 } else { 1 })
                .ok_or(Failure::Collection)?;
            if minimum > self.remaining {
                return Err(Failure::Nodes);
            }
            if minimum > self.input.len().saturating_sub(self.offset) {
                return Err(Failure::Syntax);
            }
        }
        let mut keys = BTreeSet::new();
        let mut visited = 0_usize;
        loop {
            if count == Some(visited) {
                return Ok(());
            }
            if count.is_none() && self.input.get(self.offset) == Some(&0xff) {
                self.byte()?;
                return Ok(());
            }
            visited = visited.checked_add(1).ok_or(Failure::Collection)?;
            if visited > self.limits.collection {
                return Err(Failure::Collection);
            }
            if map && !keys.insert(self.key()?) {
                return Err(Failure::Duplicate);
            }
            self.item(depth)?;
        }
    }
}

struct CappedWriter {
    bytes: Vec<u8>,
    limit: usize,
    failure: Option<Failure>,
}
impl Write for CappedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let result = self.append(bytes);
        if let Err(failure) = result {
            self.failure = Some(failure);
        }
        result.map_err(io::Error::other)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl CappedWriter {
    fn append(&mut self, bytes: &[u8]) -> Result<usize, Failure> {
        let next = self
            .bytes
            .len()
            .checked_add(bytes.len())
            .filter(|next| *next <= self.limit)
            .ok_or(Failure::Bytes)?;
        if next > self.bytes.capacity() {
            let capacity = self
                .bytes
                .capacity()
                .saturating_mul(2)
                .max(next)
                .min(self.limit);
            self.bytes
                .try_reserve_exact(capacity.saturating_sub(self.bytes.len()))
                .map_err(|_| Failure::Allocation)?;
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
}

pub(super) fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, Failure> {
    encode_with_limits(value, LIMITS)
}
#[cfg(feature = "stepfunctions")]
pub(super) fn encode_bounded<T: Serialize>(value: &T, bytes: usize) -> Result<Vec<u8>, Failure> {
    encode_with_limits(
        value,
        Limits {
            bytes: bytes.min(LIMITS.bytes),
            ..LIMITS
        },
    )
}
fn encode_with_limits<T: Serialize>(value: &T, limits: Limits) -> Result<Vec<u8>, Failure> {
    let mut writer = CappedWriter {
        bytes: Vec::new(),
        limit: limits.bytes,
        failure: None,
    };
    let result = ciborium::into_writer(value, &mut writer);
    if let Some(failure) = writer.failure {
        return Err(failure);
    }
    result.map_err(|_| Failure::Encoding)?;
    Scan::validate(&writer.bytes, limits)?;
    Ok(writer.bytes)
}

pub(super) fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, Failure> {
    decode_with_limits(bytes, LIMITS)
}
fn decode_with_limits<T: DeserializeOwned>(bytes: &[u8], limits: Limits) -> Result<T, Failure> {
    Scan::validate(bytes, limits)?;
    let mut reader = Cursor::new(bytes);
    let value = ciborium::de::from_reader_with_recursion_limit(&mut reader, TYPED_RECURSION_LIMIT)
        .map_err(|_| Failure::Schema)?;
    if usize::try_from(reader.position()).map_err(|_| Failure::Trailing)? != bytes.len() {
        return Err(Failure::Trailing);
    }
    Ok(value)
}

#[cfg(test)]
mod tests;
