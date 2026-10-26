//! Allocation-bounded CBOR structural validation before Serde materialization.

use std::{collections::HashSet, str};

use super::{MAX_FRAME_BYTES, MAX_METADATA_BYTES, MAX_PAYLOAD_BYTES, ProtocolError};

#[derive(Debug)]
struct Scanner<'a> {
    remaining: &'a [u8],
    members: usize,
    payload: Option<usize>,
}

pub(super) fn validate(body: &[u8]) -> Result<(), ProtocolError> {
    if body.len() > MAX_FRAME_BYTES {
        return Err(ProtocolError::TooLarge);
    }
    let mut scanner = Scanner {
        remaining: body,
        members: 0,
        payload: None,
    };
    scanner.item(1, false)?;
    if !scanner.remaining.is_empty() {
        return Err(ProtocolError::InvalidStructure);
    }
    if body.len().saturating_sub(scanner.payload.unwrap_or(0)) > MAX_METADATA_BYTES {
        return Err(ProtocolError::TooLarge);
    }
    Ok(())
}

impl<'a> Scanner<'a> {
    fn take(&mut self, length: usize) -> Result<&'a [u8], ProtocolError> {
        let (value, rest) = self
            .remaining
            .split_at_checked(length)
            .ok_or(ProtocolError::InvalidStructure)?;
        self.remaining = rest;
        Ok(value)
    }

    fn header(&mut self) -> Result<(u8, u64), ProtocolError> {
        let (&initial, rest) = self
            .remaining
            .split_first()
            .ok_or(ProtocolError::InvalidStructure)?;
        self.remaining = rest;
        let major = initial >> 5;
        if major == 7 && initial != 0xf4 && initial != 0xf5 {
            return Err(ProtocolError::InvalidStructure);
        }
        let additional = initial & 31;
        let value = match additional {
            0..=23 => u64::from(additional),
            24 => u64::from(u8::from_be_bytes(
                self.take(1)?
                    .try_into()
                    .map_err(|_| ProtocolError::InvalidStructure)?,
            )),
            25 => u64::from(u16::from_be_bytes(
                self.take(2)?
                    .try_into()
                    .map_err(|_| ProtocolError::InvalidStructure)?,
            )),
            26 => u64::from(u32::from_be_bytes(
                self.take(4)?
                    .try_into()
                    .map_err(|_| ProtocolError::InvalidStructure)?,
            )),
            27 => u64::from_be_bytes(
                self.take(8)?
                    .try_into()
                    .map_err(|_| ProtocolError::InvalidStructure)?,
            ),
            _ => return Err(ProtocolError::InvalidStructure),
        };
        Ok((major, value))
    }

    fn text(&mut self, key: bool) -> Result<&'a str, ProtocolError> {
        let (major, length) = self.header()?;
        if major != 3 {
            return Err(ProtocolError::InvalidStructure);
        }
        let length = usize::try_from(length)?;
        if length > if key { 64 } else { 2048 } {
            return Err(ProtocolError::TooLarge);
        }
        str::from_utf8(self.take(length)?).map_err(|_| ProtocolError::InvalidStructure)
    }

    fn item(&mut self, depth: usize, payload: bool) -> Result<(), ProtocolError> {
        if depth > 16 {
            return Err(ProtocolError::TooLarge);
        }
        let (major, value) = self.header()?;
        match major {
            0 => Ok(()),
            2 if payload && self.payload.is_none() => {
                let length = usize::try_from(value)?;
                if length > MAX_PAYLOAD_BYTES {
                    return Err(ProtocolError::TooLarge);
                }
                self.take(length)?;
                self.payload = Some(length);
                Ok(())
            }
            3 => {
                let length = usize::try_from(value)?;
                if length > 2048 {
                    return Err(ProtocolError::TooLarge);
                }
                str::from_utf8(self.take(length)?).map_err(|_| ProtocolError::InvalidStructure)?;
                Ok(())
            }
            5 => {
                let count = usize::try_from(value)?;
                self.members = self
                    .members
                    .checked_add(count)
                    .ok_or(ProtocolError::TooLarge)?;
                if self.members > 128 {
                    return Err(ProtocolError::TooLarge);
                }
                let mut keys = HashSet::with_capacity(count);
                for _ in 0..count {
                    let key = self.text(true)?;
                    if !keys.insert(key) {
                        return Err(ProtocolError::InvalidStructure);
                    }
                    self.item(depth + 1, key == "payload")?;
                }
                Ok(())
            }
            7 if value == 20 || value == 21 => Ok(()),
            _ => Err(ProtocolError::InvalidStructure),
        }
    }
}
