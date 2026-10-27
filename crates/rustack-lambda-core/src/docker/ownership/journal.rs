//! Bounded, atomically replaced container identities; no deployment secrets.

use std::fmt;

use sha2::{Digest, Sha256};
use tokio::time::Instant;
use uuid::Uuid;

use super::{DirectoryLease, DirectoryLeaseError, snapshot::Codec};
use crate::docker::{
    DockerError, DockerKey, EnvironmentPlan, models::ContainerInspectResponse, plan::ContainerRole,
};

const PUBLISHED: &str = "containers.v1";
const STAGING: &str = "containers.pending";
const HEADER: &[u8; 8] = b"RSDKCTR1";
const RECORD_BYTES: usize = 114;
const MAX_RECORDS: usize = 64;
const MAX_BYTES: usize = 25 + MAX_RECORDS * RECORD_BYTES + 32;

#[derive(Clone, PartialEq, Eq)]
pub(in crate::docker) struct ContainerRecord {
    pub(in crate::docker) run: Uuid,
    pub(in crate::docker) environment: Uuid,
    resource: Uuid,
    function: bool,
    image: [u8; 32],
    pub(in crate::docker) known: Option<[u8; 32]>,
}
impl fmt::Debug for ContainerRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ContainerRecord(<redacted>)")
    }
}
impl ContainerRecord {
    pub(in crate::docker) fn from_plan(
        plan: &EnvironmentPlan,
        role: ContainerRole,
    ) -> Result<Self, DockerError> {
        let image = plan
            .image_id(role)
            .strip_prefix("sha256:")
            .ok_or(DockerError::Configuration)?;
        let mut digest = [0; 32];
        hex::decode_to_slice(image, &mut digest).map_err(|_| DockerError::Configuration)?;
        Ok(Self {
            run: plan.directory.run_id(),
            environment: plan
                .environment_id()
                .to_string()
                .parse()
                .map_err(|_| DockerError::Configuration)?,
            resource: plan
                .revision()
                .resource_id
                .to_string()
                .parse()
                .map_err(|_| DockerError::Configuration)?,
            function: matches!(role, ContainerRole::Function),
            image: digest,
            known: None,
        })
    }
    pub(in crate::docker) fn function(&self) -> bool {
        self.function
    }
    fn role(&self) -> &'static str {
        if self.function { "function" } else { "relay" }
    }
    pub(in crate::docker) fn key(&self) -> Result<DockerKey, DockerError> {
        format!("rustack-{}-{}", self.role(), self.environment).parse()
    }
    pub(in crate::docker) fn known_key(&self) -> Result<Option<DockerKey>, DockerError> {
        self.known.map(|id| hex::encode(id).parse()).transpose()
    }
    pub(in crate::docker) fn owned(
        &self,
        owner: Uuid,
        inspected: &ContainerInspectResponse,
    ) -> Result<DockerKey, DockerError> {
        let config = inspected.config.as_ref().ok_or(DockerError::Protocol)?;
        let labels = config.labels.as_ref().ok_or(DockerError::Quarantined)?;
        let id = inspected.id.as_deref().ok_or(DockerError::Protocol)?;
        let mut decoded = [0; 32];
        if id.len() != 64
            || !id
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(DockerError::Quarantined);
        }
        hex::decode_to_slice(id, &mut decoded).map_err(|_| DockerError::Quarantined)?;
        if self.known.is_some_and(|expected| expected != decoded)
            || inspected.name.as_deref() != Some(format!("/{}", self.key()?.as_str()).as_str())
            || config.image.as_deref()
                != Some(format!("sha256:{}", hex::encode(self.image)).as_str())
            || ![
                ("rustack.lambda.ownerId", owner.to_string()),
                ("rustack.lambda.runId", self.run.to_string()),
                ("rustack.lambda.environmentId", self.environment.to_string()),
                (
                    "rustack.lambda.functionResourceId",
                    self.resource.to_string(),
                ),
                ("rustack.lambda.role", self.role().to_owned()),
            ]
            .iter()
            .all(|(key, value)| labels.get(*key) == Some(value))
        {
            return Err(DockerError::Quarantined);
        }
        id.parse()
    }
    fn same_key(&self, other: &Self) -> bool {
        self.environment == other.environment && self.function == other.function
    }
    fn identity(&self, other: &Self) -> bool {
        self.run == other.run
            && self.environment == other.environment
            && self.resource == other.resource
            && self.function == other.function
            && self.image == other.image
    }
}
impl DirectoryLease {
    async fn journal<T: Send + 'static>(
        &self,
        deadline: Instant,
        work: impl FnOnce(&mut Vec<ContainerRecord>) -> Result<T, DirectoryLeaseError> + Send + 'static,
    ) -> Result<T, DirectoryLeaseError> {
        self.snapshot::<ContainerCodec, T>(deadline, work).await
    }
    pub(in crate::docker) async fn container_records(
        &self,
        deadline: Instant,
    ) -> Result<Vec<ContainerRecord>, DirectoryLeaseError> {
        self.journal(deadline, |records| Ok(records.clone())).await
    }
    pub(in crate::docker) async fn prepare_container(
        &self,
        record: ContainerRecord,
        deadline: Instant,
    ) -> Result<(), DirectoryLeaseError> {
        if record.run != self.run_id() {
            return Err(DirectoryLeaseError::InvalidState);
        }
        self.journal(deadline, move |records| {
            if let Some(previous) = records.iter().find(|old| old.same_key(&record)) {
                if !previous.identity(&record) {
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
    pub(in crate::docker) async fn know_container(
        &self,
        record: ContainerRecord,
        id: DockerKey,
        deadline: Instant,
    ) -> Result<(), DirectoryLeaseError> {
        let mut decoded = [0; 32];
        if id.as_str().len() != 64
            || !id
                .as_str()
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(DirectoryLeaseError::InvalidState);
        }
        hex::decode_to_slice(id.as_str(), &mut decoded)
            .map_err(|_| DirectoryLeaseError::InvalidState)?;
        self.journal(deadline, move |records| {
            let previous = records
                .iter_mut()
                .find(|old| old.same_key(&record))
                .ok_or(DirectoryLeaseError::InvalidState)?;
            if !previous.identity(&record) || previous.known.is_some_and(|old| old != decoded) {
                return Err(DirectoryLeaseError::InvalidState);
            }
            previous.known = Some(decoded);
            Ok(())
        })
        .await
    }
    pub(in crate::docker) async fn settle_container(
        &self,
        record: ContainerRecord,
        deadline: Instant,
    ) -> Result<(), DirectoryLeaseError> {
        self.journal(deadline, move |records| {
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

fn encode(owner: Uuid, records: &[ContainerRecord]) -> Result<Vec<u8>, DirectoryLeaseError> {
    if records.len() > MAX_RECORDS || owner.is_nil() {
        return Err(DirectoryLeaseError::InvalidState);
    }
    let mut bytes = Vec::with_capacity(MAX_BYTES);
    bytes.extend_from_slice(HEADER);
    bytes.extend_from_slice(owner.as_bytes());
    bytes.push(u8::try_from(records.len()).map_err(|_| DirectoryLeaseError::InvalidState)?);
    for record in records {
        bytes.extend_from_slice(record.run.as_bytes());
        bytes.extend_from_slice(record.environment.as_bytes());
        bytes.extend_from_slice(record.resource.as_bytes());
        bytes.push(u8::from(!record.function));
        bytes.extend_from_slice(&record.image);
        bytes.push(u8::from(record.known.is_some()));
        bytes.extend_from_slice(&record.known.unwrap_or([0; 32]));
    }
    let checksum = Sha256::digest(&bytes);
    bytes.extend_from_slice(&checksum);
    decode(&bytes, owner)?;
    Ok(bytes)
}
fn decode(bytes: &[u8], owner: Uuid) -> Result<Vec<ContainerRecord>, DirectoryLeaseError> {
    let invalid = || DirectoryLeaseError::InvalidState;
    let count = usize::from(*bytes.get(24).ok_or_else(invalid)?);
    let body_len = 25 + count * RECORD_BYTES;
    if count > MAX_RECORDS
        || bytes.len() != body_len + 32
        || bytes.get(..8) != Some(HEADER.as_slice())
        || bytes.get(8..24) != Some(owner.as_bytes().as_slice())
        || bytes.get(body_len..)
            != Some(Sha256::digest(bytes.get(..body_len).ok_or_else(invalid)?).as_slice())
    {
        return Err(invalid());
    }
    let mut records: Vec<ContainerRecord> = Vec::with_capacity(count);
    for row in bytes
        .get(25..body_len)
        .ok_or_else(invalid)?
        .as_chunks::<RECORD_BYTES>()
        .0
    {
        let uuid = |start| -> Result<Uuid, DirectoryLeaseError> {
            let value = Uuid::from_slice(row.get(start..start + 16).ok_or_else(invalid)?)
                .map_err(|_| invalid())?;
            if value.is_nil() {
                Err(invalid())
            } else {
                Ok(value)
            }
        };
        let function = match row.get(48) {
            Some(0) => true,
            Some(1) => false,
            _ => return Err(invalid()),
        };
        let id: [u8; 32] = row
            .get(82..114)
            .ok_or_else(invalid)?
            .try_into()
            .map_err(|_| invalid())?;
        let known = match row.get(81) {
            Some(0) if id == [0; 32] => None,
            Some(1) => Some(id),
            _ => return Err(invalid()),
        };
        let record = ContainerRecord {
            run: uuid(0)?,
            environment: uuid(16)?,
            resource: uuid(32)?,
            function,
            image: row
                .get(49..81)
                .ok_or_else(invalid)?
                .try_into()
                .map_err(|_| invalid())?,
            known,
        };
        if records.iter().any(|old| {
            old.same_key(&record)
                || (old.environment == record.environment
                    && (old.run != record.run || old.resource != record.resource))
        }) {
            return Err(invalid());
        }
        records.push(record);
    }
    Ok(records)
}
struct ContainerCodec;
impl Codec for ContainerCodec {
    type Record = ContainerRecord;
    const PUBLISHED: &'static str = PUBLISHED;
    const STAGING: &'static str = STAGING;
    const MAX_BYTES: usize = MAX_BYTES;
    fn encode(owner: Uuid, records: &[ContainerRecord]) -> Result<Vec<u8>, DirectoryLeaseError> {
        encode(owner, records)
    }
    fn decode(bytes: &[u8], owner: Uuid) -> Result<Vec<ContainerRecord>, DirectoryLeaseError> {
        decode(bytes, owner)
    }
}

#[cfg(test)]
mod tests;
