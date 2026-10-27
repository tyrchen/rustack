//! Shared bounded file transactions; individual codecs own their wire formats.

use std::{
    fs::File,
    io::{self, Read, Write},
    time::Duration,
};

use nix::fcntl::{OFlag, renameat};
use tokio::time::{Instant, timeout_at};
use uuid::Uuid;

use super::{Control, DirectoryLease, DirectoryLeaseError, LeaseGuard, WORKERS, managed_file, run};

pub(super) trait Codec {
    type Record: Clone + PartialEq + Send + 'static;
    const PUBLISHED: &'static str;
    const STAGING: &'static str;
    const MAX_BYTES: usize;
    fn encode(owner: Uuid, records: &[Self::Record]) -> Result<Vec<u8>, DirectoryLeaseError>;
    fn decode(bytes: &[u8], owner: Uuid) -> Result<Vec<Self::Record>, DirectoryLeaseError>;
}
impl DirectoryLease {
    fn namespace(&self) -> &File {
        match &self.0.guard {
            LeaseGuard::Held { directory, .. } => directory,
            #[cfg(test)]
            LeaseGuard::Testing { directory, .. } => directory,
        }
    }
    pub(super) async fn snapshot<C: Codec, T: Send + 'static>(
        &self,
        deadline: Instant,
        work: impl FnOnce(&mut Vec<C::Record>) -> Result<T, DirectoryLeaseError> + Send + 'static,
    ) -> Result<T, DirectoryLeaseError> {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .filter(|duration| !duration.is_zero())
            .ok_or(DirectoryLeaseError::Deadline)?;
        if remaining > Duration::from_secs(60) {
            return Err(DirectoryLeaseError::Configuration);
        }
        let permit = timeout_at(deadline, self.0.transaction.clone().acquire_owned())
            .await
            .map_err(|_| DirectoryLeaseError::Deadline)?
            .map_err(|_| DirectoryLeaseError::Worker)?;
        let lease = self.clone();
        run(&WORKERS, deadline, move |control| {
            let _transaction = permit;
            let mut records = read::<C>(lease.namespace(), lease.owner_id())?;
            control.check()?;
            let original = records.clone();
            let result = work(&mut records)?;
            if records != original {
                write::<C>(lease.namespace(), lease.owner_id(), &records, control)?;
            }
            Ok(result)
        })
        .await
    }
}
fn read<C: Codec>(directory: &File, owner: Uuid) -> Result<Vec<C::Record>, DirectoryLeaseError> {
    optional::<C>(directory, C::STAGING)?;
    let Some(mut file) = optional::<C>(directory, C::PUBLISHED)? else {
        directory.sync_all()?;
        return Ok(Vec::new());
    };
    let mut bytes = Vec::with_capacity(C::MAX_BYTES);
    Read::by_ref(&mut file)
        .take((C::MAX_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    let records = C::decode(&bytes, owner)?;
    file.sync_all()?;
    directory.sync_all()?;
    Ok(records)
}
fn optional<C: Codec>(directory: &File, name: &str) -> Result<Option<File>, DirectoryLeaseError> {
    match managed_file(directory, name, OFlag::O_RDONLY, C::MAX_BYTES as u64) {
        Ok(file) => Ok(Some(file)),
        Err(DirectoryLeaseError::Io(error)) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}
fn write<C: Codec>(
    directory: &File,
    owner: Uuid,
    records: &[C::Record],
    control: &Control,
) -> Result<(), DirectoryLeaseError> {
    let bytes = C::encode(owner, records)?;
    if bytes.len() > C::MAX_BYTES {
        return Err(DirectoryLeaseError::InvalidState);
    }
    let mut file = managed_file(
        directory,
        C::STAGING,
        OFlag::O_RDWR | OFlag::O_CREAT,
        C::MAX_BYTES as u64,
    )?;
    control.check()?;
    file.set_len(0)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    control.check()?;
    renameat(directory, C::STAGING, directory, C::PUBLISHED)?;
    directory.sync_all()?;
    Ok(())
}
