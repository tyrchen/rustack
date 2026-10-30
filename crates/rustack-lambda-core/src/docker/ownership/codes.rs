//! Bounded code journal, checked producer transitions and monotonic retirement.

use std::{collections::BTreeSet, fs::File, io};

use nix::fcntl::OFlag;
use sha2::{Digest, Sha256};
use tokio::time::Instant;
use typed_builder::TypedBuilder;
use uuid::Uuid;

use super::{
    DirectoryLease, DirectoryLeaseError, Provenance, managed_file,
    snapshot::{Codec, read},
};
use crate::docker::{DockerError, DockerKey, mutation::MutationReceipt};

mod identity;

const MAGIC: &[u8; 8] = b"RSDKCOD1";
const RECORD_BYTES: usize = 194;
const MAX_RECORDS: usize = 32;
const MAX_BYTES: usize = 25 + MAX_RECORDS * RECORD_BYTES + 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::docker) enum VolumeState {
    Planned,
    Uncertain,
    Known,
    Retired,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::docker) enum LoaderState {
    Planned,
    Uncertain,
    Known([u8; 32]),
    Retired,
}
#[derive(Debug, Clone, Copy)]
pub(in crate::docker) enum CodeEffect {
    Volume,
    Loader,
}

#[derive(Clone, PartialEq, Eq, TypedBuilder)]
pub(in crate::docker) struct CodeRecord {
    pub(in crate::docker) run: Uuid,
    pub(in crate::docker) environment: Uuid,
    resource: Uuid,
    revision: Uuid,
    zip: [u8; 32],
    base: [u8; 32],
    fingerprint: [u8; 32],
    pub(in crate::docker) volume: VolumeState,
    pub(in crate::docker) loader: LoaderState,
}
impl std::fmt::Debug for CodeRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CodeRecord(<redacted>)")
    }
}
pub(in crate::docker) fn volume_key(environment: Uuid) -> Result<DockerKey, DockerError> {
    format!("rustack-code-{environment}").parse()
}

impl CodeRecord {
    fn valid(&self) -> bool {
        ![self.run, self.environment, self.resource, self.revision]
            .iter()
            .any(Uuid::is_nil)
            && match self.volume {
                VolumeState::Planned | VolumeState::Uncertain => {
                    self.loader == LoaderState::Planned
                }
                VolumeState::Known => true,
                VolumeState::Retired => self.loader == LoaderState::Retired,
            }
    }
    pub(in crate::docker) fn same_identity(&self, next: &Self) -> bool {
        let mut identity = self.clone();
        identity.volume = next.volume;
        identity.loader = next.loader;
        identity == *next
    }
    fn advances(&self, next: &Self) -> bool {
        self.same_identity(next)
            && next.valid()
            && ((self.volume == next.volume
                && matches!(
                    (self.loader, next.loader),
                    (
                        LoaderState::Planned | LoaderState::Known(_),
                        LoaderState::Retired
                    ) | (LoaderState::Planned, LoaderState::Uncertain)
                        | (LoaderState::Uncertain, LoaderState::Known(_))
                ))
                || (self.loader == next.loader
                    && matches!(
                        (self.volume, next.volume),
                        (VolumeState::Planned, VolumeState::Uncertain)
                            | (VolumeState::Uncertain, VolumeState::Known)
                            | (VolumeState::Known, VolumeState::Retired)
                    ))
                || (self.volume == VolumeState::Planned
                    && self.loader == LoaderState::Planned
                    && next.volume == VolumeState::Retired
                    && next.loader == LoaderState::Retired))
    }
}

