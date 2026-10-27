//! Complete bounded observed inventories, never authority to adopt an effect.

use std::fmt;

use bytes::Bytes;
use http::{Method, StatusCode};
use serde::{Deserialize, Deserializer};
use tokio::time::Instant;

use crate::{
    artifact::ImageReference,
    docker::{
        DirectoryLease, DockerError, DockerPort, Operation, OwnerFilter, check_deadline, decode,
        models::ContainerSummary,
    },
};

const MAX_IMAGES: usize = 4096;
const MAX_TAGS: usize = 4096;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ImageTags {
    #[serde(rename = "RepoTags", deserialize_with = "tags")]
    tags: Vec<String>,
}
impl fmt::Debug for ImageTags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ImageTags(<redacted>)")
    }
}
fn tags<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    Ok(Option::<Vec<String>>::deserialize(deserializer)?.unwrap_or_default())
}

pub(in crate::docker) async fn containers(
    directory: &DirectoryLease,
    docker: &DockerPort,
    deadline: Instant,
) -> Result<(), DockerError> {
    docker.require_directory(directory)?;
    let filter = OwnerFilter::new(directory.owner_id(), None, None)?;
    let rows: Vec<ContainerSummary> = decode(
        docker
            .call(
                Operation::List,
                Method::GET,
                format!("/containers/json?{}", filter.query(None)?),
                Bytes::new(),
                StatusCode::OK,
                deadline,
            )
            .await?,
        deadline,
    )?;
    if rows.len() > 64 {
        return Err(DockerError::TooLarge);
    }
    if !rows.is_empty() {
        return Err(DockerError::Quarantined);
    }
    check_deadline(deadline)
}

pub(in crate::docker) async fn images(
    directory: &DirectoryLease,
    docker: &DockerPort,
    deadline: Instant,
) -> Result<(), DockerError> {
    docker.require_directory(directory)?;
    let rows: Vec<ImageTags> = decode(
        docker
            .call(
                Operation::ListImages,
                Method::GET,
                "/images/json?all=true".into(),
                Bytes::new(),
                StatusCode::OK,
                deadline,
            )
            .await?,
        deadline,
    )?;
    check_tags(rows, directory, deadline)
}
fn check_tags(
    rows: Vec<ImageTags>,
    directory: &DirectoryLease,
    deadline: Instant,
) -> Result<(), DockerError> {
    if rows.len() > MAX_IMAGES {
        return Err(DockerError::TooLarge);
    }
    let prefix = format!("rustack-internal/lambda-{}/", directory.owner_id().simple());
    let mut count = 0_usize;
    for row in rows {
        count = count
            .checked_add(row.tags.len())
            .ok_or(DockerError::TooLarge)?;
        if count > MAX_TAGS {
            return Err(DockerError::TooLarge);
        }
        for tag in row.tags {
            check_deadline(deadline)?;
            if tag.len() > 1024 {
                return Err(DockerError::TooLarge);
            }
            if tag == "<none>:<none>" {
                continue;
            }
            let familiar = tag
                .strip_prefix("docker.io/")
                .or_else(|| tag.strip_prefix("index.docker.io/"))
                .unwrap_or(&tag);
            if familiar.starts_with(&prefix) {
                return Err(DockerError::Quarantined);
            }
            tag.parse::<ImageReference>()
                .map_err(|_| DockerError::Protocol)?;
        }
    }
    check_deadline(deadline)
}

#[cfg(test)]
mod tests;
