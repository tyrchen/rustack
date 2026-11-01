//! Persistent private image-tag intent; unknown absence never proves settlement.

use std::{collections::BTreeSet, fmt};

use sha2::{Digest, Sha256};
use tokio::time::Instant;
use uuid::Uuid;

use super::{DirectoryLease, DirectoryLeaseError, snapshot::Codec};
use crate::{
    artifact::Sha256Digest,
    docker::{DockerError, ImagePin},
};

const PUBLISHED: &str = "pins.v1";
const STAGING: &str = "pins.pending";
const MAGIC: &[u8; 8] = b"RSDKPIN1";
const RECORD_BYTES: usize = 65;
const MAX_RECORDS: usize = 8192;
const MAX_BYTES: usize = 26 + MAX_RECORDS * RECORD_BYTES + 32;

#[derive(Clone, PartialEq, Eq)]
pub(in crate::docker) struct ImageRecord {
    pub(in crate::docker) run: Uuid,
    pin: Uuid,
    image: [u8; 32],
    pub(in crate::docker) committed: bool,
}
impl fmt::Debug for ImageRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ImageRecord(<redacted>)")
    }
}
impl ImageRecord {
    pub(in crate::docker) fn new(
        pin: &ImagePin,
        image: &Sha256Digest,
    ) -> Result<Self, DockerError> {
        let mut digest = [0; 32];
        hex::decode_to_slice(
            image
                .as_str()
                .strip_prefix("sha256:")
                .ok_or(DockerError::Configuration)?,
            &mut digest,
        )
        .map_err(|_| DockerError::Configuration)?;
        Ok(Self {
            run: pin.run,
            pin: pin.id,
            image: digest,
            committed: false,
        })
    }
    pub(in crate::docker) fn pin(
        &self,
        directory: &DirectoryLease,
    ) -> Result<ImagePin, DockerError> {
        ImagePin::historical(directory.clone(), self.run, self.pin)
    }
    pub(in crate::docker) fn expected(&self) -> Result<Sha256Digest, DockerError> {
        format!("sha256:{}", hex::encode(self.image))
            .parse()
            .map_err(DockerError::from)
    }
    fn same_key(&self, other: &Self) -> bool {
        self.run == other.run && self.pin == other.pin
    }
    fn identity(&self, other: &Self) -> bool {
        self.same_key(other) && self.image == other.image
    }
}
impl DirectoryLease {
    pub(in crate::docker) async fn image_records(
        &self,
        deadline: Instant,
    ) -> Result<Vec<ImageRecord>, DirectoryLeaseError> {
        self.snapshot::<ImageCodec, _>(deadline, |records| Ok(records.clone()))
            .await
    }
    pub(in crate::docker) async fn prepare_image(
        &self,
        record: ImageRecord,
        deadline: Instant,
    ) -> Result<(), DirectoryLeaseError> {
        if record.run != self.run_id() {
            return Err(DirectoryLeaseError::InvalidState);
        }
        self.snapshot::<ImageCodec, _>(deadline, move |records| {
            if let Some(old) = records.iter().find(|old| old.same_key(&record)) {
                if !old.identity(&record) {
                    return Err(DirectoryLeaseError::InvalidState);
                }
            } else {
                if records.len() >= MAX_RECORDS {
                    return Err(DirectoryLeaseError::InvalidState);
                }
                records.push(record);
            }
            Ok(())
        })
        .await
    }
    pub(in crate::docker) async fn commit_image(
        &self,
        record: ImageRecord,
        deadline: Instant,
    ) -> Result<(), DirectoryLeaseError> {
        self.snapshot::<ImageCodec, _>(deadline, move |records| {
            let old = records
                .iter_mut()
                .find(|old| old.same_key(&record))
                .ok_or(DirectoryLeaseError::InvalidState)?;
            if !old.identity(&record) {
                return Err(DirectoryLeaseError::InvalidState);
            }
            old.committed = true;
            Ok(())
        })
        .await
    }
    pub(in crate::docker) async fn settle_image(
        &self,
        record: ImageRecord,
        deadline: Instant,
    ) -> Result<(), DirectoryLeaseError> {
        self.snapshot::<ImageCodec, _>(deadline, move |records| {
            if let Some(position) = records.iter().position(|old| old.same_key(&record)) {
                if !records
                    .get(position)
                    .is_some_and(|old| old.identity(&record))
                {
                    return Err(DirectoryLeaseError::InvalidState);
                }
                records.remove(position);
            }
            Ok(())
        })
        .await
    }
}
pub(super) struct ImageCodec;
impl Codec for ImageCodec {
    type Record = ImageRecord;
    const PUBLISHED: &'static str = PUBLISHED;
    const STAGING: &'static str = STAGING;
    const MAX_BYTES: usize = MAX_BYTES;
    fn encode(owner: Uuid, records: &[ImageRecord]) -> Result<Vec<u8>, DirectoryLeaseError> {
        encode(owner, records)
    }
    fn decode(bytes: &[u8], owner: Uuid) -> Result<Vec<ImageRecord>, DirectoryLeaseError> {
        decode(bytes, owner)
    }
}
fn encode(owner: Uuid, records: &[ImageRecord]) -> Result<Vec<u8>, DirectoryLeaseError> {
    if owner.is_nil() || records.len() > MAX_RECORDS {
        return Err(DirectoryLeaseError::InvalidState);
    }
    let capacity = records
        .len()
        .checked_mul(RECORD_BYTES)
        .and_then(|bytes| bytes.checked_add(58))
        .ok_or(DirectoryLeaseError::InvalidState)?;
    let mut bytes = Vec::with_capacity(capacity);
    bytes.extend_from_slice(MAGIC);
    bytes.extend_from_slice(owner.as_bytes());
    bytes.extend_from_slice(
        &u16::try_from(records.len())
            .map_err(|_| DirectoryLeaseError::InvalidState)?
            .to_be_bytes(),
    );
    for record in records {
        bytes.extend_from_slice(record.run.as_bytes());
        bytes.extend_from_slice(record.pin.as_bytes());
        bytes.extend_from_slice(&record.image);
        bytes.push(u8::from(record.committed));
    }
    bytes.extend_from_slice(&Sha256::digest(&bytes));
    decode(&bytes, owner)?;
    Ok(bytes)
}
fn decode(bytes: &[u8], owner: Uuid) -> Result<Vec<ImageRecord>, DirectoryLeaseError> {
    let invalid = || DirectoryLeaseError::InvalidState;
    let count = usize::from(u16::from_be_bytes(
        bytes
            .get(24..26)
            .ok_or_else(invalid)?
            .try_into()
            .map_err(|_| invalid())?,
    ));
    if owner.is_nil() || count > MAX_RECORDS {
        return Err(invalid());
    }
    let body_len = count
        .checked_mul(RECORD_BYTES)
        .and_then(|bytes| bytes.checked_add(26))
        .ok_or_else(invalid)?;
    let expected_len = body_len.checked_add(32).ok_or_else(invalid)?;
    if bytes.len() != expected_len
        || bytes.get(..8) != Some(MAGIC.as_slice())
        || bytes.get(8..24) != Some(owner.as_bytes().as_slice())
        || bytes.get(body_len..)
            != Some(Sha256::digest(bytes.get(..body_len).ok_or_else(invalid)?).as_slice())
    {
        return Err(invalid());
    }
    let mut records = Vec::with_capacity(count);
    let mut identities = BTreeSet::new();
    for row in bytes
        .get(26..body_len)
        .ok_or_else(invalid)?
        .as_chunks::<RECORD_BYTES>()
        .0
    {
        let run = Uuid::from_slice(row.get(..16).ok_or_else(invalid)?).map_err(|_| invalid())?;
        let pin = Uuid::from_slice(row.get(16..32).ok_or_else(invalid)?).map_err(|_| invalid())?;
        if run.is_nil() || pin.is_nil() || !identities.insert((run, pin)) {
            return Err(invalid());
        }
        let committed = match row.get(64) {
            Some(0) => false,
            Some(1) => true,
            _ => return Err(invalid()),
        };
        records.push(ImageRecord {
            run,
            pin,
            image: row
                .get(32..64)
                .ok_or_else(invalid)?
                .try_into()
                .map_err(|_| invalid())?,
            committed,
        });
    }
    Ok(records)
}

#[cfg(test)]
mod tests;
