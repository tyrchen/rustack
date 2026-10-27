//! Immutable owner/engine association, published before any journaled effect.

use sha2::{Digest, Sha256};
use tokio::time::Instant;
use uuid::Uuid;

use super::{
    DirectoryLease, DirectoryLeaseError,
    images::ImageCodec,
    journal::ContainerCodec,
    snapshot::{Codec, read, write},
};
use crate::docker::binding::DaemonIdentity;

const MAGIC: &[u8; 8] = b"RSDKDMD1";
const BYTES: usize = 72;

impl DirectoryLease {
    pub(in crate::docker) async fn bind_daemon(
        &self,
        identity: DaemonIdentity,
        deadline: Instant,
    ) -> Result<(), DirectoryLeaseError> {
        self.transaction(deadline, move |directory, owner, control| {
            let existing = read::<DaemonCodec>(directory, owner)?;
            control.check()?;
            match existing.as_slice() {
                [saved] if *saved == identity => Ok(()),
                [] => {
                    if !read::<ContainerCodec>(directory, owner)?.is_empty()
                        || !read::<ImageCodec>(directory, owner)?.is_empty()
                    {
                        return Err(DirectoryLeaseError::InvalidState);
                    }
                    write::<DaemonCodec>(directory, owner, &[identity], control)
                }
                _ => Err(DirectoryLeaseError::InvalidState),
            }
        })
        .await
    }
}

#[derive(Debug)]
struct DaemonCodec;
impl Codec for DaemonCodec {
    type Record = DaemonIdentity;
    const PUBLISHED: &'static str = "daemon.v1";
    const STAGING: &'static str = "daemon.pending";
    const MAX_BYTES: usize = BYTES;

    fn encode(owner: Uuid, records: &[DaemonIdentity]) -> Result<Vec<u8>, DirectoryLeaseError> {
        let [identity] = records else {
            return Err(DirectoryLeaseError::InvalidState);
        };
        if owner.is_nil() {
            return Err(DirectoryLeaseError::InvalidState);
        }
        let mut bytes = Vec::with_capacity(BYTES);
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(owner.as_bytes());
        bytes.extend_from_slice(identity.uuid().as_bytes());
        bytes.extend_from_slice(&Sha256::digest(&bytes));
        Ok(bytes)
    }

    fn decode(bytes: &[u8], owner: Uuid) -> Result<Vec<DaemonIdentity>, DirectoryLeaseError> {
        let invalid = || DirectoryLeaseError::InvalidState;
        if owner.is_nil()
            || bytes.len() != BYTES
            || bytes.get(..8) != Some(MAGIC.as_slice())
            || bytes.get(8..24) != Some(owner.as_bytes().as_slice())
            || bytes.get(40..)
                != Some(Sha256::digest(bytes.get(..40).ok_or_else(invalid)?).as_slice())
        {
            return Err(invalid());
        }
        let uuid =
            Uuid::from_slice(bytes.get(24..40).ok_or_else(invalid)?).map_err(|_| invalid())?;
        Ok(vec![
            DaemonIdentity::from_uuid(uuid).map_err(|_| invalid())?,
        ])
    }
}

#[cfg(test)]
mod tests;