pub(super) fn check_profile(
    directory: &File,
    owner: Uuid,
    provenance: Provenance,
) -> Result<(), DirectoryLeaseError> {
    if provenance == Provenance::CodeJournaled {
        read::<CodeCodec>(directory, owner)?;
    } else {
        for name in [CodeCodec::PUBLISHED, CodeCodec::STAGING] {
            match managed_file(directory, name, OFlag::O_RDONLY, MAX_BYTES as u64) {
                Err(DirectoryLeaseError::Io(error)) if error.kind() == io::ErrorKind::NotFound => {}
                _ => return Err(DirectoryLeaseError::InvalidState),
            }
        }
    }
    Ok(())
}
impl DirectoryLease {
    pub(in crate::docker) async fn code_records(
        &self,
        deadline: Instant,
    ) -> Result<Vec<CodeRecord>, DirectoryLeaseError> {
        let provenance = self.0.provenance;
        self.transaction(deadline, move |directory, owner, _control| {
            if provenance == Provenance::CodeJournaled {
                read::<CodeCodec>(directory, owner)
            } else {
                check_profile(directory, owner, provenance)?;
                Ok(Vec::new())
            }
        })
        .await
    }
    pub(in crate::docker) async fn prepare_code(
        &self,
        record: CodeRecord,
        deadline: Instant,
    ) -> Result<(), DirectoryLeaseError> {
        if !self.code_journaled()
            || record.run != self.run_id()
            || record.volume != VolumeState::Planned
            || record.loader != LoaderState::Planned
            || !record.valid()
        {
            return Err(DirectoryLeaseError::InvalidState);
        }
        let run = self.run_id();
        self.snapshot::<CodeCodec, _>(deadline, move |records| {
            if records.iter().any(|old| old.run != run) {
                return Err(DirectoryLeaseError::InvalidState);
            }
            if let Some(old) = records
                .iter()
                .find(|old| old.environment == record.environment)
            {
                return if *old == record {
                    Ok(())
                } else {
                    Err(DirectoryLeaseError::InvalidState)
                };
            }
            if records.len() >= MAX_RECORDS {
                return Err(DirectoryLeaseError::InvalidState);
            }
            records.push(record);
            Ok(())
        })
        .await
    }
    pub(in crate::docker) async fn abandon_code(
        &self,
        record: CodeRecord,
        effect: CodeEffect,
        receipt: &MutationReceipt,
        deadline: Instant,
    ) -> Result<(), DirectoryLeaseError> {
        if !self.code_journaled() || record.run != self.run_id() || !receipt.definitely_unissued() {
            return Err(DirectoryLeaseError::InvalidState);
        }
        let mut next = record.clone();
        match (effect, record.volume, record.loader) {
            (CodeEffect::Volume, VolumeState::Uncertain, LoaderState::Planned) => {
                next.volume = VolumeState::Retired;
                next.loader = LoaderState::Retired;
            }
            (CodeEffect::Loader, VolumeState::Known, LoaderState::Uncertain) => {
                next.loader = LoaderState::Retired;
            }
            _ => return Err(DirectoryLeaseError::InvalidState),
        }
        self.replace_code(record, next, deadline).await
    }
    pub(in crate::docker) async fn advance_code(
        &self,
        old: CodeRecord,
        next: CodeRecord,
        deadline: Instant,
    ) -> Result<(), DirectoryLeaseError> {
        let arming = (old.volume == VolumeState::Planned && next.volume == VolumeState::Uncertain)
            || (old.loader == LoaderState::Planned && next.loader == LoaderState::Uncertain);
        if !self.code_journaled() || !old.advances(&next) || (arming && old.run != self.run_id()) {
            return Err(DirectoryLeaseError::InvalidState);
        }
        self.replace_code(old, next, deadline).await
    }
    async fn replace_code(
        &self,
        old: CodeRecord,
        next: CodeRecord,
        deadline: Instant,
    ) -> Result<(), DirectoryLeaseError> {
        self.snapshot::<CodeCodec, _>(deadline, move |records| {
            let saved = records
                .iter_mut()
                .find(|record| record.environment == old.environment)
                .ok_or(DirectoryLeaseError::InvalidState)?;
            if *saved == next {
                return Ok(());
            }
            if *saved != old {
                return Err(DirectoryLeaseError::InvalidState);
            }
            *saved = next;
            Ok(())
        })
        .await
    }
    pub(in crate::docker) async fn settle_code(
        &self,
        record: CodeRecord,
        deadline: Instant,
    ) -> Result<(), DirectoryLeaseError> {
        if !self.code_journaled()
            || record.volume != VolumeState::Retired
            || record.loader != LoaderState::Retired
        {
            return Err(DirectoryLeaseError::InvalidState);
        }
        self.snapshot::<CodeCodec, _>(deadline, move |records| {
            if let Some(position) = records
                .iter()
                .position(|saved| saved.environment == record.environment)
            {
                if records.get(position) != Some(&record) {
                    return Err(DirectoryLeaseError::InvalidState);
                }
                records.remove(position);
            }
            Ok(())
        })
        .await
    }
}

pub(super) struct CodeCodec;
impl Codec for CodeCodec {
    type Record = CodeRecord;
    const PUBLISHED: &'static str = "code.v1";
    const STAGING: &'static str = "code.pending";
    const MAX_BYTES: usize = MAX_BYTES;
    fn encode(owner: Uuid, records: &[CodeRecord]) -> Result<Vec<u8>, DirectoryLeaseError> {
        if owner.is_nil() || records.len() > MAX_RECORDS {
            return Err(DirectoryLeaseError::InvalidState);
        }
        let mut bytes = Vec::with_capacity(MAX_BYTES);
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(owner.as_bytes());
        bytes.push(u8::try_from(records.len()).map_err(|_| DirectoryLeaseError::InvalidState)?);
        for record in records {
            for id in [
                record.run,
                record.environment,
                record.resource,
                record.revision,
            ] {
                bytes.extend_from_slice(id.as_bytes());
            }
            for digest in [record.zip, record.base, record.fingerprint] {
                bytes.extend_from_slice(&digest);
            }
            bytes.push(match record.volume {
                VolumeState::Planned => 0,
                VolumeState::Uncertain => 1,
                VolumeState::Known => 2,
                VolumeState::Retired => 3,
            });
            bytes.push(match record.loader {
                LoaderState::Planned => 0,
                LoaderState::Uncertain => 1,
                LoaderState::Known(_) => 2,
                LoaderState::Retired => 3,
            });
            bytes.extend_from_slice(&match record.loader {
                LoaderState::Known(id) => id,
                _ => [0; 32],
            });
        }
        bytes.extend_from_slice(&Sha256::digest(&bytes));
        Self::decode(&bytes, owner)?;
        Ok(bytes)
    }
    fn decode(bytes: &[u8], owner: Uuid) -> Result<Vec<CodeRecord>, DirectoryLeaseError> {
        let invalid = || DirectoryLeaseError::InvalidState;
        let count = usize::from(*bytes.get(24).ok_or_else(invalid)?);
        let body_len = count
            .checked_mul(RECORD_BYTES)
            .and_then(|n| n.checked_add(25))
            .ok_or_else(invalid)?;
        let expected_len = body_len.checked_add(32).ok_or_else(invalid)?;
        if owner.is_nil()
            || count > MAX_RECORDS
            || bytes.len() != expected_len
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
            .get(25..body_len)
            .ok_or_else(invalid)?
            .as_chunks::<RECORD_BYTES>()
            .0
        {
            let uuid = |start| {
                Uuid::from_slice(row.get(start..start + 16).ok_or_else(invalid)?)
                    .map_err(|_| invalid())
            };
            let digest = |start| -> Result<[u8; 32], DirectoryLeaseError> {
                row.get(start..start + 32)
                    .ok_or_else(invalid)?
                    .try_into()
                    .map_err(|_| invalid())
            };
            let volume = match row.get(160) {
                Some(0) => VolumeState::Planned,
                Some(1) => VolumeState::Uncertain,
                Some(2) => VolumeState::Known,
                Some(3) => VolumeState::Retired,
                _ => return Err(invalid()),
            };
            let id = digest(162)?;
            let loader = match row.get(161) {
                Some(0) if id == [0; 32] => LoaderState::Planned,
                Some(1) if id == [0; 32] => LoaderState::Uncertain,
                Some(2) => LoaderState::Known(id),
                Some(3) if id == [0; 32] => LoaderState::Retired,
                _ => return Err(invalid()),
            };
            let record = CodeRecord::builder()
                .run(uuid(0)?)
                .environment(uuid(16)?)
                .resource(uuid(32)?)
                .revision(uuid(48)?)
                .zip(digest(64)?)
                .base(digest(96)?)
                .fingerprint(digest(128)?)
                .volume(volume)
                .loader(loader)
                .build();
            if !record.valid() || !identities.insert(record.environment) {
                return Err(invalid());
            }
            records.push(record);
        }
        Ok(records)
    }
}

#[cfg(test)]
pub(in crate::docker) mod tests;
